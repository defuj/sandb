import { active, type Conn } from './state.svelte';

export type AuditResult =
  | { status: 'ok'; rows: number }
  | { status: 'blocked' | 'denied' }
  | { status: 'error'; message: string };

export interface AuditEntry {
  id: number;
  time: Date;
  connId: string;
  kind: 'READ' | 'WRITE';
  source: 'AI' | 'You';
  sql: string;
  result: AuditResult;
  ms: number;
}

const min = (m: number) => new Date(Date.now() - m * 60_000);
let seq = 0;

const seed: Omit<AuditEntry, 'id'>[] = [
  { time: min(2890), connId: 'notes', kind: 'READ', source: 'You', sql: 'SELECT * FROM notes WHERE pinned = 1 ORDER BY created_at DESC;', result: { status: 'ok', rows: 7 }, ms: 3 },
  { time: min(2710), connId: 'notes', kind: 'WRITE', source: 'You', sql: "UPDATE notes SET pinned = 0 WHERE title = 'Trip plan';", result: { status: 'ok', rows: 1 }, ms: 4 },
  { time: min(1460), connId: 'shop', kind: 'READ', source: 'AI', sql: 'SELECT p.name, sum(ci.qty) AS units\nFROM cart_items ci\nJOIN products p ON p.id = ci.product_id\nGROUP BY p.name\nORDER BY units DESC\nLIMIT 10;', result: { status: 'ok', rows: 10 }, ms: 41 },
  { time: min(1442), connId: 'shop', kind: 'WRITE', source: 'AI', sql: 'DELETE FROM carts\nWHERE updated_at < now() - interval 30 day;', result: { status: 'blocked' }, ms: 0 },
  { time: min(1180), connId: 'shop', kind: 'READ', source: 'You', sql: 'SELECT * FROM customer LIMIT 100;', result: { status: 'error', message: 'Table \'shop.customer\' doesn\'t exist' }, ms: 6 },
  { time: min(395), connId: 'prod', kind: 'READ', source: 'You', sql: "SELECT *\nFROM public.orders\nWHERE status = 'paid'\nLIMIT 50;", result: { status: 'ok', rows: 50 }, ms: 22 },
  { time: min(388), connId: 'prod', kind: 'READ', source: 'AI', sql: 'SELECT date_trunc(\'day\', created_at) AS day, count(*)\nFROM public.orders\nWHERE created_at > now() - interval \'14 days\'\nGROUP BY 1\nORDER BY 1;', result: { status: 'ok', rows: 14 }, ms: 118 },
  { time: min(240), connId: 'prod', kind: 'WRITE', source: 'AI', sql: "UPDATE public.customers\nSET email = lower(email)\nWHERE email <> lower(email);", result: { status: 'denied' }, ms: 0 },
  { time: min(122), connId: 'shop', kind: 'WRITE', source: 'AI', sql: "UPDATE products\nSET stock = 0\nWHERE discontinued = 1;", result: { status: 'ok', rows: 38 }, ms: 27 },
  { time: min(64), connId: 'prod', kind: 'READ', source: 'AI', sql: "SELECT count(*) FROM public.orders\nWHERE status = 'cancelled' AND created_at < now() - interval '90 days';", result: { status: 'ok', rows: 1 }, ms: 64 },
  { time: min(61), connId: 'prod', kind: 'WRITE', source: 'AI', sql: "DELETE FROM public.orders\nWHERE status = 'cancelled'\n  AND created_at < now() - interval '90 days';", result: { status: 'ok', rows: 214 }, ms: 312 },
  { time: min(9), connId: 'notes', kind: 'READ', source: 'You', sql: 'SELECT * FROM tags LIMIT 100;', result: { status: 'ok', rows: 12 }, ms: 2 },
];

export const audit = $state({
  entries: seed.map((e) => ({ ...e, id: ++seq })) as AuditEntry[],
});

export function logQuery(e: Omit<AuditEntry, 'id' | 'time' | 'connId'> & { conn?: Conn }) {
  const { conn = active(), ...rest } = e;
  audit.entries.push({ ...rest, id: ++seq, time: new Date(), connId: conn.id });
}
