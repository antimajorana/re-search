import assert from 'node:assert/strict';
import test from 'node:test';

import {
  BUDGET_EXCEEDED_CODE,
  CITED_STATUSES,
  createInvestigationContextDrawer,
  openCitedLookup,
} from '../assets/investigation_context.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    return {
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
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
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
  };
  const prevDoc = globalThis.document;
  const prevWin = globalThis.window;
  globalThis.document = {
    createElement: (t) => makeEl(t, null),
    createElementNS: (ns, t) => makeEl(t, ns),
  };
  globalThis.window = {};
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() {
      globalThis.document = prevDoc;
      globalThis.window = prevWin;
    },
  };
}

test('rapid openCitedLookup discards slower older response and closes stale overlay/returnRef', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    let resolveFirst;
    let resolveSecond;
    let call = 0;
    const drawer = createInvestigationContextDrawer(host, {
      onLookup: () => new Promise((resolve) => {
        call += 1;
        if (call === 1) resolveFirst = resolve;
        else resolveSecond = resolve;
      }),
    });

    drawer.renderResult({
      status: CITED_STATUSES.found,
      bib_key: 'Old',
      short_ref: 'Old 2020',
      return_ref: { session: 's1', id: 'old-tag' },
      statement: { tag: 'n-old', text: 'Old statement' },
      cited_view: { selected_tag: 'n-old', mapview: { nodes: [{ id: 'n-old', kind: 'theorem', label: 'Old', spans: [] }], edges: [], bib: [] } },
    });
    drawer.openCitedOverlay(drawer.getState().activeResult);
    assert.equal(drawer.getState().overlayOpen, true);

    const p1 = openCitedLookup(drawer, { tag: 'n-1', bibKey: 'SlowRef', locator: 'Theorem 1' });
    assert.equal(drawer.getState().overlayOpen, false, 'new lookup must close open overlay');
    assert.equal(drawer.getState().returnRef, null, 'new lookup must clear stale returnRef');

    const p2 = openCitedLookup(drawer, { tag: 'n-1', bibKey: 'FastRef', locator: 'Lemma 2' });
    resolveSecond({
      status: CITED_STATUSES.found,
      bib_key: 'FastRef',
      short_ref: 'Fast 2026',
      statement: { tag: 'n-fast', text: 'FAST_RESULT_TEXT' },
    });
    await p2;
    resolveFirst({
      status: CITED_STATUSES.found,
      bib_key: 'SlowRef',
      short_ref: 'Slow 2026',
      statement: { tag: 'n-slow', text: 'STALE_SLOW_RESULT_TEXT' },
    });
    await p1;

    assert.ok(host.textContent.includes('FAST_RESULT_TEXT'));
    assert.equal(host.textContent.includes('STALE_SLOW_RESULT_TEXT'), false);
    assert.equal(drawer.getState().activeResult.bib_key, 'FastRef');
  } finally {
    dom.restore();
  }
});

test('raise_budget does not optimistically bump on failure and clears stale 429 card on success', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    let shouldFail = true;
    const drawer = createInvestigationContextDrawer(host, {
      fetchBudget: { used: 8, limit: 8 },
      onRaiseBudget: async () => {
        if (shouldFail) throw new Error('Network unavailable');
        return { fetch_budget: { used: 8, limit: 16 } };
      },
    });

    drawer.renderResult({ code: BUDGET_EXCEEDED_CODE, fetch_budget: { used: 8, limit: 8 } });
    assert.ok(host.querySelector(`[data-status="${BUDGET_EXCEEDED_CODE}"]`));

    // First click fails: limit MUST stay at 8 and an error card is rendered without unhandled rejection.
    host.querySelector('.inv-context-raise-btn').click();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(drawer.getState().fetchBudget.limit, 8);
    assert.ok(host.querySelector('[data-status="error"]'));

    // Re-show budget exceeded card, then succeed: limit becomes 16 and stale 429 card is cleared.
    shouldFail = false;
    drawer.renderResult({ code: BUDGET_EXCEEDED_CODE, fetch_budget: { used: 8, limit: 8 } });
    assert.ok(host.querySelector(`[data-status="${BUDGET_EXCEEDED_CODE}"]`));
    host.querySelector('.inv-context-raise-btn').click();
    await new Promise((r) => setTimeout(r, 0));
    assert.equal(drawer.getState().fetchBudget.limit, 16);
    assert.equal(host.querySelector(`[data-status="${BUDGET_EXCEEDED_CODE}"]`), null);
  } finally {
    dom.restore();
  }
});

test('open-new-session button omitted without callback; active KaTeX stub renders math safely without XSS', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const katexArgs = [];
    const fakeKatex = {
      renderToString(tex, opts) {
        katexArgs.push({ tex, trust: opts?.trust });
        return `<span class="katex-ok">${tex.replace(/[<>&]/g, '')}</span>`;
      },
    };
    const drawer = createInvestigationContextDrawer(host, { katex: fakeKatex });

    const xss = '<img src=x onerror=alert(1)>';
    drawer.renderResult({
      status: CITED_STATUSES.found,
      bib_key: 'K26',
      short_ref: `Ref ${xss}`,
      resolved: { key: 'arxiv:2601.00055' },
      statement: { tag: 'n-1', text: `Safe prose ${xss} and math $x^2 + 1$`, ocr: false, render_math: true },
    });

    assert.equal(host.querySelector('[data-action="open-new-session"]'), null, 'no dead button without callback');
    assert.equal(katexArgs.length, 1);
    assert.equal(katexArgs[0].tex, 'x^2 + 1');
    assert.ok(!katexArgs[0].trust, 'KaTeX trust option must not be enabled');
    // Only the single statement element with math wrote innerHTML, and prose XSS was HTML-escaped.
    assert.equal(dom.innerHtmlWrites, 1);
    const stmtHtml = host.querySelector('.inv-cited-statement').innerHTML;
    assert.ok(stmtHtml.includes('&lt;img src=x onerror=alert(1)&gt;'), 'prose XSS must be escaped');
    assert.equal(stmtHtml.includes('<img'), false, 'raw HTML tag must never appear in innerHTML');
    assert.equal(host.querySelectorAll('img').length, 0);

    // Top-level ocr=true fallback suppresses KaTeX even when statement omits ocr.
    drawer.renderResult({
      status: CITED_STATUSES.found,
      bib_key: 'Scan26',
      short_ref: 'Scan 26',
      ocr: true,
      statement: { tag: 'n-2', text: 'OCR formula $a + b$' },
    });
    assert.equal(katexArgs.length, 1, 'top-level ocr=true must suppress KaTeX');
  } finally {
    dom.restore();
  }
});
