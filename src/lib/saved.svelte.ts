import { tabs, type Tab } from './tabs.svelte';

/** A saved query (a script, as in DBeaver). It remembers the connection and database it was saved on. */
export interface SavedQuery {
  id: string;
  name: string;
  sql: string;
  connId: string;
  database: string;
  created: number;
  updated: number;
}

/** A pending Save dialog. `copy`: Save as… on a tab that is already linked; the tab moves to the new query. */
export type SaveAsk = { tab: Tab; name: string; copy: boolean };

const KEY = 'sandb.saved-queries.v1';

function load(): SavedQuery[] {
  try {
    const list = JSON.parse(localStorage.getItem(KEY) ?? '[]');
    return Array.isArray(list) ? list : [];
  } catch (e) {
    console.warn('Saved queries are not readable; starting empty.', e);
    return [];
  }
}

export const saved = $state({ list: load() });

let saveTimer: ReturnType<typeof setTimeout> | undefined;
$effect.root(() => {
  $effect(() => {
    const json = JSON.stringify(saved.list);
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try {
        localStorage.setItem(KEY, json);
      } catch (e) {
        console.warn('Saved queries were not stored.', e);
      }
    }, 200);
  });
});

export const findSaved = (id: string | null | undefined) => (id ? saved.list.find((q) => q.id === id) : undefined);

/** The tab is linked to a saved query and its text differs from the saved text. */
export function isDirty(t: Tab): boolean {
  const q = findSaved(t.savedId);
  return !!q && q.sql !== t.query;
}

/** A name from the first table the SQL reads or writes, e.g. "orders query". */
export function suggestName(sql: string): string {
  const clean = sql.replace(/--.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '');
  const m = /\b(?:from|into|update|join|table)\s+([`"[\]\w.]+)/i.exec(clean);
  const name = m?.[1].replace(/[`"[\]]/g, '').split('.').pop();
  return name ? `${name} query` : 'Untitled query';
}

/** Saves the tab's text as a new query and links the tab to it. */
export function createSaved(t: Tab, name: string): SavedQuery {
  const now = Date.now();
  saved.list.push({ id: crypto.randomUUID(), name, sql: t.query, connId: t.connId, database: t.database, created: now, updated: now });
  const q = saved.list[saved.list.length - 1];
  t.savedId = q.id;
  t.title = name;
  return q;
}

/** Writes the tab's text, connection and database to its linked query. */
export function updateSaved(t: Tab) {
  const q = findSaved(t.savedId);
  if (!q) return;
  q.sql = t.query;
  q.connId = t.connId;
  q.database = t.database;
  q.updated = Date.now();
}

/** Linked tabs take the new name too. */
export function renameSaved(id: string, name: string) {
  const q = findSaved(id);
  if (!q || !name.trim()) return;
  q.name = name.trim();
  for (const t of tabs.list) if (t.savedId === id) t.title = q.name;
}

export function duplicateSaved(id: string): SavedQuery | undefined {
  const q = findSaved(id);
  if (!q) return;
  const now = Date.now();
  saved.list.push({ ...q, id: crypto.randomUUID(), name: `${q.name} copy`, created: now, updated: now });
  return saved.list[saved.list.length - 1];
}

/** Removes the query; open tabs keep their text but lose the link. Returns a function that undoes the delete. */
export function deleteSaved(id: string): () => void {
  const i = saved.list.findIndex((q) => q.id === id);
  if (i < 0) return () => {};
  const [q] = saved.list.splice(i, 1);
  const linked = tabs.list.filter((t) => t.savedId === id).map((t) => t.id);
  for (const t of tabs.list) if (t.savedId === id) t.savedId = null;
  return () => {
    if (findSaved(q.id)) return;
    saved.list.splice(Math.min(i, saved.list.length), 0, q);
    for (const t of tabs.list) if (linked.includes(t.id) && !t.savedId) t.savedId = q.id;
  };
}
