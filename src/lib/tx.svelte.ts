import { api } from './api';
import { db, canWrite, type Conn } from './state.svelte';
import type { Tab } from './tabs.svelte';

/** Writable Production connections run in the tab's own session: changes wait for Commit or Rollback. */
export const manualCommit = (c: Conn | undefined): c is Conn => !!c && c.env === 'production' && canWrite(c);

/** A failed read keeps the last known status; it must never break the run that asked for it. */
export async function refreshStatus(t: Tab) {
  if (!manualCommit(db.connections.find((c) => c.id === t.connId))) {
    t.status = null;
    return;
  }
  try {
    t.status = await api.tabStatus(t.id);
  } catch {}
}

/** Throws the backend error; a failed end closes the session on the backend, which the refresh picks up. */
export async function endTx(t: Tab, commit: boolean) {
  try {
    await (commit ? api.commitTransaction(t.id) : api.rollbackTransaction(t.id));
  } finally {
    await refreshStatus(t);
  }
}

/**
 * Drops the session of a tab that moves to another connection or database, so it does not hold a connection.
 * Always asks the backend (a no-op without a session): `status` can be null after a failed refresh.
 */
export function leaveSession(t: Tab) {
  api.closeTabSession(t.id).catch(() => {});
  t.status = null;
}

export interface TxAsk {
  tabs: Tab[];
  /** What is about to happen, e.g. "Closing this tab". */
  action: string;
  /** `true` once every tab is committed or rolled back; `false` on Cancel. */
  resolve: (settled: boolean) => void;
}

/** The open PendingTxDialog question; one at a time. */
export const txPrompt = $state({ ask: null as TxAsk | null });
// The answer to `txPrompt.ask`, for a `queue` caller that must wait its turn.
let open: Promise<boolean> | null = null;

/**
 * Resolves `true` when none of `list` holds uncommitted changes, after asking the user (PendingTxDialog)
 * to commit or roll back those that do. `false`: the user cancelled, and the caller must not proceed.
 * One decision covers every tab in `list`. While another question is open the answer is `false`, unless
 * `queue`: then it waits for that question, gives up if it was cancelled, and asks its own afterwards
 * (a quit must not be dropped just because a tab close was asking at the time).
 */
export async function settlePending(list: Tab[], action: string, queue = false): Promise<boolean> {
  for (;;) {
    // The status from the last run can be stale (session dropped, a failed read): ask the backend first.
    await Promise.all(list.map(refreshStatus));
    if (!list.some((t) => t.status?.pending)) return true;
    if (!open) break;
    if (!queue || !(await open)) return false;
  }
  const p = new Promise<boolean>((resolve) => {
    const tabs = list.filter((t) => t.status?.pending);
    txPrompt.ask = { tabs, action, resolve: (ok) => ((open = null), resolve(ok)) };
  });
  open = p;
  return p;
}
