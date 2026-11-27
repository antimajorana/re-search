import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import * as apiExports from '../assets/api.js';
import { renderInvestigationBadges } from '../assets/investigation_badges.js';
import {
  FORBIDDEN_HANDOFF_WORDS_RE,
  HANDOFF_ACTION_BUTTONS,
  extractNodeHandoffStatuses,
  mountInvestigationNodeHandoff,
  sanitizeHandoffBadgeText,
  wirePriorWorkHalfBadge,
} from '../assets/investigation_handoff_ui.js';
import { renderInvestigationNodeCard } from '../assets/investigation_node.js';

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
      value: '',
      title: '',
      placeholder: '',
      maxLength: 0,
      hidden: false,
      disabled: false,
      checked: false,
      tabIndex: 0,
      clientWidth: 800,
      clientHeight: 500,
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
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; if (k === 'id') this.id = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      insertBefore(node) { this.children.unshift(node); return node; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      removeEventListener(type, fn) {
        const list = listeners.get(type);
        if (list) listeners.set(type, list.filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, stopPropagation() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      focus() { this._focused = true; },
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
      getElementById: (id) => byId.get(id) || null,
    },
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
  };
}

function collectAllStrings(root) {
  const out = [];
  const walk = (node) => {
    if (!node) return;
    if (node.title) out.push(node.title);
    if (typeof node.children?.length === 'number' && node.children.length === 0 && node.textContent) {
      out.push(node.textContent);
    }
    for (const c of node.children || []) walk(c);
  };
  walk(root);
  return out;
}

async function loadShellController() {
  const shellUrl = new URL('../assets/investigation_v3_shell.js', import.meta.url);
  const src = await readFile(shellUrl, 'utf8');
  const rewritten = src.replace(/(['"])\/assets\/([^'"]+)\1/g, (_, _q, file) =>
    JSON.stringify(new URL(`../assets/${file}`, import.meta.url).href));
  const dataUrl = `data:text/javascript;base64,${Buffer.from(rewritten, 'utf8').toString('base64')}`;
  return import(dataUrl);
}

test('V16 api.js exposes buildInvestigationHandoffPacket and handoff action labels obey F12 wording', () => {
  assert.equal(typeof apiExports.buildInvestigationHandoffPacket, 'function');
  assert.equal(typeof apiExports.createHandoff, 'function');
  assert.equal(typeof apiExports.getHandoffs, 'function');

  assert.deepEqual(
    HANDOFF_ACTION_BUTTONS.map((b) => b.label),
    ['Derive in Solve', 'Has this been extended?', 'Search wider in Explore'],
  );
  for (const btn of HANDOFF_ACTION_BUTTONS) {
    assert.equal(FORBIDDEN_HANDOFF_WORDS_RE.test(btn.label), false, `${btn.label} uses forbidden word`);
    assert.equal(FORBIDDEN_HANDOFF_WORDS_RE.test(btn.title), false, `${btn.title} uses forbidden word`);
  }
  assert.equal(sanitizeHandoffBadgeText('Novel result found'), 'Opened in Explore: status recorded');
  assert.equal(sanitizeHandoffBadgeText('Verified theorem'), 'Opened in Explore: status recorded');
  assert.equal(sanitizeHandoffBadgeText('New approach'), 'Opened in Explore: status recorded');
  assert.equal(sanitizeHandoffBadgeText('Opened in Explore: Model judges: partly the same'), 'Opened in Explore: Model judges: partly the same');
});

test('V16 node card activates right half-badge, mounts action buttons, and triggers Derive / Extended / Search wider', async () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');

  const mv = {
    session_id: 'inv-parent',
    nodes: [{
      id: 'n-thm1',
      kind: 'theorem',
      label: 'Theorem 1: Main',
      fp: 'fp-1',
      role: 'main_result',
      effective_is_main_result: true,
      authors_claim: {
        label: 'new',
        text: 'Authors: We claim new',
        cues: ['we prove'],
        quotes: [],
      },
      spans: [{ path: 'main.tex', start_line: 10, end_line: 15 }],
    }],
    edges: [],
    tasks: [],
    handoff_statuses: [
      {
        sent_seq: 3,
        child_session: 'solve-child-1',
        parent_node_id: 'n-thm1',
        target_mode: 'solve',
        action_kind: 'derive_solve',
        title: 'Derive in Solve — Theorem 1: Main (n-thm1)',
        state: 'finished',
        label: 'Opened in Solve: Done',
      },
    ],
  };

  const builtPackets = [];
  const createdHandoffs = [];
  const openedChildren = [];
  const mockApi = {
    buildInvestigationHandoffPacket: async (sid, payload) => {
      builtPackets.push({ sid, payload });
      return {
        packet: {
          from: { session: sid, mode: 'investigate', id: payload.tag },
          target_mode: payload.action === 'derive_solve' ? 'solve' : 'explore',
          title: `Handoff ${payload.action}`,
        },
        quote_texts: {},
        extra_source_ids: ['src-1'],
      };
    },
    createHandoff: async (sid, req) => {
      createdHandoffs.push({ sid, req });
      const mode = req.packet.target_mode;
      return {
        session_id: `child-${createdHandoffs.length}`,
        mode,
        receipt: { child_session: `child-${createdHandoffs.length}`, packet: req.packet },
      };
    },
    getHandoffs: async () => ({
      child_statuses: [
        {
          sent_seq: 3,
          child_session: 'solve-child-1',
          parent_node_id: 'n-thm1',
          target_mode: 'solve',
          action_kind: 'derive_solve',
          label: 'Opened in Solve: Done',
        },
        {
          sent_seq: 4,
          child_session: 'explore-prior-1',
          parent_node_id: 'n-thm1',
          target_mode: 'explore',
          action_kind: 'prior_work_explore',
          label: 'Opened in Explore: not finished',
        },
      ],
    }),
  };

  renderInvestigationNodeCard(host, mv, 'n-thm1', {
    nodeText: 'Theorem 1 statement.',
    onTogglePin: () => {},
    handoff: {
      sessionId: 'inv-parent',
      api: mockApi,
      openSession: async (id) => { openedChildren.push(id); },
      showStatus: () => {},
    },
  });

  // Allow live read-only getHandoffs projection microtask to flush.
  await new Promise((r) => setTimeout(r, 10));

  // Right half-badge MUST be active (not aria-disabled="true") and obey F12 wording.
  const rightHalf = host.querySelector('[data-v16-slot="prior-work-explore"]');
  assert.ok(rightHalf, 'right half-badge present');
  assert.notEqual(rightHalf.getAttribute('aria-disabled'), 'true');
  assert.equal(rightHalf.textContent, 'Opened in Explore: not finished');
  assert.equal(FORBIDDEN_HANDOFF_WORDS_RE.test(rightHalf.textContent), false);

  // All 3 V16 action buttons rendered inside .inv-node-handoff-slot.
  const slot = host.querySelector('.inv-node-handoff-slot');
  assert.ok(slot);
  assert.ok(slot.querySelector('[data-handoff-action="derive_solve"]'));
  assert.ok(slot.querySelector('[data-handoff-action="extend_explore"]'));
  assert.ok(slot.querySelector('[data-handoff-action="prior_work_explore"]'));

  // Child status chips rendered and clickable to open child session.
  const solveChip = slot.querySelector('[data-child-session="solve-child-1"]');
  assert.ok(solveChip);
  assert.equal(solveChip.textContent, 'Opened in Solve: Done');
  solveChip.click();
  assert.deepEqual(openedChildren, ['solve-child-1']);

  // Clicking right half-badge triggers prior_work_explore ("Search wider in Explore").
  rightHalf.click();
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(builtPackets.length, 1);
  assert.deepEqual(builtPackets[0], { sid: 'inv-parent', payload: { action: 'prior_work_explore', tag: 'n-thm1' } });
  assert.equal(createdHandoffs.length, 1);
  assert.deepEqual(openedChildren, ['solve-child-1', 'child-1']);

  // Clicking "Derive in Solve" & "Has this been extended?" buttons trigger corresponding packets.
  slot.querySelector('[data-handoff-action="derive_solve"]').click();
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(builtPackets[1], { sid: 'inv-parent', payload: { action: 'derive_solve', tag: 'n-thm1' } });

  slot.querySelector('[data-handoff-action="extend_explore"]').click();
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(builtPackets[2], { sid: 'inv-parent', payload: { action: 'extend_explore', tag: 'n-thm1' } });

  // Zero innerHTML writes and right badge text free of forbidden words.
  assert.equal(dom.innerHtmlWrites, 0);
  const slotStrings = collectAllStrings(slot).join(' ');
  assert.equal(/\bverified\b/i.test(slotStrings), false);
});

test('V16 shell controller forwards openSession and triggers V16 handoffs from Block A action bar', async () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = { addEventListener() {}, removeEventListener() {} };

  const shellHost = dom.makeEl('div');
  shellHost.id = 'investigation-v3-shell';
  const { createInvestigationV3Controller } = await loadShellController();

  const builtCalls = [];
  const opened = [];
  const ctrl = createInvestigationV3Controller({
    api: {
      getInvestigationMapview: async () => ({
        etag: 'h1',
        skeleton_seq: 1,
        source_id: 'src-1',
        orient_auto_start: false,
        tasks: [],
        nodes: [{
          id: 'n-thm1',
          kind: 'theorem',
          label: 'Theorem 1',
          fp: 'fp-1',
          spans: [{ path: 'main.tex', start_line: 1, end_line: 5 }],
          authors_claim: { label: 'extends', cues: ['extends'], quotes: [] },
        }],
        edges: [],
        bib: [],
        symbols: [],
        handoff_statuses: [],
      }),
      getInvestigationNodeText: async () => ({ text: 'Theorem statement.' }),
      buildInvestigationHandoffPacket: async (sid, payload) => {
        builtCalls.push({ sid, payload });
        return { packet: { from: { session: sid, id: payload.tag }, target_mode: 'solve', title: 'Solve' } };
      },
      createHandoff: async () => ({ session_id: 'solve-99', mode: 'solve', receipt: { child_session: 'solve-99' } }),
      getHandoffs: async () => ({ child_statuses: [] }),
    },
  });

  await ctrl.syncSession('s-parent', {
    mode: 'investigation',
    investigation: { source_id: 'src-1', skeleton_seq: 1 },
    sources: [{ source_id: 'src-1', format: 'tex' }],
    events: [{ seq: 1 }],
  }, {
    openSession: async (id) => { opened.push(id); },
    showStatus: () => {},
  });
  await new Promise((r) => setTimeout(r, 15));

  const actionBtn = shellHost.querySelector('[data-action="derive_solve"]');
  assert.ok(actionBtn, 'Block A derive_solve action button exists');
  actionBtn.click();
  await new Promise((r) => setTimeout(r, 15));

  assert.deepEqual(builtCalls, [{ sid: 's-parent', payload: { action: 'derive_solve', tag: 'n-thm1' } }]);
  assert.deepEqual(opened, ['solve-99']);
  assert.equal(dom.innerHtmlWrites, 0);

  // Confirm helper unit behaviors for standalone renderInvestigationBadges & wirePriorWorkHalfBadge.
  const standaloneHost = dom.makeEl('div');
  renderInvestigationBadges(standaloneHost, {
    id: 'n-standalone',
    authors_claim: { label: 'new', cues: [], quotes: [] },
  });
  assert.equal(standaloneHost.querySelector('[data-half="right"]').getAttribute('aria-disabled'), 'true');
  wirePriorWorkHalfBadge(standaloneHost, null, [
    { child_session: 'c-bad', parent_node_id: 'n-standalone', action_kind: 'prior_work_explore', label: 'Novel match' },
  ]);
  assert.equal(FORBIDDEN_HANDOFF_WORDS_RE.test(standaloneHost.querySelector('[data-half="right"]').textContent), false);
  assert.equal(extractNodeHandoffStatuses({ outgoing: [{ child_session: 'c1', target_mode: 'explore', from: { id: 'n-standalone' }, child_status: { state: 'opened', label: 'Opened in Explore: Not run yet' } }] }, 'n-standalone').length, 1);
});
