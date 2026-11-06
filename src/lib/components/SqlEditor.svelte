<script lang="ts">
  import { onMount } from 'svelte';
  import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection, gutter, GutterMarker } from '@codemirror/view';
  import { EditorState, Compartment, RangeSetBuilder, type RangeSet, type Text } from '@codemirror/state';
  import { defaultKeymap, history, historyKeymap, selectAll as selectAllCmd } from '@codemirror/commands';
  import { sql, PostgreSQL, MySQL, SQLite } from '@codemirror/lang-sql';
  import { syntaxHighlighting, HighlightStyle, syntaxTree } from '@codemirror/language';
  import { autocompletion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
  import { tags as t } from '@lezer/highlight';
  import { columnCompletion } from '../sql/columns';
  import { statementRanges, maskLiterals, type Dialect } from '../sql/text';

  let {
    value = $bindable(''),
    onrun,
    onrunstatement,
    schema = {},
    dialect = 'pg',
    columnsOf,
  }: {
    value?: string;
    onrun?: () => void;
    /** Runs one statement from the Run gutter, without changing the editor text. */
    onrunstatement?: (sql: string) => void;
    schema?: Record<string, string[]>;
    dialect?: 'pg' | 'mysql' | 'sqlite';
    /** Columns of a table that `schema` holds without columns (or not under this name); [] when unknown. */
    columnsOf?: (path: string[]) => Promise<string[]>;
  } = $props();

  const lang = new Compartment();
  const langExt = () => sql({ dialect: { pg: PostgreSQL, mysql: MySQL, sqlite: SQLite }[dialect], schema, upperCaseKeywords: true });

  // Fetched columns per `schema` object; a new snapshot clears them.
  let fetched = new Map<string, Promise<string[]>>();
  let fetchedFor: Record<string, string[]> | undefined;

  /** `FROM t a` / `FROM t AS a` aliases of the statement at `pos` → table path. Same scan as lang-sql. */
  function aliasesAt(state: EditorState, pos: number): Record<string, string[]> {
    const out: Record<string, string[]> = {};
    let stmt = syntaxTree(state).resolveInner(pos, -1);
    while (stmt.name !== 'Statement') {
      if (!stmt.parent) return out;
      stmt = stmt.parent;
    }
    const isId = (n: typeof stmt | null) => n?.name === 'Identifier' || n?.name === 'QuotedIdentifier';
    const text = (n: typeof stmt) => state.sliceDoc(n.from, n.to).replace(/^(["`])(.*)\1$/, '$2');
    const pathOf = (n: typeof stmt) => (n.name === 'CompositeIdentifier' ? n.getChildren('Identifier').concat(n.getChildren('QuotedIdentifier')).sort((a, b) => a.from - b.from).map(text) : [text(n)]);
    let sawFrom = false;
    let prev: typeof stmt | null = null;
    for (let n = stmt.firstChild; n; n = n.nextSibling) {
      const kw = n.name === 'Keyword' ? state.sliceDoc(n.from, n.to).toLowerCase() : null;
      let alias: string | null = null;
      if (!sawFrom) sawFrom = kw === 'from';
      else if (kw === 'as' && prev && isId(n.nextSibling)) alias = text(n.nextSibling!);
      else if (kw && /^(where|group|having|order|union|intersect|except|all|distinct|limit|offset|fetch|for)$/.test(kw)) break;
      else if (prev && isId(n)) alias = text(n);
      if (alias && prev) out[alias] = pathOf(prev);
      prev = /Identifier$/.test(n.name) ? n : null;
    }
    return out;
  }

  /**
   * Columns of one table: from the snapshot when it has them, else from the database (the snapshot's
   * column cap cut them, or a PG table outside `public` is written without its schema). Cached per
   * snapshot, so typing does not ask again.
   */
  async function loadColumns(path: string[]): Promise<string[]> {
    const key = path.join('.');
    if (schema[key]?.length) return schema[key];
    // The snapshot names a `public` PG table (and a `main` SQLite table) without its schema.
    const known = dialect === 'pg' && path.length === 2 && path[0] === 'public' ? schema[path[1]] : dialect === 'sqlite' && path.length === 2 && path[0] === 'main' ? schema[path[1]] : undefined;
    if (known?.length) return known;
    if (!columnsOf) return [];
    if (fetchedFor !== schema) {
      fetched = new Map();
      fetchedFor = schema;
    }
    let p = fetched.get(key);
    if (!p) {
      const cache = fetched;
      p = columnsOf(path).catch(() => (cache.delete(key), []));
      cache.set(key, p);
    }
    return p;
  }

  /**
   * Columns after `table.` or `alias.` when the snapshot has none for that table: it cut them (column
   * cap) or names the table differently (a PG table outside `public`, written without its schema).
   */
  async function lazyColumns(ctx: CompletionContext): Promise<CompletionResult | null> {
    const m = /((?:(?:[A-Za-z_][\w$]*|"[^"]+"|`[^`]+`)\.)+)([\w$]*)$/.exec(ctx.state.sliceDoc(ctx.state.doc.lineAt(ctx.pos).from, ctx.pos));
    if (!m || !columnsOf) return null;
    let path = m[1].slice(0, -1).match(/"[^"]+"|`[^`]+`|[^.]+/g)!.map((p) => p.replace(/^(["`])(.*)\1$/, '$2'));
    if (path.length === 1) path = aliasesAt(ctx.state, ctx.pos)[path[0]] ?? path;
    if (schema[path.join('.')]?.length) return null; // lang-sql completes it
    const cols = await loadColumns(path);
    if (!cols.length || ctx.aborted) return null;
    return { from: ctx.pos - m[2].length, options: cols.map((label) => ({ label, type: 'property' })), validFor: /^[\w$]*$/ };
  }

  /** Columns of the tables in the statement at the cursor, so a plain `SELECT col…` lists them. */
  const columnsInScope = (ctx: CompletionContext) => columnCompletion(ctx, { dialect, schema, load: loadColumns });

  export function focus() {
    view?.focus();
  }

  /** The selected text, or '' when nothing is selected. */
  export function selection() {
    const r = view?.state.selection.main;
    return view && r && !r.empty ? view.state.sliceDoc(r.from, r.to) : '';
  }

  /** Replaces the selection (or inserts at the cursor), as a paste would. */
  export function insert(text: string) {
    if (!view) return;
    view.dispatch(view.state.replaceSelection(text));
    view.focus();
  }

  export function selectAll() {
    if (view) selectAllCmd(view);
    view?.focus();
  }

  /** A right-click outside the selection puts the cursor there, so "statement at cursor" is the clicked one. */
  export function pointAt(x: number, y: number) {
    const pos = view?.posAtCoords({ x, y });
    const r = view?.state.selection.main;
    if (view && r && pos != null && (r.empty || pos < r.from || pos > r.to)) view.dispatch({ selection: { anchor: pos } });
  }

  /**
   * The statement under the cursor, `;` included. On a blank line it takes the next statement; after
   * the last one, the last one. '' when the editor holds no statement.
   */
  export function statementAt() {
    if (!view) return '';
    const doc = view.state.doc.toString();
    const head = view.state.selection.main.head;
    const parts = statementRanges(doc, dialect)
      .map(([from, to]) => ({ to, text: doc.slice(from, to).trim() }))
      .filter((p) => p.text && p.text !== ';');
    return (parts.find((p) => head <= p.to) ?? parts.at(-1))?.text ?? '';
  }

  /** A play button in the gutter at the first line of each statement. */
  const runIcon = '<svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 1.5 8.5 5 2 8.5Z" fill="currentColor"/></svg>';
  class RunStatementMarker extends GutterMarker {
    readonly sql: string;
    constructor(sql: string) {
      super();
      this.sql = sql;
    }
    eq(other: GutterMarker) {
      return other instanceof RunStatementMarker && other.sql === this.sql;
    }
    toDOM() {
      const b = document.createElement('button');
      b.type = 'button';
      b.className = 'cm-statement-run';
      b.title = 'Run this statement (⇧⌘↵)';
      b.setAttribute('aria-label', 'Run this statement');
      b.tabIndex = -1;
      b.innerHTML = runIcon;
      b.addEventListener('mousedown', (e) => e.preventDefault());
      b.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        onrunstatement?.(this.sql);
      });
      return b;
    }
  }

  /** One marker per line that starts a statement; comments alone get no marker. */
  let markerDoc: Text | null = null;
  let markerDialect: Dialect | null = null;
  let markerSet: RangeSet<GutterMarker> | null = null;

  const runGutter = gutter({
    class: 'cm-run-gutter',
    markers: (v) => {
      // The gutter asks for markers on every view update; only a new document or dialect can change them.
      if (v.state.doc === markerDoc && dialect === markerDialect && markerSet) return markerSet;
      const doc = v.state.doc.toString();
      const builder = new RangeSetBuilder<GutterMarker>();
      let lastLine = -1;
      for (const [from, to] of statementRanges(doc, dialect)) {
        const raw = doc.slice(from, to);
        // Comments and strings are blanked; a comments-only range gets nothing.
        const masked = maskLiterals(raw, dialect);
        if (!masked.trim()) continue;
        const text = raw.trim();
        const at = from + masked.length - masked.trimStart().length;
        const line = v.state.doc.lineAt(at);
        if (line.from === lastLine) continue;
        lastLine = line.from;
        builder.add(line.from, line.from, new RunStatementMarker(text));
      }
      markerDoc = v.state.doc;
      markerDialect = dialect;
      markerSet = builder.finish();
      return markerSet;
    },
    // Cursor moves and scrolling do not redraw the gutter; a dialect reconfigure does.
    lineMarkerChange: (u) => u.docChanged || u.transactions.some((tr) => tr.reconfigured),
    initialSpacer: () => new RunStatementMarker(''),
  });

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
    '.cm-content': { padding: '2px 0 8px' },
    '.cm-gutters': { backgroundColor: 'transparent', border: 'none', color: 'var(--c-faint)', paddingLeft: '10px', paddingRight: '4px' },
    '.cm-run-gutter': { width: '18px' },
    '.cm-run-gutter .cm-gutterElement': { display: 'flex', alignItems: 'center', justifyContent: 'center', padding: '0' },
    '.cm-run-gutter .cm-statement-run': {
      display: 'grid',
      placeItems: 'center',
      width: '16px',
      height: '18px',
      padding: '0',
      border: 'none',
      borderRadius: '3px',
      background: 'transparent',
      color: 'var(--c-faint)',
      opacity: '0.7',
      cursor: 'pointer',
    },
    '.cm-run-gutter .cm-statement-run:hover': { color: 'var(--c-accent)', opacity: '1' },
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
          ...(onrunstatement ? [runGutter] : []),
          lineNumbers(),
          // CodeMirror draws the cursor: WebKit (Tauri) drops the native caret under the glass panels.
          drawSelection(),
          highlightActiveLine(),
          history(),
          keymap.of([
            { key: 'Mod-Enter', run: () => (onrun?.(), true) },
            // Keyboard counterpart of the gutter play button.
            {
              key: 'Mod-Shift-Enter',
              run: () => {
                const sql = statementAt();
                if (!sql || !onrunstatement) return false;
                onrunstatement(sql);
                return true;
              },
            },
            ...defaultKeymap,
            ...historyKeymap,
          ]),
          lang.of(langExt()),
          autocompletion(),
          EditorState.languageData.of(() => [{ autocomplete: columnsInScope }, { autocomplete: lazyColumns }]),
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
