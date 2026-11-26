// Node tests for package X4: Explore state, setup card, candidate list, Where I looked,
// F9 run-state labels, F6 run_bar cost line, empty/failure states, XSS safety, and Rule S8.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { createExploreSessionController, renderExploreSession } from '../assets/idea.js';
import { renderCandidateList, renderWhereILooked } from '../assets/idea_search.js';
import { renderIdeaSetupCard } from '../assets/idea_setup.js';
import {
  EMPTY_STATE_MESSAGES,
  IDEA_STAGE_ORDER,
  NO_MODEL_NOTICE,
  createInitialIdeaState,
  formatCostLine,
  highlightTermsSegments,
  ideaReducer,
  isBarePaperIdentifier,
  selectFilteredCandidates,
  selectRunBarModel,
  selectStageRail,
} from '../assets/idea_state.js';
import { isExploreCardVisible, mountExploreCard } from '../assets/new_menu.js';

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
  const byId = new Map();

  const makeEl = (tag = 'div') => {
    let idVal = '';
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
      maxLength: 0,
      checked: false,
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
        toggle: (c, force) => (force ? classes.add(c) : classes.delete(c)),
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
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'id') this.id = v; },
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
      replaceWith(node) {
        if (!this.parentNode) return;
        const idx = this.parentNode.children.indexOf(this);
        if (idx >= 0) {
          node.parentNode = this.parentNode;
          this.parentNode.children.splice(idx, 1, node);
        }
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatch(type, event = {}) {
        const ev = {
          prevented: false,
          preventDefault() { this.prevented = true; },
          stopPropagation() {},
          target: this,
          ...event,
        };
        for (const fn of listeners.get(type) || []) fn(ev);
        return ev;
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('#')) return n.id === sel.slice(1);
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
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    createTextNode: (t) => { const n = makeEl('#text'); n.textContent = t; return n; },
    getElementById: (id) => byId.get(id) || null,
    querySelector: () => null,
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function sampleIdeaView(overrides = {}) {
  return {
    pipeline_version: 1,
    preset: 'standard',
    preset_label: 'Standard',
    offline: false,
    no_model_notice: NO_MODEL_NOTICE,
    statement: {
      rev: 1,
      text: 'Krylov complexity and Lanczos coefficients in a monitored Majorana chain.',
      fields: ['cond-mat.str-el'],
      seeds: ['arxiv:2405.01111'],
    },
    stages: [
      { id: 'idea', label: 'Idea', stored_state: 'finished', run_state: 'done', run_state_label: 'Done' },
      { id: 'sharpen', label: 'Sharpen', stored_state: 'interrupted', run_state: 'interrupted', run_state_label: 'Interrupted', action: 'retry' },
      { id: 'search', label: 'Search', stored_state: 'finished', run_state: 'done', run_state_label: 'Done' },
      { id: 'triage', label: 'Triage', stored_state: 'idle', run_state: null, run_state_label: 'Idle' },
      { id: 'read', label: 'Read', stored_state: 'stopped_at_cap', run_state: 'paused', run_state_label: 'Paused', action: 'continue' },
      { id: 'map', label: 'Map', stored_state: 'plan_approval', run_state: 'waiting', run_state_label: 'Waiting for you' },
      { id: 'directions', label: 'Directions', stored_state: 'failed', run_state: 'failed', run_state_label: 'Failed', action: 'retry' },
    ],
    run_bar: {
      state: 'running',
      preset_label: 'Standard',
      queries_used: 9,
      queries_max: 18,
      papers_read: 3,
      papers_max: 6,
      modelCallsUsed: 7,
      modelCallsMax: 24,
      extras: ['Standard', '9 of 18 queries', '3 of 6 papers'],
    },
    candidates: [
      {
        key: 'arxiv:2405.01111',
        title: 'Synthetic Krylov Complexity on Majorana Chains',
        authors: ['Ada Example', 'Bram Test'],
        year: 2024,
        abstract_text: 'We study Krylov complexity and Lanczos coefficients across the chain.',
        found_by: ['q-1'],
        in_library: true,
        owned: true,
        user_seed: true,
        read_badge: 'not read',
      },
      {
        key: 'arxiv:2405.03333',
        title: 'Operator Spreading Benchmark',
        authors: ['Claire Sample'],
        year: 2023,
        abstract_text: 'Synthetic benchmark abstract.',
        found_by: ['q-2'],
        in_library: false,
        owned: false,
        user_seed: false,
        read_badge: 'not read',
      },
    ],
    where_i_looked: {
      offline: false,
      providers: [
        { name: 'arxiv', label: 'arXiv', state: 'ok', queries_ok: 2, queries_failed: 0, hits: 10 },
        { name: 'inspire', label: 'INSPIRE-HEP', state: 'ok', queries_ok: 2, queries_failed: 0, hits: 5 },
        { name: 'crossref', label: 'Crossref', state: 'not_attempted', queries_ok: 0, queries_failed: 0, hits: 0 },
        { name: 'semantic_scholar', label: 'Semantic Scholar', state: 'not_configured', queries_ok: 0, queries_failed: 0, hits: 0 },
      ],
      library: { hits: 1, owned_hits: 1 },
      queries: [{ qid: 'q-1', provider: 'arxiv', q: 'all:"Krylov complexity"', status: 'ok', count: 10 }],
      not_searched: ['books and monographs', 'talks and unpublished notes'],
    },
    empty_state: null,
    map: { nodes: [], edges: [], legend: [] },
    map_stale: false,
    ...overrides,
  };
}

test('stage rail, F9 run-state labels, cost line, and reducer filters work deterministically', () => {
  let state = createInitialIdeaState({ hasModel: false });
  state = ideaReducer(state, { type: 'SET_IDEA_VIEW', sessionId: 's-1', view: sampleIdeaView() });

  const rail = selectStageRail(state);
  assert.deepEqual(rail.map((s) => s.id), [...IDEA_STAGE_ORDER]);
  assert.equal(rail[0].runStateLabel, 'Done');
  assert.equal(rail[1].runStateLabel, 'Interrupted');
  assert.equal(rail[1].action, 'retry');
  assert.equal(rail[3].runStateLabel, 'Idle');
  assert.equal(rail[4].runStateLabel, 'Paused');
  assert.equal(rail[4].action, 'continue');
  assert.equal(rail[5].runStateLabel, 'Waiting for you');
  assert.equal(rail[6].runStateLabel, 'Failed');

  const rb = selectRunBarModel(state);
  assert.equal(
    rb.costLine,
    'Standard - 9 of 18 queries - 3 of 6 papers - 7 of at most 24 model calls',
  );
  assert.equal(
    formatCostLine({ preset_label: 'Standard', queries_used: 9, queries_max: 18, papers_read: 3, papers_max: 6, modelCallsUsed: 7, modelCallsMax: 24 }),
    'Standard - 9 of 18 queries - 3 of 6 papers - 7 of at most 24 model calls',
  );

  // Quick preset with max_papers=0 must show "0 of 0 papers" (not 0 of 6), and done_open_items works.
  assert.equal(
    formatCostLine({ preset_label: 'Quick', queries_used: 2, queries_max: 8, papers_read: 0, papers_max: 0, modelCallsUsed: 0, modelCallsMax: 5 }),
    'Quick - 2 of 8 queries - 0 of 0 papers - 0 of at most 5 model calls',
  );
  const doneOpen = selectRunBarModel(createInitialIdeaState({
    view: sampleIdeaView({ run_bar: { state: 'done_open_items', preset_label: 'Quick', queries_used: 2, queries_max: 8, papers_read: 0, papers_max: 0, modelCallsUsed: 0, modelCallsMax: 5, open_items: ['Check scaling', 'Check sign'] } }),
  }));
  assert.deepEqual(doneOpen.openItems, ['Check scaling', 'Check sign']);
  assert.equal(doneOpen.costLine, 'Quick - 2 of 8 queries - 0 of 0 papers - 0 of at most 5 model calls');

  // Filters: libraryOnly, provider via qid->provider map, searchText.
  assert.equal(selectFilteredCandidates(state).length, 2);
  state = ideaReducer(state, { type: 'SET_FILTER', patch: { libraryOnly: true } });
  assert.equal(selectFilteredCandidates(state).length, 1);
  assert.equal(selectFilteredCandidates(state)[0].key, 'arxiv:2405.01111');
  state = ideaReducer(state, { type: 'SET_FILTER', patch: { libraryOnly: false, provider: 'arxiv' } });
  assert.equal(selectFilteredCandidates(state).length, 1);
  assert.equal(selectFilteredCandidates(state)[0].key, 'arxiv:2405.01111');
  state = ideaReducer(state, { type: 'SET_FILTER', patch: { provider: 'all', searchText: 'benchmark' } });
  assert.equal(selectFilteredCandidates(state).length, 1);
  assert.equal(selectFilteredCandidates(state)[0].key, 'arxiv:2405.03333');
});

test('renderExploreSession renders stage rail, run_bar, no-model banner, empty/failure states, and Rule S8', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const scenarios = [
      { view: sampleIdeaView(), hasModel: false, expectedEmpty: null },
      { view: sampleIdeaView({ offline: true, empty_state: { code: 'offline_library_only', message: EMPTY_STATE_MESSAGES.offline_library_only } }), hasModel: false, expectedEmpty: EMPTY_STATE_MESSAGES.offline_library_only },
      { view: sampleIdeaView({ candidates: [], empty_state: { code: 'all_services_failed', message: EMPTY_STATE_MESSAGES.all_services_failed } }), hasModel: true, expectedEmpty: EMPTY_STATE_MESSAGES.all_services_failed },
      { view: sampleIdeaView({ candidates: [], empty_state: { code: 'zero_candidates', message: EMPTY_STATE_MESSAGES.zero_candidates } }), hasModel: true, expectedEmpty: EMPTY_STATE_MESSAGES.zero_candidates },
      { view: sampleIdeaView({ empty_state: { code: 'stage_failed', stage: 'triage', message: 'Model call failed at Triage: candidates are still listed; Retry' } }), hasModel: true, expectedEmpty: 'Model call failed at Triage: candidates are still listed; Retry' },
    ];

    for (const sc of scenarios) {
      let retried = 0;
      const state = createInitialIdeaState({ hasModel: sc.hasModel, view: sc.view });
      renderExploreSession(host, state, { onRetry: () => { retried += 1; } });

      const text = host.textContent;
      assert.equal(dom.innerHtmlWrites, 0, 'Explore session rendering must never write innerHTML');
      if (!sc.hasModel) {
        assert.ok(text.includes(NO_MODEL_NOTICE));
      } else {
        assert.equal(host.querySelector('[data-idea-banner="no-model"]'), null);
      }
      if (sc.expectedEmpty) assert.ok(text.includes(sc.expectedEmpty));
      if (sc.view.empty_state?.code === 'stage_failed') {
        const retryBtn = host.querySelector('[data-empty-state="stage_failed"]')?.querySelector('button');
        assert.ok(retryBtn);
        retryBtn.dispatch('click');
        assert.equal(retried, 1);
      }
      const allStrings = collectStrings(host).join(' ');
      assert.equal(RULE_S8_RE.test(allStrings), false, `Rule S8 violated in Explore DOM: ${allStrings}`);
    }

    const state = createInitialIdeaState({ hasModel: false, view: sampleIdeaView() });
    renderExploreSession(host, state);
    assert.ok(host.querySelectorAll('mark').length > 0);
    assert.ok(host.textContent.includes('not read'));
    assert.ok(host.textContent.includes('in your library'));
    assert.ok(host.textContent.includes('7 of at most 24 model calls'));
  } finally {
    dom.restore();
  }
});

test('mountExploreCard hides Explore, validates IDs, guards stale previews & Ctrl+Enter zero queries', async () => {
  const dom = makeFakeDom();
  try {
    assert.equal(isExploreCardVisible(), false);
    const hiddenHost = dom.makeEl('div');
    assert.equal(mountExploreCard(hiddenHost, { exploreEnabled: false }), null);
    assert.equal(hiddenHost.hidden, true);
    assert.equal(hiddenHost.children.length, 0);

    // Paste detection matches Rust parse_paper_input / split_arxiv_id.
    for (const ok of ['2405.01234', 'arXiv:2405.01234v2', 'https://arxiv.org/abs/2405.01234', 'hep-th/9711200', '10.1103/PhysRevLett.100.010401', 'inspire:123456', 'openalex:W2741809807']) {
      assert.equal(isBarePaperIdentifier(ok), true, `expected bare ID: ${ok}`);
    }
    for (const bad of ['9999.99999', '0612.1234', '1401.12345', 'https://arxiv.org/html/2405.01234', 'Krylov complexity']) {
      assert.equal(isBarePaperIdentifier(bad), false, `expected non-ID: ${bad}`);
    }

    const focused = [];
    const created = [];
    let releaseSlowPreview = null;
    const host = dom.makeEl('div');
    const ctrl = mountExploreCard(host, {
      exploreEnabled: true,
      hasModel: false,
      debounceMs: 0,
      onFocusInvestigate: (id) => focused.push(id),
      previewFn: async ({ text }) => {
        if (text === 'slow-stale-idea') {
          await new Promise((r) => { releaseSlowPreview = r; });
        }
        return {
          focus_investigate: isBarePaperIdentifier(text),
          terms: [{ text, kind: 'keyword' }],
          paper_refs: [],
          invalid_seeds: text.includes('badseed') ? ['not-a-seed'] : [],
          library_matches: [{ paper: { key: 'arxiv:2405.01111', title: 'Synthetic Library Match' }, owned: true }],
          first_queries: [{ qid: 'q-1', provider: 'arxiv', q: `all:"${text}"` }],
        };
      },
      createFn: async (payload) => { created.push(payload); return { id: 's-exp-1', mode: 'explore' }; },
    });
    const textarea = host.querySelector('#idea-setup-text');
    textarea.value = '2405.01234';
    await ctrl.runPreviewNow();
    assert.ok(focused.includes('2405.01234'));

    // Clearing text while a slow preview is in-flight drops the stale reply.
    textarea.value = 'slow-stale-idea';
    const pending = ctrl.runPreviewNow();
    textarea.value = '';
    textarea.dispatch('input');
    releaseSlowPreview();
    await pending;
    assert.equal(host.querySelector('[data-idea-preview="true"]').hidden, true);

    // Unchecking all preview queries online blocks Ctrl+Enter submission (Rule P4).
    textarea.value = 'Krylov complexity in Majorana chains';
    await ctrl.runPreviewNow();
    assert.equal(RULE_S8_RE.test(collectStrings(host).join(' ')), false, 'Rule S8 on setup card');
    const qBox = host.querySelector('[data-preview-section="first-queries"]')?.querySelector('input');
    qBox.checked = false;
    qBox.dispatch('change');
    textarea.dispatch('keydown', { key: 'Enter', ctrlKey: true });
    await Promise.resolve();
    assert.equal(created.length, 0, 'Ctrl+Enter must not submit online with zero checked queries');

    // Editing text after preview invalidates stale preview; submit refreshes preview before create.
    textarea.value = 'Lanczos growth in SYK';
    textarea.dispatch('input');
    await ctrl.submitExplore();
    assert.equal(created.length, 1);
    assert.equal(created[0].text, 'Lanczos growth in SYK');
    assert.deepEqual(created[0].queries, [{ provider: 'arxiv', q: 'all:"Lanczos growth in SYK"' }]);
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('createExploreSessionController preserves filter focus, guards loadSession race, handles SSE & 501', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    let resolveS1 = null;
    const ctrl = createExploreSessionController(host, {
      hasModel: false,
      getExploreSession: async (id) => {
        if (id === 's-1') {
          await new Promise((r) => { resolveS1 = r; });
          return { idea: sampleIdeaView({ statement: { rev: 1, text: 'First session' } }) };
        }
        return { idea: sampleIdeaView({ statement: { rev: 1, text: 'Second session' } }) };
      },
      postIdeaOp: async (_id, op) => {
        const err = new Error(`stage not built (${op})`);
        err.status = 501;
        throw err;
      },
    });

    // Race: s-1 starts first, s-2 resolves first, late s-1 must not overwrite s-2.
    const p1 = ctrl.loadSession('s-1');
    await ctrl.loadSession('s-2');
    resolveS1();
    await p1;
    assert.equal(ctrl.getState().sessionId, 's-2');
    assert.equal(ctrl.getState().view.statement.text, 'Second session');

    // Focus preservation: typing in filter input keeps the exact same input DOM node.
    const filterInput = host.querySelector('[data-idea-filter-input="true"]');
    assert.ok(filterInput);
    filterInput.value = 'benchmark';
    filterInput.dispatch('input');
    assert.equal(host.querySelector('[data-idea-filter-input="true"]'), filterInput, 'filter input node must stay mounted');
    assert.equal(host.querySelectorAll('.idea-candidate-card').length, 1);

    // SSE envelope updates stages + run_bar + candidates badge.
    ctrl.handleSseEvent({
      session: 's-2',
      kind: 'idea.stage',
      payload: { stage: 'search', state: 'stopped_at_cap', reason: 'Query cap reached' },
    });
    assert.equal(ctrl.getState().view.run_bar.state, 'paused');
    assert.equal(ctrl.getState().view.stages.find((s) => s.id === 'search').run_state_label, 'Paused');

    // HTTP 501 renders status notice banner (not error banner).
    await ctrl.invokeOp('continue', {});
    assert.equal(ctrl.getState().error, null);
    assert.ok(host.querySelector('[data-idea-banner="status-notice"]')?.textContent.includes('stage not built'));
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('XSS payloads inject zero elements, assets.rs registers X4, and idea.css uses valid tokens', () => {
  const dom = makeFakeDom();
  try {
    const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
    const view = sampleIdeaView({
      statement: { rev: 1, text: `Idea ${xss}` },
      candidates: [{ key: 'arxiv:2405.00001', title: `Title ${xss}`, authors: [xss], year: 2024, abstract_text: `Abstract Krylov ${xss}`, found_by: [xss], in_library: true, owned: true, read_badge: 'not read' }],
      where_i_looked: { providers: [{ name: 'arxiv', label: xss, state: 'failed', queries_ok: 0, queries_failed: 1, hits: 0 }], library: { hits: 1, owned_hits: 1 }, queries: [{ qid: 'q-1', provider: 'arxiv', q: xss, status: 'failed', count: 0 }], not_searched: [xss] },
    });
    const host = dom.makeEl('div');
    renderExploreSession(host, createInitialIdeaState({ hasModel: false, view, error: `Error ${xss}` }));
    renderCandidateList(view.candidates, { highlightTerms: ['Krylov', xss] });
    renderWhereILooked(view.where_i_looked);
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(host.querySelectorAll('img').length, 0);
    assert.equal(host.querySelectorAll('script').length, 0);
    assert.ok(host.textContent.includes(xss));
    assert.equal(highlightTermsSegments('cost $a+b$ (test)', ['$a+b$', '(test)']).filter((s) => s.match).length, 2);
  } finally {
    dom.restore();
  }

  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  for (const p of ['/assets/api_idea.js', '/assets/idea_state.js', '/assets/idea_setup.js', '/assets/idea_search.js', '/assets/idea.js', '/assets/idea.css']) {
    assert.ok(assetsRs.includes(`"${p}"`), `assets.rs must register ${p}`);
  }
  const ideaCss = fs.readFileSync(path.join(crateRoot, 'assets', 'idea.css'), 'utf8');
  const tokensCss = fs.readFileSync(path.join(crateRoot, 'assets', 'tokens.css'), 'utf8');
  assert.equal(ideaCss.includes('--surface-'), false, 'idea.css must not use undefined --surface-* tokens');
  for (const m of ideaCss.matchAll(/var\((--[a-z0-9-]+)\)/g)) {
    assert.ok(tokensCss.includes(m[1]), `idea.css references unknown token ${m[1]}`);
  }
});
