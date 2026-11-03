// Mirrors the Rust serde types in src-tauri/src/core (camelCase fields, lowercase enums).

export type Engine = 'pg' | 'mysql' | 'sqlite';
export type Env = 'production' | 'staging' | 'local';
export type SslMode = 'disable' | 'prefer' | 'require';

/** A saved connection. Never contains the password; that lives in the encrypted credentials file. */
export interface Connection {
  id: string;
  name: string;
  /** `mysql` covers MySQL and MariaDB. */
  engine: Engine;
  /** Server version from the last successful test, e.g. `PostgreSQL 16.4`; empty if never tested. */
  version: string;
  env: Env;
  host: string;
  port: number;
  /** `main` for SQLite. */
  database: string;
  user: string;
  ssl: SslMode;
  /** SQLite database file; empty for other engines. */
  file: string;
  aiWrite: boolean;
  /** Blocks every write on this connection, the user's own queries included. */
  readOnly: boolean;
}

export interface ConnectionInput {
  name: string;
  engine: Engine;
  env: Env;
  host: string;
  port: number;
  database: string;
  user: string;
  ssl: SslMode;
  file: string;
  aiWrite: boolean;
  readOnly: boolean;
  /** Version reported by a test in the dialog; null keeps the stored one. */
  version?: string | null;
}

export interface TestResult {
  serverVersion: string;
  latencyMs: number;
}

export interface TableInfo {
  name: string;
  kind: 'table' | 'view';
  /** Server statistics, not an exact count. */
  rowEstimate?: number;
  /** SQLite only: the table has an implicit rowid. */
  hasRowid?: boolean;
}

export interface ColumnInfo {
  name: string;
  dataType: string;
  nullable: boolean;
  default?: string;
  isPk: boolean;
  /** 1-based position in the primary key. */
  pkOrdinal?: number;
}

export interface SnapshotTable {
  /** PG schema; absent on MySQL and SQLite. */
  schema?: string;
  name: string;
  kind: 'table' | 'view';
  columns: { name: string; dataType: string }[];
}

export interface SchemaSnapshot {
  database: string;
  tables: SnapshotTable[];
  /** More than 2000 columns or 10000 tables: the rest are not in the snapshot. Table names past the column cap are kept, with no columns. */
  truncated: boolean;
}

/** Binary cell value; `$bytes` is base64 of at most the first 4 KiB, `len` the full size. */
export interface BytesValue {
  $bytes: string;
  len: number;
}

/** JSON as decoded by the backend: numbers, strings (bigint/numeric too), bool, null, JSON values, bytes. */
export type CellValue = string | number | boolean | null | BytesValue | CellValue[] | { [k: string]: unknown };

export interface StatementResult {
  sql: string;
  columns: { name: string; dataType: string }[];
  rows: CellValue[][];
  rowCount: number;
  /** More rows exist than the limit; they were not fetched. */
  truncated: boolean;
  /** Rows changed, for statements that return no columns. */
  affected?: number;
  /** A plain read: "Load more" and export may run it again. */
  isRead: boolean;
  elapsedMs: number;
  /** Mock only: the table the prototype's edit mode works on. */
  table?: string;
}

export interface QueryRun {
  statements: StatementResult[];
  /** E.g. an open transaction that was rolled back at the end of the run. */
  notices?: string[];
  elapsedMs: number;
}

/** Live statement progress of a running query. */
export interface QueryProgress {
  done: number;
  total: number;
}

export interface ExportResult {
  /** The user closed the save dialog; nothing was written. */
  cancelled: boolean;
  rows: number;
  path: string;
}

export interface RunRequest {
  connId: string;
  database: string;
  sql: string;
  /** Chosen by the caller; `cancelQuery` uses it. */
  queryId: string;
  limit: number;
  /** Editor tab that runs the query. On a writable Production connection the run executes in the tab's session until commit or rollback. */
  tabId?: string;
}

/** A tab's session on its connection. Only writable Production connections open one. */
export interface TabStatus {
  /** The tab has a dedicated session. */
  open: boolean;
  /** An explicit transaction is open. */
  txOpen: boolean;
  /** The transaction holds uncommitted changes. */
  pending: boolean;
  /** PG: the transaction aborted; only rollback helps. */
  aborted: boolean;
}

/** Result of the AI read guard (layer 1) for one piece of SQL. */
export interface GuardCheck {
  allowed: boolean;
  /** Why the SQL was refused, written so the model can rewrite it. */
  reason?: string;
  /** `SELECT`, `SHOW`, `DESCRIBE`, `EXPLAIN`, `PRAGMA`, or the refused statement's kind. */
  kind?: string;
}

export type WriteWarningCode =
  | 'update_without_where'
  | 'delete_without_where'
  | 'drop'
  | 'truncate'
  | 'alter_drop'
  | 'multiple_statements'
  | 'unparsable'
  | 'executable_comment';

export interface WriteInfo {
  /** `kind`: `INSERT`, `UPDATE`, `DROP TABLE`, …; `table`: the target as written. */
  statements: { kind: string; table?: string }[];
  /** `statement`: 0-based index of the statement; absent for the whole text. */
  warnings: { code: WriteWarningCode; message: string; statement?: number }[];
}

/** One AI read: at most 500 rows, stopped after 15 s, run in a read-only session. */
export interface AiReadResult {
  columns: { name: string; dataType: string }[];
  rows: CellValue[][];
  /** More rows exist than the cap; they were not fetched. */
  truncated: boolean;
  elapsedMs: number;
}

export type ErrorCode =
  | 'not_found'
  | 'invalid_input'
  | 'auth_failed'
  | 'connect_failed'
  | 'timeout'
  | 'secret_store'
  | 'io'
  | 'database'
  /** The user cancelled a running query. */
  | 'cancelled'
  /** The AI read guard refused the SQL before it reached the database. */
  | 'guard_rejected'
  /** The database refused a write inside a read-only AI read. */
  | 'read_only_violation'
  /** A write was refused: the connection is read-only (HTTP 403 in the bridge). */
  | 'connection_read_only'
  | 'internal'
  /** Dev bridge only: missing or wrong bearer token. */
  | 'unauthorized';

export interface AppErrorBody {
  code: ErrorCode;
  message: string;
  detail?: string;
}

export class ApiError extends Error {
  code: ErrorCode;
  detail?: string;

  constructor(body: AppErrorBody) {
    super(body.message);
    this.name = 'ApiError';
    this.code = body.code;
    this.detail = body.detail;
  }

  static from(e: unknown): ApiError {
    if (e instanceof ApiError) return e;
    if (e && typeof e === 'object' && 'code' in e && 'message' in e) return new ApiError(e as AppErrorBody);
    return new ApiError({ code: 'internal', message: e instanceof Error ? e.message : String(e) });
  }
}

/**
 * The commands every transport implements. `password`: null keeps the stored secret
 * (update/duplicate) or means none (create); an empty string clears it.
 */
export interface Backend {
  kind: 'tauri' | 'bridge' | 'mock';
  listConnections(): Promise<Connection[]>;
  createConnection(input: ConnectionInput, password: string | null): Promise<Connection>;
  updateConnection(id: string, input: ConnectionInput, password: string | null): Promise<Connection>;
  /** `input` overrides the copied fields; `password: null` copies the source secret. */
  duplicateConnection(id: string, input: ConnectionInput | null, password: string | null): Promise<Connection>;
  deleteConnection(id: string): Promise<void>;
  /** Tests `input`. Without a password, the stored secret of `id` is used. */
  testConnection(input: ConnectionInput, password: string | null, id: string | null): Promise<TestResult>;
  listDatabases(connId: string): Promise<string[]>;
  /** PG only; empty for MySQL and SQLite (no schema level). */
  listSchemas(connId: string, database: string): Promise<string[]>;
  listTables(connId: string, database: string, schema: string | null): Promise<TableInfo[]>;
  listColumns(connId: string, database: string, schema: string | null, table: string): Promise<ColumnInfo[]>;
  schemaSnapshot(connId: string, database: string): Promise<SchemaSnapshot>;
  /** Runs every statement in `sql` on one connection; fails with code `cancelled` after `cancelQuery`. */
  runQuery(request: RunRequest): Promise<QueryRun>;
  cancelQuery(queryId: string): Promise<void>;
  /** `done`/`total` while the run is in progress; `null` for an unknown or finished id. */
  queryProgress(queryId: string): Promise<QueryProgress | null>;
  /** Commits the tab's open transaction. Fails with `not_found` when the tab has no session. */
  commitTransaction(tabId: string): Promise<void>;
  /** Rolls back the tab's open transaction; the session stays open. */
  rollbackTransaction(tabId: string): Promise<void>;
  /** `null` when the tab has no session. */
  tabStatus(tabId: string): Promise<TabStatus | null>;
  /** Drops the tab's session, rolling back anything uncommitted. */
  closeTabSession(tabId: string): Promise<void>;
  /** Closes the app. Pending transactions are rolled back, so commit or roll back each tab first. */
  quitApp(): Promise<void>;
  /** Tauri only: asks for a file in a native save dialog and streams the full result of one read to it. Other transports export in the browser. `tabId` reads inside the tab's open transaction, as `runQuery` does. */
  exportCsv(connId: string, database: string, sql: string, tabId?: string): Promise<ExportResult>;
  /** `CREATE` statement(s) of one table or view: the table, then its indexes (and triggers on SQLite). */
  tableDdl(connId: string, database: string, schema: string | null, table: string): Promise<string>;
  /** Tauri only: asks for a file in a native save dialog and writes the table's DDL to it; with `data`, INSERT statements for every row follow. */
  exportTableSql(connId: string, database: string, schema: string | null, table: string, data: boolean): Promise<ExportResult>;
  /** Checks SQL against the AI read guard without running it. */
  guardCheck(engine: Engine, sql: string): Promise<GuardCheck>;
  /** Describes a proposed write for the approval preview. Never runs it. */
  classifyWrite(engine: Engine, sql: string): Promise<WriteInfo>;
  /** AI read path: guard, then a read-only session with the default limits. Fails with `guard_rejected`, `read_only_violation` or `timeout`. */
  runAiRead(connId: string, database: string | null, sql: string): Promise<AiReadResult>;
}
