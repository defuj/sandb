/** `object`: JSON values and `{ $bytes, len }` from a real database. */
export type Cell = string | number | boolean | null | object;
export type Row = Record<string, Cell>;
export interface QueryResult {
  columns: string[];
  rows: Row[];
  ms: number;
  note?: string;
  error?: string;
  /** Source table and SQL of a SELECT result; edit mode uses them. */
  table?: string;
  sql?: string;
  /** Set by real runs: column types, more rows exist, the row limit used, the database. */
  types?: string[];
  truncated?: boolean;
  limit?: number;
  database?: string;
  /** One line per statement of a multi-statement run. */
  summary?: string[];
  /** The shown statement is a plain read, so "Load more" and export may run it again. */
  isRead?: boolean;
  /** Backend notices, e.g. a rolled-back open transaction. */
  notices?: string[];
  /** From a real database (not the mock); edit mode is mock-only for now. */
  live?: boolean;
}

export type Schema = Record<string, string[]>;
export type Engine = keyof typeof defaultNs;

export const schemas: Record<string, Schema> = {
  prod: {
    customers: ['id', 'name', 'email', 'country', 'plan', 'created_at'],
    orders: ['id', 'customer', 'status', 'total', 'paid', 'shipped_at'],
    order_items: ['id', 'order_id', 'product_id', 'qty', 'price'],
    products: ['id', 'sku', 'name', 'price', 'stock'],
    invoices: ['id', 'order_id', 'number', 'amount', 'issued_at'],
    daily_revenue: ['id', 'day', 'orders', 'revenue', 'refunds'],
    events: ['id', 'user_id', 'name', 'source', 'created_at'],
  },
  shop: {
    users: ['id', 'name', 'email', 'country', 'created_at'],
    carts: ['id', 'user_id', 'status', 'total', 'updated_at'],
    cart_items: ['id', 'cart_id', 'product_id', 'qty', 'price'],
    payments: ['id', 'cart_id', 'status', 'amount', 'paid_at'],
  },
  notes: {
    notes: ['id', 'title', 'body', 'pinned', 'created_at'],
    tags: ['id', 'name'],
    note_tags: ['note_id', 'tag_id'],
  },
};

// Tables outside the engine's default schema. Table names are unique across the mock data.
const tableNs: Record<string, string> = { daily_revenue: 'analytics', events: 'analytics' };
const defaultNs = { pg: 'public', mysql: 'shop', sqlite: 'main' } as const;
export const nsOf = (engine: keyof typeof defaultNs, table: string) => tableNs[table] ?? defaultNs[engine];

const names = ['Livia Saris', 'Jaydon Aminoff', 'Maria Lubin', 'Ann Press', 'John Clarkson', 'Aspen Lubin', 'Kierra Gouse', 'Omar Dias', 'Mila Novak', 'Theo Grant'];
const statuses = ['paid', 'shipped', 'pending', 'cancelled', 'refunded'];
const countries = ['ID', 'US', 'DE', 'SG', 'JP', 'NL'];
const plans = ['free', 'pro', 'team'];

function rng(seed: number) {
  return () => (seed = (seed * 16807) % 2147483647) / 2147483647;
}
const pick = <T,>(r: () => number, a: T[]) => a[Math.floor(r() * a.length)];
const day = (r: () => number) => new Date(2026, 9, 1 + Math.floor(r() * 30)).toISOString().slice(0, 10);
const money = (r: () => number, max: number) => Math.round(r() * max * 100) / 100 + 5;

const gen: Record<string, (i: number, r: () => number) => Row> = {
  customers: (i, r) => {
    const n = pick(r, names);
    return { id: 501 + i, name: n, email: n.toLowerCase().replace(' ', '.') + '@mail.com', country: pick(r, countries), plan: pick(r, plans), created_at: day(r) };
  },
  orders: (i, r) => ({ id: 10234 + i, customer: pick(r, names), status: pick(r, statuses), total: money(r, 900), paid: r() > 0.3, shipped_at: r() > 0.4 ? day(r) : null }),
  order_items: (i, r) => ({ id: 1 + i, order_id: 10234 + Math.floor(r() * 120), product_id: 1 + Math.floor(r() * 40), qty: 1 + Math.floor(r() * 5), price: money(r, 200) }),
  products: (i, r) => ({ id: 1 + i, sku: 'SKU-' + (1000 + i), name: pick(r, ['Desk', 'Lamp', 'Chair', 'Monitor', 'Keyboard']) + ' ' + (i + 1), price: money(r, 400), stock: r() > 0.85 ? null : Math.floor(r() * 300) }),
  invoices: (i, r) => ({ id: 1 + i, order_id: 10234 + i, number: 'INV-2026-' + String(i + 1).padStart(4, '0'), amount: money(r, 900), issued_at: day(r) }),
  daily_revenue: (i, r) => ({ id: 1 + i, day: new Date(2026, 5, 1 + i).toISOString().slice(0, 10), orders: 20 + Math.floor(r() * 180), revenue: money(r, 25000), refunds: money(r, 900) }),
  events: (i, r) => ({ id: 90001 + i, user_id: 501 + Math.floor(r() * 120), name: pick(r, ['page_view', 'signup', 'login', 'add_to_cart', 'checkout']), source: pick(r, ['web', 'ios', 'android']), created_at: day(r) }),
};

const words = ['Weekly sync', 'Ideas', 'Groceries', 'Reading list', 'Bug notes', 'Trip plan', 'Draft post'];

// Fills tables without a hand-written generator, guessing values from column names.
function genericRow(cols: string[], i: number, r: () => number): Row {
  const row: Row = {};
  for (const c of cols) {
    if (c === 'id') row[c] = 1 + i;
    else if (c.endsWith('_id')) row[c] = 1 + Math.floor(r() * 120);
    else if (c === 'name') row[c] = pick(r, names);
    else if (c === 'email') row[c] = pick(r, names).toLowerCase().replace(' ', '.') + '@mail.com';
    else if (c === 'country') row[c] = pick(r, countries);
    else if (c === 'status') row[c] = pick(r, statuses);
    else if (c.endsWith('_at')) row[c] = r() > 0.15 ? day(r) : null;
    else if (['total', 'amount', 'price'].includes(c)) row[c] = money(r, 600);
    else if (c === 'qty') row[c] = 1 + Math.floor(r() * 5);
    else if (c === 'pinned') row[c] = r() > 0.7;
    else if (c === 'title') row[c] = pick(r, words);
    else if (c === 'body') row[c] = pick(r, ['Call Ann about Q4', 'Milk, eggs, coffee', 'Fix login timeout', 'Book train to Bandung']);
    else row[c] = pick(r, words);
  }
  return row;
}

// note_tags stays at 120: genericRow draws *_id values from 1..120, so more rows repeat its (note_id, tag_id) key.
export function rowsFor(table: string, cols: string[], n = table === 'note_tags' ? 120 : 500): Row[] {
  const r = rng(table.length * 7919);
  const g = gen[table];
  return Array.from({ length: n }, (_, i) => (g ? g(i, r) : genericRow(cols, i, r)));
}

// Primary keys per table; tables not listed here use `id`.
const pks: Record<string, string[]> = { note_tags: ['note_id', 'tag_id'] };
export const pkOf = (table: string) => pks[table] ?? ['id'];

/** Display type of a column, guessed from the first non-null mock value. */
export function colType(schema: Schema, table: string, col: string) {
  const v = tableData(schema, table).base.find((r) => r[col] !== null)?.[col];
  if (typeof v === 'number') return Number.isInteger(v) ? 'int' : 'numeric';
  if (typeof v === 'boolean') return 'bool';
  if (typeof v === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(v)) return 'date';
  return 'text';
}

// Rows persist per schema so saved edits show up when the query runs again.
// Inserted rows live in `added` so they stay visible past the LIMIT slice.
const store = new WeakMap<Schema, Record<string, { base: Row[]; added: Row[] }>>();
export function tableData(schema: Schema, table: string) {
  let s = store.get(schema);
  if (!s) store.set(schema, (s = {}));
  return (s[table] ??= { base: rowsFor(table, schema[table]), added: [] });
}

export function statementKind(sql: string): 'read' | 'write' | 'empty' {
  const first = sql.replace(/--.*$/gm, '').trim().split(/\s+/)[0]?.toLowerCase() ?? '';
  if (!first) return 'empty';
  return ['select', 'with', 'show', 'explain', 'describe'].includes(first) ? 'read' : 'write';
}

// Prototype only: fakes a query run against generated rows.
export function runMock(sql: string, schema: Schema, conn: { engine: Engine; database: string }): QueryResult {
  const ms = 8 + Math.floor(Math.random() * 40);
  if (statementKind(sql) === 'write') {
    return { columns: [], rows: [], ms, note: 'Statement ran (mock). 0 rows affected.' };
  }
  const m = sql.match(/from\s+(?:(\w+)\.)?(\w+)/i);
  const prefix = m?.[1]?.toLowerCase();
  const table = m?.[2]?.toLowerCase() ?? Object.keys(schema)[0];
  // MySQL also accepts the database name as the qualifier (`db.table`).
  const okPrefix = !prefix || prefix === nsOf(conn.engine, table) || (conn.engine === 'mysql' && prefix === conn.database.toLowerCase());
  if (!schema[table] || !okPrefix) {
    return { columns: [], rows: [], ms, error: `relation "${prefix ? prefix + '.' : ''}${table}" does not exist` };
  }
  const limit = Number(sql.match(/limit\s+(\d+)/i)?.[1] ?? 200);
  const d = tableData(schema, table);
  const rows = [...d.base.slice(0, limit), ...d.added].map((r) => ({ ...r }));
  return { columns: schema[table], rows, ms, table, sql };
}
