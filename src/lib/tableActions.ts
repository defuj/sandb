import type { ColumnInfo, Engine } from './api';
import type { Conn } from './state.svelte';
import { quoteIdent } from './results';

/** A table or view from the connection tree. `schema` is null on MySQL and SQLite. */
export type TableRef = { database: string; schema: string | null; name: string; kind: 'table' | 'view' };

export type TableAction = 'view' | 'count' | 'ddl' | 'select' | 'insert' | 'exportDdl' | 'exportSql' | 'exportCsv' | 'truncate' | 'drop';

/** A Truncate or Drop that waits for the user to confirm it. */
export type Danger = { action: 'truncate' | 'drop'; c: Conn; t: TableRef };

export const qualified = (e: Engine, t: TableRef) => (t.schema ? quoteIdent(e, t.schema) + '.' : '') + quoteIdent(e, t.name);

/** Unquoted, for labels and the clipboard. */
export const displayName = (t: TableRef) => (t.schema ? t.schema + '.' : '') + t.name;

export const selectSql = (e: Engine, t: TableRef, cols: ColumnInfo[]) =>
  `SELECT ${cols.map((c) => quoteIdent(e, c.name)).join(', ') || '*'}\nFROM ${qualified(e, t)};`;

// A value of the column's kind, so the template runs once the user edits it.
function placeholder(e: Engine, dataType: string) {
  if (/bool/i.test(dataType)) return e === 'pg' ? 'false' : '0';
  if (/int|serial|numeric|decimal|real|double|float|money|bit/i.test(dataType)) return '0';
  return "''";
}

export const insertSql = (e: Engine, t: TableRef, cols: ColumnInfo[]) =>
  `INSERT INTO ${qualified(e, t)} (${cols.map((c) => quoteIdent(e, c.name)).join(', ')})\nVALUES (${cols.map((c) => placeholder(e, c.dataType)).join(', ')});`;

/** SQLite has no TRUNCATE; a DELETE without WHERE uses its truncate optimization. */
export function dangerSql(e: Engine, t: TableRef, a: 'truncate' | 'drop') {
  if (a === 'drop') return `DROP ${t.kind === 'view' ? 'VIEW' : 'TABLE'} ${qualified(e, t)};`;
  return e === 'sqlite' ? `DELETE FROM ${qualified(e, t)};` : `TRUNCATE TABLE ${qualified(e, t)};`;
}
