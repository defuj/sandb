import { schemas, type Row } from './mock';
import { active } from './state.svelte';
import { logQuery } from './audit.svelte';

export type ToolStatus = 'running' | 'done' | 'blocked' | 'awaiting' | 'approved' | 'denied';
export type Part =
  | { kind: 'text'; text: string }
  | { kind: 'tool'; name: string; sql?: string; status: ToolStatus; rows?: Row[]; note?: string };
export interface Msg {
  role: 'user' | 'assistant';
  parts: Part[];
}

export const chat = $state({
  messages: [] as Msg[],
  busy: false,
  pending: null as null | { sql: string; resolve: (ok: boolean) => void },
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function stream(msg: Msg, text: string) {
  const p: Part = { kind: 'text', text: '' };
  msg.parts.push(p);
  const part = msg.parts[msg.parts.length - 1] as { kind: 'text'; text: string };
  for (const w of text.split(/(\s+)/)) {
    part.text += w;
    await sleep(18);
  }
}

async function tool(msg: Msg, t: Extract<Part, { kind: 'tool' }>, ms = 600) {
  msg.parts.push(t);
  const live = msg.parts[msg.parts.length - 1] as Extract<Part, { kind: 'tool' }>;
  await sleep(ms);
  return live;
}

const WRITE = /\b(delete|remove|update|insert|drop|truncate|alter|mark)\b/i;

// Prototype only: scripted agent that mimics the real tool-calling loop.
export async function send(text: string) {
  if (!text.trim() || chat.busy) return;
  chat.busy = true;
  chat.messages.push({ role: 'user', parts: [{ kind: 'text', text }] });
  chat.messages.push({ role: 'assistant', parts: [] });
  const msg = chat.messages[chat.messages.length - 1];

  const conn = active();
  const [table, cols] = Object.entries(schemas[conn.schema])[0];
  const t1 = await tool(msg, { kind: 'tool', name: 'describe_table', sql: table, status: 'running' }, 500);
  t1.status = 'done';
  t1.note = `${cols.length} columns · 48,210 rows`;

  if (WRITE.test(text)) {
    const sql = "DELETE FROM public.orders\nWHERE status = 'cancelled'\n  AND created_at < now() - interval '90 days';";
    const c = await tool(msg, { kind: 'tool', name: 'run_read_query', sql: "SELECT count(*) FROM public.orders\nWHERE status = 'cancelled' AND created_at < now() - interval '90 days';", status: 'running' });
    c.status = 'done';
    c.rows = [{ count: 214 }];
    logQuery({ conn, kind: 'READ', source: 'AI', sql: c.sql!, result: { status: 'ok', rows: 1 }, ms: 58 });
    const w = await tool(msg, { kind: 'tool', name: 'propose_write', sql, status: conn.aiWrite ? 'awaiting' : 'blocked' }, 300);

    if (!conn.aiWrite) {
      w.note = `Blocked: the assistant is read-only on ${conn.name}.`;
      logQuery({ conn, kind: 'WRITE', source: 'AI', sql, result: { status: 'blocked' }, ms: 0 });
      await stream(msg, "This would delete **214** cancelled orders older than 90 days. I can't run it because the assistant is read-only. Turn on write mode with the shield button, or copy the SQL into the editor and run it yourself.");
    } else {
      const ok = await new Promise<boolean>((resolve) => (chat.pending = { sql, resolve }));
      chat.pending = null;
      w.status = ok ? 'approved' : 'denied';
      w.note = ok ? '214 rows affected · committed' : 'You denied this statement. Nothing changed.';
      logQuery({ conn, kind: 'WRITE', source: 'AI', sql, result: ok ? { status: 'ok', rows: 214 } : { status: 'denied' }, ms: ok ? 296 : 0 });
      await stream(msg, ok ? 'Done. I deleted 214 cancelled orders inside one transaction. The change is in the audit log.' : 'OK, I did not run it. The data is unchanged.');
    }
  } else {
    const q = await tool(msg, { kind: 'tool', name: 'run_read_query', sql: 'SELECT status, count(*) AS orders, sum(total) AS revenue\nFROM public.orders\nGROUP BY status\nORDER BY revenue DESC;', status: 'running' }, 800);
    q.status = 'done';
    logQuery({ conn, kind: 'READ', source: 'AI', sql: q.sql!, result: { status: 'ok', rows: 5 }, ms: 84 });
    q.rows = [
      { status: 'paid', orders: 21480, revenue: 1843210.5 },
      { status: 'shipped', orders: 15120, revenue: 1290442.1 },
      { status: 'pending', orders: 6230, revenue: 402118.0 },
      { status: 'cancelled', orders: 3910, revenue: 0 },
      { status: 'refunded', orders: 1470, revenue: -98220.4 },
    ];
    await stream(msg, '**Paid** orders bring the most revenue ($1.84M from 21,480 orders), then **shipped** ($1.29M). About 8% of orders are cancelled, and refunds remove about $98K.');
  }
  chat.busy = false;
}
