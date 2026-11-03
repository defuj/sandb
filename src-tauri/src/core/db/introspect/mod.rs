//! Lazy schema browsing: databases → schemas (PG only) → tables and views → columns, plus a
//! capped snapshot for editor autocomplete.

mod mysql;
mod pg;
mod sqlite;

use std::collections::HashMap;

use serde::Serialize;

use crate::core::connections::{self, Engine};
use crate::core::db::pool::DbPool;
use crate::core::error::AppError;
use crate::core::state::AppState;

/// Columns in one autocomplete snapshot; larger schemas are cut and flagged `truncated`.
pub const SNAPSHOT_MAX_COLUMNS: usize = 2000;
/// Tables in one snapshot. Separate from the column cap so that wide tables early in the
/// order cannot hide later table names from autocomplete.
pub const SNAPSHOT_MAX_TABLES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
    /// Server statistics, not an exact count; absent when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_estimate: Option<i64>,
    /// SQLite only: the table has an implicit `rowid` (not `WITHOUT ROWID`, not a view).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_rowid: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    pub is_pk: bool,
    /// 1-based position in the primary key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pk_ordinal: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTable {
    /// PG schema; absent on MySQL and SQLite (the database is the namespace).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<SnapshotColumn>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    pub database: String,
    pub tables: Vec<SnapshotTable>,
    pub truncated: bool,
}

/// One table or view, ordered as the engine lists them, at most `SNAPSHOT_MAX_TABLES + 1`.
pub(crate) struct SnapTable {
    pub schema: Option<String>,
    pub table: String,
    pub kind: TableKind,
}

/// One column, ordered by table, at most `SNAPSHOT_MAX_COLUMNS + 1`.
pub(crate) struct SnapRow {
    pub schema: Option<String>,
    pub table: String,
    pub column: String,
    pub data_type: String,
}

/// Every table (up to `SNAPSHOT_MAX_TABLES`), with columns filled in up to `SNAPSHOT_MAX_COLUMNS`.
pub(crate) fn group_snapshot(
    database: String,
    tables: Vec<SnapTable>,
    rows: Vec<SnapRow>,
) -> SchemaSnapshot {
    let truncated = tables.len() > SNAPSHOT_MAX_TABLES || rows.len() > SNAPSHOT_MAX_COLUMNS;
    let mut tables: Vec<SnapshotTable> = tables
        .into_iter()
        .take(SNAPSHOT_MAX_TABLES)
        .map(|t| SnapshotTable {
            schema: t.schema,
            name: t.table,
            kind: t.kind,
            columns: vec![],
        })
        .collect();
    let at: HashMap<(Option<String>, String), usize> = tables
        .iter()
        .enumerate()
        .map(|(i, t)| ((t.schema.clone(), t.name.clone()), i))
        .collect();
    for r in rows.into_iter().take(SNAPSHOT_MAX_COLUMNS) {
        if let Some(&i) = at.get(&(r.schema, r.table)) {
            tables[i].columns.push(SnapshotColumn {
                name: r.column,
                data_type: r.data_type,
            });
        }
    }
    SchemaSnapshot {
        database,
        tables,
        truncated,
    }
}

pub(crate) fn not_found_table(table: &str) -> AppError {
    AppError::not_found(format!("Table {table} not found"))
}

fn engine_of(state: &AppState, conn_id: &str) -> Result<Engine, AppError> {
    Ok(connections::get(state, conn_id)?.engine)
}

/// Databases the user can open. SQLite has exactly one: `main`.
pub async fn list_databases(state: &AppState, conn_id: &str) -> Result<Vec<String>, AppError> {
    // PG and MySQL list databases from the connection's own database.
    match connections::pool(state, conn_id)? {
        DbPool::Pg(p) => pg::databases(&p).await,
        DbPool::MySql(p) => mysql::databases(&p).await,
        DbPool::Sqlite(_) => Ok(vec!["main".into()]),
    }
}

/// PG schemas of `database`. Empty for MySQL and SQLite, which have no schema level.
pub async fn list_schemas(
    state: &AppState,
    conn_id: &str,
    database: &str,
) -> Result<Vec<String>, AppError> {
    if engine_of(state, conn_id)? != Engine::Pg {
        return Ok(vec![]);
    }
    match connections::pool_for(state, conn_id, Some(database))? {
        DbPool::Pg(p) => pg::schemas(&p).await,
        _ => Ok(vec![]),
    }
}

/// Tables and views. `schema` is required on PG (default `public`) and ignored elsewhere.
pub async fn list_tables(
    state: &AppState,
    conn_id: &str,
    database: &str,
    schema: Option<&str>,
) -> Result<Vec<TableInfo>, AppError> {
    match connections::pool_for(state, conn_id, Some(database))? {
        DbPool::Pg(p) => pg::tables(&p, schema.unwrap_or("public")).await,
        DbPool::MySql(p) => mysql::tables(&p, database).await,
        DbPool::Sqlite(p) => sqlite::tables(&p).await,
    }
}

pub async fn list_columns(
    state: &AppState,
    conn_id: &str,
    database: &str,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>, AppError> {
    let cols = match connections::pool_for(state, conn_id, Some(database))? {
        DbPool::Pg(p) => pg::columns(&p, schema.unwrap_or("public"), table).await?,
        DbPool::MySql(p) => mysql::columns(&p, database, table).await?,
        DbPool::Sqlite(p) => sqlite::columns(&p, table).await?,
    };
    if cols.is_empty() {
        return Err(not_found_table(table));
    }
    Ok(cols)
}

pub async fn schema_snapshot(
    state: &AppState,
    conn_id: &str,
    database: &str,
) -> Result<SchemaSnapshot, AppError> {
    let (tables, rows) = match connections::pool_for(state, conn_id, Some(database))? {
        DbPool::Pg(p) => (pg::snapshot_tables(&p).await?, pg::snapshot(&p).await?),
        DbPool::MySql(p) => (
            mysql::snapshot_tables(&p, database).await?,
            mysql::snapshot(&p, database).await?,
        ),
        DbPool::Sqlite(p) => (
            sqlite::snapshot_tables(&p).await?,
            sqlite::snapshot(&p).await?,
        ),
    };
    let db = if engine_of(state, conn_id)? == Engine::Sqlite {
        "main".to_string()
    } else {
        database.to_string()
    };
    Ok(group_snapshot(db, tables, rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tbl(schema: Option<&str>, table: &str) -> SnapTable {
        SnapTable {
            schema: schema.map(Into::into),
            table: table.into(),
            kind: TableKind::Table,
        }
    }

    fn row(schema: Option<&str>, table: &str, col: &str) -> SnapRow {
        SnapRow {
            schema: schema.map(Into::into),
            table: table.into(),
            column: col.into(),
            data_type: "int".into(),
        }
    }

    #[test]
    fn snapshot_groups_by_schema_and_table() {
        let s = group_snapshot(
            "shop".into(),
            vec![
                tbl(Some("analytics"), "events"),
                tbl(Some("public"), "events"),
            ],
            vec![
                row(Some("analytics"), "events", "id"),
                row(Some("analytics"), "events", "name"),
                row(Some("public"), "events", "id"),
            ],
        );
        assert_eq!(s.tables.len(), 2);
        assert_eq!(s.tables[0].columns.len(), 2);
        assert_eq!(s.tables[1].columns.len(), 1);
        assert!(!s.truncated);
    }

    // Many partition tables can use up the column cap early; autocomplete must still offer
    // the names of the tables that come after it.
    #[test]
    fn column_cap_keeps_every_table_name() {
        let tables: Vec<SnapTable> = (0..300).map(|i| tbl(None, &format!("t{i:03}"))).collect();
        let rows = (0..SNAPSHOT_MAX_COLUMNS + 1)
            .map(|i| row(None, &format!("t{:03}", i / 10), "c"))
            .collect();
        let s = group_snapshot("db".into(), tables, rows);
        assert!(s.truncated);
        assert_eq!(s.tables.len(), 300);
        let n: usize = s.tables.iter().map(|t| t.columns.len()).sum();
        assert_eq!(n, SNAPSHOT_MAX_COLUMNS);
        assert_eq!(s.tables[299].name, "t299");
        assert!(s.tables[299].columns.is_empty());
    }

    #[test]
    fn table_cap_is_flagged() {
        let tables = (0..SNAPSHOT_MAX_TABLES + 1)
            .map(|i| tbl(None, &format!("t{i}")))
            .collect();
        let s = group_snapshot("db".into(), tables, vec![]);
        assert!(s.truncated);
        assert_eq!(s.tables.len(), SNAPSHOT_MAX_TABLES);
    }

    #[test]
    fn json_shapes() {
        let t = TableInfo {
            name: "t".into(),
            kind: TableKind::View,
            row_estimate: None,
            has_rowid: None,
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            serde_json::json!({"name": "t", "kind": "view"})
        );
        let c = ColumnInfo {
            name: "id".into(),
            data_type: "integer".into(),
            nullable: false,
            default: None,
            is_pk: true,
            pk_ordinal: Some(1),
        };
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            serde_json::json!({"name": "id", "dataType": "integer", "nullable": false, "isPk": true, "pkOrdinal": 1})
        );
    }
}
