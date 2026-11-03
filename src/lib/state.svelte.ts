import { api, ApiError, type Connection, type Engine, type Env } from './api';
import { seedConnections } from './api/mock';
import { tabs, activeTab, setTabConn } from './tabs.svelte';

export type { Engine, Env };

/** A saved connection (see api/types.ts) plus UI-only fields. `aiWrite`: AI may propose writes (each still needs approval). */
export interface Conn extends Connection {
  /** Key into the mock schemas in mock.ts. Schema contents stay mock until introspection exists. */
  schema: string;
  /** Set on an unsaved duplicate: the id of the connection it copies (and whose password it reuses). */
  copyOf?: string;
}

const mockSchemaOf: Record<Engine, string> = { pg: 'prod', mysql: 'shop', sqlite: 'notes' };
export const toConn = (c: Connection): Conn => ({ ...c, schema: mockSchemaOf[c.engine] });

export const engines: Record<Engine, { label: string; short: string; badge: string; port: number }> = {
  pg: { label: 'PostgreSQL', short: 'PG', badge: 'text-sky-700 dark:text-sky-400', port: 5432 },
  mysql: { label: 'MySQL / MariaDB', short: 'MY', badge: 'text-orange-800 dark:text-orange-400', port: 3306 },
  sqlite: { label: 'SQLite', short: 'SL', badge: 'text-violet-700 dark:text-violet-400', port: 0 },
};

export const envDot: Record<Env, string> = {
  // The ring tells production from staging without relying on hue.
  production: 'bg-red-600 ring-2 ring-red-600/30 dark:bg-red-500 dark:ring-red-500/35',
  staging: 'bg-amber-700 dark:bg-amber-500',
  local: 'bg-stone-500 dark:bg-stone-400',
};

export const envTag: Record<Env, string> = {
  production: 'text-red-700 dark:text-red-300',
  staging: 'text-amber-700 dark:text-amber-400',
  local: 'text-muted',
};

export const ui = $state({
  dark: false,
  chatOpen: true,
  page: 'query' as 'query' | 'settings' | 'audit',
  theme: 'light' as 'light' | 'dark' | 'system',
  /** Sidebar width in px; the title bar uses it too, so the tabs stay in line with the main card. */
  sidebarWidth: 224,
  /** Editor height as a fraction of the main card; the results grid takes the rest. */
  editorSplit: 0.34,
  /** False hides the results grid; the editor then fills the card. */
  resultsOpen: true,
});

export type Provider = 'ollama' | 'lmstudio' | 'openrouter' | 'openai' | 'custom';

export const providers: Record<Provider, { label: string; url: string; model: string; needsKey: boolean }> = {
  ollama: { label: 'Ollama', url: 'http://localhost:11434/v1', model: 'llama3.1:8b', needsKey: false },
  lmstudio: { label: 'LM Studio', url: 'http://localhost:1234/v1', model: 'qwen2.5-coder-7b', needsKey: false },
  openrouter: { label: 'OpenRouter', url: 'https://openrouter.ai/api/v1', model: 'anthropic/claude-sonnet-4.5', needsKey: true },
  openai: { label: 'OpenAI', url: 'https://api.openai.com/v1', model: 'gpt-4.1-mini', needsKey: true },
  custom: { label: 'Custom', url: '', model: '', needsKey: true },
};

export const ai = $state({
  provider: 'ollama' as Provider,
  baseUrl: providers.ollama.url,
  apiKey: '',
  model: providers.ollama.model,
  /** 'results' = schema + query results go to the model; 'schema' = schema only. */
  privacy: 'results' as 'results' | 'schema',
  maxRows: 500,
  timeoutSec: 15,
});

// Mock mode starts with the seed synchronously, exactly like the prototype; real backends load on start.
const isMock = api.kind === 'mock';

export const db = $state({
  connections: (isMock ? seedConnections.map(toConn) : []) as Conn[],
  loading: !isMock,
  loadError: '',
});

export async function loadConnections() {
  if (isMock) return;
  db.loading = true;
  db.loadError = '';
  try {
    db.connections = (await api.listConnections()).map(toConn);
    // Tabs opened before the load (or on a connection that is gone) move to the first connection.
    const first = db.connections[0];
    if (first) for (const t of tabs.list) if (!db.connections.some((c) => c.id === t.connId)) setTabConn(t, first);
  } catch (e) {
    db.loadError = ApiError.from(e).message;
  } finally {
    db.loading = false;
  }
}

// Stands in while there are no connections, so views that read active() keep working.
const placeholder: Conn = { id: '', name: '', engine: 'pg', version: '', env: 'local', host: '', port: 0, database: '', user: '', ssl: 'prefer', file: '', aiWrite: false, readOnly: false, schema: 'prod' };

export function active(): Conn {
  const id = activeTab()?.connId;
  return db.connections.find((c) => c.id === id) ?? db.connections[0] ?? placeholder;
}

/** False on a read-only connection: no write UI (editor, AI, edit mode) may act on it. */
export function canWrite(conn: Conn | Connection): boolean {
  return !conn.readOnly;
}

export function toggleTheme() {
  setTheme(ui.dark ? 'light' : 'dark');
}

export function setTheme(t: 'light' | 'dark' | 'system') {
  ui.theme = t;
  ui.dark = t === 'system' ? matchMedia('(prefers-color-scheme: dark)').matches : t === 'dark';
  document.documentElement.classList.toggle('dark', ui.dark);
}
