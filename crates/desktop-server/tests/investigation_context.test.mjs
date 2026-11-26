import assert from 'node:assert/strict';
import test from 'node:test';

import {
  ATTACH_FILE_LABEL,
  CITED_STATUSES,
  DEFAULT_FETCH_BUDGET_LIMIT,
  INSPECT_IN_CITED_LABEL,
  OPEN_NEW_SESSION_LABEL,
  OUTSIDE_PAPER_HEADER_PREFIX,
  RAISE_BUDGET_LABEL,
  RETURN_TO_CLAIM_LABEL,
  createInvestigationContextDrawer,
  formatFetchBudgetText,
  openCitedLookup,
} from '../assets/investigation_context.js';
import { BORDER_SOLID, OCR_TAG_WORD } from '../assets/ui_badge.js';

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
    return el;
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

function collectAllStrings(el, out = []) {
  if (!el) return out;
  if (el.textContent) out.push(el.textContent);
  if (el.innerHTML) out.push(el.innerHTML);
  if (el.title) out.push(el.title);
  for (const [, v] of el.attributes || []) out.push(v);
  for (const c of el.children || []) collectAllStrings(c, out);
  return out;
}

function assertNoVerifiedWord(el, label = 'DOM') {
  const all = collectAllStrings(el).join('\n').toLowerCase();
  assert.equal(all.includes('verified'), false, `forbidden word "verified" found in ${label}`);
}

function sampleCitedView() {
  return {
    selected_tag: 'n-thm0000002',
    quality_line: 'Structure read from TeX source, v2',
    mapview: {
      source_id: 'ext-src-b',
      quality: 'full',
      quality_line: 'Structure read from TeX source, v2',
      nodes: [
        {
          id: 'n-def0000001', kind: 'definition', label: 'Definition 1.1: Spectral gap',
          spans: [{ path: 'paperB.tex', start_line: 10, end_line: 18 }], head_cites: [], cites: [],
        },
        {
          id: 'n-thm0000002', kind: 'theorem', label: 'Theorem 2.1: Cheeger inequality',
          spans: [{ path: 'paperB.tex', start_line: 40, end_line: 55 }],
          proof_tag: 'n-prf0000003', head_cites: [], cites: [],
        },
        {
          id: 'n-prf0000003', kind: 'proof', label: 'Proof',
          spans: [{ path: 'paperB.tex', start_line: 56, end_line: 80 }], head_cites: [], cites: [],
        },
      ],
      edges: [
        { src: 'n-def0000001', dst: 'n-thm0000002', kind: 'uses', role: 'statement', basis: 'code', rule: 'ref' },
      ],
      bib: [],
    },
  };
}

test('constants, budget counter formatting, and all 5 cited statuses render cleanly', async () => {
  const dom = makeFakeDom();
  try {
    assert.deepEqual(Object.values(CITED_STATUSES), [
      'found', 'candidates', 'paper_found_locator_not_found', 'no_tex', 'unresolved',
    ]);
    assert.equal(OUTSIDE_PAPER_HEADER_PREFIX, 'Outside this paper');
    assert.equal(DEFAULT_FETCH_BUDGET_LIMIT, 8);
    assert.equal(formatFetchBudgetText(3, 8), '3 / 8 external papers fetched');

    const host = dom.makeEl('div');
    const attached = [];
    const openedSources = [];
    const openedNew = [];
    const drawer = createInvestigationContextDrawer(host, {
      fetchBudget: { used: 3, limit: 8 },
      onOpenSource: (target) => openedSources.push(target),
      onAttachFile: (res) => attached.push(res.status),
      onOpenNewSession: (res) => openedNew.push(res.resolved?.key),
    });
    assert.ok(host.textContent.includes('3 / 8 external papers fetched'));

    // 1. found
    const foundRes = {
      status: CITED_STATUSES.found,
      tag: 'n-lem0000003',
      bib_key: 'keller2020',
      locator: 'Theorem 2.1',
      short_ref: 'Keller 2020 (arXiv:2001.01234v2)',
      resolved: { key: 'arxiv:2001.01234', version: 'v2' },
      external_source_id: 'ext-src-b',
      external_tag: 'n-thm0000002',
      version_caveat: 'numbering taken from arXiv v2; the journal version may differ',
      statement: {
        tag: 'n-thm0000002',
        text: 'For every finite weighted graph, lambda_1 >= h^2 / 2.',
        proof_text: 'By coarea formula.',
        spans: [{ path: 'paperB.tex', start_line: 40, end_line: 55 }],
        start_page: 4,
        ocr: false,
        render_math: true,
      },
      cited_view: sampleCitedView(),
      return_ref: { session: 's-1', mode: 'investigation', id: 'n-lem0000003', paper: 'arxiv:2601.00001', node_fp: 'fp-1' },
    };
    const foundCard = drawer.renderResult(foundRes);
    assert.equal(foundCard.getAttribute('data-provenance'), 'outside_paper');
    assert.equal(foundCard.getAttribute('data-border'), BORDER_SOLID);
    assert.equal(foundCard.getAttribute('data-status'), 'found');
    assert.ok(foundCard.className.includes('inv-cited-card'));
    assert.ok(foundCard.className.includes('inv-cited-card--outside'));
    assert.ok(foundCard.textContent.includes('Outside this paper: Keller 2020 (arXiv:2001.01234v2)'));
    assert.ok(foundCard.textContent.includes('numbering taken from arXiv v2; the journal version may differ'));
    assert.ok(foundCard.textContent.includes('lambda_1 >= h^2 / 2'));
    assert.ok(foundCard.textContent.includes('p. 4'));
    assert.ok(foundCard.querySelector(`[data-action="inspect-cited"]`));
    assert.equal(foundCard.querySelector(`[data-action="inspect-cited"]`).textContent, INSPECT_IN_CITED_LABEL);
    foundCard.querySelector('.ui-quote-chip-open').click();
    assert.equal(openedSources.length, 1);
    assert.equal(openedSources[0].source_id, 'ext-src-b');
    foundCard.querySelector('[data-action="open-new-session"]').click();
    assert.deepEqual(openedNew, ['arxiv:2001.01234']);
    assertNoVerifiedWord(host, 'found');

    // 2. candidates
    const candCard = drawer.renderResult({
      status: CITED_STATUSES.candidates,
      tag: 'n-lem0000003',
      bib_key: 'smith1999',
      locator: 'Lemma 9',
      short_ref: 'Smith 1999',
      reason: 'PDF statement boundaries are estimated (rule 6.9)',
      version_caveat: 'numbering taken from source v1; the journal version may differ',
      external_source_id: 'ext-src-c',
      candidates: [
        {
          tag: 'n-lem-pdf-9',
          label: 'Lemma 9: Trace bound',
          spans: [{ path: 'scan.txt', start_line: 120, end_line: 130 }],
          start_page: 7,
          ocr: true,
          excerpt: 'Trace bound on page 7.',
        },
      ],
    });
    assert.equal(candCard.getAttribute('data-status'), 'candidates');
    assert.ok(candCard.textContent.includes('Outside this paper: Smith 1999'));
    assert.ok(candCard.textContent.includes('p. 7'));
    assert.ok(candCard.textContent.includes(OCR_TAG_WORD));
    assert.ok(candCard.textContent.includes('Trace bound on page 7.'));
    assertNoVerifiedWord(host, 'candidates');

    // 3. paper_found_locator_not_found
    const notFoundCard = drawer.renderResult({
      status: CITED_STATUSES.paper_found_locator_not_found,
      tag: 'n-lem0000003',
      bib_key: 'keller2020',
      locator: 'Theorem 99',
      short_ref: 'Keller 2020',
      reason: 'no statement matching `Theorem 99` was found in this paper',
      external_source_id: 'ext-src-b',
      candidates: [{ tag: 'n-thm0000002', label: 'Theorem 2.1: Cheeger inequality', spans: [{ path: 'paperB.tex', start_line: 40, end_line: 55 }] }],
    });
    assert.equal(notFoundCard.getAttribute('data-status'), 'paper_found_locator_not_found');
    assert.ok(notFoundCard.querySelector('[data-candidate-tag="n-thm0000002"]'));
    assertNoVerifiedWord(host, 'paper_found_locator_not_found');

    // 4a. no_tex (needs_file)
    const noTexNeedsFile = drawer.renderResult({
      status: CITED_STATUSES.no_tex,
      bib_key: 'paywall2024',
      short_ref: 'Paywall 2024',
      reason: 'needs_file',
      bib: { key: 'paywall2024', title: 'Closed Journal Paper', doi: '10.1000/closed', raw: 'Paywall, Closed Journal Paper, 2024.' },
    });
    assert.equal(noTexNeedsFile.getAttribute('data-status'), 'no_tex');
    assert.ok(noTexNeedsFile.textContent.includes('Closed Journal Paper'));
    assert.ok(noTexNeedsFile.textContent.includes('needs_file'));
    const attach1 = noTexNeedsFile.querySelector('[data-action="attach-file"]');
    assert.equal(attach1.textContent, ATTACH_FILE_LABEL);
    attach1.click();

    // 4b. no_tex (empty_text_layer)
    const noTexEmpty = drawer.renderResult({
      status: CITED_STATUSES.no_tex,
      bib_key: 'scan1970',
      short_ref: 'Scan 1970',
      reason: 'empty_text_layer',
      bib: { key: 'scan1970', title: 'Old Scanned Volume', raw: 'Scan, 1970.' },
    });
    assert.ok(noTexEmpty.textContent.includes('empty_text_layer'));
    assertNoVerifiedWord(host, 'no_tex');

    // 5. unresolved
    const unresolvedCard = drawer.renderResult({
      status: CITED_STATUSES.unresolved,
      bib_key: 'anon',
      short_ref: 'anon',
      bib: { key: 'anon', title: 'Unpublished notes', raw: 'Author, Unpublished notes.' },
    });
    assert.equal(unresolvedCard.getAttribute('data-status'), 'unresolved');
    assert.ok(unresolvedCard.textContent.includes('Unpublished notes'));
    unresolvedCard.querySelector('[data-action="attach-file"]').click();
    assert.deepEqual(attached, ['no_tex', 'unresolved']);
    assertNoVerifiedWord(host, 'unresolved');
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('Rule 6.21 OCR text never invokes KaTeX math rendering even when katex is present', () => {
  const dom = makeFakeDom();
  try {
    let katexCalls = 0;
    const fakeKatex = {
      renderToString(tex) {
        katexCalls += 1;
        return `<span class="katex">${tex}</span>`;
      },
    };
    const host = dom.makeEl('div');
    const openedPdf = [];
    const drawer = createInvestigationContextDrawer(host, {
      katex: fakeKatex,
      onOpenPdf: (target) => openedPdf.push(target),
    });

    const card = drawer.renderResult({
      status: CITED_STATUSES.found,
      bib_key: 'scan1980',
      short_ref: 'Scan 1980',
      external_source_id: 'src-ocr',
      statement: {
        tag: 'n-ocr-1',
        text: 'For $x \\in \\mathbb{R}$, $x^2 \\ge 0$.',
        spans: [{ path: 'scan.txt', start_line: 12, end_line: 15 }],
        start_page: 9,
        ocr: true,
        render_math: false,
      },
    });

    assert.equal(katexCalls, 0, 'OCR statement must never invoke KaTeX');
    assert.equal(dom.innerHtmlWrites, 0, 'OCR statement must never write innerHTML');
    assert.ok(card.textContent.includes('$x \\in \\mathbb{R}$'));
    assert.ok(card.textContent.includes(OCR_TAG_WORD));
    assert.ok(card.textContent.includes('p. 9'));
    card.querySelector('.ui-quote-chip-open').click();
    assert.equal(openedPdf.length, 1);
    assert.equal(openedPdf[0].openPageImage, true);
    assert.equal(openedPdf[0].page, 9);
    assertNoVerifiedWord(host, 'ocr');
  } finally {
    dom.restore();
  }
});

test('candidate picker invokes onPick and transitions to found; openCitedLookup handles 429 budget & raise', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const lookupCalls = [];
    const pickCalls = [];
    const raiseCalls = [];

    let budgetLimit = 8;
    const drawer = createInvestigationContextDrawer(host, {
      fetchBudget: { used: 8, limit: 8 },
      onLookup: async (req) => {
        lookupCalls.push(req);
        if (budgetLimit <= 8) {
          const err = new Error('Fetch budget exceeded');
          err.status = 429;
          err.payload = { code: 'fetch_budget_exceeded', fetch_budget: { used: 8, limit: 8 } };
          throw err;
        }
        return {
          status: CITED_STATUSES.candidates,
          tag: req.tag,
          bib_key: req.bib_key,
          locator: req.locator,
          short_ref: 'Paper C',
          external_source_id: 'ext-src-c',
          reason: 'PDF statement boundaries are estimated (rule 6.9)',
          fetch_budget: { used: 9, limit: budgetLimit },
          candidates: [
            { tag: 'n-c-1', label: 'Theorem 1', start_page: 3, spans: [{ path: 'c.txt', start_line: 20, end_line: 25 }] },
          ],
        };
      },
      onPick: async (req) => {
        pickCalls.push(req);
        return {
          status: CITED_STATUSES.found,
          tag: req.tag,
          bib_key: req.bib_key,
          locator: req.locator,
          short_ref: 'Paper C',
          external_source_id: req.external_source_id,
          external_tag: req.external_tag,
          statement: { tag: req.external_tag, text: 'Picked theorem statement.', start_page: 3, ocr: false },
        };
      },
      onRaiseBudget: async (req) => {
        raiseCalls.push(req);
        budgetLimit = req.new_limit;
        return { fetch_budget: { used: 8, limit: req.new_limit } };
      },
    });

    await openCitedLookup(drawer, { tag: 'n-a-1', bibKey: 'paperC', locator: 'Theorem 1' });
    assert.equal(lookupCalls.length, 1);
    assert.equal(lookupCalls[0].action, 'lookup');
    assert.equal(host.querySelector('[data-status="fetch_budget_exceeded"]') != null, true);

    const raiseBtn = host.querySelector('[data-action="raise-budget"]');
    assert.equal(raiseBtn.textContent, RAISE_BUDGET_LABEL);
    raiseBtn.click();
    await new Promise((r) => setTimeout(r, 0));

    assert.equal(raiseCalls.length, 1);
    assert.deepEqual(raiseCalls[0], { action: 'raise_budget', new_limit: 16, tag: 'n-a-1' });
    assert.ok(host.textContent.includes('8 / 16 external papers fetched'));

    await openCitedLookup(drawer, { tag: 'n-a-1', bibKey: 'paperC', locator: 'Theorem 1' });
    assert.ok(host.textContent.includes('9 / 16 external papers fetched'));
    const candBtn = host.querySelector('[data-candidate-tag="n-c-1"]');
    assert.ok(candBtn);
    candBtn.click();
    await new Promise((r) => setTimeout(r, 0));

    assert.equal(pickCalls.length, 1);
    assert.deepEqual(pickCalls[0], {
      action: 'pick',
      tag: 'n-a-1',
      bib_key: 'paperC',
      locator: 'Theorem 1',
      external_source_id: 'ext-src-c',
      external_tag: 'n-c-1',
    });
    assert.ok(host.textContent.includes('Picked theorem statement.'));
    assert.equal(host.querySelector('[data-status="found"]') != null, true);
    assertNoVerifiedWord(host, 'budget-pick');
  } finally {
    dom.restore();
  }
});

test('in-session cited overlay inspects neighbor nodes and returns exact return_ref; V12 section seam coexists', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const inspectCalls = [];
    const returnedRefs = [];
    const originalRef = {
      session: 'sess-a',
      mode: 'investigation',
      id: 'n-lem0000003',
      paper: 'arxiv:2601.00001',
      node_fp: 'fp-lem-3',
    };

    const drawer = createInvestigationContextDrawer(host, {
      onInspectNode: async (req) => {
        inspectCalls.push(req);
        return {
          selected_tag: req.external_tag,
          statement: {
            tag: req.external_tag,
            text: 'A weighted graph has spectral gap lambda_1.',
            spans: [{ path: 'paperB.tex', start_line: 10, end_line: 18 }],
          },
        };
      },
      onReturnToClaim: (ref) => returnedRefs.push(ref),
    });

    const v12Card = dom.makeEl('div');
    v12Card.className = 'v12-model-context';
    v12Card.textContent = 'Model context card (model)';
    assert.equal(drawer.mountContextSection('v12-cards', v12Card), true);
    assert.ok(host.textContent.includes('Model context card (model)'));

    drawer.renderResult({
      status: CITED_STATUSES.found,
      tag: 'n-lem0000003',
      bib_key: 'keller2020',
      short_ref: 'Keller 2020',
      external_source_id: 'ext-src-b',
      external_tag: 'n-thm0000002',
      statement: {
        tag: 'n-thm0000002',
        text: 'Cheeger inequality holds.',
        proof_text: 'Proof text.',
      },
      cited_view: sampleCitedView(),
      return_ref: originalRef,
    });

    // V12 section still present alongside cited card
    assert.ok(host.querySelector('.v12-model-context'));

    host.querySelector('[data-action="inspect-cited"]').click();
    assert.equal(drawer.getState().overlayOpen, true);
    assert.ok(host.textContent.includes('Theorem 2.1: Cheeger inequality'));
    assert.ok(host.textContent.includes('Structure read from TeX source, v2'));
    assert.equal(host.querySelector('[data-action="return-to-claim"]').textContent, RETURN_TO_CLAIM_LABEL);

    // Click neighbor statement chip inside Paper B's focused node card
    const neighborBtn = host.querySelector('[data-node-id="n-def0000001"]');
    assert.ok(neighborBtn);
    neighborBtn.click();
    await new Promise((r) => setTimeout(r, 0));

    assert.equal(inspectCalls.length, 1);
    assert.deepEqual(inspectCalls[0], {
      action: 'inspect_node',
      external_source_id: 'ext-src-b',
      external_tag: 'n-def0000001',
    });
    assert.ok(host.textContent.includes('A weighted graph has spectral gap lambda_1.'));

    // Return to original claim restores exact NodeRef
    host.querySelector('[data-action="return-to-claim"]').click();
    assert.equal(drawer.getState().overlayOpen, false);
    assert.deepEqual(returnedRefs, [originalRef]);

    assert.equal(drawer.unmountContextSection('v12-cards'), true);
    assert.equal(drawer.getState().mountedSections.length, 0);
    assertNoVerifiedWord(host, 'overlay');
  } finally {
    dom.restore();
  }
});

test('XSS payloads in titles, statements, candidate labels, bib raw text, and locators never write innerHTML', () => {
  const dom = makeFakeDom();
  try {
    const xss = '<script>alert(1)</script><img src=x onerror=alert(2)>';
    const host = dom.makeEl('div');
    const drawer = createInvestigationContextDrawer(host);

    drawer.renderResult({
      status: CITED_STATUSES.found,
      bib_key: `key-${xss}`,
      locator: `Theorem 1 ${xss}`,
      short_ref: `Ref ${xss}`,
      external_source_id: 'ext-xss',
      version_caveat: `caveat ${xss}`,
      statement: { tag: 'n-1', text: `Statement ${xss}`, spans: [{ path: `${xss}.tex`, start_line: 1, end_line: 2 }], ocr: false },
      cited_view: {
        selected_tag: 'n-1',
        quality_line: `Quality ${xss}`,
        mapview: {
          nodes: [{ id: 'n-1', kind: 'theorem', label: `Title ${xss}`, spans: [] }],
          edges: [],
          bib: [],
        },
      },
    });
    drawer.openCitedOverlay(drawer.getState().activeResult);

    drawer.renderResult({
      status: CITED_STATUSES.candidates,
      bib_key: `key-${xss}`,
      locator: xss,
      short_ref: xss,
      reason: `Reason ${xss}`,
      candidates: [{ tag: 'c-1', label: `Cand ${xss}`, excerpt: `Excerpt ${xss}`, start_page: 2, ocr: true }],
    });

    drawer.renderResult({
      status: CITED_STATUSES.no_tex,
      bib_key: `key-${xss}`,
      short_ref: xss,
      reason: `Reason ${xss}`,
      bib: { key: xss, title: `Title ${xss}`, raw: `BibRaw ${xss}`, doi: `10.1000/${xss}` },
    });

    assert.equal(dom.innerHtmlWrites, 0, 'XSS payloads must trigger zero innerHTML writes');
    assert.equal(host.querySelectorAll('script').length, 0);
    assert.equal(host.querySelectorAll('img').length, 0);
    assert.ok(host.textContent.includes(xss));
    assertNoVerifiedWord(host, 'xss');
  } finally {
    dom.restore();
  }
});
