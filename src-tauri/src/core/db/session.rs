//! Phase 3b, manual commit on Production: each editor tab on a production connection gets its
//! own dedicated session — one connection, detached from the pool, inside an explicit
//! transaction the user commits or rolls back. The session exists only while that
//! transaction is open: it opens with the first statement of a run that is not a plain read,
//! and closes on Commit / Rollback, when a run ends without pending changes, when the tab
//! closes or its target changes, or when the app exits. There is no timer.

use std::collections::HashMap;
use std::ptr;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use sqlx::mysql::{MySql, MySqlConnection};
use sqlx::postgres::{PgConnection, Postgres};
use sqlx::sqlite::{Sqlite, SqliteConnection};
use sqlx::{Connection as _, Executor};

use crate::core::connections::{self, Connection};
use crate::core::db::pool::DbPool;
use crate::core::db::query::CLOSE_TIMEOUT;
use crate::core::error::{AppError, ErrorCode};
use crate::core::sql::facts::StatementKind;
use crate::core::state::AppState;

/// Bound for a commit / rollback / status probe on the session; the connection dies after it.
const TX_TIMEOUT: Duration = Duration::from_secs(10);

/// The session's own connection, detached from the pool: an open transaction may hold it for
/// as long as the user takes to decide, and must not count against the pool's connections
/// (a 6th tab would stall every run on that database otherwise). It never returns to the pool.
pub enum Conn {
    Pg(PgConnection),
    MySql(MySqlConnection),
    Sqlite(SqliteConnection),
}

/// One tab's dedicated session. `dirty` tracks "a statement that could have changed data ran":
/// the pending probes (`pg_current_xact_id_if_assigned`, `innodb_trx`) only see write
/// transactions, so the flag gates them. SQLite asks `sqlite3_txn_state` directly.
pub struct TabSession {
    pub conn_id: String,
    pub database: String,
    pub conn: Conn,
    /// The session is inside the explicit transaction (verified after each statement that
    /// could end it, so MySQL implicit commits and user-typed COMMITs are seen).
    pub tx_open: bool,
    /// The transaction ran a statement that could have changed data.
    pub dirty: bool,
    /// PG only: a statement failed inside the transaction, which is now aborted (25P02).
    pub aborted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabStatus {
    /// The tab has a dedicated session on this target.
    pub open: bool,
    /// An explicit transaction is open.
    pub tx_open: bool,
    /// The transaction holds uncommitted changes.
    pub pending: bool,
    /// PG: the transaction aborted; only ROLLBACK helps.
    pub aborted: bool,
}

enum Slot {
    Live(TabSession),
    /// A command holds the session, or is opening one for `conn_id`: two commands never use
    /// one at once. `closed`: the tab closed or its connection changed meanwhile, so the
    /// session dies when the command hands it back instead of returning to the map.
    InUse {
        conn_id: String,
        closed: bool,
    },
}

#[derive(Default)]
struct Sessions {
    tabs: HashMap<String, Slot>,
}

/// Tab sessions keyed by tab id. A command takes the session out of its slot and leaves an
/// `InUse` marker, so a second run on the same tab fails fast; every take ends in `put` or
/// `discard`, which clear the marker.
#[derive(Default)]
pub struct SessionRegistry {
    inner: Mutex<Sessions>,
}

fn in_use() -> AppError {
    AppError::invalid("The tab's transaction is in use by another run")
}

impl SessionRegistry {
    fn lock(&self) -> MutexGuard<'_, Sessions> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The tab's session, `None` when it has none, or `InvalidInput` while another command
    /// holds it (e.g. a still-running query on the same tab). A returned session must go
    /// back through `put` or `discard`.
    pub fn take(&self, tab_id: &str) -> Result<Option<TabSession>, AppError> {
        self.take_inner(tab_id, None)
    }

    /// Like `take`, but reserves the slot for a session the caller opens on `conn_id` when
    /// none exists, so two concurrent runs cannot both open one. The reservation ends in
    /// `put` or `discard` too.
    fn take_or_reserve(&self, tab_id: &str, conn_id: &str) -> Result<Option<TabSession>, AppError> {
        self.take_inner(tab_id, Some(conn_id))
    }

    fn take_inner(
        &self,
        tab_id: &str,
        reserve: Option<&str>,
    ) -> Result<Option<TabSession>, AppError> {
        let mut g = self.lock();
        match g.tabs.remove(tab_id) {
            Some(Slot::Live(s)) => {
                let conn_id = reserve.unwrap_or(&s.conn_id).to_string();
                g.tabs.insert(
                    tab_id.to_string(),
                    Slot::InUse {
                        conn_id,
                        closed: false,
                    },
                );
                Ok(Some(s))
            }
            Some(busy) => {
                g.tabs.insert(tab_id.to_string(), busy);
                Err(in_use())
            }
            None => {
                if let Some(conn_id) = reserve {
                    let conn_id = conn_id.to_string();
                    g.tabs.insert(
                        tab_id.to_string(),
                        Slot::InUse {
                            conn_id,
                            closed: false,
                        },
                    );
                }
                Ok(None)
            }
        }
    }

    /// The tab has a session, live or in use.
    pub fn has(&self, tab_id: &str) -> bool {
        self.lock().tabs.contains_key(tab_id)
    }

    /// Hands a taken session back. When the tab was closed meanwhile the session is closed
    /// instead; a session the slot somehow still held is closed, never returned to a pool
    /// mid-transaction.
    pub async fn put(&self, tab_id: &str, s: TabSession) {
        let dead = {
            let mut g = self.lock();
            match g.tabs.remove(tab_id) {
                Some(Slot::InUse { closed: false, .. }) => {
                    g.tabs.insert(tab_id.to_string(), Slot::Live(s));
                    None
                }
                Some(Slot::Live(old)) => {
                    g.tabs.insert(tab_id.to_string(), Slot::Live(s));
                    Some(old)
                }
                Some(Slot::InUse { closed: true, .. }) | None => Some(s),
            }
        };
        if let Some(s) = dead {
            close_session(s).await;
        }
    }

    /// Ends a take without handing a session back: clears the `InUse` marker and closes `s`
    /// (its transaction rolls back).
    pub async fn discard(&self, tab_id: &str, s: Option<TabSession>) {
        {
            let mut g = self.lock();
            if matches!(g.tabs.get(tab_id), Some(Slot::InUse { .. })) {
                g.tabs.remove(tab_id);
            }
        }
        if let Some(s) = s {
            close_session(s).await;
        }
    }

    /// Drops the tab's session (its transaction rolls back when the connection dies). A
    /// session in use is marked, so it dies when its command hands it back.
    pub async fn close(&self, tab_id: &str) {
        let s = {
            let mut g = self.lock();
            match g.tabs.get_mut(tab_id) {
                Some(Slot::InUse { closed, .. }) => {
                    *closed = true;
                    None
                }
                Some(Slot::Live(_)) => match g.tabs.remove(tab_id) {
                    Some(Slot::Live(s)) => Some(s),
                    _ => None,
                },
                None => None,
            }
        };
        if let Some(s) = s {
            close_session(s).await;
        }
    }

    /// Drops every session of a saved connection (edit or delete); sessions in use die when
    /// their command hands them back.
    pub async fn close_conn(&self, conn_id: &str) {
        let dead: Vec<TabSession> = {
            let mut g = self.lock();
            let ids: Vec<String> = g
                .tabs
                .iter_mut()
                .filter_map(|(k, v)| match v {
                    Slot::Live(s) if s.conn_id == conn_id => Some(k.clone()),
                    Slot::InUse { conn_id: c, closed } if c == conn_id => {
                        *closed = true;
                        None
                    }
                    _ => None,
                })
                .collect();
            ids.iter()
                .filter_map(|id| match g.tabs.remove(id) {
                    Some(Slot::Live(s)) => Some(s),
                    _ => None,
                })
                .collect()
        };
        for s in dead {
            close_session(s).await;
        }
    }

    /// Tabs with a session holding uncommitted work; the quit intercept reads it. A session
    /// in use by a run counts: the run may be writing right now.
    pub fn pending_tab_ids(&self) -> Vec<String> {
        self.lock()
            .tabs
            .iter()
            .filter(|(_, v)| match v {
                Slot::Live(s) => s.tx_open && (s.dirty || s.aborted),
                Slot::InUse { closed, .. } => !closed,
            })
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// Closes the session's connection, rolling back whatever transaction it held.
pub async fn close_session(s: TabSession) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, close_conn(s.conn)).await;
}

async fn close_conn(c: Conn) -> Result<(), sqlx::Error> {
    match c {
        Conn::Pg(c) => c.close().await,
        Conn::MySql(c) => c.close().await,
        Conn::Sqlite(c) => c.close().await,
    }
}

/// Hands the session back after a command. It is kept only while it holds an open
/// transaction on a live connection: without one there is nothing to commit, and the
/// connection is closed rather than held (its SET / USE state must not reach the pool).
pub async fn settle(state: &AppState, tab_id: &str, mut s: TabSession) {
    if s.tx_open && conn_alive(&mut s.conn).await {
        state.sessions.put(tab_id, s).await;
    } else {
        state.sessions.discard(tab_id, Some(s)).await;
    }
}

/// `PoolConnection` does not expose whether its connection is still alive; every session is
/// closed on the errors that kill it (Io / Protocol) or when a later statement fails.
pub async fn conn_alive(c: &mut Conn) -> bool {
    match c {
        Conn::Pg(c) => alive::<Postgres>(c).await,
        Conn::MySql(c) => alive::<MySql>(c).await,
        Conn::Sqlite(c) => alive::<Sqlite>(c).await,
    }
}

async fn alive<DB: sqlx::Database>(c: &mut DB::Connection) -> bool {
    match tokio::time::timeout(CLOSE_TIMEOUT, c.ping()).await {
        Ok(r) => match r {
            Ok(()) => true,
            // 25P02: the ping itself failed inside an aborted transaction, but the connection
            // is alive — the session must survive so Rollback stays available.
            Err(sqlx::Error::Database(e)) => e.code().as_deref() == Some("25P02"),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// The tab's session for (`conn`, `database`), or a new one. A session on a stale target is
/// closed (its transaction rolls back) and replaced. The caller owns the slot until it hands
/// the session back (`settle`, `put`, `discard`).
pub async fn for_tab(
    state: &AppState,
    conn: &Connection,
    database: &str,
    tab_id: &str,
) -> Result<TabSession, AppError> {
    let old = match state.sessions.take_or_reserve(tab_id, &conn.id)? {
        Some(s) if s.conn_id == conn.id && s.database == database => return Ok(s),
        old => old,
    };
    if let Some(s) = old {
        close_session(s).await;
    }
    match open(state, conn, database).await {
        Ok(s) => Ok(s),
        Err(e) => {
            state.sessions.discard(tab_id, None).await;
            Err(e)
        }
    }
}

/// A pool connection, detached at once (see [`Conn`]).
async fn open(state: &AppState, conn: &Connection, database: &str) -> Result<TabSession, AppError> {
    let c = match connections::pool_for(state, &conn.id, Some(database))? {
        DbPool::Pg(p) => Conn::Pg(p.acquire().await?.detach()),
        DbPool::MySql(p) => Conn::MySql(p.acquire().await?.detach()),
        DbPool::Sqlite(p) => Conn::Sqlite(p.acquire().await?.detach()),
    };
    Ok(TabSession {
        conn_id: conn.id.clone(),
        database: database.to_string(),
        conn: c,
        tx_open: false,
        dirty: false,
        aborted: false,
    })
}

/// Opens the session's explicit transaction: before the first statement of a run that is not
/// a plain read, and again when a statement ended it mid-run, so no later write autocommits.
pub async fn begin(s: &mut TabSession) -> Result<(), AppError> {
    match &mut s.conn {
        Conn::Pg(c) => run_tx::<Postgres>(c, "BEGIN").await,
        Conn::MySql(c) => run_tx::<MySql>(c, "START TRANSACTION").await,
        Conn::Sqlite(c) => run_tx::<Sqlite>(c, "BEGIN").await,
    }?;
    s.tx_open = true;
    s.dirty = false;
    s.aborted = false;
    Ok(())
}

/// COMMIT or ROLLBACK inside the session.
pub async fn end_tx(s: &mut TabSession, commit: bool) -> Result<(), AppError> {
    if !s.tx_open {
        return Err(AppError::invalid("No transaction is open in this tab"));
    }
    let sql = if commit { "COMMIT" } else { "ROLLBACK" };
    let r = match &mut s.conn {
        Conn::Pg(c) => run_tx::<Postgres>(c, sql).await,
        Conn::MySql(c) => run_tx::<MySql>(c, sql).await,
        Conn::Sqlite(c) => run_tx::<Sqlite>(c, sql).await,
    };
    r.map_err(|e| {
        AppError::new(
            ErrorCode::Database,
            format!(
                "{} failed; the transaction may still be open",
                if commit { "Commit" } else { "Rollback" }
            ),
        )
        .with_detail(e.to_string())
    })?;
    s.tx_open = false;
    s.dirty = false;
    s.aborted = false;
    Ok(())
}

async fn run_tx<DB>(c: &mut DB::Connection, sql: &str) -> Result<(), sqlx::Error>
where
    DB: sqlx::Database,
    for<'x> &'x mut DB::Connection: Executor<'x, Database = DB>,
{
    match tokio::time::timeout(TX_TIMEOUT, c.execute(sqlx::raw_sql(sql))).await {
        Ok(r) => {
            r?;
            Ok(())
        }
        // A timed-out COMMIT may still have landed; the session is closed by the caller's
        // liveness check on error paths anyway.
        Err(e) => Err(sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            e.to_string(),
        ))),
    }
}

/// Refreshes `tx_open` / `aborted` / `dirty` after a run. MySQL's probe doubles as the
/// implicit-commit detector; PG's handles the aborted state (25P02 still counts as "in
/// transaction").
pub async fn refresh(s: &mut TabSession) {
    match &mut s.conn {
        Conn::Pg(c) => {
            // Simple protocol, so the probe is its own statement: outside a transaction it
            // starts one and the clocks are equal; inside one, now() is the BEGIN. A second
            // column reports the write xact (`pg_current_xact_id_if_assigned` is NULL in a
            // read-only transaction), which is what "pending" means on PG.
            match tokio::time::timeout(
                TX_TIMEOUT,
                c.fetch_one(sqlx::raw_sql(
                    "SELECT now() <> statement_timestamp(), \
                     pg_current_xact_id_if_assigned() IS NOT NULL",
                )),
            )
            .await
            {
                Ok(Ok(row)) => {
                    use sqlx::Row;
                    let in_tx: bool = row.try_get(0).unwrap_or(false);
                    let write_xact: bool = row.try_get(1).unwrap_or(false);
                    s.tx_open = in_tx;
                    s.dirty = (s.dirty || write_xact) && in_tx;
                    s.aborted = false;
                }
                // 25P02: a statement failed inside the transaction — it is open and aborted.
                Ok(Err(sqlx::Error::Database(e))) if e.code().as_deref() == Some("25P02") => {
                    s.tx_open = true;
                    s.aborted = true;
                }
                _ => {}
            }
        }
        Conn::MySql(c) => {
            if let Some(open) = mysql_in_transaction(c).await {
                // The statement itself may have committed implicitly (DDL on MySQL).
                s.tx_open = open;
                s.dirty = s.dirty && open;
            }
        }
        Conn::Sqlite(c) => {
            if let Ok(mut h) = c.lock_handle().await {
                // SAFETY: the locked handle gives exclusive access to the live connection.
                let state = unsafe {
                    libsqlite3_sys::sqlite3_txn_state(h.as_raw_handle().as_ptr(), ptr::null())
                };
                s.tx_open = state != libsqlite3_sys::SQLITE_TXN_NONE;
                s.dirty = state == libsqlite3_sys::SQLITE_TXN_WRITE;
            }
        }
    }
}

/// Whether the transaction holds changes worth asking about ("pending"), asked of the server
/// where the engine reports it precisely.
pub async fn pending(s: &mut TabSession) -> bool {
    if !s.tx_open {
        return false;
    }
    match &mut s.conn {
        Conn::Pg(c) => {
            if s.aborted {
                return true;
            }
            tokio::time::timeout(
                TX_TIMEOUT,
                c.fetch_one(sqlx::raw_sql(
                    "SELECT pg_current_xact_id_if_assigned() IS NOT NULL",
                )),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
            .and_then(|row| sqlx::Row::try_get::<bool, _>(&row, 0).ok())
            .unwrap_or(s.dirty)
        }
        Conn::MySql(c) => {
            if !s.dirty {
                return false;
            }
            innodb_trx_has_changes(c).await.unwrap_or(s.dirty)
        }
        Conn::Sqlite(_) => s.dirty,
    }
}

/// The session's own `innodb_trx` row: present only while uncommitted locks or writes exist.
/// Only asked after a write-capable statement (`s.dirty`).
async fn innodb_trx_has_changes(c: &mut MySqlConnection) -> Option<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT CAST(count(*) AS SIGNED) FROM information_schema.innodb_trx \
         WHERE trx_mysql_thread_id = CONNECTION_ID()",
    )
    .fetch_one(&mut *c)
    .await
    .ok()?;
    Some(n > 0)
}

/// MySQL/MariaDB: is the session inside a transaction. `@@in_transaction` exists only on
/// MariaDB; on MySQL `SET TRANSACTION READ WRITE` fails with 25001 while a transaction is open
/// (harmless on a read-write session: it only presets the next transaction's mode).
async fn mysql_in_transaction(c: &mut MySqlConnection) -> Option<bool> {
    tokio::time::timeout(TX_TIMEOUT, mysql_probe(c))
        .await
        .ok()
        .flatten()
}

async fn mysql_probe(c: &mut MySqlConnection) -> Option<bool> {
    if let Ok(v) = sqlx::query_scalar::<_, i64>("SELECT CAST(@@in_transaction AS SIGNED)")
        .fetch_one(&mut *c)
        .await
    {
        return Some(v != 0);
    }
    match c.execute(sqlx::raw_sql("SET TRANSACTION READ WRITE")).await {
        Ok(_) => Some(false),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("25001") => Some(true),
        Err(_) => None,
    }
}

/// Ends a transaction that holds nothing worth asking about, once a run is done: a plain
/// read must not leave the tab idle in a transaction the user cannot end (Commit / Rollback
/// are offered only for pending work) on a stale snapshot. ROLLBACK, not COMMIT: nothing is
/// pending, so nothing is lost, and no write ever commits without the user deciding.
pub async fn end_if_clean(s: &mut TabSession) {
    if s.tx_open && !s.aborted && !pending(s).await && end_tx(s, false).await.is_err() {
        // The connection is suspect; `settle` closes it, which rolls back on the server.
        s.tx_open = false;
    }
}

/// COMMIT (`commit: true`) or ROLLBACK the tab's transaction. The session closes either way:
/// without a transaction it has nothing to keep, and a failed end must take its transaction
/// with it.
pub async fn end_tx_for(state: &AppState, tab_id: &str, commit: bool) -> Result<(), AppError> {
    let mut s = state
        .sessions
        .take(tab_id)?
        .ok_or_else(|| AppError::not_found("No session for this tab"))?;
    let r = end_tx(&mut s, commit).await;
    state.sessions.discard(tab_id, Some(s)).await;
    r
}

/// Reports the tab's transaction state. `None` when the tab has no session (a session whose
/// transaction ended or whose connection died is closed here).
pub async fn status(state: &AppState, tab_id: &str) -> Result<Option<TabStatus>, AppError> {
    let Some(mut s) = state.sessions.take(tab_id)? else {
        return Ok(None);
    };
    if !conn_alive(&mut s.conn).await {
        state.sessions.discard(tab_id, Some(s)).await;
        return Ok(None);
    }
    refresh(&mut s).await;
    if !s.tx_open {
        state.sessions.discard(tab_id, Some(s)).await;
        return Ok(None);
    }
    let out = TabStatus {
        open: true,
        tx_open: true,
        pending: pending(&mut s).await,
        aborted: s.aborted,
    };
    state.sessions.put(tab_id, s).await;
    Ok(Some(out))
}

/// The statement may change data. A statement that is not a plain read but parses as a query
/// (`SELECT f()`) may write through the function.
pub fn may_write(kind: Option<StatementKind>) -> bool {
    !matches!(
        kind,
        Some(StatementKind::Show | StatementKind::Explain { analyze: false })
            | Some(StatementKind::Transaction | StatementKind::Pragma)
    )
}

/// The statement may end the open transaction: user transaction control, and on MySQL every
/// statement with an implicit commit (DDL, LOCK, SET autocommit, CALL, GRANT, …). Plain DML
/// and reads never do, so they skip the probe.
pub fn may_end_tx(kind: Option<StatementKind>) -> bool {
    !matches!(
        kind,
        Some(
            StatementKind::Query
                | StatementKind::Show
                | StatementKind::Explain { .. }
                | StatementKind::Insert
                | StatementKind::Update
                | StatementKind::Delete
                | StatementKind::Merge
        )
    )
}

/// The notice when statement `i` of `n` closed the transaction implicitly (MySQL), naming
/// the earlier uncommitted changes it committed along with it (`swept`).
pub fn implicit_commit_notice(
    kind: Option<StatementKind>,
    swept: bool,
    i: usize,
    n: usize,
) -> String {
    let ddl = matches!(
        kind,
        Some(
            StatementKind::Create
                | StatementKind::Alter
                | StatementKind::Drop
                | StatementKind::Truncate
        )
    );
    let mut m = if n > 1 {
        format!("Statement {} of {n}: ", i + 1)
    } else {
        String::new()
    };
    m.push_str(if ddl {
        "MySQL committed this DDL immediately"
    } else {
        "MySQL committed the transaction implicitly at this statement"
    });
    if swept {
        m.push_str(", including the uncommitted changes made before it");
    }
    if i + 1 < n {
        m.push_str("; the statements after it run in a new transaction");
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_commit_notice_names_swept_changes() {
        let k = Some(StatementKind::Alter);
        assert_eq!(
            implicit_commit_notice(k, false, 0, 1),
            "MySQL committed this DDL immediately"
        );
        let m = implicit_commit_notice(k, true, 0, 2);
        assert!(m.starts_with("Statement 1 of 2: MySQL committed this DDL immediately"));
        assert!(m.contains("including the uncommitted changes"), "{m}");
        assert!(m.contains("new transaction"), "{m}");
    }

    #[test]
    fn probe_and_dirty_rules() {
        use StatementKind as K;
        assert!(!may_end_tx(Some(K::Insert)));
        assert!(!may_end_tx(Some(K::Query)));
        assert!(may_end_tx(Some(K::Alter)));
        assert!(may_end_tx(Some(K::Transaction)));
        assert!(may_end_tx(Some(K::Other)));
        assert!(may_end_tx(None), "unparsable text is probed");
        assert!(may_write(Some(K::Query)));
        assert!(may_write(None));
        assert!(!may_write(Some(K::Transaction)));
    }

    fn busy(conn_id: &str) -> Slot {
        Slot::InUse {
            conn_id: conn_id.into(),
            closed: false,
        }
    }

    #[tokio::test]
    async fn reservation_blocks_a_second_take_and_discard_clears_it() {
        let reg = SessionRegistry::default();
        assert!(reg.take_or_reserve("t", "c").unwrap().is_none());
        assert!(
            reg.take("t").is_err(),
            "the reserved slot must read as in use"
        );
        assert!(reg.take_or_reserve("t", "c").is_err());
        assert_eq!(reg.pending_tab_ids(), vec!["t".to_string()]);
        reg.discard("t", None).await;
        assert!(!reg.has("t"));
        assert!(reg.take("t").unwrap().is_none());
        assert!(!reg.has("t"), "a plain take reserves nothing");
    }

    #[tokio::test]
    async fn close_marks_in_use_slots() {
        let reg = SessionRegistry::default();
        reg.lock().tabs.insert("a".into(), busy("c1"));
        reg.lock().tabs.insert("b".into(), busy("c2"));
        reg.close("a").await;
        reg.close_conn("c2").await;
        for t in ["a", "b"] {
            assert!(
                matches!(
                    reg.lock().tabs.get(t),
                    Some(Slot::InUse { closed: true, .. })
                ),
                "{t}"
            );
            assert!(reg.take(t).is_err(), "{t}: still in use until handed back");
        }
        assert!(reg.pending_tab_ids().is_empty());
        reg.discard("a", None).await;
        assert!(!reg.has("a"));
    }
}
