//! SQLite: `sqlite_master` plus `pragma_table_info`. One database, `main`.

use sqlx::SqlitePool;

use super::{
    ColumnInfo, SNAPSHOT_MAX_COLUMNS, SNAPSHOT_MAX_TABLES, SnapRow, SnapTable, TableInfo, TableKind,
};
use crate::core::error::AppError;

fn kind(t: &str) -> TableKind {
    if t == "view" {
        TableKind::View
    } else {
        TableKind::Table
    }
}

/// `CREATE TABLE … ) WITHOUT ROWID` has no implicit rowid.
pub(crate) fn without_rowid(create_sql: &str) -> bool {
    let compact: String = create_sql
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    compact.trim_end_matches(';').ends_with("WITHOUTROWID")
}

pub async fn tables(p: &SqlitePool) -> Result<Vec<TableInfo>, AppError> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, type, sql FROM sqlite_master \
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
    )
    .fetch_all(p)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, t, sql)| {
            let k = kind(&t);
            TableInfo {
                name,
                kind: k,
                row_estimate: None,
                has_rowid: Some(
                    k == TableKind::Table && !without_rowid(sql.as_deref().unwrap_or("")),
                ),
            }
        })
        .collect())
}

pub async fn columns(p: &SqlitePool, table: &str) -> Result<Vec<ColumnInfo>, AppError> {
    let rows: Vec<(String, String, i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT name, CAST(type AS TEXT), \"notnull\", CAST(dflt_value AS TEXT), pk \
         FROM pragma_table_info(?) ORDER BY cid",
    )
    .bind(table)
    .fetch_all(p)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, not_null, default, pk)| ColumnInfo {
            name,
            data_type,
            nullable: not_null == 0,
            default,
            is_pk: pk > 0,
            pk_ordinal: (pk > 0).then_some(pk as u32),
        })
        .collect())
}

pub async fn snapshot_tables(p: &SqlitePool) -> Result<Vec<SnapTable>, AppError> {
    let sql = format!(
        "SELECT name, type FROM sqlite_master \
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
         ORDER BY name LIMIT {}",
        SNAPSHOT_MAX_TABLES + 1
    );
    let rows: Vec<(String, String)> = sqlx::query_as(&sql).fetch_all(p).await?;
    Ok(rows
        .into_iter()
        .map(|(table, t)| SnapTable {
            schema: None,
            table,
            kind: kind(&t),
        })
        .collect())
}

pub async fn snapshot(p: &SqlitePool) -> Result<Vec<SnapRow>, AppError> {
    let sql = format!(
        "SELECT m.name, c.name, CAST(c.type AS TEXT) \
         FROM sqlite_master m JOIN pragma_table_info(m.name) c \
         WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
         ORDER BY m.name, c.cid LIMIT {}",
        SNAPSHOT_MAX_COLUMNS + 1
    );
    let rows: Vec<(String, String, String)> = sqlx::query_as(&sql).fetch_all(p).await?;
    Ok(rows
        .into_iter()
        .map(|(table, column, data_type)| SnapRow {
            schema: None,
            table,
            column,
            data_type,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_without_rowid() {
        assert!(without_rowid(
            "CREATE TABLE t (a PRIMARY KEY) WITHOUT ROWID"
        ));
        assert!(without_rowid(
            "create table t(a primary key)\n  without   rowid ;"
        ));
        assert!(!without_rowid("CREATE TABLE rowid_only (label TEXT)"));
        assert!(!without_rowid(""));
    }
}
