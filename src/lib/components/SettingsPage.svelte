<script lang="ts">
  import { ArrowLeft, Bot, KeyRound, Eye, EyeOff, Lock, Gauge, ShieldCheck, Palette, Sun, Moon, Monitor, Check, LoaderCircle, X, CircleArrowUp } from '@lucide/svelte';
  import { ui, ai, db, providers, setTheme, engines, envTag, canWrite, aiSettings, saveAiSoon, saveAiKeySoon, flushAi, removeAiKey, type Provider, type Conn } from '../state.svelte';
  import { api, ApiError } from '../api';
  import { appVersion, updates, checkForUpdates, installUpdate, restartApp, updatesEnabled as canUpdate } from '../updates.svelte';

  let { onEnableWrites }: { onEnableWrites: (c: Conn) => void } = $props();

  let showKey = $state(false);
  let test = $state<'idle' | 'testing' | 'ok' | 'fail'>('idle');
  let tested = $state<{ models: number; ms: number } | null>(null);
  let testError = $state<ApiError | null>(null);
  // A test that a later edit made out of date must not show its result.
  let testSeq = 0;

  const keyNote = { tauri: 'Stored encrypted on this computer.', bridge: 'Kept in memory while the dev bridge runs.', mock: 'Demo build: no AI backend.' }[api.kind];

  const p = $derived(providers[ai.provider]);

  function changed() {
    testSeq++;
    test = 'idle';
    saveAiSoon();
  }

  function keyChanged() {
    testSeq++;
    test = 'idle';
    saveAiKeySoon();
  }

  function pick(id: Provider) {
    ai.provider = id;
    if (id !== 'custom') {
      ai.baseUrl = providers[id].url;
      ai.model = providers[id].model;
    }
    changed();
  }

  async function removeKey() {
    testSeq++;
    test = 'idle';
    try {
      await removeAiKey();
    } catch (e) {
      testError = ApiError.from(e);
      test = 'fail';
    }
  }

  async function runTest() {
    const seq = ++testSeq;
    test = 'testing';
    try {
      await flushAi();
      const r = await api.testAi(aiSettings(), ai.apiKey || null);
      if (seq !== testSeq) return;
      tested = { models: r.modelCount, ms: r.latencyMs };
      test = 'ok';
    } catch (e) {
      if (seq !== testSeq) return;
      testError = ApiError.from(e);
      test = 'fail';
    }
  }

  function toggleWrites(c: Conn) {
    if (c.aiWrite) c.aiWrite = false;
    else if (canWrite(c)) onEnableWrites(c);
  }
</script>

<div class="flex h-full flex-col">
  <header class="flex h-12 shrink-0 items-center gap-2.5 border-b border-line px-4">
    <button class="btn btn-ghost -ml-1.5 size-7 p-0" onclick={() => (ui.page = 'query')} aria-label="Back to query"><ArrowLeft size={15} /></button>
    <h1 class="text-[14px] font-semibold">Settings</h1>
    <span class="truncate text-[12.5px] text-faint">AI provider, privacy, safeguards and appearance</span>
  </header>

  <div class="flex-1 overflow-auto">
    <div class="mx-auto max-w-[44rem] divide-y divide-line px-6 py-2">
      <!-- AI provider -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><Bot size={14} class="text-faint" /> AI provider</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">Any OpenAI-compatible endpoint. Your key stays on this computer.</p>
        </div>
        <div class="min-w-0">
          <div class="seg flex-wrap">
            {#each Object.entries(providers) as [id, v] (id)}
              <button class="seg-item {ai.provider === id ? 'seg-on' : ''}" onclick={() => pick(id as Provider)}>{v.label}</button>
            {/each}
          </div>

          <div class="mt-3 grid gap-3 sm:grid-cols-2">
            <label class="block sm:col-span-2">
              <span class="label mb-1.5 block">Base URL</span>
              <input class="input font-mono text-[12.5px]" bind:value={ai.baseUrl} oninput={changed} placeholder="https://host/v1" />
            </label>
            <label class="block">
              <span class="label mb-1.5 flex items-center gap-1">API key {#if !p.needsKey}<span>(optional)</span>{/if}
                {#if ai.hasKey}<button class="ml-auto rounded px-1 font-medium text-muted hover:bg-hover hover:text-red-600 dark:hover:text-red-400" onclick={removeKey} title="Delete the saved key">Remove</button>{/if}
              </span>
              <div class="relative">
                <input class="input pr-9 font-mono text-[12.5px]" type={showKey ? 'text' : 'password'} bind:value={ai.apiKey} oninput={keyChanged} placeholder={ai.hasKey ? 'Saved key is hidden. Type to replace it.' : p.needsKey ? 'sk-…' : 'Not needed for local models'} />
                <button class="absolute inset-y-0 right-1 grid w-7 place-items-center text-faint hover:text-ink" onclick={() => (showKey = !showKey)} aria-label={showKey ? 'Hide key' : 'Show key'}>
                  {#if showKey}<EyeOff size={14} />{:else}<Eye size={14} />{/if}
                </button>
              </div>
            </label>
            <label class="block">
              <span class="label mb-1.5 block">Model</span>
              <input class="input font-mono text-[12.5px]" bind:value={ai.model} oninput={changed} placeholder="model-name" />
            </label>
          </div>

          <div class="mt-3 flex items-center gap-3">
            <span class="flex items-center gap-1.5 text-[12px] text-faint"><KeyRound size={12} /> {keyNote}</span>
            <div class="flex-1"></div>
            {#if test === 'ok' && tested}<span class="flex items-center gap-1 text-[12px] font-medium text-emerald-700 dark:text-emerald-400"><Check size={13} /> Connected · {tested.models.toLocaleString()} {tested.models === 1 ? 'model' : 'models'} · {tested.ms} ms</span>
            {:else if test === 'fail'}<span class="flex items-center gap-1 text-[12px] font-medium text-red-700 dark:text-red-400"><X size={13} /> Failed</span>
            {:else if test === 'testing'}<span class="text-[12px] text-muted">testing…</span>{/if}
            <button class="btn btn-secondary" onclick={runTest} disabled={test === 'testing'}>
              {#if test === 'testing'}<LoaderCircle size={13} class="animate-spin" />{/if} Test
            </button>
          </div>
          {#if test === 'fail' && testError}
            <p class="mt-1.5 text-right text-[12px] break-words text-muted">{testError.message}{#if testError.code === 'model_not_found' && testError.detail}{' '}<span class="text-faint">Models: {testError.detail}</span>{/if}</p>
          {/if}
        </div>
      </section>

      <!-- Privacy -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><Lock size={14} class="text-faint" /> What the AI can see</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">Choose what leaves this machine when you ask the assistant.</p>
        </div>
        <div class="grid min-w-0 gap-2 sm:grid-cols-2">
          {#each [{ id: 'results', t: 'Schema and query results', d: 'Best answers. Result rows go to the model.' }, { id: 'schema', t: 'Schema only', d: 'Rows never leave this computer. The AI only writes SQL.' }] as o (o.id)}
            <button class="rounded-lg border px-3.5 py-3 text-left transition {ai.privacy === o.id ? 'border-accent/70 bg-accent/6 ring-3 ring-accent/12' : 'border-line hover:bg-hover'}" onclick={() => { ai.privacy = o.id as 'results' | 'schema'; changed(); }}>
              <div class="font-medium">{o.t}</div>
              <div class="mt-0.5 text-[12.5px] leading-snug text-muted">{o.d}</div>
            </button>
          {/each}
        </div>
      </section>

      <!-- Limits -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><Gauge size={14} class="text-faint" /> Limits for AI queries</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">Caps applied to every query the assistant runs.</p>
        </div>
        <div class="grid min-w-0 gap-3 sm:grid-cols-2">
          <label class="block">
            <span class="label mb-1.5 block">Max rows</span>
            <input class="input font-mono tabular-nums" type="number" min="1" max="10000" bind:value={ai.maxRows} oninput={changed} />
          </label>
          <label class="block">
            <span class="label mb-1.5 block">Timeout (seconds)</span>
            <input class="input font-mono tabular-nums" type="number" min="1" max="300" bind:value={ai.timeoutSec} oninput={changed} />
          </label>
        </div>
      </section>

      <!-- Safeguards -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><ShieldCheck size={14} class="text-faint" /> Safeguards</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">Read-only is on for every connection by default. With write mode on, each write still needs your approval.</p>
        </div>
        <div class="min-w-0 divide-y divide-line rounded-lg border border-line">
          {#each db.connections as c (c.id)}
            {@const on = c.aiWrite && canWrite(c)}
            <div class="flex items-center gap-2 px-3 py-2.5">
              <span class="w-5 shrink-0 font-mono text-[10.5px] font-semibold {engines[c.engine].badge}">{engines[c.engine].short}</span>
              <span class="truncate font-medium">{c.name}</span>
              <span class="tag {envTag[c.env]}">{c.env}</span>
              <div class="flex-1"></div>
              {#if canWrite(c)}
                <span class="text-[12px] {on ? 'text-amber-600 dark:text-amber-400' : 'text-muted'}">{on ? 'Writes need approval' : 'Read-only'}</span>
              {:else}
                <span class="text-[12px] text-muted">Connection is read-only</span>
              {/if}
              <button
                role="switch"
                aria-checked={on}
                aria-label="AI write mode for {c.name}"
                class="relative h-5 w-9 shrink-0 rounded-full transition disabled:cursor-not-allowed disabled:opacity-50 {on ? 'bg-amber-500' : 'bg-stone-500'}"
                disabled={!canWrite(c)}
                title={canWrite(c) ? undefined : 'Turn off read-only in the connection settings to allow AI writes'}
                onclick={() => toggleWrites(c)}>
                <span class="absolute top-0.5 size-4 rounded-full bg-white shadow transition-all {on ? 'left-[18px]' : 'left-0.5'}"></span>
              </button>
            </div>
          {/each}
        </div>
      </section>

      <!-- Appearance -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><Palette size={14} class="text-faint" /> Appearance</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">Theme for the whole app.</p>
        </div>
        <div class="min-w-0">
          <div class="seg">
            {#each [{ id: 'light', l: 'Light', i: Sun }, { id: 'dark', l: 'Dark', i: Moon }, { id: 'system', l: 'System', i: Monitor }] as t (t.id)}
              <button
                class="seg-item flex items-center gap-1.5 {ui.theme === t.id ? 'seg-on' : ''}"
                onclick={() => setTheme(t.id as 'light' | 'dark' | 'system')}><t.i size={13} /> {t.l}</button>
            {/each}
          </div>
        </div>
      </section>

      <!-- Updates -->
      <section class="grid gap-x-10 gap-y-3 py-6 sm:grid-cols-[180px_1fr]">
        <div>
          <h2 class="flex items-center gap-2 text-[13px] font-semibold"><CircleArrowUp size={14} class="text-faint" /> Updates</h2>
          <p class="mt-1 text-[12.5px] leading-relaxed text-muted">sandb asks before it installs anything.</p>
        </div>
        <div class="min-w-0">
          <div class="flex items-center gap-3">
            <span class="font-mono text-[12.5px]">sandb v{appVersion}</span>
            <div class="flex-1"></div>
            {#if updates.status === 'latest'}<span class="flex items-center gap-1 text-[12px] font-medium text-emerald-700 dark:text-emerald-400"><Check size={13} /> Up to date</span>
            {:else if updates.status === 'available'}<span class="text-[12px] font-medium text-amber-600 dark:text-amber-400">v{updates.next} is available</span>
            {:else if updates.status === 'error'}<span class="flex items-center gap-1 text-[12px] font-medium text-red-700 dark:text-red-400"><X size={13} /> Check failed</span>{/if}
            {#if updates.status === 'restart'}
              <span class="text-[12px] font-medium text-amber-600 dark:text-amber-400">v{updates.next} is installed</span>
              <button class="btn btn-primary" onclick={restartApp}>Restart to finish the update</button>
            {:else if updates.status === 'available' || updates.status === 'installing'}
              <button class="btn btn-primary" onclick={installUpdate} disabled={updates.status === 'installing'}>
                {#if updates.status === 'installing'}<LoaderCircle size={13} class="animate-spin" /> Installing…{:else}Install and restart{/if}
              </button>
            {:else}
              <button class="btn btn-secondary" onclick={() => checkForUpdates()} disabled={!canUpdate || updates.status === 'checking'} title={canUpdate ? undefined : 'Updates work in the installed desktop app only'}>
                {#if updates.status === 'checking'}<LoaderCircle size={13} class="animate-spin" />{/if} Check for updates
              </button>
            {/if}
          </div>
          {#if updates.status === 'available' && updates.notes}<p class="mt-2 text-[12.5px] leading-relaxed whitespace-pre-line text-muted">{updates.notes}</p>{/if}
          {#if updates.error && (updates.status === 'error' || updates.status === 'restart')}<p class="mt-1.5 text-[12px] break-words text-muted">{updates.error}</p>{/if}
        </div>
      </section>
    </div>
  </div>
</div>
