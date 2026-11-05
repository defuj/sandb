<script lang="ts">
  import { BookmarkPlus } from '@lucide/svelte';
  import { db, engines } from '../state.svelte';
  import type { SaveAsk } from '../saved.svelte';

  let { ask = $bindable(null), onsave }: { ask?: SaveAsk | null; onsave: (a: SaveAsk, name: string) => void } = $props();

  let name = $state('');
  const connId = $derived(ask?.tab.connId);
  const conn = $derived(db.connections.find((c) => c.id === connId));
  const lines = $derived(ask ? ask.tab.query.split('\n').length : 0);

  $effect(() => {
    if (ask) name = ask.name;
  });

  // The name effect fills the input after mount, so the select waits a frame.
  const select = (el: HTMLInputElement) => {
    el.focus();
    requestAnimationFrame(() => el.select());
  };

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (ask && name.trim()) onsave(ask, name.trim());
  }
</script>

<svelte:window onkeydown={(e) => { if (ask && e.key === 'Escape') { e.preventDefault(); ask = null; } }} />

{#if ask}
  {@const a = ask}
  <div class="dialog-backdrop">
    <button class="absolute inset-0 cursor-default" aria-label="Close" tabindex="-1" onclick={() => (ask = null)}></button>
    <div class="dialog max-w-sm" role="dialog" aria-labelledby="save-q-title">
    <form onsubmit={submit}>
      <header class="dialog-head">
        <BookmarkPlus size={15} class="text-accent" />
        <h2 id="save-q-title" class="flex-1">{a.copy ? 'Save as new query' : 'Save query'}</h2>
      </header>

      <div class="dialog-body space-y-3">
        <label class="block space-y-1.5">
          <span class="label">Name</span>
          <input class="input" bind:value={name} use:select placeholder="e.g. Daily paid orders" autocapitalize="off" autocorrect="off" spellcheck="false" autocomplete="off" />
        </label>
        <div class="space-y-1.5">
          <span class="label">Saved for</span>
          <p class="flex items-center gap-2 rounded-lg bg-sunken px-3 py-2 text-[12.5px]">
            {#if conn}<span class="font-mono text-[10.5px] font-semibold {engines[conn.engine].badge}">{engines[conn.engine].short}</span>{/if}
            <span class="min-w-0 truncate font-medium text-ink">{conn?.name ?? 'No connection'}</span>
            {#if a.tab.database}<span class="text-faint">/</span><span class="min-w-0 truncate text-muted">{a.tab.database}</span>{/if}
            <span class="ml-auto shrink-0 text-[11.5px] text-faint tabular-nums">{lines} {lines === 1 ? 'line' : 'lines'}</span>
          </p>
        </div>
        <p class="text-[12px] leading-relaxed text-faint">Opening the query later selects this connection and database again. After this, <kbd class="kbd">⌘S</kbd> saves your changes without this dialog.</p>
      </div>

      <footer class="dialog-foot">
        <button type="button" class="btn btn-secondary" onclick={() => (ask = null)}>Cancel</button>
        <button type="submit" class="btn btn-primary" disabled={!name.trim()}>Save <kbd class="ml-0.5 font-sans text-[11px] font-normal opacity-80" aria-hidden="true">↵</kbd></button>
      </footer>
    </form>
    </div>
  </div>
{/if}
