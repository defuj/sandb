import { invoke } from '@tauri-apps/api/core';
import { ApiError, type Backend } from './types';

async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw ApiError.from(e);
  }
}

export const tauriBackend: Backend = {
  kind: 'tauri',
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
  queryProgress: (queryId) => call('query_progress', { queryId }),
  commitTransaction: (tabId) => call('commit_transaction', { tabId }),
  rollbackTransaction: (tabId) => call('rollback_transaction', { tabId }),
  tabStatus: (tabId) => call('tab_status', { tabId }),
  closeTabSession: (tabId) => call('close_tab_session', { tabId }),
  quitApp: () => call('quit_app'),
  exportCsv: (connId, database, sql, tabId) => call('export_csv', { request: { connId, database, sql, tabId } }),
  tableDdl: (connId, database, schema, table) => call('table_ddl', { connId, database, schema, table }),
  exportTableSql: (connId, database, schema, table, data) => call('export_table_sql', { request: { connId, database, schema, table, data } }),
  guardCheck: (engine, sql) => call('guard_check', { engine, sql }),
  classifyWrite: (engine, sql) => call('classify_write', { engine, sql }),
  runAiRead: (connId, database, sql) => call('run_ai_read', { connId, database, sql }),
};
