import type { ResultSort, TabStatus } from './api';
import type { QueryResult } from './mock';
import { active, type Conn } from './state.svelte';
import type { TableFilter, TableRef } from './tableActions';

/** An editor tab. `id` is also the backend `tabId` of its session. */
export interface Tab {
  id: string;
  title: string;
  connId: string;
  /** Database the editor runs against; PG switches it when a table of another database opens. */
  database: string;
  query: string;
  result: QueryResult | null;
  /** The connection that produced `result`. */
  resultConn: Conn;
  /** Tree key and SQL of the table opened from the tree or the palette (real backends). */
  opened: { key: string; sql: string } | null;
  status: TabStatus | null;
  /** Set on a table tab (double-click or ⌘-click in the tree): it shows the table's data or properties, not the editor. */
  table: TableRef | null;
  /** Structured filters for the table Data view. */
  filters: TableFilter[];
  /** Database-side sort shared by table and query results. */
  sort?: ResultSort;
  view: 'data' | 'properties';
  /** The saved query this tab edits; ⌘S writes to it. */
  savedId: string | null;
  /** `ai`: the tab was opened by the assistant; it may be reused for a later answer. */
  origin: 'user' | 'ai';
  /** The full SQL the editor is typing out for an AI answer; null when it is not typing. */
  aiTyping: string | null;
}

// state.svelte.ts imports this module too; neither touches the other at load time, so the cycle is safe.
export const tabs = $state({ list: [] as Tab[], activeId: '' });

let seq = 0;

export const defaultDb = (c: Conn) => (c.engine === 'sqlite' ? 'main' : c.database);

export function activeTab(): Tab | undefined {
  return tabs.list.find((t) => t.id === tabs.activeId) ?? tabs.list[0];
}

/** Points the tab at another connection, on its default database. */
export function setTabConn(t: Tab, c: Conn) {
  t.connId = c.id;
  t.database = defaultDb(c);
}

/** Opens a tab on the active tab's connection and database, and makes it active. */
export function newTab(query = ''): Tab {
  const c = active();
  const from = activeTab();
  tabs.list.push({
    id: crypto.randomUUID(),
    title: `Query ${++seq}`,
    connId: from?.connId ?? c.id,
    database: from?.database ?? defaultDb(c),
    query,
    result: null,
    resultConn: c,
    opened: null,
    status: null,
    table: null,
    filters: [],
    view: 'data',
    savedId: null,
    origin: 'user',
    aiTyping: null,
  });
  // The pushed proxy, so callers mutate the stored tab.
  const t = tabs.list[tabs.list.length - 1];
  tabs.activeId = t.id;
  return t;
}

/** Opens a table tab on `c`, or selects the tab that already shows this table. Returns the tab and if it is new. */
export function openTableTab(c: Conn, t: TableRef): { tab: Tab; created: boolean } {
  const same = tabs.list.find((x) => x.table && x.connId === c.id && x.table.database === t.database && x.table.schema === t.schema && x.table.name === t.name);
  if (same) {
    tabs.activeId = same.id;
    return { tab: same, created: false };
  }
  const nt = newTab();
  nt.connId = c.id;
  nt.database = c.engine === 'sqlite' ? 'main' : t.database;
  nt.resultConn = c;
  nt.title = t.name;
  nt.table = t;
  return { tab: nt, created: true };
}

export function selectTab(id: string) {
  if (tabs.list.some((t) => t.id === id)) tabs.activeId = id;
}

/** Closing the last tab leaves a fresh empty "Query 1" on the same connection, so the editor never disappears. */
export function closeTab(id: string) {
  const i = tabs.list.findIndex((t) => t.id === id);
  if (i < 0) return;
  if (tabs.list.length === 1) {
    seq = 0;
    tabs.activeId = id;
    newTab();
    tabs.list.splice(i, 1);
    return;
  }
  tabs.list.splice(i, 1);
  if (tabs.activeId === id) tabs.activeId = tabs.list[Math.min(i, tabs.list.length - 1)].id;
}

export interface AiTabInput {
  title: string;
  sql: string;
  conn: Conn;
  database: string;
  /** The model's choice: a new tab, the active one, or the best match. */
  target: 'new' | 'current' | 'best';
  result: QueryResult;
}

const normalizeSql = (sql: string) => sql.replace(/\s+/g, ' ').trim();

/** A tab the assistant may write into: no table view, no saved query, no open transaction, and empty or AI-owned. */
function aiWritable(t: Tab | undefined): t is Tab {
  return !!t && !t.table && !t.savedId && !t.status?.pending && !t.status?.aborted && (!t.query.trim() || t.origin === 'ai');
}

/**
 * Opens the result of an AI answer and makes its tab active at once. `target` is the model's
 * choice; this function enforces the safety rules: it never replaces a tab that holds the user's
 * work, a saved query, a table view or an open transaction. A new tab is the fallback.
 */
export function openAiTab(input: AiTabInput): Tab {
  const same = (t: Tab) => t.connId === input.conn.id && t.database === input.database;
  const exact = (t: Tab) => !!t.query.trim() && normalizeSql(t.query) === normalizeSql(input.sql);
  const siblings = tabs.list.filter(same);
  let t: Tab | undefined;

  if (input.target !== 'new') {
    // The same query is already in a safe tab: show the result there, does not type it again.
    t = [...siblings].reverse().find((x) => exact(x) && aiWritable(x));
    if (!t && input.target === 'current') {
      const a = activeTab();
      if (a && same(a) && aiWritable(a)) t = a;
    }
    if (!t) t = [...siblings].reverse().find(aiWritable);
  }

  if (!t) t = newTab();
  const unchanged = exact(t);
  t.connId = input.conn.id;
  t.database = input.database;
  t.resultConn = input.conn;
  t.title = input.title.trim() || 'AI query';
  t.result = input.result;
  t.opened = null;
  t.table = null;
  t.savedId = null;
  t.filters = [];
  t.sort = undefined;
  t.view = 'data';
  t.origin = 'ai';
  // Type the SQL only when it changes; a refresh of the same query keeps the text as it is.
  t.query = unchanged ? input.sql : '';
  t.aiTyping = unchanged ? null : input.sql;
  tabs.activeId = t.id;
  return t;
}
