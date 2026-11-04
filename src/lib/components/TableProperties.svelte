<script lang="ts" module>
  import type { ColumnInfo, TableProperties as TableProps } from '../api';
  // Survives the remount when the tab switches between Data and Properties; Refresh replaces the entry.
  const cache = new Map<string, { columns: ColumnInfo[]; info: TableProps; ddl: string }>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { Columns2, KeyRound, Link, Link2, ListTree, FileCode, LoaderCircle, RotateCw, CircleAlert } from '@lucide/svelte';
  import { api, ApiError } from '../api';
  import type { Conn } from '../state.svelte';
  import { displayName, type TableRef } from '../tableActions';

  let { conn, table }: { conn: Conn; table: TableRef } = $props();

  type Section = 'columns' | 'constraints' | 'foreignKeys' | 'references' | 'indexes' | 'ddl';
  const sections: { id: Section; label: string; icon: typeof Columns2 }[] = [
    { id: 'columns', label: 'Columns', icon: Columns2 },
    { id: 'constraints', label: 'Constraints', icon: KeyRound },
    { id: 'foreignKeys', label: 'Foreign Keys', icon: Link },
    { id: 'references', label: 'References', icon: Link2 },
    { id: 'indexes', label: 'Indexes', icon: ListTree },
    { id: 'ddl', label: 'DDL', icon: FileCode },
  ];
  let section = $state<Section>('columns');

  // The parent keys this component by tab, and a tab's table never changes.
  const key = untrack(() => `${conn.id}/${table.database}/${table.schema ?? ''}/${table.name}`);
  const hit = cache.get(key);
  let columns = $state<ColumnInfo[] | null>(hit?.columns ?? null);
  let info = $state<TableProps | null>(hit?.info ?? null);
  let ddl = $state<string | null>(hit?.ddl ?? null);
  let error = $state('');
  let loading = $state(false);

  // One load fills every section, so switching sections never waits.
  async function load() {
    loading = true;
    error = '';
    const { database, schema, name } = table;
    try {
      const [c, p, d] = await Promise.all([
        api.listColumns(conn.id, database, schema, name),
        api.tableProperties(conn.id, database, schema, name),
        api.tableDdl(conn.id, database, schema, name),
      ]);
      cache.set(key, { columns: c, info: p, ddl: d });
      [columns, info, ddl] = [c, p, d];
    } catch (e) {
      error = ApiError.from(e).message;
    } finally {
      loading = false;
    }
  }
  if (!hit) load();

  const count = (s: Section) =>
    s === 'columns' ? columns?.length : s === 'ddl' ? undefined : info?.[s].length;
  const fkTarget = (schema: string | undefined, t: string) => (schema ? `${schema}.${t}` : t);
  const rule = (r: string) => (r === 'NO ACTION' ? '—' : r);
</script>

{#snippet empty(text: string)}
  <div class="px-3 py-6 text-center text-[12.5px] text-faint">{text}</div>
{/snippet}

<div class="flex min-h-0 flex-1">
  <nav class="flex w-44 shrink-0 flex-col gap-0.5 border-r border-line p-1.5" aria-label="Table properties">
    {#each sections as s (s.id)}
      {@const n = count(s.id)}
      <button class="nav-item {section === s.id ? 'nav-item-active' : ''}" aria-current={section === s.id ? 'page' : undefined} onclick={() => (section = s.id)}>
        <s.icon size={13} class="shrink-0 {section === s.id ? 'text-accent' : 'text-faint'}" />
        <span class="flex-1">{s.label}</span>
        {#if n != null}<span class="text-[11px] text-faint tabular-nums">{n}</span>{/if}
      </button>
    {/each}
    <div class="mt-auto flex items-center gap-1.5 px-1 pt-2 text-[11.5px] text-faint">
      <span class="min-w-0 flex-1 truncate" title={displayName(table)}>{displayName(table)}</span>
      <button class="btn btn-ghost size-6 p-0" title="Refresh properties" aria-label="Refresh properties" disabled={loading} onclick={load}>
        {#if loading}<LoaderCircle size={13} class="animate-spin" />{:else}<RotateCw size={13} />{/if}
      </button>
    </div>
  </nav>

  <div class="min-h-0 min-w-0 flex-1 overflow-auto">
    {#if error}
      <div class="m-4 flex items-start gap-2.5 rounded-lg bg-red-500/8 px-3.5 py-3 text-[12.5px] text-red-700 dark:text-red-400" role="alert">
        <CircleAlert size={15} class="mt-px shrink-0" /> <span>{error}</span>
      </div>
    {:else if loading && !columns}
      <div class="grid h-full place-items-center text-faint"><LoaderCircle size={16} class="animate-spin" /></div>
    {:else if section === 'ddl'}
      <pre class="p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink">{ddl ?? ''}</pre>
    {:else}
      <table class="w-full border-separate border-spacing-0 text-[12.5px]">
        <thead class="sticky top-0 z-10 bg-surface text-left">
          <tr class="[&>th]:h-8 [&>th]:border-b [&>th]:border-line [&>th]:px-3 [&>th]:font-medium [&>th]:whitespace-nowrap [&>th]:text-muted">
            {#if section === 'columns'}
              <th class="w-10">#</th><th>Name</th><th>Type</th><th>Not null</th><th>Default</th><th>Key</th>
            {:else if section === 'constraints'}
              <th>Name</th><th>Type</th><th>Columns</th><th>Definition</th>
            {:else if section === 'foreignKeys' || section === 'references'}
              <th>Name</th>{#if section === 'references'}<th>Table</th>{/if}<th>Columns</th><th>{section === 'references' ? 'Referenced columns' : 'Referenced table'}</th>{#if section === 'foreignKeys'}<th>Referenced columns</th>{/if}<th>On update</th><th>On delete</th>
            {:else}
              <th>Name</th><th>Columns</th><th>Unique</th><th>Method</th><th>Definition</th>
            {/if}
          </tr>
        </thead>
        <tbody class="[&_td]:h-8 [&_td]:border-b [&_td]:border-line/60 [&_td]:px-3 [&_td]:whitespace-nowrap [&_tr:hover]:bg-hover">
          {#if section === 'columns'}
            {#each columns ?? [] as c, i (c.name)}
              <tr>
                <td class="text-faint tabular-nums">{i + 1}</td>
                <td class="font-medium text-ink">{c.name}</td>
                <td class="font-mono text-[12px] text-muted">{c.dataType}</td>
                <td>{c.nullable ? '' : '✓'}</td>
                <td class="font-mono text-[12px] text-muted">{c.default ?? ''}</td>
                <td>{#if c.isPk}<span class="tag text-accent"><KeyRound size={11} /> PK{c.pkOrdinal && c.pkOrdinal > 1 ? ` ${c.pkOrdinal}` : ''}</span>{/if}</td>
              </tr>
            {:else}<tr><td colspan="6">{@render empty('No columns')}</td></tr>{/each}
          {:else if section === 'constraints'}
            {#each info?.constraints ?? [] as c (c.name)}
              <tr>
                <td class="font-medium text-ink">{c.name}</td>
                <td><span class="tag">{c.kind === 'primary' ? 'PRIMARY KEY' : c.kind.toUpperCase()}</span></td>
                <td class="font-mono text-[12px]">{c.columns.join(', ')}</td>
                <td class="font-mono text-[12px] text-muted">{c.definition ?? ''}</td>
              </tr>
            {:else}<tr><td colspan="4">{@render empty('No constraints')}</td></tr>{/each}
          {:else if section === 'foreignKeys'}
            {#each info?.foreignKeys ?? [] as f (f.name)}
              <tr>
                <td class="font-medium text-ink">{f.name}</td>
                <td class="font-mono text-[12px]">{f.columns.join(', ')}</td>
                <td>{fkTarget(f.refSchema, f.refTable)}</td>
                <td class="font-mono text-[12px]">{f.refColumns.join(', ')}</td>
                <td class="text-muted">{rule(f.onUpdate)}</td>
                <td class="text-muted">{rule(f.onDelete)}</td>
              </tr>
            {:else}<tr><td colspan="6">{@render empty('No foreign keys')}</td></tr>{/each}
          {:else if section === 'references'}
            {#each info?.references ?? [] as f (`${f.schema}.${f.table}.${f.name}`)}
              <tr>
                <td class="font-medium text-ink">{f.name}</td>
                <td>{fkTarget(f.schema, f.table)}</td>
                <td class="font-mono text-[12px]">{f.columns.join(', ')}</td>
                <td class="font-mono text-[12px]">{f.refColumns.join(', ')}</td>
                <td class="text-muted">{rule(f.onUpdate)}</td>
                <td class="text-muted">{rule(f.onDelete)}</td>
              </tr>
            {:else}<tr><td colspan="6">{@render empty('No other table references this table')}</td></tr>{/each}
          {:else}
            {#each info?.indexes ?? [] as x (x.name)}
              <tr>
                <td class="font-medium text-ink">{x.name}{#if x.primary} <span class="tag ml-1 text-accent">PK</span>{/if}</td>
                <td class="font-mono text-[12px]">{x.columns.join(', ')}</td>
                <td>{x.unique ? '✓' : ''}</td>
                <td class="text-muted">{x.method ?? ''}</td>
                <td class="max-w-[28rem] truncate font-mono text-[12px] text-muted" title={x.definition}>{x.definition ?? ''}</td>
              </tr>
            {:else}<tr><td colspan="5">{@render empty('No indexes')}</td></tr>{/each}
          {/if}
        </tbody>
      </table>
    {/if}
  </div>
</div>
