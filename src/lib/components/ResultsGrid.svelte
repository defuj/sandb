<script lang="ts">
  import { Download, ArrowUpDown, ArrowUp, ArrowDown, Check, X, Pencil, Plus, Minus, Eye, Undo2, CircleSlash, ChevronsDown, CircleAlert, CircleCheck } from '@lucide/svelte';
  import { pkOf, tableData, type QueryResult, type Cell, type Row, type Schema } from '../mock';
  import { canWrite, type Conn } from '../state.svelte';
  import { activeTab } from '../tabs.svelte';
  import { edits, editBlocker, pendingCount, resetEdits, setCell, addRow, parseInput, buildStatements, saveEdits } from '../edits.svelte';
  import EditSaveDialog from './EditSaveDialog.svelte';
  import { api, ApiError } from '../api';
  import { csvCell, formatCell } from '../results';

  let {
    result,
    conn,
    schema,
    loading = false,
    onloadmore,
    onsaved,
  }: { result: QueryResult | null; conn: Conn; schema: Schema; loading?: boolean; onloadmore?: () => void; onsaved: () => void } = $props();

  let sortKey = $state<string | null>(null);
  let asc = $state(true);

  /** key: `r<index>` for result rows, `a<n>` for rows added in edit mode. */
  type GridRow = { key: string; idx: number; row: Row; isNew: boolean };

  // Numeric columns sort by value, so 10 comes after 9 and big integers keep their precision.
  const NUMERIC = /^(int|integer|smallint|tinyint|mediumint|bigint|int[248]|serial|bigserial|smallserial|real|double|float|float[48]|decimal|numeric|number|money)\b/i;
  const asNumber = (v: Cell, numericCol: boolean): number | bigint | null => {
    if (typeof v === 'number') return v;
    if (typeof v !== 'string' || !numericCol) return null;
    const s = v.trim();
    if (/^-?\d+$/.test(s)) return BigInt(s);
    const n = Number(s);
    return s !== '' && Number.isFinite(n) ? n : null;
  };
  const cmpNum = (x: number | bigint, y: number | bigint) => (x < y ? -1 : x > y ? 1 : 0);

  const sorted = $derived.by(() => {
    if (!result) return [];
    const list: GridRow[] = result.rows.map((row, idx) => ({ key: 'r' + idx, idx, row, isNew: false }));
    const k = sortKey;
    if (!k) return list;
    const numericCol = NUMERIC.test(result.types?.[result.columns.indexOf(k)] ?? '');
    const text = (v: Cell) => (typeof v === 'object' ? formatCell(v!) : String(v));
    return list.sort((a, b) => {
      const x = a.row[k], y = b.row[k];
      if (x === y) return 0;
      // NULLs last in both directions.
      if (x == null) return 1;
      if (y == null) return -1;
      const nx = asNumber(x, numericCol), ny = asNumber(y, numericCol);
      const c = nx !== null && ny !== null ? cmpNum(nx, ny) : text(x).localeCompare(text(y), undefined, { numeric: true });
      return c * (asc ? 1 : -1);
    });
  });
  const rows = $derived<GridRow[]>([...sorted, ...edits.added.map((a) => ({ key: 'a' + a.key, idx: -1, row: a.row, isNew: true }))]);

  const blocker = $derived(canWrite(conn) ? editBlocker(result) : 'Read-only connection: edits are off');
  const count = $derived(pendingCount());

  // A new result (or a connection turned read-only) drops edit mode unless the result is still editable.
  $effect(() => {
    if (blocker) edits.on = false;
  });

  function sortBy(c: string) {
    if (sortKey === c) asc = !asc;
    else (sortKey = c), (asc = true);
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

  // Row keys are result indexes, so they point at other rows once the result changes.
  $effect(() => {
    result;
    selected = {};
    editing = null;
    menu = null;
  });

  const value = (r: GridRow, c: string): Cell => (!r.isNew && edits.changed[r.idx] && c in edits.changed[r.idx] ? edits.changed[r.idx][c] : r.row[c]);
  const isChanged = (r: GridRow, c: string) => !r.isNew && !!edits.changed[r.idx] && c in edits.changed[r.idx];
  const isDeleted = (r: GridRow) => !r.isNew && !!edits.deleted[r.idx];
  const sample = (c: string) => result?.rows.find((x) => x[c] !== null)?.[c] ?? null;

  function toggleEdit() {
    if (edits.on && count && !confirm(`Discard ${count} unsaved change(s)?`)) return;
    if (edits.on) resetEdits();
    else edits.on = true;
    selected = {};
    editing = null;
  }

  function write(r: GridRow, c: string, v: Cell) {
    if (r.isNew) {
      const a = edits.added.find((x) => 'a' + x.key === r.key);
      if (a) a.row[c] = v;
    } else setCell(r.idx, r.row, c, v);
  }

  function startEdit(r: GridRow, c: string) {
    if (!edits.on || isDeleted(r)) return;
    const v = value(r, c);
    draft = v === null ? '' : String(v);
    inputError = null;
    editing = { key: r.key, col: c };
  }

  function commit(r: GridRow, c: string, blur = false) {
    // Esc clears `editing` first, so the blur that follows does nothing.
    if (editing?.key !== r.key || editing.col !== c) return;
    const before = value(r, c);
    const parsed = parseInput(draft, sample(c));
    if (parsed === undefined) {
      // Enter keeps the editor open with the error; leaving the cell drops the invalid draft.
      if (!blur) return void (inputError = 'Not a number');
    } else if (!(before === null && draft === '')) write(r, c, parsed); // Leaving a NULL cell empty keeps it NULL instead of turning it into ''.
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
    const pk = pkOf(result.table!);
    if (pk.length === 1) {
      const d = tableData(schema, result.table!);
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

  function doSave() {
    if (!result) return;
    const ok = saveEdits(result, conn, schema);
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
    <div class="min-h-0 flex-1 overflow-auto">
      <table class="w-full border-separate border-spacing-0 font-mono text-[12px]">
        <thead class="sticky top-0 z-10">
          <tr>
            <th class="w-9 border-b border-line bg-surface px-2 py-0 text-right font-sans text-[10.5px] font-normal text-faint"></th>
            {#each result.columns as c, ci}
              <th class="group/th h-8 border-b border-l border-line bg-surface px-2.5 py-0 text-left font-normal" title={result.types?.[ci]}>
                <button class="flex w-full items-baseline gap-1.5 text-left whitespace-nowrap" onclick={() => sortBy(c)}>
                  <span class="font-sans text-[12px] font-semibold {sortKey === c ? 'text-accent' : 'text-ink'}">{c}</span>
                  {#if result.types?.[ci]}<span class="max-w-72 min-w-0 truncate text-[10.5px] text-faint">{result.types?.[ci]}</span>{/if}
                  <span class="ml-auto self-center">
                    {#if sortKey === c}
                      {#if asc}<ArrowUp size={12} class="text-accent" />{:else}<ArrowDown size={12} class="text-accent" />{/if}
                    {:else}<ArrowUpDown size={12} class="text-faint opacity-0 transition-opacity group-hover/th:opacity-100" />{/if}
                  </span>
                </button>
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as r, i (r.key)}
            <tr class={rowTint(r)}>
              <td
                class="border-b border-line/60 px-2 text-right text-[10.5px] tabular-nums {selected[r.key] ? 'font-medium text-ink' : 'text-faint'} {edits.on ? 'cursor-pointer' : ''}"
                onclick={(e) => pickRow(r, e)}
              >{r.isNew ? '+' : i + 1}</td>
              {#each result.columns as c}
                {@const v = value(r, c)}
                {@const isEd = editing?.key === r.key && editing.col === c}
                <td
                  class="h-[26px] max-w-72 truncate border-b border-l border-line/60 {isEd ? 'p-0' : 'px-2.5'} {isChanged(r, c) && !isDeleted(r) ? 'bg-amber-400/15 dark:bg-amber-400/12' : ''} {typeof v === 'number' ? 'text-right' : ''}"
                  ondblclick={() => startEdit(r, c)}
                  onclick={(e) => { if (edits.on && !e.metaKey && !e.ctrlKey) selected = { [r.key]: true }; }}
                  oncontextmenu={(e) => openMenu(e, r, c)}
                >
                  {#if isEd}
                    <input
                      class="block h-[25px] w-full min-w-24 bg-field px-2.5 font-mono text-[12px] outline-2 -outline-offset-2 {inputError ? 'outline-red-500' : 'outline-accent'}"
                      title={inputError}
                      aria-invalid={!!inputError}
                      oninput={() => (inputError = null)}
                      bind:value={draft}
                      use:focusInput
                      placeholder="NULL"
                      onkeydown={(e) => {
                        if (e.key === 'Enter') commit(r, c);
                        else if (e.key === 'Escape') { e.stopPropagation(); editing = null; }
                      }}
                      onblur={() => commit(r, c, true)}
                    />
                  {:else if v === null}
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
          disabled={!!blocker}
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
          <span title={result.truncated ? `First ${result.rows.length} rows; more exist` : undefined}>{rows.length}{result.truncated ? '+' : ''} rows</span>
          <span class="size-0.5 rounded-full bg-faint"></span>
          <span>{result.ms} ms</span>
        </span>
        {#if result.truncated && onloadmore}
          <span title={result.isRead ? undefined : 'Load more runs the SQL again, so it is only offered for a single read-only statement'}>
            <button class="btn btn-ghost h-7 px-2 text-[12px]" disabled={loading || !result.isRead} onclick={onloadmore}><ChevronsDown size={13} /> Load more</button>
          </span>
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
