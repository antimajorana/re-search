// Node tests for package X13: Directions cards & F7 hand-over buttons.
// Covers Rule D3 (`idea_direction_marks_suggestions`: only node chips count as citations;
// all free prose tagged "suggestion (model)"), Concrete vs collapsed Loose ideas `<details>`,
// fixed card layout, "Open in Solve" (Concrete & Loose) + "Investigate this paper" actions,
// read-only child status chips, controller handoff flow, zero innerHTML, XSS safety, Rule S8.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { createExploreSessionController, renderExploreSession } from '../assets/idea.js';
import {
  CHECK_FIRST_LABEL,
  CONCRETE_GROUP_LABEL,
  INVESTIGATE_PAPER_LABEL,
  LOOSE_GROUP_LABEL,
  OPEN_IN_SOLVE_LABEL,
  SUGGESTION_MODEL_LABEL,
  indexHandoffStatuses,
  renderIdeaDirectionsSection,
} from '../assets/idea_directions.js';
import { createInitialIdeaState, ideaReducer } from '../assets/idea_state.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const RULE_S8_RE = /\b(verified|new|novel|proved)\b/i;

function collectStrings(node, out = []) {
  if (!node) return out;
  if (node.tagName === '#TEXT' && node.textContent) out.push(node.textContent);
  for (const attr of ['title', 'placeholder']) {
    if (node[attr]) out.push(String(node[attr]));
  }
  const aria = typeof node.getAttribute === 'function' ? node.getAttribute('aria-label') : null;
  if (aria) out.push(aria);
  if (node.children?.length) {
    for (const c of node.children) collectStrings(c, out);
  } else if (node.tagName !== '#TEXT' && node.textContent) {
    out.push(node.textContent);
  }
  return out;
}

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    return {
      tagName: String(tag).toUpperCase(),
      parentNode: null, children: [], style: {}, type: '', title: '', value: '', placeholder: '',
      disabled: false, hidden: false, open: false, tabIndex: 0, clientWidth: 800, clientHeight: 500,
      get className() { return [...classes].join(' '); },
      set className(v) { classes.clear(); String(v || '').split(/\s+/).filter(Boolean).forEach((c) => classes.add(c)); },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, force) => (force ? classes.add(c) : classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      get textContent() { return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text; },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
        if (k === 'class') this.className = String(v);
        if (k === 'open') this.open = true;
      },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      append(...nodes) {
        for (const n of nodes) {
          if (n == null) continue;
          const child = typeof n === 'string' ? Object.assign(makeEl('#text'), { textContent: n }) : n;
          child.parentNode = this;
          this.children.push(child);
        }
      },
      appendChild(n) { this.append(n); return n; },
      replaceChildren(...nodes) { this.children = []; text = ''; this.append(...nodes); },
      addEventListener(evt, fn) { listeners.set(evt, [...(listeners.get(evt) || []), fn]); },
      removeEventListener(evt, fn) { listeners.set(evt, (listeners.get(evt) || []).filter((x) => x !== fn)); },
      dispatch(evt, evObj = {}) {
        const event = { target: this, preventDefault() {}, stopPropagation() {}, ...evObj };
        for (const fn of listeners.get(evt) || []) fn(event);
      },
      click() { this.dispatch('click'); },
      findAll(pred, acc = []) { if (pred(this)) acc.push(this); for (const c of this.children) c.findAll(pred, acc); return acc; },
      findOne(pred) { return this.findAll(pred)[0] || null; },
    };
  };
  const prevDocument = globalThis.document;
  globalThis.document = {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    createTextNode: (t) => Object.assign(makeEl('#text'), { textContent: t }),
  };
  return { get innerHtmlWrites() { return innerHtmlWrites; }, makeEl, restore() { globalThis.document = prevDocument; } };
}

function sampleDirectionsView(overrides = {}) {
  return {
    statement: { rev: 1, text: 'Bi-Lanczos growth separates volume-law and area-law phases.' },
    map: {
      map_seq: 12,
      idea_rev: 1,
      nodes: [
        { id: 'k-1', kind: 'known_result', label: 'Linear Lanczos growth', evidence: ['xe-4'] },
        { id: 'q-1', kind: 'open_question', label: 'Monitoring effect on Lanczos slope', evidence: ['xe-5'] },
        { id: 'w-1', kind: 'related_work', label: 'Abstract-only reference', evidence: ['xa-6'] },
      ],
      edges: [],
    },
    directions: [
      {
        id: 'r-1',
        title: 'Bi-Lanczos slope along the no-click trajectory',
        question: 'Does the bi-Lanczos slope change across the measurement transition?',
        known: [
          { text: 'Unmonitored chain exhibits linear Lanczos growth', node_ids: ['k-1'] },
        ],
        missing: [
          { text: 'Scaling under projective monitoring', basis: 'open_question(q-1)' },
          { text: 'Finite-size crossover exponent', basis: 'no_evidence_in_scope' },
        ],
        first_calculation: {
          setup: 'Build the quadratic no-click effective Hamiltonian on L = 64..256 sites and run bi-Lanczos for the first calculation.',
          inputs: ['k-1'],
          method: 'numeric',
          expected_form: 'Numeric slope table alpha(gamma)',
          check: {
            text: 'Zero-measurement limit reproduces linear growth of k-1',
            reproduces: 'k-1',
          },
          effort: 'days',
        },
        risks: [
          { text: 'No-click trajectory may differ from averaged ensemble', kind: 'technical', node_ids: ['k-1'] },
        ],
        stop_if: 'Slopes overlap within uncertainty at L = 256',
        solve_setup: {
          statement: 'Compute bi-Lanczos coefficients b_n for the monitored Majorana chain.',
          task_kind: 'calculate',
          target: { description: 'Numeric slope alpha(gamma)', form: 'numeric' },
          conventions: 'Natural units hbar = 1',
          source_keys: ['arxiv:2405.01111'],
        },
        concreteness: 'concrete',
      },
      {
        id: 'r-2',
        title: 'Analogue bound via non-Hermitian spectral flow',
        question: 'Can spectral flow constrain bi-Lanczos growth?',
        known: [
          { text: 'Mentioned only in an abstract', node_ids: ['w-1'] },
        ],
        missing: [
          { text: 'Explicit spectral-flow inequality', basis: 'no_evidence_in_scope' },
        ],
        first_calculation: {
          setup: 'Sketch a spectral-flow estimate.',
          inputs: [],
          method: 'by_hand',
          expected_form: 'Qualitative bound',
          check: { text: '' },
          effort: 'weeks',
        },
        risks: [],
        stop_if: '',
        solve_setup: {
          statement: 'Explore spectral flow bound for bi-Lanczos coefficients.',
          task_kind: 'prove',
          target: { description: 'Upper bound on b_n', form: 'bound' },
          conventions: '',
          source_keys: ['arxiv:2405.02222'],
        },
        concreteness: 'sketch',
        concreteness_reasons: [
          'needs >= 1 known item naming a node backed by full-text xe evidence',
          'first_calculation.setup must be >= 40 characters',
        ],
      },
    ],
    ...overrides,
  };
}

test('idea_direction_marks_suggestions enforces Rule D3, Concrete vs collapsed Loose details, and fixed card layout', () => {
  const dom = makeFakeDom();
  try {
    const selectedNodes = [];
    const view = sampleDirectionsView();
    const sec = renderIdeaDirectionsSection(view, {
      onSelectNode: (info) => selectedNodes.push(info),
    });

    // Concrete group is expanded (`div`, not `<details>`), Loose ideas is a collapsed `<details>`.
    const concreteGroup = sec.findOne((n) => n.getAttribute?.('data-directions-group') === 'concrete');
    const looseGroup = sec.findOne((n) => n.getAttribute?.('data-directions-group') === 'loose');
    assert.ok(concreteGroup && concreteGroup.tagName === 'DIV');
    assert.ok(concreteGroup.textContent.includes(`${CONCRETE_GROUP_LABEL} (1)`));
    assert.ok(looseGroup && looseGroup.tagName === 'DETAILS');
    assert.equal(looseGroup.getAttribute('open'), null, 'Loose ideas <details> must start collapsed');
    assert.equal(looseGroup.open, false);
    assert.ok(looseGroup.textContent.includes(`${LOOSE_GROUP_LABEL} (1)`));

    const r1 = concreteGroup.findOne((n) => n.getAttribute?.('data-direction-card') === 'r-1');
    const r2 = looseGroup.findOne((n) => n.getAttribute?.('data-direction-card') === 'r-2');
    assert.ok(r1 && r2);
    assert.equal(r1.getAttribute('data-concreteness'), 'concrete');
    assert.equal(r2.getAttribute('data-concreteness'), 'sketch');

    // Fixed card layout fields present on r-1.
    for (const field of ['question', 'known', 'missing', 'first-calculation', 'check-first', 'risks', 'stop-if']) {
      assert.ok(r1.findOne((n) => n.getAttribute?.('data-direction-field') === field), `missing field ${field}`);
    }
    assert.ok(r1.textContent.includes(CHECK_FIRST_LABEL));
    assert.ok(r1.textContent.includes('effort: days'));
    assert.ok(r1.textContent.includes('open question (q-1)'));
    assert.ok(r1.textContent.includes('no evidence in searched scope'));
    assert.ok(r2.findOne((n) => n.getAttribute?.('data-concreteness-reasons') === 'true'));

    // Rule D3: every free-text suggestion block carries "suggestion (model)" with dashed border.
    const suggestionBlocks = r1.findAll((n) => n.getAttribute?.('data-model-suggestion') === 'true');
    assert.ok(suggestionBlocks.length >= 8, `expected >= 8 tagged prose blocks, got ${suggestionBlocks.length}`);
    for (const blk of suggestionBlocks) {
      assert.ok(blk.textContent.includes(SUGGESTION_MODEL_LABEL), `prose block missing Rule D3 tag: ${blk.textContent}`);
      const dashedBadge = blk.findOne((n) => n.getAttribute?.('data-border') === 'dashed');
      assert.ok(dashedBadge, 'suggestion (model) badge must use dashed border');
    }

    // Rule D3: ONLY node chips carry `data-citation-chip="true"` and click selects the map node.
    const citeChips = r1.findAll((n) => n.getAttribute?.('data-citation-chip') === 'true');
    assert.equal(citeChips.length, 4, 'k-1 appears in known, inputs, check.reproduces, and risks');
    for (const chip of citeChips) {
      assert.equal(chip.getAttribute('data-node-id'), 'k-1');
    }
    citeChips[0].click();
    assert.equal(selectedNodes.length, 1);
    assert.equal(selectedNodes[0].id, 'k-1');
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('Open in Solve works on both Concrete and Loose cards, Investigate button fires, and child status chips render read-only', () => {
  const dom = makeFakeDom();
  try {
    const solveClicks = [];
    const invClicks = [];
    const openedChildren = [];
    const handoffs = {
      outgoing: [
        {
          sent_seq: 20,
          child_session: 's-solve-1',
          target_mode: 'solve',
          from: { session: 's-exp', mode: 'explore', id: 'r-1' },
          title: 'Bi-Lanczos slope',
          child_status: { state: 'finished', label: 'Opened in Solve: Done' },
        },
        {
          sent_seq: 21,
          child_session: 's-solve-2',
          target_mode: 'solve',
          from: { session: 's-exp', mode: 'explore', id: 'r-2' },
          title: 'Spectral flow',
          child_status: { state: 'finished_open_items', label: 'Opened in Solve: Done with open items' },
        },
        {
          sent_seq: 22,
          child_session: 's-inv-1',
          target_mode: 'investigate',
          // Originated from map node k-1 for paper arxiv:2405.01111 -> should still match paper button!
          from: { session: 's-exp', mode: 'explore', id: 'k-1', paper: { key: 'arxiv:2405.01111' } },
          title: 'Investigate paper',
          child_status: { state: 'skeleton', label: 'Opened in Investigate: Skeleton ready' },
        },
      ],
    };

    const indexed = indexHandoffStatuses(handoffs);
    assert.equal(indexed.get('r-1')?.child_session, 's-solve-1');
    assert.equal(indexed.get('arxiv:2405.01111')?.child_session, 's-inv-1');

    const sec = renderIdeaDirectionsSection(sampleDirectionsView(), {
      handoffs,
      onOpenInSolve: (p) => solveClicks.push(p),
      onInvestigatePaper: (p) => invClicks.push(p),
      onOpenChildSession: (p) => openedChildren.push(p),
    });

    // Both Concrete r-1 and Loose r-2 have active "Open in Solve" buttons.
    const solveBtns = sec.findAll((n) => n.getAttribute?.('data-action') === 'open-in-solve');
    assert.equal(solveBtns.length, 2);
    assert.equal(solveBtns[0].textContent, OPEN_IN_SOLVE_LABEL);
    solveBtns[0].click();
    solveBtns[1].click();
    assert.deepEqual(solveClicks.map((c) => c.direction_id), ['r-1', 'r-2']);

    // "Investigate this paper" button fires with canonical paper_key.
    const invBtns = sec.findAll((n) => n.getAttribute?.('data-action') === 'investigate-paper');
    assert.equal(invBtns.length, 2);
    assert.ok(invBtns[0].textContent.includes(INVESTIGATE_PAPER_LABEL));
    invBtns[0].click();
    assert.deepEqual(invClicks, [{ paper_key: 'arxiv:2405.01111' }]);

    // Read-only child status chips render and open the child session on click.
    const statusChips = sec.findAll((n) => n.getAttribute?.('data-handoff-status'));
    assert.equal(statusChips.length, 3);
    assert.ok(statusChips.some((c) => c.textContent === 'Opened in Solve: Done'));
    assert.ok(statusChips.some((c) => c.textContent === 'Opened in Solve: Done with open items'));
    assert.ok(statusChips.some((c) => c.textContent === 'Opened in Investigate: Skeleton ready'));
    statusChips[0].click();
    assert.deepEqual(openedChildren, [{ session_id: 's-solve-1', mode: 'solve' }]);
  } finally {
    dom.restore();
  }
});

test('createExploreSessionController loads read-only handoffs and wires Open in Solve & Investigate buttons', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const ops = [];
    const opened = [];
    let handoffFetches = 0;
    const view = sampleDirectionsView();

    const ctrl = createExploreSessionController(host, {
      hasModel: true,
      getExploreSession: async () => ({ idea: view }),
      getExploreHandoffs: async () => {
        handoffFetches += 1;
        return {
          outgoing: handoffFetches >= 2
            ? [{
                sent_seq: 30,
                child_session: 's-child-solve',
                target_mode: 'solve',
                from: { id: 'r-1' },
                child_status: { state: 'opened', label: 'Opened in Solve: Not run yet' },
              }]
            : [],
        };
      },
      postIdeaOp: async (sid, op, payload) => {
        ops.push({ sid, op, payload });
        return { session_id: 's-child-solve', mode: 'solve', prefill: { title: 'Bi-Lanczos slope' }, idea: view };
      },
      onOpenChildSession: (info) => { opened.push(info); },
    });

    await ctrl.loadSession('s-exp-1', true);
    assert.equal(handoffFetches, 1);

    const solveBtn = host.findOne((n) => n.getAttribute?.('data-action') === 'open-in-solve' && n.getAttribute?.('data-direction-id') === 'r-1');
    assert.ok(solveBtn);
    solveBtn.click();
    await new Promise((r) => setTimeout(r, 15));

    assert.deepEqual(ops, [{ sid: 's-exp-1', op: 'handoff', payload: { action: 'open_in_solve', direction_id: 'r-1' } }]);
    assert.equal(handoffFetches, 2);
    assert.deepEqual(opened, [{ session_id: 's-child-solve', mode: 'solve', title: 'Bi-Lanczos slope' }]);
    assert.ok(host.textContent.includes('Opened in Solve: Not run yet'));
  } finally {
    dom.restore();
  }
});

test('X13 Directions UI passes XSS safety, Rule S8 sweep, zero lean references, and assets.rs registration', () => {
  const dom = makeFakeDom();
  try {
    const xssView = sampleDirectionsView();
    xssView.directions[0].title = '<img src=x onerror=alert(1)>';
    xssView.directions[0].question = '<script>alert(2)</script>';
    const host = dom.makeEl('div');
    const st = { ...createInitialIdeaState({ hasModel: true }), sessionId: 's-xss', view: xssView };
    renderExploreSession(host, st, {});
    assert.equal(dom.innerHtmlWrites, 0, 'must never write innerHTML');
    assert.ok(host.textContent.includes('<img src=x onerror=alert(1)>'));

    // Rule S8 sweep over clean full Explore session with Concrete + Loose directions.
    const cleanHost = dom.makeEl('div');
    const cleanSt = { ...createInitialIdeaState({ hasModel: true }), sessionId: 's-s8', view: sampleDirectionsView() };
    renderExploreSession(cleanHost, cleanSt, {});
    const allStrings = collectStrings(cleanHost);
    for (const s of allStrings) {
      assert.equal(RULE_S8_RE.test(s), false, `Rule S8 forbidden word in Explore DOM string: ${JSON.stringify(s)}`);
    }

    // Source checks: assets.rs serves /assets/idea_directions.js; idea_directions.js has zero innerHTML and zero lean.
    const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
    assert.ok(assetsRs.includes('"/assets/idea_directions.js"'));
    const dirJs = fs.readFileSync(path.join(crateRoot, 'assets', 'idea_directions.js'), 'utf8');
    assert.equal(/\.innerHTML\b|outerHTML\b|insertAdjacentHTML\b/.test(dirJs), false);
    assert.equal(/\blean\b/i.test(dirJs), false, 'idea_directions.js must never offer lean');
  } finally {
    dom.restore();
  }
});

test('SSE reducer updates idea.directions, clears on idea.stated & idea.map, and scopes Solve badges by map_seq', () => {
  const view = sampleDirectionsView();
  let st = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-sse',
    view: { ...view, directions: [] },
  });
  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.directions',
    payload: { directions: view.directions },
  });
  assert.equal(st.view.directions.length, 2);

  // Re-mapping via SSE clears pre-remap directions.
  st = ideaReducer(st, { type: 'APPLY_SSE_EVENT', kind: 'idea.map', payload: { map_seq: 30, nodes: [] } });
  assert.equal(st.view.directions.length, 0);

  // Revising idea via SSE clears directions.
  st = ideaReducer(st, { type: 'APPLY_SSE_EVENT', kind: 'idea.directions', payload: { directions: view.directions } });
  assert.equal(st.view.directions.length, 2);
  st = ideaReducer(st, { type: 'APPLY_SSE_EVENT', kind: 'idea.stated', payload: { rev: 2, text: 'Revised idea' } });
  assert.equal(st.view.directions.length, 0);

  // indexHandoffStatuses ignores pre-remap Solve handoffs (sent_seq < active map_seq) while keeping paper handoffs.
  const idx = indexHandoffStatuses({
    outgoing: [
      { sent_seq: 10, child_session: 's-old-solve', target_mode: 'solve', from: { id: 'r-1' }, child_status: { state: 'finished', label: 'Opened in Solve: Done' } },
      { sent_seq: 35, child_session: 's-cur-solve', target_mode: 'solve', from: { id: 'r-2' }, child_status: { state: 'opened', label: 'Opened in Solve: Not run yet' } },
      { sent_seq: 11, child_session: 's-inv', target_mode: 'investigate', from: { id: 'k-1', paper: { key: 'arxiv:2405.01111' } }, child_status: { state: 'skeleton', label: 'Opened in Investigate: Skeleton ready' } },
    ],
  }, 30);
  assert.equal(idx.get('r-1'), undefined, 'stale Solve handoff from older map must not collide with new r-1');
  assert.equal(idx.get('r-2')?.child_session, 's-cur-solve');
  assert.equal(idx.get('arxiv:2405.01111')?.child_session, 's-inv');
});
