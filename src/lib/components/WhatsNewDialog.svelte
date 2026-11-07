<script lang="ts">
  import { Sparkles } from '@lucide/svelte';
  import Markdown from './Markdown.svelte';
  import { appVersion, closeWhatsNew, whatsNew } from '../updates.svelte';
</script>

<svelte:window onkeydown={(e) => { if (whatsNew.releases.length && e.key === 'Escape') { e.preventDefault(); closeWhatsNew(); } }} />

{#if whatsNew.releases.length}
  <div class="dialog-backdrop">
    <button class="absolute inset-0 cursor-default" aria-label="Close" tabindex="-1" onclick={closeWhatsNew}></button>
    <div class="dialog relative flex max-h-[80vh] max-w-lg flex-col" role="dialog" aria-labelledby="whats-new-title">
      <header class="dialog-head">
        <Sparkles size={15} class="text-accent" />
        <h2 id="whats-new-title" class="flex-1">What's new in sandb {appVersion}</h2>
      </header>
      <div class="dialog-body min-h-0 space-y-4 overflow-y-auto">
        {#each whatsNew.releases as r (r.version)}
          <section>
            <h3 class="text-[12.5px] font-semibold text-ink">{r.version} <span class="font-normal text-faint">{r.date}</span></h3>
            <div class="text-[12.5px] text-muted"><Markdown text={r.notes} /></div>
          </section>
        {/each}
      </div>
      <footer class="dialog-foot">
        <button class="btn btn-primary" onclick={closeWhatsNew}>Got it</button>
      </footer>
    </div>
  </div>
{/if}
