<script lang="ts" module>
  import type { ColumnInfo, TableProperties as TableProps } from '../api';
  // Survives the remount when the tab switches between Data and Properties; Refresh replaces the entry.
  const cache = new Map<string, { columns: ColumnInfo[]; info: TableProps; ddl: string }>();
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { Columns2, KeyRound, Link, Link2, ListTree, FileCode, Network, LoaderCircle, RotateCw, CircleAlert, Plus, Pencil, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../api';
  import { canWrite, type Conn } from '../state.svelte';
  import { displayName, canAlterConstraints, columnDef, constraintDef, foreignKeyDef, indexDef, indexEdit, type SchemaChange, type TableRef } from '../tableActions';
  import SchemaChangeDialog, { type SchemaAsk, type SchemaEdit } from './SchemaChangeDialog.svelte';
  import ErDiagram from './ErDiagram.svelte';

  /** `onchanged`: after a structure change, so the caller can reload what depends on the columns. `onopen`: open a related table from the ER diagram. */
  let { conn, table, onchanged, onopen }: { conn: Conn; table: TableRef; onchanged?: () => void; onopen?: (t: TableRef) => void } = $props();

  type Section = 'columns' | 'constraints' | 'foreignKeys' | 'references' | 'indexes' | 'ddl' | 'diagram';
  const sections: { id: Section; label: string; icon: typeof Columns2 }[] = [
    { id: 'columns', label: 'Columns', icon: Columns2 },
    { id: 'constraints', label: 'Constraints', icon: KeyRound },
    { id: 'foreignKeys', label: 'Foreign Keys', icon: Link },
    { id: 'references', label: 'References', icon: Link2 },
    { id: 'indexes', label: 'Indexes', icon: ListTree },
    { id: 'ddl', label: 'DDL', icon: FileCode },
    { id: 'diagram', label: 'ER Diagram', icon: Network },
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
    s === 'columns' ? columns?.length : s === 'ddl' || s === 'diagram' ? undefined : info?.[s].length;
  const fkTarget = (schema: string | undefined, t: string) => (schema ? `${schema}.${t}` : t);
  const rule = (r: string) => (r === 'NO ACTION' ? '—' : r);

  // Views and read-only connections show the structure only.
  const editable = $derived(table.kind === 'table' && canWrite(conn));
  const addable: Partial<Record<Section, 'column' | 'constraint' | 'foreignKey' | 'index'>> = { columns: 'column', constraints: 'constraint', foreignKeys: 'foreignKey', indexes: 'index' };
  const addLabel = { column: 'Add column', constraint: 'Add constraint', foreignKey: 'Add foreign key', index: 'Add index' };
  // SQLite keeps constraints and foreign keys inside CREATE TABLE; they cannot change afterwards.
  const locked = (s: Section) => (s === 'constraints' || s === 'foreignKeys') && !canAlterConstraints(conn.engine);
  const dropCol = $derived(editable && !!addable[section] && !locked(section));
  const add = $derived(addable[section]);

  let ask = $state<SchemaAsk | null>(null);
  const askDrop = (drop: SchemaChange, label: string) => (ask = { drop, label });
  function changed() {
    ask = null;
    load();
    onchanged?.();
  }
</script>

{#snippet empty(text: string)}
  <div class="px-3 py-6 text-center text-[12.5px] text-faint">{text}</div>
{/snippet}

{#snippet actions(edit: SchemaEdit | null, drop: SchemaChange | null, label: string)}
  <td class="w-16 !px-1 text-right">
    {#if edit}<button class="btn btn-ghost size-6 p-0 text-faint opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:text-ink" title="Edit {label}" aria-label="Edit {label}" onclick={() => (ask = { edit })}><Pencil size={13} /></button>{/if}
    {#if drop}<button class="btn btn-ghost size-6 p-0 text-faint opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:text-red-600 dark:hover:text-red-400" title="Drop {label}" aria-label="Drop {label}" onclick={() => askDrop(drop, label)}><Trash2 size={13} /></button>{/if}
  </td>
{/snippet}

<div class="flex min-h-0 min-w-0 flex-1">
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

  <div class="flex min-h-0 min-w-0 flex-1 flex-col">
  {#if editable && add && !error && columns}
    <div class="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">
      {#if locked(section)}
        <span class="px-1 text-[12px] text-faint">SQLite cannot add or drop {section === 'constraints' ? 'constraints' : 'foreign keys'} on an existing table.</span>
      {:else}
        <button class="btn btn-ghost h-7 px-2" onclick={() => (ask = { add })}><Plus size={13} /> {addLabel[add]}</button>
      {/if}
    </div>
  {/if}
  <div class="min-h-0 min-w-0 flex-1 overflow-auto">
    {#if error}
      <div class="m-4 flex items-start gap-2.5 rounded-lg bg-red-500/8 px-3.5 py-3 text-[12.5px] text-red-700 dark:text-red-400" role="alert">
        <CircleAlert size={15} class="mt-px shrink-0" /> <span>{error}</span>
      </div>
    {:else if loading && !columns}
      <div class="grid h-full place-items-center text-faint"><LoaderCircle size={16} class="animate-spin" /></div>
    {:else if section === 'ddl'}
      <pre class="p-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink">{ddl ?? ''}</pre>
    {:else if section === 'diagram'}
      {#if columns && info}<ErDiagram {conn} {table} {columns} {info} {onopen} />{/if}
    {:else}
      <table class="w-full border-separate border-spacing-0 text-[12.5px]">
        <thead class="sticky top-0 z-10 bg-surface text-left">
          <tr class="[&>th]:h-8 [&>th]:border-b [&>th]:border-line [&>th]:px-3 [&>th]:font-medium [&>th]:whitespace-nowrap [&>th]:text-muted">
            {#if section === 'columns'}
              <th class="w-10">#</th><th>Name</th><th>Type</th><th>Not null</th><th>Default</th><th>Key</th>{#if dropCol}<th></th>{/if}
            {:else if section === 'constraints'}
              <th>Name</th><th>Type</th><th>Columns</th><th>Definition</th>{#if dropCol}<th></th>{/if}
            {:else if section === 'foreignKeys' || section === 'references'}
              <th>Name</th>{#if section === 'references'}<th>Table</th>{/if}<th>Columns</th><th>{section === 'references' ? 'Referenced columns' : 'Referenced table'}</th>{#if section === 'foreignKeys'}<th>Referenced columns</th>{/if}<th>On update</th><th>On delete</th>{#if dropCol}<th></th>{/if}
            {:else}
              <th>Name</th><th>Columns</th><th>Unique</th><th>Method</th><th>Definition</th>{#if dropCol}<th></th>{/if}
            {/if}
          </tr>
        </thead>
        <tbody class="[&_td]:h-8 [&_td]:border-b [&_td]:border-line/60 [&_td]:px-3 [&_td]:whitespace-nowrap [&_tr:hover]:bg-hover">
          {#if section === 'columns'}
            {#each columns ?? [] as c, i (c.name)}
              <tr class="group">
                <td class="text-faint tabular-nums">{i + 1}</td>
                <td class="font-medium text-ink">{c.name}</td>
                <td class="font-mono text-[12px] text-muted">{c.dataType}</td>
                <td>{c.nullable ? '' : '✓'}</td>
                <td class="font-mono text-[12px] text-muted">{c.default ?? ''}</td>
                <td>{#if c.isPk}<span class="tag text-accent"><KeyRound size={11} /> PK{c.pkOrdinal && c.pkOrdinal > 1 ? ` ${c.pkOrdinal}` : ''}</span>{/if}</td>
                {#if dropCol}{@render actions({ kind: 'column', from: columnDef(conn.engine, c) }, { op: 'dropColumn', name: c.name }, `column "${c.name}"`)}{/if}
              </tr>
            {:else}<tr><td colspan={6 + +dropCol}>{@render empty('No columns')}</td></tr>{/each}
          {:else if section === 'constraints'}
            {#each info?.constraints ?? [] as c (c.name)}
              <tr class="group">
                <td class="font-medium text-ink">{c.name}</td>
                <td><span class="tag">{c.kind === 'primary' ? 'PRIMARY KEY' : c.kind.toUpperCase()}</span></td>
                <td class="font-mono text-[12px]">{c.columns.join(', ')}</td>
                <td class="font-mono text-[12px] text-muted">{c.definition ?? ''}</td>
                {#if dropCol}
                  {@const from = constraintDef(c)}
                  {@render actions(from && { kind: 'constraint', from }, { op: 'dropConstraint', kind: c.kind, name: c.name }, `constraint "${c.name}"`)}
                {/if}
              </tr>
            {:else}<tr><td colspan={4 + +dropCol}>{@render empty('No constraints')}</td></tr>{/each}
          {:else if section === 'foreignKeys'}
            {#each info?.foreignKeys ?? [] as f (f.name)}
              <tr class="group">
                <td class="font-medium text-ink">{f.name}</td>
                <td class="font-mono text-[12px]">{f.columns.join(', ')}</td>
                <td>{fkTarget(f.refSchema, f.refTable)}</td>
                <td class="font-mono text-[12px]">{f.refColumns.join(', ')}</td>
                <td class="text-muted">{rule(f.onUpdate)}</td>
                <td class="text-muted">{rule(f.onDelete)}</td>
                {#if dropCol}{@render actions({ kind: 'foreignKey', from: foreignKeyDef(table, f) }, { op: 'dropForeignKey', name: f.name }, `foreign key "${f.name}"`)}{/if}
              </tr>
            {:else}<tr><td colspan={6 + +dropCol}>{@render empty('No foreign keys')}</td></tr>{/each}
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
              <tr class="group">
                <td class="font-medium text-ink">{x.name}{#if x.primary} <span class="tag ml-1 text-accent">PK</span>{/if}</td>
                <td class="font-mono text-[12px]">{x.columns.join(', ')}</td>
                <td>{x.unique ? '✓' : ''}</td>
                <td class="text-muted">{x.method ?? ''}</td>
                <td class="max-w-[28rem] truncate font-mono text-[12px] text-muted" title={x.definition}>{x.definition ?? ''}</td>
                {#if dropCol}
                  {@const how = indexEdit(conn.engine, x, columns ?? [])}
                  <!-- The primary key's index goes with its constraint. -->
                  {@render actions(how && { kind: 'index', from: indexDef(conn.engine, x), renameOnly: how === 'rename' }, x.primary ? null : { op: 'dropIndex', name: x.name }, `index "${x.name}"`)}
                {/if}
              </tr>
            {:else}<tr><td colspan={5 + +dropCol}>{@render empty('No indexes')}</td></tr>{/each}
          {/if}
        </tbody>
      </table>
    {/if}
  </div>
  </div>
</div>

{#if ask}
  <SchemaChangeDialog {conn} {table} columns={columns ?? []} {ask} onclose={() => (ask = null)} ondone={changed} />
{/if}
