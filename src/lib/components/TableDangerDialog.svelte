<script lang="ts">
  import { Trash, Eraser, ShieldAlert, LoaderCircle } from '@lucide/svelte';
  import { engines, envTag } from '../state.svelte';
  import { ApiError } from '../api';
  import { dangerSql, displayName, kindLabel, type Danger } from '../tableActions';

  let { target = $bindable(null), onconfirm }: { target?: Danger | null; onconfirm: (d: Danger) => Promise<void> } = $props();

  let ack = $state(false);
  let busy = $state(false);
  let error = $state('');
  const prod = $derived(target?.c.env === 'production');

  $effect(() => {
    if (!target) {
      ack = false;
      busy = false;
      error = '';
    }
  });

  async function go(d: Danger) {
    busy = true;
    error = '';
    try {
      await onconfirm(d);
    } catch (e) {
      const err = ApiError.from(e);
      error = err.detail ? `${err.message}: ${err.detail}` : err.message;
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={(e) => target && !busy && e.key === 'Escape' && (target = null)} />

{#if target}
  {@const d = target}
  {@const drop = d.action === 'drop'}
  {@const what = kindLabel[d.t.kind]}
  <div class="dialog-backdrop">
    <div class="dialog max-w-md" role="alertdialog" aria-labelledby="danger-title">
      <header class="dialog-head">
        {#if drop}<Trash size={15} class="text-red-600 dark:text-red-400" />{:else}<Eraser size={15} class="text-red-600 dark:text-red-400" />{/if}
        <h2 id="danger-title" class="flex-1">{drop ? `Drop this ${what}?` : 'Truncate this table?'}</h2>
      </header>

      <div class="dialog-body">
        <p class="flex flex-wrap items-center gap-2 rounded-lg bg-sunken px-3 py-2">
          <span class="font-mono text-[10.5px] font-semibold {engines[d.c.engine].badge}">{engines[d.c.engine].short}</span>
          <b class="font-medium text-ink">{d.c.name}</b>
          <span class="tag {envTag[d.c.env]}">{d.c.env}</span>
          <span class="font-mono text-[12px] text-muted">{d.c.engine === 'sqlite' ? '' : d.t.database + ' · '}{displayName(d.t)}</span>
        </p>
        <p class="mt-3 leading-relaxed text-muted">
          {drop ? `This removes the ${what} and all its data from the database.` : 'This deletes every row of the table.'}
          The statement runs immediately and commits. You cannot undo it.
        </p>
        <pre class="mt-3 overflow-x-auto rounded-lg bg-sunken px-3 py-2 font-mono text-[12px] text-ink">{dangerSql(d.c.engine, d.t, d.action)}</pre>

        {#if prod}
          <label class="mt-3 flex cursor-pointer items-start gap-2.5 rounded-lg bg-red-500/8 px-3 py-2.5 text-[12.5px]">
            <input type="checkbox" class="mt-0.5 accent-red-600" bind:checked={ack} />
            <span><ShieldAlert size={13} class="mr-0.5 inline text-red-600 dark:text-red-400" /> This is a <b>production</b> connection. I want to {d.action} <b class="font-mono">{displayName(d.t)}</b>.</span>
          </label>
        {/if}

        {#if error}
          <p class="mt-3 break-words rounded-lg bg-red-500/8 px-3 py-2 text-[12.5px] text-red-700 dark:text-red-300" role="alert"><b class="font-semibold">{drop ? 'Drop' : 'Truncate'} failed:</b> {error}</p>
        {/if}
      </div>

      <footer class="dialog-foot">
        <button class="btn btn-secondary" disabled={busy} onclick={() => (target = null)}>Cancel</button>
        <button class="btn bg-red-600 text-white enabled:hover:bg-red-500" disabled={(prod && !ack) || busy} onclick={() => go(d)}>
          {#if busy}<LoaderCircle size={13} class="animate-spin" />{/if} {drop ? `Drop ${what}` : 'Truncate table'}
        </button>
      </footer>
    </div>
  </div>
{/if}
