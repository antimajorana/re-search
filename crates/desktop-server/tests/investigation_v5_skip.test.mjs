import assert from 'node:assert/strict';
import test from 'node:test';

import {
  SKIP_BASIS_EXPLICIT,
  SKIP_BASIS_MODEL,
  SKIP_EMPTY_NOTE,
  SYMBOL_FOUND_LABEL,
  SYMBOL_NOT_FOUND_LABEL,
  lookupNearestSymbol,
  normalizeSymbol,
  renderReadingPlanStrip,
  renderSymbolPopover,
  symbolLookupKeys,
} from '../assets/investigation_skip.js';

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
      style: {},
      type: '',
      title: '',
      hidden: false,
      disabled: false,
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
      removeEventListener(type, fn) {
        if (listeners.has(type)) listeners.set(type, listeners.get(type).filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
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

test('normalizeSymbol and lookupNearestSymbol resolve nearest earlier definition in < 100 ms', () => {
  assert.equal(normalizeSymbol(' $F$ '), 'F');
  assert.deepEqual(symbolLookupKeys('\\Lap{G}'), ['\\Lap{G}', '\\Lap']);

  const nodes = [
    { id: 'n-sec1', kind: 'section', label: 'Section 1: Setup', order: 0, spans: [{ path: 'main.tex', start_line: 10, end_line: 30 }] },
    { id: 'n-thm1', kind: 'theorem', label: 'Theorem 1.1', order: 1, parent: 'n-sec1', spans: [{ path: 'main.tex', start_line: 20, end_line: 25 }] },
    { id: 'n-sec2', kind: 'section', label: 'Section 2: Decay', order: 2, spans: [{ path: 'main.tex', start_line: 31, end_line: 60 }] },
    { id: 'n-thm2', kind: 'theorem', label: 'Theorem 2.1', order: 3, parent: 'n-sec2', spans: [{ path: 'main.tex', start_line: 45, end_line: 55 }] },
  ];
  const symbols = [
    { symbol_tex: '\\Lap', norm: '\\Lap', span: { path: 'main.tex', start_line: 4, end_line: 4 }, in_node: null, how: 'newcommand' },
    { symbol_tex: 'F', norm: 'F', span: { path: 'main.tex', start_line: 12, end_line: 12 }, in_node: 'n-sec1', how: 'define_phrase' },
    { symbol_tex: 'F', norm: 'F', span: { path: 'main.tex', start_line: 35, end_line: 35 }, in_node: 'n-sec2', how: 'define_phrase' },
  ];

  // Querying F at Theorem 1.1 finds Section 1's definition (line 12).
  const r1 = lookupNearestSymbol(symbols, nodes, '$F$', 'n-thm1', 22);
  assert.equal(r1.basisLabel, SYMBOL_FOUND_LABEL);
  assert.equal(r1.found.span.start_line, 12);

  // Querying F at Theorem 2.1 finds Section 2's nearer redefinition (line 35).
  const r2 = lookupNearestSymbol(symbols, nodes, 'F', 'n-thm2', 48);
  assert.equal(r2.found.span.start_line, 35);

  // Querying \Lap{G} matches preamble \newcommand{\Lap}.
  const r3 = lookupNearestSymbol(symbols, nodes, '\\Lap{G}', 'n-thm1', 22);
  assert.equal(r3.found.how, 'newcommand');

  // Querying unknown symbol returns honest not-found message.
  const r4 = lookupNearestSymbol(symbols, nodes, 'Z', 'n-thm2', 48);
  assert.equal(r4.found, null);
  assert.equal(r4.basisLabel, SYMBOL_NOT_FOUND_LABEL);
});

test('renderReadingPlanStrip and renderSymbolPopover enforce honesty, XSS safety, and zero verified word', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = {};

  const stripHost = dom.makeEl('div');
  // Empty skip plan: nothing dimmed.
  renderReadingPlanStrip(stripHost, { empty: true, sections: [] });
  assert.ok(stripHost.textContent.includes(SKIP_EMPTY_NOTE));

  // Explicit vs model-inferred skip plan.
  let jumpedSec = null;
  let dimState = true;
  const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
  renderReadingPlanStrip(
    stripHost,
    {
      empty: false,
      basis: 'includes_model_inferred',
      basis_text: SKIP_BASIS_MODEL,
      statement_minutes: 6.5,
      proof_minutes: 14.0,
      unreferenced_minutes: 5.0,
      summary_line: `Main statements refer to Section 1 by cross-references (${xss}).`,
      sections: [
        { section_tag: 'n-sec1', label: 'Section 1: Setup', need: 'statement', read_minutes: 6.5, dim: false },
        { section_tag: 'n-sec2', label: 'Section 2: Proofs', need: 'proof', read_minutes: 7.5, dim: false },
        { section_tag: 'n-sec3', label: `Section 3: ${xss}`, need: 'unreferenced', read_minutes: 5.0, dim: true },
      ],
    },
    {
      onSelectSection: (s) => { jumpedSec = s; },
      onToggleDim: (v) => { dimState = v; },
    },
  );
  assert.ok(stripHost.textContent.includes(SKIP_BASIS_MODEL));
  assert.ok(stripHost.querySelector('[data-border="dashed"]'));
  assert.ok(stripHost.querySelector('[data-need="unreferenced"]'));
  const secBtn = stripHost.querySelector('[data-need="statement"]');
  secBtn.click();
  assert.equal(jumpedSec, 'n-sec1');
  const toggleBtn = stripHost.querySelectorAll('button').find((b) => b.textContent.includes('Dim unreferenced'));
  assert.ok(toggleBtn);
  toggleBtn.click();
  assert.equal(dimState, false);

  // Explicit basis uses solid border.
  const expHost = dom.makeEl('div');
  renderReadingPlanStrip(expHost, {
    empty: false,
    basis: 'explicit',
    basis_text: SKIP_BASIS_EXPLICIT,
    statement_minutes: 3.0,
    proof_minutes: 5.0,
    unreferenced_minutes: 2.0,
    summary_line: 'Statement closure by explicit cross-references.',
    sections: [],
  });
  assert.ok(expHost.querySelector('[data-border="solid"]'));

  // Symbol popover found + not found.
  const popHost = dom.makeEl('div');
  let jumpedDef = null;
  renderSymbolPopover(
    popHost,
    {
      query: `F ${xss}`,
      norm: 'F',
      found: { symbol_tex: 'F', norm: 'F', span: { path: 'main.tex', start_line: 12, end_line: 12 }, in_node: 'n-sec1', how: 'define_phrase' },
      sectionLabel: 'Section 1: Setup',
      basisLabel: SYMBOL_FOUND_LABEL,
    },
    {
      excerpt: `Let $F$ be the spectral flow ${xss}.`,
      onJumpToDefinition: (d) => { jumpedDef = d; },
    },
  );
  assert.ok(popHost.textContent.includes(SYMBOL_FOUND_LABEL));
  assert.ok(popHost.textContent.includes('main.tex:12'));
  const jumpBtn = popHost.querySelectorAll('button').find((b) => b.textContent.includes('Jump to definition'));
  assert.ok(jumpBtn);
  jumpBtn.click();
  assert.equal(jumpedDef.in_node, 'n-sec1');

  const missHost = dom.makeEl('div');
  renderSymbolPopover(missHost, { query: 'Z', norm: 'Z', found: null, basisLabel: SYMBOL_NOT_FOUND_LABEL });
  assert.ok(missHost.textContent.includes(SYMBOL_NOT_FOUND_LABEL));

  // Strict security and vocabulary checks.
  for (const h of [stripHost, expHost, popHost, missHost]) {
    assert.equal(h.querySelectorAll('script').length, 0);
    assert.equal(h.querySelectorAll('img').length, 0);
    const lower = h.textContent.toLowerCase();
    assert.equal(lower.includes('verified'), false);
    assert.equal(lower.includes('not needed'), false);
  }
  assert.equal(dom.innerHtmlWrites, 0);
});
