// Failing proof tests for UI audit findings in Investigation UI modules (F8-F13, F16).
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');
const katex = createRequire(import.meta.url)(path.join(crateRoot, 'vendor', 'katex', 'katex.min.js'));

const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
const citesUrl = pathToFileURL(path.join(assetsDir, 'paper_cites.js')).href;
const stateUrl = pathToFileURL(path.join(assetsDir, 'investigation_state.js')).href;
const scopeUrl = pathToFileURL(path.join(assetsDir, 'investigation_scope.js')).href;

const mathMod = await import(mathUrl);
const { resetMathMacros } = mathMod;
const { linkLineCitations } = await import(citesUrl);
const { paperViewState } = await import(stateUrl);

function dataUrl(code) {
  return 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
}

function makeEl(tag = 'div') {
  const listeners = new Map();
  const attrs = new Map();
  let text = '';
  let html = '';
  return {
    nodeType: 1,
    tagName: String(tag).toUpperCase(),
    id: '',
    className: '',
    dataset: {},
    children: [],
    get childNodes() { return this.children; },
    hidden: false,
    disabled: false,
    open: false,
    value: '',
    type: '',
    title: '',
    scrollTop: 0,
    isConnected: true,
    classList: { toggle() {}, add() {}, remove() {}, contains() { return false; } },
    get textContent() {
      if (this.children.length) return this.children.map((c) => c.textContent ?? '').join('');
      return text;
    },
    set textContent(v) {
      this.children = [];
      text = String(v ?? '');
      html = '';
    },
    get innerHTML() {
      if (html) return html;
      if (this.children.length) return this.children.map((c) => c.innerHTML || c.textContent || '').join('');
      return text;
    },
    set innerHTML(v) {
      this.children = [];
      html = String(v ?? '');
      text = html.replace(/<[^>]+>/g, '');
    },
    setAttribute(k, v) { attrs.set(k, String(v)); },
    getAttribute(k) { return attrs.get(k); },
    removeAttribute(k) { attrs.delete(k); },
    addEventListener(type, fn) { listeners.set(type, fn); },
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
    reset() {},
    focus() {},
    getBoundingClientRect() { return { top: 0 }; },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    querySelectorAll(sel) {
      const out = [];
      const match = (n) => {
        if (!n || !n.tagName) return false;
        if (sel.startsWith('.')) return String(n.className || '').split(/\s+/).includes(sel.slice(1));
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
}

test('F8 [major] investigation_answer.js:64-73,102-127: macro defined in earlier answer must resolve in newest answer after session switch', async (t) => {
  const prev = { document: globalThis.document, window: globalThis.window };
  globalThis.window = { katex };
  globalThis.document = {
    createElement: makeEl,
    createTextNode: (v) => ({ nodeType: 3, nodeValue: String(v ?? ''), textContent: String(v ?? ''), children: [] }),
  };
  t.after(() => Object.assign(globalThis, prev));

  const ansSrc = (await fs.readFile(path.join(assetsDir, 'investigation_answer.js'), 'utf8'))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/paper_cites.js'", JSON.stringify(citesUrl));
  const { createAnswerCard } = await import(dataUrl(ansSrc));

  // Simulate switching away and back to the Investigation session (`selectSession` resets macros).
  resetMathMacros();
  const host = makeEl('section');
  const card = createAnswerCard(host);
  card.update([
    { seq: 1, question: 'Q1', status: 'answered', text: 'Define $\\def\\Hop{\\hat{H}_0}\\Hop$ on the lattice.' },
    { seq: 2, question: 'Q2', status: 'answered', text: 'Using $\\Hop$, we find $\\Hop \\ge 0$.' },
  ], { hasMap: false, running: false });

  const body = host.querySelector('.investigation-answer-body');
  assert.ok(
    !body.innerHTML.includes('var(--status-err)'),
    `Newest answer rendered \\Hop as undefined macro error because Q1 is collapsed/unrendered: ${body.innerHTML}`,
  );
});

test('F9 [major] investigation.js:82-86,165-167 & investigation_state.js:59-60: resolving a load-time pending approval must clear approval status', async (t) => {
  const ids = [
    'app-shell', 'investigation-map', 'investigation-conventions', 'new-investigation-btn',
    'show-paper-map', 'show-investigation-chat', 'investigation-file', 'investigation-existing',
    'investigation-original', 'investigation-setup', 'investigation-pages', 'investigation-panel',
    'investigation-nav', 'investigation-status', 'investigation-origin', 'investigation-question',
    'investigation-page-start', 'investigation-page-end', 'investigation-existing-wrap',
  ];
  const byId = new Map(ids.map((id) => {
    const el = makeEl('div');
    el.id = id;
    return [id, el];
  }));
  byId.get('investigation-conventions').append(makeEl('summary'));
  byId.get('investigation-file').files = [];
  byId.get('investigation-existing').options = [];
  byId.get('investigation-existing').add = (opt) => byId.get('investigation-existing').options.push(opt);
  byId.get('investigation-status').after = (...nodes) => {
    for (const n of nodes) { if (n?.id) byId.set(n.id, n); }
  };
  byId.get('investigation-setup').append(makeEl('button'));

  const docListeners = new Map();
  const prev = { document: globalThis.document, window: globalThis.window, Option: globalThis.Option };
  globalThis.window = { katex };
  globalThis.Option = function Option(label, value) { return { label, value }; };
  globalThis.document = {
    getElementById: (id) => byId.get(id),
    createElement: makeEl,
    createTextNode: (v) => ({ nodeType: 3, textContent: String(v ?? ''), children: [] }),
    addEventListener(name, fn) { docListeners.set(name, fn); },
    activeElement: null,
  };
  t.after(() => Object.assign(globalThis, prev));

  let invSrc = await fs.readFile(path.join(assetsDir, 'investigation.js'), 'utf8');
  const stubs = {
    'api.js': 'export const getSession = async () => ({}); export const findInvestigationConventions = async () => {};',
    'composer.js': 'export const arrayBufferToBase64 = () => ""; export const setComposerPlaceholder = () => {};',
    'investigation_map.js': 'export const DEFAULT_PAPER_REQUEST = "Identify"; export const createPaperMap = () => ({ reset() {}, update() {}, updateSummary() {}, renderAnchors() {}, openNode: () => true });',
    'pdf_preview.js': 'export const openOriginalPdf = () => {};',
    'investigation_conventions.js': 'export const createConventionCatalog = () => ({ reset() {}, update() {} });',
    'investigation_answer.js': 'export const createAnswerCard = () => ({ reset() {}, update() {} });',
    'source_links.js': 'export const citedPaperLinks = () => [];',
  };
  invSrc = invSrc
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/investigation_state.js'", JSON.stringify(stateUrl))
    .replace("'/assets/investigation_scope.js'", JSON.stringify(scopeUrl));
  for (const [name, stub] of Object.entries(stubs)) {
    invSrc = invSrc.replace(`'/assets/${name}'`, JSON.stringify(dataUrl(stub)));
  }

  const mod = await import(dataUrl(invSrc));
  mod.initInvestigation({
    create: async () => {},
    open: async () => {},
    useQuestion: () => {},
    requestOptions: () => ({}),
    send: async () => {},
    openSource: () => {},
  });

  mod.showInvestigationSession('s-approve', {
    mode: 'investigation',
    running: true,
    pending_approvals: [{ call_id: 'call-1' }],
    sources: [{ source_id: 'src-1', primary_path: 'paper.txt' }],
    events: [{ seq: 2, ts_ms: Date.now() - 5000, kind: 'investigation.mapping_requested', payload: {} }],
    investigation: { source_id: 'src-1', mapping_requested: true, answers: [] },
  });
  t.after(() => mod.showInvestigationSession(null, null));

  assert.match(byId.get('investigation-status').textContent, /waiting for your approval/);

  // User resolves the approval in Conversation view; app.js dispatches waiting: false.
  docListeners.get('research:approvals')?.({ detail: { sessionId: 's-approve', waiting: false } });
  assert.doesNotMatch(
    byId.get('investigation-status').textContent,
    /waiting for your approval/,
    'Stale session.pending_approvals kept approval banner stuck after research:approvals { waiting: false }',
  );
});

test('F10 [major] investigation_map.js:42-89: renderAnchorExcerptInto must widen quote span across \\begin{equation}...\\end{equation}', async (t) => {
  const prev = { document: globalThis.document, window: globalThis.window };
  globalThis.window = { katex };
  globalThis.document = { createElement: makeEl, getElementById: () => makeEl('div') };
  t.after(() => Object.assign(globalThis, prev));

  const graphUrl = pathToFileURL(path.join(assetsDir, 'investigation_graph.js')).href;
  const mapSrc = (await fs.readFile(path.join(assetsDir, 'investigation_map.js'), 'utf8'))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/investigation_graph.js'", JSON.stringify(graphUrl));
  const { renderAnchorExcerptInto } = await import(dataUrl(mapSrc));

  const excerpt = 'We study the chain Hamiltonian\n\\begin{equation}\nH = \\sum_{i=1}^{N} J_i \\sigma_i^z\n\\end{equation}\nwith open boundaries.';
  const quote = 'H = \\sum_{i=1}^{N} J_i \\sigma_i^z';
  const el = makeEl('blockquote');
  renderAnchorExcerptInto(el, excerpt, quote);

  assert.ok(el.innerHTML.includes('katex'), `\\begin{equation} excerpt failed to render KaTeX when quote matched body: ${el.innerHTML}`);
  assert.ok(!el.textContent.includes('\\begin{equation}'), `Raw \\begin{equation} leaked into excerpt textContent: ${el.textContent}`);
});

test('F11 [major] investigation_conventions.js:42-45,142-144: section-catalog paper expansion must surface running state and conventions_error', async (t) => {
  const prev = { document: globalThis.document };
  const host = makeEl('details');
  const summary = makeEl('summary');
  host.append(summary);
  globalThis.document = {
    createElement: makeEl,
    createTextNode: (s) => ({ textContent: String(s ?? ''), children: [] }),
    getElementById: () => makeEl('div'),
    activeElement: null,
  };
  t.after(() => Object.assign(globalThis, prev));

  const convSrc = (await fs.readFile(path.join(assetsDir, 'investigation_conventions.js'), 'utf8'))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const { createConventionCatalog } = await import(dataUrl(convSrc));
  const catalog = createConventionCatalog(host, () => {}, async () => {});

  catalog.update({
    map_seq: 19,
    source_id: 'src-1',
    conventions_requested: true,
    conventions_is_current: true,
    conventions_expanding: false,
    conventions_error: 'The conventions review is still incomplete. Its saved entries are available.',
    conventions: {
      done: true,
      coverage: 'paper',
      coverage_scope: 'section',
      limitations: [],
      items: [{ id: 'b', category: 'notation', label: 'b', meaning: 'Burgers vector', scope: 'Sec. V.D.4', basis: 'explicit', anchors: [] }],
    },
  }, false);

  assert.match(
    host.textContent,
    /Conventions review stopped|still incomplete/,
    `conventions_error was silently dropped on section-done catalog expansion failure: "${host.textContent}"`,
  );
});

test('F12 [minor] paper_cites.js: wide read ranges stay clickable buttons (clamping handled by openCite)', (t) => {
  const prev = { document: globalThis.document };
  const makeText = (v) => ({
    nodeType: 3,
    nodeValue: String(v ?? ''),
    get textContent() { return this.nodeValue; },
    parentNode: null,
    replaceWith(...nodes) {
      const idx = this.parentNode.children.indexOf(this);
      for (const n of nodes) n.parentNode = this.parentNode;
      this.parentNode.children.splice(idx, 1, ...nodes);
    },
  });
  globalThis.document = {
    createElement: (tag) => {
      const el = makeEl(tag);
      const handlers = new Map();
      el.addEventListener = (type, fn) => handlers.set(type, fn);
      el.click = () => handlers.get('click')?.({ currentTarget: el });
      el.append = (...nodes) => {
        for (const n of nodes) {
          const c = typeof n === 'string' ? makeText(n) : n;
          c.parentNode = el;
          el.children.push(c);
        }
      };
      return el;
    },
    createTextNode: makeText,
  };
  t.after(() => Object.assign(globalThis, prev));

  const root = globalThis.document.createElement('div');
  const p = globalThis.document.createElement('p');
  p.append('Whole section [L10–550] remains a clickable citation button.');
  root.append(p);

  const opened = [];
  linkLineCitations(root, [{ start: 10, end: 550, read: true }], (s, e) => opened.push([s, e]));
  const buttons = root.querySelectorAll('.line-cite');
  assert.equal(
    buttons.length,
    1,
    '[L10–550] (read: true) must remain a clickable citation button; investigation.js::openCite clamps to 513 lines',
  );
  buttons[0].click();
  assert.deepEqual(opened, [[10, 550]]);
});

test('F13 [minor] investigation.js:20,38: #investigation-scope-chip must appear above #investigation-answer, not below it', async (t) => {
  const siblings = [];
  const statusEl = makeEl('div');
  statusEl.id = 'investigation-status';
  siblings.push(statusEl);
  statusEl.after = (...nodes) => {
    const idx = siblings.indexOf(statusEl);
    siblings.splice(idx + 1, 0, ...nodes);
  };

  const byId = new Map([
    ['investigation-status', statusEl],
    ['investigation-conventions', (() => { const d = makeEl('details'); d.append(makeEl('summary')); return d; })()],
    ['investigation-file', Object.assign(makeEl('input'), { files: [] })],
    ['investigation-existing', Object.assign(makeEl('select'), { options: [], add() {} })],
    ['investigation-setup', (() => { const f = makeEl('form'); f.append(makeEl('button')); return f; })()],
  ]);

  const prev = { document: globalThis.document, window: globalThis.window };
  globalThis.window = { katex };
  globalThis.document = {
    getElementById: (id) => {
      if (byId.has(id)) return byId.get(id);
      const found = siblings.find((n) => n.id === id);
      if (found) return found;
      if (id === 'investigation-answer') return null;
      const el = makeEl('div'); el.id = id; byId.set(id, el); return el;
    },
    createElement: makeEl,
    createTextNode: (v) => ({ textContent: String(v ?? ''), children: [] }),
    addEventListener() {},
  };
  t.after(() => Object.assign(globalThis, prev));

  let invSrc = await fs.readFile(path.join(assetsDir, 'investigation.js'), 'utf8');
  const stubs = {
    'api.js': 'export const getSession = async () => ({}); export const findInvestigationConventions = async () => {};',
    'composer.js': 'export const arrayBufferToBase64 = () => ""; export const setComposerPlaceholder = () => {};',
    'investigation_map.js': 'export const DEFAULT_PAPER_REQUEST = "Identify"; export const createPaperMap = () => ({ reset() {}, update() {}, updateSummary() {}, renderAnchors() {}, openNode: () => true });',
    'pdf_preview.js': 'export const openOriginalPdf = () => {};',
    'investigation_conventions.js': 'export const createConventionCatalog = () => ({ reset() {}, update() {} });',
    'investigation_answer.js': 'export const createAnswerCard = () => ({ reset() {}, update() {} });',
    'source_links.js': 'export const citedPaperLinks = () => [];',
  };
  invSrc = invSrc
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/investigation_state.js'", JSON.stringify(stateUrl))
    .replace("'/assets/investigation_scope.js'", JSON.stringify(scopeUrl));
  for (const [name, stub] of Object.entries(stubs)) {
    invSrc = invSrc.replace(`'/assets/${name}'`, JSON.stringify(dataUrl(stub)));
  }

  const mod = await import(dataUrl(invSrc));
  mod.initInvestigation({ create: async () => {}, open: async () => {}, useQuestion: () => {}, requestOptions: () => ({}), send: async () => {}, openSource: () => {} });

  const order = siblings.map((n) => n.id);
  assert.deepEqual(
    order,
    ['investigation-status', 'investigation-scope-chip', 'investigation-answer'],
    `Expected scope chip above answer card, got DOM order: ${order.join(' -> ')}`,
  );
});

test('F16 [minor] investigation_state.js:24-32: stopping a later conventions/branch/check turn must not mask an earlier mapping_error', () => {
  const inv = {
    source_id: 'src-1',
    mapping_requested: true,
    mapping_status: 'failed',
    mapping_error: 'Provider rate limit 429 during mapping.',
    map: { title: 'Saved earlier map', nodes: [{ id: 'n1' }], edges: [] },
    map_seq: 3,
    answers: [],
  };
  const events = [
    { seq: 2, ts_ms: 1000, kind: 'investigation.mapping_requested', payload: {} },
    { seq: 3, ts_ms: 2000, kind: 'investigation.map', payload: {} },
    { seq: 10, ts_ms: 3000, kind: 'investigation.mapping_requested', payload: {} },
    { seq: 11, ts_ms: 4000, kind: 'session.abort', payload: { stopped: false, reason: '429' } },
    { seq: 12, ts_ms: 4010, kind: 'investigation.mapping_finished', payload: { request_seq: 10, status: 'failed', error: 'Provider rate limit 429 during mapping.' } },
    { seq: 15, ts_ms: 5000, kind: 'investigation.conventions_resumed', payload: { for_request_seq: 2 } },
    { seq: 16, ts_ms: 6000, kind: 'session.abort', payload: { stopped: true, reason: 'Stopped by user' } },
  ];
  const state = paperViewState(inv, { running: false, events }, 10_000);
  assert.match(
    state.statusText,
    /Provider rate limit 429/,
    `Later conventions stop overwrote mapping_error with: "${state.statusText}"`,
  );
});
