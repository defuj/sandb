//! CSV export: re-runs one plain-read statement without a row cap and streams it to a file.

use std::path::{Path, PathBuf};

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Column, Database, Executor, Row};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};

use crate::core::connections::{self, Env};
use crate::core::db::pool::DbPool;
use crate::core::db::query::{mysql_cell, pg_cell, sqlite_cell};
use crate::core::db::session::{self, Conn};
use crate::core::error::AppError;
use crate::core::sql::{facts, split};
use crate::core::state::AppState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub conn_id: String,
    #[serde(default)]
    pub database: Option<String>,
    pub sql: String,
    /// Production editor tab: the export reads inside the tab's open transaction.
    #[serde(default)]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// The user closed the save dialog; nothing was written.
    pub cancelled: bool,
    pub rows: u64,
    pub path: String,
}
/// RFC 4180: quote when the field has a comma, quote, CR or LF; double inner quotes.
pub fn escape(field: &str) -> std::borrow::Cow<'_, str> {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\"")).into()
    } else {
        field.into()
    }
}

/// NULL → empty, bytes → base64, JSON values → compact JSON, the rest → their text.
pub fn field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Object(o) => match o.get("$bytes") {
            Some(Value::String(b64)) if o.contains_key("len") => b64.clone(),
            _ => v.to_string(),
        },
        Value::Array(_) => v.to_string(),
    }
}

pub fn line<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let mut s = fields.into_iter().map(escape).collect::<Vec<_>>().join(",");
    s.push_str("\r\n");
    s
}

type CellFn<DB> = fn(&<DB as Database>::Row, usize, Option<usize>) -> Value;

/// `left` is set when the stream was dropped mid-result (a write to `out` failed): the
/// server is still sending rows, so the connection cannot be reused — a session holding it
/// must close.
async fn stream_rows<DB>(
    conn: &mut DB::Connection,
    sql: &str,
    out: &mut (impl AsyncWrite + Unpin),
    cell: CellFn<DB>,
    left: &mut bool,
) -> Result<u64, AppError>
where
    DB: Database,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
    let mut n = 0u64;
    {
        let mut stream = sqlx::raw_sql(sql).fetch(&mut *conn);
        while let Some(row) = stream.try_next().await? {
            if n == 0 {
                let names: Vec<&str> = row.columns().iter().map(|c| c.name()).collect();
                if let Err(e) = out.write_all(line(names).as_bytes()).await {
                    *left = true;
                    return Err(e.into());
                }
            }
            let fields: Vec<String> = (0..row.len())
                .map(|i| field(&cell(&row, i, None)))
                .collect();
            let bytes = line(fields.iter().map(String::as_str));
            if let Err(e) = out.write_all(bytes.as_bytes()).await {
                *left = true;
                return Err(e.into());
            }
            n += 1;
        }
    }
    if n == 0 {
        // No row carried the column names, so ask the server. Some reads (e.g. SHOW) cannot be
        // described; the file is then empty.
        if let Ok(d) = (&mut *conn).describe(sql).await {
            let names: Vec<&str> = d.columns().iter().map(|c| c.name()).collect();
            if !names.is_empty() {
                out.write_all(line(names).as_bytes()).await?;
            }
        }
    }
    Ok(n)
}
/// Rejects anything but one plain read, so the export never repeats a write. Call it before
/// asking the user for a file.
pub fn check(state: &AppState, req: &ExportRequest) -> Result<(), AppError> {
    let conn = connections::get(state, &req.conn_id)?;
    match split::split(conn.engine, &req.sql).as_slice() {
        [sql] if facts::is_plain_read_sql(conn.engine, sql) => Ok(()),
        _ => Err(AppError::invalid(
            "CSV export needs exactly one read-only statement (a SELECT)",
        )),
    }
}

/// Hidden sibling of `path`, so the final rename stays on one file system.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.part", uuid::Uuid::new_v4()))
}

/// Streams the result to a temp file next to `path`, then renames it over `path`. On failure
/// only the temp file is removed; an existing file at `path` is never touched.
pub async fn export_csv(
    state: &AppState,
    req: ExportRequest,
    path: &Path,
) -> Result<ExportResult, AppError> {
    check(state, &req)?;
    if !path.is_absolute() {
        return Err(AppError::invalid("Choose a file to export to"));
    }
    let conn = connections::get(state, &req.conn_id)?;
    let sql = split::split(conn.engine, &req.sql).remove(0);
    let tmp = temp_path(path);
    let file = tokio::fs::File::create(&tmp).await?;
    let mut out = BufWriter::new(file);
    // A production tab exports what its open transaction sees: the same session the run used.
    // Without one (no open transaction, or a session on another target) the pool's view is
    // the same committed data. Read-only connections never hold a session.
    let database = connections::target_database(&conn, req.database.as_deref());
    let mut session = None;
    if conn.env == Env::Production
        && !conn.read_only
        && let Some(tab_id) = req.tab_id.as_deref()
        && let Some(s) = state.sessions.take(tab_id)?
    {
        if s.conn_id == conn.id && s.database == database {
            session = Some((tab_id.to_string(), s));
        } else {
            state.sessions.put(tab_id, s).await;
        }
    }
    // Set when the export dropped its row stream mid-result (write to the file failed): the
    // server is still sending rows, so a session connection cannot be probed or kept.
    let mut left = false;
    let res = async {
        let n = match &mut session {
            Some((_, s)) => match &mut s.conn {
                Conn::Pg(c) => {
                    stream_rows::<sqlx::Postgres>(&mut *c, &sql, &mut out, pg_cell, &mut left)
                        .await?
                }
                Conn::MySql(c) => {
                    stream_rows::<sqlx::MySql>(&mut *c, &sql, &mut out, mysql_cell, &mut left)
                        .await?
                }
                Conn::Sqlite(c) => {
                    stream_rows::<sqlx::Sqlite>(&mut *c, &sql, &mut out, sqlite_cell, &mut left)
                        .await?
                }
            },
            None => {
                let pool = connections::pool_for(state, &conn.id, req.database.as_deref())?;
                match pool {
                    DbPool::Pg(p) => {
                        let mut c = p.acquire().await?;
                        stream_rows::<sqlx::Postgres>(&mut c, &sql, &mut out, pg_cell, &mut left)
                            .await?
                    }
                    DbPool::MySql(p) => {
                        let mut c = p.acquire().await?;
                        stream_rows::<sqlx::MySql>(&mut c, &sql, &mut out, mysql_cell, &mut left)
                            .await?
                    }
                    DbPool::Sqlite(p) => {
                        let mut c = p.acquire().await?;
                        stream_rows::<sqlx::Sqlite>(&mut c, &sql, &mut out, sqlite_cell, &mut left)
                            .await?
                    }
                }
            }
        };
        out.flush().await?;
        out.get_ref().sync_all().await?;
        Ok::<_, AppError>(n)
    }
    .await;
    let mut res = res;
    if let Some((tab_id, mut s)) = session {
        if left {
            // As in `run_in_session`: probing first would drain the unfinished result behind
            // the probe's timeout. Close the session (rolling its transaction back) and say so.
            if s.tx_open
                && let Err(e) = &mut res
            {
                e.message = format!(
                    "{} The open transaction was rolled back; its uncommitted changes are lost.",
                    e.message
                );
            }
            state.sessions.discard(&tab_id, Some(s)).await;
        } else {
            session::refresh(&mut s).await;
            session::settle(state, &tab_id, s).await;
        }
    }
    drop(out);
    let res = match res {
        Ok(rows) => tokio::fs::rename(&tmp, path)
            .await
            .map(|()| rows)
            .map_err(AppError::from),
        Err(e) => Err(e),
    };
    match res {
        Ok(rows) => Ok(ExportResult {
            cancelled: false,
            rows,
            path: path.display().to_string(),
        }),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc4180_quoting() {
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape("a,b"), "\"a,b\"");
        assert_eq!(escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(escape("two\nlines"), "\"two\nlines\"");
        assert_eq!(escape("cr\r"), "\"cr\r\"");
        assert_eq!(escape(""), "");
        assert_eq!(line(["a", "b,c", ""]), "a,\"b,c\",\r\n");
    }

    #[test]
    fn fields_by_value_kind() {
        assert_eq!(field(&Value::Null), "");
        assert_eq!(field(&json!("日本語 🚀")), "日本語 🚀");
        assert_eq!(field(&json!(true)), "true");
        assert_eq!(field(&json!(-1.5)), "-1.5");
        assert_eq!(field(&json!({"$bytes": "3q2+7w==", "len": 4})), "3q2+7w==");
        assert_eq!(field(&json!({"a": [1, 2]})), r#"{"a":[1,2]}"#);
        assert_eq!(field(&json!([1, null])), "[1,null]");
    }
}
