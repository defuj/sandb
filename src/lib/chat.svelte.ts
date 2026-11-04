import { schemas, tableData, nsOf, type Row, type Cell } from './mock';
import { active, ui, canWrite, flushAi, type Conn } from './state.svelte';
import { activeTab } from './tabs.svelte';
import { api, ApiError, type ChatEvent, type ChatRequest, type ErrorCode } from './api';
import { logQuery, type AuditResult } from './audit.svelte';

export type ToolStatus = 'running' | 'done' | 'blocked' | 'awaiting' | 'approved' | 'denied';
/** `id`: the backend's tool call id (live replies only). */
export type ToolPart = { kind: 'tool'; id?: string; name: string; sql?: string; status: ToolStatus; rows?: Row[]; note?: string };
/** `answer`: missing while the question is open, null when the user skipped it. */
export type QuestionPart = { kind: 'question'; text: string; options: string[]; answer?: string | null };
/** The reply stopped: an error, or the user pressed Stop. */
export type ErrorPart = { kind: 'error'; text: string };
export type Part = { kind: 'text'; text: string } | ToolPart | QuestionPart | ErrorPart;
export interface Msg {
  role: 'user' | 'assistant';
  parts: Part[];
}
/** One conversation. It belongs to one connection. */
export interface Session {
  id: string;
  connId: string;
  title: string;
  updated: number;
  messages: Msg[];
}

const KEY = 'sandb.chat.v1';
const MAX_SESSIONS = 50;

// A saved session can stop in the middle of a reply (the app closed). Its open steps can never finish.
function load(): Session[] {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) ?? '[]') as Session[];
    for (const s of list)
      for (const m of s.messages)
        for (const p of m.parts) {
          if (p.kind === 'tool' && (p.status === 'running' || p.status === 'awaiting')) {
            p.status = 'denied';
            p.note = 'Stopped: the app closed before this step finished. Nothing ran.';
          }
          if (p.kind === 'question' && p.answer === undefined) p.answer = null;
        }
    return list;
  } catch (e) {
    console.warn('Chat history is not readable; starting empty.', e);
    return [];
  }
}

export const chat = $state({
  sessions: load(),
  /** Open session per connection id. Missing: the newest one; '': a new, empty chat. */
  open: {} as Record<string, string>,
  /** Id of the session that the running reply writes to; null when the assistant is idle. */
  live: null as string | null,
  /** A proposed write that waits for Approve or Deny. */
  pending: null as null | { resolve: (ok: boolean) => void },
  /** A question to the user that waits for an answer. */
  question: null as null | { resolve: (answer: string | null) => void },
  /** Stops the running live reply; null when there is nothing to stop. */
  cancel: null as null | (() => void),
});

let saveTimer: ReturnType<typeof setTimeout> | undefined;
$effect.root(() => {
  $effect(() => {
    const json = JSON.stringify(chat.sessions);
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try {
        localStorage.setItem(KEY, json);
      } catch (e) {
        console.warn('Chat history was not saved.', e);
      }
    }, 300);
  });
});

export const sessionsOf = (connId: string) => chat.sessions.filter((s) => s.connId === connId).sort((a, b) => b.updated - a.updated);

/** The session the panel shows. While a reply runs, that session, so its questions stay visible. */
export function current(connId: string): Session | undefined {
  if (chat.live) return chat.sessions.find((s) => s.id === chat.live);
  const id = chat.open[connId];
  if (id === '') return undefined;
  return (id && chat.sessions.find((s) => s.id === id)) || sessionsOf(connId)[0];
}

export function openSession(connId: string, id: string) {
  if (!chat.live) chat.open[connId] = id;
}

export function deleteSession(id: string) {
  if (id !== chat.live) chat.sessions = chat.sessions.filter((s) => s.id !== id);
}

export const decide = (ok: boolean) => chat.pending?.resolve(ok);
export const answer = (a: string | null) => chat.question?.resolve(a);

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function stream(msg: Msg, text: string) {
  msg.parts.push({ kind: 'text', text: '' });
  const part = msg.parts[msg.parts.length - 1] as { kind: 'text'; text: string };
  for (const w of text.split(/(\s+)/)) {
    part.text += w;
    await sleep(18);
  }
}

async function tool(msg: Msg, t: ToolPart, ms = 600) {
  msg.parts.push(t);
  const live = msg.parts[msg.parts.length - 1] as ToolPart;
  await sleep(ms);
  return live;
}

async function ask(msg: Msg, text: string, options: string[]) {
  msg.parts.push({ kind: 'question', text, options });
  const part = msg.parts[msg.parts.length - 1] as QuestionPart;
  ui.chatOpen = true;
  const a = await new Promise<string | null>((resolve) => {
    chat.question = { resolve };
  });
  chat.question = null;
  part.answer = a;
  return a;
}

export async function send(text: string) {
  if (!text.trim() || chat.live) return;
  const conn = active();
  let s = current(conn.id);
  if (!s) {
    chat.sessions.unshift({ id: crypto.randomUUID(), connId: conn.id, title: text.trim(), updated: Date.now(), messages: [] });
    s = chat.sessions[0];
    if (chat.sessions.length > MAX_SESSIONS) {
      const oldest = [...chat.sessions].sort((a, b) => a.updated - b.updated)[0];
      chat.sessions = chat.sessions.filter((x) => x !== oldest);
    }
  }
  chat.open[conn.id] = s.id;
  chat.live = s.id;
  s.updated = Date.now();
  const history = historyOf(s.messages);
  s.messages.push({ role: 'user', parts: [{ kind: 'text', text }] });
  s.messages.push({ role: 'assistant', parts: [] });
  const msg = s.messages[s.messages.length - 1];
  try {
    if (api.kind === 'mock') await reply(msg, text, conn);
    else await liveReply(msg, text, conn, history);
  } finally {
    chat.live = chat.pending = chat.question = chat.cancel = null;
  }
}

// Earlier turns as plain text: the user's message, or the assistant's text parts. Tool steps stay out.
function historyOf(messages: Msg[]): ChatRequest['history'] {
  return messages.flatMap((m) => {
    const content = m.parts
      .flatMap((p) => (p.kind === 'text' ? [p.text] : []))
      .join('\n\n')
      .trim();
    return content ? [{ role: m.role, content }] : [];
  });
}

const BLOCKED: Partial<Record<ErrorCode, true>> = { guard_rejected: true, write_blocked: true, read_only_violation: true };
const NOT_BUILT = 'Approving AI writes is not built yet. Copy the SQL to the editor to run it yourself.';

/** First number of a summary such as "5 rows · 42 ms". */
const countIn = (summary: string) => Number(/^([\d,]+)/.exec(summary)?.[1].replace(/,/g, '') ?? NaN);

// Real backends: the Rust tool loop streams its steps; this maps each event onto the reply's parts.
async function liveReply(msg: Msg, text: string, conn: Conn, history: ChatRequest['history']) {
  const chatId = crypto.randomUUID();
  let stopped = false;
  let ended = false;
  chat.cancel = () => {
    stopped = true;
    api.chatCancel(chatId).catch((e) => console.warn('The reply was not stopped.', ApiError.from(e).message));
  };
  const fail = (code: string, message: string) => {
    msg.parts.push({ kind: 'error', text: code === 'cancelled' ? 'Stopped.' : message });
  };

  try {
    await flushAi();
  } catch (e) {
    const err = ApiError.from(e);
    fail(err.code, `The AI settings are not saved: ${err.message}`);
    return;
  }
  if (stopped) return fail('cancelled', '');

  const tab = activeTab();
  const request: ChatRequest = {
    chatId,
    connId: conn.id,
    database: tab?.connId === conn.id && tab.database ? tab.database : null,
    schema: null,
    history,
    message: text,
    allowWrites: conn.aiWrite && canWrite(conn),
  };
  const steps = new Map<string, { part: ToolPart; started: number }>();

  const onEvent = (e: ChatEvent) => {
    switch (e.type) {
      case 'text': {
        const last = msg.parts.at(-1);
        if (last?.kind === 'text') last.text += e.delta;
        else msg.parts.push({ kind: 'text', text: e.delta });
        break;
      }
      case 'tool_call': {
        const sql = typeof e.args.sql === 'string' ? e.args.sql : typeof e.args.table === 'string' ? e.args.table : undefined;
        msg.parts.push({ kind: 'tool', id: e.id, name: e.name, sql, status: 'running' });
        steps.set(e.id, { part: msg.parts[msg.parts.length - 1] as ToolPart, started: Date.now() });
        break;
      }
      case 'write_preview': {
        const step = steps.get(e.id);
        if (!step) break;
        step.part.sql = e.sql;
        step.part.status = 'awaiting';
        step.part.note = NOT_BUILT;
        ui.chatOpen = true;
        break;
      }
      case 'tool_result': {
        const step = steps.get(e.id);
        if (!step) break;
        const { part, started } = step;
        const blocked = !e.ok && !!e.error && !!BLOCKED[e.error.code];
        // An allowed write stays open: its preview already set the status and the note.
        if (part.status !== 'awaiting') {
          part.status = e.ok ? 'done' : blocked ? 'blocked' : 'denied';
          part.note = e.ok ? e.summary : (e.error?.message ?? e.summary);
        }
        const columns = e.columns;
        if (e.ok && columns && e.rows?.length) part.rows = e.rows.slice(0, 10).map((r) => Object.fromEntries(columns.map((c, i) => [c, r[i] as Cell])));
        if (part.name === 'run_read_query' && part.sql) {
          const n = countIn(e.summary);
          const result: AuditResult = e.ok ? { status: 'ok', rows: Number.isFinite(n) ? n : (e.rows?.length ?? 0) } : blocked ? { status: 'blocked' } : { status: 'error', message: e.error?.message ?? e.summary };
          logQuery({ conn, kind: 'READ', source: 'AI', sql: part.sql, result, ms: e.ok ? Date.now() - started : 0 });
        }
        if (part.name === 'propose_write' && part.sql) {
          const result: AuditResult = part.status === 'awaiting' ? { status: 'error', message: 'Not run: approving AI writes is not built yet' } : blocked ? { status: 'blocked' } : { status: 'error', message: e.error?.message ?? e.summary };
          logQuery({ conn, kind: 'WRITE', source: 'AI', sql: part.sql, result, ms: 0 });
        }
        break;
      }
      case 'error':
        ended = true;
        fail(e.error.code, e.error.message);
        break;
      case 'done':
        ended = true;
        break;
    }
  };

  try {
    await api.chatSend(request, onEvent);
  } catch (e) {
    if (!ended) {
      const err = ApiError.from(e);
      fail(err.code, err.message);
    }
  } finally {
    // Steps without a result can never finish now.
    for (const { part } of steps.values())
      if (part.status === 'running') {
        part.status = 'denied';
        part.note = 'Stopped before this step finished.';
      }
  }
}

const WRITE = /\b(delete|remove|update|insert|drop|truncate|alter|mark)\b/i;
const find = (tables: string[], name: string) => tables.find((t) => t.toLowerCase() === name.toLowerCase());

// Prototype only: scripted agent that mimics the real tool-calling loop.
async function reply(msg: Msg, text: string, conn: Conn) {
  const schema = schemas[conn.schema];
  const tables = Object.keys(schema);

  if (WRITE.test(text)) {
    const t1 = await tool(msg, { kind: 'tool', name: 'describe_table', sql: tables[0], status: 'running' }, 500);
    t1.status = 'done';
    t1.note = `${schema[tables[0]].length} columns · 48,210 rows`;
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
      ui.chatOpen = true;
      const ok = await new Promise<boolean>((resolve) => {
        chat.pending = { resolve };
      });
      chat.pending = null;
      w.status = ok ? 'approved' : 'denied';
      w.note = ok ? '214 rows affected · committed' : 'You denied this statement. Nothing changed.';
      logQuery({ conn, kind: 'WRITE', source: 'AI', sql, result: ok ? { status: 'ok', rows: 214 } : { status: 'denied' }, ms: ok ? 296 : 0 });
      await stream(msg, ok ? 'Done. I deleted 214 cancelled orders inside one transaction. The change is in the audit log.' : 'OK, I did not run it. The data is unchanged.');
    }
    return;
  }

  // A table named in the message ("table notes", "from notes", "from table notes"), else any table name in the text.
  const named = /\b(?:from|table)\s+(?:table\s+)?[`"']?(\w+)/i.exec(text)?.[1];
  let target = named ? find(tables, named) : tables.find((t) => new RegExp(`\\b${t}\\b`, 'i').test(text));

  if (!target && !named && schema.orders && /\b(orders?|revenue)\b/i.test(text)) {
    const t1 = await tool(msg, { kind: 'tool', name: 'describe_table', sql: 'orders', status: 'running' }, 500);
    t1.status = 'done';
    t1.note = `${schema.orders.length} columns · 48,210 rows`;
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
    return;
  }

  if (!target) {
    await sleep(400);
    const question = named ? `I can't find a table named **${named}** on ${conn.name}. Which table do you mean?` : `Which table on ${conn.name} should I look at?`;
    const a = await ask(msg, question, tables);
    if (a === null) {
      await stream(msg, 'OK. Tell me which table to use when you are ready.');
      return;
    }
    target = find(tables, a.trim()) ?? tables.find((t) => new RegExp(`\\b${t}\\b`, 'i').test(a));
    if (!target) {
      await stream(msg, `I can't find **${a.trim()}** either. The tables on ${conn.name} are: ${tables.join(', ')}.`);
      return;
    }
  }

  const data = tableData(schema, target);
  const total = data.base.length + data.added.length;
  const t1 = await tool(msg, { kind: 'tool', name: 'describe_table', sql: target, status: 'running' }, 500);
  t1.status = 'done';
  t1.note = `${schema[target].length} columns · ${total.toLocaleString()} rows`;
  const q = await tool(msg, { kind: 'tool', name: 'run_read_query', sql: `SELECT *\nFROM ${nsOf(conn.engine, target)}.${target}\nLIMIT 5;`, status: 'running' }, 700);
  q.status = 'done';
  q.rows = data.base.slice(0, 5);
  logQuery({ conn, kind: 'READ', source: 'AI', sql: q.sql!, result: { status: 'ok', rows: q.rows.length }, ms: 41 });
  await stream(msg, `Here are the first ${q.rows.length} rows of **${target}** (${total.toLocaleString()} rows in total). Ask me to filter or summarize them.`);
}
