import type { BytesValue, QueryRun, SchemaSnapshot, SnapshotTable, StatementResult } from './api';
import type { Cell, Engine, QueryResult, Schema } from './mock';

export const isBytes = (v: unknown): v is BytesValue => !!v && typeof v === 'object' && '$bytes' in v && 'len' in v;

/** Short text for a JSON or bytes cell. */
export function formatCell(v: object): string {
  if (isBytes(v)) {
    const head = atob(v.$bytes.slice(0, 12)).slice(0, 8);
    const hex = [...head].map((c) => c.charCodeAt(0).toString(16).padStart(2, '0')).join('');
    return `\\x${hex}${v.len > 8 ? '…' : ''} (${v.len} B)`;
  }
  const s = JSON.stringify(v);
  return s.length > 200 ? s.slice(0, 199) + '…' : s;
}

/** CSV text of a cell: NULL is empty, bytes are base64 (the loaded preview), JSON is JSON. */
export function csvCell(v: Cell): string {
  const s = v == null ? '' : typeof v === 'object' ? (isBytes(v) ? v.$bytes : JSON.stringify(v)) : String(v);
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** Always quoted: names from the tree may be reserved words, mixed case or contain any character. */
export function quoteIdent(engine: Engine, name: string): string {
  return engine === 'mysql' ? '`' + name.replace(/`/g, '``') + '`' : '"' + name.replace(/"/g, '""') + '"';
}

/** Palette and autocomplete name of a snapshot table: PG tables outside `public` are `schema.table`. */
export const snapshotKey = (t: SnapshotTable, engine: Engine) => (engine === 'pg' && t.schema && t.schema !== 'public' ? `${t.schema}.${t.name}` : t.name);

/** Table → columns for autocomplete and the palette, keyed by `snapshotKey`. */
export function snapshotSchema(s: SchemaSnapshot | null, engine: Engine): Schema {
  const out: Schema = {};
  for (const t of s?.tables ?? []) out[snapshotKey(t, engine)] = t.columns.map((c) => c.name);
  return out;
}

const shortSql = (sql: string) => {
  const s = sql.replace(/\s+/g, ' ').trim();
  return s.length > 60 ? s.slice(0, 59) + '…' : s;
};

const outcome = (s: StatementResult) =>
  s.columns.length ? `${s.rowCount}${s.truncated ? '+' : ''} row${s.rowCount === 1 ? '' : 's'}` : `${s.affected ?? 0} affected`;

// Grid rows are keyed by column name, so a repeated name (`SELECT a.id, b.id`) gets a suffix.
function uniqueNames(names: string[]) {
  const seen = new Map<string, number>();
  return names.map((n) => {
    const k = (seen.get(n) ?? 0) + 1;
    seen.set(n, k);
    return k === 1 ? n : `${n} (${k})`;
  });
}

/** One grid result for a statement that returned columns. */
function rowResult(s: StatementResult, mock: boolean, limit: number, database: string): QueryResult {
  const columns = uniqueNames(s.columns.map((c) => c.name));
  const rows = s.rows.map((r) => Object.fromEntries(columns.map((c, i) => [c, r[i] as Cell])));
  return {
    columns,
    types: s.columns.map((c) => c.dataType),
    rows,
    truncated: s.truncated,
    limit,
    isRead: s.isRead,
    sortable: s.isRead && s.sortable,
    table: s.table,
    sql: s.sql,
    ms: s.elapsedMs,
    database,
    live: !mock,
  };
}

/**
 * The result tabs of a run: one per statement that returned columns, plus one Messages tab for
 * statements that changed rows or returned none. The active tab is the last grid result.
 */
export function toResults(run: QueryRun, mock: boolean, limit: number, database: string): { results: QueryResult[]; active: number } {
  const results: QueryResult[] = [];
  const messages: string[] = [];
  let lastRow = -1;
  for (const [i, s] of run.statements.entries()) {
    if (s.columns.length) {
      results.push(rowResult(s, mock, limit, database));
      lastRow = results.length - 1;
    } else {
      messages.push(`${i + 1}. ${shortSql(s.sql)} — ${outcome(s)} · ${s.elapsedMs} ms`);
    }
  }
  if (messages.length) {
    const only = run.statements.length === 1 ? run.statements[0] : undefined;
    const note = only
      ? mock
        ? 'Statement ran (mock). 0 rows affected.'
        : `Statement ran. ${only.affected ?? 0} row${only.affected === 1 ? '' : 's'} affected.`
      : `${run.statements.length} statements ran.`;
    results.push({
      columns: [],
      rows: [],
      ms: run.elapsedMs,
      note,
      title: only ? undefined : 'Messages',
      summary: only ? undefined : messages,
      database,
      live: !mock,
    });
    if (lastRow < 0) lastRow = results.length - 1;
  }
  const notices = run.notices?.length ? run.notices : undefined;
  const last = results.at(-1);
  if (notices && last) last.notices = notices;
  return { results, active: Math.max(0, lastRow) };
}

/** The active result of a run, as a single grid result. */
export function toResult(run: QueryRun, mock: boolean, limit: number, database: string): QueryResult {
  const { results, active } = toResults(run, mock, limit, database);
  return results[active] ?? { columns: [], rows: [], ms: run.elapsedMs, note: 'Statement ran.' };
}
