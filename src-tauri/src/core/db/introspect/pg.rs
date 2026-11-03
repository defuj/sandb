//! PostgreSQL catalogs. Text casts keep decoding independent of catalog column types.

use sqlx::PgPool;

use super::{
    ColumnInfo, SNAPSHOT_MAX_COLUMNS, SNAPSHOT_MAX_TABLES, SnapRow, SnapTable, TableInfo, TableKind,
};
use crate::core::error::AppError;

const SYSTEM_SCHEMAS: &str = "n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
    AND n.nspname NOT LIKE 'pg\\_temp\\_%' AND n.nspname NOT LIKE 'pg\\_toast\\_temp\\_%'";

pub async fn databases(p: &PgPool) -> Result<Vec<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT datname::text FROM pg_database \
         WHERE NOT datistemplate AND datallowconn AND has_database_privilege(datname, 'CONNECT') \
         ORDER BY datname",
    )
    .fetch_all(p)
    .await?)
}

pub async fn schemas(p: &PgPool) -> Result<Vec<String>, AppError> {
    let sql = format!(
        "SELECT n.nspname::text FROM pg_namespace n WHERE {SYSTEM_SCHEMAS} \
         AND has_schema_privilege(n.oid, 'USAGE') ORDER BY n.nspname"
    );
    Ok(sqlx::query_scalar(&sql).fetch_all(p).await?)
}

pub async fn tables(p: &PgPool, schema: &str) -> Result<Vec<TableInfo>, AppError> {
    let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT c.relname::text, c.relkind::text, \
           CASE WHEN c.reltuples >= 0 THEN c.reltuples::int8 END \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
         ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(p)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, kind, est)| {
            let view = kind == "v" || kind == "m";
            TableInfo {
                name,
                kind: if view {
                    TableKind::View
                } else {
                    TableKind::Table
                },
                row_estimate: if view { None } else { est },
                has_rowid: None,
            }
        })
        .collect())
}

/// name, formatted type, NOT NULL, default expression, position in the primary key.
type ColumnRow = (String, String, bool, Option<String>, Option<i32>);

pub async fn columns(p: &PgPool, schema: &str, table: &str) -> Result<Vec<ColumnInfo>, AppError> {
    let rows: Vec<ColumnRow> = sqlx::query_as(
        "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, \
           pg_get_expr(d.adbin, d.adrelid), \
           array_position(pk.conkey, a.attnum)::int4 \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
         LEFT JOIN pg_constraint pk ON pk.conrelid = c.oid AND pk.contype = 'p' \
         WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
         ORDER BY a.attnum",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(p)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, data_type, not_null, default, pk)| ColumnInfo {
            name,
            data_type,
            nullable: !not_null,
            default,
            is_pk: pk.is_some(),
            pk_ordinal: pk.map(|o| o as u32),
        })
        .collect())
}

pub async fn snapshot_tables(p: &PgPool) -> Result<Vec<SnapTable>, AppError> {
    let sql = format!(
        "SELECT n.nspname::text, c.relname::text, c.relkind::text \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE {SYSTEM_SCHEMAS} AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
         ORDER BY n.nspname, c.relname LIMIT {}",
        SNAPSHOT_MAX_TABLES + 1
    );
    let rows: Vec<(String, String, String)> = sqlx::query_as(&sql).fetch_all(p).await?;
    Ok(rows
        .into_iter()
        .map(|(schema, table, kind)| SnapTable {
            schema: Some(schema),
            table,
            kind: if kind == "v" || kind == "m" {
                TableKind::View
            } else {
                TableKind::Table
            },
        })
        .collect())
}

pub async fn snapshot(p: &PgPool) -> Result<Vec<SnapRow>, AppError> {
    let sql = format!(
        "SELECT n.nspname::text, c.relname::text, a.attname::text, \
           format_type(a.atttypid, a.atttypmod) \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
         WHERE {SYSTEM_SCHEMAS} AND c.relkind IN ('r', 'p', 'f', 'v', 'm') \
         ORDER BY n.nspname, c.relname, a.attnum LIMIT {}",
        SNAPSHOT_MAX_COLUMNS + 1
    );
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(&sql).fetch_all(p).await?;
    Ok(rows
        .into_iter()
        .map(|(schema, table, column, data_type)| SnapRow {
            schema: Some(schema),
            table,
            column,
            data_type,
        })
        .collect())
}
