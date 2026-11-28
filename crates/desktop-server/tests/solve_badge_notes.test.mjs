// Tests for WP1 badge notes (check_note warning chip), unchecked_deps jump links,
// trivial outcome in solve_why.js, producing sympy_eval code & source_expr rendering,
// clarified own_check_line for steps with prose details, and solve_state.js same-model rename.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  OWN_CHECK_EQUATIONS_VERIFIED_LINE,
  OWN_CHECK_VERIFIED_LINE,
  createSolveBadgeGroup,
  resolveSafeOwnCheckLine,
} from '../assets/solve_badges.js';
import { buildSetupSubmission, createInitialSolveState } from '../assets/solve_state.js';
import { createSolveWorkspace } from '../assets/solve_steps.js';
import { createSolveWhyPopover } from '../assets/solve_why.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const repoRoot = path.resolve(crateRoot, '..', '..');
const fixturePath = path.join(repoRoot, 'tests', 'fixtures', 'solve_view_fixture.json');

function loadFixture() {
  return JSON.parse(fs.readFileSync(fixturePath, 'utf8'));
}

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      parentNode: null,
      style: {},
      type: '',
      name: '',
      value: '',
      placeholder: '',
      title: '',
      hidden: false,
      disabled: false,
      checked: false,
      get firstChild() { return this.children[0] || null; },
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
      set textContent(v) { this.children = []; text = String(v ?? ''); },
      get innerHTML() { return ''; },
      set innerHTML(_) { innerHtmlWrites += 1; throw new Error('innerHTML write forbidden'); },
      set outerHTML(_) { innerHtmlWrites += 1; throw new Error('outerHTML write forbidden'); },
      insertAdjacentHTML() { innerHtmlWrites += 1; throw new Error('insertAdjacentHTML forbidden'); },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
        if (k === 'id') this.id = String(v);
        if (k === 'class') this.className = v;
      },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      removeAttribute(k) { attrs.delete(String(k)); },
      getAttributes: () => attrs,
      append(...nodes) {
        for (const n of nodes) {
          if (n && typeof n === 'object') n.parentNode = this;
          this.children.push(n);
        }
      },
      appendChild(n) { this.append(n); return n; },
      removeChild(n) {
        const i = this.children.indexOf(n);
        if (i >= 0) { this.children.splice(i, 1); n.parentNode = null; }
        return n;
      },
      replaceChildren(...nodes) {
        for (const c of this.children) if (c && typeof c === 'object') c.parentNode = null;
        this.children = [];
        text = '';
        this.append(...nodes);
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatch(type, event = {}) {
        const ev = { target: this, preventDefault() {}, stopPropagation() {}, ...event };
        for (const fn of [...(listeners.get(type) || [])]) fn(ev);
        return ev;
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('#')) return n.id === sel.slice(1);
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const attr = sel.match(/^([a-zA-Z0-9_-]+)?\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (attr) {
            if (attr[1] && n.tagName !== attr[1].toUpperCase()) return false;
            const val = n.getAttribute(attr[2]);
            return attr[3] === undefined ? val !== null : val === attr[3];
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
    createElement: (tag) => makeEl(tag, null),
    createElementNS: (ns, tag) => makeEl(tag, ns),
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

test('R1-F3 & R1-F5: check_note warning chip, unchecked_deps jump links, and clarified own_check_line', () => {
  const dom = makeFakeDom();
  try {
    const jumps = [];
    const group = createSolveBadgeGroup({
      own: 'unchecked',
      effective: 'unchecked',
      label: 'Unchecked',
      check_note: 'both sides identical on 1 of 1 claim',
      rests_on_line: 'rests on e-2 (unchecked)',
      lowered_by: [],
      unchecked_deps: ['e-2'],
      broken_by: [],
      flags: [],
    }, { onSelectStep: (id) => jumps.push(id) });

    const noteChip = group.querySelector('[data-badge-role="check-note"]');
    assert.ok(noteChip, 'check_note warning chip must render');
    assert.equal(noteChip.className, 'solve-check-note-chip');
    assert.equal(noteChip.textContent, 'both sides identical on 1 of 1 claim');

    const jumpBtn = group.querySelector('[data-jump-step="e-2"]');
    assert.ok(jumpBtn, 'unchecked_deps jump button must render when lowered_by is empty');
    jumpBtn.dispatch('click');
    assert.deepEqual(jumps, ['e-2']);

    // resolveSafeOwnCheckLine allows clarified line only when own == verified && effective != verified && !stale.
    assert.equal(
      resolveSafeOwnCheckLine({ own: 'verified', effective: 'unchecked', own_check_line: OWN_CHECK_EQUATIONS_VERIFIED_LINE, stale: false }),
      OWN_CHECK_EQUATIONS_VERIFIED_LINE,
    );
    assert.equal(
      resolveSafeOwnCheckLine({ own: 'verified', effective: 'unchecked', own_check_line: 'spoofed text', stale: false }),
      OWN_CHECK_VERIFIED_LINE,
    );
    assert.equal(
      resolveSafeOwnCheckLine({ own: 'checked_numerically', effective: 'unchecked', own_check_line: OWN_CHECK_EQUATIONS_VERIFIED_LINE, stale: false }),
      null,
    );
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('VAC-2 & VAC-3: trivial outcome, Show check record toggle, producing sympy_eval code, source_expr, and clarified own_check_line in UI', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    // Add producing sympy_eval code on call-1 and source_expr on lhs_a@1.
    view.receipts[0].code = 'lhs_a = eps * m * omega**2 * (3 / (4 * m**2 * omega**2))\nsave_expr("lhs_a", lhs_a)';
    view.handles[0].source_expr = 'eps * m * omega**2 * (3 / (4 * m**2 * omega**2))';
    // Add trivial check on e-4 for solve_why test.
    view.checks.push({
      check_id: 'chk-trivial',
      claim_sha256: view.steps.find((s) => s.entry_id === 'e-4').claim_sha256,
      claim_index: 0,
      entry_id: 'e-4',
      math_sha256: view.setup.math_sha256,
      name: 'identity_symbolic',
      outcome: 'trivial',
      asked_by: 'host',
      receipt: { call_id: 'call-1', result_seq: 8 },
      evidence: { reason: 'both sides identical' },
    });

    const wsHost = dom.makeEl('div');
    const ws = createSolveWorkspace(wsHost, view, { katex: null });

    // Clarified own_check_line appears on e-6 and e-10 because they have non-empty prose details.
    const e6Card = wsHost.querySelector('[data-entry-id="e-6"]');
    assert.equal(e6Card.querySelector('[data-badge-role="own-check"]')?.textContent, OWN_CHECK_EQUATIONS_VERIFIED_LINE);

    // Step card expandable receipt displays Python code and handle source_expr.
    const e4Card = wsHost.querySelector('[data-entry-id="e-4"]');
    assert.ok(e4Card.querySelector('.solve-receipt-details'), 'receipt with code/source_expr must be expandable');
    assert.ok(e4Card.querySelector('[data-testid="solve-receipt-code"]')?.textContent.includes('save_expr("lhs_a"'));
    assert.ok(e4Card.querySelector('[data-testid="solve-handle-source"]')?.textContent.includes('lhs_a@1: eps * m'));

    // Why popover: trivial outcome badge, "Show check record" / "Hide check record" button, producing code block, and clarified lowering text.
    const whyHost = dom.makeEl('div');
    const pop = createSolveWhyPopover(whyHost, {
      step: view.steps.find((s) => s.entry_id === 'e-4'),
      view,
      sessionId: 's-1',
    });
    assert.equal(whyHost.querySelector('[data-check-outcome="trivial"]')?.textContent, 'Identical sides (nothing checked)');
    const toggleBtn = whyHost.querySelector('[data-action="toggle-code"]');
    assert.equal(toggleBtn.textContent, 'Show check record');
    toggleBtn.dispatch('click');
    assert.equal(whyHost.querySelector('[data-action="toggle-code"]').textContent, 'Hide check record');
    assert.ok(whyHost.querySelector('[data-testid="solve-why-code"]'));
    const prodCode = whyHost.querySelector('[data-testid="solve-why-producing-code"]');
    assert.ok(prodCode?.textContent.includes('save_expr("lhs_a"'));

    // Update to lowered step e-6 with prose details -> clarified own check line in Why popover.
    pop.update({ step: view.steps.find((s) => s.entry_id === 'e-6'), view, sessionId: 's-1' });
    assert.ok(whyHost.querySelector('[data-testid="solve-why-lowering"]')?.textContent.includes(OWN_CHECK_EQUATIONS_VERIFIED_LINE));

    // Step with empty details keeps plain 'own check: verified'.
    const e6NoDetails = { ...view.steps.find((s) => s.entry_id === 'e-6'), details: '' };
    pop.update({ step: e6NoDetails, view, sessionId: 's-1' });
    const loweringText = whyHost.querySelector('[data-testid="solve-why-lowering"]')?.textContent || '';
    assert.ok(loweringText.includes('own check: verified'));
    assert.ok(!loweringText.includes(OWN_CHECK_EQUATIONS_VERIFIED_LINE));

    pop.destroy();
    ws.destroy();
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('GATE-3 solve_state.js buildSetupSubmission renames final_different_model_review and target description when acceptSameModel is set', () => {
  const state = createInitialSolveState({
    statement: 'Prove X',
    taskKind: 'prove',
    target: { description: 'Structured proof reviewed by an independent model', form: 'proof' },
    solver: { provider: 'openai', model: 'gpt-5' },
    verifier: { provider: 'openai', model: 'gpt-5' },
    acceptSameModel: true,
  });
  const payload = buildSetupSubmission(state);
  assert.equal(payload.target.description, 'Structured proof with model review');
  assert.ok(!payload.done.some((c) => c.id === 'final_different_model_review'));
  const renamed = payload.done.find((c) => c.id === 'final_model_review');
  assert.ok(renamed);
  assert.equal(renamed.params.tier, 'model_agreed_same_model');
  assert.equal(renamed.params.require_different_model, false);
});
