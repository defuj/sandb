//! Maps database values to JSON for the grid and CSV export. Pure functions, unit tested; the
//! per-engine row readers in `query.rs` call them.
//!
//! Rules: integers → number while JS holds them exactly, else string; numeric/decimal → string
//! (keeps precision); floats → number; json → JSON value; bytes → `{"$bytes": base64, "len": n}`;
//! NULL → null; the rest → text. With a cap (the grid preview), long text and bytes are cut.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Number, Value, json};

/// Bytes shown per binary or text cell in the grid. CSV export passes `None` (no cap).
pub const PREVIEW_BYTES: usize = 4096;

/// Largest integer a JS number holds exactly (2^53 - 1).
const MAX_SAFE_INT: u64 = 9_007_199_254_740_991;

pub fn bytes(b: &[u8], cap: Option<usize>) -> Value {
    let shown = match cap {
        Some(c) => &b[..b.len().min(c)],
        None => b,
    };
    json!({"$bytes": STANDARD.encode(shown), "len": b.len()})
}

/// A number while JS can hold it exactly, else a string.
pub fn int(i: i64) -> Value {
    if i.unsigned_abs() <= MAX_SAFE_INT {
        Value::from(i)
    } else {
        Value::String(i.to_string())
    }
}

/// NaN and infinities have no JSON number form; they become strings.
pub fn float(f: f64) -> Value {
    Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(f.to_string()))
}

pub fn unsupported(type_name: &str) -> Value {
    Value::String(format!("<unsupported: {type_name}>"))
}

fn text(s: &str) -> Value {
    Value::String(s.to_string())
}

/// Text cut to at most `cap` bytes on a char boundary, with `…` when cut. Geometry as WKT or
/// hex EWKB runs to megabytes per cell; sending it whole to the grid freezes the UI.
pub fn text_capped(s: &str, cap: Option<usize>) -> Value {
    match cap {
        Some(c) if s.len() > c => {
            let end = (0..=c).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
            Value::String(format!("{}…", &s[..end]))
        }
        _ => text(s),
    }
}

fn int_or_text(s: &str) -> Value {
    s.trim().parse::<i64>().map(int).unwrap_or_else(|_| text(s))
}

fn float_or_text(s: &str) -> Value {
    match s.trim().parse::<f64>() {
        Ok(f) if f.is_finite() => float(f),
        _ => text(s),
    }
}

fn json_or_text(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| text(s))
}

/// PG text-format value (simple query protocol) by type name as sqlx reports it (`INT4`, `TEXT[]`, …).
pub fn pg_text(type_name: &str, s: &str, cap: Option<usize>) -> Value {
    let t = type_name.to_ascii_uppercase();
    if let Some(elem) = t.strip_suffix("[]") {
        return pg_array(elem, s, cap).unwrap_or_else(|| text(s));
    }
    match t.as_str() {
        "BOOL" => match s {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => text(s),
        },
        "INT2" | "INT4" | "INT8" | "OID" => int_or_text(s),
        "FLOAT4" | "FLOAT8" => float_or_text(s),
        "JSON" | "JSONB" => json_or_text(s),
        "BYTEA" => pg_bytea(s).map_or_else(|| text(s), |b| bytes(&b, cap)),
        // NUMERIC and MONEY stay strings on purpose; dates, uuid, enums, unknown types are text.
        _ => text_capped(s, cap),
    }
}

/// `\xdeadbeef` (hex output, the default since PG 9.0) → bytes.
pub fn pg_bytea(s: &str) -> Option<Vec<u8>> {
    let hex = s.strip_prefix("\\x")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// PG array literal (`{1,2,NULL}`, `{"a","b c"}`, `{{1,2},{3,4}}`, `[0:1]={…}`) → JSON array.
pub fn pg_array(elem_type: &str, s: &str, cap: Option<usize>) -> Option<Value> {
    let body = if s.starts_with('[') {
        &s[s.find('=')? + 1..]
    } else {
        s
    };
    let mut p = ArrayParser {
        chars: body.chars().collect(),
        pos: 0,
    };
    let v = p.array(elem_type, cap)?;
    p.skip_ws();
    (p.pos == p.chars.len()).then_some(v)
}

struct ArrayParser {
    chars: Vec<char>,
    pos: usize,
}

impl ArrayParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn array(&mut self, elem: &str, cap: Option<usize>) -> Option<Value> {
        self.skip_ws();
        if self.bump()? != '{' {
            return None;
        }
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Some(Value::Array(out));
        }
        loop {
            self.skip_ws();
            let v = match self.peek()? {
                '{' => self.array(elem, cap)?,
                '"' => {
                    self.pos += 1;
                    let mut s = String::new();
                    loop {
                        match self.bump()? {
                            '\\' => s.push(self.bump()?),
                            '"' => break,
                            c => s.push(c),
                        }
                    }
                    pg_text(elem, &s, cap)
                }
                _ => {
                    let start = self.pos;
                    while self.peek().is_some_and(|c| c != ',' && c != '}') {
                        self.pos += 1;
                    }
                    let raw: String = self.chars[start..self.pos].iter().collect();
                    let raw = raw.trim();
                    if raw.eq_ignore_ascii_case("NULL") {
                        Value::Null
                    } else {
                        pg_text(elem, raw, cap)
                    }
                }
            };
            out.push(v);
            self.skip_ws();
            match self.bump()? {
                ',' => continue,
                '}' => return Some(Value::Array(out)),
                _ => return None,
            }
        }
    }
}

/// MySQL/MariaDB text-protocol value by the sqlx type name (`INT`, `BOOLEAN`, `BLOB`, …).
pub fn mysql_value(type_name: &str, raw: &[u8], cap: Option<usize>) -> Value {
    let t = type_name.to_ascii_uppercase();
    let as_text = std::str::from_utf8(raw);
    match t.as_str() {
        "NULL" => Value::Null,
        "BOOLEAN" => match raw {
            b"0" => Value::Bool(false),
            b"1" => Value::Bool(true),
            _ => as_text.map_or_else(|_| unsupported(&t), int_or_text),
        },
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" | "TINYINT UNSIGNED"
        | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED" | "BIGINT UNSIGNED" => {
            as_text.map_or_else(|_| unsupported(&t), int_or_text)
        }
        "FLOAT" | "DOUBLE" => as_text.map_or_else(|_| unsupported(&t), float_or_text),
        "JSON" => as_text.map_or_else(|_| bytes(raw, cap), json_or_text),
        // sqlx reports text with a binary collation (utf8mb4_bin, MariaDB's JSON) as these types and
        // does not expose the collation, so readable UTF-8 is shown as text and anything else as bytes.
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => match as_text {
            Ok(s) if looks_like_text(s) => text_capped(s, cap),
            _ => bytes(raw, cap),
        },
        "BIT" | "GEOMETRY" => bytes(raw, cap),
        // DECIMAL stays a string; text, enum, set, date and time types are text.
        "DECIMAL" | "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT"
        | "ENUM" | "SET" | "DATE" | "TIME" | "DATETIME" | "TIMESTAMP" => {
            as_text.map_or_else(|_| bytes(raw, cap), |s| text_capped(s, cap))
        }
        _ => as_text.map_or_else(|_| unsupported(&t), |s| text_capped(s, cap)),
    }
}

/// Non-empty and free of control characters other than tab, CR and LF.
fn looks_like_text(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}

/// A SQLite value by its storage class.
pub enum SqliteRaw<'a> {
    Null,
    Int(i64),
    Real(f64),
    Text(&'a str),
    Blob(&'a [u8]),
}

/// `declared`: the sqlx column type name (`BOOLEAN` when declared so); storage class decides the rest.
pub fn sqlite_value(declared: &str, raw: SqliteRaw<'_>, cap: Option<usize>) -> Value {
    match raw {
        SqliteRaw::Null => Value::Null,
        SqliteRaw::Int(i) if declared.eq_ignore_ascii_case("BOOLEAN") && (i == 0 || i == 1) => {
            Value::Bool(i == 1)
        }
        SqliteRaw::Int(i) => int(i),
        SqliteRaw::Real(f) => float(f),
        SqliteRaw::Text(s) => text_capped(s, cap),
        SqliteRaw::Blob(b) => bytes(b, cap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints_keep_precision() {
        assert_eq!(int(42), json!(42));
        assert_eq!(
            int(-9_007_199_254_740_991),
            json!(-9_007_199_254_740_991_i64)
        );
        assert_eq!(int(9_007_199_254_740_993), json!("9007199254740993"));
        assert_eq!(int(i64::MIN), json!("-9223372036854775808"));
    }

    #[test]
    fn floats_and_non_finite() {
        assert_eq!(float(3.5), json!(3.5));
        assert_eq!(float(f64::NAN), json!("NaN"));
        assert_eq!(float(f64::INFINITY), json!("inf"));
    }

    #[test]
    fn bytes_are_base64_with_cap() {
        assert_eq!(
            bytes(&[0xde, 0xad, 0xbe, 0xef], None),
            json!({"$bytes": "3q2+7w==", "len": 4})
        );
        assert_eq!(
            bytes(&[1, 2, 3, 4], Some(2)),
            json!({"$bytes": "AQI=", "len": 4})
        );
    }

    #[test]
    fn long_text_is_cut_only_with_a_cap() {
        let wkt = format!("POLYGON(({}))", "106.8 -6.2, ".repeat(100_000));
        let v = pg_text("TEXT", &wkt, Some(PREVIEW_BYTES));
        let s = v.as_str().unwrap();
        assert!(s.ends_with('…') && s.len() <= PREVIEW_BYTES + '…'.len_utf8());
        assert!(wkt.starts_with(s.trim_end_matches('…')));
        assert_eq!(
            pg_text("TEXT", &wkt, None),
            json!(wkt),
            "export keeps the full value"
        );
        assert_eq!(text_capped("short", Some(PREVIEW_BYTES)), json!("short"));
        // A cut inside a multi-byte char backs off to the char start.
        assert_eq!(text_capped("ab日本", Some(3)), json!("ab…"));
        assert_eq!(
            mysql_value("LONGTEXT", wkt.as_bytes(), Some(PREVIEW_BYTES)),
            pg_text("TEXT", &wkt, Some(PREVIEW_BYTES))
        );
        assert_eq!(
            sqlite_value("TEXT", SqliteRaw::Text(&wkt), Some(PREVIEW_BYTES)),
            pg_text("TEXT", &wkt, Some(PREVIEW_BYTES))
        );
    }

    #[test]
    fn pg_scalars() {
        assert_eq!(pg_text("INT4", "-2147483648", None), json!(-2147483648_i64));
        assert_eq!(
            pg_text("INT8", "9007199254740993", None),
            json!("9007199254740993")
        );
        assert_eq!(pg_text("INT8", "42", None), json!(42));
        assert_eq!(pg_text("NUMERIC", "1234.5678", None), json!("1234.5678"));
        assert_eq!(pg_text("FLOAT8", "-1.5e+300", None), json!(-1.5e300));
        assert_eq!(pg_text("FLOAT8", "NaN", None), json!("NaN"));
        assert_eq!(pg_text("BOOL", "t", None), json!(true));
        assert_eq!(pg_text("BOOL", "f", None), json!(false));
        assert_eq!(
            pg_text("JSONB", r#"{"a": [1, 2]}"#, None),
            json!({"a": [1, 2]})
        );
        assert_eq!(pg_text("JSON", "not json", None), json!("not json"));
        assert_eq!(
            pg_text("BYTEA", "\\xdeadbeef", None),
            json!({"$bytes": "3q2+7w==", "len": 4})
        );
        assert_eq!(pg_text("?", "happy", None), json!("happy"));
        assert_eq!(
            pg_text("TIMESTAMPTZ", "2026-11-02 03:30:00+00", None),
            json!("2026-11-02 03:30:00+00")
        );
        assert_eq!(pg_text("TEXT", "🚀 café", None), json!("🚀 café"));
    }

    #[test]
    fn pg_bytea_rejects_bad_hex() {
        assert_eq!(pg_bytea("\\x00ff"), Some(vec![0, 255]));
        assert_eq!(pg_bytea("\\x0"), None);
        assert_eq!(pg_bytea("\\xzz"), None);
        assert_eq!(pg_bytea("abc"), None);
        assert_eq!(pg_text("BYTEA", "abc", None), json!("abc"));
    }

    #[test]
    fn pg_arrays() {
        assert_eq!(pg_text("INT4[]", "{1,2,3}", None), json!([1, 2, 3]));
        assert_eq!(pg_text("INT4[]", "{}", None), json!([]));
        assert_eq!(
            pg_text("TEXT[]", r#"{a,"b c",NULL,"NULL","x\"y\\z"}"#, None),
            json!(["a", "b c", null, "NULL", "x\"y\\z"])
        );
        assert_eq!(
            pg_text("INT8[]", "{{1,2},{3,4}}", None),
            json!([[1, 2], [3, 4]])
        );
        assert_eq!(pg_text("INT4[]", "[0:1]={7,8}", None), json!([7, 8]));
        assert_eq!(
            pg_text("BYTEA[]", r#"{"\\x00"}"#, None),
            json!([{"$bytes": "AA==", "len": 1}])
        );
        assert_eq!(pg_text("INT4[]", "{1,2", None), json!("{1,2"));
    }

    #[test]
    fn mysql_values() {
        assert_eq!(mysql_value("BOOLEAN", b"1", None), json!(true));
        assert_eq!(mysql_value("BOOLEAN", b"0", None), json!(false));
        assert_eq!(mysql_value("INT", b"-5", None), json!(-5));
        assert_eq!(
            mysql_value("BIGINT", b"-9223372036854775808", None),
            json!("-9223372036854775808")
        );
        assert_eq!(mysql_value("DECIMAL", b"-0.0001", None), json!("-0.0001"));
        assert_eq!(mysql_value("BIGINT", b"1", None), json!(1));
        assert_eq!(
            mysql_value("BLOB", &[0], None),
            json!({"$bytes": "AA==", "len": 1})
        );
        assert_eq!(
            mysql_value("VARBINARY", b"", None),
            json!({"$bytes": "", "len": 0})
        );
        // Binary-collated text (utf8mb4_bin, MariaDB JSON) reads as text.
        assert_eq!(
            mysql_value("LONGBLOB", "{\"a\": 1} 🚀\n".as_bytes(), None),
            json!("{\"a\": 1} 🚀\n")
        );
        assert_eq!(mysql_value("DOUBLE", b"2.5e-3", None), json!(0.0025));
        assert_eq!(mysql_value("JSON", br#"{"a": 1}"#, None), json!({"a": 1}));
        assert_eq!(
            mysql_value("BLOB", &[0xde, 0xad, 0xbe, 0xef], None),
            json!({"$bytes": "3q2+7w==", "len": 4})
        );
        assert_eq!(mysql_value("ENUM", b"happy", None), json!("happy"));
        assert_eq!(
            mysql_value("VARCHAR", "emoji 👍".as_bytes(), None),
            json!("emoji 👍")
        );
        assert_eq!(
            mysql_value("VARCHAR", &[0xff], None),
            json!({"$bytes": "/w==", "len": 1})
        );
        assert_eq!(mysql_value("WHATEVER", b"x", None), json!("x"));
        assert_eq!(
            mysql_value("WHATEVER", &[0xff], None),
            json!("<unsupported: WHATEVER>")
        );
    }

    #[test]
    fn sqlite_values() {
        assert_eq!(
            sqlite_value("BOOLEAN", SqliteRaw::Int(1), None),
            json!(true)
        );
        assert_eq!(sqlite_value("BOOLEAN", SqliteRaw::Int(7), None), json!(7));
        assert_eq!(
            sqlite_value("INTEGER", SqliteRaw::Int(9_007_199_254_740_993), None),
            json!("9007199254740993")
        );
        assert_eq!(sqlite_value("REAL", SqliteRaw::Real(0.5), None), json!(0.5));
        assert_eq!(
            sqlite_value("TEXT", SqliteRaw::Text("日本語"), None),
            json!("日本語")
        );
        assert_eq!(
            sqlite_value("BLOB", SqliteRaw::Blob(&[0]), None),
            json!({"$bytes": "AA==", "len": 1})
        );
        assert_eq!(sqlite_value("TEXT", SqliteRaw::Null, None), Value::Null);
    }
}
