// Unit & UI tests for Solve prefill merge semantics (F2) and dom_focus.js edge cases (F1).
import assert from 'node:assert/strict';
import test from 'node:test';

import { keepFocus } from '../assets/dom_focus.js';
import { createSolveSetupView } from '../assets/solve_setup.js';
import {
  applyPrefillToState,
  applySolveViewToState,
  createInitialSolveState,
  markDoneToggled,
  markFieldOwned,
  markSymbolEdited,
  markSymbolRemoved,
  prefillTaskKindOverride,
  switchTaskKind,
} from '../assets/solve_state.js';

function makeFocusDom() {
  let doc = null;
  const makeNode = (tag = 'div') => {
    let text = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const node = {
      tagName: String(tag).toUpperCase(),
      ownerDocument: doc,
      children: [],
      style: {},
      type: '',
      value: '',
      placeholder: '',
      checked: false,
      selected: false,
      disabled: false,
      rows: 0,
      selectionStart: 0,
      selectionEnd: 0,
      selectionDirection: 'none',
      scrollTop: 0,
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
      setAttribute(k, v) { attrs.set(String(k), String(v)); },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      contains(target) {
        if (!target) return false;
        if (target === this) return true;
        return this.children.some((c) => typeof c?.contains === 'function' && c.contains(target));
      },
      focus() { if (doc) doc.activeElement = this; },
      setSelectionRange(start, end, dir = 'none') {
        this.selectionStart = start;
        this.selectionEnd = end;
        this.selectionDirection = dir;
      },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(n) { this.children.push(n); return n; },
      replaceChildren(...nodes) {
        if (doc?.activeElement && doc.activeElement !== this && this.contains(doc.activeElement)) {
          doc.activeElement = null;
        }
        this.children = [...nodes];
        text = '';
      },
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
  doc = { activeElement: null, createElement: (tag) => makeNode(tag), createElementNS: (_ns, tag) => makeNode(tag) };
  globalThis.document = doc;
  return { doc, makeEl: (tag = 'div') => makeNode(tag), restore() { globalThis.document = prevDoc; } };
}

test('prefill merges symbols without overwriting edited, added, blank, or deleted symbols', () => {
  let st = createInitialSolveState();
  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'calculate',
      symbols: [
        { name: 'phi', assumptions: ['real'] },
        { name: 'm', assumptions: ['real', 'positive'] },
        { name: 'x', assumptions: ['real'] },
      ],
    },
  });
  assert.deepEqual(st.symbols.map((s) => s.name), ['phi', 'm', 'x']);

  // Edit phi assumptions, delete m, add custom g and an in-progress blank row.
  st.symbols[0].assumptions = ['complex'];
  markSymbolEdited(st, 'phi');
  markSymbolRemoved(st, 'm');
  st.symbols.splice(1, 1);
  st.symbols.push({ name: 'g', assumptions: ['positive'] });
  markSymbolEdited(st, 'g');
  st.symbols.push({ name: '', assumptions: ['real'] });

  // Next prefill drops unedited x, re-offers m and phi (real), and adds lambda.
  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'calculate',
      symbols: [
        { name: 'phi', assumptions: ['real'] },
        { name: 'm', assumptions: ['real', 'positive'] },
        { name: 'lambda', assumptions: ['real', 'positive'] },
      ],
    },
  });

  assert.deepEqual(st.symbols, [
    { name: 'phi', assumptions: ['complex'] },
    { name: 'g', assumptions: ['positive'] },
    { name: '', assumptions: ['real'] },
    { name: 'lambda', assumptions: ['real', 'positive'] },
  ]);
  assert.ok(st.removedSymbols.includes('m'));
});

test('prefill never overwrites user-edited dimensions, conventions, taskKind, target, budgets, or review', () => {
  let st = createInitialSolveState();
  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'prove',
      target: { description: 'Server proof', form: 'proof' },
      conventions: 'Euclidean',
      dimensions: { phi: { mass: 1 } },
      budgets: { max_model_calls: 60 },
      review: 'prose_only',
    },
  });
  assert.equal(st.taskKind, 'prove');
  assert.equal(st.conventions, 'Euclidean');
  assert.equal(prefillTaskKindOverride(st), null);

  // User edits every field (including clearing conventions to empty string).
  switchTaskKind(st, 'calculate');
  assert.equal(prefillTaskKindOverride(st), 'calculate');
  st.target = { description: 'Custom closed form', form: 'closed_form', order: 'eps^3' };
  markFieldOwned(st, 'target');
  st.conventions = '';
  markFieldOwned(st, 'conventions');
  st.dimensions = [{ symbol: 'phi', base: 'length', power: '-1' }];
  markFieldOwned(st, 'dimensions');
  st.budgets = { ...st.budgets, max_model_calls: 25 };
  markFieldOwned(st, 'budgets');
  st.review = 'every_step';
  markFieldOwned(st, 'review');

  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'prove',
      target: { description: 'Overwritten', form: 'proof' },
      conventions: 'Euclidean\nhbar = 1',
      dimensions: { phi: { mass: 1 }, m: { mass: 1 } },
      budgets: { max_model_calls: 99 },
      review: 'prose_only',
    },
  });

  assert.equal(st.taskKind, 'calculate');
  assert.equal(st.target.order, 'eps^3');
  assert.equal(st.conventions, '');
  assert.deepEqual(st.dimensions, [{ symbol: 'phi', base: 'length', power: '-1' }]);
  assert.equal(st.budgets.max_model_calls, 25);
  assert.equal(st.review, 'every_step');
});

test('prefill preserves unchecked done criteria by id and clears done ownership on kind switch', () => {
  let st = createInitialSolveState();
  const flagsCrit = st.done.find((c) => c.id === 'no_open_flags');
  flagsCrit.enabled = false;
  markDoneToggled(st, 'no_open_flags', false);

  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'calculate',
      done: [
        { id: 'final_result_tier', kind: 'final_tier_at_least', params: { tier: 'verified' } },
        { id: 'no_open_flags', kind: 'no_open_flags', params: {} },
        { id: 'show_vanishes_to_order', kind: 'named_check_passed', params: { check: 'vanishes_to_order' } },
      ],
    },
  });

  assert.equal(st.done.find((c) => c.id === 'no_open_flags')?.enabled, false);
  assert.equal(st.done.find((c) => c.id === 'show_vanishes_to_order')?.enabled, true);

  // Switching kind clears done ownership and resets criteria for the new kind.
  switchTaskKind(st, 'prove');
  assert.equal(st.userOwned.done, false);
  assert.equal(st.done.find((c) => c.id === 'no_open_flags')?.enabled, true);

  st = applyPrefillToState(st, {
    prefill: {
      task_kind: 'prove',
      done: [
        { id: 'final_different_model_review', kind: 'final_tier_at_least', params: { tier: 'model_agreed', require_different_model: true } },
        { id: 'no_open_flags', kind: 'no_open_flags', params: {} },
      ],
    },
  });
  assert.equal(st.done.find((c) => c.id === 'no_open_flags')?.enabled, true);
});

test('initial.setup and applySolveViewToState hydration mark fields user-owned and send task_kind override', async () => {
  // 1. initial.setup sets all setup fields user-owned.
  let fromInit = createInitialSolveState({
    setup: { statement: 'Saved', revision: 1 },
    taskKind: 'prove',
    symbols: [{ name: 'phi', assumptions: ['complex'] }],
    conventions: 'Saved conv',
    dimensions: { phi: { mass: 2 } },
    done: [{ id: 'final_different_model_review', kind: 'final_tier_at_least', params: { tier: 'model_agreed' } }],
    budgets: { max_model_calls: 17 },
    review: 'every_step',
  });
  fromInit = applyPrefillToState(fromInit, {
    prefill: {
      task_kind: 'calculate',
      symbols: [{ name: 'phi', assumptions: ['real'] }, { name: 'm', assumptions: ['real'] }],
      conventions: 'Different conv',
      dimensions: { m: { mass: 1 } },
      done: [{ id: 'final_result_tier', kind: 'final_tier_at_least', params: { tier: 'verified' } }],
      budgets: { max_model_calls: 60 },
      review: 'prose_only',
    },
  });
  assert.equal(fromInit.taskKind, 'prove');
  assert.deepEqual(fromInit.symbols, [{ name: 'phi', assumptions: ['complex'] }]);
  assert.equal(fromInit.conventions, 'Saved conv');
  assert.deepEqual(fromInit.dimensions, [{ symbol: 'phi', base: 'mass', power: '2' }]);
  assert.deepEqual(fromInit.done.map((c) => c.id), ['final_different_model_review']);
  assert.equal(fromInit.budgets.max_model_calls, 17);
  assert.equal(fromInit.review, 'every_step');

  // 2. applySolveViewToState revision hydration + UI triggerPrefill sends task_kind override.
  const dom = makeFocusDom();
  try {
    const prefillBodies = [];
    const host = dom.makeEl('div');
    const view = createSolveSetupView(host, {
      onPrefill: async (body) => {
        prefillBodies.push(body);
        return {
          prefill: {
            task_kind: 'calculate',
            symbols: [{ name: 'x', assumptions: ['real'] }],
            conventions: 'Overwriting conv',
            dimensions: { x: { length: 1 } },
            done: [{ id: 'no_open_flags', kind: 'no_open_flags', params: {} }],
            budgets: { max_model_calls: 60 },
            review: 'prose_only',
          },
        };
      },
    });

    // Before picking a kind, task_kind sent to /solve/prefill is null.
    await view.triggerPrefill();
    assert.equal(prefillBodies[0].task_kind, null);

    // Hydrate revision 1, enter Edit setup, trigger prefill -> sends hydrated task_kind & preserves hydrated setup.
    view.applyView({
      setup: {
        revision: 1,
        statement: 'Prove $f(x) > 0$',
        task_kind: 'prove',
        target: { description: 'Hydrated proof', form: 'proof' },
        conventions: 'Hydrated conv',
        symbols: [{ name: 'f', assumptions: ['real'] }],
        dimensions: { f: { mass: 1 } },
        done: [{ id: 'final_different_model_review', kind: 'final_tier_at_least', params: { tier: 'model_agreed' } }],
        budgets: { max_model_calls: 33 },
        review: 'every_step',
      },
      steps: [],
    });
    host.querySelector('[data-action="edit-setup"]').dispatch('click');
    await view.triggerPrefill();
    assert.equal(prefillBodies[1].task_kind, 'prove');
    const st = view.getState();
    assert.equal(st.taskKind, 'prove');
    assert.deepEqual(st.symbols, [{ name: 'f', assumptions: ['real'] }]);
    assert.equal(st.conventions, 'Hydrated conv');
    assert.deepEqual(st.dimensions, [{ symbol: 'f', base: 'mass', power: '1' }]);
    assert.deepEqual(st.done.map((c) => c.id), ['final_different_model_review']);
    assert.equal(st.budgets.max_model_calls, 33);
    assert.equal(st.review, 'every_step');
  } finally {
    dom.restore();
  }
});

test('keepFocus places caret at end when value changes and avoids CSS selector injection', () => {
  const dom = makeFocusDom();
  try {
    const root = dom.makeEl('div');
    const trickyLabel = 'Dimension "1"] [x="y';
    const inp1 = dom.makeEl('input');
    inp1.type = 'text';
    inp1.setAttribute('aria-label', trickyLabel);
    inp1.value = 'old';
    root.append(inp1);
    inp1.focus();
    inp1.setSelectionRange(1, 2, 'forward');

    keepFocus(root, () => {
      const inp2 = dom.makeEl('input');
      inp2.type = 'text';
      inp2.setAttribute('aria-label', trickyLabel);
      inp2.value = 'updated-value';
      root.replaceChildren(inp2);
    });

    assert.equal(dom.doc.activeElement, root.children[0]);
    assert.equal(dom.doc.activeElement.selectionStart, 'updated-value'.length);
    assert.equal(dom.doc.activeElement.selectionEnd, 'updated-value'.length);
  } finally {
    dom.restore();
  }
});
