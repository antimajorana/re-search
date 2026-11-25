import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const katex = require('../vendor/katex/katex.min.js');

const assetsDir = new URL('../assets/', import.meta.url);
const mathUrl = new URL('math.js', assetsDir).href;
const graphUrl = new URL('investigation_graph.js', assetsDir).href;

const mapSrc = (await readFile(new URL('investigation_map.js', assetsDir), 'utf8'))
  .replace("'/assets/math.js'", JSON.stringify(mathUrl))
  .replace("'/assets/investigation_graph.js'", JSON.stringify(graphUrl));
const { createPaperMap, foldSearchSpan, renderAnchorExcerptInto } = await import(
  'data:text/javascript;base64,' + Buffer.from(mapSrc).toString('base64')
);

const convSrc = (await readFile(new URL('investigation_conventions.js', assetsDir), 'utf8'))
  .replace("'/assets/math.js'", JSON.stringify(mathUrl));
const { createConventionCatalog } = await import(
  'data:text/javascript;base64,' + Buffer.from(convSrc).toString('base64')
);

function makeFakeDom() {
  const mk = (tag = 'div') => {
    let text = '', html = '';
    const node = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      children: [],
      hidden: false,
      open: false,
      disabled: false,
      value: '',
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) {
        this.children = [];
        text = String(v ?? '');
        html = '';
      },
      get innerHTML() {
        return html || (this.children.length ? this.children.map((c) => c.innerHTML || c.textContent || '').join('') : text);
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
      getBoundingClientRect() {
        return { top: 0 };
      },
      querySelector(sel) {
        return this.querySelectorAll(sel)[0] || null;
      },
      querySelectorAll(sel) {
        const out = [];
        const visit = (n) => {
          for (const c of n.children || []) {
            const match = sel.startsWith('.')
              ? String(c.className || '').split(/\s+/).includes(sel.slice(1))
              : c.tagName === sel.toUpperCase();
            if (match) out.push(c);
            visit(c);
          }
        };
        visit(this);
        return out;
      },
    };
    return node;
  };
  const prevDoc = globalThis.document;
  const prevWin = globalThis.window;
  globalThis.window = { katex };
  globalThis.document = {
    createElement: mk,
    createTextNode: (s) => ({ textContent: String(s) }),
    getElementById: () => mk('div'),
    activeElement: null,
  };
  return {
    mk,
    restore() {
      globalThis.document = prevDoc;
      globalThis.window = prevWin;
    },
  };
}

test('audit_honesty: foldSearchSpan must not widen <mark> across earlier currency $ into unquoted prose', () => {
  const dom = makeFakeDom();
  try {
    const excerpt = 'At $5 per sample, the gap $\\Delta > 0$ persists.';
    const quote = 'the gap $\\Delta > 0$ persists';
    const span = foldSearchSpan(excerpt, quote);
    assert.deepEqual(
      excerpt.slice(span[0], span[1]),
      quote,
      'currency $5 before the quote must not be absorbed into the highlighted quote span',
    );
    const quoteEl = dom.mk('blockquote');
    renderAnchorExcerptInto(quoteEl, excerpt, quote);
    const mark = quoteEl.querySelectorAll('mark')[0];
    assert.ok(
      !mark.textContent.includes('$5 per sample'),
      `unquoted prose leaked inside <mark>: ${mark.textContent}`,
    );
  } finally {
    dom.restore();
  }
});

test('audit_honesty: renderAnchors must not claim lines were corrected when cited_range equals final range', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.mk('div');
    const mapApi = createPaperMap(host, {
      openSource() {},
      openPdf: () => false,
      citedPaperLinks: () => [],
      act() {},
    });
    const container = dom.mk('div');
    mapApi.renderAnchors(container, [
      {
        source_id: 'paper',
        path: 'paper/main.txt',
        start_line: 10,
        end_line: 13,
        excerpt: 'Eq. (1)\n\n\nThe mass gap strictly increases with coupling.',
        quote: 'mass gap strictly increases with coupling',
        kind: 'equation',
        label: 'Eq. (1)',
        cited_range: [10, 13],
        range_basis: ['narrowed', 'trimmed', 'label_extended'],
      },
    ]);
    const notes = container.querySelectorAll('.investigation-anchor-note');
    assert.equal(
      notes.length,
      0,
      `unchanged range 10–13 was falsely labelled corrected: ${notes.map((n) => n.textContent).join('; ')}`,
    );
  } finally {
    dom.restore();
  }
});

test('audit_honesty: convention catalog with pending_items must not lock out Continue button when done is true', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.mk('details');
    const summary = dom.mk('summary');
    host.append(summary);
    const catalogUi = createConventionCatalog(host, () => {}, async () => {});
    catalogUi.update(
      {
        source_id: 'src-1',
        map_seq: 11,
        conventions_requested: true,
        conventions_is_current: true,
        conventions: {
          items: [
            {
              id: 'hbar',
              category: 'units',
              label: 'hbar = 1',
              meaning: 'Natural units',
              scope: 'Paper',
              basis: 'explicit',
              anchors: [],
            },
          ],
          done: true,
          coverage: 'paper',
          limitations: [],
          pending_items: ['metric-sign'],
        },
      },
      false,
    );
    assert.ok(summary.textContent.includes('1 citation pending'), summary.textContent);
    const resumeBtn = host.querySelectorAll('button')[0];
    assert.equal(
      resumeBtn.hidden,
      false,
      `catalog shows "${summary.textContent}" yet hides the Continue button so pending citations can never be resolved`,
    );
  } finally {
    dom.restore();
  }
});
