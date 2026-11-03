import { ApiError, type Backend } from './types';

// Dev-only transport to the Rust bridge (`pnpm bridge`). The bridge only allows the origin http://localhost:5199.
const base = ((import.meta.env.VITE_BRIDGE_URL as string | undefined) || 'http://127.0.0.1:58123').replace(/\/+$/, '');
const token = (import.meta.env.VITE_BRIDGE_TOKEN as string | undefined) ?? '';

async function call<T>(cmd: string, args: Record<string, unknown> = {}, keepalive = false): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${base}/api/${cmd}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify(args),
      keepalive,
    });
  } catch (e) {
    throw new ApiError({ code: 'io', message: 'Cannot reach the dev bridge', detail: `${base}: ${e instanceof Error ? e.message : e}` });
  }
  const body: unknown = await res.json().catch(() => null);
  if (!res.ok) throw ApiError.from(body ?? { code: 'internal', message: `Bridge returned HTTP ${res.status}` });
  return body as T;
}

export const httpBackend: Backend = {
  kind: 'bridge',
  listConnections: () => call('list_connections'),
  createConnection: (input, password) => call('create_connection', { input, password }),
  updateConnection: (id, input, password) => call('update_connection', { id, input, password }),
  duplicateConnection: (id, input, password) => call('duplicate_connection', { id, input, password }),
  deleteConnection: (id) => call('delete_connection', { id }),
  testConnection: (input, password, id) => call('test_connection', { input, password, id }),
  listDatabases: (connId) => call('list_databases', { connId }),
  listSchemas: (connId, database) => call('list_schemas', { connId, database }),
  listTables: (connId, database, schema) => call('list_tables', { connId, database, schema }),
  listColumns: (connId, database, schema, table) => call('list_columns', { connId, database, schema, table }),
  schemaSnapshot: (connId, database) => call('schema_snapshot', { connId, database }),
  runQuery: (request) => call('run_query', { request }),
  cancelQuery: (queryId) => call('cancel_query', { queryId }),
  commitTransaction: (tabId) => call('commit_transaction', { tabId }),
  rollbackTransaction: (tabId) => call('rollback_transaction', { tabId }),
  tabStatus: (tabId) => call('tab_status', { tabId }),
  // keepalive: the page-hide cleanup sends this while the page unloads, which cancels plain requests.
  closeTabSession: (tabId) => call('close_tab_session', { tabId }, true),
  quitApp: () => call('quit_app'),
  // The bridge never writes files on request; the UI exports the loaded rows in the browser instead.
  guardCheck: (engine, sql) => call('guard_check', { engine, sql }),
  classifyWrite: (engine, sql) => call('classify_write', { engine, sql }),
  runAiRead: (connId, database, sql) => call('run_ai_read', { connId, database, sql }),
  exportCsv: () => Promise.reject(new ApiError({ code: 'invalid_input', message: 'CSV export to a file needs the desktop app' })),
};
