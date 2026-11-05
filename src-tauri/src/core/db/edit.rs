//! DBeaver-style grid editing for one simple table SELECT.
//!
//! Editability is derived again when saving: the original SQL, current primary-key metadata and
//! connection write permission are all checked before any statement runs. Every value is bound;
//! identifiers from parsed SQL and schema metadata are quoted, never interpolated as values.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlparser::ast::{
    Expr, GroupByExpr, Ident, ObjectName, ObjectNamePart, Query, SelectFlavor, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor, WildcardAdditionalOptions,
};
use sqlparser::parser::Parser;
use sqlx::mysql::{MySqlArguments, MySqlConnection};
use sqlx::postgres::{PgArguments, PgConnection};
use sqlx::sqlite::{SqliteArguments, SqliteConnection};
use sqlx::{Executor as _, MySql, Postgres, Sqlite};

use crate::core::connections::{self, Connection, Engine, Env};
use crate::core::db::introspect::{self, ColumnInfo};
use crate::core::db::pool::DbPool;
use crate::core::db::session::{self, Conn};
use crate::core::error::{AppError, ErrorCode};
use crate::core::sql::split;
use crate::core::state::AppState;

pub type CellValue = Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableRequest {
    pub conn_id: String,
    pub database: String,
    pub sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub pk: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl EditableResult {
    fn not_editable(reason: impl Into<String>) -> Self {
        Self {
            table: None,
            schema: None,
            pk: Vec::new(),
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyEditsRequest {
    pub conn_id: String,
    pub database: String,
    pub sql: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOperation {
    pub key: String,
    pub kind: EditKind,
    pub pk: BTreeMap<String, CellValue>,
    pub set: BTreeMap<String, CellValue>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditKind {
    Update,
    Insert,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyEditsResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<EditFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFailure {
    pub index: usize,
    pub message: String,
}

#[derive(Debug)]
enum ParsedQuery {
    NotEditable(String),
    Simple(SimpleQuery),
}

#[derive(Debug)]
struct SimpleQuery {
    engine: Engine,
    qualifier: Option<Ident>,
    table: Ident,
    projection: Projection,
}

#[derive(Debug, Default)]
struct Projection {
    all: bool,
    columns: Vec<ProjectedColumn>,
}

#[derive(Debug)]
struct ProjectedColumn {
    name: String,
    output_name: String,
}

#[derive(Debug)]
struct Target {
    result: EditableResult,
    /// Database used for MySQL's qualified table and metadata lookup.
    database: String,
    /// Qualified, quoted table name. All parts came from parsed identifiers or known defaults.
    relation: String,
    columns: BTreeMap<String, String>,
}

enum Resolution {
    NotEditable(String),
    Editable(Target),
}

fn plain_wildcard(o: &WildcardAdditionalOptions) -> bool {
    o.opt_ilike.is_none()
        && o.opt_exclude.is_none()
        && o.opt_except.is_none()
        && o.opt_replace.is_none()
        && o.opt_rename.is_none()
        && o.opt_alias.is_none()
}

fn ident_name(engine: Engine, ident: &Ident) -> String {
    match engine {
        Engine::Pg if ident.quote_style.is_none() => ident.value.to_lowercase(),
        Engine::Mysql | Engine::Sqlite => ident.value.to_lowercase(),
        Engine::Pg => ident.value.clone(),
    }
}

fn output_name(engine: Engine, ident: &Ident) -> String {
    match engine {
        Engine::Pg => ident_name(engine, ident),
        // These engines report the projection's original spelling, which must match metadata
        // exactly before it can be used as an edit key.
        Engine::Mysql | Engine::Sqlite => ident.value.clone(),
    }
}

fn object_idents(name: &ObjectName) -> Option<Vec<Ident>> {
    name.0
        .iter()
        .map(|p| match p {
            ObjectNamePart::Identifier(i) => Some(i.clone()),
            _ => None,
        })
        .collect()
}

fn projected_column(
    engine: Engine,
    expr: &Expr,
    alias: Option<&Ident>,
    table: &Ident,
    table_alias: Option<&Ident>,
) -> Option<ProjectedColumn> {
    let (name, qualifier) = match expr {
        Expr::Identifier(i) => (i, None),
        Expr::CompoundIdentifier(parts) if parts.len() == 2 => (&parts[1], Some(&parts[0])),
        _ => return None,
    };
    if let Some(q) = qualifier {
        let q = ident_name(engine, q);
        if q != ident_name(engine, table) && table_alias.is_none_or(|a| q != ident_name(engine, a))
        {
            return None;
        }
    }
    if alias.is_some_and(|alias| alias.value != name.value) {
        return None;
    }
    Some(ProjectedColumn {
        name: output_name(engine, name),
        output_name: output_name(engine, name),
    })
}

/// Parses exactly the SELECT shape whose rows can be traced back to one base-table key.
fn parse_simple(engine: Engine, sql: &str) -> Result<ParsedQuery, AppError> {
    let dialect = split::dialect(engine);
    let parsed = Parser::parse_sql(&*dialect, sql)
        .map_err(|e| AppError::invalid("Invalid SQL").with_detail(e.to_string()))?;
    let [Statement::Query(query)] = parsed.as_slice() else {
        return Ok(ParsedQuery::NotEditable(
            "Grid editing requires one SELECT query".into(),
        ));
    };
    match simple_query(engine, query) {
        Some(q) => Ok(ParsedQuery::Simple(q)),
        None => Ok(ParsedQuery::NotEditable(
            "Only a simple single-table SELECT with plain columns or * is editable".into(),
        )),
    }
}

fn simple_query(engine: Engine, query: &Query) -> Option<SimpleQuery> {
    if query.with.is_some()
        || !query.locks.is_empty()
        || query.for_clause.is_some()
        || query.format_clause.is_some()
        || query.settings.is_some()
        || !query.pipe_operators.is_empty()
    {
        return None;
    }
    let SetExpr::Select(select) = &*query.body else {
        return None;
    };
    if select.distinct.is_some()
        || select.top.is_some()
        || select.into.is_some()
        || !select.lateral_views.is_empty()
        || select.prewhere.is_some()
        || !select.connect_by.is_empty()
        || !matches!(&select.group_by, GroupByExpr::Expressions(v, _) if v.is_empty())
        || !select.cluster_by.is_empty()
        || !select.distribute_by.is_empty()
        || !select.sort_by.is_empty()
        || select.having.is_some()
        || !select.named_window.is_empty()
        || select.qualify.is_some()
        || select.value_table_mode.is_some()
        || select.flavor != SelectFlavor::Standard
        || select.from.len() != 1
        || !select.from[0].joins.is_empty()
    {
        return None;
    }
    let TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        with_ordinality,
        partitions,
        json_path,
        sample,
        index_hints,
    } = &select.from[0].relation
    else {
        return None;
    };
    if args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
        || alias
            .as_ref()
            .is_some_and(|a| !a.columns.is_empty() || a.at.is_some())
    {
        return None;
    }
    let mut parts = object_idents(name)?;
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let table = parts.pop()?;
    let qualifier = parts.pop();
    let table_alias = alias.as_ref().map(|a| &a.name);
    let mut projection = Projection::default();
    let mut output_names = BTreeSet::new();
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(o) if plain_wildcard(o) => {
                if projection.all {
                    return None;
                }
                projection.all = true;
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::ObjectName(n), o)
                if plain_wildcard(o) =>
            {
                let ids = object_idents(n)?;
                let Some(q) = ids.last() else { return None };
                let q = ident_name(engine, q);
                if q != ident_name(engine, &table)
                    && table_alias.is_none_or(|a| q != ident_name(engine, a))
                {
                    return None;
                }
                if projection.all {
                    return None;
                }
                projection.all = true;
            }
            SelectItem::UnnamedExpr(expr) => {
                let col = projected_column(engine, expr, None, &table, table_alias)?;
                if !output_names.insert(col.output_name.clone()) {
                    return None;
                }
                projection.columns.push(col);
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let col = projected_column(engine, expr, Some(alias), &table, table_alias)?;
                if !output_names.insert(col.output_name.clone()) {
                    return None;
                }
                projection.columns.push(col);
            }
            _ => return None,
        }
    }
    if !projection.all && projection.columns.is_empty() {
        return None;
    }
    Some(SimpleQuery {
        engine,
        qualifier,
        table,
        projection,
    })
}

fn quote(engine: Engine, ident: &str) -> String {
    match engine {
        Engine::Mysql => format!("`{}`", ident.replace('`', "``")),
        Engine::Pg | Engine::Sqlite => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

fn table_names(q: &SimpleQuery, requested_database: &str) -> (Option<String>, String, String) {
    match q.engine {
        Engine::Pg => {
            let table = ident_name(q.engine, &q.table);
            let schema = q
                .qualifier
                .as_ref()
                .map(|s| ident_name(q.engine, s))
                .unwrap_or_else(|| "public".into());
            let relation = format!("{}.{}", quote(q.engine, &schema), quote(q.engine, &table));
            (Some(schema), table, relation)
        }
        Engine::Mysql => {
            let table = q.table.value.clone();
            let database = q
                .qualifier
                .as_ref()
                .map(|s| s.value.clone())
                .unwrap_or_else(|| requested_database.to_string());
            let relation = if q.qualifier.is_some() {
                format!("{}.{}", quote(q.engine, &database), quote(q.engine, &table))
            } else {
                quote(q.engine, &table)
            };
            (None, table, relation)
        }
        Engine::Sqlite => {
            let table = q.table.value.clone();
            let schema = q
                .qualifier
                .as_ref()
                .map(|s| s.value.clone())
                .unwrap_or_else(|| "main".into());
            let relation = format!("{}.{}", quote(q.engine, &schema), quote(q.engine, &table));
            (None, table, relation)
        }
    }
}

async fn resolve(
    state: &AppState,
    conn: &Connection,
    database: &str,
    sql: &str,
) -> Result<Resolution, AppError> {
    let simple = match parse_simple(conn.engine, sql)? {
        ParsedQuery::NotEditable(reason) => return Ok(Resolution::NotEditable(reason)),
        ParsedQuery::Simple(q) => q,
    };
    let (schema, table, relation) = table_names(&simple, database);
    if conn.engine == Engine::Sqlite && schema_is_not_main(&simple) {
        return Ok(Resolution::NotEditable(
            "Only tables in SQLite's main database are editable".into(),
        ));
    }
    let metadata_database = if conn.engine == Engine::Mysql {
        simple
            .qualifier
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_else(|| database.to_string())
    } else {
        database.to_string()
    };
    let columns = introspect::list_columns(
        state,
        &conn.id,
        &metadata_database,
        schema.as_deref(),
        &table,
    )
    .await?;
    let declared: BTreeSet<String> = columns.iter().map(|c| c.name.to_lowercase()).collect();
    for projected in &simple.projection.columns {
        let actual = columns.iter().find(|c| c.name == projected.output_name);
        if actual.is_some() && simple.projection.all {
            return Ok(Resolution::NotEditable(format!(
                "Output column {} is duplicated by the wildcard",
                projected.output_name
            )));
        }
        let implicit_rowid = conn.engine == Engine::Sqlite
            && ["rowid", "_rowid_", "oid"].contains(&projected.output_name.as_str())
            && !declared.contains(&projected.output_name.to_lowercase());
        if actual.is_none() && !implicit_rowid {
            return Ok(Resolution::NotEditable(format!(
                "Output column {} does not exactly match a table column",
                projected.output_name
            )));
        }
    }
    let mut pk: Vec<&ColumnInfo> = columns.iter().filter(|c| c.is_pk).collect();
    pk.sort_by_key(|c| c.pk_ordinal);
    let mut pk: Vec<String> = pk.into_iter().map(|c| c.name.clone()).collect();
    if pk.is_empty() {
        if conn.engine == Engine::Sqlite {
            let has_rowid = introspect::list_tables(state, &conn.id, database, None)
                .await?
                .into_iter()
                .find(|t| t.name.eq_ignore_ascii_case(&table))
                .and_then(|t| t.has_rowid)
                .unwrap_or(false);
            let rowid = if has_rowid {
                ["rowid", "_rowid_", "oid"].into_iter().find(|candidate| {
                    !declared.contains(*candidate)
                        && simple
                            .projection
                            .columns
                            .iter()
                            .any(|c| c.name == *candidate && c.output_name == *candidate)
                })
            } else {
                None
            };
            if let Some(rowid) = rowid {
                pk.push(rowid.into());
            }
        }
        if pk.is_empty() {
            return Ok(Resolution::NotEditable(
                "The table has no primary key included in the result".into(),
            ));
        }
    }
    let missing: Vec<&str> = pk
        .iter()
        .map(String::as_str)
        .filter(|name| {
            !simple.projection.all
                && !simple
                    .projection
                    .columns
                    .iter()
                    .any(|c| c.name == *name && c.output_name == *name)
        })
        .collect();
    if !missing.is_empty() {
        return Ok(Resolution::NotEditable(format!(
            "The result does not include primary-key column(s): {}",
            missing.join(", ")
        )));
    }
    let result = EditableResult {
        table: Some(table),
        schema,
        pk,
        reason: None,
    };
    let mut column_types: BTreeMap<String, String> =
        columns.into_iter().map(|c| (c.name, c.data_type)).collect();
    for name in &result.pk {
        if conn.engine == Engine::Sqlite && !column_types.contains_key(name) {
            column_types.insert(name.clone(), "INTEGER".into());
        }
    }
    Ok(Resolution::Editable(Target {
        result,
        database: metadata_database,
        relation,
        columns: column_types,
    }))
}

fn schema_is_not_main(q: &SimpleQuery) -> bool {
    q.qualifier
        .as_ref()
        .is_some_and(|s| !s.value.eq_ignore_ascii_case("main"))
}

/// Describes how the result can be edited. A valid but unsupported query is a normal result with
/// `reason`, not a command error.
pub async fn editable_result(
    state: &AppState,
    request: EditableRequest,
) -> Result<EditableResult, AppError> {
    let conn = connections::get(state, &request.conn_id)?;
    let database = connections::target_database(&conn, Some(&request.database));
    match resolve(state, &conn, &database, &request.sql).await? {
        Resolution::NotEditable(reason) => Ok(EditableResult::not_editable(reason)),
        Resolution::Editable(_) if conn.read_only => {
            Ok(EditableResult::not_editable("The connection is read-only"))
        }
        Resolution::Editable(target) => Ok(target.result),
    }
}

struct BoundValue<'a> {
    value: &'a CellValue,
    data_type: &'a str,
}

fn pg_base_type(data_type: &str) -> String {
    let without_typmod = if let Some(open) = data_type.find('(')
        && let Some(close) = data_type[open..].find(')').map(|i| open + i)
    {
        format!("{}{}", &data_type[..open], &data_type[close + 1..])
    } else {
        data_type.to_string()
    };
    // Bare CHARACTER and BIT mean length one in PostgreSQL and explicitly casting to them
    // truncates. Their unbounded counterparts preserve the value so assignment to char(n) or
    // bit(n) reports overflow, and unbounded values also work in PK comparisons.
    if let Some(scalar) = without_typmod.strip_suffix("[]") {
        let scalar = match scalar {
            "character" | "char" => "bpchar",
            "bit" => "bit varying",
            other => other,
        };
        format!("{scalar}[]")
    } else {
        match without_typmod.as_str() {
            "character" | "char" => "bpchar".into(),
            "bit" => "bit varying".into(),
            _ => without_typmod,
        }
    }
}

fn placeholder(engine: Engine, index: usize, data_type: &str) -> String {
    match engine {
        // The cast makes text representations of bigint/numeric/uuid/… bind with the column's
        // actual PG type instead of SQLx's TEXT parameter type. Types come from server metadata.
        // Removing the typmod is important: CAST(x AS varchar(n)) truncates instead of letting
        // assignment to the column report an over-length value.
        Engine::Pg => format!("CAST(${index} AS {})", pg_base_type(data_type)),
        Engine::Mysql | Engine::Sqlite => "?".into(),
    }
}

/// The grid shows geometry as plain WKT, which has SRID 0. A value without an SRID gets the
/// column's SRID: the typmod's (`geometry(Point,4326)`), else on update the row's old one.
/// A value with its own SRID (EWKT) is left as it is.
fn keep_srid(engine: Engine, p: &str, column: &str, data_type: &str, update: bool) -> String {
    if engine != Engine::Pg {
        return p.into();
    }
    // `geometry`, or `extensions.geometry` when PostGIS is not on the search path.
    let base = pg_base_type(data_type);
    let Some(schema) = base.strip_suffix("geometry") else {
        return p.into();
    };
    if !(schema.is_empty() || schema.ends_with('.')) {
        return p.into();
    }
    let typmod_srid = data_type
        .split_once('(')
        .and_then(|(_, args)| args.trim_end_matches(')').split(',').nth(1))
        .and_then(|s| s.trim().parse::<i32>().ok())
        .filter(|&s| s != 0);
    let srid = match typmod_srid {
        Some(s) => s.to_string(),
        None if update => format!("COALESCE({schema}st_srid({}), 0)", quote(engine, column)),
        None => return p.into(),
    };
    format!("CASE WHEN {schema}st_srid({p}) = 0 THEN {schema}st_setsrid({p}, {srid}) ELSE {p} END")
}

fn operation_sql<'a>(
    target: &'a Target,
    engine: Engine,
    op: &'a EditOperation,
) -> Result<(String, Vec<BoundValue<'a>>), String> {
    if !matches!(op.kind, EditKind::Insert) {
        let expected: BTreeSet<&str> = target.result.pk.iter().map(String::as_str).collect();
        let actual: BTreeSet<&str> = op.pk.keys().map(String::as_str).collect();
        if actual != expected {
            return Err(
                "The operation must contain exactly the original primary-key values".into(),
            );
        }
    }
    for name in op.set.keys() {
        if !target.columns.contains_key(name) {
            return Err(format!("Unknown table column {name}"));
        }
    }
    let mut binds = Vec::new();
    let mut next = 1usize;
    let mut push = |name: &'a str, value: &'a CellValue| {
        let data_type = target.columns.get(name).expect("validated column");
        let p = placeholder(engine, next, data_type);
        next += 1;
        binds.push(BoundValue { value, data_type });
        p
    };
    let sql = match op.kind {
        EditKind::Update => {
            if op.set.is_empty() {
                return Err("An update needs at least one changed column".into());
            }
            let assignments = op
                .set
                .iter()
                .map(|(name, value)| {
                    let p = push(name, value);
                    let p = keep_srid(engine, &p, name, &target.columns[name.as_str()], true);
                    format!("{} = {p}", quote(engine, name))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let predicates = op
                .pk
                .iter()
                .map(|(name, value)| {
                    let p = push(name, value);
                    format!("{} = {p}", quote(engine, name))
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            format!(
                "UPDATE {} SET {assignments} WHERE {predicates}",
                target.relation
            )
        }
        EditKind::Delete => {
            let predicates = op
                .pk
                .iter()
                .map(|(name, value)| {
                    let p = push(name, value);
                    format!("{} = {p}", quote(engine, name))
                })
                .collect::<Vec<_>>()
                .join(" AND ");
            format!("DELETE FROM {} WHERE {predicates}", target.relation)
        }
        EditKind::Insert if op.set.is_empty() && engine == Engine::Mysql => {
            format!("INSERT INTO {} () VALUES ()", target.relation)
        }
        EditKind::Insert if op.set.is_empty() => {
            format!("INSERT INTO {} DEFAULT VALUES", target.relation)
        }
        EditKind::Insert => {
            let names = op
                .set
                .keys()
                .map(|name| quote(engine, name))
                .collect::<Vec<_>>()
                .join(", ");
            let values = op
                .set
                .iter()
                .map(|(name, value)| {
                    let p = push(name, value);
                    keep_srid(engine, &p, name, &target.columns[name.as_str()], false)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "INSERT INTO {} ({names}) VALUES ({values})",
                target.relation
            )
        }
    };
    Ok((sql, binds))
}

fn binary(value: &Value) -> Result<Option<Vec<u8>>, String> {
    let Value::Object(o) = value else {
        return Ok(None);
    };
    if !o.contains_key("$bytes") {
        return Ok(None);
    }
    let Some(encoded) = o.get("$bytes").and_then(Value::as_str) else {
        return Err("Invalid binary cell value".into());
    };
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "Invalid binary cell value".to_string())?;
    if o.get("len").and_then(Value::as_u64) != Some(bytes.len() as u64) {
        return Err("A truncated binary preview cannot be saved".into());
    }
    Ok(Some(bytes))
}

trait EditExecutor {
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), AppError>>;
    fn execute<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<u64, String>>
    where
        'v: 'a;
    /// First column of the first row as text; `None` when no row matches.
    fn fetch_text<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<Option<Option<String>>, AppError>>
    where
        'v: 'a;
}

fn first_text<R: sqlx::Row>(row: Option<R>) -> Result<Option<Option<String>>, AppError>
where
    for<'r> Option<String>: sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    usize: sqlx::ColumnIndex<R>,
{
    row.map(|r| r.try_get::<Option<String>, _>(0))
        .transpose()
        .map_err(|e| AppError::internal("Could not read a value").with_detail(e.to_string()))
}

type PgQuery<'q> = sqlx::query::Query<'q, Postgres, PgArguments>;
type MySqlQuery<'q> = sqlx::query::Query<'q, MySql, MySqlArguments>;
type SqliteQuery<'q> = sqlx::query::Query<'q, Sqlite, SqliteArguments<'q>>;

fn pg_array_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn pg_array_element(value: &Value) -> Result<String, String> {
    Ok(match value {
        Value::Null => "NULL".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => pg_array_string(s),
        Value::Array(_) => pg_array_text(value)?.expect("array value"),
        Value::Object(o) if o.contains_key("$bytes") => {
            let bytes = binary(value)?.expect("binary object");
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut encoded = String::with_capacity(2 + bytes.len() * 2);
            encoded.push_str("\\x");
            for byte in bytes {
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0f) as usize] as char);
            }
            // The array parser consumes one backslash layer, leaving bytea's \x hex input.
            pg_array_string(&encoded)
        }
        // Other JSON objects can be elements of JSON/JSONB arrays; their compact text is one
        // quoted array element rather than PostgreSQL array syntax.
        Value::Object(_) => pg_array_string(&value.to_string()),
    })
}

fn pg_array_text(value: &Value) -> Result<Option<String>, String> {
    let Value::Array(items) = value else {
        return Ok(None);
    };
    Ok(Some(format!(
        "{{{}}}",
        items
            .iter()
            .map(pg_array_element)
            .collect::<Result<Vec<_>, _>>()?
            .join(",")
    )))
}

fn bind_pg<'q>(mut q: PgQuery<'q>, b: &BoundValue<'q>) -> Result<PgQuery<'q>, String> {
    q = match b.value {
        Value::Null => q.bind(Option::<String>::None),
        Value::Bool(v) => q.bind(*v),
        Value::Number(n) if n.is_i64() => q.bind(n.as_i64().unwrap_or_default()),
        Value::Number(n) if n.is_u64() && n.as_u64().is_some_and(|v| v <= i64::MAX as u64) => {
            q.bind(n.as_u64().unwrap_or_default() as i64)
        }
        Value::Number(n) if n.is_u64() => q.bind(n.to_string()),
        Value::Number(n) if n.is_f64() => q.bind(n.as_f64().unwrap_or_default()),
        Value::String(s) => q.bind(s.as_str()),
        other => {
            if let Some(bytes) = binary(other)? {
                q.bind(bytes)
            } else if b.data_type.ends_with("[]")
                && let Some(array) = pg_array_text(other)?
            {
                q.bind(array)
            } else {
                q.bind(other.to_string())
            }
        }
    };
    Ok(q)
}

fn bind_mysql<'q>(mut q: MySqlQuery<'q>, b: &BoundValue<'q>) -> Result<MySqlQuery<'q>, String> {
    q = match b.value {
        Value::Null => q.bind(Option::<String>::None),
        Value::Bool(v) => q.bind(*v),
        Value::Number(n) if n.is_i64() => q.bind(n.as_i64().unwrap_or_default()),
        Value::Number(n) if n.is_u64() => q.bind(n.as_u64().unwrap_or_default()),
        Value::Number(n) => q.bind(n.as_f64().unwrap_or_default()),
        Value::String(s) => q.bind(s.as_str()),
        other => {
            if let Some(bytes) = binary(other)? {
                q.bind(bytes)
            } else {
                q.bind(other.to_string())
            }
        }
    };
    Ok(q)
}

fn bind_sqlite<'q>(mut q: SqliteQuery<'q>, b: &BoundValue<'q>) -> Result<SqliteQuery<'q>, String> {
    q = match b.value {
        Value::Null => q.bind(Option::<String>::None),
        Value::Bool(v) => q.bind(*v),
        Value::Number(n) if n.is_i64() => q.bind(n.as_i64().unwrap_or_default()),
        Value::Number(n) if n.is_u64() && n.as_u64().is_some_and(|v| v <= i64::MAX as u64) => {
            q.bind(n.as_u64().unwrap_or_default() as i64)
        }
        Value::Number(n) if n.is_u64() => q.bind(n.as_u64().unwrap_or_default().to_string()),
        Value::Number(n) => q.bind(n.as_f64().unwrap_or_default()),
        Value::String(s) => q.bind(s.as_str()),
        other => {
            if let Some(bytes) = binary(other)? {
                q.bind(bytes)
            } else {
                q.bind(other.to_string())
            }
        }
    };
    Ok(q)
}

impl EditExecutor for PgConnection {
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            sqlx::query(sql).execute(self).await?;
            Ok(())
        })
    }

    fn execute<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<u64, String>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_pg(q, b)?;
            }
            q.execute(self)
                .await
                .map(|r| r.rows_affected())
                .map_err(|e| AppError::from(e).message)
        })
    }

    fn fetch_text<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<Option<Option<String>>, AppError>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_pg(q, b).map_err(AppError::invalid)?;
            }
            first_text(q.fetch_optional(self).await?)
        })
    }
}

impl EditExecutor for MySqlConnection {
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            (&mut *self).fetch_optional(sqlx::raw_sql(sql)).await?;
            Ok(())
        })
    }

    fn execute<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<u64, String>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_mysql(q, b)?;
            }
            q.execute(self)
                .await
                .map(|r| r.rows_affected())
                .map_err(|e| AppError::from(e).message)
        })
    }

    fn fetch_text<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<Option<Option<String>>, AppError>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_mysql(q, b).map_err(AppError::invalid)?;
            }
            first_text(q.fetch_optional(self).await?)
        })
    }
}

impl EditExecutor for SqliteConnection {
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            sqlx::query(sql).execute(self).await?;
            Ok(())
        })
    }

    fn execute<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<u64, String>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_sqlite(q, b)?;
            }
            q.execute(self)
                .await
                .map(|r| r.rows_affected())
                .map_err(|e| AppError::from(e).message)
        })
    }

    fn fetch_text<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<Option<Option<String>>, AppError>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            let mut q = sqlx::query(sql);
            for b in binds {
                q = bind_sqlite(q, b).map_err(AppError::invalid)?;
            }
            first_text(q.fetch_optional(self).await?)
        })
    }
}

impl EditExecutor for Conn {
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), AppError>> {
        Box::pin(async move {
            match self {
                Conn::Pg(c) => c.control(sql).await,
                Conn::MySql(c) => c.control(sql).await,
                Conn::Sqlite(c) => c.control(sql).await,
            }
        })
    }

    fn execute<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<u64, String>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            match self {
                Conn::Pg(c) => EditExecutor::execute(c, sql, binds).await,
                Conn::MySql(c) => EditExecutor::execute(c, sql, binds).await,
                Conn::Sqlite(c) => EditExecutor::execute(c, sql, binds).await,
            }
        })
    }

    fn fetch_text<'a, 'v>(
        &'a mut self,
        sql: &'a str,
        binds: &'a [BoundValue<'v>],
    ) -> BoxFuture<'a, Result<Option<Option<String>>, AppError>>
    where
        'v: 'a,
    {
        Box::pin(async move {
            match self {
                Conn::Pg(c) => c.fetch_text(sql, binds).await,
                Conn::MySql(c) => c.fetch_text(sql, binds).await,
                Conn::Sqlite(c) => c.fetch_text(sql, binds).await,
            }
        })
    }
}

async fn run_operations<C: EditExecutor>(
    c: &mut C,
    target: &Target,
    engine: Engine,
    operations: &[EditOperation],
) -> Option<EditFailure> {
    for (index, op) in operations.iter().enumerate() {
        let result = match operation_sql(target, engine, op) {
            Ok((sql, binds)) => c.execute(&sql, &binds).await,
            Err(message) => return Some(EditFailure { index, message }),
        };
        match result {
            Ok(0) if matches!(op.kind, EditKind::Update | EditKind::Delete) => {
                return Some(EditFailure {
                    index,
                    message: "The row no longer exists or its primary key changed".into(),
                });
            }
            Ok(_) => {}
            Err(message) => return Some(EditFailure { index, message }),
        }
    }
    None
}

async fn apply_autocommit<C: EditExecutor>(
    c: &mut C,
    target: &Target,
    engine: Engine,
    operations: &[EditOperation],
) -> Result<ApplyEditsResult, AppError> {
    c.control("BEGIN").await?;
    let failed = run_operations(c, target, engine, operations).await;
    if let Some(failed) = failed {
        c.control("ROLLBACK").await?;
        return Ok(ApplyEditsResult {
            failed: Some(failed),
        });
    }
    if let Err(e) = c.control("COMMIT").await {
        let _ = c.control("ROLLBACK").await;
        return Err(e);
    }
    Ok(ApplyEditsResult { failed: None })
}

async fn apply_in_session(
    state: &AppState,
    conn: &Connection,
    database: &str,
    tab_id: &str,
    target: &Target,
    operations: &[EditOperation],
) -> Result<ApplyEditsResult, AppError> {
    let mut s = session::for_tab(state, conn, database, tab_id).await?;
    let r = async {
        if !s.tx_open {
            session::begin(&mut s).await?;
        }
        s.conn.control("SAVEPOINT sandb_grid_edit").await?;
        let failed = run_operations(&mut s.conn, target, conn.engine, operations).await;
        if let Some(failed) = failed {
            s.conn
                .control("ROLLBACK TO SAVEPOINT sandb_grid_edit")
                .await?;
            s.conn.control("RELEASE SAVEPOINT sandb_grid_edit").await?;
            return Ok(ApplyEditsResult {
                failed: Some(failed),
            });
        }
        s.conn.control("RELEASE SAVEPOINT sandb_grid_edit").await?;
        s.dirty = true;
        Ok(ApplyEditsResult { failed: None })
    }
    .await;
    session::refresh(&mut s).await;
    session::end_if_clean(&mut s).await;
    session::settle(state, tab_id, s).await;
    r
}

/// Revalidates the source query and applies the whole batch atomically. Production tab sessions
/// use a savepoint inside their manual-commit transaction; only standalone saves COMMIT.
pub async fn apply_edits(
    state: &AppState,
    request: ApplyEditsRequest,
) -> Result<ApplyEditsResult, AppError> {
    let conn = connections::get(state, &request.conn_id)?;
    if conn.read_only {
        return Err(AppError::new(
            ErrorCode::ConnectionReadOnly,
            format!("Connection \"{}\" is read-only", conn.name),
        ));
    }
    let database = connections::target_database(&conn, Some(&request.database));
    let target = match resolve(state, &conn, &database, &request.sql).await? {
        Resolution::NotEditable(reason) => return Err(AppError::invalid(reason)),
        Resolution::Editable(target) => target,
    };
    if request.operations.is_empty() {
        return Ok(ApplyEditsResult { failed: None });
    }
    if conn.env == Env::Production
        && let Some(tab_id) = request.tab_id.as_deref()
    {
        return apply_in_session(
            state,
            &conn,
            &database,
            tab_id,
            &target,
            &request.operations,
        )
        .await;
    }
    match connections::pool_for(state, &conn.id, Some(&target.database))? {
        DbPool::Pg(p) => {
            let mut c = p.acquire().await?;
            apply_autocommit(&mut *c, &target, conn.engine, &request.operations).await
        }
        DbPool::MySql(p) => {
            let mut c = p.acquire().await?;
            apply_autocommit(&mut *c, &target, conn.engine, &request.operations).await
        }
        DbPool::Sqlite(p) => {
            let mut c = p.acquire().await?;
            apply_autocommit(&mut *c, &target, conn.engine, &request.operations).await
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellValueRequest {
    pub conn_id: String,
    pub database: String,
    pub sql: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    pub pk: BTreeMap<String, CellValue>,
    pub column: String,
}

/// The full text of one cell that the grid shows cut at `value::PREVIEW_BYTES`, read by
/// primary key so that it can be edited. PostGIS geometry comes back as WKT. A tab with an
/// open transaction reads through it, so the value includes the tab's unsaved changes.
pub async fn cell_value(
    state: &AppState,
    request: CellValueRequest,
) -> Result<Option<String>, AppError> {
    let conn = connections::get(state, &request.conn_id)?;
    let database = connections::target_database(&conn, Some(&request.database));
    let target = match resolve(state, &conn, &database, &request.sql).await? {
        Resolution::NotEditable(reason) => return Err(AppError::invalid(reason)),
        Resolution::Editable(target) => target,
    };
    let engine = conn.engine;
    let Some(data_type) = target.columns.get(&request.column) else {
        return Err(AppError::invalid(format!(
            "Unknown table column {}",
            request.column
        )));
    };
    let expected: BTreeSet<&str> = target.result.pk.iter().map(String::as_str).collect();
    if request
        .pk
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != expected
    {
        return Err(AppError::invalid(
            "The request must contain exactly the primary-key values",
        ));
    }
    let mut binds = Vec::new();
    let predicates = request
        .pk
        .iter()
        .enumerate()
        .map(|(i, (name, value))| {
            let data_type = &target.columns[name.as_str()];
            binds.push(BoundValue { value, data_type });
            format!(
                "{} = {}",
                quote(engine, name),
                placeholder(engine, i + 1, data_type)
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let text = match engine {
        Engine::Pg => "text",
        Engine::Mysql => "CHAR",
        Engine::Sqlite => "TEXT",
    };
    let sql = format!(
        "SELECT CAST({} AS {text}) FROM {} WHERE {predicates} LIMIT 1",
        quote(engine, &request.column),
        target.relation
    );
    let session = match request.tab_id.as_deref() {
        Some(tab_id) => state.sessions.take(tab_id)?.map(|s| (tab_id, s)),
        None => None,
    };
    let row = match session {
        Some((tab_id, mut s)) if s.conn_id == conn.id && s.database == database => {
            let r = s.conn.fetch_text(&sql, &binds).await;
            state.sessions.put(tab_id, s).await;
            r?
        }
        other => {
            if let Some((tab_id, s)) = other {
                state.sessions.put(tab_id, s).await;
            }
            match connections::pool_for(state, &conn.id, Some(&target.database))? {
                DbPool::Pg(p) => p.acquire().await?.fetch_text(&sql, &binds).await?,
                DbPool::MySql(p) => p.acquire().await?.fetch_text(&sql, &binds).await?,
                DbPool::Sqlite(p) => p.acquire().await?.fetch_text(&sql, &binds).await?,
            }
        }
    };
    let Some(value) = row else {
        return Err(AppError::not_found(
            "The row no longer exists or its primary key changed",
        ));
    };
    let base = pg_base_type(data_type);
    let spatial =
        engine == Engine::Pg && (base.ends_with("geometry") || base.ends_with("geography"));
    Ok(value.map(|v| match spatial {
        true => crate::core::db::wkb::hex_ewkb_to_wkt(&v, None).unwrap_or(v),
        false => v,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::connections::{ConnectionInput, SslMode};
    use crate::core::db::query::{self, RunRequest};
    use crate::core::secrets::MemoryStore;
    use sqlx::Connection as _;
    use sqlx::sqlite::SqliteConnectOptions;

    async fn sqlite_state(env: Env) -> (tempfile::TempDir, AppState, String) {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(dir.path(), Box::new(MemoryStore::new())).unwrap();
        let file = dir.path().join("grid.db");
        let mut c = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&file)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::Executor::execute(
            &mut c,
            sqlx::raw_sql(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER); \
                 INSERT INTO users VALUES (1, 'one', 1); \
                 CREATE TABLE logs (message TEXT); \
                 INSERT INTO logs VALUES ('first'); \
                 CREATE TABLE docs (id INTEGER PRIMARY KEY, payload JSON)",
            ),
        )
        .await
        .unwrap();
        c.close().await.unwrap();
        let conn = connections::create(
            &state,
            ConnectionInput {
                name: "sqlite".into(),
                engine: Engine::Sqlite,
                env,
                host: String::new(),
                port: 0,
                database: "main".into(),
                user: String::new(),
                ssl: SslMode::Disable,
                file: file.to_string_lossy().into_owned(),
                ai_write: false,
                read_only: false,
                version: None,
            },
            None,
        )
        .unwrap();
        (dir, state, conn.id)
    }

    fn op(kind: EditKind, pk: &[(&str, i64)], set: &[(&str, Value)]) -> EditOperation {
        EditOperation {
            key: uuid::Uuid::new_v4().to_string(),
            kind,
            pk: pk
                .iter()
                .map(|(k, v)| (k.to_string(), Value::from(*v)))
                .collect(),
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    async fn save(
        state: &AppState,
        conn_id: &str,
        operations: Vec<EditOperation>,
        tab_id: Option<&str>,
    ) -> ApplyEditsResult {
        apply_edits(
            state,
            ApplyEditsRequest {
                conn_id: conn_id.into(),
                database: "main".into(),
                sql: "SELECT id, name, age FROM users".into(),
                tab_id: tab_id.map(str::to_string),
                operations,
            },
        )
        .await
        .unwrap()
    }

    async fn user_rows(state: &AppState, conn_id: &str) -> Vec<Vec<Value>> {
        query::run(
            state,
            RunRequest {
                conn_id: conn_id.into(),
                database: Some("main".into()),
                sql: "SELECT id, name, age FROM users ORDER BY id".into(),
                query_id: uuid::Uuid::new_v4().to_string(),
                limit: None,
                tab_id: None,
                sort: None,
            },
        )
        .await
        .unwrap()
        .statements
        .remove(0)
        .rows
    }

    fn simple(engine: Engine, sql: &str) -> Option<SimpleQuery> {
        match parse_simple(engine, sql).unwrap() {
            ParsedQuery::Simple(q) => Some(q),
            ParsedQuery::NotEditable(_) => None,
        }
    }

    #[test]
    fn accepts_plain_single_table_selects() {
        for sql in [
            "SELECT * FROM users",
            "SELECT id, name FROM users WHERE active = true ORDER BY name LIMIT 20",
            "SELECT u.id, u.name AS name FROM public.users AS u",
        ] {
            assert!(simple(Engine::Pg, sql).is_some(), "{sql}");
        }
    }

    #[test]
    fn refuses_ambiguous_or_computed_selects() {
        for sql in [
            "SELECT a.id FROM a JOIN b ON b.id = a.id",
            "WITH x AS (SELECT * FROM a) SELECT * FROM x",
            "SELECT id FROM a UNION SELECT id FROM b",
            "SELECT DISTINCT id FROM a",
            "SELECT id, count(*) FROM a GROUP BY id",
            "SELECT lower(name) FROM a",
            "SELECT id AS other FROM a",
            "SELECT id AS ID FROM a",
            "SELECT id, id FROM a",
            "SELECT *, name AS id FROM a",
            "SELECT *, * FROM a",
            "SELECT * FROM (SELECT * FROM a) x",
            "UPDATE a SET name = 'x'",
        ] {
            assert!(simple(Engine::Pg, sql).is_none(), "{sql}");
        }
    }

    #[test]
    fn mysql_table_names_preserve_case() {
        let query = simple(Engine::Mysql, "SELECT id FROM Sales.CustomerOrder").unwrap();
        let (schema, table, relation) = table_names(&query, "other");
        assert_eq!(schema, None);
        assert_eq!(table, "CustomerOrder");
        assert_eq!(relation, "`Sales`.`CustomerOrder`");
    }

    #[test]
    fn quotes_identifiers_without_inlining_values() {
        let target = Target {
            result: EditableResult {
                table: Some("user".into()),
                schema: Some("public".into()),
                pk: vec!["id".into()],
                reason: None,
            },
            database: "db".into(),
            relation: "\"public\".\"user\"".into(),
            columns: BTreeMap::from([
                ("id".into(), "bigint".into()),
                ("name".into(), "character varying(255)".into()),
            ]),
        };
        let op = EditOperation {
            key: "0".into(),
            kind: EditKind::Update,
            pk: BTreeMap::from([("id".into(), Value::String("42".into()))]),
            set: BTreeMap::from([(
                "name".into(),
                Value::String("x'; DROP TABLE user; --".into()),
            )]),
        };
        let (sql, binds) = operation_sql(&target, Engine::Pg, &op).unwrap();
        assert_eq!(
            sql,
            "UPDATE \"public\".\"user\" SET \"name\" = CAST($1 AS character varying) WHERE \"id\" = CAST($2 AS bigint)"
        );
        assert!(!sql.contains("DROP TABLE"));
        assert_eq!(binds.len(), 2);
        assert_eq!(
            pg_base_type("timestamp(3) with time zone"),
            "timestamp with time zone"
        );
        assert_eq!(pg_base_type("character(2)[]"), "bpchar[]");
    }

    #[test]
    fn pg_char_and_bit_sql_uses_unbounded_comparable_types() {
        let update = |pk_type: &str, set_type: &str| {
            let target = Target {
                result: EditableResult {
                    table: Some("t".into()),
                    schema: Some("public".into()),
                    pk: vec!["id".into()],
                    reason: None,
                },
                database: "db".into(),
                relation: "\"public\".\"t\"".into(),
                columns: BTreeMap::from([
                    ("id".into(), pk_type.into()),
                    ("value".into(), set_type.into()),
                ]),
            };
            let op = EditOperation {
                key: "0".into(),
                kind: EditKind::Update,
                pk: BTreeMap::from([("id".into(), Value::from("key"))]),
                set: BTreeMap::from([("value".into(), Value::from("new"))]),
            };
            operation_sql(&target, Engine::Pg, &op).unwrap().0
        };
        assert_eq!(
            update("character(8)", "character(4)"),
            "UPDATE \"public\".\"t\" SET \"value\" = CAST($1 AS bpchar) WHERE \"id\" = CAST($2 AS bpchar)"
        );
        assert_eq!(
            update("bit(8)", "bit(4)"),
            "UPDATE \"public\".\"t\" SET \"value\" = CAST($1 AS bit varying) WHERE \"id\" = CAST($2 AS bit varying)"
        );
    }

    #[test]
    fn pg_array_text_handles_null_nested_empty_and_escaped_strings() {
        assert_eq!(
            pg_array_text(&serde_json::json!([])).unwrap().unwrap(),
            "{}"
        );
        assert_eq!(
            pg_array_text(&serde_json::json!([1, null, true]))
                .unwrap()
                .unwrap(),
            "{1,NULL,true}"
        );
        assert_eq!(
            pg_array_text(&serde_json::json!([[1, 2], [], ["x"]]))
                .unwrap()
                .unwrap(),
            r#"{{1,2},{},{"x"}}"#
        );
        assert_eq!(
            pg_array_text(&serde_json::json!(["a\"b", "c\\d", "{x,y}", "NULL"]))
                .unwrap()
                .unwrap(),
            r#"{"a\"b","c\\d","{x,y}","NULL"}"#
        );
    }

    #[test]
    fn pg_bytea_arrays_use_hex_and_reject_truncated_previews() {
        assert_eq!(
            pg_array_text(&serde_json::json!([
                {"$bytes": "AA==", "len": 1},
                {"$bytes": "ACr/", "len": 3}
            ]))
            .unwrap()
            .unwrap(),
            r#"{"\\x00","\\x002aff"}"#
        );
        let error = pg_array_text(&serde_json::json!([
            {"$bytes": "AA==", "len": 2}
        ]))
        .unwrap_err();
        assert_eq!(error, "A truncated binary preview cannot be saved");
    }

    #[test]
    fn empty_insert_uses_database_defaults() {
        let target = Target {
            result: EditableResult {
                table: Some("t".into()),
                schema: None,
                pk: vec!["id".into()],
                reason: None,
            },
            database: "main".into(),
            relation: "\"main\".\"t\"".into(),
            columns: BTreeMap::from([("id".into(), "INTEGER".into())]),
        };
        let op = EditOperation {
            key: "new".into(),
            kind: EditKind::Insert,
            pk: BTreeMap::new(),
            set: BTreeMap::new(),
        };
        let (sql, binds) = operation_sql(&target, Engine::Sqlite, &op).unwrap();
        assert_eq!(sql, "INSERT INTO \"main\".\"t\" DEFAULT VALUES");
        assert!(binds.is_empty());
    }

    #[tokio::test]
    async fn sqlite_metadata_requires_keys_and_supports_safe_rowid() {
        let (_dir, state, conn_id) = sqlite_state(Env::Local).await;
        let describe = |sql: &str| EditableRequest {
            conn_id: conn_id.clone(),
            database: "main".into(),
            sql: sql.into(),
        };
        let editable = editable_result(&state, describe("SELECT id, name FROM users"))
            .await
            .unwrap();
        assert_eq!(editable.table.as_deref(), Some("users"));
        assert_eq!(editable.pk, ["id"]);
        assert!(editable.reason.is_none());
        assert!(
            editable_result(&state, describe("SELECT id AS id, name FROM users"))
                .await
                .unwrap()
                .reason
                .is_none()
        );
        for sql in [
            "SELECT ID, name FROM users",
            "SELECT id, id FROM users",
            "SELECT *, id FROM users",
            "SELECT *, name AS id FROM users",
        ] {
            assert!(
                editable_result(&state, describe(sql))
                    .await
                    .unwrap()
                    .reason
                    .is_some(),
                "{sql}"
            );
        }
        assert!(
            editable_result(&state, describe("SELECT name FROM users"))
                .await
                .unwrap()
                .reason
                .is_some()
        );
        let rowid = editable_result(&state, describe("SELECT rowid, message FROM logs"))
            .await
            .unwrap();
        assert_eq!(rowid.pk, ["rowid"]);
        assert!(
            editable_result(&state, describe("SELECT * FROM logs"))
                .await
                .unwrap()
                .reason
                .is_some()
        );
        assert!(
            editable_result(&state, describe("SELECT id + 1 AS id FROM users"))
                .await
                .unwrap()
                .reason
                .is_some()
        );
    }

    #[tokio::test]
    async fn sqlite_batches_commit_or_roll_back_atomically() {
        let (_dir, state, conn_id) = sqlite_state(Env::Local).await;
        let failed = save(
            &state,
            &conn_id,
            vec![
                op(
                    EditKind::Update,
                    &[("id", 1)],
                    &[("name", Value::from("changed"))],
                ),
                op(
                    EditKind::Insert,
                    &[],
                    &[("id", Value::from(2)), ("name", Value::from("two"))],
                ),
                op(EditKind::Delete, &[("id", 999)], &[]),
            ],
            None,
        )
        .await;
        assert_eq!(failed.failed.map(|f| f.index), Some(2));
        assert_eq!(user_rows(&state, &conn_id).await[0][1], Value::from("one"));
        assert_eq!(user_rows(&state, &conn_id).await.len(), 1);

        let saved = save(
            &state,
            &conn_id,
            vec![
                op(
                    EditKind::Update,
                    &[("id", 1)],
                    &[("name", Value::from("changed")), ("age", Value::Null)],
                ),
                op(
                    EditKind::Insert,
                    &[],
                    &[("id", Value::from(2)), ("name", Value::from("two"))],
                ),
            ],
            None,
        )
        .await;
        assert!(saved.failed.is_none());
        let rows = user_rows(&state, &conn_id).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            vec![Value::from(1), Value::from("changed"), Value::Null]
        );
        let deleted = save(
            &state,
            &conn_id,
            vec![op(EditKind::Delete, &[("id", 2)], &[])],
            None,
        )
        .await;
        assert!(deleted.failed.is_none());
        assert_eq!(user_rows(&state, &conn_id).await.len(), 1);
    }

    #[tokio::test]
    async fn sqlite_json_objects_and_arrays_bind_as_compact_text() {
        let (_dir, state, conn_id) = sqlite_state(Env::Local).await;
        let saved = apply_edits(
            &state,
            ApplyEditsRequest {
                conn_id: conn_id.clone(),
                database: "main".into(),
                sql: "SELECT id, payload FROM docs".into(),
                tab_id: None,
                operations: vec![
                    op(
                        EditKind::Insert,
                        &[],
                        &[
                            ("id", Value::from(1)),
                            ("payload", serde_json::json!({"b": [2, 1]})),
                        ],
                    ),
                    op(
                        EditKind::Insert,
                        &[],
                        &[
                            ("id", Value::from(2)),
                            ("payload", serde_json::json!(["a", 1])),
                        ],
                    ),
                ],
            },
        )
        .await
        .unwrap();
        assert!(saved.failed.is_none());
        let run = query::run(
            &state,
            RunRequest {
                conn_id,
                database: Some("main".into()),
                sql: "SELECT payload FROM docs ORDER BY id".into(),
                query_id: "json".into(),
                limit: None,
                tab_id: None,
                sort: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(run.statements[0].rows[0][0], Value::from(r#"{"b":[2,1]}"#));
        assert_eq!(run.statements[0].rows[1][0], Value::from(r#"["a",1]"#));
    }

    #[tokio::test]
    async fn sqlite_sort_orders_before_limit_and_keeps_source_sql() {
        let (_dir, state, conn_id) = sqlite_state(Env::Local).await;
        save(
            &state,
            &conn_id,
            vec![op(
                EditKind::Insert,
                &[],
                &[("id", Value::from(2)), ("name", Value::from("two"))],
            )],
            None,
        )
        .await;
        let source = "SELECT id FROM users ORDER BY id ASC";
        let run = query::run(
            &state,
            RunRequest {
                conn_id,
                database: Some("main".into()),
                sql: source.into(),
                query_id: "sorted".into(),
                limit: Some(1),
                tab_id: None,
                sort: Some(query::RunSort {
                    column: "id".into(),
                    column_index: 0,
                    ascending: false,
                }),
            },
        )
        .await
        .unwrap();
        assert_eq!(run.statements[0].rows[0][0], Value::from(2));
        assert_eq!(run.statements[0].sql, source);
    }

    #[tokio::test]
    async fn production_savepoint_preserves_manual_rollback() {
        let (_dir, state, conn_id) = sqlite_state(Env::Production).await;
        let saved = save(
            &state,
            &conn_id,
            vec![op(
                EditKind::Update,
                &[("id", 1)],
                &[("name", Value::from("uncommitted"))],
            )],
            Some("tab"),
        )
        .await;
        assert!(saved.failed.is_none());
        let status = session::status(&state, "tab").await.unwrap().unwrap();
        assert!(status.tx_open && status.pending);
        session::end_tx_for(&state, "tab", false).await.unwrap();
        assert_eq!(user_rows(&state, &conn_id).await[0][1], Value::from("one"));
    }

    #[test]
    fn contracts_use_camel_case_and_optional_failure() {
        let request: ApplyEditsRequest = serde_json::from_value(serde_json::json!({
            "connId": "c",
            "database": "main",
            "sql": "SELECT * FROM t",
            "tabId": "tab",
            "operations": [{
                "key": "0", "kind": "update", "pk": {"id": 1}, "set": {"name": "n"}
            }]
        }))
        .unwrap();
        assert_eq!(request.tab_id.as_deref(), Some("tab"));
        assert_eq!(
            serde_json::to_value(ApplyEditsResult { failed: None }).unwrap(),
            serde_json::json!({})
        );
        assert_eq!(
            serde_json::to_value(EditableResult {
                table: Some("t".into()),
                schema: None,
                pk: vec!["id".into()],
                reason: None,
            })
            .unwrap(),
            serde_json::json!({"table": "t", "pk": ["id"]})
        );
    }
}
