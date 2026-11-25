import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const katex = require('../vendor/katex/katex.min.js');

const assetsDir = new URL('../assets/', import.meta.url);
const mathUrl = new URL('math.js', assetsDir).href;
const graphUrl = new URL('investigation_graph.js', assetsDir).href;

let mapSrc = await readFile(new URL('investigation_map.js', assetsDir), 'utf8');
mapSrc = mapSrc
  .replace("'/assets/math.js'", JSON.stringify(mathUrl))
  .replace("'/assets/investigation_graph.js'", JSON.stringify(graphUrl));
const { foldSearchSpan, renderAnchorExcerptInto } = await import(
  'data:text/javascript;base64,' + Buffer.from(mapSrc).toString('base64')
);

function makeFakeDom() {
  const mk = tag => {
    let text = '';
    let html = '';
    return {
      tagName: String(tag || 'div').toUpperCase(),
      className: '',
      hidden: false,
      children: [],
      get textContent() {
        return this.children.length ? this.children.map(c => c.textContent ?? '').join('') : text;
      },
      set textContent(v) {
        this.children = [];
        text = String(v ?? '');
        html = '';
      },
      get innerHTML() {
        return html || (this.children.length ? this.children.map(c => c.innerHTML || c.textContent || '').join('') : text);
      },
      set innerHTML(v) {
        this.children = [];
        html = String(v ?? '');
        text = html.replace(/<[^>]+>/g, '');
      },
      append(...nodes) {
        this.children.push(...nodes);
      },
      replaceChildren(...nodes) {
        this.children = [...nodes];
        text = '';
        html = '';
      },
      addEventListener() {},
      setAttribute() {},
    };
  };
  const prevDoc = globalThis.document;
  const prevWin = globalThis.window;
  globalThis.window = { katex };
  globalThis.document = {
    createElement: mk,
    createTextNode: s => ({ textContent: String(s) }),
  };
  return {
    mk,
    restore() {
      globalThis.document = prevDoc;
      globalThis.window = prevWin;
    },
  };
}

// Finding F21 (minor): `mathSpans` in `investigation_map.js` checks `$`, `$$`, `\(`, `\[`,
// but ignores `\begin{equation}...\end{equation}` math environments. Highlighting a quote
// inside `\begin{equation}...\end{equation}` splits `\begin` and `\end` across separate DOM
// fragments so KaTeX renders 0 equations and leaks raw LaTeX delimiters.
test('audit_honesty_anchor_quote_inside_begin_equation_must_render_katex', () => {
  const dom = makeFakeDom();
  try {
    const excerpt = '\\begin{equation}\nE = mc^2 + pc\n\\end{equation}';
    const quote = 'E = mc^2 + pc';
    const span = foldSearchSpan(excerpt, quote);
    const quoteEl = dom.mk('blockquote');
    renderAnchorExcerptInto(quoteEl, excerpt, quote);
    const html = quoteEl.innerHTML;
    assert.ok(
      span && span[0] === 0 && span[1] === excerpt.length && html.includes('katex'),
      `expected widened span [0,${excerpt.length}] and KaTeX equation, got span=${JSON.stringify(span)} html=${html}`,
    );
  } finally {
    dom.restore();
  }
});
