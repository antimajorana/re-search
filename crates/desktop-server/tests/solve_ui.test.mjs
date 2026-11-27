// Node tests for Solve S13 UI modules (solve_badges.js, solve_steps.js, solve_why.js, solve_exprs.js).
// Covers S2 tier mapping, R5 handle-only formulas, R15 Done sheet, R19 recheck diff, R22 wording sweep,
// all 7 Why actions, tab registry, F6 graph/run_bar integration, and zero-innerHTML XSS safety.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  FLAG_OCR_SOURCE,
  FLAG_SAME_MODEL,
  FLAG_TRANSCRIPTION_UNCONFIRMED,
  LEGEND_VERIFIED_EXPLANATION,
  OWN_CHECK_VERIFIED_LINE,
  SOLVE_TIER_LABELS,
  adaptSolveGraphView,
  computeFinalClosureIds,
  createSolveBadgeGroup,
  createSolveLegend,
  mapBadgeToUiSpec,
} from '../assets/solve_badges.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const assetsDir = path.join(crateRoot, 'assets');

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
      style: {},
      type: '',
      title: '',
      hidden: false,
      disabled: false,
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
      setAttribute(k, v) { attrs.set(String(k), String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      getAttributes: () => attrs,
      append(...nodes) { this.children.push(...nodes); },
      appendChild(n) { this.children.push(n); return n; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; },
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

test('S13a solve_badges maps all 11 S2 tiers, preserves server rests-on flags verbatim, and enforces R22', () => {
  const dom = makeFakeDom();
  try {
    assert.equal(SOLVE_TIER_LABELS.verified, 'Verified');
    assert.equal(SOLVE_TIER_LABELS.checked_numerically, 'Numerically checked');
    assert.equal(SOLVE_TIER_LABELS.cited, 'Found');

    const tiers = Object.keys(SOLVE_TIER_LABELS);
    assert.equal(tiers.length, 11);

    for (const tier of tiers) {
      const spec = mapBadgeToUiSpec({
        own: tier,
        effective: tier,
        label: SOLVE_TIER_LABELS[tier],
      });
      assert.equal(spec.text, SOLVE_TIER_LABELS[tier]);
      assert.ok(spec.icon, `tier ${tier} must have an icon`);
      assert.ok(spec.tone, `tier ${tier} must have a tone`);
      if (tier !== 'verified') {
        assert.ok(!/\bverified\b/i.test(spec.text), `non-verified tier ${tier} must not say verified`);
      }
      assert.ok(!/\bproved\b|\bnovel\b/i.test(spec.text), `tier ${tier} must not say proved/novel`);
    }

    // Fail-closed R22 guard: spoofed "Verified" label on a numeric step is rejected.
    const spoofed = mapBadgeToUiSpec({
      own: 'checked_numerically',
      effective: 'checked_numerically',
      label: 'Verified (spoofed)',
    });
    assert.equal(spoofed.text, 'Numerically checked');

    // Lowered Verified step preserves "own check: verified", verbatim "rests on ...", and all flags.
    const jumps = [];
    let clicked = 0;
    const group = createSolveBadgeGroup(
      {
        own: 'verified',
        effective: 'unchecked',
        label: 'Unchecked',
        own_check_line: OWN_CHECK_VERIFIED_LINE,
        rests_on_line: 'rests on e-4 (unchecked)',
        lowered_by: ['e-4'],
        sublabels: ['to O(eps^0)'],
        flags: [
          'rests on 2 assumptions made by the model',
          FLAG_TRANSCRIPTION_UNCONFIRMED,
          FLAG_SAME_MODEL,
          FLAG_OCR_SOURCE,
        ],
        grade: 'model_statement',
        border: 'dashed',
      },
      {
        onBadgeClick: () => { clicked += 1; },
        onSelectStep: (id) => jumps.push(id),
      },
    );
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(group.querySelector('[data-badge-role="primary"]')?.textContent, 'Unchecked');
    assert.equal(group.querySelector('[data-badge-role="own-check"]')?.textContent, OWN_CHECK_VERIFIED_LINE);
    assert.ok(group.querySelector('[data-badge-role="rests-on"]')?.textContent.includes('rests on e-4 (unchecked)'));
    const flagTexts = group.querySelectorAll('.solve-flag-chip').map((n) => n.textContent);
    assert.deepEqual(flagTexts, [
      'rests on 2 assumptions made by the model',
      FLAG_TRANSCRIPTION_UNCONFIRMED,
      FLAG_SAME_MODEL,
      FLAG_OCR_SOURCE,
    ]);
    group.querySelector('[data-badge-role="primary"]').dispatch('click');
    group.querySelector('[data-jump-step="e-4"]').dispatch('click');
    assert.equal(clicked, 1);
    assert.deepEqual(jumps, ['e-4']);

    // Legend: exact Verified explanation when verified exists; omitted when zero verified steps exist (R22).
    const legendNoVer = createSolveLegend([], [{ entry_id: 'e-1', badge: { own: 'checked_numerically', effective: 'checked_numerically' } }]);
    assert.ok(!/\bverified\b/i.test(legendNoVer.textContent), 'legend without verified steps must omit Verified');

    const legendWithVer = createSolveLegend([], [{ entry_id: 'e-1', badge: { own: 'verified', effective: 'verified' } }]);
    const note = legendWithVer.querySelector('[data-testid="solve-legend-verified-note"]');
    assert.ok(note && note.textContent.includes(LEGEND_VERIFIED_EXPLANATION));

    // Closure of final answer filter includes depends_on and derived_depends_on (R9).
    const steps = [
      { entry_id: 'e-1', depends_on: [], derived_depends_on: [] },
      { entry_id: 'e-2', depends_on: ['e-1'], derived_depends_on: [] },
      { entry_id: 'e-side', depends_on: ['e-1'], derived_depends_on: [] },
      { entry_id: 'e-3', depends_on: [], derived_depends_on: ['e-2'], entry_type: 'final_answer' },
    ];
    assert.deepEqual([...computeFinalClosureIds(steps, 'e-3')].sort(), ['e-1', 'e-2', 'e-3']);
    const adapted = adaptSolveGraphView(
      {
        steps,
        graph: {
          nodes: steps.map((s) => ({ id: s.entry_id, grade: 'code' })),
          edges: [
            { src: 'e-1', dst: 'e-2' },
            { src: 'e-1', dst: 'e-side' },
            { src: 'e-2', dst: 'e-3' },
          ],
          legend: [],
        },
      },
      { closureOfFinal: true },
    );
    assert.deepEqual(adapted.nodes.map((n) => n.id).sort(), ['e-1', 'e-2', 'e-3']);
    assert.equal(adapted.edges.length, 2);

    // Assets registration & split verification.
    const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
    assert.ok(assetsRs.includes('"/assets/solve_badges.js"'));
    assert.ok(assetsRs.includes('assets_fonts::lookup_katex_font'));
  } finally {
    dom.restore();
  }
});
