<script lang="ts">
  import DOMPurify from 'dompurify';
  import { marked } from 'marked';

  let { text }: { text: string } = $props();

  // Model output is untrusted. Keep Markdown formatting, but never its active HTML.
  // Each table gets a scroll box: the table fills the panel width and scrolls when it is wider.
  const html = $derived(
    DOMPurify.sanitize(marked.parse(text, { async: false, gfm: true, breaks: true }), {
      FORBID_TAGS: ['style', 'form', 'input', 'button', 'textarea', 'select', 'option', 'img'],
      FORBID_ATTR: ['style'],
    })
      .replaceAll('<table>', '<div class="table-box"><table>')
      .replaceAll('</table>', '</table></div>'),
  );
</script>

<div class="chat-markdown">{@html html}</div>

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
