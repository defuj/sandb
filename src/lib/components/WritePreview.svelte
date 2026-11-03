<script lang="ts">
  import { TriangleAlert, X } from '@lucide/svelte';
  import { chat } from '../chat.svelte';
  import { active, engines, envTag } from '../state.svelte';

  const conn = $derived(active());

  const destructive = $derived(/^\s*(delete|drop|truncate)/i.test(chat.pending?.sql ?? ''));
  const lines = $derived((chat.pending?.sql ?? '').split('\n'));
  const decide = (ok: boolean) => chat.pending?.resolve(ok);
</script>

{#if chat.pending}
  <div class="dialog-backdrop">
    <div class="dialog max-w-lg">
      <header class="dialog-head">
        <TriangleAlert size={15} class="text-amber-600 dark:text-amber-400" />
        <h2 class="flex-1">The assistant wants to change data</h2>
        <button class="btn btn-ghost -my-1 -mr-1.5 size-7 p-0" onclick={() => decide(false)} aria-label="Close"><X size={15} /></button>
      </header>

      <div class="dialog-body">
        <p class="text-muted">Read the statement below. It runs only if you approve it.</p>

        <div class="mt-3 flex items-center gap-2">
          <span class="font-mono text-[10.5px] font-semibold {engines[conn.engine].badge}">{engines[conn.engine].short}</span>
          <span class="font-medium">{conn.name}</span>
          <span class="tag {envTag[conn.env]}">{conn.env}</span>
          <span class="tag font-mono text-amber-700 dark:text-amber-400">{chat.pending.sql.trim().split(/\s+/)[0].toUpperCase()}</span>
          <span class="ml-auto text-[12px] text-faint tabular-nums">≈ 214 rows</span>
        </div>

        <div class="mt-2.5 flex overflow-x-auto rounded-lg border border-line bg-sunken p-3 font-mono text-[12px] leading-relaxed">
          <div aria-hidden="true" class="mr-3 shrink-0 text-right text-faint select-none">{#each lines as _, i}{i + 1}{'\n'}{/each}</div>
          <pre>{chat.pending.sql}</pre>
        </div>

        {#if destructive}
          <div class="mt-3 rounded-lg bg-red-500/8 px-3 py-2 text-[12.5px] text-red-700 dark:text-red-300">
            This statement removes data. It runs inside a transaction, but you cannot undo it after it commits.
          </div>
        {/if}
      </div>

      <footer class="dialog-foot">
        <button class="btn btn-secondary" onclick={() => decide(false)}>Deny</button>
        <button class="btn {destructive ? 'bg-red-600 text-white hover:bg-red-500' : 'btn-primary'}" onclick={() => decide(true)}>Approve and run</button>
      </footer>
    </div>
  </div>
{/if}
