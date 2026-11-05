import { nsOf, pkOf, type Cell, type QueryResult, type Row } from './mock';
import { logQuery } from './audit.svelte';
import { api } from './api';
import type { Conn } from './state.svelte';

/** Pending grid edits. Keys of `changed`/`deleted` are row indexes in the current result. */
export const edits = $state({
  on: false,
  changed: {} as Record<number, Row>,
  added: [] as { key: number; row: Row }[],
  deleted: {} as Record<number, true>,
  error: null as { message: string; key: string } | null,
});

let addSeq = 0;

export function pendingCount() {
  const del = Object.keys(edits.deleted).length;
  const upd = Object.keys(edits.changed).filter((i) => !edits.deleted[+i]).length;
  return upd + edits.added.length + del;
}

export function resetEdits(keepOn = false) {
  edits.changed = {};
  edits.added = [];
  edits.deleted = {};
  edits.error = null;
  if (!keepOn) edits.on = false;
}

/** Asks before pending edits are thrown away. Returns true if it is fine to continue. */
export function confirmDiscard() {
  if (pendingCount() === 0) return true;
  if (!confirm(`Discard ${pendingCount()} unsaved change(s)?`)) return false;
  resetEdits(true);
  return true;
}

/** Why the result cannot be edited, or null if it can. Live metadata loads on the first edit attempt. */
export function editBlocker(r: QueryResult | null): string | null {
  if (!r || r.error || r.note || !r.sql || r.isRead === false) return 'Run a SELECT to edit its rows';
  if (r.live) return r.editReason ?? null;
  if (!r.table) return 'Run a SELECT to edit its rows';
  const sql = r.sql.replace(/--.*$/gm, '');
  if (/^\s*with\b/i.test(sql) || /\bunion\b/i.test(sql)) return 'Read-only: result is not from a single table';
  if (/\bjoin\b/i.test(sql) || /\bfrom\s+[\w.]+(\s+\w+)?\s*,/i.test(sql)) return 'Read-only: result joins several tables';
  if (/\b(group\s+by|distinct)\b|\b(count|sum|avg|min|max)\s*\(/i.test(sql)) return 'Read-only: result has aggregates';
  const list = sql.match(/select\s+([\s\S]*?)\s+from\s/i)?.[1]?.trim() ?? '';
  if (list !== '*' && !/^\w+(\s*,\s*\w+)*$/.test(list)) return 'Read-only: result has computed columns';
  const cols = list === '*' ? r.columns : list.split(/\s*,\s*/);
  const pk = pkOf(r.table);
  if (!pk.every((k) => cols.includes(k))) return `Read-only: primary key (${pk.join(', ')}) is not in the result`;
  return null;
}

const editLookups = new WeakMap<QueryResult, Promise<string | null>>();

/** Loads and caches the source table and primary key for a live result. */
export function prepareEdit(r: QueryResult, conn: Conn): Promise<string | null> {
  const blocked = editBlocker(r);
  if (blocked || !r.live || r.edit) return Promise.resolve(blocked);
  const existing = editLookups.get(r);
  if (existing) return existing;
  const lookup = api.editableResult({ connId: conn.id, database: r.database ?? conn.database, sql: r.sql! })
    .then((meta) => {
      if (meta.reason) r.editReason = meta.reason;
      else r.edit = meta;
      return editBlocker(r);
    })
    .catch((e) => {
      editLookups.delete(r);
      return e instanceof Error ? e.message : String(e);
    });
  editLookups.set(r, lookup);
  return lookup;
}

/** Stores a cell edit; editing a value back to the original clears it. */
export function setCell(i: number, orig: Row, col: string, v: Cell) {
  const next = { ...edits.changed[i] };
  if (orig[col] === v) delete next[col];
  else next[col] = v;
  if (Object.keys(next).length) edits.changed[i] = next;
  else delete edits.changed[i];
}

export function addRow(row: Row) {
  edits.added.push({ key: ++addSeq, row });
}

/** Turns typed text into a value compatible with the column. Returns undefined for invalid numbers, booleans or JSON. */
export function parseInput(text: string, sample: Cell, dataType = ''): Cell | undefined {
  const numeric = typeof sample === 'number' || /^(int|integer|smallint|tinyint|mediumint|bigint|int[248]|serial|bigserial|smallserial|real|double|float|float[48]|decimal|numeric|number|money)\b/i.test(dataType);
  if (numeric) {
    if (text.trim() === '') return null;
    if (typeof sample !== 'number') return /^-?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text.trim()) ? text : undefined;
    const n = Number(text);
    return Number.isFinite(n) ? n : undefined;
  }
  if (typeof sample === 'boolean' || /bool/i.test(dataType)) {
    if (!/^(true|false|1|0)$/i.test(text.trim())) return undefined;
    return /^(true|1)$/i.test(text.trim());
  }
  if (typeof sample === 'object' && sample !== null) {
    try { return JSON.parse(text) as Cell; } catch { return undefined; }
  }
  return text;
}

function lit(v: Cell, engine: Conn['engine']) {
  if (v === null) return 'NULL';
  if (typeof v === 'number') return String(v);
  if (typeof v === 'boolean') return engine === 'pg' ? String(v).toUpperCase() : v ? '1' : '0';
  // Edit mode runs on mock rows only, which hold no JSON or bytes; serialize just in case.
  let s = (typeof v === 'object' ? JSON.stringify(v) : v).replace(/'/g, "''");
  if (engine === 'mysql') s = s.replace(/\\/g, '\\\\');
  return `'${s}'`;
}

function ident(name: string, engine: Conn['engine']) {
  if (/^[a-z_][a-z0-9_]*$/.test(name)) return name;
  return engine === 'mysql' ? `\`${name.replace(/`/g, '``')}\`` : `"${name.replace(/"/g, '""')}"`;
}

export interface Stmt {
  kind: 'UPDATE' | 'INSERT' | 'DELETE';
  sql: string;
  /** Grid row key: `r<index>` for result rows, `a<key>` for new rows. */
  key: string;
  pk: Row;
  set: Row;
}

export function buildStatements(r: QueryResult, conn: Conn): Stmt[] {
  const meta = r.live ? r.edit : undefined;
  const t = meta?.table ?? r.table!;
  const e = conn.engine;
  const namespace = meta?.schema ?? (e === 'pg' ? nsOf(e, t) : '');
  const table = (namespace ? ident(namespace, e) + '.' : '') + ident(t, e);
  const pk = meta?.pk ?? pkOf(t);
  const where = (row: Row) => pk.map((k) => `${ident(k, e)} = ${lit(row[k], e)}`).join(' AND ');
  const pkOfRow = (row: Row) => Object.fromEntries(pk.map((k) => [k, row[k]]));
  const out: Stmt[] = [];
  // Deletes run first so an update or insert can reuse a deleted row's primary key.
  for (const i of Object.keys(edits.deleted)) {
    const row = r.rows[+i];
    out.push({ kind: 'DELETE', key: 'r' + i, pk: pkOfRow(row), set: {}, sql: `DELETE FROM ${table} WHERE ${where(row)};` });
  }
  for (const [i, set] of Object.entries(edits.changed)) {
    if (edits.deleted[+i]) continue;
    const row = r.rows[+i];
    const assigns = Object.entries(set).map(([c, v]) => `${ident(c, e)} = ${lit(v, e)}`).join(', ');
    out.push({ kind: 'UPDATE', key: 'r' + i, pk: pkOfRow(row), set, sql: `UPDATE ${table} SET ${assigns} WHERE ${where(row)};` });
  }
  for (const { key, row } of edits.added) {
    const cols = r.columns.filter((c) => row[c] !== null);
    out.push({ kind: 'INSERT', key: 'a' + key, pk: pkOfRow(row), set: row, sql: `INSERT INTO ${table} (${cols.map((c) => ident(c, e)).join(', ')}) VALUES (${cols.map((c) => lit(row[c], e)).join(', ')});` });
  }
  return out;
}

/** Saves every operation in one backend transaction and keeps the failed row highlighted. */
export async function saveEdits(r: QueryResult, conn: Conn, tabId?: string): Promise<boolean> {
  const stmts = buildStatements(r, conn);
  if (!stmts.length) return true;
  try {
    const out = await api.applyEdits({
      connId: conn.id,
      database: r.database ?? conn.database,
      sql: r.sql!,
      tabId,
      operations: stmts.map((s) => ({
        key: s.key,
        kind: s.kind.toLowerCase() as 'update' | 'insert' | 'delete',
        pk: s.pk,
        set: s.kind === 'INSERT' ? Object.fromEntries(Object.entries(s.set).filter(([, v]) => v !== null)) : s.set,
      })),
    });
    if (out.failed) {
      const failed = stmts[out.failed.index];
      const message = /rolled back/i.test(out.failed.message) ? out.failed.message : `${out.failed.message} Transaction rolled back, nothing was saved.`;
      edits.error = { message, key: failed?.key ?? stmts[0].key };
      if (failed) logQuery({ kind: 'WRITE', source: 'You', sql: failed.sql, result: { status: 'error', message: out.failed.message }, ms: 0, conn });
      return false;
    }
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    edits.error = { message, key: stmts[0].key };
    logQuery({ kind: 'WRITE', source: 'You', sql: stmts[0].sql, result: { status: 'error', message }, ms: 0, conn });
    return false;
  }
  for (const s of stmts) logQuery({ kind: 'WRITE', source: 'You', sql: s.sql, result: { status: 'ok', rows: 1 }, ms: 0, conn });
  resetEdits(true);
  return true;
}
