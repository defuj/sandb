<script lang="ts">
  import { Database, Search, Settings, ScrollText, Sun, Moon, Plus, Sparkles, PanelRight, CircleArrowUp } from '@lucide/svelte';
  import { FileCode, Play, Square, X, LoaderCircle, Check, Undo2 } from '@lucide/svelte';
  import { tick, untrack } from 'svelte';
  import { ui, db, toggleTheme, active, type Conn } from './lib/state.svelte';
  import { api, ApiError } from './lib/api';
  import { schemas, runMock, statementKind, nsOf, type QueryResult, type Schema } from './lib/mock';
  import { toResult, snapshotSchema, quoteIdent } from './lib/results';
  import ConnectionTree from './lib/components/ConnectionTree.svelte';
  import ConnectionDialog from './lib/components/ConnectionDialog.svelte';
  import SqlEditor from './lib/components/SqlEditor.svelte';
  import ResultsGrid from './lib/components/ResultsGrid.svelte';
  import ChatPanel from './lib/components/ChatPanel.svelte';
  import WritePreview from './lib/components/WritePreview.svelte';
  import SafeguardDialog from './lib/components/SafeguardDialog.svelte';
  import SettingsPage from './lib/components/SettingsPage.svelte';
  import AuditPage from './lib/components/AuditPage.svelte';
  import CommandPalette from './lib/components/CommandPalette.svelte';
  import DeleteConnectionDialog from './lib/components/DeleteConnectionDialog.svelte';
  import TargetBar from './lib/components/TargetBar.svelte';
  import { logQuery } from './lib/audit.svelte';
  import { chat } from './lib/chat.svelte';
  import { confirmDiscard, resetEdits } from './lib/edits.svelte';
  import { tabs, activeTab, newTab, selectTab, closeTab, setTabConn, defaultDb, type Tab } from './lib/tabs.svelte';
  import { manualCommit, refreshStatus, endTx, leaveSession, settlePending, txPrompt } from './lib/tx.svelte';
  import PendingTxDialog from './lib/components/PendingTxDialog.svelte';
  import { updates, checkForUpdates } from './lib/updates.svelte';

  const isMock = api.kind === 'mock';
  // The overlay title bar puts the macOS window buttons over the top-left corner.
  const inset = api.kind === 'tauri' && /Mac/.test(navigator.userAgent);
  // Mock mode opens on a sample query, like the prototype.
  if (!tabs.list.length) newTab(isMock ? 'SELECT *\nFROM public.orders\nWHERE status = \'paid\'\nLIMIT 50;' : '');
  // Never undefined: closing the last tab opens a fresh one.
  const tab = $derived(activeTab()!);
  checkForUpdates(true);

  let guardTarget = $state<Conn | null>(null);
  let connDialog = $state(false);
  let connEdit = $state<Conn | null>(null);
  let connDelete = $state<Conn | null>(null);
  let paletteOpen = $state(false);
  let editor = $state<ReturnType<typeof SqlEditor>>();

  // Capture phase so ⌘K wins even while CodeMirror has focus.
  $effect(() => {
    const onkey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        e.stopPropagation();
        if (connDialog || connDelete || guardTarget || chat.pending || txPrompt.ask) return;
        paletteOpen = !paletteOpen;
      }
    };
    window.addEventListener('keydown', onkey, true);
    return () => window.removeEventListener('keydown', onkey, true);
  });

  async function focusEditor() {
    ui.page = 'query';
    await tick();
    editor?.focus();
  }

  async function switchConn(c: Conn) {
    if (c.id === conn.id || !confirmDiscard()) return;
    const t = tab;
    if (!(await settlePending([t], 'Switching the connection'))) return;
    leaveSession(t);
    setTabConn(t, c);
    openFirstTable();
  }

  // The session belongs to the old database, like a connection switch.
  async function switchDb(name: string) {
    if (conn.engine === 'sqlite' || name === database || !confirmDiscard()) return;
    const t = tab;
    if (!(await settlePending([t], 'Switching the database'))) return;
    leaveSession(t);
    t.database = name;
    openFirstTable();
  }

  // One discard prompt covers both the connection switch and the table open.
  async function openTableIn(c: Conn, t: string, at?: { database: string; schema: string | null }) {
    if (!confirmDiscard()) return;
    const cur = tab;
    const moves = c.id !== cur.connId || (!!at && c.engine !== 'sqlite' && at.database !== database);
    if (moves) {
      if (!(await settlePending([cur], c.id !== cur.connId ? 'Switching the connection' : 'Switching the database'))) return;
      leaveSession(cur);
    }
    if (c.id !== cur.connId) setTabConn(cur, c);
    openTable(t, at);
  }

  // The tabs stay in the title bar on every page; picking one goes back to the query view.
  function pickTab(id: string) {
    if (id !== tab.id) {
      if (!confirmDiscard()) return;
      selectTab(id);
    }
    ui.page = 'query';
  }

  let tabStrip = $state<HTMLDivElement>();
  async function addTab() {
    if (!confirmDiscard()) return;
    newTab();
    ui.page = 'query';
    await tick();
    tabStrip?.lastElementChild?.scrollIntoView({ inline: 'nearest' });
  }

  // Pending edits belong to the active tab's result; other tabs can close without asking.
  async function dropTab(t: Tab) {
    if (t.id === tab.id && !confirmDiscard()) return;
    if (!(await settlePending([t], 'Closing this tab')) || !tabs.list.some((x) => x.id === t.id)) return;
    const r = running[t.id];
    if (r) api.cancelQuery(r.id).catch(() => {});
    closeTab(t.id);
    // Every tab may hold a session; the backend ignores tabs without one.
    api.closeTabSession(t.id).catch(() => {});
  }

  let renaming = $state<string | null>(null);
  function rename(t: Tab, title: string) {
    if (renaming !== t.id) return;
    renaming = null;
    if (title.trim()) t.title = title.trim();
  }
  const focusSelect = (el: HTMLInputElement) => el.select();

  const conn = $derived(active());
  // `||`: a tab opened before the connections loaded has no database yet.
  const database = $derived(conn.engine === 'sqlite' ? 'main' : tab.database || conn.database);
  let snapshot = $state<{ key: string; schema: Schema } | null>(null);
  const schema = $derived(isMock ? schemas[conn.schema] : snapshot?.key === `${conn.id}/${database}` ? snapshot.schema : {});
  const tables = $derived(Object.keys(schema));
  // Tree highlight follows the shown result; joins, unions, CTEs and aggregates have no single table.
  const selected = $derived.by(() => {
    const { result: r, opened, resultConn } = tab;
    if (!isMock) return opened && r && !r.error && r.sql === opened.sql ? opened.key : null;
    if (!r?.table || !r.sql || r.error) return null;
    if (/\b(join|union|group\s+by|distinct)\b|^\s*with\b|\bfrom\s+[\w.]+(\s+\w+)?\s*,|\b(count|sum|avg|min|max)\s*\(/i.test(r.sql.replace(/--.*$/gm, ''))) return null;
    return `${resultConn.id}/${nsOf(resultConn.engine, r.table)}/${r.table}`;
  });
  const kind = $derived(statementKind(tab.query));

  // Autocomplete and palette tables of real connections; failures only leave autocomplete empty.
  $effect(() => {
    if (isMock || !conn.id) return;
    const key = `${conn.id}/${database}`;
    const engine = conn.engine;
    if (untrack(() => snapshot?.key) === key) return;
    api.schemaSnapshot(conn.id, database).then(
      (s) => { if (`${active().id}/${database}` === key) snapshot = { key, schema: snapshotSchema(s, engine) }; },
      () => {},
    );
  });

  // Pending edits belong to the previous tab's (or connection's) result.
  $effect(() => {
    tab.id;
    conn.id;
    resetEdits();
  });

  const DEFAULT_LIMIT = 200;
  // Keyed by tab id; the Run/Cancel button and timer show the active tab's run only.
  const running = $state<Record<string, { id: string; started: number }>>({});
  const busy = $derived(running[tab.id]);
  let now = $state(Date.now());
  $effect(() => {
    if (!busy) return;
    const t = setInterval(() => (now = Date.now()), 250);
    return () => clearInterval(t);
  });

  /** Runs the selection, or the whole editor. `again` re-runs the shown result with a higher limit. */
  async function run(again?: { sql: string; limit: number }) {
    // The result lands on this tab even if the user switches tabs while it runs.
    const t = tab;
    if (running[t.id] || (!again && !confirmDiscard())) return;
    const sql = again?.sql ?? (editor?.selection().trim() || t.query);
    const k = statementKind(sql);
    if (k === 'empty') return;
    const c = conn;
    const dbName = database;
    const limit = again?.limit ?? DEFAULT_LIMIT;
    const id = crypto.randomUUID();
    const started = Date.now();
    running[t.id] = { id, started };
    now = started;
    let r: QueryResult;
    try {
      r = toResult(await api.runQuery({ connId: c.id, database: dbName, sql, queryId: id, limit, tabId: t.id }), isMock, limit, dbName);
    } catch (e) {
      const err = ApiError.from(e);
      r = { columns: [], rows: [], ms: Date.now() - started, error: err.code === 'cancelled' ? 'Query cancelled' : err.message, live: !isMock };
    } finally {
      delete running[t.id];
      refreshStatus(t);
    }
    t.result = r;
    t.resultConn = c;
    if (again) return;
    logQuery({
      kind: k === 'write' ? 'WRITE' : 'READ',
      source: 'You',
      sql: sql.trim(),
      result: r.error ? { status: 'error', message: r.error } : { status: 'ok', rows: r.rows.length },
      ms: r.ms,
    });
  }

  function cancel() {
    if (busy) api.cancelQuery(busy.id).catch(() => {});
  }

  // Error of the last Commit/Rollback button press, shown on its tab only.
  let txError = $state<{ tab: string; text: string } | null>(null);
  let txBusy = $state(false);
  async function endActive(commit: boolean) {
    const t = tab;
    txBusy = true;
    txError = null;
    try {
      await endTx(t, commit);
    } catch (e) {
      txError = { tab: t.id, text: ApiError.from(e).message };
    } finally {
      txBusy = false;
    }
  }

  // Create and duplicate point the active tab at the new connection.
  async function canSwitch() {
    if (!confirmDiscard()) return false;
    const t = tab;
    if (!(await settlePending([t], 'Switching the connection'))) return false;
    leaveSession(t);
    return true;
  }

  // Saving a connection drops every tab session on it (the backend reconnects with the new settings).
  const canUpdate = (c: Conn) => settlePending(tabs.list.filter((t) => t.connId === c.id), 'Saving the connection');

  // Tauri: closing the window or menu Quit with a pending transaction asks here first; the backend rolls back
  // whatever is left. A quit during another tab prompt waits for it (Cancel there cancels the quit too).
  $effect(() => {
    if (api.kind !== 'tauri') return;
    let off: (() => void) | undefined;
    let gone = false;
    import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen('sandb://quit-requested', async () => {
          if (await settlePending(tabs.list, 'Quitting sandb', true)) api.quitApp().catch(() => {});
        }),
      )
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  });

  // Bridge and mock: best effort only. The browser asks before leaving; on leave, sessions are dropped (rolled back).
  // Every tab, as `status` may be stale; the bridge request uses keepalive so the unload does not cancel it.
  $effect(() => {
    if (api.kind === 'tauri') return;
    const onbefore = (e: BeforeUnloadEvent) => {
      if (tabs.list.some((t) => t.status?.pending)) e.preventDefault();
    };
    const onhide = () => {
      for (const t of tabs.list) api.closeTabSession(t.id).catch(() => {});
    };
    window.addEventListener('beforeunload', onbefore);
    window.addEventListener('pagehide', onhide);
    return () => {
      window.removeEventListener('beforeunload', onbefore);
      window.removeEventListener('pagehide', onhide);
    };
  });

  function loadMore() {
    const r = tab.result;
    if (r?.sql && r.limit && r.isRead && !r.summary) run({ sql: r.sql, limit: r.limit * 2 });
  }
  function newConnection() {
    connEdit = null;
    connDialog = true;
  }

  function editConnection(c: Conn) {
    connEdit = c;
    connDialog = true;
  }

  // The copy is saved (and switched to) only when the user clicks Save in the dialog.
  function duplicateConnection(c: Conn) {
    const name = `${c.name} (copy)`;
    connEdit = { ...c, id: name.toLowerCase().replace(/[^a-z0-9]+/g, '-') + '-' + Date.now().toString(36), name, aiWrite: false, copyOf: c.id };
    connDialog = true;
  }

  // The dialog has already asked about pending edits when the active connection's database changed.
  function connEdited(c: Conn, dbChanged: boolean) {
    // The save closed the tab sessions on this connection.
    for (const t of tabs.list) if (t.connId === c.id) refreshStatus(t);
    if (!dbChanged) return;
    for (const t of tabs.list) if (t.connId === c.id) t.database = defaultDb(c);
    if (c.id !== conn.id) return;
    resetEdits(true);
    if (!isMock) return openFirstTable();
    if (tab.result?.sql) tab.result = runMock(tab.result.sql, schema, conn);
  }

  // Throws on a backend error; DeleteConnectionDialog shows it and keeps the connection.
  async function deleteConnection(c: Conn) {
    if (db.connections.length <= 1) return;
    const wasActive = c.id === conn.id;
    if (wasActive && !confirmDiscard()) return;
    // One decision for every tab on the connection; Cancel leaves the delete dialog open.
    if (!(await settlePending(tabs.list.filter((t) => t.connId === c.id), 'Deleting the connection'))) return;
    await api.deleteConnection(c.id);
    db.connections.splice(db.connections.findIndex((x) => x.id === c.id), 1);
    connDelete = null;
    // Other tabs on it move too; their results would otherwise stay editable against a deleted connection.
    for (const t of tabs.list) {
      if (t.connId !== c.id) continue;
      setTabConn(t, db.connections[0]);
      t.result = null;
      t.opened = null;
      // The backend dropped its session with the connection.
      t.status = null;
    }
    if (wasActive) openFirstTable();
  }

  function openFirstTable() {
    if (isMock) return openTable(tables[0]);
    // Real databases: nothing runs until the user opens a table or runs a query.
    tab.result = null;
    tab.opened = null;
  }

  /** `t`: a table name (`schema.table` on PG outside public, as the palette lists it). */
  function openTable(t: string, at?: { database: string; schema: string | null }) {
    if (!confirmDiscard()) return;
    if (isMock) {
      tab.query = `SELECT *\nFROM ${conn.engine === 'pg' ? nsOf(conn.engine, t) + '.' : ''}${t}\nLIMIT 200;`;
      return run();
    }
    let schemaName = at?.schema ?? null;
    let table = t;
    if (!at && conn.engine === 'pg') {
      const dot = t.indexOf('.');
      [schemaName, table] = dot > 0 ? [t.slice(0, dot), t.slice(dot + 1)] : ['public', t];
    }
    if (at && conn.engine !== 'sqlite') tab.database = at.database;
    const dbName = at?.database ?? database;
    // The run limit pages the rows, so the SQL has no LIMIT and "Load more" can fetch further.
    const from = schemaName ? `${quoteIdent(conn.engine, schemaName)}.${quoteIdent(conn.engine, table)}` : quoteIdent(conn.engine, table);
    tab.query = `SELECT *\nFROM ${from};`;
    tab.opened = { key: `${conn.id}/${dbName}/${schemaName ?? ''}/${table}`, sql: tab.query.replace(/;\s*$/, '') };
    run();
  }</script>

<div class="flex h-full flex-col">
  <!-- Title bar. data-tauri-drag-region acts on the element itself only, so the empty strips drag the window and the buttons stay clickable. -->
  <div class="flex h-10 shrink-0 items-center px-2 select-none" data-tauri-drag-region>
    <div class="flex h-full w-56 shrink-0 items-center gap-2 xl:w-60 {inset ? 'pl-[76px]' : 'pl-2.5'}" data-tauri-drag-region>
      <Database size={14} strokeWidth={2.25} class="shrink-0 text-accent" />
      <span class="text-[13px] font-semibold tracking-[-0.01em]" data-tauri-drag-region>sandb</span>
    </div>
    {#if db.connections.length}
      <!-- The strip starts at the main card's left edge, so the first tab lines up with the card below. -->
      <div bind:this={tabStrip} class="ml-1.5 flex h-full min-w-0 items-center gap-1 overflow-x-auto pl-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {#each tabs.list as t (t.id)}
          {@const on = t.id === tab.id && ui.page === 'query'}
          <div class="group flex h-7 shrink-0 items-center rounded-md text-[12.5px] {on ? 'tab-on font-medium text-ink' : 'text-muted hover:bg-hover hover:text-ink'}">
            {#if renaming === t.id}
              <span class="flex items-center gap-2 pl-2.5"><FileCode size={13} class="text-faint" />
                <input class="w-28 bg-transparent outline-none" value={t.title} aria-label="Tab name" use:focusSelect onblur={(e) => rename(t, e.currentTarget.value)} onkeydown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); else if (e.key === 'Escape') renaming = null; }} />
              </span>
            {:else}
              <button class="flex h-full items-center gap-2 pl-2.5" aria-current={t.id === tab.id ? 'page' : undefined} onclick={() => pickTab(t.id)} ondblclick={() => (renaming = t.id)} title="Double-click to rename">
                <FileCode size={13} class={on ? 'text-accent' : 'text-faint'} /> <span class="max-w-40 truncate">{t.title}</span>
                {#if running[t.id] && !on}<LoaderCircle size={12} class="animate-spin text-faint" />{/if}
                {#if t.status?.pending}<span class="size-1.5 rounded-full bg-amber-500" title="Transaction open"></span>{/if}
              </button>
            {/if}
            <button class="mr-1 ml-1.5 grid size-[18px] place-items-center rounded text-faint hover:bg-hover hover:text-ink {on ? '' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}" title="Close tab" aria-label="Close {t.title}" onclick={() => dropTab(t)}><X size={12} /></button>
          </div>
        {/each}
      </div>
      <button class="ml-1 grid size-7 shrink-0 place-items-center rounded-md text-faint hover:bg-hover hover:text-ink" title="New query" aria-label="New query" onclick={addTab}><Plus size={15} /></button>
    {/if}
    <div class="h-full min-w-6 flex-1" data-tauri-drag-region></div>
    {#if updates.status === 'available' || updates.status === 'restart'}
      <button class="mr-1 flex h-7 items-center gap-1.5 rounded-md px-2 text-[12px] font-medium text-amber-600 hover:bg-hover dark:text-amber-400" title="sandb v{updates.next} is available" onclick={() => (ui.page = 'settings')}><CircleArrowUp size={14} /> Update</button>
    {/if}
    <button class="btn btn-ghost size-7 p-0 {ui.chatOpen ? 'bg-hover text-ink' : ''}" onclick={() => (ui.chatOpen = !ui.chatOpen)} title="Toggle AI chat" aria-label="Toggle AI chat"><PanelRight size={15} /></button>
  </div>

  <div class="relative flex min-h-0 flex-1 gap-2 px-2 pb-2">
  <!-- Sidebar: sits on the canvas, so it reads as one piece with the title bar. -->
  <aside class="flex w-56 shrink-0 flex-col overflow-hidden xl:w-60">
    <div class="pt-1 pb-2">
      <button class="flex h-7 w-full items-center gap-2 rounded-md bg-hover px-2.5 text-muted hover:bg-active hover:text-ink" onclick={() => (paletteOpen = true)}>
        <Search size={13} /> <span class="flex-1 text-left">Quick search</span>
        <kbd class="text-[11px]">⌘K</kbd>
      </button>
    </div>

    <nav class="-mx-1 flex-1 overflow-auto px-1 pb-2">
      <ConnectionTree {selected} onopen={openTableIn} onnew={newConnection} onedit={editConnection} onduplicate={duplicateConnection} ondelete={(c) => (connDelete = c)} />
    </nav>

    <div class="flex items-center gap-0.5 pt-1">
      <button class="nav-item flex-1 {ui.page === 'settings' ? 'nav-item-active' : ''}" onclick={() => (ui.page = ui.page === 'settings' ? 'query' : 'settings')}><Settings size={14} /> Settings</button>
      <button class="nav-item size-[26px] justify-center p-0 {ui.page === 'audit' ? 'nav-item-active' : ''}" title="Audit log" aria-label="Audit log" onclick={() => (ui.page = ui.page === 'audit' ? 'query' : 'audit')}><ScrollText size={14} /></button>
      <button class="btn btn-ghost size-[26px] p-0" title="Toggle theme" onclick={toggleTheme}>
        {#if ui.dark}<Sun size={14} />{:else}<Moon size={14} />{/if}
      </button>
    </div>
  </aside>

  <!-- Main -->
  <main class="card flex min-w-[300px] flex-1 flex-col overflow-hidden">
    {#if ui.page === 'settings'}
      <SettingsPage onEnableWrites={(c) => (guardTarget = c)} />
    {:else if ui.page === 'audit'}
      <AuditPage />
    {:else if !db.connections.length}
      <div class="grid flex-1 place-items-center p-6 text-center">
        <div class="max-w-xs space-y-2">
          <div class="mx-auto grid size-9 place-items-center rounded-lg bg-accent/12 text-accent"><Database size={17} /></div>
          <p class="font-medium">{db.loading ? 'Loading connections…' : 'No connections yet'}</p>
          {#if db.loadError}<p class="text-[12px] text-red-600 dark:text-red-400" role="alert">{db.loadError}</p>{/if}
          {#if !db.loading}<button class="btn btn-primary mx-auto" onclick={newConnection}><Plus size={13} /> New connection</button>{/if}
        </div>
      </div>
    {:else}
    <div class="flex h-11 shrink-0 items-center gap-1.5 px-2.5">
      <TargetBar {conn} {database} onconn={switchConn} ondb={switchDb} />
      <!-- The container query sits on this group, not on the toolbar: in WebKit before 18.2 a size container
           also traps TargetBar's fixed menu backdrop. A size container has no content width, so min-w keeps
           room for the icon-only buttons; labels show only when the group has more room. -->
      <div class="@container flex flex-1 items-center gap-1.5 {manualCommit(conn) ? 'min-w-[88px]' : 'min-w-0'}">
      {#if manualCommit(conn)}
        {@const st = tab.status}
        <span class="mx-1 h-4 w-px shrink-0 bg-line"></span>
        <!-- An aborted transaction counts as pending; Rollback is the way out of it. Disabled buttons do not show tooltips reliably, so the wrappers carry the title. -->
        <span class="shrink-0" title="Commit the tab's open transaction"><button class="btn btn-ghost px-2 @[16rem]:px-2.5" onclick={() => endActive(true)} disabled={!st?.pending || st.aborted || !!busy || txBusy} title="Commit the tab's open transaction" aria-label="Commit"><Check size={14} /> <span class="hidden @[16rem]:inline">Commit</span></button></span>
        <span class="shrink-0" title="Roll back the tab's open transaction"><button class="btn btn-ghost px-2 @[16rem]:px-2.5" onclick={() => endActive(false)} disabled={!(st?.pending || st?.aborted) || !!busy || txBusy} title="Roll back the tab's open transaction" aria-label="Rollback"><Undo2 size={14} /> <span class="hidden @[16rem]:inline">Rollback</span></button></span>
        {#if st?.pending}
          <span class="flex min-w-0 items-center gap-1.5 text-[12px] font-medium text-amber-700 dark:text-amber-400" role="status" title="Transaction open"><span class="size-1.5 shrink-0 rounded-full bg-amber-500"></span> <span class="truncate">Transaction open</span></span>
        {/if}
        {#if txError?.tab === tab.id}<span class="min-w-0 truncate text-[12px] text-red-700 dark:text-red-400" role="alert">{txError.text}</span>{/if}
      {/if}
      </div>
      {#if kind === 'read'}
        <span class="hidden shrink-0 px-1 text-[12px] font-medium text-emerald-700 lg:inline dark:text-emerald-400">Read</span>
      {:else if kind === 'write'}
        <span class="shrink-0 px-1 text-[12px] font-medium text-amber-700 dark:text-amber-400">Write</span>
      {/if}
      <!-- With the chat already open this only repeats it, so narrow toolbars drop it. -->
      <button class="btn btn-ghost shrink-0 px-2 2xl:px-2.5 {ui.chatOpen ? 'hidden lg:inline-flex' : ''}" onclick={() => (ui.chatOpen = true)} title="Explain with AI"><Sparkles size={14} /> <span class="hidden 2xl:inline">Explain with AI</span></button>
      {#if busy}
        <span class="font-mono text-[11.5px] text-muted tabular-nums">{((now - busy.started) / 1000).toFixed(1)} s</span>
        <button class="btn btn-secondary" onclick={cancel} title="Cancel the running query"><Square size={11} class="fill-current" /> Cancel</button>
      {:else}
        <button class="btn btn-primary pr-2" onclick={() => run()} disabled={kind === 'empty'} title="Run the selection, or the whole editor"><Play size={11} class="fill-current" /> Run <kbd class="ml-0.5 hidden font-sans text-[11px] font-normal lg:inline" aria-hidden="true">⌘↵</kbd></button>
      {/if}
    </div>

    <div class="h-[34%] min-h-28 overflow-hidden">
      <!-- Keyed so undo never crosses into another tab's text. -->
      {#key tab.id}
        <SqlEditor bind:this={editor} bind:value={tab.query} onrun={() => run()} {schema} dialect={conn.engine} />
      {/key}
    </div>

    <div class="min-h-0 flex-1 border-t border-line">
      <ResultsGrid result={tab.result} conn={tab.resultConn} {schema} loading={!!busy} onloadmore={loadMore} onsaved={() => tab.result?.sql && (tab.result = runMock(tab.result.sql, schema, conn))} />
    </div>
    {/if}
  </main>

  {#if ui.chatOpen}
    <ChatPanel onEnableWrites={() => (guardTarget = conn)} onInsertSql={(s) => (tab.query = s)} />
  {/if}
  </div>
</div>
<SafeguardDialog bind:target={guardTarget} />
<ConnectionDialog bind:open={connDialog} conn={connEdit} canswitch={canSwitch} canupdate={canUpdate} onsaved={openFirstTable} onedited={connEdited} />
<DeleteConnectionDialog bind:target={connDelete} ondelete={deleteConnection} />
<WritePreview />
<CommandPalette
  bind:open={paletteOpen}
  {tables}
  onfocuseditor={focusEditor}
  onrun={() => { if (kind === 'empty') return; ui.page = 'query'; run(); }}
  ontogglechat={() => (ui.chatOpen = !ui.chatOpen)}
  onpage={(p) => (ui.page = p)}
  ontoggletheme={toggleTheme}
  onnewconnection={newConnection}
  onopentable={(t) => { ui.page = 'query'; openTable(t); }}
  onswitch={(c) => { ui.page = 'query'; switchConn(c); }}
/>
<!-- Last, so it stacks above the connection and delete dialogs that wait on it. -->
<PendingTxDialog />
