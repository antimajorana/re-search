// Regression tests for Solve v0.2.1 review fixes:
// - run_bar Continue click event payload hygiene + error banner (no unhandled rejection)
// - rapid double-click Run re-entrancy guard (single setup + single continue)
// - CLI-only route D16 notice banner without doomed solve/continue call
// - transient poll error recovery while Solve run is active
// - collapsed Solve stage pending manual tool approvals (`tool_id` matching)
// - prefill CLI-provider filtering, preserved user "Not selected" choice, and provider-family sameModel check
import assert from 'node:assert/strict';
import test from 'node:test';

import { createModeStagesController } from '../assets/mode_stages.js';
import { createSolveSetupView } from '../assets/solve_setup.js';
import { isSameModelSelection } from '../assets/solve_state.js';

function makeDom() {
  let innerHtmlWrites = 0;
  const byId = new Map();
  const docListeners = new Map();
  let doc = null;

  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const node = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      ownerDocument: doc,
      parentNode: null,
      children: [],
      style: {},
      dataset: {},
      hidden: false,
      disabled: false,
      checked: false,
      selected: false,
      value: '',
      type: '',
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => c && classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        contains: (c) => classes.has(c),
        toggle: (c, force) => {
          const on = force === undefined ? !classes.has(c) : Boolean(force);
          if (on) classes.add(c); else classes.delete(c);
          return on;
        },
      },
      get id() { return attrs.get('id') || ''; },
      set id(v) {
        const s = String(v || '');
        if (s) { attrs.set('id', s); byId.set(s, node); }
      },
      get textContent() {
        return node.children.length ? node.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { node.children = []; text = String(v ?? ''); },
      get innerHTML() { return ''; },
      set innerHTML(_v) { innerHtmlWrites += 1; node.children = []; text = ''; },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
        if (k === 'id' && v) byId.set(String(v), node);
      },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      removeAttribute(k) { attrs.delete(String(k)); },
      append(...nodes) { for (const n of nodes) if (n) node.appendChild(n); },
      appendChild(child) {
        if (!child) return child;
        child.parentNode = node;
        node.children.push(child);
        return child;
      },
      replaceChildren(...nodes) {
        node.children = [];
        text = '';
        for (const n of nodes) if (n) node.appendChild(n);
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      removeEventListener(type, fn) {
        if (listeners.has(type)) listeners.set(type, listeners.get(type).filter((f) => f !== fn));
      },
      dispatch(type, extra = {}) {
        const ev = { type, target: node, currentTarget: node, isTrusted: true, preventDefault() {}, stopPropagation() {}, ...extra };
        for (const fn of listeners.get(type) || []) fn(ev);
        return ev;
      },
      querySelector(sel) { return node.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName || n.tagName === '#TEXT') return false;
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
        visit(node);
        return out;
      },
    };
    return node;
  };

  const head = makeEl('head');
  const shell = makeEl('div'); shell.id = 'app-shell';
  const homeExplore = makeEl('div'); homeExplore.id = 'home-explore-card'; homeExplore.hidden = true;
  const exploreStage = makeEl('div'); exploreStage.id = 'explore-session-stage'; exploreStage.hidden = true;
  const solveStage = makeEl('div'); solveStage.id = 'solve-session-stage'; solveStage.hidden = true;
  const activeTitle = makeEl('h1'); activeTitle.id = 'active-session-title';
  shell.append(activeTitle, homeExplore, exploreStage, solveStage);

  const prevDoc = globalThis.document;
  doc = {
    head,
    createElement: (tag) => { const n = makeEl(tag, null); n.ownerDocument = doc; return n; },
    createElementNS: (ns, tag) => { const n = makeEl(tag, ns); n.ownerDocument = doc; return n; },
    createTextNode: (txt) => { const n = makeEl('#text', null); n.textContent = String(txt ?? ''); n.ownerDocument = doc; return n; },
    getElementById: (id) => byId.get(id) || null,
    querySelector: (sel) => head.querySelector(sel) || shell.querySelector(sel) || null,
    addEventListener: (t, fn) => { if (!docListeners.has(t)) docListeners.set(t, []); docListeners.get(t).push(fn); },
    removeEventListener: (t, fn) => { if (docListeners.has(t)) docListeners.set(t, docListeners.get(t).filter((f) => f !== fn)); },
    dispatchEvent: (ev) => { for (const fn of docListeners.get(ev?.type) || []) fn(ev); },
  };
  for (const n of [head, shell, homeExplore, exploreStage, solveStage, activeTitle]) n.ownerDocument = doc;
  globalThis.document = doc;

  return {
    solveStage,
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

test('Continue button click event sends clean {} body, surfaces HTTP 400 banner, and double-click Run submits once', async () => {
  const dom = makeDom();
  try {
    const calls = [];
    let resolveSetup = null;
    let continueShouldFail = false;

    const stages = createModeStagesController({
      isModelConnected: () => true,
      listConnectedModels: () => [{ provider: 'chatgpt', model: 'gpt-6.2-sol', label: 'ChatGPT / gpt-6.2-sol' }],
      solveOpts: { debounceMs: 0 },
      fetchJson: async (method, url, body) => {
        // Ensure JSON.stringify never sees a cyclic DOM Event `target`.
        if (body !== undefined) JSON.stringify(body);
        calls.push({ method, url, body });
        if (method === 'GET' && url.endsWith('/solve/view')) {
          return { setup: null, steps: [], handles: [], checks: [], receipts: [], run_state: { state: 'ready', running: false }, model_connected: true };
        }
        if (method === 'POST' && url.endsWith('/solve/prefill')) {
          return { prefill: { task_kind: 'calculate' }, model_connected: true };
        }
        if (method === 'POST' && url.endsWith('/solve/setup')) {
          return new Promise((resolve) => {
            resolveSetup = () => resolve({
              setup: { ...body, revision: 1 },
              view: { setup: { ...body, revision: 1 }, steps: [], run_state: { state: 'paused', reason: 'ready', running: false }, model_connected: true },
            });
          });
        }
        if (method === 'POST' && url.endsWith('/solve/continue')) {
          if (continueShouldFail) throw new Error('Solve continuation requires an in-core API or ChatGPT provider');
          return { ok: true, accepted: true, view: { setup: { statement: 'x', revision: 1 }, steps: [], run_state: { state: 'paused', reason: 'end_turn', running: false }, model_connected: true } };
        }
        throw new Error(`Unexpected: ${method} ${url}`);
      },
    });

    await stages.syncFromRail('s-dbl', [{ id: 's-dbl', mode: 'solve' }]);
    const stmt = dom.solveStage.querySelector('[data-testid="solve-statement-input"]');
    stmt.value = 'Compute $x^2$';
    stmt.dispatch('input');

    // Rapid double-click on Run before POST /solve/setup resolves -> single setup + single continue.
    const runBtn = dom.solveStage.querySelector('[data-testid="solve-run-btn"]');
    runBtn.dispatch('click');
    assert.equal(runBtn.disabled, true);
    runBtn.dispatch('click');
    resolveSetup();
    await new Promise((r) => setTimeout(r, 15));

    assert.equal(calls.filter((c) => c.url.endsWith('/solve/setup')).length, 1, 'must post solve/setup once');
    assert.equal(calls.filter((c) => c.url.endsWith('/solve/continue')).length, 1, 'must post solve/continue once');

    // Clicking RunBar Continue dispatches a cyclic DOM click event -> must serialize `{}` cleanly, and surface 400 in banner.
    calls.length = 0;
    continueShouldFail = true;
    const contBtn = dom.solveStage.querySelector('[data-run-action="continue"]');
    assert.ok(contBtn, 'Continue button should render on paused run bar');
    contBtn.dispatch('click');
    await new Promise((r) => setTimeout(r, 15));

    const contCall = calls.find((c) => c.url.endsWith('/solve/continue'));
    assert.deepEqual(contCall?.body, {});
    const banner = dom.solveStage.querySelector('[data-testid="solve-stage-banner"]');
    assert.ok(banner?.textContent.includes('in-core API or ChatGPT provider'));
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('CLI-only D16 notice, transient poll recovery, and collapsed Solve pending approval card work cleanly', async () => {
  const dom = makeDom();
  try {
    const calls = [];
    let pollCount = 0;

    const stages = createModeStagesController({
      isModelConnected: () => true,
      listConnectedModels: () => [],
      pollIntervalMs: 10,
      solveOpts: { debounceMs: 0 },
      fetchJson: async (method, url, body) => {
        calls.push({ method, url, body });
        if (url === '/api/sessions/s-cli-only/solve/view') {
          return { setup: null, steps: [], model_connected: false, cli_review: { on_cli_route: true, notice: 'CLI route notice text' } };
        }
        if (url === '/api/sessions/s-cli-only/solve/setup') {
          return {
            setup: { ...body, revision: 1 },
            view: {
              setup: { ...body, revision: 1 }, steps: [], model_connected: false,
              cli_review: { on_cli_route: true, notice: 'CLI route notice text' },
            },
          };
        }
        if (url === '/api/sessions/s-wait/solve/view') {
          pollCount += 1;
          if (pollCount === 1) {
            return {
              setup: { statement: 'Integrate x^2', revision: 1 },
              steps: [],
              run_state: { state: 'waiting', running: true, model_calls_used: 1, model_calls_max: 60 },
              approval_grants: {
                grants: [],
                pending_approvals: [{ call_id: 'ap-77', caller: 'solver:solve', tool_id: 'sympy_eval', effect: 'execute-code', digest: 'dig-77', allow_session_approve: false }],
              },
              model_connected: true,
            };
          }
          if (pollCount === 2) throw new Error('transient network hiccup');
          return {
            setup: { statement: 'Integrate x^2', revision: 1 },
            steps: [],
            run_state: { state: 'done', running: false, model_calls_used: 2, model_calls_max: 60 },
            approval_grants: { grants: [], pending_approvals: [] },
            model_connected: true,
          };
        }
        if (url === '/api/sessions/s-wait/approvals/ap-77') {
          return { ok: true, approved: body.approved };
        }
        return { prefill: { task_kind: 'calculate' } };
      },
    });

    // 1. CLI-only route saves setup without calling /solve/continue and surfaces D16 notice.
    await stages.syncFromRail('s-cli-only', [{ id: 's-cli-only', mode: 'solve' }]);
    const stmt = dom.solveStage.querySelector('[data-testid="solve-statement-input"]');
    stmt.value = 'Compute derivative';
    stmt.dispatch('input');
    await stages.getSolveView().submitSetup();
    assert.equal(calls.some((c) => c.url.endsWith('/solve/continue')), false);
    assert.equal(dom.solveStage.querySelector('[data-testid="solve-stage-banner"]')?.textContent, 'CLI route notice text');

    // 2. Collapsed running session renders pending manual approval card AND recovers across transient poll error.
    await stages.syncFromRail('s-wait', [{ id: 's-wait', mode: 'solve' }]);
    const allowBtn = dom.solveStage.querySelector('[data-action="allow-one"]');
    assert.ok(allowBtn, 'pending approval Allow button must be visible on collapsed Solve stage');
    allowBtn.dispatch('click');
    await new Promise((r) => setTimeout(r, 45));
    assert.ok(calls.some((c) => c.url === '/api/sessions/s-wait/approvals/ap-77' && c.body.approved === true));
    assert.ok(pollCount >= 3, `expected poll recovery after transient error, got pollCount=${pollCount}`);
    assert.equal(dom.solveStage.querySelector('[data-run-state="done"]') !== null, true);
  } finally {
    dom.restore();
  }
});

test('Prefill filters out CLI roles, preserves user Not-selected choice, and checks provider-family sameModel', () => {
  const dom = makeDom();
  try {
    assert.equal(isSameModelSelection({ provider: 'openai', model: 'gpt-5.4' }, { provider: 'chatgpt', model: 'gpt-5.4' }), true);
    assert.equal(
      isSameModelSelection(
        { provider: '', model: '' },
        { provider: 'chatgpt', model: 'gpt-6.2-sol' },
        [{ provider: 'chatgpt', model: 'gpt-6.2-sol' }],
      ),
      true,
    );

    const host = dom.makeEl('div');
    const view = createSolveSetupView(host, {
      initial: {
        modelConnected: true,
        connectedModels: [
          { provider: 'chatgpt', model: 'gpt-6.2-sol', label: 'ChatGPT / gpt-6.2-sol' },
          { provider: 'openai', model: 'gpt-4.1', label: 'OpenAI / gpt-4.1' },
        ],
      },
    });

    // CLI role in prefill must never poison state.solver.
    view.applyPrefill({
      prefill: { task_kind: 'calculate' },
      roles: { solver: { provider: 'claude-cli', model: 'sonnet' }, verifier: { provider: 'chatgpt', model: 'gpt-6.2-sol' } },
      model_connected: true,
    });
    assert.deepEqual(view.getState().solver, { provider: '', model: '' });
    assert.deepEqual(view.getState().verifier, { provider: 'chatgpt', model: 'gpt-6.2-sol' });

    // Once user explicitly picks "Not selected" for Verifier, subsequent prefill must not overwrite it.
    const verSel = host.querySelector('[data-testid="solve-verifier-select"]');
    verSel.value = '';
    verSel.dispatch('change');
    view.applyPrefill({
      prefill: { task_kind: 'calculate' },
      roles: { verifier: { provider: 'chatgpt', model: 'gpt-6.2-sol' } },
      model_connected: true,
    });
    assert.deepEqual(view.getState().verifier, { provider: '', model: '' });
  } finally {
    dom.restore();
  }
});

test('Same-session syncFromRail refresh during an active Solve run preserves syncSeq and live polling', async () => {
  const dom = makeDom();
  try {
    let viewPolls = 0;
    const stages = createModeStagesController({
      isModelConnected: () => true,
      listConnectedModels: () => [{ provider: 'anthropic', model: 'claude-haiku-4-5', label: 'Anthropic / claude-haiku-4-5' }],
      pollIntervalMs: 15,
      solveOpts: { debounceMs: 0 },
      fetchJson: async (method, url, body) => {
        if (url === '/api/sessions/s-poll-sync/solve/view') {
          viewPolls += 1;
          if (viewPolls === 1) {
            return {
              setup: { statement: 'Integrate x^2', revision: 1 },
              steps: [{ entry_id: 'e-1', entry_type: 'assumption', summary: 'Setup conventions', badge: 'given' }],
              run_state: { state: 'running', running: true, model_calls_used: 0, model_calls_max: 60 },
              model_connected: true,
            };
          }
          return {
            setup: { statement: 'Integrate x^2', revision: 1 },
            steps: [
              { entry_id: 'e-1', entry_type: 'assumption', summary: 'Setup conventions', badge: 'given' },
              { entry_id: 'e-2', entry_type: 'assumption', summary: 'Evaluate integral of x^2', badge: 'model_agreed' },
            ],
            run_state: { state: 'paused', running: false, model_calls_used: 2, model_calls_max: 60 },
            model_connected: true,
          };
        }
        return { ok: true };
      },
    });

    await stages.syncFromRail('s-poll-sync', [{ id: 's-poll-sync', mode: 'solve' }]);
    // Simulate session rail refreshes firing while the Solve run is active.
    await stages.syncFromRail('s-poll-sync', [{ id: 's-poll-sync', mode: 'solve' }]);
    await stages.syncFromRail('s-poll-sync', [{ id: 's-poll-sync', mode: 'solve' }]);
    await new Promise((r) => setTimeout(r, 60));

    assert.ok(viewPolls >= 2, `expected live poll to fire after same-session syncFromRail, got viewPolls=${viewPolls}`);
    assert.equal(dom.solveStage.querySelector('[data-run-state="paused"]') !== null, true);
    const wsText = dom.solveStage.querySelector('[data-testid="solve-workspace"]')?.textContent || '';
    assert.ok(wsText.includes('Evaluate integral of x^2'), `expected workspace to render polled e-2 step, got: ${wsText}`);
  } finally {
    dom.restore();
  }
});
