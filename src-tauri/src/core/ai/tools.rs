//! The tools the model can call. Reads go through the AI read path (guard, read-only session,
//! limits); `propose_write` never runs SQL, it only shows the statement to the user.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::core::ai::session::ChatEvent;
use crate::core::ai::settings::{AiSettings, Privacy};
use crate::core::conn_ro;
use crate::core::connections::{Connection, Engine};
use crate::core::db::introspect;
use crate::core::db::readonly::{self, AiReadResult, ReadLimits};
use crate::core::error::{AppError, ErrorCode};
use crate::core::guard;
use crate::core::state::AppState;

/// Largest tool output sent to the model; longer output is cut with a marker.
pub const MAX_CONTENT_BYTES: usize = 32 * 1024;
/// Rows of a query result sent to the UI.
pub const UI_ROWS: usize = 50;
const CUT_MARKER: &str = "\n[cut: the tool output was longer than 32 KiB]";

/// OpenAI function tool definitions.
pub fn definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "list_tables",
                "description": "List the tables and views of the database.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "schema": {"type": "string", "description": "PostgreSQL schema. Default: the current schema. Ignored on MySQL and SQLite."}
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "describe_table",
                "description": "List the columns of one table or view: name, type, nullable, default, primary key.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "table": {"type": "string", "description": "Table name without schema."},
                        "schema": {"type": "string", "description": "PostgreSQL schema. Default: the current schema."}
                    },
                    "required": ["table"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_read_query",
                "description": "Run one read-only SQL statement (SELECT, SHOW, DESCRIBE, EXPLAIN, PRAGMA) and get the result. Writes are refused.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": {"type": "string", "description": "Exactly one read-only statement."}
                    },
                    "required": ["sql"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "propose_write",
                "description": "Propose SQL that changes data or schema. It does not run: the user sees it and decides.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": {"type": "string", "description": "The SQL to propose."}
                    },
                    "required": ["sql"]
                }
            }
        }
    ])
}

/// The arguments as an object; `None` when the text is not a JSON object.
pub fn parse_args(raw: &str) -> Option<Map<String, Value>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(Map::new());
    }
    match serde_json::from_str(raw) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// What one chat runs against.
pub struct ToolContext<'a> {
    pub state: &'a AppState,
    pub conn: &'a Connection,
    pub database: &'a str,
    /// PG schema for `list_tables` and `describe_table` when the model gives none.
    pub schema: Option<&'a str>,
    pub settings: &'a AiSettings,
    /// The chat's write mode is on.
    pub allow_writes: bool,
}

/// The result of one tool call.
pub struct ToolOutput {
    /// For the UI, in order: a `write_preview` when there is one, then the `tool_result`.
    pub events: Vec<ChatEvent>,
    /// The `tool` message for the model, at most `MAX_CONTENT_BYTES`.
    pub content: String,
}

/// UI part of a successful call.
struct Done {
    summary: String,
    columns: Option<Vec<String>>,
    rows: Option<Vec<Vec<Value>>>,
    truncated: Option<bool>,
    preview: Option<ChatEvent>,
    content: Value,
}

impl Done {
    fn plain(summary: String, content: Value) -> Self {
        Self {
            summary,
            columns: None,
            rows: None,
            truncated: None,
            preview: None,
            content,
        }
    }
}

pub async fn execute(ctx: &ToolContext<'_>, id: &str, name: &str, raw_args: &str) -> ToolOutput {
    let res = match parse_args(raw_args) {
        Some(args) => run(ctx, id, name, &args).await,
        None => Err(AppError::invalid(
            "The tool arguments are not a JSON object",
        )),
    };
    match res {
        Ok(d) => {
            let mut events = Vec::with_capacity(2);
            events.extend(d.preview);
            events.push(ChatEvent::ToolResult {
                id: id.to_string(),
                ok: true,
                summary: d.summary,
                columns: d.columns,
                rows: d.rows,
                truncated: d.truncated,
                error: None,
            });
            ToolOutput {
                events,
                content: cap(d.content.to_string()),
            }
        }
        Err(e) => {
            let hide = name == "run_read_query" && ctx.settings.privacy == Privacy::Schema;
            let err = model_error(&e, hide);
            ToolOutput {
                events: vec![ChatEvent::ToolResult {
                    id: id.to_string(),
                    ok: false,
                    summary: e.message.clone(),
                    columns: None,
                    rows: None,
                    truncated: None,
                    error: Some(e),
                }],
                content: cap(json!({ "error": err }).to_string()),
            }
        }
    }
}

/// Error codes whose message is our own fixed text, never the server's, so it cannot quote a
/// row value.
const OWN_MESSAGE_CODES: [ErrorCode; 7] = [
    ErrorCode::GuardRejected,
    ErrorCode::Timeout,
    ErrorCode::Cancelled,
    ErrorCode::ReadOnlyViolation,
    ErrorCode::ConnectionReadOnly,
    ErrorCode::InvalidInput,
    ErrorCode::NotFound,
];

/// The model's view of a failed call. Database messages are kept so the model can fix its SQL.
/// `hide` (a read in `schema` privacy): a server message can quote a row value
/// (`invalid input syntax for type integer: "alice@example.com"`), so only our own messages
/// pass; the rest become the server's error code. The detail is left out.
fn model_error(e: &AppError, hide: bool) -> Value {
    if !hide {
        let mut err = json!({"code": e.code, "message": e.message});
        if let Some(d) = &e.detail {
            err["detail"] = json!(d);
        }
        return err;
    }
    let message = if OWN_MESSAGE_CODES.contains(&e.code) {
        e.message.clone()
    } else {
        let hidden = "The error text is hidden by the privacy setting.";
        match &e.server_code {
            Some(c) => format!("The database refused the query ({c}). {hidden}"),
            None => format!("The query failed. {hidden}"),
        }
    };
    json!({"code": e.code, "message": message})
}

async fn run(
    ctx: &ToolContext<'_>,
    id: &str,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    match name {
        "list_tables" => list_tables(ctx, args).await,
        "describe_table" => describe_table(ctx, args).await,
        "run_read_query" => run_read_query(ctx, args).await,
        "propose_write" => propose_write(ctx, id, args),
        _ => Err(AppError::not_found(format!("Unknown tool {name}"))),
    }
}

fn opt_str<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn req_str<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, AppError> {
    opt_str(args, key).ok_or_else(|| AppError::invalid(format!("The tool needs \"{key}\"")))
}

/// `n` and the noun, singular for 1.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn schema_of<'a>(ctx: &'a ToolContext<'a>, args: &'a Map<String, Value>) -> Option<&'a str> {
    if ctx.conn.engine != Engine::Pg {
        return None;
    }
    opt_str(args, "schema").or(ctx.schema)
}

async fn list_tables(ctx: &ToolContext<'_>, args: &Map<String, Value>) -> Result<Done, AppError> {
    let schema = schema_of(ctx, args);
    let tables = introspect::list_tables(ctx.state, &ctx.conn.id, ctx.database, schema).await?;
    let mut content = json!({ "tables": tables });
    if ctx.conn.engine == Engine::Pg {
        content["schema"] = json!(schema.unwrap_or("public"));
    }
    Ok(Done::plain(count(tables.len(), "table", "tables"), content))
}

async fn describe_table(
    ctx: &ToolContext<'_>,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let table = req_str(args, "table")?;
    let schema = schema_of(ctx, args);
    let columns =
        introspect::list_columns(ctx.state, &ctx.conn.id, ctx.database, schema, table).await?;
    Ok(Done::plain(
        count(columns.len(), "column", "columns"),
        json!({ "table": table, "columns": columns }),
    ))
}

async fn run_read_query(
    ctx: &ToolContext<'_>,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let sql = req_str(args, "sql")?;
    let limits = ReadLimits {
        max_rows: ctx.settings.max_rows as usize,
        timeout: Duration::from_secs(ctx.settings.timeout_sec),
    };
    let r = readonly::run_ai_read(ctx.state, &ctx.conn.id, Some(ctx.database), sql, limits).await?;
    let n = r.rows.len();
    let more = if r.truncated {
        " (row limit reached)"
    } else {
        ""
    };
    let summary = format!("{}{more} · {} ms", count(n, "row", "rows"), r.elapsed_ms);
    let content = read_content(&r, ctx.settings.privacy);
    let columns: Vec<String> = r.columns.into_iter().map(|c| c.name).collect();
    let ui_truncated = r.truncated || n > UI_ROWS;
    let mut rows = r.rows;
    rows.truncate(UI_ROWS);
    Ok(Done {
        summary,
        columns: Some(columns),
        rows: Some(rows),
        truncated: Some(ui_truncated),
        preview: None,
        content,
    })
}

/// The model's view of a read. `schema` privacy: names and counts only, never a row value.
fn read_content(r: &AiReadResult, privacy: Privacy) -> Value {
    let columns: Vec<Value> = r
        .columns
        .iter()
        .map(|c| json!({"name": c.name, "type": c.data_type}))
        .collect();
    let mut out = json!({
        "columns": columns,
        "rowCount": r.rows.len(),
        "truncated": r.truncated,
    });
    if privacy == Privacy::Schema {
        out["note"] = json!("Row values are hidden by the privacy setting.");
        return out;
    }
    // Whole rows only, so the JSON stays valid; the cap leaves room for the other fields.
    let budget = MAX_CONTENT_BYTES.saturating_sub(out.to_string().len() + 200);
    let mut used = 0;
    let mut rows = Vec::new();
    for row in &r.rows {
        let len = serde_json::to_string(row).map_or(0, |s| s.len()) + 1;
        if used + len > budget {
            break;
        }
        used += len;
        rows.push(row.clone());
    }
    if rows.len() < r.rows.len() {
        out["note"] = json!(format!(
            "Only the first {} rows fit in the 32 KiB tool output limit.",
            rows.len()
        ));
    }
    out["rows"] = Value::from(rows);
    out
}

fn propose_write(
    ctx: &ToolContext<'_>,
    id: &str,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let sql = req_str(args, "sql")?;
    if !conn_ro::writes_allowed(ctx.conn) {
        return Err(AppError::new(
            ErrorCode::WriteBlocked,
            format!(
                "Connection \"{}\" is read-only, so the assistant cannot propose writes",
                ctx.conn.name
            ),
        ));
    }
    if !ctx.allow_writes {
        return Err(AppError::new(
            ErrorCode::WriteBlocked,
            "Write mode is off, so the assistant cannot propose writes",
        ));
    }
    let info = guard::classify_write(sql, ctx.conn.engine);
    let mut done = Done::plain(
        "Waiting for your approval".into(),
        json!({
            "status": "pending_approval",
            "message": "The SQL has not run. The user sees it and decides. Do not say that it ran.",
        }),
    );
    done.preview = Some(ChatEvent::WritePreview {
        id: id.to_string(),
        sql: sql.to_string(),
        info,
    });
    Ok(done)
}

/// Cuts `s` to `MAX_CONTENT_BYTES` on a char boundary and adds a marker.
fn cap(mut s: String) -> String {
    if s.len() <= MAX_CONTENT_BYTES {
        return s;
    }
    let mut end = MAX_CONTENT_BYTES - CUT_MARKER.len();
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str(CUT_MARKER);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db::query::ColumnMeta;

    fn read(rows: Vec<Vec<Value>>) -> AiReadResult {
        AiReadResult {
            columns: vec![ColumnMeta {
                name: "email".into(),
                data_type: "TEXT".into(),
            }],
            rows,
            truncated: false,
            elapsed_ms: 3,
        }
    }

    #[test]
    fn schema_privacy_never_has_values() {
        let r = read(vec![vec![json!("alice@example.com")]]);
        let v = read_content(&r, Privacy::Schema);
        assert!(!v.to_string().contains("alice"), "{v}");
        assert_eq!(v["rowCount"], 1);
        assert_eq!(v["columns"][0]["name"], "email");
        let v = read_content(&r, Privacy::Results);
        assert_eq!(v["rows"][0][0], "alice@example.com");
    }

    #[test]
    fn results_are_cut_by_whole_rows() {
        let rows = (0..2000)
            .map(|i| vec![json!(format!("value-{i:05}-padding"))])
            .collect();
        let v = read_content(&read(rows), Privacy::Results);
        let s = v.to_string();
        assert!(s.len() <= MAX_CONTENT_BYTES, "{}", s.len());
        let shown = v["rows"].as_array().unwrap().len();
        assert!(shown > 100 && shown < 2000, "{shown}");
        assert_eq!(v["rowCount"], 2000);
        assert!(
            v["note"]
                .as_str()
                .unwrap()
                .contains(&format!("first {shown} rows"))
        );
    }

    #[test]
    fn cap_marks_the_cut() {
        let s = "é".repeat(MAX_CONTENT_BYTES);
        let c = cap(s);
        assert!(c.len() <= MAX_CONTENT_BYTES);
        assert!(c.ends_with(CUT_MARKER));
        assert_eq!(cap("short".into()), "short");
    }

    #[test]
    fn args_parse() {
        assert_eq!(parse_args("").unwrap().len(), 0);
        assert_eq!(
            parse_args(r#"{"sql": "SELECT 1"}"#).unwrap()["sql"],
            "SELECT 1"
        );
        assert!(parse_args("[1]").is_none());
        assert!(parse_args("{bad").is_none());
    }

    #[test]
    fn definitions_name_four_tools() {
        let names: Vec<String> = definitions()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            [
                "list_tables",
                "describe_table",
                "run_read_query",
                "propose_write"
            ]
        );
    }

    #[test]
    fn schema_privacy_hides_server_error_text() {
        let mut server = AppError::new(
            ErrorCode::Database,
            "invalid input syntax for type integer: \"alice@example.com\"",
        )
        .with_detail("error returned from database: alice@example.com");
        server.server_code = Some("SQLSTATE 22P02".into());
        let v = model_error(&server, true);
        assert!(!v.to_string().contains("alice"), "{v}");
        assert_eq!(v["code"], "database");
        assert!(
            v["message"].as_str().unwrap().contains("(SQLSTATE 22P02)"),
            "{v}"
        );
        server.server_code = None;
        let v = model_error(&server, true);
        assert!(!v.to_string().contains("alice"), "{v}");

        // Our own messages pass, without the detail.
        let guard = AppError::new(ErrorCode::GuardRejected, "Blocked: DELETE is not a read")
            .with_detail("guard detail");
        assert_eq!(
            model_error(&guard, true),
            json!({"code": "guard_rejected", "message": "Blocked: DELETE is not a read"})
        );

        // `results` privacy keeps everything so the model can fix its SQL.
        let v = model_error(&server, false);
        assert!(v["message"].as_str().unwrap().contains("alice"), "{v}");
        assert!(v["detail"].as_str().unwrap().contains("alice"), "{v}");
    }
}
