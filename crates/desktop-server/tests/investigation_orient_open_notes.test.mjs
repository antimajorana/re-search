import assert from 'node:assert/strict';
import test from 'node:test';

import {
  AUTHORS_CLAIM_LABELS,
  MAIN_RESULT_MODEL_LABEL,
  MAIN_RESULT_USER_LABEL,
  NO_EXPLICIT_CUE_TEXT,
  PIN_ACTION_LABEL,
  RIGHT_SLOT_V16_LABEL,
  UNPIN_ACTION_LABEL,
  formatHeadingCiteLine,
  renderInvestigationBadges,
} from '../assets/investigation_badges.js';
import {
  extractAuthorsClaimLabel,
  inspectMapviewCapabilities,
  isMainResultNode,
} from '../assets/investigation_wholemap_model.js';
import {
  CHANGED_DIFF_HEADING,
  NO_MODEL_EXPLANATION_TEXT,
  OPEN_NODE_ACTION_LABEL,
  OPEN_STEP_ACTION_LABEL,
  REUSE_STEP_ACTION_LABEL,
  buildNodeBreadcrumb,
  renderInvestigationNodeModel,
} from '../assets/investigation_node_model.js';
import {
  MAX_NOTE_TEXT_CHARS,
  NOTE_EMPTY_ERROR,
  NOTES_SECTION_TITLE,
  renderInvestigationNotes,
} from '../assets/investigation_notes_ui.js';
import { renderInvestigationNodeCard } from '../assets/investigation_node.js';

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
      placeholder: '',
      maxLength: 0,
      hidden: false,
      disabled: false,
      checked: false,
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
      focus() { this._focused = true; },
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

test('V7 investigation_badges renders authors half-badge, cues, heading cites, quote popover, V16 slot, and pin toggle', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const pinCalls = [];
  const quoteOpens = [];

  const node = {
    id: 'n-thm1111111',
    kind: 'theorem',
    label: 'Theorem 1.1: Decay Estimate',
    role: 'main_result',
    effective_is_main_result: true,
    head_cites: ['NS06'],
    authors_claim: {
      present: true,
      label: 'new',
      text: AUTHORS_CLAIM_LABELS.new,
      cues: ['we prove', 'our main'],
      cue_line: 'Cues in quoted lines: we prove, our main',
      heading_cite_line: 'cites [NS06] in its heading',
      relation_note: 'extends [NS06] to stretched decay',
      attributed_to: ['NS06'],
      quotes: [{
        source_id: 'src-1',
        path: 'main.tex',
        start_line: 14,
        end_line: 16,
        quote: 'We prove our main decay estimate extending [NS06].',
      }],
    },
  };

  const ctrl = renderInvestigationBadges(host, node, {
    onTogglePin: (n, nextPin) => pinCalls.push({ id: n.id, nextPin }),
    onOpenQuote: (target) => quoteOpens.push(target),
  });

  const left = host.querySelector('[data-half="left"]');
  const right = host.querySelector('[data-half="right"]');
  assert.ok(left, 'left half-badge present');
  assert.ok(right, 'right V16 placeholder half-badge present');
  assert.equal(left.textContent, AUTHORS_CLAIM_LABELS.new);
  assert.equal(right.textContent, RIGHT_SLOT_V16_LABEL);
  assert.equal(right.getAttribute('data-v16-slot'), 'prior-work-explore');
  assert.equal(right.getAttribute('aria-disabled'), 'true');
  assert.ok(!right.textContent.toLowerCase().includes('new'));
  assert.ok(!right.textContent.toLowerCase().includes('novel'));

  const headCite = host.querySelector('.inv-v7-heading-cites');
  assert.ok(headCite.textContent.includes('cites [NS06] in its heading'));
  assert.ok(!headCite.textContent.toLowerCase().includes('attributed'));

  // Popover starts collapsed, toggles open on left half-badge click.
  const popover = host.querySelector('.inv-v7-claim-popover');
  assert.equal(popover.hidden, true);
  left.click();
  assert.equal(popover.hidden, false);
  assert.ok(popover.textContent.includes('Cues in quoted lines: we prove, our main'));
  assert.ok(popover.textContent.includes('extends [NS06] to stretched decay (model)'));

  const quoteBtn = popover.querySelector('.ui-quote-chip-open');
  assert.ok(quoteBtn);
  quoteBtn.click();
  assert.equal(quoteOpens.length, 1);
  assert.equal(quoteOpens[0].start_line, 14);

  // Pin toggle flips main_result -> not_main_result.
  const pinBtn = host.querySelector('.inv-v7-pin-btn');
  assert.equal(pinBtn.textContent, UNPIN_ACTION_LABEL);
  pinBtn.click();
  assert.deepEqual(pinCalls, [{ id: 'n-thm1111111', nextPin: 'not_main_result' }]);
  assert.ok(host.textContent.includes(MAIN_RESULT_MODEL_LABEL));

  // Verify user-pinned display and no-cue wording on another node.
  const noCueHost = dom.makeEl('div');
  renderInvestigationBadges(noCueHost, {
    id: 'n-lem2222222',
    pin: 'main_result',
    pinned: true,
    authors_claim: { label: 'background', cues: [], quotes: [] },
  }, { initialPopoverOpen: true, onTogglePin: () => {} });
  assert.ok(noCueHost.textContent.includes(MAIN_RESULT_USER_LABEL));
  assert.ok(noCueHost.textContent.includes(NO_EXPLICIT_CUE_TEXT));
  assert.equal(formatHeadingCiteLine(['K1', 'K2', 'K1']), 'cites [K1, K2] in its heading');

  // Unpinned non-main node shows "Pin as main result".
  const unpinHost = dom.makeEl('div');
  renderInvestigationBadges(unpinHost, { id: 'n-lem3333333', role: 'supporting' }, { onTogglePin: () => {} });
  assert.equal(unpinHost.querySelector('.inv-v7-pin-btn').textContent, PIN_ACTION_LABEL);

  // Wording property & XSS check.
  const xssHost = dom.makeEl('div');
  renderInvestigationBadges(xssHost, {
    id: 'n-xss',
    head_cites: ['<img src=x onerror=alert(1)>'],
    authors_claim: {
      label: 'restated',
      relation_note: '<script>alert(2)</script>',
      cues: [],
      quotes: [{ source_id: 's1', path: 'a.tex', start_line: 1, end_line: 1, quote: '<svg onload=alert(3)>' }],
    },
  }, { initialPopoverOpen: true });
  assert.equal(dom.innerHtmlWrites, 0, 'zero innerHTML writes');
  assert.equal(xssHost.querySelectorAll('script').length, 0);
  assert.equal(xssHost.querySelectorAll('img').length, 0);

  const allText = collectAllStrings(host).concat(collectAllStrings(noCueHost)).join('\n').toLowerCase();
  assert.ok(!allText.includes('verified'), 'Investigate UI must never show "verified"');
  assert.ok(!allText.includes('branch'), 'New Investigate UI drops the word "branch"');
  assert.ok(ctrl);
});

test('V7 enriched mapview automatically activates V13 Main results and Authors label filters', () => {
  const mv = {
    main_result_tags: ['n-thm1'],
    nodes: [
      {
        id: 'n-thm1',
        kind: 'theorem',
        label: 'Theorem 1',
        group: 'Section 1',
        role: 'main_result',
        effective_is_main_result: true,
        authors_claim: { label: 'new', text: AUTHORS_CLAIM_LABELS.new },
      },
      {
        id: 'n-lem2',
        kind: 'lemma',
        label: 'Lemma 2',
        group: 'Section 1',
        role: 'main_result',
        pin: 'not_main_result',
        pinned: false,
        unpinned: true,
        effective_is_main_result: false,
        authors_claim: { label: 'restated', text: AUTHORS_CLAIM_LABELS.restated },
      },
    ],
    edges: [],
    skip_plan: { empty: false, main_result_tags: ['n-thm1'], basis: 'explicit', sections: [] },
  };
  const caps = inspectMapviewCapabilities(mv);
  assert.equal(caps.hasMainResults, true);
  assert.equal(caps.hasAuthorsClaims, true);
  assert.deepEqual(caps.availableAuthorsLabels, ['new', 'restated']);
  assert.equal(isMainResultNode(mv.nodes[0], caps.mainResultIds), true);
  assert.equal(isMainResultNode(mv.nodes[1], caps.mainResultIds), false, 'reader pin=not_main_result overrides model role');
  assert.equal(extractAuthorsClaimLabel(mv.nodes[0]), 'new');
  assert.equal(extractAuthorsClaimLabel(mv.nodes[1]), 'restated');
});

test('V8 investigation_node_model renders unconnected, run bar states, streaming, child steps, reuse, breadcrumb, and diff', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;

  // 1. Unconnected with no saved explanation.
  const emptyMv = { nodes: [{ id: 'n-thm1', label: 'Theorem 1', kind: 'theorem' }], tasks: [] };
  const hostEmpty = dom.makeEl('div');
  const ctrlEmpty = renderInvestigationNodeModel(hostEmpty, {
    mapview: emptyMv, tag: 'n-thm1', modelConnected: false,
  });
  assert.ok(hostEmpty.textContent.includes(NO_MODEL_EXPLANATION_TEXT));

  // Connected with no saved explanation shows Explain button.
  const opened = [];
  ctrlEmpty.update({ modelConnected: true, onOpenNode: (id) => opened.push(id) });
  const openBtn = hostEmpty.querySelector('[data-open-node="n-thm1"]');
  assert.ok(openBtn);
  assert.equal(openBtn.textContent, OPEN_NODE_ACTION_LABEL);
  openBtn.click();
  assert.deepEqual(opened, ['n-thm1']);

  // 2. Populated multi-depth tree + run bar states + streaming + diff.
  const mv = {
    nodes: [
      {
        id: 'n-thm1',
        kind: 'theorem',
        label: 'Theorem 1: Spectral Stability',
        gist: 'Eigenmodes decay exponentially under coercivity.',
        purpose: 'Establishes unconditional decay.',
        hypotheses: [{
          text: 'Discrete energy is coercive.',
          anchor: { source_id: 'src-1', path: 'main.tex', start_line: 10, end_line: 10, quote: 'Q(v) >= alpha ||v||^2' },
        }],
        changed_since_last: {
          label: CHANGED_DIFF_HEADING,
          changes: [{ field: 'gist', previous: 'Old gist', current: 'Eigenmodes decay exponentially under coercivity.' }],
        },
        children: ['m-step1aaaaa', 'm-step2bbbbb', 'm-oq11111111'],
      },
      {
        id: 'm-step1aaaaa',
        model_node: true,
        attach_to: 'n-thm1',
        parent: 'n-thm1',
        kind: 'proof_step',
        model_kind: 'proof_step',
        label: 'Apply coercivity bound',
        gist: 'Use discrete coercivity to bound the energy.',
        anchors: [{ source_id: 'src-1', path: 'main.tex', start_line: 16, end_line: 16, quote: 'Combining Lemma 1 with damping.' }],
        children: ['m-sub1cccccc'],
      },
      {
        id: 'm-step2bbbbb',
        model_node: true,
        attach_to: 'n-thm1',
        parent: 'n-thm1',
        kind: 'proof_step',
        model_kind: 'proof_step',
        label: 'Conclude exponential decay',
        gist: 'Integrate the differential inequality.',
        anchors: [{ source_id: 'src-1', path: 'main.tex', start_line: 17, end_line: 17, quote: 'yields exponential mode decay.' }],
        children: [],
      },
      {
        id: 'm-sub1cccccc',
        model_node: true,
        attach_to: 'm-step1aaaaa',
        parent: 'm-step1aaaaa',
        kind: 'proof_step',
        model_kind: 'proof_step',
        label: 'Substep 1a',
        gist: 'Check positivity on boundary terms.',
        anchors: [{ source_id: 'src-1', path: 'main.tex', start_line: 16, end_line: 16, quote: 'Combining Lemma 1' }],
        children: [],
      },
      {
        id: 'm-oq11111111',
        model_node: true,
        attach_to: 'n-thm1',
        parent: 'n-thm1',
        kind: 'open_question',
        model_kind: 'open_question',
        label: 'Uniformity in mesh size?',
        gist: 'Does alpha stay uniform as h -> 0?',
        children: [],
      },
    ],
    tasks: [{
      request_seq: 11,
      task: 'open',
      tags: ['n-thm1'],
      stored_status: 'partial',
      run_state: 'paused',
      saved_items: 2,
      model_calls: { used: 3, cap: 10 },
    }],
  };

  const selectedCalls = [];
  const openStepCalls = [];
  const continued = [];
  const stopped = [];
  const retried = [];
  const host = dom.makeEl('div');
  const ctrl = renderInvestigationNodeModel(host, {
    mapview: mv,
    tag: 'n-thm1',
    modelConnected: true,
    streamingText: 'Checking damping constant...',
    onSelectNode: (id, meta) => selectedCalls.push({ id, meta }),
    onOpenNode: (id) => openStepCalls.push(id),
    onContinueTask: (t) => continued.push(t.request_seq),
    onStopTask: (t) => stopped.push(t.request_seq),
    onRetryTask: (t) => retried.push(t.request_seq),
  });

  // Run bar maps stored "partial" -> "Paused: 2 items saved" with Continue; never "Partial" as state.
  const runBar = host.querySelector('.ui-run-bar');
  assert.ok(runBar);
  assert.equal(runBar.getAttribute('data-run-state'), 'paused');
  assert.ok(runBar.textContent.includes('Paused: 2 items saved'));
  assert.ok(runBar.textContent.includes('3 of at most 10 model calls'));
  assert.ok(!runBar.textContent.toLowerCase().includes('partial'));
  host.querySelector('[data-run-action="continue"]').click();
  assert.deepEqual(continued, [11]);

  // Streaming text, gist (model), hypotheses quote chip, diff, and open questions rendered.
  assert.ok(host.textContent.includes('Checking damping constant... (model)'));
  assert.ok(host.textContent.includes('Eigenmodes decay exponentially under coercivity. (model)'));
  assert.ok(host.textContent.includes('Discrete energy is coercive. (model)'));
  assert.ok(host.textContent.includes(CHANGED_DIFF_HEADING));
  assert.ok(host.textContent.includes('Does alpha stay uniform as h -> 0? (model)'));

  // Step 1 has saved substeps -> reuses saved steps without calling onOpenNode.
  const step1Btn = host.querySelector('[data-step-id="m-step1aaaaa"]');
  assert.equal(step1Btn.getAttribute('data-step-action'), 'reuse');
  assert.ok(step1Btn.textContent.includes(REUSE_STEP_ACTION_LABEL));
  step1Btn.click();
  assert.deepEqual(selectedCalls, [{ id: 'm-step1aaaaa', meta: { reuseSaved: true } }]);
  assert.equal(openStepCalls.length, 0);

  // Step 2 is unexpanded -> clicking opens ONLY step 2.
  const step2Btn = host.querySelector('[data-step-id="m-step2bbbbb"]');
  assert.equal(step2Btn.getAttribute('data-step-action'), 'open');
  assert.equal(step2Btn.textContent, OPEN_STEP_ACTION_LABEL);
  step2Btn.click();
  assert.deepEqual(openStepCalls, ['m-step2bbbbb']);

  // Running state shows Stop; Interrupted & Failed show Retry.
  ctrl.update({ task: { request_seq: 12, task: 'open', tags: ['n-thm1'], stored_status: 'requested', run_state: 'running', model_calls: { used: 1, cap: 10 } } });
  host.querySelector('[data-run-action="stop"]').click();
  assert.deepEqual(stopped, [12]);

  ctrl.update({ task: { request_seq: 13, task: 'open', tags: ['n-thm1'], stored_status: 'interrupted', run_state: 'interrupted', model_calls: { used: 2, cap: 10 } } });
  host.querySelector('[data-run-action="retry"]').click();
  assert.deepEqual(retried, [13]);

  // Breadcrumb at depth 3 (n-thm1 -> m-step1aaaaa -> m-sub1cccccc).
  const trail = buildNodeBreadcrumb(mv, 'm-sub1cccccc');
  assert.deepEqual(trail.map((c) => c.id), ['n-thm1', 'm-step1aaaaa', 'm-sub1cccccc']);
  const deepHost = dom.makeEl('div');
  const backNav = [];
  renderInvestigationNodeModel(deepHost, {
    mapview: mv, tag: 'm-sub1cccccc', modelConnected: true, onSelectNode: (id) => backNav.push(id),
  });
  const parentCrumb = deepHost.querySelector('[data-crumb-id="n-thm1"]');
  assert.ok(parentCrumb);
  parentCrumb.click();
  assert.deepEqual(backNav, ['n-thm1']);

  const allText = collectAllStrings(host).concat(collectAllStrings(deepHost)).join('\n').toLowerCase();
  assert.ok(!allText.includes('verified'));
  assert.ok(!allText.includes('branch'));
  assert.equal(dom.innerHtmlWrites, 0);
});

test('V10 investigation_notes_ui renders user-grade notes, validates, appends additively, and escapes XSS', async () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const savedPayloads = [];

  const node = {
    id: 'm-step1aaaaa',
    fp: 'fp999',
    notes: [
      { seq: 5, tag: 'm-step1aaaaa', node_fp: 'fp999', stale: false, text: 'Check constant alpha.' },
      { seq: 6, tag: 'm-step1aaaaa', node_fp: 'oldfp', stale: true, text: '<script>alert(9)</script>' },
    ],
  };

  renderInvestigationNotes(host, node, {
    onSaveNote: async (payload) => {
      savedPayloads.push(payload);
      return { seq: 15, ...payload };
    },
  });

  assert.ok(host.textContent.includes(NOTES_SECTION_TITLE));
  assert.ok(host.textContent.includes('Check constant alpha.'));
  assert.ok(host.textContent.includes('<script>alert(9)</script>'));
  assert.equal(host.querySelectorAll('script').length, 0);
  const noteItems = host.querySelectorAll('.inv-v10-note-item');
  assert.equal(noteItems.length, 2);
  assert.equal(noteItems[0].getAttribute('data-grade'), 'user');
  assert.equal(noteItems[0].getAttribute('data-border'), 'solid');
  assert.equal(noteItems[1].getAttribute('data-stale'), 'true');

  // Empty submit shows inline validation error.
  const form = host.querySelector('.inv-v10-notes-form');
  const input = host.querySelector('.inv-v10-notes-input');
  assert.equal(input.maxLength, MAX_NOTE_TEXT_CHARS);
  form.dispatch('submit');
  assert.ok(host.querySelector('.inv-v10-notes-error').textContent.includes(NOTE_EMPTY_ERROR));
  assert.equal(savedPayloads.length, 0);

  // Valid submit appends additively without dropping earlier notes.
  input.value = 'Confirmed sign of commutator <img src=x onerror=alert(1)>';
  input.dispatch('input');
  form.dispatch('submit');
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(savedPayloads.length, 1);
  assert.deepEqual(savedPayloads[0], {
    tag: 'm-step1aaaaa',
    node_fp: 'fp999',
    text: 'Confirmed sign of commutator <img src=x onerror=alert(1)>',
  });
  assert.equal(host.querySelectorAll('.inv-v10-note-item').length, 3);
  assert.equal(input.value, '');
  assert.equal(host.querySelectorAll('img').length, 0);
  assert.equal(dom.innerHtmlWrites, 0);

  const allText = collectAllStrings(host).join('\n').toLowerCase();
  assert.ok(!allText.includes('verified'));
  assert.ok(!allText.includes('branch'));
});

test('Node card mounts V7 badges, V8 Block B, V10 notes, and V16 .inv-node-handoff-slot together', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const mv = {
    nodes: [{
      id: 'n-thm1',
      kind: 'theorem',
      label: 'Theorem 1: Main',
      fp: 'fp-1',
      role: 'main_result',
      effective_is_main_result: true,
      gist: 'Main result gist.',
      head_cites: ['Toy95'],
      authors_claim: { label: 'new', text: AUTHORS_CLAIM_LABELS.new, cues: ['we prove'], quotes: [] },
      notes: [{ seq: 2, tag: 'n-thm1', node_fp: 'fp-1', stale: false, text: 'Keep across reruns.' }],
      spans: [{ path: 'main.tex', start_line: 10, end_line: 12 }],
    }],
    edges: [],
    tasks: [],
  };
  const ctrl = renderInvestigationNodeCard(host, mv, 'n-thm1', {
    nodeText: 'Statement text.',
    modelConnected: true,
    onTogglePin: () => {},
    onOpenNode: () => {},
    onSaveNote: async (p) => ({ seq: 9, ...p }),
  });
  assert.ok(ctrl.badgesCtrl);
  assert.ok(ctrl.modelCtrl);
  assert.ok(ctrl.notesCtrl);
  assert.ok(host.querySelector('.inv-node-handoff-slot'));
  assert.ok(host.querySelector('.inv-v7-badges'));
  assert.ok(host.querySelector('.inv-v8-model'));
  assert.ok(host.querySelector('.inv-v10-notes'));
  assert.equal(dom.innerHtmlWrites, 0);
  const allText = collectAllStrings(host).join('\n').toLowerCase();
  assert.ok(!allText.includes('verified'));
  assert.ok(!allText.includes('branch'));
});
