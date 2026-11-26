import assert from 'node:assert/strict';
import test from 'node:test';

import { createGraphView } from '../assets/graph_view.js';
import {
  BORDER_DASHED,
  BORDER_SOLID,
  createBadge,
  createNodeCardShell,
  createSplitBadge,
  describeEvidenceGrade,
  formatModelText,
} from '../assets/ui_badge.js';
import {
  RUN_STATES,
  createRunBar,
  formatElapsed,
  formatModelCallBudget,
  formatRunSummary,
  normalizeRunState,
} from '../assets/run_bar.js';
import {
  MAX_QUOTE_CHARS,
  clampQuoteText,
  createQuoteChip,
} from '../assets/quote_chip.js';

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
      placeholder: '',
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
      getBoundingClientRect() { return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight }; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const attr = sel.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (attr) {
            const val = n.getAttribute(attr[1]);
            return attr[2] === undefined ? val !== null : val === attr[2];
          }
          return n.tagName === sel.toUpperCase();
        };
        const visit = (n) => {
          for (const c of n.children || []) {
            if (match(c)) out.push(c);
            visit(c);
          }
        };
        visit(this);
        return out;
      },
    };
    return el;
  };
  const prevDoc = globalThis.document;
  globalThis.document = {
    createElement: (tag) => makeEl(tag, null),
    createElementNS: (ns, tag) => makeEl(tag, ns),
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function walkTree(node, fn) {
  fn(node);
  for (const child of node.children || []) walkTree(child, fn);
}

test('ui_badge always pairs text with icon, enforces evidence grades, and builds two-block node cards', () => {
  const dom = makeFakeDom();
  try {
    // Even an empty badge spec gets fallback text AND fallback SVG icon (never colour alone).
    const fallback = createBadge({ tone: 'ok' });
    assert.ok(fallback.querySelector('.ui-badge-icon')?.querySelector('svg'));
    assert.ok(fallback.querySelector('.ui-badge-text').textContent.length > 0);

    // Evidence scale borders (overview 2.10).
    assert.equal(describeEvidenceGrade('code').border, BORDER_SOLID);
    assert.equal(describeEvidenceGrade('quote_fulltext').border, BORDER_SOLID);
    assert.equal(describeEvidenceGrade('quote_ocr').border, BORDER_SOLID);
    assert.equal(describeEvidenceGrade('quote_abstract').border, BORDER_DASHED);
    assert.equal(describeEvidenceGrade('model').border, BORDER_DASHED);
    assert.equal(describeEvidenceGrade('user').border, BORDER_SOLID);
    assert.equal(describeEvidenceGrade('unknown_future_grade').border, BORDER_DASHED);

    // Split badge keeps two distinct half-badges.
    const split = createSplitBadge(
      { text: 'Authors: New (model)', icon: 'model', tone: 'info' },
      { text: 'Earlier work: search wider in Explore', icon: 'link', tone: 'neutral' },
    );
    assert.equal(split.querySelectorAll('.ui-badge').length, 2);

    // Model text formatting follows overview 3.2 row 24.
    assert.equal(formatModelText('Main result'), 'Main result (model)');
    assert.equal(formatModelText('earlier statement', { isVerdict: true }), 'Model judges: earlier statement');
    assert.equal(formatModelText('Model agreed', { isVerdict: true }), 'Model agreed');

    // Node card shell has code block on top and model block below.
    const card = createNodeCardShell({
      title: 'Theorem 3.1',
      tag: 'n-thm0000006',
      grade: 'code',
      provenance: 'outside_paper',
      provenanceRef: '[Toy98]',
      codeBlock: { text: 'Verbatim statement from source.' },
      modelBlock: { text: '' },
    });
    const blocks = card.querySelectorAll('.ui-node-card-block');
    assert.equal(blocks.length, 2);
    assert.equal(blocks[0].getAttribute('data-block'), 'code');
    assert.equal(blocks[0].getAttribute('data-border'), BORDER_SOLID);
    assert.equal(blocks[1].getAttribute('data-block'), 'model');
    assert.equal(blocks[1].getAttribute('data-border'), BORDER_DASHED);
    assert.match(blocks[1].textContent, /Connect a model to get an explanation/);
  } finally {
    dom.restore();
  }
});

test('run_bar renders all seven states of overview 3.3, elapsed time, call budget, and controls', () => {
  const dom = makeFakeDom();
  try {
    assert.deepEqual(Object.keys(RUN_STATES), [
      'running', 'waiting', 'paused', 'interrupted', 'failed', 'done', 'done_open_items',
    ]);
    assert.equal(formatElapsed(134000), '02:14');
    assert.equal(formatElapsed(3902, { unit: 's' }), '1:05:02');
    assert.equal(formatModelCallBudget(7, 24), '7 of at most 24 model calls');
    assert.equal(normalizeRunState('partial').key, 'paused');
    assert.equal(normalizeRunState('stopped_at_cap').key, 'paused');
    assert.equal(normalizeRunState('transport_error').key, 'interrupted');
    assert.equal(normalizeRunState('finished_open_items').key, 'done_open_items');

    const host = dom.makeEl('div');
    const calls = [];
    const callbacks = {
      onStop: () => calls.push('stop'),
      onContinue: () => calls.push('continue'),
      onRetry: () => calls.push('retry'),
    };
    const bar = createRunBar(host, {
      state: 'running',
      detail: 'Step 5: checking series',
      elapsedMs: 134000,
      modelCallsUsed: 7,
      modelCallsMax: 24,
    }, callbacks);
    assert.equal(bar.element.getAttribute('data-run-state'), 'running');
    assert.match(bar.element.textContent, /Running/);
    assert.match(bar.element.textContent, /02:14/);
    assert.match(bar.element.textContent, /7 of at most 24 model calls/);
    bar.element.querySelector('[data-run-action="stop"]').click();
    assert.deepEqual(calls, ['stop']);

    bar.update({ state: 'waiting', detail: 'Approval required' });
    assert.match(bar.element.textContent, /Waiting for you/);
    assert.ok(bar.element.querySelector('[data-run-action="stop"]'));

    // "Partial" is never a state word: maps to Paused with reason + Continue.
    bar.update({ state: 'partial', reason: '3 results annotated' });
    assert.equal(bar.element.getAttribute('data-run-state'), 'paused');
    assert.match(bar.element.textContent, /Paused: 3 results annotated/);
    assert.doesNotMatch(bar.element.textContent, /\bPartial\b/i);
    bar.element.querySelector('[data-run-action="continue"]').click();
    assert.deepEqual(calls, ['stop', 'continue']);

    bar.update({ state: 'interrupted', reason: '' });
    assert.match(bar.element.textContent, /Interrupted/);
    bar.element.querySelector('[data-run-action="retry"]').click();
    assert.deepEqual(calls, ['stop', 'continue', 'retry']);

    bar.update({ state: 'failed', reason: 'Provider timeout' });
    assert.match(bar.element.textContent, /Failed: Provider timeout/);
    assert.ok(bar.element.querySelector('[data-run-action="retry"]'));

    bar.update({ state: 'done', reason: '', detail: '' });
    assert.match(bar.element.textContent, /Done/);
    assert.equal(bar.element.querySelectorAll('button').length, 0);

    bar.update({ state: 'done_open_items', openItems: ['2 steps unchecked', '1 missing source'] });
    assert.match(bar.element.textContent, /Done with open items/);
    assert.equal(bar.element.querySelectorAll('li').length, 2);

    assert.match(
      formatRunSummary({ state: 'running', elapsedMs: 60000, modelCallsUsed: 3, modelCallsMax: 8 }),
      /Running · 01:00 · 3 of at most 8 model calls/,
    );
  } finally {
    dom.restore();
  }
});

test('quote_chip clamps to 300 chars, distinguishes fulltext/OCR/abstract, and opens source', () => {
  const dom = makeFakeDom();
  try {
    const longQuote = 'A'.repeat(420);
    const clamped = clampQuoteText(longQuote);
    assert.equal(clamped.length, MAX_QUOTE_CHARS);
    assert.ok(clamped.endsWith('…'));

    const opened = [];
    const fullChip = createQuoteChip({
      quote: longQuote,
      paper: { label: 'arXiv:0000.00001' },
      source_id: 'src-1',
      path: 'main.tex',
      start_line: 212,
      end_line: 219,
      page: 7,
      evidence_id: 'xe-12',
    }, { onOpen: (target) => opened.push(target) });

    assert.equal(fullChip.getAttribute('data-border'), BORDER_SOLID);
    assert.equal(fullChip.querySelector('blockquote').textContent.length, 300);
    assert.match(fullChip.textContent, /matched, p\. 7/);
    assert.doesNotMatch(fullChip.textContent, /\bverified\b/i);
    assert.match(fullChip.textContent, /arXiv:0000\.00001 · lines 212–219 · p\. 7/);
    fullChip.querySelector('.ui-quote-chip-open').click();
    assert.equal(opened.length, 1);
    assert.equal(opened[0].start_line, 212);
    assert.equal(opened[0].end_line, 219);
    assert.equal(opened[0].page, 7);
    assert.equal(opened[0].openPageImage, false);

    // OCR quote: tagged OCR, opens page image.
    const ocrOpened = [];
    const ocrChip = createQuoteChip({
      quote: 'Scanned formula line.',
      paper: 'Old Journal 1972',
      source_id: 'src-2',
      path: 'scan.txt',
      start_line: 14,
      end_line: 15,
      page: 4,
      ocr: true,
    }, { onOpenPageImage: (target) => ocrOpened.push(target) });
    assert.equal(ocrChip.getAttribute('data-border'), BORDER_SOLID);
    assert.equal(ocrChip.getAttribute('data-ocr'), 'true');
    assert.match(ocrChip.textContent, /matched in OCR text, p\. 4/);
    ocrChip.querySelector('.ui-quote-chip-open').click();
    assert.equal(ocrOpened.length, 1);
    assert.equal(ocrOpened[0].openPageImage, true);

    // Abstract-only quote: dashed border.
    const absChip = createQuoteChip({
      quote: 'Abstract sentence.',
      paper: 'Preprint 2026',
      abstract_only: true,
    });
    assert.equal(absChip.getAttribute('data-border'), BORDER_DASHED);
    assert.match(absChip.textContent, /abstract only/);
  } finally {
    dom.restore();
  }
});

test('XSS payloads through every F6 component create zero injected elements and zero innerHTML writes', () => {
  const dom = makeFakeDom();
  try {
    const xss1 = '<img src=x onerror=alert(1)>';
    const xss2 = '<script>alert("xss")</script>';
    const xss3 = '<svg onload=alert(3)><a href="javascript:alert(4)">click</a></svg>';
    const roots = [];

    roots.push(createBadge({ text: xss1, icon: xss2, tone: xss3, secondaryText: xss2, title: xss1 }));
    roots.push(createSplitBadge({ text: xss1, icon: 'model' }, { text: xss3, icon: 'link' }));
    roots.push(createNodeCardShell({
      title: xss1,
      tag: xss2,
      grade: 'model',
      provenance: 'outside_paper',
      provenanceRef: xss3,
      badges: [{ text: xss1, icon: xss2, tone: xss3 }],
      flags: [xss1, xss2],
      codeBlock: { label: xss2, text: xss1 },
      modelBlock: { label: xss3, text: xss2 },
    }));

    const runHost = dom.makeEl('div');
    createRunBar(runHost, {
      state: 'done_open_items',
      reason: xss1,
      detail: xss2,
      extras: [xss3],
      openItems: [xss1, xss2, xss3],
      elapsedMs: 5000,
      modelCallsUsed: 2,
      modelCallsMax: 8,
    });
    roots.push(runHost);

    roots.push(createQuoteChip({
      quote: `${xss1} ${xss2} ${xss3}`,
      paper: xss1,
      source_id: xss2,
      path: xss3,
      start_line: 1,
      end_line: 2,
      page: 3,
      evidence_id: xss2,
      ocr: true,
    }));

    const graphHost = dom.makeEl('div');
    createGraphView(graphHost, {
      nodes: [
        {
          id: xss1, kind: xss2, label: xss1, group: xss3, grade: 'model',
          badges: [{ text: xss2, icon: xss1, tone: xss3 }], flags: [xss3], stale: true, dim: false,
        },
        {
          id: 'safe-dst', kind: 'theorem', label: xss2, group: xss1, grade: 'code',
          badges: [{ text: xss3, icon: 'code', tone: 'ok' }], flags: [xss1], stale: false, dim: true,
        },
      ],
      edges: [{ src: xss1, dst: 'safe-dst', kind: xss2, role: xss3, basis: 'model' }],
      legend: [xss1, { label: xss2, icon: xss3, tone: xss1 }],
    });
    roots.push(graphHost);

    assert.equal(dom.innerHtmlWrites, 0, 'F6 components must never write innerHTML');
    const forbiddenTags = new Set(['IMG', 'SCRIPT', 'IFRAME', 'A', 'OBJECT', 'EMBED']);
    for (const root of roots) {
      walkTree(root, (el) => {
        assert.equal(forbiddenTags.has(el.tagName), false, `unexpected injected tag <${el.tagName}>`);
        for (const [attrName, attrVal] of el.attributes.entries()) {
          assert.equal(/^on/i.test(attrName), false, `unexpected event attribute ${attrName}`);
          assert.equal(/^\s*javascript:/i.test(attrVal), false, `unexpected javascript: URL in ${attrName}`);
        }
      });
    }
    assert.ok(roots.some((r) => r.textContent.includes(xss1)));
    assert.ok(roots.some((r) => r.textContent.includes(xss2)));
  } finally {
    dom.restore();
  }
});
