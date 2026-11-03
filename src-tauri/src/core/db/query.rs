//! Runs editor SQL: one connection per run, statements in order, rows capped, cancellable.
//! Runs are auto-commit: a connection goes back to the pool only after a lone plain read, so
//! session state (open transaction, SET, USE, temp tables) never reaches the next run.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::mysql::{MySql, MySqlConnection, MySqlRow};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, PgRow, PgValueFormat, Postgres};
use sqlx::sqlite::{Sqlite, SqliteRow};
use sqlx::{Column, Connection as _, Database, Decode, Either, Executor, Row, TypeInfo, ValueRef};

use crate::core::conn_ro;
use crate::core::connections::{self, Engine, Env};
use crate::core::db::pool::DbPool;
use crate::core::db::session::{self, Conn};
use crate::core::db::value::{self, PREVIEW_BYTES, SqliteRaw};
use crate::core::error::{AppError, ErrorCode};
use crate::core::sql::{facts, split};
use crate::core::state::AppState;

/// Rows per result set unless the caller asks for more (same as the prototype).
pub const DEFAULT_LIMIT: usize = 200;
/// Upper bound for "Load more", so one result set cannot exhaust memory.
pub const MAX_LIMIT: usize = 100_000;
/// Added to a run that ended with a transaction still open; closing its connection rolled it back.
pub const OPEN_TX_NOTICE: &str =
    "Open transaction was rolled back. Runs are auto-commit; put BEGIN…COMMIT in one run.";
/// How long a cancel is remembered for a query that has not registered yet.
const EARLY_CANCEL_TTL: Duration = Duration::from_secs(30);
/// Bound for opening the extra connection that sends a cancel.
const CANCEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Bound for closing a connection whose session state must not be reused.
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub conn_id: String,
    /// The connection's own database when absent.
    #[serde(default)]
    pub database: Option<String>,
    pub sql: String,
    /// Chosen by the caller; `cancel_query` uses it.
    pub query_id: String,
    #[serde(default)]
    pub limit: Option<usize>,
    /// The editor tab the run belongs to. On a production connection the run then executes
    /// inside that tab's own session and open transaction (manual commit).
    #[serde(default)]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementResult {
    pub sql: String,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
    pub row_count: usize,
    /// More rows exist than `limit`. For a read they were not fetched; for a write that returns
    /// rows (RETURNING) the statement ran to the end and only the first `limit` rows are kept.
    pub truncated: bool,
    /// Rows changed, for statements that return no columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected: Option<u64>,
    /// A plain read (`facts::is_plain_read`): "Load more" and export may run it again.
    pub is_read: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRun {
    pub statements: Vec<StatementResult>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
    pub elapsed_ms: u64,
}

// ---- cancel registry ----

/// Raw SQLite handle, used only for `sqlite3_interrupt`.
#[derive(Debug, Clone, Copy)]
pub struct SqliteHandle(pub(crate) *mut libsqlite3_sys::sqlite3);

// SAFETY: the pointer is only passed to `sqlite3_interrupt`, which SQLite allows from any thread,
// and the registry clears it (under the same lock `cancel` holds) before the connection is released.
unsafe impl Send for SqliteHandle {}
unsafe impl Sync for SqliteHandle {}

/// The server session running a query, so another connection can stop it.
#[derive(Debug, Clone, Copy)]
pub enum Session {
    Pending,
    Pg(i32),
    MySql(i64),
    Sqlite(SqliteHandle),
}

struct Running {
    conn_id: String,
    database: String,
    cancelled: Arc<AtomicBool>,
    session: Session,
}

#[derive(Default)]
struct Registry {
    running: HashMap<String, Running>,
    /// Cancels that arrived before their run registered, by arrival time.
    early: HashMap<String, Instant>,
}

#[derive(Default)]
pub struct QueryRegistry {
    inner: Mutex<Registry>,
}

impl QueryRegistry {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_running(&self, query_id: &str) -> bool {
        self.lock().running.contains_key(query_id)
    }

    /// The returned guard's `cancelled` is already set when a cancel for `query_id` came first.
    fn register(
        &self,
        query_id: &str,
        conn_id: &str,
        database: &str,
    ) -> Result<RunGuard<'_>, AppError> {
        let mut reg = self.lock();
        if reg.running.contains_key(query_id) {
            return Err(AppError::invalid(format!(
                "Query {query_id} is already running"
            )));
        }
        let early = reg
            .early
            .remove(query_id)
            .is_some_and(|t| t.elapsed() < EARLY_CANCEL_TTL);
        let cancelled = Arc::new(AtomicBool::new(early));
        reg.running.insert(
            query_id.to_string(),
            Running {
                conn_id: conn_id.to_string(),
                database: database.to_string(),
                cancelled: cancelled.clone(),
                session: Session::Pending,
            },
        );
        Ok(RunGuard {
            reg: self,
            id: query_id.to_string(),
            cancelled,
        })
    }

    fn attach(&self, query_id: &str, session: Session) -> SessionGuard<'_> {
        if let Some(r) = self.lock().running.get_mut(query_id) {
            r.session = session;
        }
        SessionGuard {
            reg: self,
            id: query_id.to_string(),
        }
    }
}

/// Removes the run from the registry when the run ends or its future is dropped.
struct RunGuard<'a> {
    reg: &'a QueryRegistry,
    id: String,
    cancelled: Arc<AtomicBool>,
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.reg.lock().running.remove(&self.id);
    }
}

/// Clears the session so no cancel targets the connection any more. Drop it before reading
/// `cancelled` for the last time: a cancel that saw the session set the flag first.
struct SessionGuard<'a> {
    reg: &'a QueryRegistry,
    id: String,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if let Some(r) = self.reg.lock().running.get_mut(&self.id) {
            r.session = Session::Pending;
        }
    }
}

#[cfg(feature = "it")]
pub enum Kill {
    Pg(i32),
    MySql(i64),
}

#[cfg(not(feature = "it"))]
pub(crate) enum Kill {
    Pg(i32),
    MySql(i64),
}

/// Stops a running query. Unknown or finished ids are not an error: the query may have just ended,
/// or not registered yet, in which case it starts cancelled. Repeats the kill a few times while
/// the run is still registered, which covers a cancel that lands between two statements or before
/// the first one starts.
pub async fn cancel(state: &AppState, query_id: &str) -> Result<(), AppError> {
    for attempt in 0..4 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let target = {
            let mut reg = state.queries.lock();
            let Some(r) = reg.running.get(query_id) else {
                if attempt == 0 {
                    reg.early.retain(|_, t| t.elapsed() < EARLY_CANCEL_TTL);
                    reg.early.insert(query_id.to_string(), Instant::now());
                }
                return Ok(());
            };
            r.cancelled.store(true, Ordering::SeqCst);
            match r.session {
                Session::Pending => None,
                Session::Sqlite(h) => {
                    // SAFETY: see `SqliteHandle`; the map lock keeps the connection open here.
                    unsafe { libsqlite3_sys::sqlite3_interrupt(h.0) };
                    None
                }
                Session::Pg(pid) => Some((r.conn_id.clone(), r.database.clone(), Kill::Pg(pid))),
                Session::MySql(id) => {
                    Some((r.conn_id.clone(), r.database.clone(), Kill::MySql(id)))
                }
            }
        };
        let Some((conn_id, database, kill)) = target else {
            continue;
        };
        let sent = async {
            let pool = connections::pool_for(state, &conn_id, Some(&database))?;
            send_kill(pool, kill).await
        }
        .await;
        // A kill that races with the end of the query fails harmlessly (e.g. MySQL "unknown thread").
        if let Err(e) = sent
            && attempt == 0
            && state.queries.is_running(query_id)
        {
            return Err(e);
        }
    }
    Ok(())
}

/// Sends the kill on an idle pooled connection, or on a new one outside the pool, so a cancel
/// never queues behind the busy connections of a saturated pool.
pub(crate) async fn send_kill(pool: DbPool, kill: Kill) -> Result<(), AppError> {
    let timed_out = || {
        AppError::new(
            ErrorCode::Timeout,
            format!(
                "Could not reach the server to cancel within {} s",
                CANCEL_CONNECT_TIMEOUT.as_secs()
            ),
        )
    };
    match (pool, kill) {
        (DbPool::Pg(p), Kill::Pg(pid)) => {
            let q = sqlx::query("SELECT pg_cancel_backend($1)").bind(pid);
            match p.try_acquire() {
                Some(mut c) => {
                    q.execute(&mut *c).await?;
                }
                None => {
                    let opts = p.connect_options();
                    let mut c = tokio::time::timeout(
                        CANCEL_CONNECT_TIMEOUT,
                        PgConnection::connect_with(&opts),
                    )
                    .await
                    .map_err(|_| timed_out())??;
                    q.execute(&mut c).await?;
                    let _ = c.close().await;
                }
            }
        }
        (DbPool::MySql(p), Kill::MySql(id)) => {
            let sql = format!("KILL QUERY {id}");
            match p.try_acquire() {
                Some(mut c) => {
                    c.execute(sqlx::query(&sql)).await?;
                }
                None => {
                    let opts = p.connect_options();
                    let mut c = tokio::time::timeout(
                        CANCEL_CONNECT_TIMEOUT,
                        MySqlConnection::connect_with(&opts),
                    )
                    .await
                    .map_err(|_| timed_out())??;
                    c.execute(sqlx::query(&sql)).await?;
                    let _ = c.close().await;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ---- running ----

fn cancelled_error() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Query cancelled")
}

pub(crate) fn meta<C: Column>(c: &C) -> ColumnMeta {
    let t = c.type_info().name();
    ColumnMeta {
        name: c.name().to_string(),
        // sqlx names a PG type it has not looked up (e.g. an enum in a simple query) `?`.
        data_type: if t == "?" {
            "unknown".into()
        } else {
            t.to_string()
        },
    }
}

/// Statements that may return a result set even when it has no rows.
fn looks_like_read(sql: &str) -> bool {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix('#') {
            s = rest.split_once('\n').map_or("", |(_, r)| r).trim_start();
        } else if let Some(rest) = s.strip_prefix('(') {
            s = rest.trim_start();
        } else {
            break;
        }
    }
    let word: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        word.as_str(),
        "select"
            | "with"
            | "values"
            | "show"
            | "explain"
            | "describe"
            | "desc"
            | "pragma"
            | "table"
    )
}

type CellFn<DB> = fn(&<DB as Database>::Row, usize, Option<usize>) -> Value;
type AffectedFn<DB> = fn(&<DB as Database>::QueryResult) -> u64;

struct Outcome {
    results: Vec<StatementResult>,
    /// The last statement stopped reading mid-result; its connection must not be reused.
    cut_off: bool,
}

/// One result set (or command) of a piece of SQL.
#[derive(Default)]
struct ResultSet {
    columns: Option<Vec<ColumnMeta>>,
    rows: Vec<Vec<Value>>,
    truncated: bool,
    changed: u64,
}

/// Runs one piece of SQL and returns one result per result set: one for a split statement,
/// several for text the splitter could not split. The `bool` is true when a read stopped
/// mid-result at `limit` and left the connection unusable; `in_session` holds the run's
/// connection, so a session-mode read drains like a write instead (it never sets the flag).
async fn run_one<DB>(
    conn: &mut DB::Connection,
    sql: &str,
    read: bool,
    limit: usize,
    in_session: bool,
    cell: CellFn<DB>,
    affected: AffectedFn<DB>,
) -> Result<(Vec<StatementResult>, bool), sqlx::Error>
where
    DB: Database,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
    let started = Instant::now();
    let mut sets: Vec<ResultSet> = Vec::new();
    let mut cur = ResultSet::default();
    let mut cut_off = false;
    {
        let mut stream = sqlx::raw_sql(sql).fetch_many(&mut *conn);
        while let Some(item) = stream.try_next().await? {
            match item {
                // Every engine ends each result set or command with its own completion.
                Either::Left(done) => {
                    cur.changed += affected(&done);
                    sets.push(std::mem::take(&mut cur));
                }
                Either::Right(row) => {
                    if cur.columns.is_none() {
                        cur.columns = Some(row.columns().iter().map(meta).collect());
                    }
                    if cur.rows.len() >= limit {
                        cur.truncated = true;
                        // Dropping the stream of a write (RETURNING) would roll it back, and
                        // inside a tab session it would cost the open transaction the next
                        // statement or probe, so only an autocommit read stops early; the rest
                        // runs to the end, dropping the extra rows, and keeps `limit` of them.
                        if read && !in_session {
                            cut_off = true;
                            break;
                        }
                        continue;
                    }
                    cur.rows.push(
                        (0..row.len())
                            .map(|i| cell(&row, i, Some(PREVIEW_BYTES)))
                            .collect(),
                    );
                }
            }
        }
    }
    if cut_off || cur.columns.is_some() || sets.is_empty() {
        sets.push(cur);
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    // No rows came back, so no row described the columns; ask the server instead.
    let described = if let [set] = sets.as_slice()
        && set.columns.is_none()
        && looks_like_read(sql)
    {
        match (&mut *conn).describe(sql).await {
            Ok(d) => Some(d.columns().iter().map(meta).collect::<Vec<_>>()),
            Err(_) => None,
        }
    } else {
        None
    };
    let mut out = Vec::with_capacity(sets.len());
    for set in sets {
        let columns = set
            .columns
            .or_else(|| described.clone())
            .unwrap_or_default();
        out.push(StatementResult {
            sql: sql.to_string(),
            row_count: set.rows.len(),
            affected: columns.is_empty().then_some(set.changed),
            columns,
            rows: set.rows,
            truncated: set.truncated,
            is_read: read,
            elapsed_ms,
        });
    }
    Ok((out, cut_off))
}

async fn run_all<DB>(
    conn: &mut DB::Connection,
    stmts: &[String],
    reads: &[bool],
    limit: usize,
    cancelled: &AtomicBool,
    cell: CellFn<DB>,
    affected: AffectedFn<DB>,
) -> Result<Outcome, AppError>
where
    DB: Database,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
    let mut out = Outcome {
        results: Vec::with_capacity(stmts.len()),
        cut_off: false,
    };
    let n = stmts.len();
    for (i, sql) in stmts.iter().enumerate() {
        if cancelled.load(Ordering::SeqCst) {
            return Err(cancelled_error());
        }
        match run_one::<DB>(&mut *conn, sql, reads[i], limit, false, cell, affected).await {
            Ok((r, cut_off)) => {
                out.cut_off = cut_off;
                out.results.extend(r);
            }
            Err(_) if cancelled.load(Ordering::SeqCst) => return Err(cancelled_error()),
            Err(e) => {
                let mut e = AppError::from(e);
                if n > 1 {
                    e.message = format!("Statement {} of {n}: {}", i + 1, e.message);
                }
                return Err(e);
            }
        }
    }
    Ok(out)
}

/// Returns the connection to the pool, or closes it so its session dies with everything the run
/// left in it. Closing rolls back an open transaction on the server.
async fn release<DB: Database>(c: PoolConnection<DB>, reuse: bool) {
    if !reuse {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, c.detach().close()).await;
    }
}

/// Asks the server whether the session is inside an explicit transaction. `None` when it
/// cannot tell (error or timeout).
async fn pg_in_transaction(c: &mut PgConnection) -> Option<bool> {
    // Simple query protocol, so the probe is a statement of its own: outside a transaction it
    // starts one, and both clocks are equal; inside one, now() is the transaction start.
    // Through `Executor` (a boxed future): RawSql's generic async fns make `run` non-Send.
    let probe = c.fetch_one(sqlx::raw_sql("SELECT now() <> statement_timestamp()"));
    match tokio::time::timeout(CLOSE_TIMEOUT, probe).await.ok()? {
        Ok(row) => row.try_get::<bool, _>(0).ok(),
        // 25P02: a statement failed inside the transaction, which is still open.
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("25P02") => Some(true),
        Err(_) => None,
    }
}

/// MySQL/MariaDB version of [`pg_in_transaction`]. Changes the next transaction's
/// characteristics on MySQL, so call it only on a connection that is closed afterwards.
async fn mysql_in_transaction(c: &mut MySqlConnection) -> Option<bool> {
    tokio::time::timeout(CLOSE_TIMEOUT, mysql_probe(c))
        .await
        .ok()
        .flatten()
}

async fn mysql_probe(c: &mut MySqlConnection) -> Option<bool> {
    // MariaDB only: MySQL 8.4 has no such variable (error 1193).
    if let Ok(v) = sqlx::query_scalar::<_, i64>("SELECT CAST(@@in_transaction AS SIGNED)")
        .fetch_one(&mut *c)
        .await
    {
        return Some(v != 0);
    }
    // Fails with SQLSTATE 25001 only while a transaction is open.
    match c.execute(sqlx::raw_sql("SET TRANSACTION READ WRITE")).await {
        Ok(_) => Some(false),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("25001") => Some(true),
        Err(_) => None,
    }
}

/// (return the connection to the pool, ask the server whether a transaction is open).
/// A lone plain read cannot open a transaction; after a cut-off read the server is still
/// sending rows, so it is not asked either and the text heuristic stands in.
fn after_run(
    lone_read: bool,
    r: &Result<Outcome, AppError>,
    cancelled: &AtomicBool,
) -> (bool, bool) {
    let finished = r.as_ref().is_ok_and(|o| !o.cut_off) && !cancelled.load(Ordering::SeqCst);
    (lone_read && finished, !lone_read && finished)
}

pub(crate) fn pg_cell(row: &PgRow, i: usize, cap: Option<usize>) -> Value {
    let Ok(v) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if v.is_null() {
        return Value::Null;
    }
    let ty = v.type_info().name().to_string();
    match (v.format(), v.as_bytes()) {
        (PgValueFormat::Text, Ok(b)) => match std::str::from_utf8(b) {
            Ok(s) => value::pg_text(&ty, s, cap),
            Err(_) => value::bytes(b, cap),
        },
        // raw_sql uses the simple query protocol, which always answers in text.
        _ => value::unsupported(&ty),
    }
}

pub(crate) fn mysql_cell(row: &MySqlRow, i: usize, cap: Option<usize>) -> Value {
    let Ok(v) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if v.is_null() {
        return Value::Null;
    }
    let ty = v.type_info().name().to_string();
    match <&[u8] as Decode<MySql>>::decode(v) {
        Ok(b) => value::mysql_value(&ty, b, cap),
        Err(_) => value::unsupported(&ty),
    }
}

pub(crate) fn sqlite_cell(row: &SqliteRow, i: usize, cap: Option<usize>) -> Value {
    let declared = row
        .columns()
        .get(i)
        .map(|c| c.type_info().name().to_string())
        .unwrap_or_default();
    let Ok(v) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if v.is_null() {
        return Value::Null;
    }
    // A value's own type is its storage class (INTEGER, REAL, TEXT, BLOB).
    let storage = v.type_info().name().to_string();
    let decoded = match storage.as_str() {
        "INTEGER" | "BOOLEAN" => <i64 as Decode<Sqlite>>::decode(v)
            .map(|x| value::sqlite_value(&declared, SqliteRaw::Int(x), cap)),
        "REAL" => <f64 as Decode<Sqlite>>::decode(v)
            .map(|x| value::sqlite_value(&declared, SqliteRaw::Real(x), cap)),
        "BLOB" => <&[u8] as Decode<Sqlite>>::decode(v)
            .map(|x| value::sqlite_value(&declared, SqliteRaw::Blob(x), cap)),
        _ => <&str as Decode<Sqlite>>::decode(v)
            .map(|x| value::sqlite_value(&declared, SqliteRaw::Text(x), cap)),
    };
    decoded.unwrap_or_else(|_| match row.try_get_raw(i) {
        // Text that is not valid UTF-8 is shown as bytes.
        Ok(v) => <&[u8] as Decode<Sqlite>>::decode(v)
            .map(|b| value::bytes(b, cap))
            .unwrap_or_else(|_| value::unsupported(&declared)),
        Err(_) => value::unsupported(&declared),
    })
}

/// MySQL/MariaDB binary-protocol value (prepared statement) by the sqlx type name. Integers and
/// floats are little-endian; strings, blobs, decimals and BIT come as their content bytes;
/// DATE/TIME family is unpacked to ISO text.
pub(crate) fn mysql_cell_binary(row: &MySqlRow, i: usize, cap: Option<usize>) -> Value {
    let Ok(v) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if v.is_null() {
        return Value::Null;
    }
    let ty = v.type_info().name().to_string();
    let t = ty.to_ascii_uppercase();
    let Ok(raw) = <&[u8] as Decode<MySql>>::decode(v) else {
        return value::unsupported(&t);
    };
    match t.as_str() {
        "BOOLEAN" => return Value::Bool(raw.first() == Some(&1)),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
            // Little-endian, two's complement; sign-extend widths below 8 bytes.
            let u = raw.iter().rev().fold(0u64, |a, b| (a << 8) | u64::from(*b));
            let i = if raw.len() < 8 && raw.last().is_some_and(|b| b & 0x80 != 0) {
                (u | (u64::MAX << (raw.len() * 8))) as i64
            } else {
                u as i64
            };
            return value::int(i);
        }
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED"
        | "BIGINT UNSIGNED" => {
            let u = raw.iter().rev().fold(0u64, |a, b| (a << 8) | u64::from(*b));
            return match i64::try_from(u) {
                Ok(i) => value::int(i),
                Err(_) => Value::String(u.to_string()),
            };
        }
        "YEAR" if raw.len() == 2 => {
            return value::int(i64::from(u16::from_le_bytes([raw[0], raw[1]])));
        }
        "FLOAT" if raw.len() == 4 => {
            return value::float(f32::from_le_bytes(raw.try_into().unwrap_or([0; 4])) as f64);
        }
        "DOUBLE" if raw.len() == 8 => {
            return value::float(f64::from_le_bytes(raw.try_into().unwrap_or([0; 8])));
        }
        "DECIMAL" | "NUMERIC" => {
            return value::mysql_value("DECIMAL", raw, cap);
        }
        "DATE" | "DATETIME" | "TIMESTAMP" => {
            if let Some(s) = mysql_binary_datetime(raw) {
                return Value::String(s);
            }
        }
        "TIME" => {
            if let Some(s) = mysql_binary_time(raw) {
                return Value::String(s);
            }
        }
        _ => {}
    }
    // Strings, blobs, JSON, enums: the content bytes are the value.
    value::mysql_value(&t, raw, cap)
}

/// `YYYY-MM-DD[ HH:MM:SS[.ffffff]]` from the packed binary form (length byte first).
fn mysql_binary_datetime(raw: &[u8]) -> Option<String> {
    let (n, b) = (*raw.first()? as usize, &raw[1..]);
    if n == 0 || b.len() < 4 {
        return None;
    }
    let (y, mo, d) = (u16::from_le_bytes([b[0], b[1]]), b[2], b[3]);
    let mut s = format!("{y:04}-{mo:02}-{d:02}");
    if n >= 7 && b.len() >= 7 {
        s += &format!(" {:02}:{:02}:{:02}", b[4], b[5], b[6]);
        if n >= 11 && b.len() >= 11 {
            let us = u32::from_le_bytes([b[7], b[8], b[9], b[10]]);
            s += &format!(".{us:06}");
        }
    }
    Some(s)
}

/// `[-][d]HH:MM:SS[.ffffff]` from the packed binary TIME form.
fn mysql_binary_time(raw: &[u8]) -> Option<String> {
    let (n, b) = (*raw.first()? as usize, &raw[1..]);
    if n == 0 {
        return Some("00:00:00".to_string());
    }
    if n < 8 || b.len() < 8 {
        return None;
    }
    let neg = if b[0] == 1 { "-" } else { "" };
    let days = u32::from_le_bytes([b[1], b[2], b[3], b[4]]);
    let mut s = format!(
        "{neg}{:02}:{:02}:{:02}",
        days * 24 + u32::from(b[5]),
        b[6],
        b[7]
    );
    if n >= 12 && b.len() >= 12 {
        let us = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        s += &format!(".{us:06}");
    }
    Some(s)
}
fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Runs `req.sql` statement by statement on one connection, so `SET` and temp tables persist
/// within the run (not after it). No restrictions on the SQL: this is the user's own editor.
/// On a writable production connection with `tab_id`, the run executes inside the tab's
/// dedicated session and open transaction instead (manual commit).
pub async fn run(state: &AppState, req: RunRequest) -> Result<QueryRun, AppError> {
    run_inner(state, req, true).await
}

/// Same editor path without the read-only pre-check, so integration tests can prove that the
/// database itself refuses a write that got past it. Never compiled into a normal build.
#[cfg(feature = "it")]
pub async fn run_editor_unchecked(state: &AppState, req: RunRequest) -> Result<QueryRun, AppError> {
    run_inner(state, req, false).await
}

async fn run_inner(state: &AppState, req: RunRequest, check: bool) -> Result<QueryRun, AppError> {
    let conn = connections::get(state, &req.conn_id)?;
    let stmts = split::split(conn.engine, &req.sql);
    if stmts.is_empty() {
        return Err(AppError::invalid("Nothing to run"));
    }
    // Read-only connection: the session barrier is the real wall; this check gives the clear
    // error and refuses statements that could undo the barrier itself.
    if check && conn.read_only {
        conn_ro::check_read_only(&conn, &stmts)?;
    }
    let database = connections::target_database(&conn, req.database.as_deref());
    let reads: Vec<bool> = stmts
        .iter()
        .map(|s| facts::is_plain_read_sql(conn.engine, s))
        .collect();
    // Read-only connections never enter a tab session: there is nothing to commit, and the
    // frontend hides Commit / Rollback for them. Plain reads on a tab without a session need
    // none either: they take the pool like any autocommit read.
    if conn.env == Env::Production
        && !conn.read_only
        && let Some(tab_id) = req.tab_id.as_deref()
        && (state.sessions.has(tab_id) || reads.contains(&false))
    {
        return run_in_session(state, &conn, &database, tab_id, &req, &stmts, &reads).await;
    }
    let lone_read = reads == [true];
    let limit = clamp_limit(req.limit);
    let started = Instant::now();
    let run = state.queries.register(&req.query_id, &conn.id, &database)?;
    let cancelled = run.cancelled.clone();
    if cancelled.load(Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    let pool = connections::pool_for(state, &conn.id, Some(&database))?;
    let mut in_tx: Option<bool> = None;
    let outcome = match pool {
        DbPool::Pg(p) => {
            let mut c = p.acquire().await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *c)
                .await?;
            let session = state.queries.attach(&req.query_id, Session::Pg(pid));
            let r = run_all::<Postgres>(&mut c, &stmts, &reads, limit, &cancelled, pg_cell, |d| {
                d.rows_affected()
            })
            .await;
            drop(session);
            let (reuse, ask) = after_run(lone_read, &r, &cancelled);
            if ask {
                in_tx = pg_in_transaction(&mut c).await;
            }
            release(c, reuse).await;
            r
        }
        DbPool::MySql(p) => {
            let mut c = p.acquire().await?;
            let id: i64 = sqlx::query_scalar("SELECT CAST(CONNECTION_ID() AS SIGNED)")
                .fetch_one(&mut *c)
                .await?;
            let session = state.queries.attach(&req.query_id, Session::MySql(id));
            let r = run_all::<MySql>(&mut c, &stmts, &reads, limit, &cancelled, mysql_cell, |d| {
                d.rows_affected()
            })
            .await;
            drop(session);
            let (reuse, ask) = after_run(lone_read, &r, &cancelled);
            if ask {
                in_tx = mysql_in_transaction(&mut c).await;
            }
            release(c, reuse).await;
            r
        }
        DbPool::Sqlite(p) => {
            let mut c = p.acquire().await?;
            let handle = {
                let mut locked = c.lock_handle().await?;
                SqliteHandle(locked.as_raw_handle().as_ptr())
            };
            let session = state.queries.attach(&req.query_id, Session::Sqlite(handle));
            let r = run_all::<Sqlite>(
                &mut c,
                &stmts,
                &reads,
                limit,
                &cancelled,
                sqlite_cell,
                |d| d.rows_affected(),
            )
            .await;
            drop(session);
            // sqlx only tracks transactions it started itself, so ask SQLite.
            in_tx = match c.lock_handle().await {
                // SAFETY: the locked handle gives exclusive access to the open connection.
                Ok(mut h) => Some(unsafe {
                    libsqlite3_sys::sqlite3_get_autocommit(h.as_raw_handle().as_ptr()) == 0
                }),
                Err(_) => None,
            };
            let (reuse, _) = after_run(lone_read, &r, &cancelled);
            release(c, reuse && in_tx == Some(false)).await;
            r
        }
    }?;
    // MySQL's KILL QUERY can end SLEEP() normally instead of with an error.
    if conn.engine == Engine::Mysql && cancelled.load(Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    drop(run);
    // The text heuristic only stands in when the server could not be asked.
    let open_tx = in_tx.unwrap_or_else(|| split::leaves_transaction_open(conn.engine, &req.sql));
    Ok(QueryRun {
        statements: outcome.results,
        notices: if open_tx {
            vec![OPEN_TX_NOTICE.to_string()]
        } else {
            Vec::new()
        },
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Errors whose connection cannot stay in the session: it is dead or unusable.
fn is_lost(e: &AppError) -> bool {
    matches!(
        e.code,
        ErrorCode::ConnectFailed | ErrorCode::Timeout | ErrorCode::Internal
    ) || {
        let d = e.detail.as_deref().unwrap_or("");
        d.contains("connection closed") || d.contains("broken pipe") || d.contains("pool is closed")
    }
}

/// Production manual-commit path: run `stmts` inside the tab session's explicit transaction.
/// The session outlives the run while that transaction is open, for Commit / Rollback and the
/// next run.
///
/// Plain reads run as they are; every other statement runs inside the transaction, which is
/// opened before it when none is open. Statements that may end the transaction (a typed
/// COMMIT, MySQL's implicit commits) are followed by a probe, so a statement after them opens
/// a new one instead of autocommitting.
async fn run_in_session(
    state: &AppState,
    conn: &connections::Connection,
    database: &str,
    tab_id: &str,
    req: &RunRequest,
    stmts: &[String],
    reads: &[bool],
) -> Result<QueryRun, AppError> {
    let kinds: Vec<Option<facts::StatementKind>> = stmts
        .iter()
        .map(|s| {
            facts::analyze(conn.engine, s)
                .ok()
                .and_then(|f| f.statements.first().map(|s| s.kind))
        })
        .collect();
    let mysql = conn.engine == Engine::Mysql;
    let limit = clamp_limit(req.limit);
    let started = Instant::now();
    let run = state.queries.register(&req.query_id, &conn.id, database)?;
    let cancelled = run.cancelled.clone();
    if cancelled.load(Ordering::SeqCst) {
        return Err(cancelled_error());
    }
    let mut s = session::for_tab(state, conn, database, tab_id).await?;
    let mut notices = Vec::new();
    let r = async {
        let _session_guard = match &mut s.conn {
            Conn::Pg(c) => {
                let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(&mut *c)
                    .await?;
                state.queries.attach(&req.query_id, Session::Pg(pid))
            }
            Conn::MySql(c) => {
                let id: i64 = sqlx::query_scalar("SELECT CAST(CONNECTION_ID() AS SIGNED)")
                    .fetch_one(&mut *c)
                    .await?;
                state.queries.attach(&req.query_id, Session::MySql(id))
            }
            Conn::Sqlite(c) => {
                let handle = {
                    let mut locked = c.lock_handle().await?;
                    SqliteHandle(locked.as_raw_handle().as_ptr())
                };
                state.queries.attach(&req.query_id, Session::Sqlite(handle))
            }
        };
        let n = stmts.len();
        let mut results = Vec::with_capacity(n);
        for (i, sql) in stmts.iter().enumerate() {
            if cancelled.load(Ordering::SeqCst) {
                return Err(cancelled_error());
            }
            let kind = kinds[i];
            let probe = !reads[i] && session::may_end_tx(kind);
            // MySQL: the earlier uncommitted work an implicit commit would take along.
            let mut swept = false;
            if !reads[i] {
                if !s.tx_open {
                    session::begin(&mut s).await?;
                }
                if mysql && probe && !matches!(kind, Some(facts::StatementKind::Transaction)) {
                    swept = session::pending(&mut s).await;
                }
                if session::may_write(kind) {
                    s.dirty = true;
                }
            }
            match exec(&mut s.conn, sql, reads[i], limit).await {
                Ok(r) => results.extend(r),
                Err(_) if cancelled.load(Ordering::SeqCst) => return Err(cancelled_error()),
                Err(e) => {
                    let mut e = AppError::from(e);
                    if n > 1 {
                        e.message = format!("Statement {} of {n}: {}", i + 1, e.message);
                    }
                    return Err(e);
                }
            }
            if probe {
                session::refresh(&mut s).await;
                if mysql && !s.tx_open && !matches!(kind, Some(facts::StatementKind::Transaction)) {
                    notices.push(session::implicit_commit_notice(kind, swept, i, n));
                }
            }
        }
        Ok(results)
    }
    .await;
    // MySQL's KILL QUERY can end SLEEP() normally instead of with an error.
    let r = if mysql && cancelled.load(Ordering::SeqCst) {
        r.and(Err(cancelled_error()))
    } else {
        r
    };
    session::refresh(&mut s).await;
    let r = match r {
        Ok(statements) => Ok(QueryRun {
            statements,
            notices,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }),
        Err(mut e) => {
            // The transaction aborts with the failed statement: the user must see it, and the
            // session stays open so Rollback is still available.
            if s.aborted {
                e.message = format!("{} Transaction aborted, roll back to continue.", e.message);
            }
            // An implicit commit before the failure already happened; the error must say so.
            if !notices.is_empty() {
                e.message = format!("{} ({})", e.message, notices.join("; "));
            }
            Err(e)
        }
    };
    if matches!(&r, Err(e) if is_lost(e)) {
        state.sessions.discard(tab_id, Some(s)).await;
    } else {
        session::end_if_clean(&mut s).await;
        session::settle(state, tab_id, s).await;
    }
    drop(run);
    r
}

/// One statement on the session's connection. Reads inside a session drain past `limit`
/// like writes (`in_session`): dropping the stream mid-result would leave the server
/// sending rows and cost the open transaction the next statement or probe.
async fn exec(
    c: &mut Conn,
    sql: &str,
    read: bool,
    limit: usize,
) -> Result<Vec<StatementResult>, sqlx::Error> {
    Ok(match c {
        Conn::Pg(c) => {
            run_one::<Postgres>(c, sql, read, limit, true, pg_cell, |d| d.rows_affected())
                .await?
                .0
        }
        Conn::MySql(c) => {
            run_one::<MySql>(c, sql, read, limit, true, mysql_cell, |d| d.rows_affected())
                .await?
                .0
        }
        Conn::Sqlite(c) => {
            run_one::<Sqlite>(c, sql, read, limit, true, sqlite_cell, |d| {
                d.rows_affected()
            })
            .await?
            .0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_detection_skips_comments_and_parens() {
        assert!(looks_like_read("SELECT 1"));
        assert!(looks_like_read("  -- hi\n/* x */ (select 1)"));
        assert!(looks_like_read("# mysql comment\nSHOW TABLES"));
        assert!(looks_like_read("with x as (select 1) select * from x"));
        assert!(looks_like_read("PRAGMA table_info(t)"));
        assert!(!looks_like_read("UPDATE t SET a = 1"));
        assert!(!looks_like_read("create table t (id int)"));
        assert!(!looks_like_read("-- only a comment"));
    }

    #[test]
    fn limit_defaults_and_bounds() {
        assert_eq!(clamp_limit(None), 200);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(20_000)), 20_000);
        assert_eq!(clamp_limit(Some(usize::MAX)), MAX_LIMIT);
    }

    #[test]
    fn registry_rejects_duplicate_ids_and_cleans_up() {
        let reg = QueryRegistry::default();
        {
            let _g = reg.register("q1", "c", "db").unwrap();
            assert!(reg.is_running("q1"));
            assert_eq!(
                reg.register("q1", "c", "db").err().unwrap().code,
                ErrorCode::InvalidInput
            );
            {
                let _s = reg.attach("q1", Session::Pg(42));
                assert!(matches!(reg.lock().running["q1"].session, Session::Pg(42)));
            }
            assert!(
                matches!(reg.lock().running["q1"].session, Session::Pending),
                "the session is cleared before the connection is released"
            );
        }
        assert!(!reg.is_running("q1"));
    }

    #[test]
    fn cancel_before_register_starts_the_run_cancelled() {
        let reg = QueryRegistry::default();
        reg.lock().early.insert("q1".into(), Instant::now());
        let g = reg.register("q1", "c", "db").unwrap();
        assert!(g.cancelled.load(Ordering::SeqCst));
        drop(g);
        let g = reg.register("q1", "c", "db").unwrap();
        assert!(
            !g.cancelled.load(Ordering::SeqCst),
            "an early cancel is used once"
        );
        drop(g);
        reg.lock()
            .early
            .insert("q2".into(), Instant::now() - EARLY_CANCEL_TTL);
        assert!(
            !reg.register("q2", "c", "db")
                .unwrap()
                .cancelled
                .load(Ordering::SeqCst)
        );
    }

    #[test]
    fn result_json_shape_is_camel_case() {
        let r = QueryRun {
            statements: vec![StatementResult {
                sql: "SELECT 1".into(),
                columns: vec![ColumnMeta {
                    name: "a".into(),
                    data_type: "INT4".into(),
                }],
                rows: vec![vec![Value::from(1)]],
                row_count: 1,
                truncated: false,
                affected: None,
                is_read: true,
                elapsed_ms: 3,
            }],
            notices: vec![],
            elapsed_ms: 4,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"statements": [{"sql": "SELECT 1", "columns": [{"name": "a", "dataType": "INT4"}],
                "rows": [[1]], "rowCount": 1, "truncated": false, "isRead": true, "elapsedMs": 3}], "elapsedMs": 4})
        );
    }
}
