import assert from 'node:assert/strict';
import test from 'node:test';

import { createGraphView } from '../assets/graph_view.js';
import {
  buildOutlineTree,
  createQualityBanner,
  formatReadMinutes,
  renderInvestigationOutline,
} from '../assets/investigation_outline.js';
import {
  PHYSICS_EMPTY_NOTE,
  filterResultNodes,
  renderInvestigationResults,
} from '../assets/investigation_results.js';
import {
  NODE_NO_MODEL_TEXT,
  collectNodeRelations,
  renderInvestigationNodeCard,
} from '../assets/investigation_node.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      dataset: {},
      style: {},
      type: '',
      value: '',
      title: '',
      hidden: false,
      disabled: false,
      tabIndex: 0,
      clientWidth: 800,
      clientHeight: 500,
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        contains: (c) => classes.has(c),
        toggle: (c, force) => {
          const next = force === undefined ? !classes.has(c) : Boolean(force);
          if (next) classes.add(c); else classes.delete(c);
          return next;
        },
      },
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, stopPropagation() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      getBoundingClientRect() { return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight }; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const m = /^\[([^=\]]+)(?:="([^"]*)")?\]$/.exec(sel);
          if (m) return m[2] === undefined ? n.attributes.has(m[1]) : n.getAttribute(m[1]) === m[2];
          return n.tagName.toLowerCase() === sel.toLowerCase();
        };
        const walk = (n) => {
          for (const c of n.children || []) {
            if (match(c)) out.push(c);
            walk(c);
          }
        };
        walk(this);
        return out;
      },
    };
    return el;
  };
  return {
    document: {
      createElement: (t) => makeEl(t, null),
      createElementNS: (ns, t) => makeEl(t, ns),
    },
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
  };
}

function collectAllStrings(el, out = []) {
  if (!el) return out;
  if (el.textContent) out.push(el.textContent);
  if (el.innerHTML) out.push(el.innerHTML);
  if (el.title) out.push(el.title);
  for (const [, v] of el.attributes || []) out.push(v);
  for (const c of el.children || []) collectAllStrings(c, out);
  return out;
}

function sampleMapview() {
  return {
    etag: 'head-abc',
    skeleton_seq: 3,
    skeleton_blob: 'blob-123',
    source_id: 'paper-1',
    paper_key: 'arxiv:2601.00001',
    version: 'v1',
    builder: 'tex-1',
    quality: 'full',
    quality_reason: null,
    quality_line: 'Structure read from TeX source, v1',
    nodes: [
      {
        id: 'n-sec0000001', kind: 'section', label: 'Section 1: Setup', group: 'Section 1: Setup',
        parent: null, grade: 'code', badges: [{ text: 'By code', icon: 'code', tone: 'ok' }],
        flags: [], stale: false, dim: false, order: 0, number: '1', title: 'Setup',
        spans: [{ path: 'main.tex', start_line: 10, end_line: 35 }], head_cites: [], cites: [],
        words: 180, eq_count: 1, read_minutes: 1.25,
      },
      {
        id: 'n-def0000002', kind: 'definition', label: 'Definition 1.1: Weighted Laplacian',
        group: 'Section 1: Setup', parent: 'n-sec0000001', grade: 'code',
        badges: [{ text: 'By code', icon: 'code', tone: 'ok' }], flags: [], stale: false, dim: false,
        order: 1, number: '1.1', title: 'Weighted Laplacian',
        spans: [{ path: 'main.tex', start_line: 12, end_line: 18 }], head_cites: [], cites: [],
        words: 45, eq_count: 0, read_minutes: 0.25,
      },
      {
        id: 'n-lem0000003', kind: 'lemma', label: 'Lemma 1.2: Dirichlet identity',
        group: 'Section 1: Setup', parent: 'n-sec0000001', grade: 'code',
        badges: [{ text: 'By code', icon: 'code', tone: 'ok' }],
        flags: ['cites [keller2020] in its heading'], stale: false, dim: false,
        order: 2, number: '1.2', title: 'Dirichlet identity',
        spans: [{ path: 'main.tex', start_line: 20, end_line: 26 }],
        proof_tag: 'n-prf0000004', head_cites: ['keller2020'],
        cites: [{ key: 'keller2020', role: 'statement', rule: 'head_cite' }],
        words: 36, eq_count: 1, read_minutes: 0.45,
      },
      {
        id: 'n-prf0000004', kind: 'proof', label: 'Proof',
        group: 'Section 1: Setup', parent: 'n-sec0000001', grade: 'code',
        badges: [{ text: 'By code', icon: 'code', tone: 'ok' }], flags: [], stale: false, dim: false,
        order: 3, number: null, title: null, proof_of: 'n-lem0000003',
        spans: [{ path: 'main.tex', start_line: 27, end_line: 30 }], head_cites: [], cites: [],
        words: 20, eq_count: 0, read_minutes: 0.11,
      },
      {
        id: 'n-thm0000005', kind: 'theorem', label: 'Theorem 1.3: Maximum principle',
        group: 'Section 1: Setup', parent: 'n-sec0000001', grade: 'code',
        badges: [{ text: 'By code', icon: 'code', tone: 'ok' }], flags: [], stale: false, dim: false,
        order: 4, number: '1.3', title: 'Maximum principle',
        spans: [{ path: 'main.tex', start_line: 31, end_line: 35 }], head_cites: [], cites: [],
        words: 40, eq_count: 0, read_minutes: 0.22,
      },
    ],
    edges: [
      { src: 'n-def0000002', dst: 'n-lem0000003', kind: 'uses', role: 'statement', basis: 'code', rule: 'ref' },
      { src: 'n-def0000002', dst: 'n-thm0000005', kind: 'uses', role: 'statement', basis: 'code', rule: 'ref' },
      { src: 'n-lem0000003', dst: 'n-thm0000005', kind: 'uses', role: 'proof', basis: 'model', rule: 'text_mention' },
      { src: 'n-prf0000004', dst: 'n-lem0000003', kind: 'proves', role: 'proof', basis: 'code', rule: 'proof_adjacent' },
    ],
    legend: [
      { label: 'By code (skeleton)', icon: 'code', tone: 'ok', dashed: false },
      { label: 'Heuristic reference (not checked by code)', icon: 'model', tone: 'muted', dashed: true },
    ],
    stats: [
      { section_tag: 'n-sec0000001', words: 180, read_minutes: 1.25, counts: { definition: 1, lemma: 1, proof: 1, theorem: 1 } },
    ],
    bib: [
      {
        key: 'keller2020',
        raw: 'M. Keller, Discrete Laplacians, 2020.',
        arxiv_id: '2001.01234',
        doi: null,
        title: 'Discrete Laplacians',
        cited_at: [{ path: 'main.tex', start_line: 20, end_line: 20 }],
        locators: [{ site: { path: 'main.tex', start_line: 20, end_line: 20 }, text: 'Proposition 2.1' }],
      },
    ],
    macros: [],
  };
}

test('V3 outline, results, node card, and GraphView render cleanly without verified word', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = {};
  const mv = sampleMapview();

  // Outline
  assert.equal(formatReadMinutes(1.25), '1 min');
  assert.equal(formatReadMinutes(0.2), '< 1 min');
  const tree = buildOutlineTree(mv);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].result_count, 2);

  const outlineHost = dom.makeEl('div');
  let selected = null;
  const outlineCtrl = renderInvestigationOutline(outlineHost, mv, {
    onSelectNode: (id) => { selected = id; },
  });
  assert.ok(outlineHost.textContent.includes('Structure read from TeX source, v1'));
  const secBtn = outlineHost.querySelector('.inv-outline-btn');
  assert.ok(secBtn);
  secBtn.click();
  assert.equal(selected, 'n-sec0000001');
  outlineCtrl.setSelected('n-sec0000001');
  assert.equal(outlineCtrl.getSelected(), 'n-sec0000001');

  // Results list
  assert.equal(filterResultNodes(mv.nodes, 'results').length, 2);
  assert.equal(filterResultNodes(mv.nodes, 'definitions').length, 1);
  const resHost = dom.makeEl('div');
  const resCtrl = renderInvestigationResults(resHost, mv, {
    nodeTexts: { 'n-lem0000003': { text: 'For all $f,g$, $\\langle f, L g \\rangle \\ge 0$.' } },
    onSelectNode: (id) => { selected = id; },
  });
  assert.ok(resHost.textContent.includes('Lemma 1.2'));
  assert.ok(resHost.textContent.includes('cites [keller2020] in its heading'));
  assert.ok(resHost.textContent.includes('Statement uses: 1'));
  resCtrl.destroy();

  // Node card Block A on F6 shell
  const rel = collectNodeRelations(mv, 'n-thm0000005');
  assert.equal(rel.statementUses.length, 1);
  assert.equal(rel.statementUses[0].dashed, false);
  assert.equal(rel.proofUses.length, 1);
  assert.equal(rel.proofUses[0].dashed, true);

  const nodeHost = dom.makeEl('div');
  let clickedAction = null;
  let clickedCite = null;
  const cardCtrl = renderInvestigationNodeCard(nodeHost, mv, 'n-lem0000003', {
    nodeText: 'Dirichlet identity holds.',
    proofText: 'Swap $u$ and $v$.',
    hasPdf: false,
    modelConnected: false,
    onSelectNode: (id) => { selected = id; },
    onSelectCitation: (c) => { clickedCite = c.key; },
    onAction: (act) => { clickedAction = act; },
  });
  assert.ok(cardCtrl.card.querySelector('[data-block="code"]'));
  assert.ok(cardCtrl.card.querySelector('[data-block="model"]'));
  assert.ok(nodeHost.textContent.includes(NODE_NO_MODEL_TEXT));
  assert.ok(nodeHost.textContent.includes('keller2020'));
  assert.ok(nodeHost.textContent.includes('Proposition 2.1'));
  const citeBtn = nodeHost.querySelector('.inv-chip--cite');
  assert.ok(citeBtn);
  citeBtn.click();
  assert.equal(clickedCite, 'keller2020');
  const deriveBtn = nodeHost.querySelectorAll('.inv-node-action-btn').find((b) => b.textContent.includes('Derive in Solve'));
  assert.ok(deriveBtn);
  deriveBtn.click();
  assert.equal(clickedAction, 'derive_solve');

  // Shared F6 GraphView mounts the V3 mapview directly
  const graphHost = dom.makeEl('div');
  createGraphView(graphHost, mv);
  assert.ok(graphHost.querySelector('svg'));

  // Strictly no "verified" anywhere on screen in Investigate mode.
  for (const host of [outlineHost, resHost, nodeHost, graphHost]) {
    const allText = collectAllStrings(host).join('\n').toLowerCase();
    assert.equal(allText.includes('verified'), false, `forbidden word "verified" found in UI output`);
  }
  assert.equal(dom.innerHtmlWrites, 0, 'no innerHTML writes without KaTeX');
});

test('V3 physics fallback and XSS payload escaping', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = {};

  const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
  const physMv = {
    quality: 'partial',
    quality_line: `Structure read from TeX source (partial) — ${xss}`,
    nodes: [
      {
        id: 'n-sec1111111', kind: 'section', label: `Section 1: ${xss}`, group: 'Section 1',
        parent: null, grade: 'code', badges: [{ text: 'By code', icon: 'code', tone: 'ok' }],
        flags: [xss], stale: false, dim: false, order: 0, number: '1', title: xss,
        spans: [{ path: 'main.tex', start_line: 1, end_line: 10 }], head_cites: [], cites: [],
        words: 80, eq_count: 1, read_minutes: 0.7,
      },
      {
        id: 'n-eq11111111', kind: 'equation', label: 'Eq. (1)', group: 'Section 1',
        parent: 'n-sec1111111', grade: 'code', badges: [{ text: 'By code', icon: 'code', tone: 'ok' }],
        flags: [], stale: false, dim: false, order: 1, number: '1', title: null,
        spans: [{ path: 'main.tex', start_line: 5, end_line: 7 }], head_cites: [], cites: [],
        words: 10, eq_count: 0, read_minutes: 0.3,
      },
    ],
    edges: [],
    stats: [{ section_tag: 'n-sec1111111', words: 80, read_minutes: 0.7, counts: { equation: 1 } }],
    bib: [],
  };

  const banner = createQualityBanner(physMv);
  assert.ok(banner.textContent.includes(xss));
  assert.equal(banner.querySelectorAll('script').length, 0);
  assert.equal(banner.querySelectorAll('img').length, 0);

  const resHost = dom.makeEl('div');
  renderInvestigationResults(resHost, physMv, {
    nodeTexts: { 'n-eq11111111': { text: xss } },
  });
  assert.ok(resHost.textContent.includes(PHYSICS_EMPTY_NOTE));
  assert.ok(resHost.textContent.includes('Equation 1') || resHost.textContent.includes('Eq. (1)'));
  assert.equal(resHost.querySelectorAll('script').length, 0);
  assert.equal(resHost.querySelectorAll('img').length, 0);
  assert.equal(dom.innerHtmlWrites, 0);
});
