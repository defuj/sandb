<script lang="ts">
  import { ArrowUp, Square, ShieldCheck, ShieldOff, Database, Ban, LoaderCircle, Clock, Check, X, Copy, Sparkles, History, SquarePen, Trash2, MessageCircleQuestion, TriangleAlert, ArrowLeft } from '@lucide/svelte';
  import { chat, send, current, sessionsOf, openSession, deleteSession, decide, answer } from '../chat.svelte';
  import { ui, ai, active, engines, envTag } from '../state.svelte';

  const conn = $derived(active());
  const session = $derived(current(conn.id));
  const messages = $derived(session?.messages ?? []);
  const past = $derived(sessionsOf(conn.id));

  let { onEnableWrites, onInsertSql }: { onEnableWrites: () => void; onInsertSql: (sql: string) => void } = $props();

  let input = $state('');
  let showHistory = $state(false);
  let list = $state<HTMLDivElement>();
  let composer = $state<HTMLTextAreaElement>();

  const suggestions = ['Which order status brings the most revenue?', 'Delete cancelled orders older than 90 days'];

  // While a question is open, the composer text is the answer to it.
  function submit(text = input) {
    if (!text.trim()) return;
    if (chat.question) {
      input = '';
      answer(text.trim());
      return;
    }
    if (chat.live) return;
    input = '';
    showHistory = false;
    send(text);
  }

  function toggleMode() {
    if (conn.aiWrite) conn.aiWrite = false;
    else onEnableWrites();
  }

  function newChat() {
    openSession(conn.id, '');
    showHistory = false;
    composer?.focus();
  }

  function pick(id: string) {
    openSession(conn.id, id);
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

  const label: Record<string, string> = { list_tables: 'List tables', describe_table: 'Describe table', run_read_query: 'Read query', propose_write: 'Proposed write' };
  const fmt = (v: unknown) => (typeof v === 'number' ? v.toLocaleString() : v !== null && typeof v === 'object' ? JSON.stringify(v) : String(v));
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

  const placeholder = $derived(chat.question ? 'Type your own answer…' : chat.pending ? 'Approve or deny the write above…' : 'Ask about your data…');
</script>

<section class="card absolute inset-y-2 right-2 z-30 flex max-w-[calc(100%-1rem)] flex-col overflow-hidden shadow-2xl min-[880px]:static min-[880px]:shrink-0 min-[880px]:shadow-[var(--card-shadow)]" style:width="{ui.chatWidth}px">
  <header class="flex h-11 shrink-0 items-center gap-1 pr-2 pl-4">
    <span class="mr-1 text-[13px] font-semibold">{showHistory ? 'Chats' : 'Assistant'}</span>
    <button class="ml-auto max-w-40 truncate rounded-md px-1.5 py-0.5 font-mono text-[11px] text-faint hover:bg-hover hover:text-ink" title="Change model in Settings" onclick={() => (ui.page = 'settings')}>{ai.model || 'no model'}</button>
    <button class="btn btn-ghost size-7 p-0 {showHistory ? 'bg-hover text-ink' : ''}" onclick={() => (showHistory = !showHistory)} disabled={!!chat.live} aria-pressed={showHistory} title={chat.live ? 'Wait for the reply to finish' : 'Chat history'} aria-label="Chat history"><History size={14} /></button>
    <button class="btn btn-ghost size-7 p-0" onclick={newChat} disabled={!!chat.live || (!session && !showHistory)} title="New chat" aria-label="New chat"><SquarePen size={14} /></button>
    <button class="btn btn-ghost size-7 p-0" onclick={() => (ui.chatOpen = false)} aria-label="Close assistant"><X size={14} /></button>
  </header>

  {#if showHistory}
    <div class="flex-1 overflow-auto px-2 pt-1 pb-3">
      <button class="nav-item mb-1 text-muted" onclick={() => (showHistory = false)}><ArrowLeft size={13} /> Back to chat</button>
      <div class="px-2 pt-2 pb-1.5 text-[11.5px] font-medium text-faint">On {conn.name || 'this connection'}</div>
      {#each past as s (s.id)}
        <div class="group relative">
          <button class="flex w-full flex-col items-start gap-0.5 rounded-lg px-2.5 py-2 pr-9 text-left hover:bg-hover {s.id === session?.id ? 'bg-active' : ''}" onclick={() => pick(s.id)}>
            <span class="w-full truncate text-[12.5px] text-ink">{s.title}</span>
            <span class="text-[11.5px] text-faint">{ago(s.updated)} · {Math.ceil(s.messages.length / 2)} {s.messages.length > 2 ? 'questions' : 'question'}</span>
          </button>
          <button class="absolute top-1/2 right-1.5 grid size-6 -translate-y-1/2 place-items-center rounded text-faint opacity-0 group-hover:opacity-100 hover:bg-active hover:text-red-600 focus-visible:opacity-100" onclick={() => deleteSession(s.id)} title="Delete chat" aria-label="Delete chat {s.title}"><Trash2 size={13} /></button>
        </div>
      {:else}
        <p class="px-2.5 py-2 text-[12.5px] text-muted">No chats on this connection yet.</p>
      {/each}
    </div>
  {:else}
  <div bind:this={list} class="flex-1 space-y-4 overflow-auto px-4 pt-1 pb-4">
    {#if messages.length === 0}
      <div class="pt-8">
        <div class="grid size-8 place-items-center rounded-lg bg-accent/12 text-accent"><Sparkles size={15} /></div>
        <div class="mt-3 text-[14px] font-semibold">Ask about your data</div>
        <p class="mt-1 text-[12.5px] leading-relaxed text-muted">The assistant can read your schema and run read-only queries. It cannot change data unless you turn on write mode.</p>
        <div class="mt-5 space-y-1.5">
          {#each suggestions as s}
            <button class="flex w-full items-center rounded-lg border border-line px-3 py-2 text-left text-[12.5px] text-muted hover:bg-hover hover:text-ink" onclick={() => submit(s)}>
              {s}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    {#each messages as m}
      {#if m.role === 'user'}
        <div class="ml-auto w-fit max-w-[85%] rounded-xl rounded-br-sm bg-active px-3 py-2 text-ink">{(m.parts[0] as { text: string }).text}</div>
      {:else}
        <div class="space-y-2.5">
          {#each m.parts as p}
            {#if p.kind === 'text'}
              <p class="leading-relaxed break-words whitespace-pre-wrap">{@html bold(p.text)}</p>
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
            {:else}
              {@const asking = p.status === 'awaiting' && !!chat.pending}
              {@const danger = asking && !!p.sql && destructive(p.sql)}
              <div class="overflow-hidden rounded-lg border {asking ? (danger ? 'border-red-500/50 ring-3 ring-red-500/10' : 'border-amber-500/50 ring-3 ring-amber-500/10') : 'border-line'}">
                <div class="flex h-8 items-center gap-2 px-2.5 text-[12px]">
                  {#if p.status === 'running'}<LoaderCircle size={13} class="animate-spin text-accent" />
                  {:else if p.status === 'blocked' || p.status === 'denied'}<Ban size={13} class="text-red-600 dark:text-red-400" />
                  {:else if p.status === 'awaiting'}<Clock size={13} class="text-amber-600 dark:text-amber-400" />
                  {:else if p.name === 'describe_table' || p.name === 'list_tables'}<Database size={13} class="text-faint" />
                  {:else}<Check size={13} class="text-emerald-600 dark:text-emerald-400" />{/if}
                  <span class="font-medium text-ink">{label[p.name] ?? p.name}</span>
                  {#if p.name === 'describe_table'}<span class="min-w-0 truncate font-mono text-[11.5px] text-muted">{p.sql}</span>{/if}
                  <span class="ml-auto shrink-0 text-[11.5px] {statusTone[p.status]}">{#if p.status === 'done'}{statusText.done}{:else}<span class="sentence">{statusText[p.status] ?? p.status}</span>{/if}</span>
                </div>
                {#if p.sql && p.name !== 'describe_table'}
                  <pre class="overflow-x-auto border-t border-line bg-sunken px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">{p.sql}</pre>
                {/if}
                {#if p.rows}
                  <div class="overflow-x-auto border-t border-line">
                    <table class="w-full text-[12px]">
                      <thead><tr>{#each Object.keys(p.rows[0]) as c}<th class="px-3 pt-2 pb-1 text-left text-[11.5px] font-medium whitespace-nowrap text-faint">{c}</th>{/each}</tr></thead>
                      <tbody class="font-mono text-[11.5px]">{#each p.rows as r}<tr class="border-t border-line/60">{#each Object.values(r) as v}<td class="max-w-48 truncate px-3 py-1 whitespace-nowrap tabular-nums">{fmt(v)}</td>{/each}</tr>{/each}</tbody>
                    </table>
                  </div>
                {/if}
                {#if asking && p.sql}
                  <div class="space-y-2.5 border-t border-line px-3 py-2.5">
                    <div class="flex items-center gap-2 text-[12px]">
                      <span class="font-mono text-[10.5px] font-semibold {engines[conn.engine].badge}">{engines[conn.engine].short}</span>
                      <span class="min-w-0 truncate font-medium">{conn.name}</span>
                      <span class="tag {envTag[conn.env]}">{conn.env}</span>
                      <span class="tag font-mono text-amber-700 dark:text-amber-400">{p.sql.trim().split(/\s+/)[0].toUpperCase()}</span>
                      <span class="ml-auto shrink-0 text-faint tabular-nums">≈ 214 rows</span>
                    </div>
                    {#if danger}
                      <div class="flex gap-2 rounded-md bg-red-500/8 px-2.5 py-2 text-[12px] leading-relaxed text-red-700 dark:text-red-300">
                        <TriangleAlert size={13} class="mt-0.5 shrink-0" />
                        <span>This statement removes data. It runs inside a transaction, but you cannot undo it after it commits.</span>
                      </div>
                    {:else}
                      <p class="text-[12px] text-muted">It runs only if you approve it.</p>
                    {/if}
                    <div class="flex items-center gap-1.5">
                      <button class="flex items-center gap-1 rounded px-1 text-[12px] font-medium text-accent hover:underline" onclick={() => onInsertSql(p.sql!)}><Copy size={12} /> To editor</button>
                      {#if danger}
                        <button class="btn btn-secondary ml-auto h-7 px-2.5 text-[12px]" use:autofocus onclick={() => decide(false)}>Deny</button>
                        <button class="btn h-7 bg-red-600 px-2.5 text-[12px] text-white hover:bg-red-500" onclick={() => decide(true)}>Approve and run</button>
                      {:else}
                        <button class="btn btn-secondary ml-auto h-7 px-2.5 text-[12px]" onclick={() => decide(false)}>Deny</button>
                        <button class="btn btn-primary h-7 px-2.5 text-[12px]" use:autofocus onclick={() => decide(true)}>Approve and run</button>
                      {/if}
                    </div>
                  </div>
                {/if}
                {#if p.note}
                  <div class="flex items-center gap-2 border-t border-line px-3 py-1.5 text-[12px] text-muted">
                    <span class="flex-1">{p.note}</span>
                    {#if (p.status === 'blocked' || (p.status === 'awaiting' && !chat.pending)) && p.sql}
                      <button class="flex items-center gap-1 text-[12px] font-medium text-accent hover:underline" onclick={() => onInsertSql(p.sql!)}><Copy size={12} /> To editor</button>
                    {/if}
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
          {#if chat.live && m === messages.at(-1) && m.parts.length === 0}
            <div class="flex items-center gap-2 text-[12.5px] text-muted"><LoaderCircle size={13} class="animate-spin text-accent" /> Thinking…</div>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
  {/if}

  <div class="mx-3 mb-3 rounded-xl border border-line bg-field p-2 transition-[border-color,box-shadow] focus-within:border-accent/70 focus-within:ring-3 focus-within:ring-accent/15">
    <textarea
      rows="2"
      bind:this={composer}
      bind:value={input}
      {placeholder}
      class="block w-full resize-none bg-transparent px-1 py-0.5 text-[13px] outline-none placeholder:text-faint"
      onkeydown={(e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
          e.preventDefault();
          submit();
        }
      }}
    ></textarea>
    <div class="mt-1 flex items-center gap-2">
      {#if conn.aiWrite}
        <button class="flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[12px] font-medium text-amber-700 hover:bg-hover dark:text-amber-400" onclick={toggleMode} title="Switch back to read-only">
          <ShieldOff size={13} /> Writes need approval
        </button>
      {:else}
        <button class="flex h-6 items-center gap-1.5 rounded-md px-1.5 text-[12px] font-medium text-muted hover:bg-hover hover:text-ink" onclick={toggleMode} title="Allow the assistant to propose writes">
          <ShieldCheck size={13} class="text-emerald-600 dark:text-emerald-400" /> Read-only
        </button>
      {/if}
      {#if chat.live && chat.cancel && !chat.question}
        <button class="btn btn-primary ml-auto size-7 rounded-lg p-0" onclick={() => chat.cancel?.()} aria-label="Stop" title="Stop the reply">
          <Square size={11} class="fill-current" />
        </button>
      {:else}
        <button class="btn btn-primary ml-auto size-7 rounded-lg p-0" onclick={() => submit()} disabled={(!!chat.live && !chat.question) || !input.trim()} aria-label={chat.question ? 'Send answer' : 'Send'}>
          <ArrowUp size={15} />
        </button>
      {/if}
    </div>
  </div>
</section>
