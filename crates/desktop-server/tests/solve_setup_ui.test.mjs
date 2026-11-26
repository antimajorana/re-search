// Node UI tests for Checkpoint S5a (shared approval_card.js) and S5b (Solve setup card,
// solve_state.js, solve_setup.js), including XSS zero-innerHTML checks and R22 wording.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  ALLOW_LISTED_LABEL,
  GRANT_KIND_SESSION_CAPABILITY,
  GRANT_KIND_WORKER_SESSION,
  NO_AUTO_APPROVE_NOTE,
  WORKER_RESTART_NOTE,
  createApprovalCard,
} from '../assets/approval_card.js';
import {
  NO_MODEL_BANNER,
  PROVE_PROSE_NOTICE,
  SAME_MODEL_WARNING,
  TIER_NUMERIC_WORD,
  TIER_VERIFIED_WORD,
} from '../assets/solve_state.js';
import { createSolveSetupView, renderSafeMathPreview } from '../assets/solve_setup.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeNode = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();

    const node = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      style: {},
      type: '',
      value: '',
      placeholder: '',
      checked: false,
      selected: false,
      disabled: false,
      rows: 0,
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
      setAttribute(k, v) { attrs.set(String(k), String(v)); },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      getAttributes() { return attrs; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(n) { this.children.push(n); return n; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatch(type, event = {}) {
        const ev = { target: this, preventDefault() {}, stopPropagation() {}, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
        return ev;
      },
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
    return node;
  };

  const prevDoc = globalThis.document;
  globalThis.document = {
    createElement: (tag) => makeNode(tag, null),
    createElementNS: (ns, tag) => makeNode(tag, ns),
  };
  return {
    makeEl: (tag = 'div') => makeNode(tag, null),
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function walkTree(node, fn) {
  fn(node);
  for (const child of node.children || []) walkTree(child, fn);
}

test('approval_card.js renders both grant kinds and dispatches separate per-grant approvals with zero auto-approve', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const dispatched = [];
    const grants = [
      {
        id: 'w-solver',
        kind: GRANT_KIND_WORKER_SESSION,
        caller: 'solver:solve',
        tool_id: 'sympy_eval',
        effect: 'execute-code',
        label: 'Solver runs Python calculations',
        granted: false,
      },
      {
        id: 'w-check',
        kind: GRANT_KIND_WORKER_SESSION,
        caller: 'host:check',
        tool_id: 'check_run',
        effect: 'execute-code',
        label: 'Host runs checks',
        granted: false,
      },
      {
        id: 's-net',
        kind: GRANT_KIND_SESSION_CAPABILITY,
        effect: 'network-read',
        label: 'Look up cited papers',
        granted: false,
      },
    ];

    const card = createApprovalCard(host, {
      grants,
      onAllowGrant: async (grant) => {
        dispatched.push({ id: grant.id, kind: grant.kind, caller: grant.caller, effect: grant.effect });
        // Reject the middle grant to prove fail-closed per-grant handling.
        return { approved: grant.id !== 'w-check', reason: grant.id === 'w-check' ? 'denied by user' : '' };
      },
    });

    assert.equal(dom.innerHtmlWrites, 0, 'approval_card must never write innerHTML');
    const rows = host.querySelectorAll('.approval-row');
    assert.equal(rows.length, 3);
    assert.equal(rows[0].getAttribute('data-grant-kind'), GRANT_KIND_WORKER_SESSION);
    assert.equal(rows[2].getAttribute('data-grant-kind'), GRANT_KIND_SESSION_CAPABILITY);
    assert.ok(host.textContent.includes(WORKER_RESTART_NOTE));
    assert.ok(host.textContent.includes(NO_AUTO_APPROVE_NOTE));

    const allowListedBtn = host.querySelector('[data-action="allow-listed"]');
    assert.equal(allowListedBtn.textContent, ALLOW_LISTED_LABEL);
    const results = await card.allowListed();

    // Dispatched 3 separate calls — one per grant, never a merged/auto-approve scope.
    assert.equal(dispatched.length, 3);
    assert.deepEqual(dispatched.map((d) => d.id), ['w-solver', 'w-check', 's-net']);
    assert.equal(results.length, 3);
    const state = card.getGrants();
    assert.equal(state[0].granted, true);
    assert.equal(state[1].granted, false);
    assert.equal(state[1].status, 'denied');
    assert.equal(state[2].granted, true);
  } finally {
    dom.restore();
  }
});

test('solve_setup.js covers empty, prefilled Calculate/Prove/Check-derivation, same-model, no-model, Run collapse, and revision edit', async () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const submitted = [];
    const katexCalls = [];
    const fakeKatex = {
      render(tex, target) {
        katexCalls.push(tex);
        target.textContent = `[TEX:${tex}]`;
      },
    };

    const view = createSolveSetupView(host, {
      katex: fakeKatex,
      initial: {
        modelConnected: false,
        connectedModels: [
          { provider: 'openai', model: 'gpt-5', label: 'OpenAI / gpt-5' },
          { provider: 'anthropic', model: 'claude-sonnet', label: 'Anthropic / claude-sonnet' },
        ],
        toolbox: { available: false, line: 'Toolbox: unavailable (Windows sandbox required)' },
        approvalGrants: {
          pre_turn_approver_supported: true,
          grants: [
            { id: 'w1', kind: 'worker_session', caller: 'solver:solve', tool_id: 'sympy_eval', label: 'Solver code' },
            { id: 's1', kind: 'session_capability', effect: 'network-read', label: 'Paper lookups' },
          ],
        },
      },
      onSubmitSetup: async (payload) => {
        submitted.push(payload);
        return {
          setup: { ...payload, revision: submitted.length },
          view: {
            setup: { ...payload, revision: submitted.length },
            steps: [
              {
                entry_id: 'e-1',
                entry_type: 'assumption',
                summary: 'Setup conventions',
                details: payload.conventions || 'Euclidean',
                badge: { label: 'Given', provenance_tag: 'you' },
              },
            ],
            model_connected: false,
          },
        };
      },
    });

    // Empty state: centre-stage card (not <dialog>), no-model banner, Ask-before-running off by default (D20).
    assert.ok(host.querySelector('[data-testid="solve-setup-card"]'));
    assert.equal(host.querySelectorAll('dialog').length, 0, 'setup card must not be a modal dialog');
    assert.ok(host.querySelector('[data-testid="solve-no-model-banner"]').textContent.includes(NO_MODEL_BANNER));
    assert.equal(host.querySelector('[data-testid="solve-ask-before-running"]').checked, false);
    assert.ok(host.querySelector('[data-testid="solve-toolbox-line"]').textContent.includes('Windows sandbox required'));

    // Apply Calculate prefill with same-model warning (D2) and live KaTeX preview.
    const stmtIn = host.querySelector('[data-testid="solve-statement-input"]');
    stmtIn.value = 'Compute $V_1(\\phi)$ in $d = 4 - 2\\epsilon$.';
    stmtIn.dispatch('input');
    assert.ok(katexCalls.includes('V_1(\\phi)'), 'live KaTeX preview should render math token via katex.render');

    view.applyPrefill({
      prefill: {
        task_kind: 'calculate',
        symbols: [
          { name: 'phi', assumptions: ['real'] },
          { name: 'eps', assumptions: ['real'] },
        ],
        conventions: 'Euclidean signature, hbar = c = 1',
        dimensions: { phi: { mass: 1 } },
        done: [
          { id: 'final_result_tier', kind: 'final_tier_at_least', params: { tier: 'verified' } },
          { id: 'closure_model_agreed', kind: 'closure_tier_at_least', params: { tier: 'model_agreed' } },
        ],
      },
      roles: {
        solver: { provider: 'openai', model: 'gpt-5' },
        verifier: { provider: 'openai', model: 'gpt-5' },
      },
      model_connected: false,
    });

    assert.ok(host.querySelector('[data-testid="solve-same-model-warning"]').textContent.includes(SAME_MODEL_WARNING));
    assert.equal(host.querySelectorAll('.solve-sym-name').length, 2);

    // Toggle D4 numeric target -> updates checklist wording to "Numerically checked" (never "Computed").
    const numToggle = host.querySelector('[data-testid="solve-allow-numeric-toggle"]');
    numToggle.checked = true;
    numToggle.dispatch('change');
    const checklistText = host.querySelector('[data-testid="solve-done-checklist"]').textContent;
    assert.ok(checklistText.includes(TIER_NUMERIC_WORD));
    assert.ok(checklistText.includes(TIER_VERIFIED_WORD));
    assert.ok(!checklistText.includes('Computed'));
    assert.ok(!checklistText.includes('Checked numerically'));

    // Switch to Prove -> explicit D21 notice shown.
    host.querySelector('[data-kind="prove"]').dispatch('click');
    assert.equal(
      host.querySelector('[data-testid="solve-prove-notice"]').textContent,
      PROVE_PROSE_NOTICE,
    );

    // Switch to Check a derivation -> D21 notice removed, transcription confirmation shown.
    host.querySelector('[data-kind="check_derivation"]').dispatch('click');
    assert.equal(host.querySelector('[data-testid="solve-prove-notice"]'), null);
    assert.ok(host.querySelector('[data-testid="solve-done-checklist"]').textContent.includes('Confirm side-by-side TeX transcriptions'));

    // Switch back to Calculate, toggle D20 Ask before running, click Run -> collapses + shows Given steps.
    host.querySelector('[data-kind="calculate"]').dispatch('click');
    const askBox = host.querySelector('[data-testid="solve-ask-before-running"]');
    askBox.checked = true;
    askBox.dispatch('change');
    await view.submitSetup();

    assert.equal(submitted.length, 1);
    assert.equal(submitted[0].ask_before_running, true);
    assert.ok(host.querySelector('[data-testid="solve-collapsed-bar"]'));
    assert.equal(host.querySelector('[data-testid="solve-setup-card"]'), null);
    const givenArea = host.querySelector('[data-testid="solve-given-steps"]');
    assert.ok(givenArea.textContent.includes('Given'));
    assert.ok(givenArea.textContent.includes('e-1'));
    assert.ok(givenArea.textContent.includes('you'));

    // Enter revision edit (D15), edit conventions, save revision 2.
    host.querySelector('[data-action="edit-setup"]').dispatch('click');
    assert.ok(host.querySelector('[data-testid="solve-setup-card"]'));
    const convIn = host.querySelector('[data-testid="solve-conventions-input"]');
    convIn.value = 'Lorentzian signature (-,+,+,+)';
    convIn.dispatch('input');
    await view.submitSetup();
    assert.equal(submitted.length, 2);
    assert.equal(submitted[1].conventions, 'Lorentzian signature (-,+,+,+)');
    assert.ok(host.querySelector('[data-testid="solve-collapsed-bar"]').textContent.includes('rev 2'));
    assert.equal(dom.innerHtmlWrites, 0, 'solve_setup.js must never write innerHTML');
  } finally {
    dom.restore();
  }
});

test('XSS payloads in statement, conventions, symbols, and approval labels produce zero innerHTML writes or injected tags', () => {
  const dom = makeFakeDom();
  try {
    const payloads = [
      '<script>window.__pwn=1</script>',
      '<img src=x onerror=alert(1)>',
      '<svg onload=alert(2)>',
    ];
    const host = dom.makeEl('div');
    createSolveSetupView(host, {
      initial: {
        statement: payloads.join(' '),
        conventions: payloads[1],
        symbols: [{ name: payloads[0], assumptions: [payloads[2]] }],
        dimensions: { [payloads[0]]: { [payloads[1]]: payloads[2] } },
        approvalGrants: {
          grants: [
            { id: 'xss-g', kind: 'worker_session', caller: payloads[0], tool_id: payloads[1], label: payloads[2] },
          ],
        },
      },
    });
    const previewEl = dom.makeEl('div');
    renderSafeMathPreview(previewEl, `${payloads[0]} $x + ${payloads[1]}$ ${payloads[2]}`, null);

    assert.equal(dom.innerHtmlWrites, 0, 'zero innerHTML writes allowed under XSS inputs');
    for (const root of [host, previewEl]) {
      walkTree(root, (node) => {
        assert.notEqual(node.tagName, 'SCRIPT', 'no <script> element may be created');
        assert.notEqual(node.tagName, 'IMG', 'no <img> element may be created');
        if (node.tagName === 'SVG') {
          assert.equal(node.getAttribute('onload'), null, 'no <svg onload> allowed');
        }
        for (const [attrName, attrVal] of node.getAttributes().entries()) {
          assert.ok(!attrName.toLowerCase().startsWith('on'), `event handler attribute ${attrName} forbidden`);
          assert.ok(!attrVal.toLowerCase().startsWith('javascript:'), `javascript: URL forbidden`);
        }
      });
    }
  } finally {
    dom.restore();
  }
});

test('assets.rs registers approval_card.js, solve_state.js, solve_setup.js, and solve.css', () => {
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  for (const asset of [
    '/assets/approval_card.js',
    '/assets/solve_state.js',
    '/assets/solve_setup.js',
    '/assets/solve.css',
  ]) {
    assert.ok(assetsRs.includes(`"${asset}"`), `assets.rs must register ${asset}`);
  }
});
