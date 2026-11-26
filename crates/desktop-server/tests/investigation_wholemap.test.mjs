import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { EDGE_DASH_ARRAY, EDGE_WEIGHTS } from '../assets/graph_view.js';
import {
  AUTHORS_CLAIM_OPTIONS,
  VIEW_MODES,
  WHOLEMAP_LABELS,
  createInvestigateWholeMap,
  createInvestigationWholeMap,
} from '../assets/investigation_wholemap.js';

const fixturePath = new URL('./fixtures/investigate_graph_view.json', import.meta.url);
const investigateFixture = JSON.parse(await readFile(fixturePath, 'utf8'));

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const windowListeners = new Map();

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
      isContentEditable: false,
      tabIndex: 0,
      clientWidth: 860,
      clientHeight: 540,
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
      parentNode: null,
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { for (const n of nodes) { if (n && typeof n === 'object') n.parentNode = this; this.children.push(n); } },
      appendChild(node) { if (node && typeof node === 'object') node.parentNode = this; this.children.push(node); return node; },
      replaceChildren(...nodes) {
        for (const c of this.children) if (c && typeof c === 'object') c.parentNode = null;
        this.children = [];
        for (const n of nodes) { if (n && typeof n === 'object') n.parentNode = this; this.children.push(n); }
        text = ''; html = '';
      },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      removeEventListener(type, fn) {
        if (listeners.has(type)) listeners.set(type, listeners.get(type).filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, stopPropagation() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      getBoundingClientRect() { return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight }; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const parts = String(sel || '').trim().match(/(?:\[[^\]]*\]|[^\s])+/g) || [];
        if (!parts.length) return [];
        const matchSimple = (n, s) => {
          if (!n || !n.tagName) return false;
          if (s === '*') return true;
          if (s.startsWith('.')) return n.classList.contains(s.slice(1));
          const m = /^\[([^=\]]+)(?:="([^"]*)")?\]$/.exec(s);
          if (m) return m[2] === undefined ? n.attributes.has(m[1]) : n.getAttribute(m[1]) === m[2];
          return n.tagName.toLowerCase() === s.toLowerCase();
        };
        const collectMatches = (roots, s) => {
          const res = [];
          const seen = new Set();
          const walk = (n) => {
            for (const c of n.children || []) {
              if (matchSimple(c, s) && !seen.has(c)) { seen.add(c); res.push(c); }
              walk(c);
            }
          };
          for (const r of roots) walk(r);
          return res;
        };
        let current = [this];
        for (const part of parts) current = collectMatches(current, part);
        return current;
      },
    };
    return el;
  };

  const fakeWindow = {
    addEventListener(type, fn) { (windowListeners.has(type) ? windowListeners : windowListeners.set(type, [])).get(type).push(fn); },
    removeEventListener(type, fn) {
      if (windowListeners.has(type)) windowListeners.set(type, windowListeners.get(type).filter((f) => f !== fn));
    },
    dispatchKey(key, extra = {}) {
      let prevented = false;
      const ev = { key, ctrlKey: false, metaKey: false, altKey: false, target: null, preventDefault() { prevented = true; }, ...extra };
      for (const fn of [...(windowListeners.get('keydown') || [])]) fn(ev);
      return prevented;
    },
    listenerCount(type) { return (windowListeners.get(type) || []).length; },
  };

  const prevDoc = globalThis.document;
  const prevWin = globalThis.window;
  globalThis.document = {
    createElement: (t) => makeEl(t, null),
    createElementNS: (ns, t) => makeEl(t, ns),
  };
  globalThis.window = fakeWindow;

  return {
    makeEl,
    window: fakeWindow,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; globalThis.window = prevWin; },
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

function makeLargeTwoSectionMapview() {
  const nodes = [
    { id: 'sec-1', kind: 'section', label: 'Section 1: Foundations', group: 'Section 1: Foundations', grade: 'code', badges: [], flags: [] },
    { id: 'sec-2', kind: 'section', label: 'Section 2: Applications', group: 'Section 2: Applications', grade: 'code', badges: [], flags: [] },
  ];
  const edges = [];
  for (let i = 0; i < 80; i++) {
    nodes.push({
      id: `s1-${i}`, kind: i === 79 ? 'theorem' : 'lemma', label: `Lemma 1.${i + 1}`,
      group: 'Section 1: Foundations', parent: 'sec-1', grade: 'code', badges: [], flags: i === 79 ? ['main_result'] : [],
    });
    if (i > 0) edges.push({ src: `s1-${i - 1}`, dst: `s1-${i}`, kind: 'uses', role: 'proof', basis: 'code' });
  }
  for (let i = 0; i < 80; i++) {
    nodes.push({
      id: `s2-${i}`, kind: i === 79 ? 'theorem' : 'lemma', label: `Lemma 2.${i + 1}`,
      group: 'Section 2: Applications', parent: 'sec-2', grade: 'code', badges: [], flags: [],
    });
    if (i > 0) edges.push({ src: `s2-${i - 1}`, dst: `s2-${i}`, kind: 'uses', role: 'proof', basis: 'code' });
  }
  edges.push({ src: 's1-79', dst: 's2-0', kind: 'uses', role: 'statement', basis: 'code' });
  edges.push({ src: 's1-78', dst: 's2-1', kind: 'uses', role: 'proof', basis: 'model' });
  return { nodes, edges, legend: [] };
}

test('focused view & Follow the argument: Statement uses, Proof uses, Used by, stepping, and breadcrumb restore', () => {
  assert.equal(createInvestigationWholeMap, createInvestigateWholeMap);
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const selectedLog = [];
    const ctrl = createInvestigateWholeMap(host, investigateFixture, {
      onSelectNode: (id) => selectedLog.push(id),
    });

    assert.equal(ctrl.getMode(), VIEW_MODES.RESULTS);
    assert.ok(ctrl.element.querySelector('[data-wm-mode="whole"]'));

    // Click Theorem 3.1 in paper-scale results view -> enters focused mode.
    ctrl.element.querySelector('.inv-wm-graph-host [data-node-id="n-thm0000006"]').click();
    assert.equal(ctrl.getMode(), VIEW_MODES.FOCUSED);
    assert.deepEqual(ctrl.getBreadcrumb(), ['n-thm0000006']);
    assert.equal(selectedLog.at(-1), 'n-thm0000006');

    // Quick Whole map (M) button is visible in focused mode.
    const focusWholeBtn = ctrl.element.querySelector('.inv-wm-focus-panel [data-wm-action="whole-map"]');
    assert.ok(focusWholeBtn);
    assert.equal(focusWholeBtn.textContent, WHOLEMAP_LABELS.modeWhole);

    // Three explicit directional columns + SVG edges.
    const stmtCol = ctrl.element.querySelector('[data-wm-rel="statement-uses"]');
    const proofCol = ctrl.element.querySelector('[data-wm-rel="proof-uses"]');
    const usedCol = ctrl.element.querySelector('[data-wm-rel="used-by"]');
    assert.deepEqual(stmtCol.querySelectorAll('.inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')), ['n-ass0000002']);
    assert.deepEqual(
      proofCol.querySelectorAll('.inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')),
      ['n-lem0000004', 'n-lem0000005', 'm-stp0000007'],
    );
    assert.deepEqual(usedCol.querySelectorAll('.inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')), ['n-cor0000008']);
    assert.equal(ctrl.element.querySelectorAll('.gv-edge').length, 5);

    // Step to n-lem0000004 in Proof uses -> updates focused neighborhood & breadcrumb.
    proofCol.querySelector('[data-node-id="n-lem0000004"]').click();
    assert.deepEqual(ctrl.getBreadcrumb(), ['n-thm0000006', 'n-lem0000004']);
    assert.deepEqual(
      ctrl.element.querySelectorAll('[data-wm-rel="statement-uses"] .inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')),
      ['n-def0000001'],
    );
    assert.deepEqual(
      ctrl.element.querySelectorAll('[data-wm-rel="proof-uses"] .inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')),
      ['n-eq00000003'],
    );
    assert.deepEqual(
      ctrl.element.querySelectorAll('[data-wm-rel="used-by"] .inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')),
      ['n-thm0000006'],
    );

    // Click first breadcrumb -> restores exact previous selection and truncates trail.
    ctrl.element.querySelector('[data-wm-crumb-index="0"]').click();
    assert.deepEqual(ctrl.getBreadcrumb(), ['n-thm0000006']);
    assert.equal(ctrl.getState().focusId, 'n-thm0000006');
    assert.equal(selectedLog.at(-1), 'n-thm0000006');

    ctrl.destroy();
    assert.equal(dom.window.listenerCount('keydown'), 0);
  } finally {
    dom.restore();
  }
});

test('whole map M shortcut ignores inputs, folds >150 nodes into 2 section boxes without merging code/model edges, and expands/refolds', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const largeMv = makeLargeTwoSectionMapview();
    const ctrl = createInvestigateWholeMap(host, { mapview: largeMv });
    assert.equal(ctrl.getMode(), VIEW_MODES.RESULTS);

    // Typing M inside INPUT or TEXTAREA must be ignored.
    dom.window.dispatchKey('m', { target: dom.makeEl('input') });
    dom.window.dispatchKey('M', { target: dom.makeEl('textarea') });
    assert.equal(ctrl.getMode(), VIEW_MODES.RESULTS);

    // Pressing M on window switches to whole mode and folds 160 nodes into 2 boxes.
    const prevented = dom.window.dispatchKey('m');
    assert.equal(prevented, true);
    assert.equal(ctrl.getMode(), VIEW_MODES.WHOLE);

    const foldedNodes = ctrl.element.querySelectorAll('.gv-node');
    assert.equal(foldedNodes.length, 2);
    assert.ok(foldedNodes.every((n) => n.getAttribute('data-folded') === 'true'));

    const foldedEdges = ctrl.element.querySelectorAll('.gv-edge');
    assert.equal(foldedEdges.length, 2);
    assert.ok(foldedEdges.some((e) => e.getAttribute('data-basis') === 'code' && e.getAttribute('data-dashed') === 'false'));
    assert.ok(foldedEdges.some((e) => e.getAttribute('data-basis') === 'model' && e.getAttribute('data-dashed') === 'true'));

    // Click folded Section 1 box -> expands Section 1 (80 + 1 folded box = 81 nodes).
    ctrl.element.querySelector('[data-node-id="group:Section 1: Foundations"]').click();
    assert.equal(ctrl.element.querySelectorAll('.gv-node').length, 81);
    assert.equal(ctrl.element.querySelector('[data-wm-section="Section 1: Foundations"]').getAttribute('data-collapsed'), 'false');

    // Click F6 refold button -> back to 2 boxes and synced section pills.
    ctrl.element.querySelector('[data-fold-action="refold"]').click();
    assert.equal(ctrl.element.querySelectorAll('.gv-node').length, 2);
    assert.equal(ctrl.element.querySelector('[data-wm-section="Section 1: Foundations"]').getAttribute('data-collapsed'), 'true');

    // Pressing M again toggles back to previous mode.
    dom.window.dispatchKey('M');
    assert.equal(ctrl.getMode(), VIEW_MODES.RESULTS);

    // Manual section collapse/expand on <150-node fixture.
    const smallHost = dom.makeEl('div');
    const smallCtrl = createInvestigateWholeMap(smallHost, investigateFixture, { initialMode: VIEW_MODES.WHOLE });
    assert.equal(smallCtrl.element.querySelectorAll('.gv-node').length, 9);
    smallCtrl.element.querySelector('[data-wm-section="Section 1: Setup"]').click();
    assert.equal(smallCtrl.element.querySelectorAll('.gv-node').length, 7);
    assert.equal(smallCtrl.element.querySelector('[data-wm-section="Section 1: Setup"]').getAttribute('data-collapsed'), 'true');
    smallCtrl.element.querySelector('[data-wm-section="Section 1: Setup"]').click();
    assert.equal(smallCtrl.element.querySelectorAll('.gv-node').length, 9);

    ctrl.destroy();
    smallCtrl.destroy();
  } finally {
    dom.restore();
  }
});

test('line style, stroke weight honesty, heuristic text_mention edges, and F6 filter toggles', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const mv = {
      nodes: [
        { id: 'n-def', kind: 'definition', label: 'Def 1', group: 'S1', grade: 'code', badges: [] },
        { id: 'n-lem', kind: 'lemma', label: 'Lem 2', group: 'S1', grade: 'quote_fulltext', badges: [] },
        { id: 'n-thm', kind: 'theorem', label: 'Thm 3', group: 'S2', grade: 'code', role: 'main_result', badges: [] },
      ],
      edges: [
        { src: 'n-def', dst: 'n-thm', kind: 'uses', role: 'statement', basis: 'code' },
        { src: 'n-lem', dst: 'n-thm', kind: 'uses', role: 'proof', basis: 'code' },
        { src: 'n-def', dst: 'n-lem', kind: 'uses', role: 'proof', basis: 'evidence' },
        { src: 'n-def', dst: 'n-thm', kind: 'uses', role: 'argument', basis: 'model' },
        { src: 'n-lem', dst: 'n-thm', kind: 'uses', role: 'prose', rule: 'text_mention', confidence: 'heuristic' },
      ],
      legend: [],
    };

    const ctrl = createInvestigateWholeMap(host, mv, { initialMode: VIEW_MODES.WHOLE });
    const edges = ctrl.element.querySelectorAll('.gv-edge');
    assert.equal(edges.length, 5);

    const stmtCode = edges.find((e) => e.getAttribute('data-role') === 'statement' && e.getAttribute('data-basis') === 'code');
    const proofCode = edges.find((e) => e.getAttribute('data-role') === 'proof' && e.getAttribute('data-basis') === 'code');
    const proofEvidence = edges.find((e) => e.getAttribute('data-role') === 'proof' && e.getAttribute('data-basis') === 'evidence');
    const modelEdge = edges.find((e) => e.getAttribute('data-role') === 'argument' && e.getAttribute('data-basis') === 'model');
    const heuristicEdge = edges.find((e) => e.getAttribute('data-role') === 'prose');

    assert.equal(stmtCode.getAttribute('data-dashed'), 'false');
    assert.equal(stmtCode.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), null);
    assert.equal(Number(stmtCode.getAttribute('data-stroke-width')), EDGE_WEIGHTS.statement);

    assert.equal(proofCode.getAttribute('data-dashed'), 'false');
    assert.equal(proofCode.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), null);
    assert.equal(Number(proofCode.getAttribute('data-stroke-width')), EDGE_WEIGHTS.proof);

    assert.equal(proofEvidence.getAttribute('data-dashed'), 'false');
    assert.equal(modelEdge.getAttribute('data-dashed'), 'true');
    assert.equal(modelEdge.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), EDGE_DASH_ARRAY);
    assert.equal(heuristicEdge.getAttribute('data-dashed'), 'true');
    assert.equal(heuristicEdge.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), EDGE_DASH_ARRAY);

    // Toggle Model links off -> hides both model and heuristic text_mention edges.
    ctrl.element.querySelector('[data-filter="model"]').click();
    assert.equal(ctrl.element.querySelectorAll('.gv-edge').length, 3);
    ctrl.element.querySelector('[data-filter="model"]').click();
    assert.equal(ctrl.element.querySelectorAll('.gv-edge').length, 5);

    // Toggle Statement uses and Proof uses independently.
    ctrl.element.querySelector('[data-filter="statement"]').click();
    assert.ok(ctrl.element.querySelectorAll('.gv-edge').every((e) => e.getAttribute('data-role') !== 'statement'));
    ctrl.element.querySelector('[data-filter="proof"]').click();
    assert.ok(ctrl.element.querySelectorAll('.gv-edge').every((e) => !['statement', 'proof'].includes(e.getAttribute('data-role'))));

    ctrl.destroy();
  } finally {
    dom.restore();
  }
});

test('skip-plan honesty (Rule 6.14): empty main_results dims zero nodes; active skip plan reports basis badge', () => {
  const dom = makeFakeDom();
  try {
    const emptySkipMv = {
      main_results: [],
      skip_plan: { empty: true },
      nodes: [
        { id: 'n-1', kind: 'theorem', label: 'Theorem 1', group: 'S1', grade: 'code', dim: true, flags: ['not referred to by any main result'], badges: [] },
        { id: 'n-2', kind: 'lemma', label: 'Lemma 2', group: 'S2', grade: 'code', dim: true, flags: [], badges: [] },
      ],
      edges: [{ src: 'n-2', dst: 'n-1', kind: 'uses', role: 'proof', basis: 'code' }],
      legend: [],
    };
    const h1 = dom.makeEl('div');
    const c1 = createInvestigateWholeMap(h1, emptySkipMv, { initialMode: VIEW_MODES.WHOLE });
    assert.ok(c1.element.querySelectorAll('.gv-node').every((n) => n.getAttribute('data-dim') === 'false'));
    assert.equal(c1.element.querySelector('[data-filter="hide-dimmed"]').hidden, true);
    assert.equal(c1.element.querySelector('.inv-wm-skip-bar').getAttribute('data-active'), 'false');
    assert.ok(c1.element.querySelector('.inv-wm-skip-summary').textContent.includes(WHOLEMAP_LABELS.skipEmptyNote));

    const explicitSkipMv = {
      main_results: ['n-1'],
      skip_plan: { empty: false, basis: 'explicit' },
      nodes: [
        { id: 'n-1', kind: 'theorem', label: 'Main Theorem', group: 'S1', grade: 'code', role: 'main_result', dim: false, badges: [] },
        { id: 'n-2', kind: 'remark', label: 'Aux Remark', group: 'S2', grade: 'code', dim: true, badges: [] },
      ],
      edges: [],
      legend: [],
    };
    const h2 = dom.makeEl('div');
    const c2 = createInvestigateWholeMap(h2, explicitSkipMv, { initialMode: VIEW_MODES.WHOLE });
    const skip2 = c2.element.querySelector('.inv-wm-skip-bar');
    assert.equal(skip2.getAttribute('data-basis'), 'explicit');
    assert.equal(skip2.querySelector('.inv-wm-skip-basis').getAttribute('data-border'), 'solid');
    assert.ok(skip2.textContent.includes('by explicit cross-references'));
    assert.equal(c2.element.querySelector('[data-node-id="n-2"]').getAttribute('data-dim'), 'true');

    const modelSkipMv = {
      ...explicitSkipMv,
      skip_plan: { empty: false, basis: 'includes_model_inferred' },
    };
    const h3 = dom.makeEl('div');
    const c3 = createInvestigateWholeMap(h3, modelSkipMv, { initialMode: VIEW_MODES.WHOLE });
    const skip3 = c3.element.querySelector('.inv-wm-skip-bar');
    assert.equal(skip3.getAttribute('data-basis'), 'includes_model_inferred');
    assert.equal(skip3.querySelector('.inv-wm-skip-basis').getAttribute('data-border'), 'dashed');
    assert.ok(skip3.textContent.includes('includes model-inferred links'));

    c1.destroy();
    c2.destroy();
    c3.destroy();
  } finally {
    dom.restore();
  }
});

test('V7-absent vs V7-present: Checkpoint 1 hides V7 bar cleanly; Checkpoint 2 filters main_result & authors_claim (model) and honors pin override', () => {
  const dom = makeFakeDom();
  try {
    const cp1Mv = {
      nodes: [
        { id: 'n-def', kind: 'definition', label: 'Definition 1', group: 'S1', grade: 'code', dim: true, badges: [] },
        { id: 'n-thm', kind: 'theorem', label: 'Theorem 1', group: 'S1', grade: 'code', badges: [] },
      ],
      edges: [{ src: 'n-def', dst: 'n-thm', kind: 'uses', role: 'statement', basis: 'code' }],
      legend: [],
    };
    const h1 = dom.makeEl('div');
    const c1 = createInvestigateWholeMap(h1, cp1Mv);
    assert.equal(c1.element.querySelector('.inv-wm-v7-bar').hidden, true);
    assert.equal(c1.element.querySelector('[data-wm-mode="results"]').textContent, WHOLEMAP_LABELS.modeResultsFallback);
    assert.deepEqual(c1.element.querySelectorAll('.gv-node').map((n) => n.getAttribute('data-node-id')), ['n-thm']);
    c1.setMode(VIEW_MODES.WHOLE);
    assert.ok(c1.element.querySelectorAll('.gv-node').every((n) => n.getAttribute('data-dim') === 'false'));

    const cp2Mv = {
      nodes: [
        { id: 't-new', kind: 'theorem', label: 'Main Theorem', group: 'S1', grade: 'code', role: 'main_result', pinned: true, authors_claim: { label: 'new' }, badges: [] },
        { id: 't-unpinned', kind: 'theorem', label: 'Overridden Theorem', group: 'S1', grade: 'code', role: 'main_result', pin: 'not_main_result', authors_claim: { label: 'restated' }, badges: [] },
        { id: 'l-bg', kind: 'lemma', label: 'Background Lemma', group: 'S2', grade: 'quote_fulltext', authors_claim: { label: 'background' }, badges: [] },
        { id: 'c-ns', kind: 'corollary', label: 'Unstated Corollary', group: 'S2', grade: 'code', authors_claim: { label: 'not_stated' }, badges: [] },
      ],
      edges: [],
      legend: [],
    };
    const h2 = dom.makeEl('div');
    const c2 = createInvestigateWholeMap(h2, cp2Mv, { initialMode: VIEW_MODES.WHOLE });
    const v7Bar = c2.element.querySelector('.inv-wm-v7-bar');
    assert.equal(v7Bar.hidden, false);
    const chips = v7Bar.querySelectorAll('[data-wm-authors-label]');
    assert.equal(chips.length, AUTHORS_CLAIM_OPTIONS.length);
    assert.ok(chips.every((ch) => ch.textContent.includes('(model)')));

    // Main results only toggle respects pin='not_main_result' override.
    v7Bar.querySelector('[data-wm-filter="main-results-only"]').click();
    assert.deepEqual(c2.element.querySelectorAll('.gv-node').map((n) => n.getAttribute('data-node-id')), ['t-new']);
    v7Bar.querySelector('[data-wm-filter="main-results-only"]').click();
    assert.equal(c2.element.querySelectorAll('.gv-node').length, 4);

    // Filter by Restated (model) chip.
    v7Bar.querySelector('[data-wm-authors-label="restated"]').click();
    assert.deepEqual(c2.element.querySelectorAll('.gv-node').map((n) => n.getAttribute('data-node-id')), ['t-unpinned']);

    c1.destroy();
    c2.destroy();
  } finally {
    dom.restore();
  }
});

test('wording and XSS safety across results, focused, and whole modes: zero innerHTML, zero forbidden words', () => {
  const dom = makeFakeDom();
  try {
    const xss = '<img src=x onerror=alert(1)><script>alert(2)</script><svg onload=alert(3)>';
    const mv = {
      main_results: ['n-1'],
      skip_plan: { empty: false, basis: 'explicit', summary_line: `1 dimmed — ${xss}` },
      nodes: [
        { id: 'n-1', kind: 'theorem', label: `Theorem ${xss}`, group: `Section ${xss}`, grade: 'code', role: 'main_result', flags: [xss], badges: [{ text: `Badge ${xss}`, icon: 'code', tone: 'ok' }] },
        { id: 'n-2', kind: 'lemma', label: `Lemma ${xss}`, group: `Section B ${xss}`, grade: 'model', dim: true, flags: [xss], badges: [] },
      ],
      edges: [{ src: 'n-2', dst: 'n-1', kind: 'uses', role: 'statement', basis: 'model' }],
      legend: [],
    };
    const host = dom.makeEl('div');
    const ctrl = createInvestigateWholeMap(host, mv);
    for (const mode of [VIEW_MODES.RESULTS, VIEW_MODES.FOCUSED, VIEW_MODES.WHOLE]) {
      ctrl.setMode(mode);
      assert.equal(dom.innerHtmlWrites, 0);
      assert.equal(ctrl.element.querySelectorAll('script').length, 0);
      assert.equal(ctrl.element.querySelectorAll('img').length, 0);
      const allStrings = collectAllStrings(ctrl.element);
      const joined = allStrings.join('\n').toLowerCase();
      assert.equal(/\bverified\b/i.test(joined), false);
      assert.equal(/\bnot\s+needed\b/i.test(joined), false);
      for (const el of [ctrl.element, ...ctrl.element.querySelectorAll('*')]) {
        for (const [k] of el.attributes || []) {
          assert.equal(/^on/i.test(k), false, `unexpected event attribute ${k}`);
        }
      }
    }
    ctrl.destroy();
  } finally {
    dom.restore();
  }
});

test('audit regressions: setMapview reset, <150 box expand banner, F6 stats sync, physics prose column, multi-hop dimming, and hidden-ancestor M guard', () => {
  const dom = makeFakeDom();
  try {
    // 1. setMapview from expanded >150-node paper with V7 filter to small paper resets state cleanly.
    const host = dom.makeEl('div');
    const largeMv = makeLargeTwoSectionMapview();
    const ctrl = createInvestigateWholeMap(host, largeMv, { initialMode: VIEW_MODES.WHOLE });
    ctrl.expandAllSections();
    ctrl.setFilter({ mainResultsOnly: true });
    ctrl.setMapview({
      nodes: [
        { id: 'e1', kind: 'equation', label: 'Eq. (1)', group: 'Section: Related Work', parent: 'l2', grade: 'code', badges: [] },
        { id: 'l2', kind: 'lemma', label: 'Aux Lemma', group: 'Section: Related Work', parent: 's2', grade: 'code', badges: [] },
        { id: 's2', kind: 'section', label: 'Section: Related Work', group: 'Section: Related Work', grade: 'code', badges: [] },
        { id: 'e2', kind: 'equation', label: 'Eq. (2)', group: 'Section 1', grade: 'code', role: 'main_result', badges: [] },
      ],
      edges: [{ src: 'e1', dst: 'e2', kind: 'uses', role: 'prose', basis: 'code' }],
      skip_plan: { empty: false, basis: 'explicit', main_result_tags: ['e2'], sections: [{ section_tag: 's2', label: 'Related Work', dim: true }] },
      legend: [],
    });
    assert.equal(ctrl.element.querySelector('.gv-fold-banner').hidden, true);
    assert.equal(ctrl.element.querySelector('[data-node-id="e1"]').getAttribute('data-dim'), 'true');
    assert.equal(ctrl.element.querySelector('[data-node-id="l2"]').getAttribute('data-dim'), 'true');

    // 2. F6 hide-dimmed toggle updates .inv-wm-stats live.
    assert.match(ctrl.element.querySelector('.inv-wm-stats').textContent, /^3 nodes/);
    ctrl.element.querySelector('[data-filter="hide-dimmed"]').click();
    assert.match(ctrl.element.querySelector('.inv-wm-stats').textContent, /^1 node /);
    ctrl.element.querySelector('[data-filter="hide-dimmed"]').click();

    // 3. <150 collapse + canvas box click expand leaves no spurious Fold groups banner.
    ctrl.collapseSection('Section: Related Work');
    ctrl.element.querySelector('[data-node-id="group:Section: Related Work"]').click();
    assert.equal(ctrl.element.querySelector('.gv-fold-banner').hidden, true);

    // 4. Physics paper prose edge exposes conditional other-uses column in focused view & breadcrumb restores FOCUSED mode.
    ctrl.focusNode('e2');
    assert.deepEqual(
      ctrl.element.querySelectorAll('[data-wm-rel="other-uses"] .inv-wm-rel-btn').map((b) => b.getAttribute('data-node-id')),
      ['e1'],
    );
    ctrl.setMode(VIEW_MODES.WHOLE);
    ctrl.goToBreadcrumb(0);
    assert.equal(ctrl.getMode(), VIEW_MODES.FOCUSED);

    // 5. Hidden ancestor and repeat/Ctrl suppress M shortcut.
    dom.window.dispatchKey('m', { repeat: true });
    dom.window.dispatchKey('m', { ctrlKey: true });
    assert.equal(ctrl.getMode(), VIEW_MODES.FOCUSED);
    host.hidden = true;
    dom.window.dispatchKey('m');
    assert.equal(ctrl.getMode(), VIEW_MODES.FOCUSED);
    ctrl.destroy();
  } finally {
    dom.restore();
  }
});
