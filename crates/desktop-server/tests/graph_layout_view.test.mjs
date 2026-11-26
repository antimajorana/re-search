import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import {
  DEFAULT_FOLD_THRESHOLD,
  filterGraphView,
  foldGraphView,
  layoutGraphView,
} from '../assets/graph_layout.js';
import {
  EDGE_DASH_ARRAY,
  EDGE_WEIGHTS,
  createGraphView,
  edgeStrokeWidth,
  isEdgeDashed,
  isNodeDashed,
} from '../assets/graph_view.js';

const fixturePath = new URL('./fixtures/investigate_graph_view.json', import.meta.url);
const investigateFixture = JSON.parse(await readFile(fixturePath, 'utf8'));

function makeFakeDom() {
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
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
      placeholder: '',
      title: '',
      hidden: false,
      disabled: false,
      tabIndex: 0,
      clientWidth: 800,
      clientHeight: 500,
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
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      removeEventListener(type, fn) {
        if (listeners.has(type)) listeners.set(type, listeners.get(type).filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      getBoundingClientRect() { return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight }; },
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
  return { makeEl, restore() { globalThis.document = prevDoc; } };
}

function makeLargeGraph(count) {
  const nodes = [];
  const edges = [];
  for (let i = 0; i < count; i++) {
    const sec = `Section ${(i % 5) + 1}`;
    nodes.push({ id: `n-${i}`, kind: 'lemma', label: `Lemma ${i}`, group: sec, grade: 'code', badges: [], flags: [] });
    if (i >= 5) edges.push({ src: `n-${i - 5}`, dst: `n-${i}`, kind: 'uses', role: 'statement', basis: 'code' });
    if (i > 0 && i % 5 !== 0) {
      edges.push({ src: `n-${i - 1}`, dst: `n-${i}`, kind: 'uses', role: 'proof', basis: i % 2 === 0 ? 'code' : 'model' });
    }
  }
  return { nodes, edges, legend: [] };
}

test('graph_layout places Investigate fixture top-to-bottom in deterministic section lanes', () => {
  const layout = layoutGraphView(investigateFixture);
  assert.equal(layout.folded, false);
  assert.equal(layout.hasLanes, true);
  assert.deepEqual(layout.lanes.map((l) => l.label), [
    'Section 1: Setup', 'Section 2: Decay Estimate', 'Section 3: Main Theorem', 'Section 4: Auxiliary Remarks',
  ]);
  assert.equal(layout.edges.length, investigateFixture.edges.length);
  for (const edge of layout.edges) {
    const src = layout.nodeById.get(edge.src);
    const dst = layout.nodeById.get(edge.dst);
    assert.ok(src.y + src.h <= dst.y, `expected ${edge.src} above ${edge.dst}`);
    assert.ok(edge.y1 < edge.y2, `expected arrow y1 < y2 for ${edge.src} -> ${edge.dst}`);
  }
  for (const node of layout.nodes) {
    const lane = layout.lanes[node.laneIndex];
    assert.ok(node.x >= lane.x && node.x + node.w <= lane.x + lane.w);
  }
  assert.doesNotMatch(JSON.stringify(investigateFixture), /\bverified\b/i);
});

test('graph_layout folds above 150 nodes without merging code and model edges, and stays unfolded at 150', () => {
  assert.equal(DEFAULT_FOLD_THRESHOLD, 150);
  const exact = foldGraphView(makeLargeGraph(150));
  assert.equal(exact.folded, false);
  assert.equal(exact.visibleNodeCount, 150);

  const large = makeLargeGraph(160);
  const folded = foldGraphView(large);
  assert.equal(folded.folded, true);
  assert.equal(folded.originalNodeCount, 160);
  assert.equal(folded.visibleNodeCount, 5);
  assert.ok(folded.nodes.every((n) => n.folded && n.foldedCount === 32 && n.internalEdgeCount > 0));
  assert.ok(folded.edges.some((e) => e.basis === 'code'));
  assert.ok(folded.edges.some((e) => e.basis === 'model'));

  const expanded = foldGraphView(large, { expandedGroups: ['Section 1'] });
  assert.equal(expanded.visibleNodeCount, 36);
});

test('graph_layout handles ungrouped graphs and cycles without dropping nodes', () => {
  const cyclic = {
    nodes: [
      { id: 'a', kind: 'step', label: 'A', group: null, grade: 'code', badges: [], flags: [] },
      { id: 'b', kind: 'step', label: 'B', group: null, grade: 'code', badges: [], flags: [] },
      { id: 'c', kind: 'step', label: 'C', group: null, grade: 'model', badges: [], flags: [] },
    ],
    edges: [
      { src: 'a', dst: 'b', kind: 'uses', role: 'statement', basis: 'code' },
      { src: 'b', dst: 'c', kind: 'uses', role: 'proof', basis: 'model' },
      { src: 'c', dst: 'b', kind: 'uses', role: 'proof', basis: 'model' },
    ],
    legend: [],
  };
  const layout = layoutGraphView(cyclic);
  assert.equal(layout.hasLanes, false);
  assert.equal(layout.nodes.length, 3);
  assert.deepEqual(new Set(layout.cyclic), new Set(['b', 'c']));
});

test('graph_view encodes solid/dashed honesty, statement vs proof weight, filters, pan, zoom, and selection', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const selectedNodes = [];
    const selectedEdges = [];
    const view = createGraphView(host, investigateFixture, {
      onSelectNode: (n) => selectedNodes.push(n.id),
      onSelectEdge: (e) => selectedEdges.push(`${e.src}->${e.dst}`),
    });

    assert.equal(isEdgeDashed({ basis: 'code' }), false);
    assert.equal(isEdgeDashed({ basis: 'evidence' }), false);
    assert.equal(isEdgeDashed({ basis: 'evidence', abstract_only: true }), true);
    assert.equal(isEdgeDashed({ basis: 'model' }), true);
    assert.equal(isNodeDashed({ grade: 'code' }), false);
    assert.equal(isNodeDashed({ grade: 'quote_ocr' }), false);
    assert.equal(isNodeDashed({ grade: 'quote_abstract' }), true);
    assert.equal(isNodeDashed({ grade: 'model' }), true);
    assert.ok(edgeStrokeWidth({ role: 'statement' }) > edgeStrokeWidth({ role: 'proof' }));

    const edges = view.element.querySelectorAll('.gv-edge');
    assert.equal(edges.length, investigateFixture.edges.length);

    const codeStmt = edges.find((e) => e.getAttribute('data-src') === 'n-def0000001' && e.getAttribute('data-dst') === 'n-lem0000004');
    const codeProof = edges.find((e) => e.getAttribute('data-src') === 'n-lem0000004' && e.getAttribute('data-dst') === 'n-thm0000006');
    const modelEdge = edges.find((e) => e.getAttribute('data-src') === 'n-lem0000005' && e.getAttribute('data-dst') === 'm-stp0000007');
    assert.equal(codeStmt.getAttribute('data-dashed'), 'false');
    assert.equal(codeProof.getAttribute('data-dashed'), 'false');
    assert.equal(modelEdge.getAttribute('data-dashed'), 'true');
    assert.equal(codeStmt.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), null);
    assert.equal(codeProof.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), null);
    assert.equal(modelEdge.querySelectorAll('path')[1].getAttribute('stroke-dasharray'), EDGE_DASH_ARRAY);
    assert.equal(Number(codeStmt.getAttribute('data-stroke-width')), EDGE_WEIGHTS.statement);
    assert.equal(Number(codeProof.getAttribute('data-stroke-width')), EDGE_WEIGHTS.proof);

    // Toggle proof uses off and statement uses off independently.
    const proofToggle = view.element.querySelector('[data-filter="proof"]');
    proofToggle.click();
    const afterProofOff = view.element.querySelectorAll('.gv-edge');
    assert.ok(afterProofOff.every((e) => e.getAttribute('data-role') !== 'proof'));
    assert.ok(afterProofOff.some((e) => e.getAttribute('data-role') === 'statement'));

    const stmtToggle = view.element.querySelector('[data-filter="statement"]');
    stmtToggle.click();
    assert.ok(view.element.querySelectorAll('.gv-edge').every((e) => !['statement', 'proof'].includes(e.getAttribute('data-role'))));

    proofToggle.click();
    stmtToggle.click();
    const dimToggle = view.element.querySelector('[data-filter="hide-dimmed"]');
    assert.equal(dimToggle.hidden, false);
    dimToggle.click();
    assert.equal(view.element.querySelectorAll('[data-node-id="n-rem0000009"]').length, 0);

    // Selection and stale badge tone stripping.
    view.element.querySelector('[data-node-id="n-thm0000006"]').click();
    assert.equal(view.getSelectedId(), 'n-thm0000006');
    assert.deepEqual(selectedNodes, ['n-thm0000006']);
    view.element.querySelectorAll('.gv-edge')[0].click();
    assert.equal(selectedEdges.length, 1);

    const staleNode = view.element.querySelector('[data-node-id="n-cor0000008"]');
    assert.equal(staleNode.getAttribute('data-stale'), 'true');
    const staleBadges = staleNode.querySelectorAll('.ui-badge');
    assert.ok(staleBadges.some((b) => b.textContent.includes('Stale')));
    assert.ok(staleBadges.every((b) => b.getAttribute('data-tone') === 'muted'));

    // Pointer-drag pan and zoom transform updates.
    const viewport = view.element.querySelector('.gv-viewport');
    const before = view.getViewTransform();
    viewport.dispatch('pointerdown', { button: 0, pointerId: 1, clientX: 100, clientY: 100 });
    viewport.dispatch('pointermove', { pointerId: 1, clientX: 160, clientY: 135 });
    viewport.dispatch('pointerup', { pointerId: 1 });
    const afterPan = view.getViewTransform();
    assert.ok(Math.abs((afterPan.tx - before.tx) - 60) < 0.01);
    assert.ok(Math.abs((afterPan.ty - before.ty) - 35) < 0.01);

    view.zoomBy(1.25);
    assert.ok(view.getViewTransform().scale > afterPan.scale);
    assert.deepEqual(filterGraphView(investigateFixture, { search: 'quasi-local' }).nodes.map((n) => n.id), ['n-thm0000006']);
    view.destroy();
  } finally {
    dom.restore();
  }
});

test('graph_view folds graphs above 150 nodes and expands/refolds sections on click', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.makeEl('div');
    const expanded = [];
    const view = createGraphView(host, makeLargeGraph(160), {
      onExpandGroup: (key) => expanded.push(key),
    });
    assert.equal(view.getLayout().folded, true);
    assert.equal(view.element.querySelectorAll('.gv-node').length, 5);
    const banner = view.element.querySelector('.gv-fold-banner');
    assert.equal(banner.hidden, false);
    assert.match(banner.textContent, /160 nodes folded above 150/);

    // Click the first folded group box to expand Section 1.
    view.element.querySelector('[data-node-id="group:Section 1"]').click();
    assert.deepEqual(expanded, ['Section 1']);
    assert.equal(view.element.querySelectorAll('.gv-node').length, 36);

    // Click refold button to fold all groups again.
    view.element.querySelector('[data-fold-action="refold"]').click();
    assert.equal(view.element.querySelectorAll('.gv-node').length, 5);
    view.destroy();
  } finally {
    dom.restore();
  }
});
