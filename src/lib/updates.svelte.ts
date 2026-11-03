import { version } from '../../package.json';
import { api } from './api';
import { tabs } from './tabs.svelte';
import { settlePending } from './tx.svelte';

export const appVersion = version;

type Status = 'idle' | 'checking' | 'latest' | 'available' | 'installing' | 'restart' | 'error';
export const updates = $state({ status: 'idle' as Status, next: '', notes: '', error: '' });

// The updater plugin only exists inside the desktop app.
export const updatesEnabled = api.kind === 'tauri' && !import.meta.env.DEV;
let found: import('@tauri-apps/plugin-updater').Update | null = null;

/** `quiet`: the startup check; a failure (offline, no release yet) shows nothing. */
export async function checkForUpdates(quiet = false) {
  if (!updatesEnabled || ['checking', 'installing', 'restart'].includes(updates.status)) return;
  updates.status = 'checking';
  try {
    const { check } = await import('@tauri-apps/plugin-updater');
    found = await check();
    if (found) Object.assign(updates, { status: 'available', next: found.version, notes: found.body ?? '' });
    else updates.status = 'latest';
  } catch (e) {
    Object.assign(updates, quiet ? { status: 'idle' } : { status: 'error', error: String(e) });
  }
}

export async function installUpdate() {
  if (!found || !(await settlePending(tabs.list, 'Installing the update', true))) return;
  updates.status = 'installing';
  try {
    await found.downloadAndInstall();
  } catch (e) {
    Object.assign(updates, { status: 'error', error: String(e) });
    return;
  }
  updates.status = 'restart';
  await restartApp();
}

/** The update is installed; it takes effect on restart. */
export async function restartApp() {
  // A transaction can start while the update downloads, so ask again right before the restart.
  if (!(await settlePending(tabs.list, 'Restarting to update', true))) return;
  try {
    const { relaunch } = await import('@tauri-apps/plugin-process');
    await relaunch();
  } catch (e) {
    updates.error = String(e);
  }
}
