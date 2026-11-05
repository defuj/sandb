<script lang="ts">
  import { untrack } from 'svelte';
  import { Download, ArrowUpDown, ArrowUp, ArrowDown, ArrowUpRight, Check, X, Pencil, Plus, Minus, Eye, Undo2, CircleSlash, LoaderCircle, CircleAlert, CircleCheck } from '@lucide/svelte';
  import { pkOf, tableData, type QueryResult, type Cell, type Row, type Schema } from '../mock';
  import { canWrite, type Conn } from '../state.svelte';
  import { activeTab } from '../tabs.svelte';
  import { edits, editBlocker, prepareEdit, pendingCount, resetEdits, setCell, addRow, parseInput, buildStatements, saveEdits } from '../edits.svelte';
  import EditSaveDialog from './EditSaveDialog.svelte';
  import { api, ApiError, type ForeignKeyInfo, type ResultSort } from '../api';
  import { csvCell, formatCell, isBytes } from '../results';

  /** `foreignKeys` + `onfollow`: a table Data view; cells of a foreign key column get a link to the referenced row. */
  let {
    result,
    conn,
    schema,
    loading = false,
    onloadmore,
    onsaved,
    sort = null,
    onsort,
    foreignKeys = [],
    onfollow,
  }: { result: QueryResult | null; conn: Conn; schema: Schema; loading?: boolean; onloadmore?: () => void; onsaved: () => void; sort?: ResultSort | null; onsort?: (sort: ResultSort) => void; foreignKeys?: ForeignKeyInfo[]; onfollow?: (fk: ForeignKeyInfo, row: Row) => void } = $props();

  // Column → the first foreign key it belongs to; only keys whose columns are all in the result.
  const fkOf = $derived.by(() => {
    const m = new Map<string, ForeignKeyInfo>();
    if (!onfollow || !result) return m;
    for (const fk of foreignKeys) {
      if (!fk.columns.every((c) => result.columns.includes(c))) continue;
      for (const c of fk.columns) if (!m.has(c)) m.set(c, fk);
    }
    return m;
  });
  // A link needs every key value as saved: no NULL, no binary or JSON value, no unsaved change.
  const linkOf = (r: GridRow, c: string) => {
    const fk = fkOf.get(c);
    if (!fk || r.isNew || isDeleted(r)) return null;
    return fk.columns.every((k) => r.row[k] !== null && typeof r.row[k] !== 'object' && !isChanged(r, k)) ? fk : null;
  };

  /** key: `r<index>` for result rows, `a<n>` for rows added in edit mode. */
  type GridRow = { key: string; idx: number; row: Row; isNew: boolean };

  const sorted = $derived<GridRow[]>(result?.rows.map((row, idx) => ({ key: 'r' + idx, idx, row, isNew: false })) ?? []);
  const rows = $derived<GridRow[]>([...sorted, ...edits.added.map((a) => ({ key: 'a' + a.key, idx: -1, row: a.row, isNew: true }))]);

  const blocker = $derived(canWrite(conn) ? editBlocker(result) : 'Read-only connection: edits are off');
  const count = $derived(pendingCount());

  // Scrolling near the end of the grid loads the next page. Not while edits are pending: a new result drops them.
  let scroller = $state<HTMLDivElement>();
  const canLoadMore = $derived(!!result?.truncated && !!onloadmore && !!result.isRead && !result.summary && !loading && !count);
  function loadIfAtEnd() {
    const el = scroller;
    if (el && canLoadMore && el.scrollHeight - el.scrollTop - el.clientHeight < 200) onloadmore?.();
  }
  // Also after each page lands: a pane taller than the rows has no scroll event to wait for.
  $effect(() => {
    if (canLoadMore) untrack(loadIfAtEnd);
  });

  // A new result (or a connection turned read-only) drops edit mode unless the result is still editable.
  $effect(() => {
    if (blocker) edits.on = false;
  });

  function sortBy(c: string) {
    if (!result?.isRead || result.summary || loading) return;
    onsort?.({ column: c, columnIndex: result.columns.indexOf(c), ascending: sort?.column === c ? !sort.ascending : true });
  }

  const MIN_COLUMN_WIDTH = 48;
  const MAX_COLUMN_WIDTH = 640;
  let columnWidths = $state<number[]>([]);
  let tableWidth = $state<number | null>(null);
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  function measuredColumns(handle: HTMLElement) {
    const table = handle.closest('table');
    const header = table?.tHead?.rows[0];
    if (!table || !header || !result) return null;
    if (columnWidths.length === result.columns.length && tableWidth !== null) return { widths: [...columnWidths], width: tableWidth };
    const widths = Array.from(header.cells).slice(1).map((cell) => cell.getBoundingClientRect().width);
    const width = table.getBoundingClientRect().width;
    columnWidths = [...widths];
    tableWidth = width;
    return { widths, width };
  }

  function setColumnWidth(ci: number, value: number, initial: { widths: number[]; width: number }) {
    const width = Math.round(clamp(value, MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH));
    columnWidths[ci] = width;
    tableWidth = initial.width + width - initial.widths[ci];
  }

  // Pointer capture keeps the moves coming after the pointer leaves the thin handle.
  function resizeColumn(e: PointerEvent, ci: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    const initial = measuredColumns(el);
    if (!initial) return;
    const x = e.clientX;
    el.setPointerCapture(e.pointerId);
    el.onpointermove = (m) => setColumnWidth(ci, initial.widths[ci] + m.clientX - x, initial);
    el.onlostpointercapture = () => (el.onpointermove = el.onlostpointercapture = null);
  }

  function keyColumnResize(e: KeyboardEvent, ci: number) {
    const d = { ArrowLeft: -16, ArrowRight: 16 }[e.key];
    if (!d) return;
    e.preventDefault();
    const initial = measuredColumns(e.currentTarget as HTMLElement);
    if (initial) setColumnWidth(ci, initial.widths[ci] + d, initial);
  }

  let exportMsg = $state<{ ok: boolean; text: string } | null>(null);
  $effect(() => {
    result;
    exportMsg = null;
  });

  // The desktop app streams every row of a single read to a file it asks for; otherwise the loaded rows export in the browser.
  async function exportCsv() {
    if (!result) return;
    const r = result;
    if (api.kind === 'tauri' && r.live && r.sql && r.isRead && !r.summary) {
      try {
        // The grid shows the active tab's result; its tab id reads inside the tab's open transaction.
        const out = await api.exportCsv(conn.id, r.database ?? conn.database, r.sql, activeTab()?.id);
        if (!out.cancelled) exportMsg = { ok: true, text: `Exported ${out.rows} rows to ${out.path}` };
      } catch (e) {
        exportMsg = { ok: false, text: ApiError.from(e).message };
      }
      return;
    }
    const csv = [r.columns.map(csvCell).join(','), ...sorted.map((x) => r.columns.map((c) => csvCell(x.row[c])).join(','))].join('\r\n');
    const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(new Blob([csv], { type: 'text/csv' })), download: 'result.csv' });
    a.click();
    URL.revokeObjectURL(a.href);
  }

  // ---- edit mode ----
  let selected = $state<Record<string, true>>({});
  let editing = $state<{ key: string; col: string } | null>(null);
  let draft = $state('');
  let menu = $state<{ x: number; y: number; r: GridRow; col: string } | null>(null);
  let dialog = $state<'preview' | 'save' | null>(null);
  let inputError = $state<string | null>(null);
  let preparing = $state(false);
  /** Full text of the cell open in the editor when the grid shows it cut off; saving it unchanged is not an edit. */
  let full: { key: string; col: string; text: string | null } | null = null;
  let loadingCell = $state(false);

  // Row keys are result indexes, so they point at other rows once the result changes.
  // Only the result identity triggers a reset: prepareEdit sets `result.edit` on the same result, and that must not clear column widths.
  $effect(() => {
    const r = result;
    untrack(() => {
      selected = {};
      editing = null;
      menu = null;
      columnWidths = [];
      tableWidth = null;
      if (r?.live && !r.edit) edits.on = false;
    });
  });

  const value = (r: GridRow, c: string): Cell => (!r.isNew && edits.changed[r.idx] && c in edits.changed[r.idx] ? edits.changed[r.idx][c] : r.row[c]);
  const isChanged = (r: GridRow, c: string) => !r.isNew && !!edits.changed[r.idx] && c in edits.changed[r.idx];
  const isDeleted = (r: GridRow) => !r.isNew && !!edits.deleted[r.idx];
  const sample = (c: string) => result?.rows.find((x) => x[c] !== null)?.[c] ?? null;

  async function enableEdit(): Promise<boolean> {
    if (!result || preparing) return false;
    if (blocker) {
      edits.error = { message: blocker, key: '' };
      return false;
    }
    preparing = true;
    const reason = await prepareEdit(result, conn);
    preparing = false;
    if (reason) {
      edits.error = { message: reason, key: '' };
      return false;
    }
    edits.on = true;
    return true;
  }

  async function toggleEdit() {
    if (edits.on && count && !confirm(`Discard ${count} unsaved change(s)?`)) return;
    if (edits.on) resetEdits();
    else if (!(await enableEdit())) return;
    selected = {};
    editing = null;
  }

  function write(r: GridRow, c: string, v: Cell) {
    if (r.isNew) {
      const a = edits.added.find((x) => 'a' + x.key === r.key);
      if (a) a.row[c] = v;
    } else setCell(r.idx, r.row, c, v);
  }

  async function startEdit(r: GridRow, c: string) {
    if (isDeleted(r)) return;
    const current = result;
    if (!edits.on || (current?.live && !current.edit)) {
      if (!(await enableEdit())) return;
    }
    if (result !== current || isDeleted(r)) return;
    const v = value(r, c);
    if (isBytes(v)) {
      edits.error = { message: 'Binary preview cells are read-only', key: r.key };
      return;
    }
    let text: string | null = v === null ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v);
    full = null;
    // The grid gets at most 4 KB per cell; load the whole value from the row's primary key.
    if (typeof v === 'string' && v.endsWith('…') && !r.isNew && !isChanged(r, c)) {
      const pk = current?.live ? current.edit?.pk : undefined;
      if (!current || !pk?.length || !current.sql) {
        edits.error = { message: 'Truncated text cells are read-only', key: r.key };
        return;
      }
      if (loadingCell) return;
      loadingCell = true;
      try {
        text = await api.cellValue({
          connId: conn.id,
          database: current.database ?? conn.database,
          sql: current.sql,
          tabId: activeTab()?.id,
          pk: Object.fromEntries(pk.map((k) => [k, r.row[k]])),
          column: c,
        });
      } catch (e) {
        edits.error = { message: ApiError.from(e).message, key: r.key };
        return;
      } finally {
        loadingCell = false;
      }
      if (result !== current || isDeleted(r)) return;
      full = { key: r.key, col: c, text };
      text ??= '';
    }
    draft = text;
    inputError = null;
    editing = { key: r.key, col: c };
  }

  function commit(r: GridRow, c: string, blur = false) {
    // Esc clears `editing` first, so the blur that follows does nothing.
    if (editing?.key !== r.key || editing.col !== c) return;
    const before = value(r, c);
    // A loaded full value saved as it is stays unchanged; its cut-off preview is the original.
    if (full?.key === r.key && full.col === c && draft === (full.text ?? '')) {
      full = null;
      editing = null;
      return;
    }
    const parsed = parseInput(draft, sample(c), result?.types?.[result.columns.indexOf(c)]);
    if (parsed === undefined) {
      // Enter keeps the editor open with the error; leaving the cell drops the invalid draft.
      if (!blur) return void (inputError = 'Invalid value');
    } else if (!(before === null && draft === '')) write(r, c, parsed); // Leaving a NULL cell empty keeps it NULL instead of turning it into ''.
    full = null;
    editing = null;
  }

  function focusInput(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  function pickRow(r: GridRow, e: MouseEvent) {
    if (!edits.on) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey) {
      if (selected[r.key]) delete selected[r.key];
      else selected[r.key] = true;
    } else selected = { [r.key]: true };
  }

  function deleteRows(keys: string[]) {
    for (const k of keys) {
      const r = rows.find((x) => x.key === k);
      if (!r) continue;
      if (r.isNew) edits.added = edits.added.filter((a) => 'a' + a.key !== k);
      else if (edits.deleted[r.idx]) delete edits.deleted[r.idx];
      else edits.deleted[r.idx] = true;
    }
    selected = {};
  }

  function newRow() {
    if (!result) return;
    const row: Row = Object.fromEntries(result.columns.map((c) => [c, null]));
    const pk = pkOf(result.table ?? '');
    if (api.kind === 'mock' && result.table && pk.length === 1) {
      const d = tableData(schema, result.table);
      const ids = [...d.base, ...d.added, ...edits.added.map((a) => a.row)].map((x) => x[pk[0]]).filter((v): v is number => typeof v === 'number');
      if (ids.length) row[pk[0]] = Math.max(...ids) + 1;
    }
    addRow(row);
  }

  function openMenu(e: MouseEvent, r: GridRow, c: string) {
    if (!edits.on) return;
    e.preventDefault();
    if (!selected[r.key]) selected = { [r.key]: true };
    menu = { x: Math.min(e.clientX, innerWidth - 170), y: Math.min(e.clientY, innerHeight - 130), r, col: c };
  }

  async function doSave() {
    if (!result) return;
    const ok = await saveEdits(result, conn, activeTab()?.id);
    dialog = null;
    if (ok) onsaved();
  }

  const stmts = $derived(dialog && result && !blocker ? buildStatements(result, conn) : []);

  function rowTint(r: GridRow) {
    if (edits.error?.key === r.key) return 'bg-red-500/15 outline outline-1 -outline-offset-1 outline-red-500/60';
    if (isDeleted(r)) return 'bg-red-500/8 text-faint line-through decoration-red-500/60 dark:bg-red-500/10';
    if (r.isNew) return 'bg-emerald-500/8 dark:bg-emerald-500/10';
    if (selected[r.key]) return 'bg-accent/10';
    return 'hover:bg-hover';
  }
</script>

<svelte:window
  onclick={() => (menu = null)}
  onbeforeunload={(e) => { if (pendingCount()) e.preventDefault(); }}
  onkeydown={(e) => { if (e.key === 'Escape') menu = null; }}
/>

{#if !result}
  <div class="grid h-full place-items-center">
    <div class="text-center">
      <div class="text-[13px] font-medium text-ink">No results yet</div>
      <div class="mt-1 flex items-center justify-center gap-1.5 text-[12.5px] text-muted">Write a query and press <kbd class="kbd">⌘↵</kbd></div>
    </div>
  </div>
{:else if result.error}
  <div class="p-4">
    <div class="flex items-start gap-2.5 rounded-lg bg-red-500/8 px-3.5 py-3" role="alert">
      <CircleAlert size={15} class="mt-px shrink-0 text-red-600 dark:text-red-400" />
      <div class="min-w-0">
        <span class="text-[12.5px] font-semibold text-red-700 dark:text-red-400">Error</span>
        <pre class="mt-1 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-red-800 dark:text-red-300">{result.error}</pre>
      </div>
    </div>
  </div>
{:else if result.note}
  <div class="flex h-full flex-col gap-3 overflow-auto p-4">
    <div class="flex items-center gap-2.5">
      <CircleCheck size={15} class="shrink-0 text-emerald-600 dark:text-emerald-400" />
      <span><span class="sr-only">Succeeded: </span>{result.note} <span class="font-mono text-[11.5px] text-faint">· {result.ms} ms</span></span>
    </div>
    {#if result.notices}{@render notices(result.notices)}{/if}
    {#if result.summary}{@render summary(result.summary)}{/if}
  </div>
{:else}
  <div class="flex h-full flex-col">
    {#if result.notices}<div class="border-b border-line px-3 py-1.5">{@render notices(result.notices)}</div>{/if}
    {#if result.summary}<div class="border-b border-line px-3 py-1.5">{@render summary(result.summary)}</div>{/if}
    <div class="min-h-0 flex-1 overflow-auto" bind:this={scroller} onscroll={loadIfAtEnd}>
      <table class="w-full border-separate border-spacing-0 font-mono text-[12px] {tableWidth === null ? '' : 'table-fixed'}" style:width={tableWidth === null ? undefined : `${tableWidth}px`}>
        <colgroup>
          <col class="w-9" />
          {#each result.columns as _, ci}
            <col style:width={columnWidths[ci] === undefined ? undefined : `${columnWidths[ci]}px`} />
          {/each}
        </colgroup>
        <thead class="sticky top-0 z-10">
          <tr>
            <th class="w-9 border-b border-line bg-surface px-2 py-0 text-right font-sans text-[10.5px] font-normal text-faint"></th>
            {#each result.columns as c, ci}
              <th class="group/th relative h-8 border-b border-l border-line bg-surface px-2.5 py-0 text-left font-normal" title={result.types?.[ci]}>
                <button class="flex w-full items-baseline gap-1.5 text-left whitespace-nowrap" onclick={() => sortBy(c)}>
                  <span class="font-sans text-[12px] font-semibold {sort?.column === c ? 'text-accent' : 'text-ink'}">{c}</span>
                  {#if result.types?.[ci]}<span class="max-w-72 min-w-0 truncate text-[10.5px] text-faint">{result.types?.[ci]}</span>{/if}
                  <span class="ml-auto self-center">
                    {#if sort?.column === c}
                      {#if sort?.ascending}<ArrowUp size={12} class="text-accent" />{:else}<ArrowDown size={12} class="text-accent" />{/if}
                    {:else}<ArrowUpDown size={12} class="text-faint opacity-0 transition-opacity group-hover/th:opacity-100" />{/if}
                  </span>
                </button>
                <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
                <div
                  class="absolute inset-y-0 -right-1 z-20 w-2 cursor-col-resize hover:bg-accent/25 focus-visible:bg-accent/40"
                  role="separator"
                  aria-orientation="vertical"
                  aria-label={`Resize ${c} column`}
                  aria-valuemin={MIN_COLUMN_WIDTH}
                  aria-valuemax={MAX_COLUMN_WIDTH}
                  aria-valuenow={columnWidths[ci]}
                  title={`Resize ${c} column`}
                  tabindex="0"
                  onfocus={(e) => measuredColumns(e.currentTarget)}
                  onpointerdown={(e) => resizeColumn(e, ci)}
                  onkeydown={(e) => keyColumnResize(e, ci)}
                ></div>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as r, i (r.key)}
            <tr class="group/row {rowTint(r)}">
              <td
                class="border-b border-line/60 px-2 text-right text-[10.5px] tabular-nums {selected[r.key] ? 'font-medium text-ink' : 'text-faint'} {edits.on ? 'cursor-pointer' : ''}"
                onclick={(e) => pickRow(r, e)}
              >{r.isNew ? '+' : i + 1}</td>
              {#each result.columns as c}
                {@const v = value(r, c)}
                {@const isEd = editing?.key === r.key && editing.col === c}
                {@const fk = linkOf(r, c)}
                <td
                  class="relative h-[26px] {tableWidth === null ? 'max-w-72' : ''} truncate border-b border-l border-line/60 {fk ? 'pr-7 pl-2.5' : 'px-2.5'} {isChanged(r, c) && !isDeleted(r) ? 'bg-amber-400/15 dark:bg-amber-400/12' : ''} {typeof v === 'number' ? 'text-right' : ''}"
                  ondblclick={() => startEdit(r, c)}
                  onclick={(e) => { if (edits.on && !e.metaKey && !e.ctrlKey) selected = { [r.key]: true }; }}
                  oncontextmenu={(e) => openMenu(e, r, c)}
                >
                  {#if isEd}
                    <input
                      class="absolute inset-0 block h-full w-full min-w-0 bg-field px-2.5 font-mono text-[12px] outline-2 -outline-offset-2 {inputError ? 'outline-red-500' : 'outline-accent'}"
                      title={inputError}
                      aria-invalid={!!inputError}
                      oninput={() => (inputError = null)}
                      bind:value={draft}
                      use:focusInput
                      placeholder="NULL"
                      autocapitalize="off"
                      autocorrect="off"
                      spellcheck="false"
                      autocomplete="off"
                      onkeydown={(e) => {
                        if (e.key === 'Enter') commit(r, c);
                        else if (e.key === 'Escape') { e.stopPropagation(); editing = null; }
                      }}
                      onblur={() => commit(r, c, true)}
                    />
                  {/if}
                  {#if fk && !isEd}
                    {@const target = (fk.refSchema ? fk.refSchema + '.' : '') + fk.refTable}
                    <button
                      class="absolute top-1/2 right-1.5 inline-flex size-4 -translate-y-1/2 items-center justify-center rounded text-faint opacity-60 group-hover/row:opacity-100 hover:bg-accent/15 hover:text-accent"
                      title="Open {target} where {fk.refColumns.map((rc, k) => `${rc} = ${String(r.row[fk.columns[k]])}`).join(' and ')}"
                      aria-label="Open referenced row in {target}"
                      onclick={(e) => { e.stopPropagation(); onfollow?.(fk, r.row); }}
                      ondblclick={(e) => e.stopPropagation()}
                    ><ArrowUpRight size={12} /></button>
                  {/if}
                  <span class:invisible={isEd}>
                    {#if v === null}
                      <span class="text-[11px] italic {selected[r.key] ? 'text-muted' : 'text-faint'}">NULL</span>
                    {:else if typeof v === 'boolean'}
                      {#if v}<Check size={14} class="text-emerald-600 dark:text-emerald-400" />{:else}<X size={14} class="text-faint" />{/if}
                    {:else if typeof v === 'number'}
                      <span class="tabular-nums">{v}</span>
                    {:else if typeof v === 'object'}
                      <span class="text-muted">{formatCell(v)}</span>
                    {:else}
                      {v}
                    {/if}
                  </span>
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if edits.error}
      <div class="flex items-center gap-2 border-t border-red-500/25 bg-red-500/6 px-3 py-1.5 text-[12px] text-red-700 dark:text-red-400">
        <CircleAlert size={13} class="shrink-0" />
        <span class="sr-only">ERROR</span>
        <span class="min-w-0 flex-1 truncate" title={edits.error.message}>{edits.error.message}</span>
        <button class="btn btn-ghost h-5 px-1 text-[11px]" onclick={() => (edits.error = null)} aria-label="Dismiss"><X size={12} /></button>
      </div>
    {/if}
    <footer class="@container flex h-9 shrink-0 items-center gap-1 border-t border-line px-2 text-[12px] text-muted">
      <span title={blocker ?? (edits.on ? 'Leave edit mode' : 'Edit rows in this result')}>
        <button
          class="btn h-7 px-2 text-[12px] {edits.on ? 'bg-accent/6 text-accent' : 'btn-ghost'}"
          disabled={!!blocker || preparing}
          onclick={toggleEdit}
        ><Pencil size={13} /> Edit</button>
      </span>
      {#if edits.on}
        <span class="px-1 text-[12px] font-medium text-accent">Editing</span>
        <button class="btn btn-ghost h-7 px-2 text-[12px]" title="Add row" onclick={newRow}><Plus size={13} /><span class="hidden @[36rem]:inline">Row</span></button>
        <button class="btn btn-ghost h-7 px-2 text-[12px]" title="Delete selected rows" disabled={!Object.keys(selected).length} onclick={() => deleteRows(Object.keys(selected))}><Minus size={13} /><span class="hidden @[36rem]:inline">Delete</span></button>
        <span class="mx-1 h-4 w-px bg-line"></span>
      {/if}
      {#if count}
        <span class="px-1 text-[12px] font-medium text-ink">{count} change{count === 1 ? '' : 's'}</span>
        <button class="btn btn-ghost h-7 px-2 text-[12px]" title="Discard changes" onclick={() => resetEdits(true)}><Undo2 size={13} /><span class="hidden @[36rem]:inline">Discard</span></button>
        <button class="btn btn-ghost h-7 px-2 text-[12px]" title="Preview SQL" onclick={() => (dialog = 'preview')}><Eye size={13} /><span class="hidden @[36rem]:inline">Preview SQL</span></button>
        <button class="btn btn-primary h-7 px-3 text-[12px]" onclick={() => (dialog = 'save')}>Save</button>
      {:else}
        <span class="flex items-center gap-2.5 px-1.5 text-[12px] text-faint tabular-nums">
          <span title={result.truncated ? `First ${result.rows.length} rows; ${result.isRead ? 'scroll down to load more' : 'more exist. Only a single read-only statement loads more, because loading runs the SQL again'}` : undefined}>{rows.length}{result.truncated ? '+' : ''} rows</span>
          <span class="size-0.5 rounded-full bg-faint"></span>
          <span>{result.ms} ms</span>
        </span>
        {#if result.truncated && onloadmore && loading}
          <span class="flex items-center gap-1.5 px-1 text-[12px] text-faint"><LoaderCircle size={13} class="animate-spin" /> Loading more…</span>
        {/if}
      {/if}
      {#if exportMsg}
        <span class="min-w-0 truncate text-[12px] {exportMsg.ok ? 'text-emerald-700 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'}" title={exportMsg.text}>{exportMsg.text}</span>
      {/if}
      <button class="btn btn-ghost ml-auto h-7 px-2 text-[12px]" title="Export CSV" onclick={exportCsv}><Download size={13} /><span class="hidden @[36rem]:inline">Export CSV</span></button>
    </footer>
  </div>
{/if}

{#if menu}
  {@const m = menu}
  <div class="popover fixed z-40 w-40 p-1" style="left:{m.x}px;top:{m.y}px" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    {#if !isDeleted(m.r)}
      <button class="nav-item" onclick={() => { startEdit(m.r, m.col); menu = null; }}><Pencil size={13} /> Edit cell</button>
      <button class="nav-item" onclick={() => { write(m.r, m.col, null); menu = null; }}><CircleSlash size={13} /> Set NULL</button>
      {#if isChanged(m.r, m.col)}
        <button class="nav-item" onclick={() => { setCell(m.r.idx, m.r.row, m.col, m.r.row[m.col]); menu = null; }}><Undo2 size={13} /> Revert cell</button>
      {/if}
    {/if}
    <button class="nav-item {isDeleted(m.r) ? '' : 'text-red-600 dark:text-red-400'}" onclick={() => { deleteRows(Object.keys(selected).length ? Object.keys(selected) : [m.r.key]); menu = null; }}>
      {#if isDeleted(m.r)}<Undo2 size={13} /> Restore row{:else}<Minus size={13} /> Delete row{Object.keys(selected).length > 1 ? 's' : ''}{/if}
    </button>
  </div>
{/if}

{#snippet notices(lines: string[])}
  <ul class="space-y-0.5 font-mono text-[11.5px] text-amber-700 dark:text-amber-400" role="status">
    {#each lines as l, i (i)}<li>{l}</li>{/each}
  </ul>
{/snippet}

{#snippet summary(lines: string[])}
  <ol class="space-y-0.5 font-mono text-[11.5px] text-muted">
    {#each lines as l, i (i)}<li class="truncate" title={l}>{l}</li>{/each}
  </ol>
{/snippet}

<EditSaveDialog bind:mode={dialog} {stmts} {conn} onsave={doSave} />
