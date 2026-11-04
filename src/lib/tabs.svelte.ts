import type { TabStatus } from './api';
import type { QueryResult } from './mock';
import { active, type Conn } from './state.svelte';
import type { TableRef } from './tableActions';

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
  view: 'data' | 'properties';
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
    view: 'data',
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
