//! MySQL / MariaDB `information_schema`. The database is the namespace (no schema level).
//! Values are read as raw bytes: MySQL 8 reports some catalog columns as binary strings,
//! which sqlx's typed `String` decode rejects.

use sqlx::mysql::MySqlRow;
use sqlx::{Decode, MySql, MySqlPool, Row};

use super::{
    ColumnInfo, SNAPSHOT_MAX_COLUMNS, SNAPSHOT_MAX_TABLES, SnapRow, SnapTable, TableInfo, TableKind,
};
use crate::core::error::AppError;

const SYSTEM_DBS: &str = "('information_schema', 'mysql', 'performance_schema', 'sys')";

fn text(row: &MySqlRow, i: usize) -> Option<String> {
    let v = row.try_get_raw(i).ok()?;
    if sqlx::ValueRef::is_null(&v) {
        return None;
    }
    <&[u8] as Decode<MySql>>::decode(v)
        .ok()
        .map(|b| String::from_utf8_lossy(b).into_owned())
}

fn req(row: &MySqlRow, i: usize) -> String {
    text(row, i).unwrap_or_default()
}

fn kind(table_type: &str) -> TableKind {
    if table_type.eq_ignore_ascii_case("VIEW") || table_type.eq_ignore_ascii_case("SYSTEM VIEW") {
        TableKind::View
    } else {
        TableKind::Table
    }
}

pub async fn databases(p: &MySqlPool) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query(&format!(
        "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA \
         WHERE SCHEMA_NAME NOT IN {SYSTEM_DBS} ORDER BY SCHEMA_NAME"
    ))
    .fetch_all(p)
    .await?;
    Ok(rows.iter().map(|r| req(r, 0)).collect())
}

pub async fn tables(p: &MySqlPool, database: &str) -> Result<Vec<TableInfo>, AppError> {
    let rows = sqlx::query(
        "SELECT TABLE_NAME, TABLE_TYPE, CAST(TABLE_ROWS AS CHAR) FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
    )
    .bind(database)
    .fetch_all(p)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let k = kind(&req(r, 1));
            TableInfo {
                name: req(r, 0),
                kind: k,
                row_estimate: match k {
                    TableKind::Table => text(r, 2).and_then(|s| s.parse().ok()),
                    TableKind::View => None,
                },
                has_rowid: None,
            }
        })
        .collect())
}

pub async fn columns(
    p: &MySqlPool,
    database: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, AppError> {
    let rows = sqlx::query(
        "SELECT c.COLUMN_NAME, c.COLUMN_TYPE, c.IS_NULLABLE, c.COLUMN_DEFAULT, \
           CAST(k.ORDINAL_POSITION AS CHAR) \
         FROM information_schema.COLUMNS c \
         LEFT JOIN information_schema.KEY_COLUMN_USAGE k \
           ON k.TABLE_SCHEMA = c.TABLE_SCHEMA AND k.TABLE_NAME = c.TABLE_NAME \
           AND k.COLUMN_NAME = c.COLUMN_NAME AND k.CONSTRAINT_NAME = 'PRIMARY' \
         WHERE c.TABLE_SCHEMA = ? AND c.TABLE_NAME = ? ORDER BY c.ORDINAL_POSITION",
    )
    .bind(database)
    .bind(table)
    .fetch_all(p)
    .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let pk = text(r, 4).and_then(|s| s.parse::<u32>().ok());
            ColumnInfo {
                name: req(r, 0),
                data_type: req(r, 1),
                nullable: req(r, 2).eq_ignore_ascii_case("YES"),
                // MariaDB reports "no default" on a nullable column as the literal NULL.
                default: text(r, 3).filter(|d| d != "NULL"),
                is_pk: pk.is_some(),
                pk_ordinal: pk,
            }
        })
        .collect())
}

pub async fn snapshot_tables(p: &MySqlPool, database: &str) -> Result<Vec<SnapTable>, AppError> {
    let rows = sqlx::query(&format!(
        "SELECT TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME LIMIT {}",
        SNAPSHOT_MAX_TABLES + 1
    ))
    .bind(database)
    .fetch_all(p)
    .await?;
    Ok(rows
        .iter()
        .map(|r| SnapTable {
            schema: None,
            table: req(r, 0),
            kind: kind(&req(r, 1)),
        })
        .collect())
}

pub async fn snapshot(p: &MySqlPool, database: &str) -> Result<Vec<SnapRow>, AppError> {
    let rows = sqlx::query(&format!(
        "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION LIMIT {}",
        SNAPSHOT_MAX_COLUMNS + 1
    ))
    .bind(database)
    .fetch_all(p)
    .await?;
    Ok(rows
        .iter()
        .map(|r| SnapRow {
            schema: None,
            table: req(r, 0),
            column: req(r, 1),
            data_type: req(r, 2),
        })
        .collect())
}
