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
  /** Results of the last run, one tab per statement that returned columns, plus Messages for writes. */
  results: QueryResult[];
  /** Index of the result shown in the grid. */
  resultIndex: number;
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
    results: [],
    resultIndex: 0,
    resultConn: c,
    opened: null,
    status: null,
    table: null,
    filters: [],
    view: 'data',
    savedId: null,
  });
  // The pushed proxy, so callers mutate the stored tab.
  const t = tabs.list[tabs.list.length - 1];
  tabs.activeId = t.id;
  return t;
}

/** Replaces the result tabs after a run. `active` is the tab to show. */
export function setResults(t: Tab, results: QueryResult[], active = results.length ? results.length - 1 : 0) {
  t.results = results;
  t.resultIndex = results.length ? Math.min(Math.max(active, 0), results.length - 1) : 0;
  // Read the stored proxy, not the plain object: re-runs find their target by identity.
  t.result = t.results[t.resultIndex] ?? null;
}

/** A re-run of one result (Load more, sort, refresh) updates that tab, not the whole set. */
export function replaceResult(t: Tab, target: QueryResult, r: QueryResult) {
  const i = t.results.indexOf(target);
  if (i < 0) return false;
  // The sort belongs to the result, so the replacement keeps it.
  r.sort = target.sort;
  t.results[i] = r;
  if (t.resultIndex === i) t.result = t.results[i];
  return true;
}

/** Replaces the result currently shown; used when the caller knows it is still active. */
export function replaceActiveResult(t: Tab, r: QueryResult) {
  const target = t.results[t.resultIndex];
  if (target) {
    replaceResult(t, target, r);
    return;
  }
  t.results = [r];
  t.resultIndex = 0;
  t.result = t.results[0];
}

export function activateResult(t: Tab, i: number) {
  if (!t.results[i] || t.resultIndex === i) return;
  t.resultIndex = i;
  t.result = t.results[i];
  t.sort = t.result.sort;
}

/** Removes one result tab; the next one at that position becomes active. */
export function closeResult(t: Tab, i: number) {
  if (i < 0 || i >= t.results.length) return;
  const idx = t.resultIndex;
  t.results.splice(i, 1);
  if (!t.results.length) {
    t.result = null;
    t.resultIndex = 0;
    return;
  }
  // Closing right of the active tab must not move the cursor to that removed tab.
  t.resultIndex = i < idx ? idx - 1 : i === idx ? Math.min(i, t.results.length - 1) : idx;
  t.result = t.results[t.resultIndex];
  t.sort = t.result.sort;
}

export function clearResults(t: Tab) {
  t.results = [];
  t.resultIndex = 0;
  t.result = null;
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
