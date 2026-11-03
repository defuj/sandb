import { ApiError, type Backend, type Connection, type ConnectionInput, type Engine, type StatementResult, type CellValue, type GuardCheck, type WriteInfo } from './types';
import { schemas, nsOf, pkOf, colType, runMock, statementKind } from '../mock';

// In-memory backend: the prototype behaviour, used when there is neither Tauri nor the dev bridge.

const versions: Record<Engine, string> = { pg: 'PostgreSQL 16.4', mysql: 'MySQL 8.4', sqlite: 'SQLite 3.46' };

export const seedConnections: Connection[] = [
  { id: 'prod', name: 'prod-analytics', engine: 'pg', version: 'PostgreSQL 16.4', env: 'production', host: 'prod-analytics.db.internal', port: 5432, database: 'analytics_prod', user: 'analyst', ssl: 'require', file: '', aiWrite: false, readOnly: false },
  { id: 'shop', name: 'shop-staging', engine: 'mysql', version: 'MySQL 8.4', env: 'staging', host: 'shop-staging.db.internal', port: 3306, database: 'shop', user: 'app', ssl: 'prefer', file: '', aiWrite: false, readOnly: false },
  { id: 'notes', name: 'notes.db', engine: 'sqlite', version: 'SQLite 3.46', env: 'local', host: '', port: 0, database: 'main', user: '', ssl: 'prefer', file: '~/data/notes.db', aiWrite: false, readOnly: false },
];

let list: Connection[] = seedConnections.map((c) => ({ ...c }));

// Tab sessions, as the backend keeps them: only writable Production connections open one, and only while its
// transaction holds changes that wait for Commit or Rollback.
const sessions = new Map<string, { connId: string }>();

// Rough stand-ins for the backend's statement facts; never a security boundary.
const bare = (p: string) => p.replace(/--.*$/gm, '').trim();
// Session and transaction control: runs on a read-only connection and changes no data.
const control = (p: string) => /^(set|reset|begin|start|commit|rollback|end|savepoint|release|pragma)\b/i.test(bare(p));
// Loose mirror of `check_read_only`: statements that would make a read-only session writable.
function escapesReadOnly(sql: string) {
  const p = bare(sql);
  if (/\b(default_transaction_read_only|transaction_read_only|tx_read_only|query_only)\b|\bread\s+write\b|^set\s+session\s+characteristics\b|^(reset|discard)\s+all\b/i.test(p)) return true;
  return /^set\s+(session\s+|global\s+)?transaction\b/i.test(p) && !/^set\s+(session\s+)?transaction\s+read\s+only$/i.test(p);
}

function dropSessionsOf(connId: string) {
  for (const [tab, s] of sessions) if (s.connId === connId) sessions.delete(tab);
}

const newId = (name: string) => name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-') + '-' + Date.now().toString(36);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function make(id: string, i: ConnectionInput, version: string): Connection {
  const sqlite = i.engine === 'sqlite';
  return {
    id,
    name: i.name.trim(),
    engine: i.engine,
    version,
    env: i.env,
    host: sqlite ? '' : i.host.trim(),
    port: sqlite ? 0 : i.port,
    database: sqlite ? 'main' : i.database.trim(),
    user: sqlite ? '' : i.user.trim(),
    ssl: i.ssl,
    file: sqlite ? i.file.trim() : '',
    aiWrite: i.aiWrite,
    readOnly: i.readOnly,
  };
}

// Same mapping as the prototype: each engine shows one of the mock schemas.
const mockSchemaOf: Record<Engine, string> = { pg: 'prod', mysql: 'shop', sqlite: 'notes' };
const mockSchema = (c: Connection) => schemas[mockSchemaOf[c.engine]];

function find(id: string) {
  const c = list.find((x) => x.id === id);
  if (!c) throw new ApiError({ code: 'not_found', message: `Connection ${id} not found` });
  return c;
}

// Keyword stand-ins for the Rust guard; the mock has no parser. Never a security boundary.
function mockGuard(sql: string): GuardCheck {
  const parts = sql.split(';').map((p) => p.trim()).filter((p) => statementKind(p) !== 'empty');
  if (parts.length !== 1) return { allowed: false, reason: parts.length ? `Exactly one statement is allowed, found ${parts.length}` : 'The query is empty' };
  const kind = parts[0].split(/\s+/)[0].toUpperCase();
  if (statementKind(parts[0]) !== 'read') return { allowed: false, reason: `${kind} is not a read (mock guard)`, kind };
  if (/\b(analy[sz]e|into|for\s+update|for\s+share)\b/i.test(parts[0])) return { allowed: false, reason: 'Not a plain read (mock guard)', kind };
  return { allowed: true, kind: kind === 'WITH' ? 'SELECT' : kind };
}

function mockClassifyWrite(sql: string): WriteInfo {
  const parts = sql.split(';').map((p) => p.trim()).filter((p) => statementKind(p) !== 'empty');
  const info: WriteInfo = { statements: [], warnings: [] };
  if (parts.length > 1) info.warnings.push({ code: 'multiple_statements', message: `${parts.length} statements will run` });
  parts.forEach((p, i) => {
    const kind = p.split(/\s+/)[0].toUpperCase();
    const table = p.match(/(?:update|from|into|table)\s+([\w.]+)/i)?.[1];
    info.statements.push({ kind, table });
    const where = /\bwhere\b/i.test(p);
    if (kind === 'UPDATE' && !where) info.warnings.push({ code: 'update_without_where', message: `UPDATE without WHERE changes every row of ${table ?? 'the table'}`, statement: i });
    if (kind === 'DELETE' && !where) info.warnings.push({ code: 'delete_without_where', message: `DELETE without WHERE removes every row of ${table ?? 'the table'}`, statement: i });
    if (kind === 'DROP') info.warnings.push({ code: 'drop', message: `DROP ${table ?? ''} cannot be undone`, statement: i });
    if (kind === 'TRUNCATE') info.warnings.push({ code: 'truncate', message: `TRUNCATE removes every row of ${table ?? 'the table'}`, statement: i });
  });
  return info;
}

export const mockBackend: Backend = {
  kind: 'mock',
  async listConnections() {
    return list.map((c) => ({ ...c }));
  },
  async createConnection(input) {
    const c = make(newId(input.name), input, input.version || versions[input.engine]);
    list.push(c);
    return { ...c };
  },
  // Saving or deleting a connection drops its tab sessions, as the backend does.
  async updateConnection(id, input) {
    const cur = find(id);
    const c = make(id, input, input.version || cur.version);
    list = list.map((x) => (x.id === id ? c : x));
    dropSessionsOf(id);
    return { ...c };
  },
  async duplicateConnection(id, input) {
    const src = find(id);
    const i: ConnectionInput = input ?? { ...src, name: `${src.name} (copy)`, aiWrite: false };
    const c = make(newId(i.name), i, src.version);
    list.push(c);
    return { ...c };
  },
  async deleteConnection(id) {
    find(id);
    list = list.filter((x) => x.id !== id);
    dropSessionsOf(id);
  },
  async testConnection(input) {
    await sleep(900);
    return { serverVersion: versions[input.engine], latencyMs: 12 };
  },
  // Schema browsing and queries run on the prototype data in ../mock.ts.
  async listDatabases(connId) {
    return [find(connId).database];
  },
  async listSchemas(connId) {
    const c = find(connId);
    if (c.engine !== 'pg') return [];
    return [...new Set(Object.keys(mockSchema(c)).map((t) => nsOf(c.engine, t)))];
  },
  async listTables(connId, _database, schema) {
    const c = find(connId);
    return Object.keys(mockSchema(c))
      .filter((t) => c.engine !== 'pg' || nsOf(c.engine, t) === (schema ?? 'public'))
      .map((name) => ({ name, kind: 'table' as const }));
  },
  async listColumns(connId, _database, _schema, table) {
    const s = mockSchema(find(connId));
    if (!s[table]) throw new ApiError({ code: 'not_found', message: `Table ${table} not found` });
    const pk = pkOf(table);
    return s[table].map((name) => ({ name, dataType: colType(s, table, name), nullable: !pk.includes(name), isPk: pk.includes(name), pkOrdinal: pk.includes(name) ? pk.indexOf(name) + 1 : undefined }));
  },
  async schemaSnapshot(connId, database) {
    const c = find(connId);
    const s = mockSchema(c);
    return {
      database,
      truncated: false,
      tables: Object.entries(s).map(([name, cols]) => ({ schema: c.engine === 'pg' ? nsOf(c.engine, name) : undefined, name, kind: 'table' as const, columns: cols.map((col) => ({ name: col, dataType: colType(s, name, col) })) })),
    };
  },
  async runQuery({ connId, sql, limit, tabId }) {
    const c = find(connId);
    const s = mockSchema(c);
    const parts = sql.split(';').map((p) => p.trim()).filter((p) => statementKind(p) !== 'empty');
    if (!parts.length) throw new ApiError({ code: 'invalid_input', message: 'Nothing to run' });
    // Refused before any statement runs, as the backend does: one write refuses the whole run.
    if (c.readOnly) {
      const refused = (m: string) => new ApiError({ code: 'connection_read_only', message: `Connection "${c.name}" is read-only: ${m}` });
      if (parts.some(escapesReadOnly)) throw refused("this statement would change the session's transaction mode");
      if (parts.some((p) => statementKind(p) === 'write' && !control(p))) throw refused('writes are refused');
    }
    // Session mode: the backend begins a transaction for the run and rolls a clean one back at the end, so a
    // tab keeps a session only while it holds changes (a typed COMMIT / ROLLBACK ends it as well).
    const manual = !!tabId && c.env === 'production' && !c.readOnly;
    let pending = manual && sessions.get(tabId)?.connId === connId;
    const statements: StatementResult[] = [];
    let total = 0;
    try {
      for (const [i, part] of parts.entries()) {
        const r = runMock(part, s, c);
        if (r.error) throw new ApiError({ code: 'database', message: parts.length > 1 ? `Statement ${i + 1} of ${parts.length}: ${r.error}` : r.error });
        if (/^(commit|rollback|end)\b/i.test(bare(part))) pending = false;
        else if (statementKind(part) === 'write' && !control(part)) pending = true;
        total += r.ms;
        const rows = r.rows.slice(0, limit);
        statements.push({
          sql: part,
          columns: r.columns.map((name) => ({ name, dataType: r.table ? colType(s, r.table, name) : 'text' })),
          rows: rows.map((row) => r.columns.map((col) => row[col] as CellValue)),
          rowCount: rows.length,
          truncated: r.rows.length > rows.length,
          affected: r.columns.length ? undefined : 0,
          isRead: parts.length === 1 && statementKind(part) === 'read',
          elapsedMs: r.ms,
          table: r.table,
        });
      }
    } finally {
      if (manual) {
        if (pending) sessions.set(tabId, { connId });
        else sessions.delete(tabId);
      }
    }
    return { statements, elapsedMs: total };
  },
  async cancelQuery() {},
  // Mock runs are instant; a run always finishes before progress could be polled.
  async queryProgress() {
    return null;
  },
  async commitTransaction(tabId) {
    if (!sessions.delete(tabId)) throw new ApiError({ code: 'invalid_input', message: 'No transaction is open in this tab' });
  },
  async rollbackTransaction(tabId) {
    if (!sessions.delete(tabId)) throw new ApiError({ code: 'invalid_input', message: 'No transaction is open in this tab' });
  },
  async tabStatus(tabId) {
    return sessions.has(tabId) ? { open: true, txOpen: true, pending: true, aborted: false } : null;
  },
  async closeTabSession(tabId) {
    sessions.delete(tabId);
  },
  async quitApp() {},
  async exportCsv(_connId, _database, _sql, _tabId) {
    throw new ApiError({ code: 'invalid_input', message: 'CSV export to a file needs the desktop app' });
  },
  // The prototype data has types and primary keys only, so the DDL has no defaults or other constraints.
  async tableDdl(connId, _database, schema, table) {
    const c = find(connId);
    const s = mockSchema(c);
    if (!s[table]) throw new ApiError({ code: 'not_found', message: `Table ${table} not found` });
    const pk = pkOf(table);
    const lines = s[table].map((col) => `  ${col} ${colType(s, table, col)}${pk.includes(col) ? ' NOT NULL' : ''}`);
    if (pk.length) lines.push(`  PRIMARY KEY (${pk.join(', ')})`);
    return `CREATE TABLE ${schema ? schema + '.' : ''}${table} (\n${lines.join(',\n')}\n);`;
  },
  async exportTableSql() {
    throw new ApiError({ code: 'invalid_input', message: 'SQL export to a file needs the desktop app' });
  },
  async guardCheck(_engine, sql) {
    return mockGuard(sql);
  },
  async classifyWrite(_engine, sql) {
    return mockClassifyWrite(sql);
  },
  async runAiRead(connId, database, sql) {
    const g = mockGuard(sql);
    if (!g.allowed) throw new ApiError({ code: 'guard_rejected', message: `Blocked by the read-only guard: ${g.reason}` });
    const run = await mockBackend.runQuery({ connId, database: database ?? '', sql, queryId: 'ai', limit: 500 });
    const s = run.statements[0];
    return { columns: s.columns, rows: s.rows, truncated: s.truncated, elapsedMs: s.elapsedMs };
  },
};
