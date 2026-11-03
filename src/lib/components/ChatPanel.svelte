<script lang="ts">
  import { ArrowUp, ShieldCheck, ShieldOff, Database, Ban, LoaderCircle, Clock, Check, X, Copy, Sparkles } from '@lucide/svelte';
  import { chat, send } from '../chat.svelte';
  import { ui, ai, active } from '../state.svelte';

  const conn = $derived(active());

  let { onEnableWrites, onInsertSql }: { onEnableWrites: () => void; onInsertSql: (sql: string) => void } = $props();

  let input = $state('');
  let list: HTMLDivElement;

  const suggestions = ['Which order status brings the most revenue?', 'Delete cancelled orders older than 90 days'];

  function submit(text = input) {
    if (!text.trim()) return;
    input = '';
    send(text);
  }

  function toggleMode() {
    if (conn.aiWrite) conn.aiWrite = false;
    else onEnableWrites();
  }

  $effect(() => {
    void chat.messages.length;
    void JSON.stringify(chat.messages.at(-1)?.parts);
    list?.scrollTo({ top: list.scrollHeight });
  });

  const label: Record<string, string> = { describe_table: 'Describe table', run_read_query: 'Read query', propose_write: 'Proposed write' };
  const fmt = (v: unknown) => (typeof v === 'number' ? v.toLocaleString() : String(v));
  const bold = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/\*\*(.+?)\*\*/g, '<b class="font-semibold">$1</b>');

  // Transcript status word and its text color.
  const statusText: Record<string, string> = { running: 'RUNNING', done: 'OK', blocked: 'BLOCKED', awaiting: 'AWAITING', approved: 'APPROVED', denied: 'DENIED' };
  const statusTone: Record<string, string> = {
    running: 'text-accent',
    done: 'text-emerald-700 dark:text-emerald-400',
    blocked: 'text-red-700 dark:text-red-400',
    awaiting: 'text-amber-700 dark:text-amber-400',
    approved: 'text-emerald-700 dark:text-emerald-400',
    denied: 'text-red-700 dark:text-red-400',
  };
</script>

<section class="card absolute inset-y-2 right-2 z-30 flex w-[340px] flex-col overflow-hidden shadow-2xl min-[880px]:static min-[880px]:w-[340px] min-[880px]:shrink-0 min-[880px]:shadow-[var(--card-shadow)]">
  <header class="flex h-11 shrink-0 items-center gap-2 pr-2 pl-4">
    <span class="text-[13px] font-semibold">Assistant</span>
    <button class="ml-auto max-w-40 truncate rounded-md px-1.5 py-0.5 font-mono text-[11px] text-faint hover:bg-hover hover:text-ink" title="Change model in Settings" onclick={() => (ui.page = 'settings')}>{ai.model || 'no model'}</button>
    <button class="btn btn-ghost size-7 p-0" onclick={() => (ui.chatOpen = false)} aria-label="Close assistant"><X size={14} /></button>
  </header>

  <div bind:this={list} class="flex-1 space-y-4 overflow-auto px-4 pt-1 pb-4">
    {#if chat.messages.length === 0}
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

    {#each chat.messages as m}
      {#if m.role === 'user'}
        <div class="ml-auto w-fit max-w-[85%] rounded-xl rounded-br-sm bg-active px-3 py-2 text-ink">{(m.parts[0] as { text: string }).text}</div>
      {:else}
        <div class="space-y-2.5">
          {#each m.parts as p}
            {#if p.kind === 'text'}
              <p class="leading-relaxed">{@html bold(p.text)}</p>
            {:else}
              <div class="overflow-hidden rounded-lg border border-line">
                <div class="flex h-8 items-center gap-2 px-2.5 text-[12px]">
                  {#if p.status === 'running'}<LoaderCircle size={13} class="animate-spin text-accent" />
                  {:else if p.status === 'blocked' || p.status === 'denied'}<Ban size={13} class="text-red-600 dark:text-red-400" />
                  {:else if p.status === 'awaiting'}<Clock size={13} class="text-amber-600 dark:text-amber-400" />
                  {:else if p.name === 'describe_table'}<Database size={13} class="text-faint" />
                  {:else}<Check size={13} class="text-emerald-600 dark:text-emerald-400" />{/if}
                  <span class="font-medium text-ink">{label[p.name] ?? p.name}</span>
                  {#if p.name === 'describe_table'}<span class="min-w-0 truncate font-mono text-[11.5px] text-muted">{p.sql}</span>{/if}
                  <span class="ml-auto shrink-0 text-[11.5px] {statusTone[p.status]}">{#if p.status === 'done'}{statusText.done}{:else}<span class="sentence">{statusText[p.status] ?? p.status}</span>{/if}</span>
                </div>
                {#if p.sql && p.name !== 'describe_table'}
                  <pre class="overflow-x-auto border-t border-line bg-sunken px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-muted">{p.sql}</pre>
                {/if}
                {#if p.rows}
                  <table class="w-full border-t border-line text-[12px]">
                    <thead><tr>{#each Object.keys(p.rows[0]) as c}<th class="px-3 pt-2 pb-1 text-left text-[11.5px] font-medium text-faint">{c}</th>{/each}</tr></thead>
                    <tbody class="font-mono text-[11.5px]">{#each p.rows as r}<tr class="border-t border-line/60">{#each Object.values(r) as v}<td class="px-3 py-1 tabular-nums">{fmt(v)}</td>{/each}</tr>{/each}</tbody>
                  </table>
                {/if}
                {#if p.note}
                  <div class="flex items-center gap-2 border-t border-line px-3 py-1.5 text-[12px] text-muted">
                    <span class="flex-1">{p.note}</span>
                    {#if p.status === 'blocked' && p.sql}
                      <button class="flex items-center gap-1 text-[12px] font-medium text-accent hover:underline" onclick={() => onInsertSql(p.sql!)}><Copy size={12} /> To editor</button>
                    {/if}
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
          {#if chat.busy && m === chat.messages.at(-1) && m.parts.length === 0}
            <div class="flex items-center gap-2 text-[12.5px] text-muted"><LoaderCircle size={13} class="animate-spin text-accent" /> Thinking…</div>
          {/if}
        </div>
      {/if}
    {/each}
  </div>

  <div class="mx-3 mb-3 rounded-xl border border-line bg-field p-2 transition-[border-color,box-shadow] focus-within:border-accent/70 focus-within:ring-3 focus-within:ring-accent/15">
    <textarea
      rows="2"
      bind:value={input}
      placeholder="Ask about your data…"
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
      <button class="btn btn-primary ml-auto size-7 rounded-lg p-0" onclick={() => submit()} disabled={chat.busy || !input.trim()} aria-label="Send">
        <ArrowUp size={15} />
      </button>
    </div>
  </div>
</section>
