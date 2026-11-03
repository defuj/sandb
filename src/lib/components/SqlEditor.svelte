<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorView, keymap, lineNumbers, highlightActiveLine } from '@codemirror/view';
  import { EditorState, Compartment } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
  import { sql, PostgreSQL, MySQL, SQLite } from '@codemirror/lang-sql';
  import { syntaxHighlighting, HighlightStyle } from '@codemirror/language';
  import { autocompletion } from '@codemirror/autocomplete';
  import { tags as t } from '@lezer/highlight';

  let {
    value = $bindable(''),
    onrun,
    schema = {},
    dialect = 'pg',
  }: { value?: string; onrun?: () => void; schema?: Record<string, string[]>; dialect?: 'pg' | 'mysql' | 'sqlite' } = $props();

  const lang = new Compartment();
  const langExt = () => sql({ dialect: { pg: PostgreSQL, mysql: MySQL, sqlite: SQLite }[dialect], schema, upperCaseKeywords: true });

  export function focus() {
    view?.focus();
  }

  /** The selected text, or '' when nothing is selected. */
  export function selection() {
    const r = view?.state.selection.main;
    return view && r && !r.empty ? view.state.sliceDoc(r.from, r.to) : '';
  }

  let el: HTMLDivElement;
  let view = $state.raw<EditorView>();

  const hl = HighlightStyle.define([
    { tag: t.keyword, color: 'var(--c-accent)', fontWeight: '500' },
    { tag: [t.string, t.special(t.string)], color: 'var(--c-syn-string)' },
    { tag: t.number, color: 'var(--c-syn-number)' },
    { tag: t.comment, color: 'var(--c-faint)', fontStyle: 'italic' },
    { tag: [t.operator, t.punctuation], color: 'var(--c-muted)' },
    { tag: t.typeName, color: 'var(--c-syn-type)' },
  ]);

  const theme = EditorView.theme({
    '&': { height: '100%', fontSize: '13px', backgroundColor: 'transparent', color: 'var(--c-ink)' },
    '&.cm-focused': { outline: 'none' },
    '.cm-scroller': { fontFamily: "'JetBrains Mono Variable', ui-monospace, monospace", lineHeight: '1.6' },
    '.cm-content': { caretColor: 'var(--c-accent)', padding: '2px 0 8px' },
    '.cm-gutters': { backgroundColor: 'transparent', border: 'none', color: 'var(--c-faint)', paddingLeft: '10px', paddingRight: '4px' },
    '.cm-activeLine': { backgroundColor: 'var(--c-hover)' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent', color: 'var(--c-ink)' },
    '.cm-cursor': { borderLeftColor: 'var(--c-accent)', borderLeftWidth: '2px' },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'color-mix(in srgb, var(--c-accent) 24%, transparent) !important' },
    '.cm-tooltip': { backgroundColor: 'var(--c-surface)', border: '1px solid var(--c-line)', borderRadius: '8px', overflow: 'hidden', boxShadow: '0 12px 32px -8px rgba(0,0,0,.3)', fontFamily: "'JetBrains Mono Variable', ui-monospace, monospace", fontSize: '12px', padding: '4px' },
    '.cm-tooltip-autocomplete ul li': { padding: '3px 8px !important', borderRadius: '5px' },
    '.cm-tooltip-autocomplete ul li[aria-selected]': { backgroundColor: 'var(--c-active)', color: 'var(--c-ink)' },
  });

  onMount(() => {
    view = new EditorView({
      parent: el,
      state: EditorState.create({
        doc: value,
        extensions: [
          lineNumbers(),
          highlightActiveLine(),
          history(),
          keymap.of([{ key: 'Mod-Enter', run: () => (onrun?.(), true) }, ...defaultKeymap, ...historyKeymap]),
          lang.of(langExt()),
          autocompletion(),
          syntaxHighlighting(hl),
          theme,
          EditorView.updateListener.of((u) => {
            if (u.docChanged) value = u.state.doc.toString();
          }),
        ],
      }),
    });
    return () => view?.destroy();
  });

  $effect(() => {
    const ext = langExt();
    view?.dispatch({ effects: lang.reconfigure(ext) });
  });

  // Sync when the parent replaces the text (e.g. clicking a table in the sidebar).
  $effect(() => {
    const v = value;
    if (view && v !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: v } });
    }
  });
</script>

<div bind:this={el} class="h-full overflow-hidden"></div>
