// Tests for solve_result_sheet.js, abandoned steps outside final-answer closure,
// parameter-based criterion descriptions, live result_sheet status_line, and model-authored sections.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  DONE_LABEL,
  DONE_OPEN_ITEMS_LABEL,
  NOT_DONE_LABEL,
  computeFinalClosureIds,
  createSolveWorkspace,
  describeCriterion,
  renderSolveResultSheet,
  resolveDoneHeading,
} from '../assets/solve_steps.js';
import { renderSolveResultSheet as renderResultSheetDirect } from '../assets/solve_result_sheet.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const repoRoot = path.resolve(crateRoot, '..', '..');
const assetsDir = path.join(crateRoot, 'assets');
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

test('assets.rs registers /assets/solve_result_sheet.js and solve_result_sheet.js has zero innerHTML', () => {
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  assert.ok(assetsRs.includes('"/assets/solve_result_sheet.js"'));
  const raw = fs.readFileSync(path.join(assetsDir, 'solve_result_sheet.js'), 'utf8');
  const codeOnly = raw.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  assert.ok(!/\b(innerHTML|outerHTML|insertAdjacentHTML)\b/.test(codeOnly));
  assert.equal(renderSolveResultSheet, renderResultSheetDirect);
  assert.equal(DONE_LABEL, 'Done');
  assert.equal(DONE_OPEN_ITEMS_LABEL, 'Done with open items');
  assert.equal(NOT_DONE_LABEL, 'Not done');
  assert.equal(resolveDoneHeading(null), NOT_DONE_LABEL);
});

test('GATE-1 & GATE-2: outside-closure steps collapse, Own checks restrict to closure, and outside note flags assumptions/priors', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    // Add an outside-closure AssumedByModel step and an earlier final_answer step outside closure.
    view.steps.push(
      {
        entry_id: 'e-11',
        entry_type: 'assumption',
        origin: 'solver',
        summary: 'Abandoned assumption outside closure',
        details: 'Never connected to final answer.',
        depends_on: [],
        derived_depends_on: [],
        claims: [],
        superseded_by: null,
        badge: { entry_id: 'e-11', own: 'assumed_by_model', effective: 'assumed_by_model', label: 'Assumed by model', flags: [], stale: false },
      },
      {
        entry_id: 'e-12',
        entry_type: 'final_answer',
        origin: 'solver',
        summary: 'Earlier final answer outside closure',
        details: '',
        depends_on: ['e-1'],
        derived_depends_on: [],
        claims: [],
        superseded_by: null,
        badge: { entry_id: 'e-12', own: 'unchecked', effective: 'unchecked', label: 'Unchecked', flags: [], stale: false },
      },
    );
    view.done_history = [{ final_entry_id: 'e-12', status: 'finished_open_items' }, { final_entry_id: 'e-10', status: 'finished_open_items' }];

    // Closure of e-10 in fixture: e-1, e-2, e-3, e-4, e-6, e-10 (6 steps).
    // Active outside closure: e-5, e-7, e-8, e-9, e-11, e-12 (6 steps).
    const closure = computeFinalClosureIds(view.steps, 'e-10');
    assert.deepEqual([...closure].sort(), ['e-1', 'e-10', 'e-2', 'e-3', 'e-4', 'e-6']);

    const host = dom.makeEl('div');
    const ws = createSolveWorkspace(host, view, { katex: null });

    const outsideGroup = host.querySelector('[data-testid="solve-outside-final-group"]');
    assert.ok(outsideGroup, 'outside-closure steps must render in collapsed details group');
    assert.equal(
      outsideGroup.querySelector('.solve-outside-final-summary')?.textContent,
      'Not used by the final answer (6 steps)',
    );
    const outsideCards = outsideGroup.querySelectorAll('[data-outside-final="true"]');
    assert.equal(outsideCards.length, 6);
    assert.ok(outsideCards[0].textContent.includes('not used by the final answer'));

    // Main steps list should have only the 6 closure cards directly (plus the details group).
    const stepsList = host.querySelector('[data-testid="solve-steps-list"]');
    const directCards = stepsList.children.filter((n) => n.getAttribute?.('data-testid') === 'solve-step-card');
    assert.equal(directCards.length, 6);

    // Own checks count in result sheet must reflect ONLY closure steps (1 Given, 1 Found, 1 Assumed by model, 3 Verified).
    // e-5 (Numerically checked), e-7 (Model agreed (same model)), e-8 (Checked by you), e-9 (Failed) are excluded!
    const countsText = host.querySelector('.solve-result-counts')?.textContent || '';
    assert.ok(!countsText.includes('Numerically checked'), `outside step leaked into Own checks: ${countsText}`);
    assert.ok(!countsText.includes('Checked by you'), `outside step leaked into Own checks: ${countsText}`);
    assert.ok(!countsText.includes('Failed'), `outside step leaked into Own checks: ${countsText}`);
    assert.ok(countsText.includes('3 Verified'), `expected 3 Verified in closure, got: ${countsText}`);

    // Result sheet summary note lists outside count, outside AssumedByModel (e-11), and earlier final (e-12).
    const outsideNote = host.querySelector('[data-testid="solve-outside-final-note"]')?.textContent || '';
    assert.ok(outsideNote.includes('6 active steps not used by the final answer'));
    assert.ok(outsideNote.includes('e-11'));
    assert.ok(outsideNote.includes('e-12'));

    ws.destroy();
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('GATE-3: describeCriterion formats from parameters, never raw IDs, and shows Different model only when required', () => {
  const sameModelCrit = {
    id: 'final_different_model_review',
    met: true,
    evidence: {
      kind: 'final_tier_at_least',
      required: 'model_agreed_same_model',
      require_different_model: false,
    },
  };
  const sameModelText = describeCriterion(sameModelCrit);
  assert.ok(!sameModelText.includes('final_different_model_review'));
  assert.ok(!sameModelText.includes('Different model'));
  assert.ok(sameModelText.includes('Model agreed (same model)'));

  const diffModelCrit = {
    id: 'final_different_model_review',
    met: false,
    evidence: {
      kind: 'final_tier_at_least',
      required: 'model_agreed',
      require_different_model: true,
      re_rooted_past_active_final: ['e-7'],
    },
  };
  const diffModelText = describeCriterion(diffModelCrit);
  assert.ok(diffModelText.includes('Different model'));
  assert.ok(diffModelText.includes('e-7'));
  assert.ok(!diffModelText.includes('final_different_model_review'));

  assert.equal(
    describeCriterion({ id: 'c-1', evidence: { kind: 'closure_tier_at_least', required: 'checked_numerically', require_different_model: false } }),
    'All closure steps: at least Numerically checked',
  );
  assert.equal(
    describeCriterion({ id: 'c-2', evidence: { kind: 'named_check_passed', check: 'dimensions', on: 'all_results' } }),
    'Check "dimensions" passed (on all result steps)',
  );
  assert.equal(
    describeCriterion({ id: 'c-3', evidence: { kind: 'no_open_flags' } }),
    'No unacknowledged assumptions or open flags',
  );
  assert.equal(
    describeCriterion({ id: 'c-4', evidence: { kind: 'user_confirms', text: 'Confirm TeX' } }),
    'Confirm TeX',
  );
});

test('GATE-9: result_sheet.status_line drives Not done heading, model labels & open_items render, and summary falls back when no claims', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    // 1. Live result_sheet.status_line overrides stale done.status === finished_open_items -> "Not done: 1 step failed"
    view.result_sheet = { status_line: 'Not done: 1 step failed' };
    // 2. Final step without claims and empty done.summary_markdown falls back to finalStep.summary.
    const finalStep = view.steps.find((s) => s.entry_id === 'e-10');
    finalStep.claims = [];
    finalStep.summary = 'Prose final answer fallback text';
    view.done.summary_markdown = '';
    view.done.open_items = ['Check boundary conditions', 'Confirm sign convention'];

    const host = dom.makeEl('div');
    const ws = createSolveWorkspace(host, view, { katex: null });

    assert.equal(host.querySelector('[data-testid="solve-done-heading"]')?.textContent, 'Not done: 1 step failed');
    assert.ok(host.querySelector('[data-testid="solve-result-answer"]')?.textContent.includes('Prose final answer fallback text'));
    const sumWrap = host.querySelector('.solve-result-summary-wrap');
    assert.ok(sumWrap?.textContent.includes('Summary (written by model)'));
    assert.ok(host.querySelector('[data-testid="solve-result-summary"]')?.textContent.includes('Prose final answer fallback text'));

    const openSec = host.querySelector('[data-testid="solve-result-open-items"]');
    assert.ok(openSec, 'open_items section must render');
    assert.ok(openSec.textContent.includes('Open items (written by model)'));
    assert.ok(openSec.textContent.includes('Check boundary conditions'));
    assert.ok(openSec.textContent.includes('Confirm sign convention'));

    ws.destroy();
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});
