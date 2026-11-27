// Node tests for Explore Sharpen ("Sharpened idea"), Search plan approval, Where I looked,
// and Triage UI (packages X6, X7, X8). Zero innerHTML, Rule S8, never "brief" on screen.
import assert from 'node:assert/strict';
import test from 'node:test';

import { renderExploreSession, renderSharpenedIdeaCard } from '../assets/idea.js';
import {
  SHARPENED_IDEA_TITLE,
  createInitialIdeaState,
  highlightSubclaimQuotes,
  ideaReducer,
} from '../assets/idea_state.js';

const RULE_S8_RE = /\b(verified|new|novel|proved)\b/i;
const FORBIDDEN_BRIEF_RE = /\bbrief\b/i;

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      parentNode: null,
      children: [],
      style: {},
      type: '',
      title: '',
      value: '',
      placeholder: '',
      rows: 0,
      checked: false,
      hidden: false,
      disabled: false,
      selected: false,
      open: false,
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, f) => (f ? classes.add(c) : classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      get textContent() {
        return this.children.length
          ? this.children.map((c) => c.textContent ?? '').join('')
          : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      setAttribute(k, v) { attrs.set(k, String(v)); },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      append(...nodes) {
        for (const n of nodes) {
          if (typeof n === 'string') {
            const t = makeEl('#text');
            t.textContent = n;
            t.parentNode = this;
            this.children.push(t);
          } else if (n) {
            n.parentNode = this;
            this.children.push(n);
          }
        }
      },
      appendChild(node) { node.parentNode = this; this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = []; text = ''; html = ''; this.append(...nodes); },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatchEvent(ev) {
        for (const fn of listeners.get(ev.type) || []) fn(ev);
      },
      click() { this.dispatchEvent({ type: 'click', preventDefault() {} }); },
      querySelectorAll(sel) {
        const out = [];
        const walk = (n) => {
          for (const c of n.children || []) {
            if (matches(c, sel)) out.push(c);
            walk(c);
          }
        };
        walk(this);
        return out;
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    };
    return el;
  };
  const matches = (node, sel) => {
    if (sel.startsWith('[')) {
      const m = sel.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
      if (!m) return false;
      const val = node.getAttribute(m[1]);
      return m[2] === undefined ? val !== null : val === m[2];
    }
    return node.tagName.toLowerCase() === sel.toLowerCase();
  };
  const document = {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    createTextNode: (t) => { const n = makeEl('#text'); n.textContent = t; return n; },
  };
  return { document, makeEl, getInnerHtmlWrites: () => innerHtmlWrites };
}

function collectVisibleStrings(node, out = []) {
  if (!node) return out;
  if (node.tagName === '#TEXT' && node.textContent) out.push(node.textContent);
  for (const attr of ['title', 'placeholder']) {
    if (node[attr]) out.push(String(node[attr]));
  }
  const aria = typeof node.getAttribute === 'function' ? node.getAttribute('aria-label') : null;
  if (aria) out.push(aria);
  if (node.children?.length) {
    for (const c of node.children) collectVisibleStrings(c, out);
  } else if (node.tagName !== '#TEXT' && node.textContent) {
    out.push(node.textContent);
  }
  return out;
}

test('X6 Sharpened idea card never shows "brief", highlights subclaim spans, and saves edits safely', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;

  const ideaText =
    'Check whether b_n grows linearly in volume-law phase and saturates in area-law phase <script>alert(1)</script>.';
  const sharpened = {
    idea_rev: 1,
    restatement: 'Check Lanczos scaling across phases <img onerror=alert(2)>',
    assumptions: ['Free-fermion trajectories'],
    subclaims: [
      { id: 'sc-1', text: 'Linear growth in volume-law', idea_quote: 'b_n grows linearly in volume-law phase' },
      { id: 'sc-2', text: 'Saturation in area-law', idea_quote: 'saturates in area-law phase' },
    ],
    terms: [{ term: 'Krylov complexity', aliases: ['spread complexity'] }],
    perspectives: [
      {
        id: 'p-1',
        kind: 'exact',
        question: 'Direct Krylov complexity work',
        queries: [{ provider: 'arxiv', q: 'Krylov complexity Majorana' }],
      },
      {
        id: 'p-2',
        kind: 'other_names',
        question: 'Spread complexity work',
        queries: [{ provider: 'inspire', q: 'spread complexity transition' }],
      },
    ],
    questions: [
      {
        id: 'q-1',
        question: 'Projective or continuous?',
        options: ['projective', 'continuous', 'both'],
        default: 'both',
      },
    ],
  };

  // Pure quote span splitter.
  const segs = highlightSubclaimQuotes(ideaText, sharpened.subclaims);
  const marked = segs.filter((s) => s.subclaimId);
  assert.equal(marked.length, 2);
  assert.equal(marked[0].subclaimId, 'sc-1');
  assert.equal(marked[1].subclaimId, 'sc-2');

  // Render full session with Sharpened idea card.
  const host = dom.makeEl('div');
  let savedPayload = null;
  const state = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-ui-x6',
    view: {
      statement: { rev: 1, text: ideaText },
      sharpened_idea: sharpened,
      question_answers: {},
      stages: [],
      run_bar: { state: 'done', modelCallsUsed: 1, modelCallsMax: 24 },
      candidates: [],
      where_i_looked: { providers: [], queries: [], library: { hits: 0, owned_hits: 0 }, not_searched: [] },
    },
  });
  renderExploreSession(host, state, {
    onSaveAnswer: (payload) => { savedPayload = payload; },
  });

  assert.equal(dom.getInnerHtmlWrites(), 0, 'must never write innerHTML');
  const marks = host.querySelectorAll('mark');
  assert.equal(marks.length, 2);
  assert.equal(marks[0].getAttribute('data-subclaim-id'), 'sc-1');
  assert.equal(marks[1].getAttribute('data-subclaim-id'), 'sc-2');

  const card = host.querySelector('[data-idea-section="sharpened-idea"]');
  assert.ok(card, 'Sharpened idea card must render');
  assert.equal(SHARPENED_IDEA_TITLE, 'Sharpened idea');

  // Edit query + answer question + click Save edits.
  const qInput = card.querySelector('input');
  qInput.value = 'Krylov complexity Majorana chain';
  qInput.dispatchEvent({ type: 'input' });
  const qSelect = card.querySelector('select');
  qSelect.value = 'projective';
  qSelect.dispatchEvent({ type: 'change' });
  const saveBtn = card.querySelector('[data-action="save-sharpen-edits"]');
  saveBtn.click();

  assert.ok(savedPayload, 'Save edits should invoke onSaveAnswer');
  assert.equal(savedPayload.perspectives[0].queries[0].q, 'Krylov complexity Majorana chain');
  assert.equal(savedPayload.answers['q-1'], 'projective');

  // Rule S8 & 00-overview.md 3.2 row 18: zero "verified"/"new"/"novel"/"proved" and zero "brief" on screen.
  const allStrings = collectVisibleStrings(host);
  for (const s of allStrings) {
    assert.ok(!RULE_S8_RE.test(s), `Rule S8 violation in UI string: ${s}`);
    assert.ok(!FORBIDDEN_BRIEF_RE.test(s), `Forbidden word "brief" shown on screen: ${s}`);
  }
  assert.ok(renderSharpenedIdeaCard(null) === null);
});

test('X6 reducer resets sharpened_idea on idea.stated and highlights repeated quote spans', () => {
  const repeated = 'alpha growth then alpha growth again';
  const segs = highlightSubclaimQuotes(repeated, [
    { id: 'sc-1', idea_quote: 'alpha growth' },
    { id: 'sc-2', idea_quote: 'alpha growth' },
  ]);
  const marked = segs.filter((s) => s.subclaimId);
  assert.equal(marked.length, 2);
  assert.equal(marked[0].subclaimId, 'sc-1');
  assert.equal(marked[1].subclaimId, 'sc-2');

  let st = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-sse-reset',
    view: { statement: { rev: 1, text: 'Initial' }, sharpened_idea: null, question_answers: {} },
  });
  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.brief',
    payload: { idea_rev: 1, restatement: 'Sharpened rev 1', subclaims: [], perspectives: [], questions: [] },
  });
  assert.equal(st.view.sharpened_idea.restatement, 'Sharpened rev 1');

  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.stated',
    payload: { rev: 2, text: 'Revised idea text' },
  });
  assert.equal(st.view.sharpened_idea, null);
  assert.deepEqual(st.view.question_answers, {});
});

test('X7 Search plan mounts shared approval_card.js and Where I looked shows Retry <service> and recall', async () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;

  const host = dom.makeEl('div');
  const allowedGrants = [];
  const retriedProviders = [];

  const state = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-ui-x7',
    view: {
      statement: { rev: 1, text: 'Test Krylov growth across phases.' },
      plan_approval: {
        round: 2,
        services: ['arxiv', 'inspire'],
        queries: [
          { qid: 'q-r2-1', provider: 'arxiv', q: 'Krylov complexity Majorana' },
          { qid: 'q-r2-2', provider: 'inspire', q: 'spread complexity <script>x</script>' },
        ],
        grants: [
          {
            id: 'explore-network-read',
            kind: 'session_capability',
            label: 'Allow literature search (arxiv, inspire) for this session',
            tool_id: 'idea_search',
            effect: 'network-read',
            granted: false,
            status: 'required',
          },
        ],
      },
      stages: [],
      run_bar: { state: 'waiting', modelCallsUsed: 1, modelCallsMax: 24 },
      candidates: [],
      where_i_looked: {
        scope_summary: 'Searched 1 of 2 literature services.',
        providers: [
          { name: 'arxiv', label: 'arXiv', state: 'ok', queries_ok: 1, queries_failed: 0, hits: 3, retryable: false },
          { name: 'inspire', label: 'INSPIRE-HEP', state: 'failed', queries_ok: 0, queries_failed: 1, hits: 0, retryable: true },
        ],
        queries: [],
        library: { hits: 1, owned_hits: 1, summary: 'Your library: 1 hit' },
        known_paper_recall: {
          given: ['arxiv:2501.01234', 'arxiv:2401.00001'],
          found_by_search: ['arxiv:2501.01234'],
          missed: ['arxiv:2401.00001'],
        },
        not_searched: ['books and monographs'],
      },
    },
  });

  renderExploreSession(host, state, {
    onAllowGrant: async (grant) => {
      allowedGrants.push(grant);
      return { approved: true };
    },
    onRetryService: (prov) => retriedProviders.push(prov),
  });

  assert.equal(dom.getInnerHtmlWrites(), 0, 'zero innerHTML writes');
  const approvalCard = host.querySelector('[data-component="approval-card"]');
  assert.ok(approvalCard, 'must mount shared approval_card.js');
  assert.equal(allowedGrants.length, 0, 'must never auto-approve');

  const allowBtn = approvalCard.querySelector('[data-action="allow-listed"]');
  assert.ok(allowBtn, 'Allow listed button must exist');
  allowBtn.click();
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(allowedGrants.length, 1);
  assert.equal(allowedGrants[0].kind, 'session_capability');
  assert.equal(allowedGrants[0].effect, 'network-read');

  const retryBtn = host.querySelector('[data-action="retry-service"]');
  assert.ok(retryBtn, 'Retry INSPIRE-HEP button must render for failed service');
  assert.equal(retryBtn.textContent, 'Retry INSPIRE-HEP');
  retryBtn.click();
  assert.deepEqual(retriedProviders, ['inspire']);

  const recall = host.querySelector('[data-idea-recall="1/2"]');
  assert.ok(recall, 'known-paper recall must render');

  for (const s of collectVisibleStrings(host)) {
    assert.ok(!RULE_S8_RE.test(s), `Rule S8 violation: ${s}`);
    assert.ok(!FORBIDDEN_BRIEF_RE.test(s), `Forbidden word "brief": ${s}`);
  }
});

test('X8 Triage groups candidates, marks "from abstract", collapses unrelated, edits reading list, and toggles pause before reading', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;

  const host = dom.makeEl('div');
  const savedReadingLists = [];
  const pauseToggles = [];

  const candidates = [
    { key: 'arxiv:2501.01234', title: 'Core Lanczos Paper <script>x</script>', authors: ['A. One'], year: 2025, read_badge: 'not read' },
    { key: 'arxiv:2406.05432', title: 'Related Spread Complexity Paper', authors: ['B. Two'], year: 2024, read_badge: 'not read' },
    { key: 'arxiv:2301.00001', title: 'Background Review', authors: ['C. Three'], year: 2023, read_badge: 'not read' },
    { key: 'arxiv:2201.00002', title: 'Unrelated Cosmology Paper', authors: ['D. Four'], year: 2022, read_badge: 'not read' },
  ];
  const triage = {
    items: [
      {
        key: 'arxiv:2501.01234',
        relevance: 'core',
        why: 'Studies Lanczos scaling directly <img onerror=alert(1)>',
        abstract_quote: 'b_n grows linearly in the volume-law phase',
        evidence_id: 'xa-12',
        label_basis: 'from abstract',
      },
      { key: 'arxiv:2406.05432', relevance: 'related', why: 'Spread complexity analogue.', label_basis: 'from abstract' },
      { key: 'arxiv:2301.00001', relevance: 'background', why: 'Background context.', label_basis: 'from abstract' },
      { key: 'arxiv:2201.00002', relevance: 'unrelated', why: 'Different domain.', label_basis: 'from abstract' },
    ],
    reading_list: ['arxiv:2501.01234'],
    quick_abstract_only: false,
  };

  const state = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-ui-x8',
    view: {
      statement: { rev: 1, text: 'Test Krylov growth across phases.' },
      stages: [],
      run_bar: { state: 'done', modelCallsUsed: 2, modelCallsMax: 24 },
      budget: { max_papers: 6 },
      pause_before_reading: false,
      candidates,
      triage,
      where_i_looked: { providers: [], queries: [], library: { hits: 0, owned_hits: 0 }, not_searched: [] },
    },
  });

  renderExploreSession(host, state, {
    onSaveReadingList: (rl) => savedReadingLists.push(rl),
    onTogglePauseBeforeReading: (val) => pauseToggles.push(val),
  });

  assert.equal(dom.getInnerHtmlWrites(), 0, 'zero innerHTML writes');
  assert.ok(host.querySelector('[data-triage-group="core"]'), 'core group must render');
  assert.ok(host.querySelector('[data-triage-group="related"]'), 'related group must render');
  assert.ok(host.querySelector('[data-triage-group="background"]'), 'background group must render');
  const unrelatedDetails = host.querySelector('[data-triage-group="unrelated"]');
  assert.ok(unrelatedDetails, 'unrelated group must render');
  assert.equal(unrelatedDetails.tagName, 'DETAILS');
  assert.equal(unrelatedDetails.open, false, 'unrelated group must start collapsed');

  const evBox = host.querySelector('[data-evidence-id="xa-12"]');
  assert.ok(evBox, 'matched xa-12 abstract evidence must render');
  assert.ok(evBox.textContent.includes('matched · abstract only · xa-12'));

  // Toggle reading list on the related paper -> adds it to reading list and accumulates on rapid ticks.
  const rlCheckboxes = host.querySelectorAll('[data-action="toggle-reading-list"]');
  const relatedCb = rlCheckboxes.find((c) => c.getAttribute('data-candidate-key') === 'arxiv:2406.05432');
  const bgCb = rlCheckboxes.find((c) => c.getAttribute('data-candidate-key') === 'arxiv:2301.00001');
  assert.ok(relatedCb && relatedCb.checked === false);
  relatedCb.checked = true;
  relatedCb.dispatchEvent({ type: 'change' });
  bgCb.checked = true;
  bgCb.dispatchEvent({ type: 'change' });
  assert.deepEqual(savedReadingLists[0], ['arxiv:2501.01234', 'arxiv:2406.05432']);
  assert.deepEqual(savedReadingLists[1], ['arxiv:2501.01234', 'arxiv:2406.05432', 'arxiv:2301.00001']);

  // Toggle "Pause before reading".
  const pauseCb = host.querySelector('[data-action="toggle-pause-before-reading"]');
  assert.ok(pauseCb);
  pauseCb.checked = true;
  pauseCb.dispatchEvent({ type: 'change' });
  assert.deepEqual(pauseToggles, [true]);

  for (const s of collectVisibleStrings(host)) {
    assert.ok(!RULE_S8_RE.test(s), `Rule S8 violation: ${s}`);
    assert.ok(!FORBIDDEN_BRIEF_RE.test(s), `Forbidden word "brief": ${s}`);
  }

  // Reaching `max_papers` cap disables remaining unchecked checkboxes.
  const capHost = dom.makeEl('div');
  const capState = ideaReducer(state, {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-ui-x8-cap',
    view: { ...state.view, budget: { max_papers: 1 } },
  });
  renderExploreSession(capHost, capState, { onSaveReadingList: () => {} });
  const capCbs = capHost.querySelectorAll('[data-action="toggle-reading-list"]');
  const capCore = capCbs.find((c) => c.getAttribute('data-candidate-key') === 'arxiv:2501.01234');
  const capRel = capCbs.find((c) => c.getAttribute('data-candidate-key') === 'arxiv:2406.05432');
  assert.equal(capCore.disabled, false);
  assert.equal(capRel.disabled, true);

  // SSE `idea.answer` syncs `triage.reading_list` and SSE `idea.triage` clears stale override.
  const afterAns = ideaReducer(state, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.answer',
    payload: { reading_list: ['arxiv:2406.05432'] },
  });
  assert.deepEqual(afterAns.view.triage.reading_list, ['arxiv:2406.05432']);
  const afterNewTriage = ideaReducer(afterAns, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.triage',
    payload: { ...triage, reading_list: ['arxiv:2501.01234'] },
  });
  assert.equal(afterNewTriage.view.reading_list_override, null);

  // Quick preset (`max_papers: 0`) disables reading-list ticks.
  const quickHost = dom.makeEl('div');
  const quickState = ideaReducer(createInitialIdeaState({ hasModel: true }), {
    type: 'SET_IDEA_VIEW',
    sessionId: 's-ui-x8-quick',
    view: {
      statement: { rev: 1, text: 'Quick preset idea.' },
      stages: [],
      run_bar: { state: 'done', modelCallsUsed: 2, modelCallsMax: 5 },
      budget: { max_papers: 0 },
      candidates,
      triage: { ...triage, reading_list: [], quick_abstract_only: true },
      where_i_looked: { providers: [], queries: [], library: { hits: 0, owned_hits: 0 }, not_searched: [] },
    },
  });
  renderExploreSession(quickHost, quickState, { onSaveReadingList: () => {} });
  const quickCbs = quickHost.querySelectorAll('[data-action="toggle-reading-list"]');
  assert.ok(quickCbs.length > 0 && quickCbs.every((c) => c.disabled && !c.checked));
});



