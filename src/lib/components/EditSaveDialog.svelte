<script lang="ts">
  import { X, ShieldAlert, Save, FileCode } from '@lucide/svelte';
  import { engines, envTag, type Conn } from '../state.svelte';
  import type { Stmt } from '../edits.svelte';

  let { mode = $bindable(), stmts, conn, onsave }: { mode: 'preview' | 'save' | null; stmts: Stmt[]; conn: Conn; onsave: () => void } = $props();

  let ack = $state(false);
  const prod = $derived(conn.env === 'production');
  const kindTint = { UPDATE: 'text-amber-700 dark:text-amber-400', INSERT: 'text-emerald-700 dark:text-emerald-400', DELETE: 'text-red-700 dark:text-red-400' };

  $effect(() => {
    if (!mode) ack = false;
  });

  // Pending edits can be discarded underneath the dialog (connection switch, Discard).
  $effect(() => {
    if (mode && stmts.length === 0) mode = null;
  });

  function onkey(e: KeyboardEvent) {
    if (mode && e.key === 'Escape') mode = null;
  }
</script>

<svelte:window onkeydown={onkey} />

{#if mode}
  <div class="dialog-backdrop">
    <div class="dialog max-w-xl">
      <header class="dialog-head">
        {#if mode === 'save'}<Save size={15} class="text-faint" />{:else}<FileCode size={15} class="text-faint" />{/if}
        <h2 class="flex-1">{mode === 'save' ? 'Save changes' : 'Generated SQL'}</h2>
        <button class="btn btn-ghost -my-1 -mr-1.5 size-7 p-0" onclick={() => (mode = null)} aria-label="Close"><X size={15} /></button>
      </header>

      <div class="dialog-body">
        <div class="flex flex-wrap items-center gap-2">
          <span class="font-mono text-[10.5px] font-semibold {engines[conn.engine].badge}">{engines[conn.engine].short}</span>
          <span class="font-medium">{conn.name}</span>
          <span class="tag {envTag[conn.env]}">{conn.env}</span>
          <span class="ml-auto text-[12px] text-faint">{stmts.length} statement{stmts.length === 1 ? '' : 's'} · one transaction</span>
        </div>

        <div class="mt-2.5 max-h-72 overflow-auto rounded-lg border border-line bg-sunken px-3 py-2.5 font-mono text-[12px] leading-relaxed">
          {#each stmts as s, i}
            <div class="flex gap-2.5">
              <span aria-hidden="true" class="w-4 shrink-0 text-right text-faint select-none">{i + 1}</span>
              <span class="min-w-0 whitespace-pre-wrap"><span class={kindTint[s.kind]}>{s.kind}</span>{s.sql.slice(s.kind.length)}</span>
            </div>
          {/each}
        </div>

        {#if mode === 'save' && prod}
          <label class="mt-3 flex cursor-pointer items-start gap-2.5 rounded-lg bg-red-500/8 px-3 py-2.5 text-[12.5px]">
            <input type="checkbox" class="mt-0.5 accent-red-600" bind:checked={ack} />
            <span><ShieldAlert size={13} class="mr-0.5 inline text-red-600 dark:text-red-400" /> This is a <b>production</b> connection. I want to write these changes to <b class="font-mono">{conn.name}</b>.</span>
          </label>
        {/if}
      </div>

      <footer class="dialog-foot">
        {#if mode === 'save'}
          <button class="btn btn-secondary" onclick={() => (mode = null)}>Cancel</button>
          <button class="btn btn-primary" disabled={prod && !ack} onclick={onsave}>Save {stmts.length} change{stmts.length === 1 ? '' : 's'}</button>
        {:else}
          <button class="btn btn-secondary" onclick={() => (mode = null)}>Close</button>
          <button class="btn btn-primary" onclick={() => (mode = 'save')}>Save…</button>
        {/if}
      </footer>
    </div>
  </div>
{/if}
