import { nsOf, pkOf, tableData, type Cell, type QueryResult, type Row, type Schema } from './mock';
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

/** Why the result cannot be edited, or null if it can. */
export function editBlocker(r: QueryResult | null): string | null {
  if (api.kind !== 'mock') return 'Edit mode comes in a later phase';
  if (!r || r.error || r.note || !r.table || !r.sql) return 'Run a SELECT to edit its rows';
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

/** Turns typed text into a value of the same type as the column sample. Returns undefined if a number column gets text that is not a number. */
export function parseInput(text: string, sample: Cell): Cell | undefined {
  if (typeof sample === 'number') {
    if (text.trim() === '') return null;
    const n = Number(text);
    return Number.isFinite(n) ? n : undefined;
  }
  if (typeof sample === 'boolean' && /^(true|false|1|0)$/i.test(text.trim())) return /^(true|1)$/i.test(text.trim());
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
  const t = r.table!;
  const e = conn.engine;
  const table = (e === 'pg' ? nsOf(e, t) + '.' : '') + ident(t, e);
  const pk = pkOf(t);
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

/** Mock transaction: checks every statement first, so a failure applies nothing. */
export function saveEdits(r: QueryResult, conn: Conn, schema: Schema): boolean {
  const stmts = buildStatements(r, conn);
  const pk = pkOf(r.table!);
  const bad = stmts.find((s) => s.kind !== 'DELETE' && pk.some((k) => k in s.set && (s.set[k] === null || s.set[k] === '')));
  if (bad) {
    const col = pk.find((k) => bad.set[k] === null || bad.set[k] === '')!;
    const message = `null value in column "${col}" of relation "${r.table}" violates not-null constraint. Transaction rolled back, nothing was saved.`;
    edits.error = { message, key: bad.key };
    logQuery({ kind: 'WRITE', source: 'You', sql: bad.sql, result: { status: 'error', message }, ms: 3, conn });
    return false;
  }
  const d = tableData(schema, r.table!);
  const same = (a: Row, b: Row) => pk.every((k) => a[k] === b[k]);
  for (const s of stmts) {
    if (s.kind === 'INSERT') d.added.push({ ...s.set });
    else
      for (const list of [d.base, d.added]) {
        const idx = list.findIndex((row) => same(row, s.pk));
        if (idx < 0) continue;
        if (s.kind === 'UPDATE') list[idx] = { ...list[idx], ...s.set };
        else list.splice(idx, 1);
      }
    logQuery({ kind: 'WRITE', source: 'You', sql: s.sql, result: { status: 'ok', rows: 1 }, ms: 2 + Math.floor(Math.random() * 6), conn });
  }
  resetEdits(true);
  return true;
}
