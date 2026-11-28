// Regression tests for F4 `mode_stages.js` `inFlightMount` resync and stale-seq guarding.
import assert from 'node:assert/strict';
import test from 'node:test';

import { createModeStagesController } from '../assets/mode_stages.js';

function makeResyncDom() {
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
      options: [],
      style: {},
      dataset: {},
      hidden: false,
      disabled: false,
      checked: false,
      selected: false,
      value: '',
      type: '',
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
      set id(v) { const s = String(v || ''); if (s) { attrs.set('id', s); byId.set(s, node); } },
      get textContent() {
        return node.children.length ? node.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { node.children = []; text = String(v ?? ''); },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
        if (k === 'id' && v) byId.set(String(v), node);
      },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      removeAttribute(k) { attrs.delete(String(k)); },
      contains(target) {
        if (!target) return false;
        if (target === node) return true;
        return node.children.some((c) => typeof c?.contains === 'function' && c.contains(target));
      },
      focus() { if (doc) doc.activeElement = node; },
      setSelectionRange(s, e, d = 'none') { node.selectionStart = s; node.selectionEnd = e; node.selectionDirection = d; },
      append(...nodes) { for (const n of nodes) if (n) node.appendChild(n); },
      appendChild(child) {
        if (!child) return child;
        child.parentNode = node;
        node.children.push(child);
        return child;
      },
      replaceChildren(...nodes) {
        if (doc?.activeElement && doc.activeElement !== node && node.contains(doc.activeElement)) {
          doc.activeElement = null;
        }
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
  const prevRail = globalThis.__reSearchSessionRail;
  doc = {
    head,
    activeElement: null,
    createElement: (tag) => { const n = makeEl(tag, null); n.ownerDocument = doc; return n; },
    createElementNS: (ns, tag) => { const n = makeEl(tag, ns); n.ownerDocument = doc; return n; },
    createTextNode: (txt) => { const n = makeEl('#text', null); n.textContent = String(txt ?? ''); n.ownerDocument = doc; return n; },
    getElementById: (id) => byId.get(id) || null,
    querySelector: (sel) => head.querySelector(sel) || shell.querySelector(sel) || null,
    addEventListener: (t, fn) => { if (!docListeners.has(t)) docListeners.set(t, []); docListeners.get(t).push(fn); },
    removeEventListener: (t, fn) => { if (docListeners.has(t)) docListeners.set(t, docListeners.get(t).filter((f) => f !== fn)); },
  };
  for (const n of [head, shell, homeExplore, exploreStage, solveStage, activeTitle]) n.ownerDocument = doc;
  globalThis.document = doc;

  return {
    doc,
    solveStage,
    restore() {
      globalThis.document = prevDoc;
      globalThis.__reSearchSessionRail = prevRail;
    },
  };
}

test('duplicate syncFromRail during active inFlightMount reuses mount without bumping syncSeq or stealing focus', async () => {
  const dom = makeResyncDom();
  try {
    let viewFetches = 0;
    let resolveView = null;
    const stages = createModeStagesController({
      isModelConnected: () => false,
      solveOpts: { debounceMs: 0 },
      fetchJson: async (_method, url) => {
        if (url === '/api/sessions/s-live/solve/view') {
          viewFetches += 1;
          return new Promise((resolve) => { resolveView = resolve; });
        }
        throw new Error(`Unexpected URL: ${url}`);
      },
    });

    const p1 = stages.syncFromRail('s-live', [{ id: 's-live', mode: 'solve' }]);
    const p2 = stages.syncFromRail('s-live', [{ id: 's-live', mode: 'solve' }]);
    assert.equal(viewFetches, 1, 'duplicate syncFromRail must join active inFlightMount without re-fetching');

    resolveView({ setup: null, steps: [], handles: [], checks: [], receipts: [], model_connected: false });
    const [v1, v2] = await Promise.all([p1, p2]);
    assert.ok(v1 && v1 === v2, 'both callers must receive the single mounted Solve view');
    assert.equal(stages.getActiveSessionId(), 's-live');
    assert.equal(stages.getActiveMode(), 'solve');

    // Subsequent same-session syncFromRail must preserve the mounted textarea and its focus.
    const stmt = dom.solveStage.querySelector('[data-testid="solve-statement-input"]');
    stmt.focus();
    await stages.syncFromRail('s-live', [{ id: 's-live', mode: 'solve' }]);
    assert.equal(dom.solveStage.querySelector('[data-testid="solve-statement-input"]'), stmt);
    assert.equal(dom.doc.activeElement, stmt);
    assert.equal(viewFetches, 1);
  } finally {
    dom.restore();
  }
});

test('syncFromRail does not reuse a stale inFlightMount whose seq was invalidated by switching away and back', async () => {
  const dom = makeResyncDom();
  try {
    const resolvers = [];
    const stages = createModeStagesController({
      isModelConnected: () => false,
      solveOpts: { debounceMs: 0 },
      fetchJson: async (_method, url) => {
        if (url === '/api/sessions/s-bounce/solve/view') {
          return new Promise((resolve) => { resolvers.push(resolve); });
        }
        throw new Error(`Unexpected URL: ${url}`);
      },
    });

    // 1. Start mounting s-bounce (seq 1).
    const firstMount = stages.syncFromRail('s-bounce', [{ id: 's-bounce', mode: 'solve' }]);
    assert.equal(resolvers.length, 1);

    // 2. Switch to a research session (bumps syncSeq to 2; invalidates seq 1).
    await stages.syncFromRail('s-research', [
      { id: 's-bounce', mode: 'solve' },
      { id: 's-research', mode: 'research' },
    ]);
    assert.equal(stages.getActiveSessionId(), 's-research');
    assert.equal(stages.getActiveMode(), 'research');

    // 3. Switch back to s-bounce BEFORE seq 1 resolves -> must start a fresh mount (seq 3).
    const secondMount = stages.syncFromRail('s-bounce', [
      { id: 's-bounce', mode: 'solve' },
      { id: 's-research', mode: 'research' },
    ]);
    assert.equal(resolvers.length, 2, 'stale inFlightMount must not be reused once syncSeq advanced');

    // 4. Stale seq 1 resolves first -> ignored cleanly; fresh seq 3 resolves -> mounts s-bounce.
    resolvers[0]({ setup: null, steps: [], model_connected: false });
    assert.equal(await firstMount, null);
    resolvers[1]({ setup: null, steps: [], model_connected: false });
    const mounted = await secondMount;
    assert.ok(mounted, 'fresh mount must succeed after stale inFlightMount was superseded');
    assert.equal(stages.getActiveSessionId(), 's-bounce');
    assert.equal(stages.getActiveMode(), 'solve');
    assert.ok(dom.solveStage.querySelector('[data-testid="solve-statement-input"]'));
  } finally {
    dom.restore();
  }
});

test('openNewSolveSession joins active matching inFlightMount triggered by rail selection without double-mounting', async () => {
  const dom = makeResyncDom();
  try {
    let viewFetches = 0;
    let resolveView = null;
    const stages = createModeStagesController({
      isModelConnected: () => false,
      solveOpts: { debounceMs: 0 },
      fetchJson: async (method, url) => {
        if (method === 'POST' && url === '/api/sessions') {
          return { id: 's-new-1', mode: 'solve', title: 'Solve' };
        }
        if (method === 'GET' && url === '/api/sessions/s-new-1/solve/view') {
          viewFetches += 1;
          return new Promise((resolve) => { resolveView = resolve; });
        }
        throw new Error(`Unexpected: ${method} ${url}`);
      },
    });

    globalThis.__reSearchSessionRail = {
      async upsertSession(item) {
        // Trigger syncFromRail synchronously without awaiting its /solve/view response.
        stages.syncFromRail(item.id, [item]);
        return true;
      },
    };

    const opening = stages.openNewSolveSession();
    await new Promise((r) => setTimeout(r, 10));
    assert.equal(viewFetches, 1, 'openNewSolveSession must reuse active inFlightMount');
    resolveView({ setup: null, steps: [], model_connected: false });
    await opening;
    assert.equal(viewFetches, 1);
    assert.equal(stages.getActiveSessionId(), 's-new-1');
    assert.equal(stages.getActiveMode(), 'solve');
  } finally {
    dom.restore();
  }
});
