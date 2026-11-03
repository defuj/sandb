<script lang="ts">
  import { Trash, ShieldAlert, LoaderCircle } from '@lucide/svelte';
  import { engines, envTag, type Conn } from '../state.svelte';
  import { ApiError } from '../api';

  let { target = $bindable(null), ondelete }: { target?: Conn | null; ondelete: (c: Conn) => void | Promise<void> } = $props();

  let ack = $state(false);
  let busy = $state(false);
  let error = $state('');
  const prod = $derived(target?.env === 'production');

  $effect(() => {
    if (!target) {
      ack = false;
      busy = false;
      error = '';
    }
  });

  async function remove(c: Conn) {
    busy = true;
    error = '';
    try {
      await ondelete(c);
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
  {@const c = target}
  <div class="dialog-backdrop">
    <div class="dialog max-w-md" role="alertdialog" aria-labelledby="del-conn-title">
      <header class="dialog-head">
        <Trash size={15} class="text-red-600 dark:text-red-400" />
        <h2 id="del-conn-title" class="flex-1">Delete this connection?</h2>
      </header>

      <div class="dialog-body">
        <p class="flex flex-wrap items-center gap-2 rounded-lg bg-sunken px-3 py-2">
          <span class="font-mono text-[10.5px] font-semibold {engines[c.engine].badge}">{engines[c.engine].short}</span>
          <b class="font-medium text-ink">{c.name}</b>
          <span class="tag {envTag[c.env]}">{c.env}</span>
        </p>
        <p class="mt-3 leading-relaxed text-muted">This removes the saved connection from sandb only. The database and its data are not changed. The audit log keeps its entries.</p>

        {#if prod}
          <label class="mt-3 flex cursor-pointer items-start gap-2.5 rounded-lg bg-red-500/8 px-3 py-2.5 text-[12.5px]">
            <input type="checkbox" class="mt-0.5 accent-red-600" bind:checked={ack} />
            <span><ShieldAlert size={13} class="mr-0.5 inline text-red-600 dark:text-red-400" /> This is a <b>production</b> connection. I want to delete <b class="font-mono">{c.name}</b>.</span>
          </label>
        {/if}

        {#if error}
          <p class="mt-3 break-words rounded-lg bg-red-500/8 px-3 py-2 text-[12.5px] text-red-700 dark:text-red-300" role="alert"><b class="font-semibold">Delete failed:</b> {error}</p>
        {/if}
      </div>

      <footer class="dialog-foot">
        <button class="btn btn-secondary" disabled={busy} onclick={() => (target = null)}>Cancel</button>
        <button class="btn bg-red-600 text-white enabled:hover:bg-red-500" disabled={(prod && !ack) || busy} onclick={() => remove(c)}>
          {#if busy}<LoaderCircle size={13} class="animate-spin" />{/if} Delete connection
        </button>
      </footer>
    </div>
  </div>
{/if}
