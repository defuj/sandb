//! The tools the model can call. Reads go through the AI read path (guard, read-only session,
//! limits); `render_chart` reads the same way and sends the rows to the UI as a chart;
//! `propose_write` never runs SQL, it only shows the statement to the user.

use std::time::Duration;

use serde::Serialize;
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
/// Rows a chart keeps. Charts are saved with the chat, so this stays small.
pub const CHART_ROWS: usize = 1000;
const CHART_TYPES: [&str; 5] = ["bar", "line", "area", "pie", "scatter"];
/// Where `open_query_tab` puts the result: a new tab, the user's active tab, or the best match.
const TAB_TARGETS: [&str; 3] = ["new", "current", "best"];
/// Most value columns of one chart.
const MAX_SERIES: usize = 8;
const CUT_MARKER: &str = "\n[cut: the tool output was longer than 32 KiB]";

/// OpenAI function tool definitions. Every tool takes `source`, one of `ids`; it is required
/// when the chat has more than one source.
pub fn definitions(ids: &[&str]) -> Value {
    let mut defs = base_definitions();
    let source = json!({
        "type": "string",
        "enum": ids,
        "description": "Id of the database to use, from the system prompt (for example db1).",
    });
    for d in defs.as_array_mut().into_iter().flatten() {
        let params = &mut d["function"]["parameters"];
        params["properties"]["source"] = source.clone();
        if ids.len() > 1 {
            match params["required"].as_array_mut() {
                Some(r) => r.push(json!("source")),
                None => params["required"] = json!(["source"]),
            }
        }
    }
    defs
}

fn base_definitions() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "list_tables",
                "description": "List the tables and views of one database.",
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
                "name": "render_chart",
                "description": "Run one read-only SQL statement and show its result to the user as a chart in the chat. This is the only way to show a chart: call it every time the user asks for a chart, graph or plot. The chart is also in the PDF export of the reply. Shape the data in SQL: one row per point, sorted, already aggregated.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": {"type": "string", "description": "Exactly one read-only statement."},
                        "type": {"type": "string", "enum": CHART_TYPES, "description": "bar, line, area, pie or scatter."},
                        "title": {"type": "string", "description": "Short chart title."},
                        "x": {"type": "string", "description": "Result column for the x axis (pie: the slice labels)."},
                        "y": {"type": "array", "items": {"type": "string"}, "description": "Result columns with the numeric values, one series each (pie: exactly one)."},
                        "series": {"type": "string", "description": "Optional result column that splits one y column into one series per distinct value (long format). Not for pie."},
                        "stacked": {"type": "boolean", "description": "Stack the series (bar and area)."}
                    },
                    "required": ["sql", "type", "x", "y"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "open_query_tab",
                "description": "Run one read-only SQL statement and show its result in an editor tab, so the user can sort, filter, page and export it. Use it when the user asks to see the data in a tab or to run a query in a tab, and when a result table is the answer. It never changes data.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": {"type": "string", "description": "Exactly one read-only statement."},
                        "title": {"type": "string", "description": "Short tab name in the user's language, 2 to 5 words. Describe the request, not the SQL."},
                        "target": {"type": "string", "enum": TAB_TARGETS, "description": "new: a new tab; current: the user's active tab when it is safe; best: the tab that already shows this result."}
                    },
                    "required": ["sql", "title"]
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

/// One database of the chat. The model names it by `id`.
#[derive(Debug)]
pub struct Source {
    /// `db1`, `db2`, … in the order of the request.
    pub id: String,
    pub conn: Connection,
    pub database: String,
    /// PG schema for `list_tables` and `describe_table` when the model gives none.
    pub schema: Option<String>,
    /// The chat's write mode for this source is on.
    pub allow_writes: bool,
}

/// What one chat runs against.
pub struct ToolContext<'a> {
    pub state: &'a AppState,
    /// At least one.
    pub sources: &'a [Source],
    pub settings: &'a AiSettings,
}

impl ToolContext<'_> {
    /// The source that `args` names. Without `source`, the only one; with more, an error.
    fn source(&self, args: &Map<String, Value>) -> Result<&Source, AppError> {
        let ids = || {
            self.sources
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match opt_str(args, "source") {
            Some(id) => self.sources.iter().find(|s| s.id == id).ok_or_else(|| {
                AppError::invalid(format!(
                    "Unknown source {}. Use one of: {}",
                    quoted(id),
                    ids()
                ))
            }),
            None if self.sources.len() == 1 => Ok(&self.sources[0]),
            None => Err(AppError::invalid(format!(
                "The tool needs \"source\", one of: {}",
                ids()
            ))),
        }
    }
}

/// The result of one tool call.
pub struct ToolOutput {
    /// For the UI, in order: a `write_preview` or `chart` when there is one, then the `tool_result`.
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
    /// An event sent before the `tool_result`: a `write_preview` or a `chart`.
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
            let read =
                name == "run_read_query" || name == "render_chart" || name == "open_query_tab";
            let hide = read && ctx.settings.privacy == Privacy::Schema;
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
        "render_chart" => render_chart(ctx, id, args).await,
        "open_query_tab" => open_query_tab(ctx, id, args).await,
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

fn schema_of<'a>(src: &'a Source, args: &'a Map<String, Value>) -> Option<&'a str> {
    if src.conn.engine != Engine::Pg {
        return None;
    }
    opt_str(args, "schema").or(src.schema.as_deref())
}

async fn list_tables(ctx: &ToolContext<'_>, args: &Map<String, Value>) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let schema = schema_of(src, args);
    let tables = introspect::list_tables(ctx.state, &src.conn.id, &src.database, schema).await?;
    let mut content = json!({ "tables": tables });
    if src.conn.engine == Engine::Pg {
        content["schema"] = json!(schema.unwrap_or("public"));
    }
    Ok(Done::plain(count(tables.len(), "table", "tables"), content))
}

async fn describe_table(
    ctx: &ToolContext<'_>,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let table = req_str(args, "table")?;
    let schema = schema_of(src, args);
    let columns =
        introspect::list_columns(ctx.state, &src.conn.id, &src.database, schema, table).await?;
    Ok(Done::plain(
        count(columns.len(), "column", "columns"),
        json!({ "table": table, "columns": columns }),
    ))
}

async fn run_read_query(
    ctx: &ToolContext<'_>,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let sql = req_str(args, "sql")?;
    let r = readonly::run_ai_read(
        ctx.state,
        &src.conn.id,
        Some(&src.database),
        sql,
        limits(ctx),
    )
    .await?;
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

/// A tab title from the model: one line, at most 48 characters.
fn tab_title(raw: &str) -> Result<String, AppError> {
    let title = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let title: String = title.chars().filter(|c| !c.is_control()).collect();
    if title.is_empty() {
        return Err(AppError::invalid("The tool needs a title for the tab"));
    }
    Ok(if title.chars().count() > 48 {
        let mut cut: String = title.chars().take(47).collect();
        cut.push('…');
        cut
    } else {
        title
    })
}

/// Runs one read and hands the rows to the UI with `tab_open`; the chat step stays compact.
async fn open_query_tab(
    ctx: &ToolContext<'_>,
    id: &str,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let sql = req_str(args, "sql")?;
    let title = tab_title(req_str(args, "title")?)?;
    let target = opt_str(args, "target").unwrap_or("new");
    if !TAB_TARGETS.contains(&target) {
        return Err(AppError::invalid(format!(
            "The tab target must be one of: {}",
            TAB_TARGETS.join(", ")
        )));
    }
    let r = readonly::run_ai_read(
        ctx.state,
        &src.conn.id,
        Some(&src.database),
        sql,
        limits(ctx),
    )
    .await?;
    let columns: Vec<String> = r.columns.iter().map(|c| c.name.clone()).collect();
    let types: Vec<String> = r.columns.iter().map(|c| c.data_type.clone()).collect();
    let n = r.rows.len();
    let summary = format!("{} · tab {}", count(n, "row", "rows"), quoted(&title));
    let mut content = read_content(&r, ctx.settings.privacy);
    content["shownInTab"] = json!(title);
    content["note"] = json!(
        "The user sees this result in an editor tab. Do not repeat the rows in your reply; summarize them."
    );
    Ok(Done {
        summary,
        columns: None,
        rows: None,
        truncated: None,
        preview: Some(ChatEvent::TabOpen {
            id: id.to_string(),
            title,
            sql: sql.to_string(),
            conn_id: src.conn.id.clone(),
            database: src.database.clone(),
            target: target.to_string(),
            columns,
            types,
            rows: r.rows,
            truncated: r.truncated,
            limit: ctx.settings.max_rows as usize,
            elapsed_ms: r.elapsed_ms,
        }),
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

fn limits(ctx: &ToolContext<'_>) -> ReadLimits {
    ReadLimits {
        max_rows: ctx.settings.max_rows as usize,
        timeout: Duration::from_secs(ctx.settings.timeout_sec),
    }
}

/// What the UI draws. Every column name is a column of the result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub x: String,
    pub y: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    pub stacked: bool,
}

/// The chart arguments. Checked before the query runs; the column names are checked after it.
fn chart_spec(args: &Map<String, Value>) -> Result<ChartSpec, AppError> {
    let kind = req_str(args, "type")?;
    if !CHART_TYPES.contains(&kind) {
        return Err(AppError::invalid(format!(
            "\"type\" must be one of: {}",
            CHART_TYPES.join(", ")
        )));
    }
    let x = req_str(args, "x")?.to_string();
    // Some models send one column as a plain string.
    let y: Vec<String> = match args.get("y") {
        Some(Value::String(s)) => vec![s.trim().to_string()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .collect(),
        _ => Vec::new(),
    };
    if y.is_empty() || y.iter().any(String::is_empty) {
        return Err(AppError::invalid(
            "The tool needs \"y\": a list of value column names",
        ));
    }
    if y.len() > MAX_SERIES {
        return Err(AppError::invalid(format!(
            "A chart takes at most {MAX_SERIES} y columns"
        )));
    }
    let series = opt_str(args, "series").map(str::to_string);
    if kind == "pie" && (y.len() != 1 || series.is_some()) {
        return Err(AppError::invalid(
            "A pie chart takes exactly one y column and no series",
        ));
    }
    if series.is_some() && y.len() != 1 {
        return Err(AppError::invalid(
            "With \"series\", give exactly one y column",
        ));
    }
    Ok(ChartSpec {
        kind: kind.to_string(),
        title: opt_str(args, "title").map(str::to_string),
        x,
        y,
        series,
        stacked: args.get("stacked").and_then(Value::as_bool) == Some(true),
    })
}

/// Every column the chart names is in the result. The error lists the result's columns, so the
/// model can fix its arguments; names are schema, never row values.
fn check_columns(spec: &ChartSpec, columns: &[String]) -> Result<(), AppError> {
    let missing: Vec<String> = std::iter::once(&spec.x)
        .chain(&spec.y)
        .chain(&spec.series)
        .filter(|c| !columns.contains(c))
        .map(|c| quoted(c))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let have: Vec<String> = columns.iter().map(|c| quoted(c)).collect();
    Err(AppError::invalid(format!(
        "The result has no column {}. Its columns: {}",
        missing.join(", "),
        have.join(", ")
    )))
}

fn quoted(name: &str) -> String {
    Value::from(name).to_string()
}

async fn render_chart(
    ctx: &ToolContext<'_>,
    id: &str,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let spec = chart_spec(args)?;
    let sql = req_str(args, "sql")?;
    let r = readonly::run_ai_read(
        ctx.state,
        &src.conn.id,
        Some(&src.database),
        sql,
        limits(ctx),
    )
    .await?;
    let columns: Vec<String> = r.columns.iter().map(|c| c.name.clone()).collect();
    check_columns(&spec, &columns)?;
    let n = r.rows.len();
    let more = if r.truncated {
        " (row limit reached)"
    } else {
        ""
    };
    let mut kind = spec.kind.clone();
    kind[..1].make_ascii_uppercase();
    let summary = format!(
        "{kind} chart · {}{more} · {} ms",
        count(n, "row", "rows"),
        r.elapsed_ms
    );
    let mut content = read_content(&r, ctx.settings.privacy);
    content["chart"] = json!(if n > CHART_ROWS {
        format!("Shown to the user. It draws the first {CHART_ROWS} rows.")
    } else {
        "Shown to the user.".to_string()
    });
    let truncated = r.truncated || n > CHART_ROWS;
    let mut rows = r.rows;
    rows.truncate(CHART_ROWS);
    let mut done = Done::plain(summary, content);
    done.preview = Some(ChatEvent::Chart {
        id: id.to_string(),
        spec,
        columns,
        rows,
        truncated,
    });
    Ok(done)
}

fn propose_write(
    ctx: &ToolContext<'_>,
    id: &str,
    args: &Map<String, Value>,
) -> Result<Done, AppError> {
    let src = ctx.source(args)?;
    let sql = req_str(args, "sql")?;
    if !conn_ro::writes_allowed(&src.conn) {
        return Err(AppError::new(
            ErrorCode::WriteBlocked,
            format!(
                "Connection \"{}\" is read-only, so the assistant cannot propose writes",
                src.conn.name
            ),
        ));
    }
    if !src.allow_writes {
        return Err(AppError::new(
            ErrorCode::WriteBlocked,
            format!(
                "Write mode is off on \"{}\", so the assistant cannot propose writes",
                src.conn.name
            ),
        ));
    }
    let info = guard::classify_write(sql, src.conn.engine);
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
    fn definitions_name_six_tools() {
        let names: Vec<String> = definitions(&["db1"])
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
                "render_chart",
                "open_query_tab",
                "propose_write"
            ]
        );
    }

    /// One source: `source` is optional, so a model that never names it still works. More
    /// sources: every tool must name one of them.
    #[test]
    fn source_is_required_only_with_several_sources() {
        for d in definitions(&["db1"]).as_array().unwrap() {
            let p = &d["function"]["parameters"];
            assert_eq!(p["properties"]["source"]["enum"], json!(["db1"]));
            let required = p["required"].as_array().cloned().unwrap_or_default();
            assert!(!required.contains(&json!("source")), "{d}");
        }
        for d in definitions(&["db1", "db2"]).as_array().unwrap() {
            let p = &d["function"]["parameters"];
            assert_eq!(p["properties"]["source"]["enum"], json!(["db1", "db2"]));
            assert!(
                p["required"].as_array().unwrap().contains(&json!("source")),
                "{d}"
            );
        }
        let describe = &definitions(&["db1", "db2"])[1]["function"]["parameters"]["required"];
        assert_eq!(describe, &json!(["table", "source"]));
    }

    fn source(id: &str, name: &str) -> Source {
        use crate::core::connections::{Env, SslMode};
        Source {
            id: id.into(),
            conn: Connection {
                id: format!("c-{id}"),
                name: name.into(),
                engine: Engine::Pg,
                version: String::new(),
                env: Env::Local,
                host: "h".into(),
                port: 5432,
                database: "shop".into(),
                user: "u".into(),
                ssl: SslMode::Prefer,
                file: String::new(),
                ai_write: false,
                read_only: false,
            },
            database: "shop".into(),
            schema: None,
            allow_writes: false,
        }
    }

    #[test]
    fn tools_run_on_the_named_source() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path(),
            Box::new(crate::core::secrets::MemoryStore::new()),
        )
        .unwrap();
        let settings = AiSettings::default();
        let args = |v: Value| chart_args(v);

        let one = [source("db1", "a")];
        let ctx = ToolContext {
            state: &state,
            sources: &one,
            settings: &settings,
        };
        assert_eq!(ctx.source(&args(json!({}))).unwrap().id, "db1");

        let two = [source("db1", "a"), source("db2", "b")];
        let ctx = ToolContext {
            state: &state,
            sources: &two,
            settings: &settings,
        };
        assert_eq!(
            ctx.source(&args(json!({"source": "db2"})))
                .unwrap()
                .conn
                .name,
            "b"
        );
        // A guess must not land on the first database.
        let e = ctx.source(&args(json!({}))).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert_eq!(e.message, "The tool needs \"source\", one of: db1, db2");
        let e = ctx.source(&args(json!({"source": "db3"}))).unwrap_err();
        assert_eq!(e.message, "Unknown source \"db3\". Use one of: db1, db2");
    }

    #[tokio::test]
    async fn write_mode_is_per_source() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(
            dir.path(),
            Box::new(crate::core::secrets::MemoryStore::new()),
        )
        .unwrap();
        let settings = AiSettings::default();
        let mut writable = source("db2", "staging");
        writable.allow_writes = true;
        let sources = [source("db1", "prod"), writable];
        let ctx = ToolContext {
            state: &state,
            sources: &sources,
            settings: &settings,
        };
        let sql = r#"{"sql": "DELETE FROM t", "source": "db1"}"#;
        let out = execute(&ctx, "w1", "propose_write", sql).await;
        assert!(matches!(
            &out.events[..],
            [ChatEvent::ToolResult { ok: false, error: Some(e), .. }]
                if e.code == ErrorCode::WriteBlocked && e.message.contains("\"prod\"")
        ));
        let sql = r#"{"sql": "DELETE FROM t", "source": "db2"}"#;
        let out = execute(&ctx, "w2", "propose_write", sql).await;
        assert!(matches!(
            &out.events[..],
            [
                ChatEvent::WritePreview { .. },
                ChatEvent::ToolResult { ok: true, .. }
            ]
        ));
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

    fn chart_args(v: Value) -> Map<String, Value> {
        let Value::Object(m) = v else { unreachable!() };
        m
    }

    #[test]
    fn chart_spec_takes_valid_arguments() {
        let s = chart_spec(&chart_args(json!({
            "sql": "SELECT 1", "type": "bar", "x": "month", "y": ["revenue", "orders"],
            "title": "Sales", "stacked": true
        })))
        .unwrap();
        assert_eq!(s.y, ["revenue", "orders"]);
        assert!(s.stacked);
        // A model that sends one column as a string still gets its chart.
        let s = chart_spec(&chart_args(
            json!({"type": "line", "x": "day", "y": "total"}),
        ))
        .unwrap();
        assert_eq!(s.y, ["total"]);
        assert!(!s.stacked);
        // The UI reads `type`, not the Rust field name.
        assert_eq!(serde_json::to_value(&s).unwrap()["type"], "line");
    }

    #[test]
    fn chart_spec_refuses_charts_the_ui_cannot_draw() {
        for (args, want) in [
            (json!({"type": "radar", "x": "a", "y": ["b"]}), "\"type\""),
            (json!({"type": "bar", "x": "a", "y": []}), "\"y\""),
            (json!({"type": "bar", "x": "a", "y": [""]}), "\"y\""),
            (json!({"type": "pie", "x": "a", "y": ["b", "c"]}), "pie"),
            (
                json!({"type": "pie", "x": "a", "y": ["b"], "series": "c"}),
                "pie",
            ),
            (
                json!({"type": "line", "x": "a", "y": ["b", "c"], "series": "d"}),
                "series",
            ),
            (
                json!({"type": "bar", "x": "a", "y": ["1","2","3","4","5","6","7","8","9"]}),
                "at most 8",
            ),
        ] {
            let e = chart_spec(&chart_args(args.clone())).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidInput, "{args}");
            assert!(e.message.contains(want), "{args}: {}", e.message);
        }
    }

    #[test]
    fn missing_chart_column_names_the_result_columns() {
        let spec = chart_spec(&chart_args(
            json!({"type": "bar", "x": "month", "y": ["revenue"], "series": "region"}),
        ))
        .unwrap();
        let cols = [
            "month".to_string(),
            "total".to_string(),
            "region".to_string(),
        ];
        let e = check_columns(&spec, &cols).unwrap_err();
        // Own message, so it reaches the model under `schema` privacy and the model can fix it.
        assert!(OWN_MESSAGE_CODES.contains(&e.code));
        assert_eq!(
            e.message,
            "The result has no column \"revenue\". Its columns: \"month\", \"total\", \"region\""
        );
        let cols = [
            "month".to_string(),
            "revenue".to_string(),
            "region".to_string(),
        ];
        assert!(check_columns(&spec, &cols).is_ok());
    }
}
