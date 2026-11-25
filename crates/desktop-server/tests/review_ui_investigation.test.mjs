// Pass 2 independent review regression tests for Investigation & Settings UI fixes.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
const stateUrl = pathToFileURL(path.join(assetsDir, 'investigation_state.js')).href;
const scopeUrl = pathToFileURL(path.join(assetsDir, 'investigation_scope.js')).href;

const { paperViewState } = await import(stateUrl);

function dataUrl(code) {
  return 'data:text/javascript;base64,' + Buffer.from(code, 'utf8').toString('base64');
}

function makeEl(tag = 'div', registry = null) {
  const listeners = new Map();
  const attrs = new Map();
  let idVal = '';
  let text = '';
  return {
    nodeType: 1,
    tagName: String(tag).toUpperCase(),
    get id() { return idVal; },
    set id(v) {
      idVal = String(v);
      if (registry && idVal) registry.set(idVal, this);
    },
    className: '',
    dataset: {},
    style: {},
    children: [],
    get childNodes() { return this.children; },
    hidden: false,
    disabled: false,
    open: false,
    checked: false,
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
    },
    setAttribute(k, v) { attrs.set(k, String(v)); },
    getAttribute(k) { return attrs.get(k); },
    removeAttribute(k) { attrs.delete(k); },
    addEventListener(type, fn) { listeners.set(type, fn); },
    async trigger(type, event = {}) {
      const fn = listeners.get(type);
      if (!fn) return undefined;
      return await fn({ currentTarget: this, preventDefault() {}, ...event });
    },
    append(...nodes) { this.children.push(...nodes); },
    appendChild(c) { this.children.push(c); return c; },
    replaceChildren(...nodes) { this.children = [...nodes]; text = ''; },
    reset() {},
    focus() {},
    getBoundingClientRect() { return { top: 0 }; },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    querySelectorAll(sel) {
      const out = [];
      const parts = sel.split(',').map((s) => s.trim()).filter(Boolean);
      const match = (n) => {
        if (!n || !n.tagName) return false;
        return parts.some((p) => (p.startsWith('.')
          ? String(n.className || '').split(/\s+/).includes(p.slice(1))
          : n.tagName === p.toUpperCase()));
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

async function loadInvestigationHarness(t, answerCardStub = 'export const createAnswerCard = () => ({ reset() {}, update() {} });') {
  const ids = [
    'app-shell', 'investigation-map', 'investigation-conventions', 'new-investigation-btn',
    'show-paper-map', 'show-investigation-chat', 'investigation-file', 'investigation-existing',
    'investigation-original', 'investigation-setup', 'investigation-pages', 'investigation-panel',
    'investigation-nav', 'investigation-status', 'investigation-origin', 'investigation-question',
    'investigation-page-start', 'investigation-page-end', 'investigation-existing-wrap',
  ];
  const byId = new Map(ids.map((id) => {
    const tag = id === 'investigation-setup' ? 'form'
      : id === 'investigation-question' ? 'textarea'
        : id === 'investigation-existing' ? 'select'
          : (id === 'investigation-file' || id === 'investigation-page-start' || id === 'investigation-page-end') ? 'input'
            : id === 'investigation-conventions' ? 'details'
              : id.endsWith('-btn') || id === 'investigation-original' ? 'button'
                : 'div';
    const el = makeEl(tag);
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
  const submitBtn = makeEl('button');
  submitBtn.type = 'submit';
  const setupForm = byId.get('investigation-setup');
  setupForm.append(
    byId.get('investigation-question'),
    byId.get('investigation-file'),
    byId.get('investigation-existing'),
    byId.get('investigation-page-start'),
    byId.get('investigation-page-end'),
    submitBtn,
  );

  const prev = { document: globalThis.document, Option: globalThis.Option };
  globalThis.Option = function Option(label, value) { return { label, value }; };
  globalThis.document = {
    getElementById: (id) => byId.get(id) || null,
    createElement: (tag) => makeEl(tag),
    createTextNode: (v) => ({ nodeType: 3, textContent: String(v ?? ''), children: [] }),
    addEventListener() {},
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
    'investigation_answer.js': answerCardStub,
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
  return { mod, byId, submitBtn };
}

test('review_ui_f8_app_js_resets_math_macros_only_on_session_switch', async () => {
  const appSrc = await fs.readFile(path.join(assetsDir, 'app.js'), 'utf8');
  const calls = [...appSrc.matchAll(/\bresetMathMacros\s*\(/g)];
  assert.equal(calls.length, 1, `Expected exactly 1 resetMathMacros() call in app.js (in selectSession), found ${calls.length}`);
  const idx = calls[0].index;
  const before = appSrc.slice(Math.max(0, idx - 400), idx);
  assert.match(before, /async function selectSession\b/, 'The single resetMathMacros() call must be inside selectSession');
});

test('review_ui_f10_f12_open_cite_clamps_wide_range', async (t) => {
  globalThis.__reviewAnswerOpts = null;
  t.after(() => { delete globalThis.__reviewAnswerOpts; });
  const { mod, byId } = await loadInvestigationHarness(
    t,
    'export const createAnswerCard = (_host, opts) => { globalThis.__reviewAnswerOpts = opts; return { reset() {}, update() {} }; };',
  );

  const opened = [];
  mod.initInvestigation({
    create: async () => {},
    open: async () => {},
    useQuestion: () => {},
    requestOptions: () => ({}),
    send: async () => {},
    openSource: (sid, anchor) => opened.push({ sid, anchor }),
  });
  mod.showInvestigationSession('sess-cite', {
    mode: 'investigation',
    running: false,
    sources: [{ source_id: 'src-1', primary_path: 'src-1/paper.tex', files: ['src-1/paper.tex'] }],
    events: [],
    investigation: { source_id: 'src-1', mapping_requested: false, answers: [{ seq: 1, status: 'answered', text: '[L10–550]' }] },
  });
  t.after(() => mod.showInvestigationSession(null, null));

  const openCite = globalThis.__reviewAnswerOpts?.openCite;
  assert.equal(typeof openCite, 'function');

  // Within the 513-line limit (10..522 = 513 lines): no clamping and no viewer-limit note.
  openCite(10, 522);
  assert.deepEqual(opened.at(-1), {
    sid: 'sess-cite',
    anchor: { source_id: 'src-1', path: 'src-1/paper.tex', start_line: 10, end_line: 522 },
  });
  assert.equal(byId.get('investigation-status').textContent, '');

  // Exceeds the 513-line viewer limit (10..550 = 541 lines): clamp end_line to 10 + 512 = 522 and show status note.
  openCite(10, 550);
  assert.deepEqual(opened.at(-1), {
    sid: 'sess-cite',
    anchor: { source_id: 'src-1', path: 'src-1/paper.tex', start_line: 10, end_line: 522 },
  });
  assert.equal(
    byId.get('investigation-status').textContent,
    'Showing lines 10–522 of 10–550 (513-line viewer limit).',
  );
});

test('review_ui_c05_subagents_paper_readers_checkbox', async (t) => {
  const puts = [];
  globalThis.__reviewSubPuts = puts;
  t.after(() => { delete globalThis.__reviewSubPuts; });

  const apiUrl = dataUrl(`
    export async function listSubagents() {
      return { builtin: [{ role_key: 'paper_reader', name: 'Paper Reader', summary: 'Reads slices.' }], custom: [] };
    }
    export async function getProviderSettings() {
      return { agent_models: {}, paper_readers_enabled: null, paper_reader_max_parallel: 3 };
    }
    export async function getModels() { return { providers: [] }; }
    export async function updateProviderSettings(body) { globalThis.__reviewSubPuts.push(body); return body; }
  `);
  const subSrc = (await fs.readFile(path.join(assetsDir, 'subagents.js'), 'utf8'))
    .replace("'/assets/api.js'", JSON.stringify(apiUrl))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const subMod = await import(dataUrl(subSrc));

  const registry = new Map();
  const builtinBox = makeEl('div', registry);
  builtinBox.id = 'builtin-agents-list';
  const prev = { document: globalThis.document, Option: globalThis.Option };
  globalThis.Option = function Option(text, val) {
    const o = makeEl('option'); o.textContent = text; o.value = val; return o;
  };
  globalThis.document = {
    getElementById: (id) => registry.get(id) || null,
    createElement: (tag) => makeEl(tag, registry),
    createTextNode: (text) => ({ nodeType: 3, textContent: String(text) }),
  };
  t.after(() => Object.assign(globalThis, prev));

  await subMod.loadSubagents();
  const cb = registry.get('paper-readers-enabled');
  assert.ok(cb, 'Expected #paper-readers-enabled checkbox');
  assert.equal(cb.tagName, 'INPUT');
  assert.equal(cb.type, 'checkbox');
  assert.equal(cb.checked, false);

  cb.checked = true;
  await cb.trigger('change');
  assert.deepEqual(puts.at(-1), { paper_readers_enabled: true });

  cb.checked = false;
  await cb.trigger('change');
  assert.deepEqual(puts.at(-1), { paper_readers_enabled: null });
});

test('review_ui_c08_reverted_and_f9_f16_preserved', () => {
  // C-08 reverted: claim check while running uses standard "Reading the paper · …" copy, not "Checking selected claim".
  const checking = paperViewState(
    { source_id: 's1', map_seq: 2, mapping_status: 'mapped', map: { nodes: [{ id: 'n1' }], edges: [] }, answers: [] },
    {
      running: true,
      events: [{ seq: 4, ts_ms: 1000, kind: 'investigation.requested', payload: {} }],
    },
    11_000,
  );
  assert.equal(checking.statusText, 'Reading the paper · 0:10');

  // C-08 reverted: stopping a conventions turn after a mapped paper does not show a mapping Stopped banner.
  const convStopped = paperViewState(
    { source_id: 's1', map_seq: 2, mapping_status: 'mapped', map: { nodes: [{ id: 'n1' }], edges: [] }, answers: [] },
    {
      running: false,
      events: [
        { seq: 1, kind: 'investigation.mapping_requested', payload: {} },
        { seq: 2, kind: 'investigation.map', payload: {} },
        { seq: 4, kind: 'investigation.conventions_resumed', payload: {} },
        { seq: 5, kind: 'session.abort', payload: { stopped: true } },
      ],
    },
    10_000,
  );
  assert.equal(convStopped.statusText, '');

  // F9 preserved: explicit approvalWaiting === false overrides stale pending_approvals.
  const clearedApproval = paperViewState(
    { source_id: 's1', mapping_requested: true, answers: [] },
    { running: true, approvalWaiting: false, pending_approvals: [{ call_id: 'c1' }], events: [] },
    5_000,
  );
  assert.equal(clearedApproval.showApprovalButton, false);
  assert.doesNotMatch(clearedApproval.statusText, /approval/);
});

test('review_ui_f11_conventions_expanding_status_and_error', async (t) => {
  const prev = { document: globalThis.document };
  const host = makeEl('details');
  const summary = makeEl('summary');
  host.append(summary);
  globalThis.document = {
    createElement: (tag) => makeEl(tag),
    createTextNode: (s) => ({ textContent: String(s ?? ''), children: [] }),
    getElementById: () => makeEl('div'),
    activeElement: null,
  };
  t.after(() => Object.assign(globalThis, prev));

  const convSrc = (await fs.readFile(path.join(assetsDir, 'investigation_conventions.js'), 'utf8'))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const { createConventionCatalog } = await import(dataUrl(convSrc));
  const catalog = createConventionCatalog(host, () => {}, async () => {});

  const baseInv = {
    map_seq: 9,
    source_id: 'src-1',
    conventions_requested: true,
    conventions_is_current: true,
    conventions: {
      done: true,
      coverage: 'paper',
      coverage_scope: 'section',
      limitations: [],
      items: [{ id: 'a', category: 'notation', label: 'a', meaning: 'lattice constant', scope: 'Sec. II', basis: 'explicit', anchors: [] }],
    },
  };

  catalog.update({ ...baseInv, conventions_expanding: true, conventions_error: null }, true);
  assert.match(summary.textContent, /section catalog · reading whole paper…$/);

  catalog.update({ ...baseInv, conventions_expanding: false, conventions_error: 'Whole-paper expansion failed.' }, false);
  assert.doesNotMatch(summary.textContent, /reading whole paper…/);
  assert.match(host.textContent, /Conventions review stopped\. Whole-paper expansion failed\./);
});

test('review_ui_new_investigation_disables_setup_while_creating', async (t) => {
  const { mod, byId, submitBtn } = await loadInvestigationHarness(t);

  let resolveCreate;
  let rejectCreate;
  let createCalls = 0;

  mod.initInvestigation({
    create: () => {
      createCalls += 1;
      return new Promise((resolve, reject) => {
        resolveCreate = resolve;
        rejectCreate = reject;
      });
    },
    open: async () => {},
    useQuestion: () => {},
    requestOptions: () => ({}),
    send: async () => {},
    openSource: () => {},
  });

  const emptyA = {
    mode: 'investigation',
    running: false,
    sources: [],
    events: [],
    investigation: { source_id: null, mapping_requested: false, answers: [] },
  };
  mod.showInvestigationSession('inv-A', emptyA);
  t.after(() => mod.showInvestigationSession(null, null));

  const setupControls = [
    byId.get('investigation-file'),
    byId.get('investigation-existing'),
    byId.get('investigation-question'),
    byId.get('investigation-page-start'),
    byId.get('investigation-page-end'),
    submitBtn,
  ];
  for (const ctrl of setupControls) assert.equal(ctrl.disabled, false);

  const newBtn = byId.get('new-investigation-btn');
  const pendingClick = newBtn.trigger('click');

  // While callbacks.create() is awaiting: button and all setup controls must be disabled.
  assert.equal(createCalls, 1);
  assert.equal(newBtn.disabled, true);
  for (const ctrl of setupControls) {
    assert.equal(ctrl.disabled, true, `${ctrl.id || ctrl.tagName} must be disabled while creating a new investigation`);
  }

  // Even an intermediate session refresh on A must not re-enable setup inputs while create is in flight.
  mod.showInvestigationSession('inv-A', emptyA);
  for (const ctrl of setupControls) {
    assert.equal(ctrl.disabled, true, `${ctrl.id || ctrl.tagName} must stay disabled across refresh while create is pending`);
  }

  // Simulate createAndSelectSession selecting fresh empty investigation B before resolving.
  mod.showInvestigationSession('inv-B', {
    mode: 'investigation',
    running: false,
    sources: [],
    events: [],
    investigation: { source_id: null, mapping_requested: false, answers: [] },
  });
  resolveCreate();
  await pendingClick;

  assert.equal(newBtn.disabled, false);
  for (const ctrl of setupControls) {
    assert.equal(ctrl.disabled, false, `${ctrl.id || ctrl.tagName} must be restored after create completes`);
  }

  // If create rejects while still on a session with a pinned source_id, finally restores pinned states accurately.
  mod.showInvestigationSession('inv-pinned', {
    mode: 'investigation',
    running: false,
    sources: [{ source_id: 'src-1', title: 'Paper' }],
    events: [],
    investigation: { source_id: 'src-1', mapping_requested: false, answers: [] },
  });
  const failingClick = newBtn.trigger('click');
  assert.equal(byId.get('investigation-question').disabled, true);
  assert.equal(submitBtn.disabled, true);
  rejectCreate(new Error('network error'));
  await assert.rejects(failingClick, /network error/);

  assert.equal(newBtn.disabled, false);
  assert.equal(byId.get('investigation-question').disabled, false);
  assert.equal(submitBtn.disabled, false);
  assert.equal(byId.get('investigation-file').disabled, true, 'Pinned source file input must stay disabled after failed create');
  assert.equal(byId.get('investigation-existing').disabled, true, 'Pinned source select must stay disabled after failed create');
});
