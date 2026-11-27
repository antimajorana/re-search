import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import {
  LEGACY_ACTIVITY_TITLE,
  LEGACY_CONVENTIONS_TITLE,
  LEGACY_EDGE_ROLE_LABEL,
  LEGACY_OVERVIEW_TITLE,
  LEGACY_REBUILD_BUTTON_LABEL,
  LEGACY_SECTION_TITLE,
  LEGACY_UNPLACED_TITLE,
  mountLegacyEarlierMap,
  mountLegacyPanel,
  relabelOldSessionButtons,
} from '../assets/investigation_legacy.js';
import { renderInvestigationNodeCard } from '../assets/investigation_node.js';
import * as apiExports from '../assets/api.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const byId = new Map();
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    let idVal = '';
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
      hidden: false,
      disabled: false,
      get id() { return idVal; },
      set id(v) {
        if (idVal && byId.get(idVal) === el) byId.delete(idVal);
        idVal = String(v || '');
        if (idVal) byId.set(idVal, el);
      },
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
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; if (k === 'id') this.id = v; },
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
      getElementById: (id) => byId.get(id) || null,
    },
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
  };
}

test('V9 mountLegacyEarlierMap renders attached nodes, unknown (older map) dashed edges, corrections, and escapes XSS cleanly', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const jumped = [];

  const ctrl = mountLegacyEarlierMap(host, [{
    id: 'legacy:10:thm1',
    kind: 'result',
    main: true,
    sentence: 'Oscillator frequency <script>alert(1)</script> bound.',
    anchors: [{ source_id: 's1', path: 'paper.tex', start_line: 10, end_line: 12, quote: '<img src=x onerror=alert(2)>' }],
    edges: [{
      id: 'legacy-edge:10:0',
      direction: 'incoming',
      src: 'legacy:10:a1',
      dst: 'legacy:10:thm1',
      peer_id: 'legacy:10:a1',
      peer_tag: 'n-lem',
      role: LEGACY_EDGE_ROLE_LABEL,
      basis: 'model',
      dashed: true,
      reason: 'Quadratic potential step.',
    }],
    corrections: [{ seq: 15, text: 'Clarify <svg onload=alert(3)> constant k > 0.' }],
  }], { onSelectNode: (tag) => jumped.push(tag) });

  assert.equal(ctrl.count, 1);
  assert.ok(host.textContent.includes(LEGACY_SECTION_TITLE));
  assert.ok(host.textContent.includes('legacy:10:thm1'));
  assert.ok(host.textContent.includes(LEGACY_EDGE_ROLE_LABEL));
  assert.doesNotMatch(host.textContent, /\bverified\b/i);
  assert.doesNotMatch(host.textContent, /\bproof\b/i);
  assert.equal(host.querySelector('.inv-legacy-edge').getAttribute('data-border'), 'dashed');
  assert.equal(host.querySelector('.inv-legacy-correction').getAttribute('data-border'), 'solid');
  assert.equal(dom.innerHtmlWrites, 0);

  host.querySelector('.inv-legacy-jump-btn').click();
  assert.deepEqual(jumped, ['n-lem']);

  // Empty array clears prior mount cleanly.
  const emptyCtrl = mountLegacyEarlierMap(host, []);
  assert.equal(emptyCtrl, null);
  assert.equal(host.querySelector('.inv-legacy-earlier-map'), null);
});

test('V9 mountLegacyPanel renders unplaced tray, overview/conventions/history, fallback note, and relabels Find main results cleanly', () => {
  const dom = makeFakeDom();
  const root = dom.makeEl('div');
  const setupForm = dom.makeEl('form');
  setupForm.id = 'investigation-setup';
  const submitBtn = dom.makeEl('button');
  submitBtn.textContent = 'Find main results';
  setupForm.appendChild(submitBtn);
  root.appendChild(setupForm);

  const shellHost = dom.makeEl('div');
  globalThis.document = { ...dom.document, querySelectorAll: (s) => root.querySelectorAll(s) };

  const legacyPayload = {
    has_legacy: true,
    fallback_old_view: true,
    diagnostic_note: 'Ingested source files changed on disk; showing the earlier view.',
    unplaced: [{
      id: 'legacy:4:old_n',
      kind: 'claim',
      main: false,
      sentence: 'Pre-pointer statement.',
      anchors: [{ source_id: 's1', path: '', start_line: 0, end_line: 0, excerpt: 'Early note <b onmouseover=x>' }],
      edges: [],
      corrections: [],
    }],
    summary: {
      title: 'Harmonic Scaling',
      what: { text: 'Bounds frequency.', anchors: [] },
      why: { unstated: true },
      how: { text: 'Energy balance.', anchors: [] },
    },
    conventions: {
      items: [{ id: 'k', symbol: 'k', meaning: 'stiffness constant' }],
      limitations: ['Single degree of freedom.'],
    },
    answers: [{ question: 'Amplitude dependence?', status: 'answered', text: 'None for linear k.' }],
    activities: [{ question: 'Check boundary condition?', status: 'complete', result: 'Checked smooth boundary.' }],
    corrections: [{ seq: 20, raw_node_id: 'ghost', text: 'Session-level note.' }],
  };

  mountLegacyPanel(shellHost, legacyPayload, { root: globalThis.document });
  const text = shellHost.textContent;
  assert.ok(text.includes(LEGACY_UNPLACED_TITLE));
  assert.ok(text.includes(LEGACY_OVERVIEW_TITLE));
  assert.ok(text.includes(LEGACY_CONVENTIONS_TITLE));
  assert.ok(text.includes(LEGACY_ACTIVITY_TITLE));
  assert.ok(text.includes('Not stated in available source.'));
  assert.ok(text.includes('showing the earlier view.'));
  assert.ok(text.includes('Session-level note.'));
  assert.doesNotMatch(text, /\bverified\b/i);
  assert.equal(submitBtn.textContent, LEGACY_REBUILD_BUTTON_LABEL);
  assert.equal(dom.innerHtmlWrites, 0);

  // Switching to a pure V2 session removes legacy panel and restores button label.
  mountLegacyPanel(shellHost, { has_legacy: false }, { root: globalThis.document });
  assert.equal(shellHost.querySelector('.inv-legacy-panel'), null);
  assert.equal(submitBtn.textContent, 'Find main results');
  assert.equal(relabelOldSessionButtons(globalThis.document, { has_legacy: true }), 1);
  assert.equal(submitBtn.textContent, LEGACY_REBUILD_BUTTON_LABEL);
  assert.equal(typeof apiExports.getInvestigationMapview, 'function');
  assert.equal(typeof apiExports.getInvestigationLegacy, 'function');
});

test('V9 direct integration: investigation_node mounts earlier_map and investigation_v3_shell mounts legacy panel + fallback', async () => {
  const dom = makeFakeDom();
  const root = dom.makeEl('div');
  const setupForm = dom.makeEl('form');
  setupForm.id = 'investigation-setup';
  const submitBtn = dom.makeEl('button');
  submitBtn.textContent = 'Find main results';
  setupForm.appendChild(submitBtn);
  const shellHost = dom.makeEl('div');
  shellHost.id = 'investigation-v3-shell';
  root.append(setupForm, shellHost);
  globalThis.document = { ...dom.document, querySelectorAll: (s) => root.querySelectorAll(s) };
  globalThis.window = { addEventListener() {}, removeEventListener() {} };

  const nodeHost = dom.makeEl('div');
  const mv = {
    etag: 'h-legacy',
    skeleton_seq: 5,
    source_id: 'src-1',
    orient_auto_start: false,
    tasks: [],
    nodes: [{
      id: 'n-thm',
      kind: 'theorem',
      label: 'Theorem 1',
      fp: 'fp-1',
      spans: [{ path: 'paper.tex', start_line: 10, end_line: 12 }],
      earlier_map: [{
        id: 'legacy:3:thm',
        kind: 'result',
        main: true,
        sentence: 'Legacy theorem sentence.',
        anchors: [{ source_id: 'src-1', path: 'paper.tex', start_line: 10, end_line: 10 }],
        edges: [{ id: 'le:1', direction: 'incoming', src: 'legacy:3:lem', dst: 'legacy:3:thm', peer_id: 'legacy:3:lem', role: LEGACY_EDGE_ROLE_LABEL, basis: 'model', dashed: true, reason: 'Dependency' }],
        corrections: [{ seq: 8, text: 'Old note on theorem.' }],
      }],
    }],
    edges: [],
    bib: [],
    symbols: [],
    legacy: {
      has_legacy: true,
      fallback_old_view: false,
      unplaced: [{ id: 'legacy:3:unp', kind: 'claim', sentence: 'Unplaced claim.', anchors: [], edges: [], corrections: [] }],
      summary: { what: { text: 'Overview text.', anchors: [] }, why: { unstated: true } },
      conventions: { items: [{ symbol: 'k', meaning: 'stiffness' }], limitations: [] },
      answers: [],
      activities: [],
      corrections: [],
    },
  };

  const nodeCtrl = renderInvestigationNodeCard(nodeHost, mv, 'n-thm', { nodeText: 'Theorem body.' });
  assert.ok(nodeCtrl.legacyCtrl);
  assert.ok(nodeHost.textContent.includes(LEGACY_SECTION_TITLE));
  assert.ok(nodeHost.textContent.includes('Legacy theorem sentence.'));
  assert.ok(nodeHost.textContent.includes(LEGACY_EDGE_ROLE_LABEL));
  assert.ok(nodeHost.textContent.includes('Old note on theorem.'));

  const shellUrl = new URL('../assets/investigation_v3_shell.js', import.meta.url);
  const src = await readFile(shellUrl, 'utf8');
  const rewritten = src.replace(/(['"])\/assets\/([^'"]+)\1/g, (_, _q, file) =>
    JSON.stringify(new URL(`../assets/${file}`, import.meta.url).href));
  const { createInvestigationV3Controller } = await import(`data:text/javascript;base64,${Buffer.from(rewritten, 'utf8').toString('base64')}`);

  let currentMv = mv;
  const ctrl = createInvestigationV3Controller({
    api: {
      getInvestigationMapview: async () => currentMv,
      getInvestigationNodeText: async () => ({ text: 'Theorem body.' }),
    },
  });
  const ok = await ctrl.syncSession('s-old', {
    mode: 'investigation',
    investigation: { source_id: 'src-1', skeleton_seq: 5 },
    sources: [{ source_id: 'src-1', format: 'tex' }],
    events: [{ seq: 5 }],
  }, { isModelConnected: () => false });
  assert.equal(ok, true);
  assert.ok(shellHost.textContent.includes(LEGACY_UNPLACED_TITLE));
  assert.ok(shellHost.textContent.includes('Overview text.'));
  assert.equal(submitBtn.textContent, LEGACY_REBUILD_BUTTON_LABEL);

  currentMv = { etag: 'h-fallback', skeleton_seq: null, nodes: [], edges: [], legacy: { has_legacy: true, fallback_old_view: true, diagnostic_note: 'Source changed; older view.', unplaced: mv.legacy.unplaced, answers: [], activities: [], corrections: [] } };
  const okFallback = await ctrl.syncSession('s-tampered', {
    mode: 'investigation',
    investigation: { source_id: 'src-1', map: {} },
    sources: [{ source_id: 'src-1', format: 'tex' }],
    events: [{ seq: 1 }],
  }, {});
  assert.equal(okFallback, false);
  assert.equal(shellHost.hidden, false);
  assert.ok(shellHost.textContent.includes('Source changed; older view.'));
  assert.equal(dom.innerHtmlWrites, 0);
});
