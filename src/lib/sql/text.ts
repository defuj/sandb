/** SQL text helpers shared by the editor and the column autocomplete. */

export type Dialect = 'pg' | 'mysql' | 'sqlite';

/**
 * Cuts the text at `;` outside strings, quoted names, comments and Postgres dollar quotes, like the
 * backend split. Each range ends after its `;`; the last one may have none.
 */
export function statementRanges(doc: string, dialect: Dialect): [number, number][] {
  const out: [number, number][] = [];
  const skipTo = (i: number, s: string) => {
    const e = doc.indexOf(s, i);
    return e < 0 ? doc.length : e + s.length - 1;
  };
  let start = 0;
  for (let i = 0; i < doc.length; i++) {
    const ch = doc[i];
    if ((ch === '-' && doc[i + 1] === '-') || (ch === '#' && dialect === 'mysql')) i = skipTo(i, '\n');
    else if (ch === '/' && doc[i + 1] === '*') i = skipTo(i + 2, '*/');
    else if (ch === "'" || ch === '"' || (ch === '`' && dialect === 'mysql')) {
      // A doubled quote stays inside the string; MySQL also escapes with a backslash.
      let j = i + 1;
      while (j < doc.length && !(doc[j] === ch && doc[j + 1] !== ch)) j += doc[j] === ch || (doc[j] === '\\' && dialect === 'mysql') ? 2 : 1;
      i = j;
    } else if (ch === '$' && dialect === 'pg') {
      // $$ or $tag$ opens a dollar quote; $1 (a parameter) does not match.
      const tag = /^\$(?:[A-Za-z_]\w*)?\$/.exec(doc.slice(i, i + 64))?.[0];
      if (tag) i = skipTo(i + tag.length, tag);
    } else if (ch === ';') {
      out.push([start, i + 1]);
      start = i + 1;
    }
  }
  if (start < doc.length) out.push([start, doc.length]);
  return out;
}

/** The statement range around `pos`; undefined when the document holds none. */
export function statementRangeAt(doc: string, pos: number, dialect: Dialect): [number, number] | undefined {
  return statementRanges(doc, dialect).find(([from, to]) => pos >= from && pos <= to);
}

/** The statement around `pos`; '' when the document holds none. */
export function statementTextAt(doc: string, pos: number, dialect: Dialect): string {
  const range = statementRangeAt(doc, pos, dialect);
  return range ? doc.slice(range[0], range[1]) : '';
}

/**
 * The same text with strings and comments replaced by spaces, so a scan for `FROM` cannot see a
 * table name inside a literal or a comment. Quoted identifiers stay: they can name a table.
 */
export function maskLiterals(text: string, dialect: Dialect): string {
  const out = text.split('');
  const blank = (from: number, to: number) => {
    for (let i = from; i < to && i < out.length; i++) if (out[i] !== '\n') out[i] = ' ';
  };
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if ((ch === '-' && text[i + 1] === '-') || (ch === '#' && dialect === 'mysql')) {
      const end = text.indexOf('\n', i);
      const to = end < 0 ? text.length : end;
      blank(i, to);
      i = to - 1;
    } else if (ch === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      const to = end < 0 ? text.length : end + 2;
      blank(i, to);
      i = to - 1;
    } else if (ch === "'") {
      // A doubled quote stays inside the string; MySQL also escapes with a backslash.
      let j = i + 1;
      while (j < text.length && !(text[j] === "'" && text[j + 1] !== "'")) j += text[j] === '\\' && dialect === 'mysql' ? 2 : 1;
      const to = Math.min(j + 1, text.length);
      blank(i, to);
      i = to - 1;
    } else if (ch === '$' && dialect === 'pg') {
      const tag = /^\$(?:[A-Za-z_]\w*)?\$/.exec(text.slice(i, i + 64))?.[0];
      if (tag) {
        const end = text.indexOf(tag, i + tag.length);
        const to = end < 0 ? text.length : end + tag.length;
        blank(i, to);
        i = to - 1;
      }
    }
  }
  return out.join('');
}
