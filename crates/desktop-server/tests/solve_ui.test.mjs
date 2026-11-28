// Node tests for Solve S13 UI modules (solve_badges.js, solve_steps.js, solve_why.js, solve_exprs.js).
// Covers S2 tier mapping, R5 handle-only formulas, R15 Done sheet, R19 recheck diff, R22 wording sweep,
// all 7 Why actions, tab registry, F6 graph/run_bar integration, new_menu Solve helper, and XSS safety.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { buildModeMenuEntries, enableSolveButton } from '../assets/new_menu.js';
import {
  FLAG_OCR_SOURCE, FLAG_SAME_MODEL, FLAG_TRANSCRIPTION_UNCONFIRMED,
  LEGEND_VERIFIED_EXPLANATION, OWN_CHECK_VERIFIED_LINE, SOLVE_TIER_LABELS,
  adaptSolveGraphView, computeFinalClosureIds, createSolveBadgeGroup, createSolveLegend, mapBadgeToUiSpec,
} from '../assets/solve_badges.js';
import { buildHandleLookup, createSolveExprsView, renderClaimFormula } from '../assets/solve_exprs.js';
import {
  DONE_LABEL, DONE_OPEN_ITEMS_LABEL, NOT_DONE_LABEL,
  createSolveWorkspace, registerSolveTab, resolveDoneHeading, unregisterSolveTab,
} from '../assets/solve_steps.js';
import {
  RECHECK_CHANGED_WARNING, REVIEW_DISABLED_NOTICE, createSolveWhyPopover, partitionStepChecks,
} from '../assets/solve_why.js';

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
      tabIndex: 0,
      clientWidth: 640,
      clientHeight: 400,
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
          if (this.tagName === 'SELECT' && n?.tagName === 'OPTION' && !this.value) this.value = n.value;
        }
      },
      appendChild(n) { this.append(n); return n; },
      removeChild(n) {
        const i = this.children.indexOf(n);
        if (i >= 0) { this.children.splice(i, 1); n.parentNode = null; }
        return n;
      },
      remove() { if (this.parentNode) this.parentNode.removeChild(this); },
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
      removeEventListener(type, fn) {
        const list = listeners.get(type);
        if (!list) return;
        const idx = list.indexOf(fn);
        if (idx >= 0) list.splice(idx, 1);
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

function walkTree(node, fn) {
  fn(node);
  for (const child of node.children || []) walkTree(child, fn);
}

test('S13a solve_badges maps all 11 S2 tiers, preserves server rests-on flags verbatim, and enforces R22', () => {
  const dom = makeFakeDom();
  try {
    assert.equal(SOLVE_TIER_LABELS.verified, 'Verified');
    assert.equal(SOLVE_TIER_LABELS.checked_numerically, 'Numerically checked');
    assert.equal(SOLVE_TIER_LABELS.cited, 'Found');
    assert.equal(Object.keys(SOLVE_TIER_LABELS).length, 11);

    for (const tier of Object.keys(SOLVE_TIER_LABELS)) {
      const spec = mapBadgeToUiSpec({ own: tier, effective: tier, label: SOLVE_TIER_LABELS[tier] });
      assert.equal(spec.text, SOLVE_TIER_LABELS[tier]);
      assert.ok(spec.icon && spec.tone);
      if (tier !== 'verified') assert.ok(!/\bverified\b/i.test(spec.text));
      assert.ok(!/\bproved\b|\bnovel\b/i.test(spec.text));
    }

    assert.equal(
      mapBadgeToUiSpec({ own: 'checked_numerically', effective: 'checked_numerically', label: 'Verified (spoofed)' }).text,
      'Numerically checked',
    );

    const jumps = [];
    let clicked = 0;
    const group = createSolveBadgeGroup({
      own: 'verified', effective: 'unchecked', label: 'Unchecked',
      own_check_line: OWN_CHECK_VERIFIED_LINE, rests_on_line: 'rests on e-4 (unchecked)',
      lowered_by: ['e-4'], sublabels: ['to O(eps^0)'],
      flags: ['rests on 2 assumptions made by the model', FLAG_TRANSCRIPTION_UNCONFIRMED, FLAG_SAME_MODEL, FLAG_OCR_SOURCE],
    }, { onBadgeClick: () => { clicked += 1; }, onSelectStep: (id) => jumps.push(id) });

    assert.equal(group.querySelector('[data-badge-role="primary"]')?.textContent, 'Unchecked');
    assert.equal(group.querySelector('[data-badge-role="own-check"]')?.textContent, OWN_CHECK_VERIFIED_LINE);
    assert.ok(group.querySelector('[data-badge-role="rests-on"]')?.textContent.includes('rests on e-4 (unchecked)'));
    assert.deepEqual(group.querySelectorAll('.solve-flag-chip').map((n) => n.textContent), [
      'rests on 2 assumptions made by the model', FLAG_TRANSCRIPTION_UNCONFIRMED, FLAG_SAME_MODEL, FLAG_OCR_SOURCE,
    ]);
    group.querySelector('[data-badge-role="primary"]').dispatch('click');
    group.querySelector('[data-jump-step="e-4"]').dispatch('click');
    assert.equal(clicked, 1);
    assert.deepEqual(jumps, ['e-4']);

    const legendNoVer = createSolveLegend([], [{ entry_id: 'e-1', badge: { own: 'checked_numerically', effective: 'checked_numerically' } }]);
    assert.ok(!/\bverified\b/i.test(legendNoVer.textContent));
    const legendWithVer = createSolveLegend([], [{ entry_id: 'e-1', badge: { own: 'verified', effective: 'verified' } }]);
    assert.ok(legendWithVer.querySelector('[data-testid="solve-legend-verified-note"]')?.textContent.includes(LEGEND_VERIFIED_EXPLANATION));

    const steps = [
      { entry_id: 'e-1', depends_on: [], derived_depends_on: [] },
      { entry_id: 'e-2', depends_on: ['e-1'], derived_depends_on: [] },
      { entry_id: 'e-side', depends_on: ['e-1'], derived_depends_on: [] },
      { entry_id: 'e-3', depends_on: [], derived_depends_on: ['e-2'], entry_type: 'final_answer' },
    ];
    assert.deepEqual([...computeFinalClosureIds(steps, 'e-3')].sort(), ['e-1', 'e-2', 'e-3']);
    const adapted = adaptSolveGraphView(
      { steps, graph: { nodes: steps.map((s) => ({ id: s.entry_id, grade: 'code' })), edges: [{ src: 'e-1', dst: 'e-2' }, { src: 'e-1', dst: 'e-side' }, { src: 'e-2', dst: 'e-3' }], legend: [] } },
      { closureOfFinal: true },
    );
    assert.deepEqual(adapted.nodes.map((n) => n.id).sort(), ['e-1', 'e-2', 'e-3']);
    assert.equal(adapted.edges.length, 2);
  } finally {
    dom.restore();
  }
});

test('S13b solve_steps enforces R5 handle-only formulas, R15 Done sheet, F6 graph closure filter, and registerSolveTab', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    // Inject a hostile model-authored step.latex field on e-4; R5 must ignore it completely!
    view.steps.find((s) => s.entry_id === 'e-4').latex = 'SPOOFED_MODEL_LATEX_SHOULD_NEVER_RENDER';

    const renderedTex = [];
    const fakeKatex = {
      render(tex, target) {
        renderedTex.push(tex);
        target.textContent = `[KATEX:${tex}]`;
      },
    };
    const acks = [];
    let extMounted = 0;
    let extDestroyed = 0;
    registerSolveTab({
      id: 'artifacts',
      label: 'Artifacts',
      mount(panel) {
        extMounted += 1;
        panel.append(document.createElement('div'));
        return { destroy() { extDestroyed += 1; } };
      },
    });

    const host = dom.makeEl('div');
    const ws = createSolveWorkspace(host, view, {
      sessionId: 's-fixture',
      katex: fakeKatex,
      onAck: (step, payload) => acks.push({ step: step.entry_id, payload }),
    });

    assert.equal(dom.innerHtmlWrites, 0);
    // R5: check handle LaTeX rendered via KaTeX, and spoofed step.latex is completely absent.
    assert.ok(!host.textContent.includes('SPOOFED_MODEL_LATEX_SHOULD_NEVER_RENDER'));
    assert.ok(renderedTex.some((t) => t.includes('\\frac{3 \\epsilon}{4 m}')), 'handle latex must render via katex');

    // R5 non-portable / missing handle fails closed without falling back.
    const tmpBox = dom.makeEl('div');
    const okNonPort = renderClaimFormula(tmpBox, { lhs: 'raw_op@1', rhs: 'rhs_a@1', relation: 'eq' }, buildHandleLookup(view.handles), fakeKatex);
    assert.equal(okNonPort, false);
    assert.ok(tmpBox.querySelector('[data-handle-unavailable="true"]'));

    // R15 Result sheet strict status check.
    assert.equal(host.querySelector('[data-testid="solve-done-heading"]')?.textContent, DONE_OPEN_ITEMS_LABEL);
    assert.ok(host.querySelector('[data-testid="solve-result-summary"]')?.textContent.endsWith('(model)'));
    assert.equal(resolveDoneHeading(null), NOT_DONE_LABEL);
    assert.equal(resolveDoneHeading({ status: 'finished', criteria: [{ id: 'c1', met: true }] }), DONE_LABEL);
    assert.equal(resolveDoneHeading({ status: 'finished', criteria: [{ id: 'c1', met: false }] }), DONE_OPEN_ITEMS_LABEL);
    assert.equal(resolveDoneHeading({ status: 'finished_open_items', criteria: [{ id: 'c1', met: true }] }), DONE_OPEN_ITEMS_LABEL);

    // One-click Acknowledge assumption in Result sheet (D10).
    host.querySelector('[data-ack-assumption="e-3"]').dispatch('click');
    assert.equal(acks.length, 1);
    assert.equal(acks[0].payload.kind, 'assumption');
    assert.equal(acks[0].payload.entry_id, 'e-3');

    // F6 run_bar & graph_view mounted directly + "closure of the final answer" filter.
    assert.ok(ws.getRunBar()?.element.getAttribute('data-run-state') === 'done_open_items');
    assert.equal(ws.getGraphView()?.getLayout().nodes.length, 10);
    const closureBox = host.querySelector('[data-testid="solve-closure-filter"]');
    closureBox.checked = true;
    closureBox.dispatch('change');
    const closureIds = ws.getGraphView().getLayout().nodes.map((n) => n.id).sort();
    assert.deepEqual(closureIds, ['e-1', 'e-10', 'e-2', 'e-3', 'e-4', 'e-6']);

    // Registered S18a Artifacts tab mounts and cleans up cleanly on tab switch.
    assert.ok(ws.getTabs().some((t) => t.id === 'artifacts'));
    ws.selectTab('artifacts');
    assert.equal(extMounted, 1);
    ws.selectTab('expressions');
    assert.equal(extDestroyed, 1);
    assert.ok(host.querySelector('[data-testid="solve-exprs-view"]'));

    ws.destroy();
  } finally {
    unregisterSolveTab('artifacts');
    dom.restore();
  }
});

test('S13b solve_why supports stale checks, code toggle, R19 recheck warning, and all 7 actions + F16b hook', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    const stepE4 = view.steps.find((s) => s.entry_id === 'e-4');
    const stepE6 = view.steps.find((s) => s.entry_id === 'e-6');
    const stepE2 = view.steps.find((s) => s.entry_id === 'e-2');

    const part = partitionStepChecks(stepE4, view.checks, view.setup.math_sha256);
    assert.equal(part.current.length, 1);
    assert.equal(part.stale.length, 1);
    assert.equal(part.stale[0].check_id, 'chk-8-stale');

    const events = { fork: [], attach: [], ack: [], addCheck: [], rerun: [], corrected: [] };
    const host = dom.makeEl('div');
    const pop = createSolveWhyPopover(host, {
      step: stepE4,
      view,
      sessionId: 's-1',
      onFork: (s) => events.fork.push(s.entry_id),
      onAttachSource: (s, p) => events.attach.push({ id: s.entry_id, p }),
      onAck: (s, p) => events.ack.push(p),
      onAddCheck: (s, p) => events.addCheck.push(p),
      onRerunChecks: (req) => { events.rerun.push(req); return { changed: true }; },
      onStepCorrected: (payload) => events.corrected.push(payload),
    });

    // Current + Stale check cards visible; code toggle reveals code_sha256.
    assert.ok(host.querySelector('[data-checks-group="current"]'));
    assert.ok(host.querySelector('[data-checks-group="stale"]'));
    host.querySelector('[data-action="toggle-code"]').dispatch('click');
    assert.ok(host.querySelector('[data-testid="solve-why-code"]')?.textContent.includes('d001ffff'));

    // R19: recheck returning changed: true displays exact warning.
    host.querySelector('[data-action="recheck-one"]').dispatch('click');
    assert.equal(host.querySelector('[data-testid="solve-recheck-warning"]')?.textContent, RECHECK_CHANGED_WARNING);

    // 7 actions: Review with another model gracefully disabled until S14 merges.
    const reviewBtn = host.querySelector('[data-why-action="review_model"]');
    assert.equal(reviewBtn.disabled, true);
    assert.equal(reviewBtn.getAttribute('aria-disabled'), 'true');
    assert.equal(reviewBtn.title, REVIEW_DISABLED_NOTICE);

    // Fork, Ack (hand_checked), Rerun all.
    host.querySelector('[data-why-action="fork"]').dispatch('click');
    host.querySelector('[data-ack-kind="hand_checked"]').dispatch('click');
    host.querySelector('[data-why-action="rerun_checks"]').dispatch('click');
    assert.deepEqual(events.fork, ['e-4']);
    assert.equal(events.ack[0].kind, 'hand_checked');
    assert.equal(events.rerun.length, 2);

    // Attach source inline form.
    host.querySelector('[data-why-action="attach_source"]').dispatch('click');
    const attachForm = host.querySelector('[data-testid="solve-attach-source-form"]');
    attachForm.querySelector('[name="source"]').value = 'arxiv:2401.00001';
    attachForm.querySelector('[name="start_line"]').value = '10';
    attachForm.querySelector('[name="end_line"]').value = '12';
    attachForm.querySelector('[name="quote"]').value = 'exact quote';
    attachForm.dispatch('submit');
    assert.equal(events.attach[0].p.library_key, 'arxiv:2401.00001');

    // Add check inline form uses canonical `check` field matching POST /solve/check.
    host.querySelector('[data-why-action="add_check"]').dispatch('click');
    const checkForm = host.querySelector('[data-testid="solve-add-check-form"]');
    checkForm.dispatch('submit');
    assert.equal(events.addCheck[0].check, 'identity_symbolic');
    assert.equal(events.addCheck[0].entry_id, 'e-4');

    // Correct this step fires onStepCorrected for F16b.
    host.querySelector('[data-why-action="correct_step"]').dispatch('click');
    const corrForm = host.querySelector('[data-testid="solve-correct-form"]');
    corrForm.querySelector('[data-testid="solve-correct-input"]').value = 'Missing factor of 1/2 in potential';
    corrForm.dispatch('submit');
    assert.equal(events.corrected.length, 1);
    assert.equal(events.corrected[0].entryId, 'e-4');
    assert.equal(events.corrected[0].correctionText, 'Missing factor of 1/2 in potential');

    // Update to lowered step e-6 (preserves "Assumed by model" tier label) and cited OCR step e-2.
    pop.update({ step: stepE6, view, sessionId: 's-1', onSelectStep: () => {} });
    assert.ok(host.querySelector('[data-testid="solve-why-lowering"]')?.textContent.includes('Assumed by model'));
    pop.update({ step: stepE2, view, sessionId: 's-1' });
    assert.ok(host.querySelector('.ui-quote-chip')?.textContent.includes('OCR'));
    assert.ok(host.querySelector('.ui-quote-chip')?.textContent.includes('p. 4'));
    pop.destroy();
  } finally {
    dom.restore();
  }
});

test('S13b solve_exprs table, new_menu Solve helper, R22 zero-verified sweep, and XSS safety audit', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    const exprHost = dom.makeEl('div');
    const jumps = [];
    const exprApi = createSolveExprsView(exprHost, view.handles, { onSelectStep: (id) => jumps.push(id) });
    assert.equal(exprHost.querySelectorAll('.solve-exprs-row').length, 9);
    const nonPortBox = exprHost.querySelector('[data-testid="solve-exprs-nonportable-only"]');
    nonPortBox.checked = true;
    nonPortBox.dispatch('change');
    assert.equal(exprHost.querySelectorAll('.solve-exprs-row').length, 1);
    assert.equal(exprHost.querySelector('.solve-exprs-row').getAttribute('data-handle'), 'raw_op@1');
    exprHost.querySelector('[data-jump-step="e-4"]').dispatch('click');
    assert.deepEqual(jumps, ['e-4']);
    exprApi.destroy();

    // new_menu Solve enablement helper (keeps default MODE_MENU_ENTRIES safe until S12 flips server flag).
    const builtEntries = buildModeMenuEntries({ solveBuilt: true });
    assert.equal(builtEntries.find((e) => e.id === 'solve').built, true);
    const rail = dom.makeEl('div');
    const btn = dom.makeEl('button');
    btn.setAttribute('id', 'new-solve-btn');
    btn.disabled = true;
    rail.append(btn);
    const chosen = [];
    enableSolveButton(rail, (mode) => chosen.push(mode));
    assert.equal(btn.disabled, false);
    btn.dispatch('click');
    assert.deepEqual(chosen, ['solve']);

    // R22 zero-verified sweep: a workspace with ONLY non-verified steps must never contain "verified", "proved", or "novel".
    const nonVerSteps = view.steps.filter((s) => s.badge.own !== 'verified' && s.badge.effective !== 'verified');
    const nonVerIds = new Set(nonVerSteps.map((s) => s.entry_id));
    const nonVerView = {
      ...view,
      steps: nonVerSteps,
      graph: {
        ...view.graph,
        nodes: view.graph.nodes.filter((n) => nonVerIds.has(n.id)),
        edges: view.graph.edges.filter((e) => nonVerIds.has(e.src) && nonVerIds.has(e.dst)),
      },
      done: {
        ...view.done,
        final_entry_id: 'e-5',
        own_tier: 'checked_numerically',
        effective_tier: 'checked_numerically',
        summary_markdown: 'Numeric check completed across 12 sample points.',
      },
    };
    const r22Host = dom.makeEl('div');
    const wsR22 = createSolveWorkspace(r22Host, nonVerView, { katex: null });
    const allText = r22Host.textContent;
    assert.ok(!/\bverified\b/i.test(allText), `R22 violation: "verified" appeared in non-verified view: ${allText}`);
    assert.ok(!/\bproved\b/i.test(allText), 'R22 violation: "proved" appeared on computed view');
    assert.ok(!/\bnovel\b/i.test(allText), 'R22 violation: "novel" appeared on Solve view');
    wsR22.destroy();

    // XSS sweep across all S13 modules.
    const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
    const xssView = {
      ...nonVerView,
      steps: [{
        ...nonVerView.steps[0],
        summary: xss,
        details: xss,
        badge: { ...nonVerView.steps[0].badge, flags: [xss], rests_on_line: xss },
      }],
      handles: [{ ...view.handles[0], latex: xss, str_preview: xss, free_symbols: [{ name: xss, assumptions: [xss] }] }],
      done: { ...nonVerView.done, summary_markdown: xss },
    };
    const xssHost = dom.makeEl('div');
    const wsXss = createSolveWorkspace(xssHost, xssView, { katex: null });
    wsXss.openWhy(xssView.steps[0].entry_id);
    assert.equal(dom.innerHtmlWrites, 0);
    walkTree(xssHost, (n) => {
      assert.notEqual(n.tagName, 'SCRIPT');
      assert.notEqual(n.tagName, 'IMG');
      for (const [k, v] of n.getAttributes().entries()) {
        assert.ok(!k.toLowerCase().startsWith('on'));
        assert.ok(!v.toLowerCase().startsWith('javascript:'));
      }
    });
    wsXss.destroy();

    // Static check: zero innerHTML/outerHTML/insertAdjacentHTML in executable code and registered in assets.rs.
    const stripComments = (s) => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
    const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
    for (const file of ['solve_badges.js', 'solve_steps.js', 'solve_why.js', 'solve_exprs.js']) {
      const codeOnly = stripComments(fs.readFileSync(path.join(assetsDir, file), 'utf8'));
      assert.ok(!/\b(innerHTML|outerHTML|insertAdjacentHTML)\b/.test(codeOnly), `${file} must not reference raw HTML APIs`);
      assert.ok(assetsRs.includes(`"/assets/${file}"`), `assets.rs must register /assets/${file}`);
    }
  } finally {
    dom.restore();
  }
});

test('S13b audit regressions: stateful update preserves R19/graph, no QuoteChip leak, stale R22, broken_by jump, strict params', () => {
  const dom = makeFakeDom();
  try {
    const view = loadFixture();
    // 1. ws.update() preserves open Why R19 warning banner AND reuses graphView instance.
    const wsHost = dom.makeEl('div');
    const ws = createSolveWorkspace(wsHost, view, {
      katex: null,
      onRerunChecks: () => ({
        changed: true,
        warning: 'result changed on re-run',
        original_evidence_sha256: 'aaaa1111bbbb2222',
        new_evidence_sha256: 'cccc3333dddd4444',
      }),
    });
    const firstGraph = ws.getGraphView();
    ws.openWhy('e-4');
    wsHost.querySelector('[data-action="recheck-one"]').dispatch('click');
    assert.ok(wsHost.querySelector('[data-testid="solve-recheck-warning"]')?.textContent.includes('result changed on re-run'));
    assert.ok(wsHost.querySelector('[data-testid="solve-recheck-warning"]')?.textContent.includes('aaaa1111'));
    ws.update({ ...view, run_state: { ...view.run_state, model_calls_used: 9 } });
    assert.equal(ws.getGraphView(), firstGraph, 'ws.update must update graph in-place without recreating');
    assert.ok(wsHost.querySelector('[data-testid="solve-recheck-warning"]')?.textContent.includes('result changed on re-run'),
      'ws.update must preserve R19 recheck warning on open Why popover');
    ws.destroy();

    // 2. Real QuoteRef without `.quote` must NEVER leak step.details or step.summary into QuoteChip.
    const leakView = {
      ...view,
      done: null,
      steps: [{
        ...view.steps[1],
        summary: 'MODEL_SUMMARY_MUST_NOT_LEAK',
        details: 'MODEL_DETAILS_MUST_NOT_LEAK',
        source_ref: { source_id: 'src-paper-1', path: 'paper.tex', start_line: 10, end_line: 12 },
      }],
    };
    const leakHost = dom.makeEl('div');
    const wsLeak = createSolveWorkspace(leakHost, leakView, { katex: null });
    const chipText = leakHost.querySelector('.ui-quote-chip')?.textContent || '';
    assert.ok(!chipText.includes('MODEL_DETAILS_MUST_NOT_LEAK'));
    assert.ok(!chipText.includes('MODEL_SUMMARY_MUST_NOT_LEAK'));
    wsLeak.destroy();

    // 3. broken_by jump button renders even when rests_on_line is null; stale own=verified never prints "Verified".
    const jumps = [];
    const brokenGroup = createSolveBadgeGroup({
      own: 'verified', effective: 'broken', stale: true,
      label: 'Broken: rests on failed e-9', rests_on_line: null, broken_by: ['e-9'], flags: [],
    }, { onSelectStep: (id) => jumps.push(id) });
    assert.ok(!/\bverified\b/i.test(brokenGroup.textContent), `stale own=verified leaked Verified: ${brokenGroup.textContent}`);
    const brokenJump = brokenGroup.querySelector('[data-jump-step="e-9"]');
    assert.ok(brokenJump, 'broken_by jump button must render even with null rests_on_line');
    brokenJump.dispatch('click');
    assert.deepEqual(jumps, ['e-9']);

    // 4. Stale own=verified step in Result sheet counts as Stale (never Verified).
    const staleHost = dom.makeEl('div');
    const staleBadge = { own: 'verified', effective: 'stale', stale: true, label: 'Stale', flags: [] };
    const wsStale = createSolveWorkspace(staleHost, {
      ...view,
      steps: [{ ...view.steps[3], badge: staleBadge }],
      graph: { nodes: [{ id: 'e-4', label: 'e-4: stale step', badge: staleBadge }], edges: [], legend: [] },
      done: { status: 'finished_open_items', final_entry_id: 'e-4', criteria: [], rests_on: { weakest: [{ entry_id: 'e-4', tier: 'Verified' }] } },
    }, { katex: null });
    assert.ok(!/\bverified\b/i.test(staleHost.textContent), `stale step leaked Verified: ${staleHost.textContent}`);
    wsStale.destroy();

    // 5. solve_exprs treats missing portable as non-portable; solve_why rejects non-object JSON & routes inspire:/file:.
    const exprHost = dom.makeEl('div');
    createSolveExprsView(exprHost, [{ name: 'x', version: 1, latex: 'x', str_preview: 'x', owner_entry: 'e-1' }]);
    assert.equal(exprHost.querySelector('.solve-exprs-row').getAttribute('data-portable'), 'false');
    assert.match(exprHost.textContent, /non-portable/i);
    const whyHost = dom.makeEl('div');
    const attached = [];
    const addChecks = [];
    const pop = createSolveWhyPopover(whyHost, {
      step: view.steps[0], view: { ...view, checks: [] }, sessionId: 's-1',
      onAttachSource: (_s, p) => attached.push(p),
      onAddCheck: (_s, p) => addChecks.push(p),
    });
    assert.equal(whyHost.querySelector('[data-why-action="rerun_checks"]').disabled, true);
    whyHost.querySelector('[data-why-action="add_check"]').dispatch('click');
    const cf = whyHost.querySelector('[data-testid="solve-add-check-form"]');
    cf.querySelector('[name="params"]').value = '[1, 2]';
    cf.dispatch('submit');
    assert.equal(addChecks.length, 0);
    assert.ok(whyHost.textContent.includes('JSON object'));
    whyHost.querySelector('[data-why-action="attach_source"]').dispatch('click');
    const af = whyHost.querySelector('[data-testid="solve-attach-source-form"]');
    af.querySelector('[name="source"]').value = 'inspire:12345';
    af.querySelector('[name="start_line"]').value = '1';
    af.querySelector('[name="quote"]').value = 'q';
    af.dispatch('submit');
    assert.equal(attached[0].library_key, 'inspire:12345');
    pop.destroy();
  } finally {
    dom.restore();
  }
});

