<script lang="ts">
  import DOMPurify from 'dompurify';
  import { Marked, type Tokens } from 'marked';
  import { fenceDialect, sqlHtml } from '../highlight';

  let { text, onrunsql }: { text: string; onrunsql?: (sql: string, anchor: HTMLElement) => void } = $props();

  // SQL code blocks are colored like the editor. Other languages stay plain.
  const md = new Marked({
    gfm: true,
    breaks: true,
    renderer: {
      code({ text: code, lang }: Tokens.Code) {
        const dialect = fenceDialect(lang, code);
        return dialect ? `<pre class="sql-code"><code class="language-sql">${sqlHtml(code, dialect)}\n</code></pre>\n` : false;
      },
    },
  });

  // Model output is untrusted. Keep Markdown formatting, but never its active HTML.
  // Each table gets a scroll box: the table fills the panel width and scrolls when it is wider.
  // Each code block gets a copy button. It is added after sanitizing, so it is our own markup.
  const copyIcon = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/></svg>';
  const checkIcon = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';
  const playIcon = '<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 3 20 12 6 21 6 3"/></svg>';
  /** A code block with its actions. The SQL action set adds Run; other languages only copy. */
  const codeBox = (run: boolean) =>
    `<div class="code-box"><span class="code-actions"><button type="button" class="code-copy" title="Copy code" aria-label="Copy code">${copyIcon}</button>` +
    (run ? `<button type="button" class="code-run" title="Run query" aria-label="Run query" aria-haspopup="menu">${playIcon}</button>` : '') +
    '</span><pre>';
  // Placeholders keep the second replaceAll from wrapping the <pre> of the first one.
  const SQL_PRE = '\u0001sql-pre\u0001';
  const CODE_PRE = '\u0001code-pre\u0001';
  const html = $derived(
    DOMPurify.sanitize(md.parse(text, { async: false }), {
      FORBID_TAGS: ['style', 'form', 'input', 'button', 'textarea', 'select', 'option', 'img'],
      FORBID_ATTR: ['style'],
    })
      .replaceAll('<table>', '<div class="table-box"><table>')
      .replaceAll('</table>', '</table></div>')
      .replaceAll('<pre class="sql-code">', SQL_PRE)
      .replaceAll('<pre>', CODE_PRE)
      .replaceAll(SQL_PRE, codeBox(!!onrunsql))
      .replaceAll(CODE_PRE, codeBox(false))
      .replaceAll('</pre>', '</pre></div>'),
  );

  /** One listener for all code blocks: copy, or run the SQL next to the clicked button. */
  function codeActions(node: HTMLElement) {
    const onClick = async (ev: MouseEvent) => {
      const btn = (ev.target as Element).closest<HTMLButtonElement>('button');
      if (!btn) return;
      const code = btn.closest('.code-box')?.querySelector('pre')?.textContent?.trimEnd();
      if (code == null) return;
      if (btn.classList.contains('code-run')) {
        onrunsql?.(code, btn);
        return;
      }
      if (!btn.classList.contains('code-copy')) return;
      try {
        await navigator.clipboard.writeText(code);
        btn.innerHTML = checkIcon;
        setTimeout(() => (btn.innerHTML = copyIcon), 1200);
      } catch (e) {
        console.warn('Copy failed.', e);
      }
    };
    node.addEventListener('click', onClick);
    return { destroy: () => node.removeEventListener('click', onClick) };
  }
</script>

<div class="chat-markdown" use:codeActions>{@html html}</div>

<style>
  .chat-markdown {
    font-size: 13px;
    line-height: 1.625;
    overflow-wrap: anywhere;
  }
  .chat-markdown > :global(*:first-child) { margin-top: 0; }
  .chat-markdown > :global(*:last-child) { margin-bottom: 0; }
  .chat-markdown :global(p) { margin: 0 0 10px; }
  .chat-markdown :global(h1),
  .chat-markdown :global(h2),
  .chat-markdown :global(h3),
  .chat-markdown :global(h4) {
    margin: 16px 0 6px;
    color: var(--c-ink);
    font-size: 13px;
    font-weight: 650;
    line-height: 1.4;
  }
  .chat-markdown :global(ul),
  .chat-markdown :global(ol) { margin: 0 0 10px; padding-left: 20px; }
  .chat-markdown :global(ul) { list-style-type: disc; }
  .chat-markdown :global(ol) { list-style-type: decimal; }
  .chat-markdown :global(li) { margin: 3px 0; padding-left: 2px; }
  .chat-markdown :global(li)::marker { color: var(--c-faint); }
  .chat-markdown :global(li) > :global(p) { margin: 0; }
  .chat-markdown :global(a) { color: var(--c-accent); text-decoration: underline; text-underline-offset: 2px; }
  .chat-markdown :global(strong) { color: var(--c-ink); font-weight: 650; }
  .chat-markdown :global(code) {
    border-radius: 4px;
    background: var(--c-active);
    padding: 1px 4px;
    font-family: 'JetBrains Mono Variable', ui-monospace, monospace;
    font-size: 11.5px;
  }
  .chat-markdown :global(pre) {
    margin: 0 0 10px;
    overflow-x: auto;
    border: 1px solid var(--c-line);
    border-radius: 7px;
    background: var(--c-sunken);
    padding: 10px 12px;
  }
  .chat-markdown :global(pre) :global(code) { background: transparent; padding: 0; font-size: 11.5px; line-height: 1.6; }
  .chat-markdown :global(.code-box) { position: relative; margin: 0 0 10px; }
  .chat-markdown :global(.code-box) > :global(pre) { margin: 0; }
  .chat-markdown :global(.code-actions) {
    position: absolute;
    top: 5px;
    right: 5px;
    display: flex;
    gap: 4px;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .chat-markdown :global(.code-box:hover .code-actions),
  .chat-markdown :global(.code-actions:focus-within) { opacity: 1; }
  .chat-markdown :global(.code-actions button) {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 4px;
    background: var(--c-sunken);
    color: var(--c-faint);
  }
  .chat-markdown :global(.code-actions button:hover) { background: var(--c-hover); color: var(--c-ink); }
  .chat-markdown :global(.code-actions .code-run:hover) { color: var(--c-accent); }
  @media (hover: none) {
    .chat-markdown :global(.code-actions) { opacity: 1; }
  }
  .chat-markdown :global(blockquote) {
    margin: 0 0 10px;
    border-left: 2px solid var(--c-line);
    padding-left: 10px;
    color: var(--c-muted);
  }
  .chat-markdown :global(.table-box) {
    margin: 0 0 10px;
    overflow-x: auto;
    border: 1px solid var(--c-line);
    border-radius: 7px;
  }
  .chat-markdown :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: 11.5px;
    white-space: nowrap;
  }
  .chat-markdown :global(th),
  .chat-markdown :global(td) { padding: 6px 9px; text-align: left; vertical-align: top; }
  .chat-markdown :global(th) { background: var(--c-sunken); color: var(--c-muted); font-weight: 600; }
  .chat-markdown :global(tbody) :global(tr) { border-top: 1px solid var(--c-line); }
  .chat-markdown :global(td) { font-variant-numeric: tabular-nums; }
  .chat-markdown :global(hr) { margin: 14px 0; border: 0; border-top: 1px solid var(--c-line); }
</style>
