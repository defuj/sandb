<script lang="ts">
  import { GitCommitHorizontal, LoaderCircle } from '@lucide/svelte';
  import { db } from '../state.svelte';
  import { ApiError } from '../api';
  import { txPrompt, endTx } from '../tx.svelte';

  let busy = $state(false);
  let error = $state('');
  const ask = $derived(txPrompt.ask);
  // Tabs already ended by a partly failed Commit or Rollback drop off the list.
  const left = $derived(ask?.tabs.filter((t) => t.status?.pending) ?? []);
  // COMMIT on an aborted PG transaction only rolls back; offering it would mislead.
  const aborted = $derived(left.some((t) => t.status?.aborted));

  function finish(settled: boolean) {
    const a = txPrompt.ask;
    txPrompt.ask = null;
    busy = false;
    error = '';
    a?.resolve(settled);
  }

  async function end(commit: boolean) {
    busy = true;
    error = '';
    for (const t of left) {
      try {
        await endTx(t, commit);
      } catch (e) {
        const err = ApiError.from(e);
        error = `${t.title}: ${err.detail ? `${err.message}: ${err.detail}` : err.message}`;
        busy = false;
        return;
      }
    }
    finish(true);
  }

  const connName = (id: string) => db.connections.find((c) => c.id === id)?.name ?? id;
</script>

<svelte:window onkeydown={(e) => ask && !busy && e.key === 'Escape' && (e.stopPropagation(), finish(false))} />

{#if ask}
  <div class="dialog-backdrop">
    <div class="dialog max-w-md" role="alertdialog" aria-labelledby="pending-tx-title">
      <header class="dialog-head">
        <GitCommitHorizontal size={15} class="text-amber-600 dark:text-amber-400" />
        <h2 id="pending-tx-title" class="flex-1">Uncommitted changes</h2>
      </header>

      <div class="dialog-body">
        <p class="leading-relaxed text-muted">{ask.action} ends the open transaction of {left.length === 1 ? 'this tab' : 'these tabs'}. Commit the changes or roll them back first.</p>
        <ul class="mt-3 divide-y divide-line rounded-lg border border-line">
          {#each left as t (t.id)}
            <li class="flex items-center gap-2 px-3 py-2">
              <span class="size-1.5 shrink-0 rounded-full bg-amber-500"></span>
              <b class="truncate font-medium">{t.title}</b>
              <span class="truncate text-[12.5px] text-muted">{connName(t.connId)}</span>
              {#if t.status?.aborted}<span class="tag ml-auto text-red-700 dark:text-red-400">aborted</span>{/if}
            </li>
          {/each}
        </ul>
        {#if aborted}
          <p class="mt-2.5 text-[12.5px] text-muted">A failed statement aborted a transaction; it can only be rolled back.</p>
        {/if}

        {#if error}
          <p class="mt-3 break-words rounded-lg bg-red-500/8 px-3 py-2 text-[12.5px] text-red-700 dark:text-red-300" role="alert">{error}</p>
        {/if}
      </div>

      <footer class="dialog-foot">
        <button class="btn btn-secondary" disabled={busy} onclick={() => finish(false)}>Cancel</button>
        <button class="btn btn-secondary" disabled={busy} onclick={() => end(false)}>Rollback</button>
        <button class="btn btn-primary" disabled={busy || aborted} onclick={() => end(true)}>
          {#if busy}<LoaderCircle size={13} class="animate-spin" />{/if} Commit
        </button>
      </footer>
    </div>
  </div>
{/if}
