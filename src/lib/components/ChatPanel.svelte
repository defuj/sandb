<script lang="ts">
  import { ArrowUp, Square, ShieldCheck, ShieldOff, Database, Ban, LoaderCircle, Clock, Check, X, Copy, Sparkles, History, SquarePen, Trash2, MessageCircleQuestion, TriangleAlert, ArrowLeft, ChevronRight, RotateCw, Table2, FileCode, FileSpreadsheet, Braces, ChartColumn, FileDown } from '@lucide/svelte';
  import Markdown from './Markdown.svelte';
  import ChartView from './ChartView.svelte';
  import ChatScope from './ChatScope.svelte';
  import ModelPicker from './ModelPicker.svelte';
  import { chat, send, current, sessionsByDate, openSession, deleteSession, decide, answer, rerun, scopeOf, setScope, connOf, dbOf, type Part, type ToolPart, type Source } from '../chat.svelte';
  import { api, ApiError } from '../api';
  import { replyPdf, pdfName } from '../report';
  import { ui, ai, engines, envTag, type Conn } from '../state.svelte';
  import { csvCell, formatCell } from '../results';
  import type { Row } from '../mock';

  const session = $derived(current());
  const messages = $derived(session?.messages ?? []);
  const past = $derived(sessionsByDate());
  const scope = $derived(scopeOf(session));
  /** The connections of the chat's databases, each once; deleted ones are left out. */
  const scopeConns = $derived([...new Set(scope.map((s) => connOf(s)).filter((c): c is Conn => !!c))]);
  const writesOn = $derived(scopeConns.some((c) => c.aiWrite));
  const many = $derived(scope.length > 1);
  /** At least one database of the chat still has its connection. */
  const ready = $derived(scopeConns.length > 0);
  /** "Connection / database", or the connection name alone for SQLite. */
  function sourceLabel(s: Source) {
    const c = connOf(s);
    if (!c) return 'Deleted connection';
    return c.engine === 'sqlite' ? c.name : `${c.name} / ${dbOf(s, c)}`;
  }

  let { tables = [], onEnableWrites, onInsertSql, onRunSql }: { tables?: string[]; onEnableWrites: (c: Conn) => void; onInsertSql: (sql: string, source?: Source) => void; onRunSql: (sql: string, source?: Source) => void } = $props();

  let input = $state('');
  let showHistory = $state(false);
  let list = $state<HTMLDivElement>();
  let composer = $state<HTMLTextAreaElement>();

  // The scripted demo flows talk about orders; real connections get prompts for their own tables.
  const suggestions = $derived.by(() => {
    const out = tables.includes('orders') ? ['Which order status brings the most revenue?', 'Delete cancelled orders older than 90 days'] : [];
    for (const t of tables.filter((t) => t !== 'orders').slice(0, 2)) out.push(`Show the latest 20 rows of ${t}`);
    if (tables.length) out.push('What tables are in this database, and what do they hold?');
    return out.slice(0, 4);
  });

  // ---- tool steps ----
  /** Consecutive tool steps of one reply form one timeline; `at` is the index of each part in the message. */
  type Block = { kind: 'part'; p: Exclude<Part, ToolPart>; at: number } | { kind: 'steps'; steps: { p: ToolPart; at: number }[] };
  function blocks(parts: Part[]): Block[] {
    const out: Block[] = [];
    parts.forEach((p, at) => {
      const last = out.at(-1);
      if (p.kind !== 'tool') out.push({ kind: 'part', p, at });
      else if (last?.kind === 'steps') last.steps.push({ p, at });
      else out.push({ kind: 'steps', steps: [{ p, at }] });
    });
    return out;
  }

  // Open state the user chose, per step. Without a choice: open while it runs, waits or failed, and for the last result with rows.
  let opened = $state<Record<string, boolean>>({});
  const stepKey = (mi: number, at: number) => `${session?.id}:${mi}:${at}`;
  function isOpen(parts: Part[], p: ToolPart, key: string, at: number) {
    if (p.status === 'running' || (p.status === 'awaiting' && !!chat.pending)) return true;
    if (key in opened) return opened[key];
    if (p.status === 'blocked' || p.status === 'denied') return true;
    return !!p.rows && !parts.slice(at + 1).some((x) => x.kind === 'tool' && !!x.rows);
  }

  /** Selected cell of a result table, shown in full under it. */
  let cell = $state<{ key: string; col: string; value: unknown } | null>(null);

  let copied = $state<string | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  async function copy(key: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = key;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = null), 1200);
    } catch (e) {
      console.warn('Copy failed.', e);
    }
  }
  const csv = (rows: Row[]) => {
    const cols = Object.keys(rows[0]);
    return [cols.map(csvCell).join(','), ...rows.map((r) => cols.map((c) => csvCell(r[c])).join(','))].join('\r\n');
  };
  const replyText = (parts: Part[]) => parts.flatMap((p) => (p.kind === 'text' ? [p.text] : [])).join('\n\n').trim();
  /** Short target of a step for its one-line summary. */
  const target = (p: ToolPart) => (p.name === 'describe_table' ? p.sql : p.sql?.trim().split('\n')[0]) ?? '';
  const duration = (ms?: number) => (ms === undefined ? '' : ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`);
  /** Columns whose first non-NULL value is a number; they align right, as in the grid. */
  function numeric(rows: Row[]) {
    const out: Record<string, boolean> = {};
    for (const c of Object.keys(rows[0])) out[c] = typeof rows.find((r) => r[c] !== null)?.[c] === 'number';
    return out;
  }
  /** The model's arguments; steps saved before they were kept fall back to the SQL or table name. */
  const argsOf = (p: ToolPart) => p.args ?? (p.sql === undefined ? {} : p.name === 'describe_table' ? { table: p.sql } : { sql: p.sql });
  /** A step still runs or waits for the user, so the model is not thinking. */
  const busy = (parts: Part[]) => parts.some((x) => x.kind === 'tool' && (x.status === 'running' || (x.status === 'awaiting' && !!chat.pending)));
  /** The reply waits for the model after a finished step or its chart. */
  const afterStep = (parts: Part[]) => (parts.at(-1)?.kind === 'tool' || parts.at(-1)?.kind === 'chart') && !busy(parts);

  // ---- PDF export ----
  let exporting = $state<number | null>(null);
  let exported = $state<{ mi: number; ok: boolean; text: string } | null>(null);
  async function exportPdf(mi: number) {
    const q = messages[mi - 1]?.parts[0];
    const question = q?.kind === 'text' ? q.text.trim() : '';
    exporting = mi;
    exported = null;
    try {
      const meta = [...scope.map(sourceLabel), new Date().toLocaleString()].join(' · ');
      const { bytes, title } = await replyPdf({ title: question.length > 90 ? `${question.slice(0, 87)}...` : question || 'Report', meta, parts: messages[mi].parts });
      const name = pdfName(title);
      if (api.kind === 'tauri') {
        const path = await api.savePdf(name, bytes);
        if (path) exported = { mi, ok: true, text: `Saved to ${path}` };
      } else {
        const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' })), download: name });
        a.click();
        URL.revokeObjectURL(a.href);
      }
    } catch (e) {
      exported = { mi, ok: false, text: `The PDF was not saved: ${ApiError.from(e).message}` };
    } finally {
      exporting = null;
    }
  }

  function retry(mi: number) {
    const q = messages[mi - 1];
    if (q?.role === 'user' && q.parts[0]?.kind === 'text') submit(q.parts[0].text);
  }

  // Grows with its text up to about eight lines.
  $effect(() => {
    void input;
    if (!composer) return;
    composer.style.height = 'auto';
    composer.style.height = `${Math.min(composer.scrollHeight, 168)}px`;
  });

  function keyComposer(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    } else if (e.key === 'ArrowUp' && !input && !chat.live) {
      // Up in an empty composer brings back the last question, to change and send again.
      const last = [...messages].reverse().find((m) => m.role === 'user')?.parts[0];
      if (last?.kind !== 'text') return;
      e.preventDefault();
      input = last.text;
    }
  }

  // While a question is open, the composer text is the answer to it.
  function submit(text = input) {
    if (!text.trim()) return;
    if (chat.question) {
      input = '';
      answer(text.trim());
      return;
    }
    if (chat.live || !ready) return;
    input = '';
    showHistory = false;
    send(text);
  }

  /** On: writes off on every connection of the chat. Off: the safeguard dialog for the one connection, or a choice of them. */
  let writeMenu = $state(false);
  function toggleMode() {
    if (writesOn) for (const c of scopeConns) c.aiWrite = false;
    else if (scopeConns.length === 1) onEnableWrites(scopeConns[0]);
    else writeMenu = !writeMenu;
  }

  function newChat() {
    openSession('');
    showHistory = false;
    composer?.focus();
  }

  function pick(id: string) {
    openSession(id);
    showHistory = false;
  }

  $effect(() => {
    void messages.length;
    void JSON.stringify(messages.at(-1)?.parts);
    list?.scrollTo({ top: list.scrollHeight });
  });

  // Puts focus on the safe default of an approval card, so Enter does not run a destructive statement.
  function autofocus(el: HTMLElement) {
    el.focus({ preventScroll: true });
  }

  const label: Record<string, string> = { list_tables: 'List tables', describe_table: 'Describe table', run_read_query: 'Read query', render_chart: 'Chart', propose_write: 'Proposed write' };
  const fmt = (v: unknown) => (typeof v === 'number' ? v.toLocaleString() : v !== null && typeof v === 'object' ? formatCell(v) : String(v));
  /** Full text of a cell for the cell view. */
  const full = (v: unknown) => (v !== null && typeof v === 'object' && !('$bytes' in v) ? JSON.stringify(v, null, 2) : fmt(v));
  const bold = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/\*\*(.+?)\*\*/g, '<b class="font-semibold">$1</b>');
  const destructive = (sql: string) => /^\s*(delete|drop|truncate)/i.test(sql);
  const ago = (t: number) => {
    const m = Math.round((Date.now() - t) / 60000);
    if (m < 1) return 'just now';
    if (m < 60) return `${m} min ago`;
    if (m < 1440) return `${Math.round(m / 60)} h ago`;
    return new Date(t).toLocaleDateString();
  };

  // Transcript status word and its text color.
  const statusText: Record<string, string> = { running: 'RUNNING', done: 'OK', blocked: 'BLOCKED', awaiting: 'NEEDS APPROVAL', approved: 'APPROVED', denied: 'DENIED' };
  const statusTone: Record<string, string> = {
    running: 'text-accent',
    done: 'text-emerald-700 dark:text-emerald-400',
    blocked: 'text-red-700 dark:text-red-400',
    awaiting: 'text-amber-700 dark:text-amber-400',
    approved: 'text-emerald-700 dark:text-emerald-400',
    denied: 'text-red-700 dark:text-red-400',
  };

  const placeholder = $derived(chat.question ? 'Type your own answer…' : chat.pending ? 'Approve or deny the write above…' : ready ? (many ? 'Ask across these databases…' : 'Ask about your data…') : 'Add a database to ask about…');
</script>

<section class="card absolute inset-y-2 right-2 z-30 flex max-w-[calc(100%-1rem)] flex-col overflow-hidden shadow-2xl min-[880px]:relative min-[880px]:inset-auto min-[880px]:min-w-[280px] min-[880px]:shadow-[var(--card-shadow)]" style:width="{ui.chatWidth}px">
  <header class="relative flex h-11 shrink-0 items-center gap-1 pr-2 pl-4">
    <span class="mr-1 text-[13px] font-semibold">{showHistory ? 'Chats' : 'Assistant'}</span>
    <ModelPicker variant="chip" />
    <button class="btn btn-ghost size-7 p-0 {showHistory ? 'bg-hover text-ink' : ''}" onclick={() => (showHistory = !showHistory)} disabled={!!chat.live} aria-pressed={showHistory} title={chat.live ? 'Wait for the reply to finish' : 'Chat history'} aria-label="Chat history"><History size={14} /></button>
    <button class="btn btn-ghost size-7 p-0" onclick={newChat} disabled={!!chat.live || (!session && !showHistory)} title="New chat" aria-label="New chat"><SquarePen size={14} /></button>
    <button class="btn btn-ghost size-7 p-0" onclick={() => (ui.chatOpen = false)} aria-label="Close assistant"><X size={14} /></button>
  </header>

  {#if showHistory}
    <div class="flex-1 overflow-auto px-2 pt-1 pb-3">
      <button class="nav-item mb-1 text-muted" onclick={() => (showHistory = false)}><ArrowLeft size={13} /> Back to chat</button>
      <div class="px-2 pt-2 pb-1.5 text-[11.5px] font-medium text-faint">All chats</div>
      {#each past as s (s.id)}
        <div class="group relative">
          <button class="flex w-full flex-col items-start gap-0.5 rounded-lg px-2.5 py-2 pr-9 text-left hover:bg-hover {s.id === session?.id ? 'bg-active' : ''}" onclick={() => pick(s.id)}>
            <span class="w-full truncate text-[12.5px] text-ink">{s.title}</span>
            <span class="w-full truncate text-[11.5px] text-faint" title={s.sources.map(sourceLabel).join(', ')}>{ago(s.updated)} · {Math.ceil(s.messages.length / 2)} {s.messages.length > 2 ? 'questions' : 'question'} · {s.sources.length === 1 ? sourceLabel(s.sources[0]) : `${s.sources.length} databases`}</span>
          </button>
          <button class="absolute top-1/2 right-1.5 grid size-6 -translate-y-1/2 place-items-center rounded text-faint opacity-0 group-hover:opacity-100 hover:bg-active hover:text-red-600 focus-visible:opacity-100" onclick={() => deleteSession(s.id)} title="Delete chat" aria-label="Delete chat {s.title}"><Trash2 size={13} /></button>
        </div>
      {:else}
        <p class="px-2.5 py-2 text-[12.5px] text-muted">No chats yet.</p>
      {/each}
    </div>
  {:else}
  <div bind:this={list} class="flex-1 space-y-4 overflow-auto px-4 pt-1 pb-4">
    {#if messages.length === 0}
      <div class="pt-8">
        <div class="grid size-8 place-items-center rounded-lg bg-accent/12 text-accent"><Sparkles size={15} /></div>
        <div class="mt-3 text-[14px] font-semibold">Ask about your data</div>
        <p class="mt-1 text-[12.5px] leading-relaxed text-muted">The assistant can read the schema of the databases below and run read-only queries. Add more databases to ask across them. It cannot change data unless you turn on write mode.</p>
        <div class="mt-5 space-y-1.5">
          {#each suggestions as s}
            <button class="flex w-full items-center rounded-lg border border-line px-3 py-2 text-left text-[12.5px] text-muted hover:bg-hover hover:text-ink" onclick={() => submit(s)}>
              {s}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    {#each messages as m, mi}
      {#if m.role === 'user'}
        <div class="ml-auto w-fit max-w-[85%] rounded-xl rounded-br-sm bg-active px-3 py-2 text-ink">{(m.parts[0] as { text: string }).text}</div>
      {:else}
        <div class="space-y-2.5">
          {#each blocks(m.parts) as b}
            {#if b.kind === 'part'}
            {@const p = b.p}
            {#if p.kind === 'text'}
              <Markdown text={p.text} />
            {:else if p.kind === 'chart'}
              <ChartView part={p} />
            {:else if p.kind === 'question'}
              {#if p.answer === undefined && chat.question}
                <div class="rounded-lg border border-accent/50 bg-accent/5 p-3 ring-3 ring-accent/10" role="group" aria-label="Question from the assistant">
                  <div class="flex items-start gap-2">
                    <MessageCircleQuestion size={15} class="mt-0.5 shrink-0 text-accent" />
                    <p class="leading-relaxed">{@html bold(p.text)}</p>
                  </div>
                  <div class="mt-2.5 space-y-1">
                    {#each p.options as o}
                      <button class="flex w-full items-center rounded-md border border-line bg-surface px-2.5 py-1.5 text-left hover:border-accent/60 hover:bg-hover focus-visible:border-accent" onclick={() => answer(o)}>
                        <span class="min-w-0 flex-1 truncate font-mono text-[12px]">{o}</span>
                      </button>
                    {/each}
                  </div>
                  <div class="mt-2 flex items-center gap-2 text-[11.5px] text-faint">
                    <span class="flex-1">Or type your own answer below.</span>
                    <button class="rounded px-1.5 py-0.5 font-medium text-muted hover:bg-hover hover:text-ink" onclick={() => answer(null)}>Skip</button>
                  </div>
                </div>
              {:else}
                <div class="rounded-lg border border-line px-3 py-2">
                  <div class="flex items-start gap-2 text-[12.5px] text-muted">
                    <MessageCircleQuestion size={13} class="mt-0.5 shrink-0 text-faint" />
                    <p class="leading-relaxed">{@html bold(p.text)}</p>
                  </div>
                  <div class="mt-1.5 ml-5 flex items-center gap-1.5 text-[12.5px]">
                    {#if p.answer}
                      <Check size={13} class="text-emerald-600 dark:text-emerald-400" /> <span class="font-medium text-ink">{p.answer}</span>
                    {:else}
                      <span class="text-faint">Skipped</span>
                    {/if}
                  </div>
                </div>
              {/if}
            {:else if p.kind === 'error'}
              <p class="flex items-start gap-2 leading-relaxed text-red-700 dark:text-red-400"><TriangleAlert size={13} class="mt-1 shrink-0" /><span class="min-w-0 break-words">{p.text}</span></p>
            {/if}
            {:else}
              {@const running = b.steps.some((s) => s.p.status === 'running')}
              {@const asking = b.steps.some((s) => s.p.status === 'awaiting') && !!chat.pending}
              {@const danger = asking && b.steps.some((s) => s.p.status === 'awaiting' && !!s.p.sql && destructive(s.p.sql))}
              {@const total = b.steps.reduce((n, s) => n + (s.p.ms ?? 0), 0)}
              <div class="overflow-hidden rounded-lg border {asking ? (danger ? 'border-red-500/50 ring-3 ring-red-500/10' : 'border-amber-500/50 ring-3 ring-amber-500/10') : 'border-line'}">
                <div class="flex h-7 items-center gap-2 bg-sunken px-2.5 text-[11.5px] text-faint">
                  {#if running}
                    <LoaderCircle size={12} class="animate-spin text-accent" /> <span>Working · step {b.steps.length}</span>
                  {:else}
                    <span>{b.steps.length} {b.steps.length === 1 ? 'step' : 'steps'}</span>
                  {/if}
                  <span class="ml-auto tabular-nums">{total ? duration(total) : ''}</span>
                </div>
                <ol>
                  {#each b.steps as { p, at }, si}
                    {@const key = stepKey(mi, at)}
                    {@const open = isOpen(m.parts, p, key, at)}
                    {@const waiting = p.status === 'awaiting' && !!chat.pending}
                    {@const risky = waiting && !!p.sql && destructive(p.sql)}
                    {@const failed = p.status === 'blocked' || p.status === 'denied'}
                    {@const sc = (p.source && connOf(p.source)) || scopeConns[0]}
                    <li class="border-t border-line">
                      <button class="flex h-8 w-full items-center gap-2 px-2.5 text-left text-[12px] hover:bg-hover" aria-expanded={open} onclick={() => (opened[key] = !open)}>
                        <span class="w-3.5 shrink-0 text-center text-[10.5px] text-faint tabular-nums">{si + 1}</span>
                        {#if p.status === 'running'}<LoaderCircle size={13} class="shrink-0 animate-spin text-accent" />
                        {:else if failed}<Ban size={13} class="shrink-0 text-red-600 dark:text-red-400" />
                        {:else if p.status === 'awaiting'}<Clock size={13} class="shrink-0 text-amber-600 dark:text-amber-400" />
                        {:else if p.name === 'describe_table' || p.name === 'list_tables'}<Database size={13} class="shrink-0 text-faint" />
                        {:else if p.name === 'render_chart'}<ChartColumn size={13} class="shrink-0 text-emerald-600 dark:text-emerald-400" />
                        {:else}<Check size={13} class="shrink-0 text-emerald-600 dark:text-emerald-400" />{/if}
                        <span class="shrink-0 font-medium text-ink">{label[p.name] ?? p.name}</span>
                        {#if many && p.source}<span class="max-w-24 shrink-0 truncate rounded bg-sunken px-1 font-mono text-[10.5px] text-muted" title={sourceLabel(p.source)}>{dbOf(p.source)}</span>{/if}
                        <span class="min-w-0 flex-1 truncate font-mono text-[11.5px] text-muted">{open && p.name !== 'describe_table' ? '' : target(p)}</span>
                        <span class="shrink-0 text-[11px] text-faint tabular-nums">{duration(p.ms)}</span>
                        {#if p.status !== 'done'}<span class="shrink-0 text-[11.5px] {statusTone[p.status]}"><span class="sentence">{statusText[p.status] ?? p.status}</span></span>{/if}
                        <ChevronRight size={13} class="shrink-0 text-faint transition-transform {open ? 'rotate-90' : ''}" />
                      </button>
                      {#if open}
                        {@const sqlActions = !!p.sql && p.name !== 'describe_table' && !waiting}
                        {#if p.sql && p.name !== 'describe_table'}
                          <pre class="overflow-x-auto border-t border-line bg-sunken px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">{p.sql}</pre>
                        {/if}
                        {#if p.rows?.length}
                          {@const cols = Object.keys(p.rows[0])}
                          {@const nums = numeric(p.rows)}
                          <div class="max-h-80 overflow-auto border-t border-line">
                            <table class="w-full text-[12px]">
                              <thead class="sticky top-0 bg-surface"><tr>{#each cols as c}<th class="px-3 pt-2 pb-1 text-[11.5px] font-medium whitespace-nowrap text-faint {nums[c] ? 'text-right' : 'text-left'}">{c}</th>{/each}</tr></thead>
                              <tbody class="font-mono text-[11.5px]">
                                {#each p.rows as r, ri}
                                  <tr class="border-t border-line/60">
                                    {#each cols as c}
                                      {@const v = r[c]}
                                      {@const sel = cell?.key === `${key}:${ri}` && cell.col === c}
                                      <td class="max-w-48 p-0">
                                        <button class="block w-full truncate px-3 py-1 whitespace-nowrap tabular-nums {nums[c] ? 'text-right' : 'text-left'} {sel ? 'bg-accent/10' : 'hover:bg-hover'}" title="Show the full value" onclick={() => (cell = sel ? null : { key: `${key}:${ri}`, col: c, value: v })}>
                                          {#if v === null}<span class="text-[11px] text-faint italic">NULL</span>{:else}{fmt(v)}{/if}
                                        </button>
                                      </td>
                                    {/each}
                                  </tr>
                                {/each}
                              </tbody>
                            </table>
                          </div>
                          {#if cell?.key.startsWith(`${key}:`)}
                            {@const c = cell}
                            <div class="border-t border-line bg-sunken px-3 py-2 text-[12px]">
                              <div class="flex items-center gap-2 text-[11.5px] text-faint">
                                <span class="min-w-0 truncate font-mono text-muted">{c.col}</span>
                                <span>row {Number(c.key.slice(key.length + 1)) + 1}</span>
                                <button class="ml-auto grid size-6 place-items-center rounded hover:bg-hover hover:text-ink" title="Copy value" aria-label="Copy value" onclick={() => copy('cell', c.value === null ? '' : full(c.value))}>{#if copied === 'cell'}<Check size={12} />{:else}<Copy size={12} />{/if}</button>
                                <button class="grid size-6 place-items-center rounded hover:bg-hover hover:text-ink" title="Close" aria-label="Close value" onclick={() => (cell = null)}><X size={12} /></button>
                              </div>
                              <pre class="mt-1 max-h-40 overflow-auto font-mono text-[11.5px] break-all whitespace-pre-wrap text-ink">{#if c.value === null}<span class="text-faint italic">NULL</span>{:else}{full(c.value)}{/if}</pre>
                            </div>
                          {/if}
                          {#if p.truncated}
                            <div class="border-t border-line px-3 py-1.5 text-[12px] text-amber-700 dark:text-amber-400">More rows exist. The chat shows the first {p.rows.length}.{p.name === 'run_read_query' ? ' Open the query in the grid to see all rows.' : ''}</div>
                          {/if}
                        {/if}
                        {#if waiting && p.sql && sc}
                          <div class="space-y-2.5 border-t border-line px-3 py-2.5">
                            <div class="flex items-center gap-2 text-[12px]">
                              <span class="font-mono text-[10.5px] font-semibold {engines[sc.engine].badge}">{engines[sc.engine].short}</span>
                              <span class="min-w-0 truncate font-medium">{p.source ? sourceLabel(p.source) : sc.name}</span>
                              <span class="tag {envTag[sc.env]}">{sc.env}</span>
                              <span class="tag font-mono text-amber-700 dark:text-amber-400">{p.sql.trim().split(/\s+/)[0].toUpperCase()}</span>
                              <span class="ml-auto shrink-0 text-faint tabular-nums">≈ 214 rows</span>
                            </div>
                            {#if risky}
                              <div class="flex gap-2 rounded-md bg-red-500/8 px-2.5 py-2 text-[12px] leading-relaxed text-red-700 dark:text-red-300">
                                <TriangleAlert size={13} class="mt-0.5 shrink-0" />
                                <span>This statement removes data. It runs inside a transaction, but you cannot undo it after it commits.</span>
                              </div>
                            {:else}
                              <p class="text-[12px] text-muted">It runs only if you approve it.</p>
                            {/if}
                            <div class="flex items-center gap-1.5">
                              <button class="flex items-center gap-1 rounded px-1 text-[12px] font-medium text-accent hover:underline" onclick={() => onInsertSql(p.sql!, p.source)}><Copy size={12} /> To editor</button>
                              {#if risky}
                                <button class="btn btn-secondary ml-auto h-7 px-2.5 text-[12px]" use:autofocus onclick={() => decide(false)}>Deny</button>
                                <button class="btn h-7 bg-red-600 px-2.5 text-[12px] text-white hover:bg-red-500" onclick={() => decide(true)}>Approve and run</button>
                              {:else}
                                <button class="btn btn-secondary ml-auto h-7 px-2.5 text-[12px]" onclick={() => decide(false)}>Deny</button>
                                <button class="btn btn-primary h-7 px-2.5 text-[12px]" use:autofocus onclick={() => decide(true)}>Approve and run</button>
                              {/if}
                            </div>
                          </div>
                        {/if}
                        {#if p.note || sqlActions}
                          <div class="flex items-center gap-2 border-t border-line px-3 py-1 text-[12px] text-muted">
                            <span class="min-w-0 flex-1 {failed ? 'text-red-700 dark:text-red-400' : ''}">{p.note ?? ''}</span>
                            {#if sqlActions}
                              <span class="flex shrink-0 items-center">
                                <button class="grid size-6 place-items-center rounded text-faint hover:bg-hover hover:text-ink" title="Copy SQL" aria-label="Copy SQL" onclick={() => copy(`${key}:sql`, p.sql!)}>{#if copied === `${key}:sql`}<Check size={13} />{:else}<Copy size={13} />{/if}</button>
                                <button class="grid size-6 place-items-center rounded text-faint hover:bg-hover hover:text-ink" title="Put the SQL in the editor" aria-label="Put the SQL in the editor" onclick={() => onInsertSql(p.sql!, p.source)}><FileCode size={13} /></button>
                                {#if p.name === 'run_read_query' && p.status === 'done'}
                                  <button class="grid size-6 place-items-center rounded text-faint hover:bg-hover hover:text-ink" title="Open in the results grid (new tab)" aria-label="Open in the results grid" onclick={() => onRunSql(p.sql!, p.source)}><Table2 size={13} /></button>
                                {/if}
                                {#if p.name === 'run_read_query' && p.status !== 'blocked'}
                                  <button class="grid size-6 place-items-center rounded text-faint hover:bg-hover hover:text-ink disabled:opacity-40" title="Run again" aria-label="Run again" disabled={p.status === 'running'} onclick={() => rerun(p)}><RotateCw size={13} /></button>
                                {/if}
                                {#if p.rows?.length}
                                  <button class="grid size-6 place-items-center rounded text-faint hover:bg-hover hover:text-ink" title="Copy rows as CSV" aria-label="Copy rows as CSV" onclick={() => copy(`${key}:csv`, csv(p.rows!))}>{#if copied === `${key}:csv`}<Check size={13} />{:else}<FileSpreadsheet size={13} />{/if}</button>
                                {/if}
                              </span>
                            {/if}
                          </div>
                        {/if}
                        <details class="group border-t border-line text-[11.5px]">
                          <summary class="flex cursor-pointer list-none items-center gap-1.5 px-3 py-1.5 text-faint select-none hover:text-ink [&::-webkit-details-marker]:hidden">
                            <Braces size={12} /> Call details <ChevronRight size={12} class="transition-transform group-open:rotate-90" />
                          </summary>
                          <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 px-3 pb-1.5 font-mono">
                            <dt class="text-faint">tool</dt><dd class="min-w-0 truncate">{p.name}</dd>
                            {#if p.id}<dt class="text-faint">call id</dt><dd class="min-w-0 truncate">{p.id}</dd>{/if}
                            {#if p.source}<dt class="text-faint">database</dt><dd class="min-w-0 truncate">{sourceLabel(p.source)}</dd>{/if}
                            <dt class="text-faint">status</dt><dd>{p.status}</dd>
                            {#if p.ms !== undefined}<dt class="text-faint">duration</dt><dd>{duration(p.ms)}</dd>{/if}
                            {#if p.rows}<dt class="text-faint">rows shown</dt><dd>{p.rows.length}{p.truncated ? ' (more exist)' : ''}</dd>{/if}
                            {#if p.note}<dt class="text-faint">result</dt><dd class="min-w-0 break-words">{p.note}</dd>{/if}
                            {#if p.error}<dt class="text-faint">error</dt><dd class="text-red-700 dark:text-red-400">{p.error}</dd>{/if}
                          </dl>
                          <div class="px-3 pb-2">
                            <div class="font-mono text-faint">arguments</div>
                            <pre class="mt-0.5 max-h-48 overflow-auto rounded bg-sunken px-2 py-1.5 font-mono break-all whitespace-pre-wrap text-muted">{JSON.stringify(argsOf(p), null, 2)}</pre>
                          </div>
                        </details>
                      {/if}
                    </li>
                  {/each}
                </ol>
              </div>
            {/if}
          {/each}
          {#if chat.live && m === messages.at(-1)}
            <!-- Steps and questions show their own state; otherwise the reply waits for the model or streams text. -->
            {#if m.parts.length === 0 || afterStep(m.parts)}
              <div class="flex items-center gap-2 text-[12.5px] text-muted"><LoaderCircle size={13} class="animate-spin text-accent" /> Thinking…</div>
            {:else if m.parts.at(-1)?.kind === 'text'}
              <div class="flex items-center gap-2 text-[12.5px] text-muted"><LoaderCircle size={13} class="animate-spin text-accent" /> Writing…</div>
            {/if}
          {:else}
            {@const text = replyText(m.parts)}
            {@const printable = !!text || m.parts.some((x) => x.kind === 'chart')}
            <div class="-ml-1 flex items-center text-faint">
              {#if text}
                <button class="grid size-6 place-items-center rounded hover:bg-hover hover:text-ink" title="Copy reply" aria-label="Copy reply" onclick={() => copy(`${mi}:reply`, text)}>{#if copied === `${mi}:reply`}<Check size={13} />{:else}<Copy size={13} />{/if}</button>
              {/if}
              {#if printable}
                <button class="flex h-6 items-center gap-1 rounded px-1.5 text-[12px] font-medium hover:bg-hover hover:text-ink disabled:opacity-40" title="Export the reply as a PDF" aria-label="Export as PDF" disabled={exporting !== null} onclick={() => exportPdf(mi)}>{#if exporting === mi}<LoaderCircle size={13} class="animate-spin" />{:else}<FileDown size={13} />{/if} Export PDF</button>
              {/if}
              <button class="grid size-6 place-items-center rounded hover:bg-hover hover:text-ink disabled:opacity-40" title="Ask again" aria-label="Ask again" disabled={!!chat.live} onclick={() => retry(mi)}><RotateCw size={13} /></button>
            </div>
            {#if exported?.mi === mi}
              <p class="text-[12px] break-all {exported.ok ? 'text-muted' : 'text-red-700 dark:text-red-400'}">{exported.text}</p>
            {/if}
          {/if}
        </div>
      {/if}
    {/each}
  </div>
  {/if}

  <div class="mx-3 mb-3 rounded-xl border border-line bg-field p-2 transition-[border-color,box-shadow] focus-within:border-accent/70 focus-within:ring-3 focus-within:ring-accent/15">
    <ChatScope sources={scope} disabled={!!chat.live} onchange={setScope} />
    <textarea
      rows="2"
      bind:this={composer}
      bind:value={input}
      {placeholder}
      class="block max-h-42 w-full resize-none bg-transparent px-1 py-0.5 text-[13px] outline-none placeholder:text-faint"
      onkeydown={keyComposer}
      autocapitalize="off"
      spellcheck="false"
    ></textarea>
    <div class="relative mt-1 flex items-center gap-2">
      {#if writesOn}
        <button class="flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[12px] font-medium text-amber-700 hover:bg-hover dark:text-amber-400" onclick={toggleMode} title="Writes need approval on {scopeConns.filter((c) => c.aiWrite).map((c) => c.name).join(', ')}. Click to switch back to read-only.">
          <ShieldOff size={13} /> Writes need approval
        </button>
      {:else}
        <button class="flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[12px] font-medium text-muted hover:bg-hover hover:text-ink disabled:opacity-40" onclick={toggleMode} disabled={!ready} aria-expanded={scopeConns.length > 1 ? writeMenu : undefined} title="Allow the assistant to propose writes">
          <ShieldCheck size={13} class="text-emerald-600 dark:text-emerald-400" /> Read-only
        </button>
      {/if}
      {#if writeMenu && !writesOn}
        <div class="popover absolute bottom-full left-0 z-50 mb-1 w-60 p-1">
          <div class="label px-2 pt-1.5 pb-1">Allow writes on</div>
          {#each scopeConns as c (c.id)}
            <button class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] hover:bg-hover" onclick={() => { writeMenu = false; onEnableWrites(c); }}>
              <span class="w-5 shrink-0 font-mono text-[10.5px] font-semibold {engines[c.engine].badge}">{engines[c.engine].short}</span>
              <span class="min-w-0 flex-1 truncate text-ink">{c.name}</span>
              <span class="tag {envTag[c.env]}">{c.env}</span>
            </button>
          {/each}
        </div>
        <button class="fixed inset-0 z-40 cursor-default" aria-label="Close" onclick={() => (writeMenu = false)}></button>
      {/if}
      <span class="min-w-0 flex-1 truncate text-right text-[11px] text-faint" aria-hidden="true">↵ send · ⇧↵ new line{messages.length ? ' · ↑ last question' : ''}</span>
      {#if chat.live && chat.cancel && !chat.question}
        <button class="btn btn-primary ml-auto size-7 rounded-lg p-0" onclick={() => chat.cancel?.()} aria-label="Stop" title="Stop the reply">
          <Square size={11} class="fill-current" />
        </button>
      {:else}
        <button class="btn btn-primary ml-auto size-7 rounded-lg p-0" onclick={() => submit()} disabled={chat.question ? !input.trim() : !!chat.live || !input.trim() || !ready} aria-label={chat.question ? 'Send answer' : 'Send'}>
          <ArrowUp size={15} />
        </button>
      {/if}
    </div>
  </div>
</section>
