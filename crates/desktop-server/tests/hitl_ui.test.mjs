import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

function mockDom(ids = []) {
  const makeEl = (tag = 'div') => {
    const classes = new Set();
    const listeners = {};
    const el = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      children: [],
      get childElementCount() { return this.children.length; },
      open: false,
      attributes: {},
      style: {},
      value: '',
      textContent: '',
      _html: '',
      set innerHTML(v) {
        this._html = String(v);
        if (!v) this.children = [];
      },
      get innerHTML() {
        return this._html;
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, f) => (f === undefined ? (classes.has(c) ? classes.delete(c) : classes.add(c)) : (f ? classes.add(c) : classes.delete(c))),
        contains: (c) => classes.has(c),
      },
      setAttribute: (k, v) => { el.attributes[k] = String(v); },
      removeAttribute: (k) => { delete el.attributes[k]; },
      getAttribute: (k) => el.attributes[k] ?? null,
      appendChild: (c) => { el.children.push(c); return c; },
      append: (...children) => el.children.push(...children),
      replaceChildren: (...children) => { el.children = children; },
      addEventListener: (ev, fn) => { (listeners[ev] ||= []).push(fn); },
      click: () => Promise.all((listeners.click || []).map((fn) => fn())),
      dispatch: (name, event = { preventDefault() {} }) => Promise.all((listeners[name] || []).map((fn) => fn(event))),
      showModal() { this.open = true; },
      close() { this.open = false; return this.dispatch('close'); },
      getClientRects() { return this.hidden ? [] : [{}]; },
      closest: () => null,
      contains: (node) => el === node || el.children.some((child) => child.contains(node)),
      querySelector: () => null,
      querySelectorAll: () => [],
      focus: () => {},
    };
    return el;
  };
  const elements = new Map(ids.map((id) => [id, makeEl()]));
  return {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    getElementById: (id) => elements.get(id) || null,
    addEventListener: () => {},
    querySelector: () => null,
    querySelectorAll: () => [],
  };
}

async function importAsset(name, replacements = {}) {
  let code = fs.readFileSync(path.join(assetsDir, name), 'utf8');
  for (const [k, v] of Object.entries(replacements)) {
    code = code.replaceAll(k, v);
  }
  return import(`data:text/javascript;base64,${Buffer.from(code, 'utf8').toString('base64')}`);
}

function assetStub(code) {
  return JSON.stringify(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
}

test('Proof graph supports keyboard inspection and forks at the displayed sequence', async () => {
  const api = {
    getDag: () => new Promise(() => {}),
    forkSession: async (id, payload) => {
      forkCalls.push([id, payload]);
      return { new_session: 'forked' };
    },
    diffSessions: async () => ({}),
    verifySession: () => new Promise((resolve) => { finishVerify = resolve; }),
  };
  const forkCalls = [];
  let finishVerify;
  const previousDoc = globalThis.document;
  const doc = mockDom([
    'proof-dag-svg', 'dag-node-detail', 'dag-verify-badge', 'dag-verify-btn',
    'dag-fork-btn', 'dag-fork-bar', 'fork-seq-label', 'fork-confirm-btn',
  ]);
  globalThis.document = doc;
  globalThis.canvasTestApi = api;
  try {
    const dag = await importAsset('canvas_dag.js', {
      "'/assets/api.js'": assetStub(
        'export const { getDag, forkSession, diffSessions, verifySession } = new Proxy({}, { get: (_, method) => (...args) => globalThis.canvasTestApi[method](...args) });',
      ),
      "'/assets/math.js'": assetStub('export const escapeHtml = String;'),
      "'/assets/model_picker.js'": assetStub('export const getModelChoice = () => null;'),
      "'/assets/canvas_sources.js'": assetStub('export const openSessionSource = () => {};'),
      "'/assets/canvas_dag_detail.js'": assetStub(
        "export const renderDagNodeDetail = (element, entry) => { element.textContent = entry?.entry_id || ''; };",
      ),
    });
    const el = (id) => doc.getElementById(id);
    dag.initDagCanvas();
    let finishOldDag;
    api.getDag = (id) => id === 'old'
      ? new Promise((resolve) => { finishOldDag = resolve; })
      : Promise.resolve({ entries: [{ entry_id: 'e-b', seq: 4 }, { entry_id: 'e-c', seq: 8 }] });
    const oldDag = dag.refreshDag('old');
    await dag.refreshDag('current');
    finishOldDag({ entries: [{ entry_id: 'e-stale', seq: 12 }] });
    await oldDag;
    const stage = el('proof-dag-svg').children.at(-1);
    const nodes = stage.children;
    stage.querySelectorAll = () => nodes;
    assert.equal(nodes[0].getAttribute('role'), 'button');
    assert.equal(nodes[0].getAttribute('tabindex'), '0');
    assert.match(nodes[0].getAttribute('aria-label'), /e-b/);
    let prevented = false;
    await nodes[0].dispatch('keydown', { key: 'Enter', preventDefault() { prevented = true; } });
    assert.equal(prevented, true);
    assert.equal(nodes[0].getAttribute('aria-pressed'), 'true');
    assert.equal(el('dag-node-detail').textContent, 'e-b');
    await el('dag-fork-btn').click();
    assert.equal(el('fork-seq-label').textContent, '4');
    dag.selectNodeById('e-c');
    await el('fork-confirm-btn').click();
    assert.deepEqual(forkCalls, [['current', { at_seq: 4, modified_assumption: undefined }]]);
    const verifying = el('dag-verify-btn').click();
    await dag.refreshDag(null);
    finishVerify({ head_hash: 'old-hash' });
    await verifying;
    assert.equal(el('dag-verify-badge').hidden, true);
    assert.equal(el('dag-fork-bar').hidden, true);
    assert.match(el('proof-dag-svg').innerHTML, /Select a session/);
  } finally {
    globalThis.document = previousDoc;
    delete globalThis.canvasTestApi;
  }
});

test('LaTeX refresh preserves edits and makes truncated files read-only', async () => {
  let finishOldFile;
  const saved = [];
  const previousDoc = globalThis.document;
  const doc = mockDom(['latex-file-select', 'latex-editor', 'latex-save-btn', 'tex-audit-btn', 'latex-file-note']);
  globalThis.document = doc;
  globalThis.canvasTestApi = {
    getWorkspace: async (id, path) => {
      if (!path) return { files: [`${id}.tex`] };
      if (id === 'old') return new Promise((resolve) => { finishOldFile = resolve; });
      if (id === 'truncated') return { content: 'Prefix only', truncated: true };
      return { content: 'Current proof' };
    },
    patchLatex: async (id, patch) => { saved.push([id, patch]); },
  };
  try {
    const latex = await importAsset('canvas_latex.js', {
      "'/assets/api.js'": assetStub(
        'export const getWorkspace = (...args) => globalThis.canvasTestApi.getWorkspace(...args); export const patchLatex = (...args) => globalThis.canvasTestApi.patchLatex(...args);',
      ),
      "'/assets/math.js'": assetStub('export const escapeHtml = String; export const renderLatexDocumentPreview = () => {};'),
    });
    const editor = doc.getElementById('latex-editor');
    const save = doc.getElementById('latex-save-btn');
    latex.initLatexCanvas();
    const oldFile = latex.refreshWorkspaceFiles('old');
    await Promise.resolve();
    const currentFile = latex.refreshWorkspaceFiles('current');
    assert.equal(editor.value, '');
    assert.equal(save.disabled, true);
    await save.click();
    assert.deepEqual(saved, []);
    await currentFile;
    finishOldFile({ content: 'Stale proof' });
    await oldFile;
    assert.equal(editor.value, 'Current proof');
    editor.value = 'Revised proof';
    await latex.refreshWorkspaceFiles('current');
    assert.equal(editor.value, 'Revised proof');
    await save.click();
    assert.deepEqual(saved, [['current', { path: 'current.tex', search: 'Current proof', replace: 'Revised proof', content: 'Revised proof' }]]);
    await latex.refreshWorkspaceFiles('truncated');
    assert.equal(editor.value, 'Prefix only');
    assert.equal(editor.readOnly, true);
    assert.equal(editor.getAttribute('aria-describedby'), 'latex-file-note');
    assert.equal(save.disabled, true);
    assert.equal(doc.getElementById('tex-audit-btn').disabled, true);
    assert.equal(doc.getElementById('latex-file-note').hidden, false);
    await save.click();
    assert.equal(saved.length, 1);
    await latex.refreshWorkspaceFiles(null);
    assert.equal(editor.value, '');
    assert.equal(save.disabled, true);
  } finally {
    globalThis.document = previousDoc;
    delete globalThis.canvasTestApi;
  }
});

test('Composer Enter waits until IME composition has completed', async () => {
  const previousDoc = globalThis.document;
  const doc = mockDom(['composer-input', 'composer-pill']);
  let submits = 0;
  doc.getElementById('composer-pill').requestSubmit = () => { submits += 1; };
  globalThis.document = doc;
  try {
    const composer = await importAsset('composer.js', {
      "'/assets/api.js'": assetStub('export {};'),
      "'/assets/math.js'": assetStub('export const escapeHtml = String;'),
      "'/assets/effort_picker.js'": assetStub('export const closeEffortWindow = () => {}; export const getReasoningEffort = () => null;'),
      "'/assets/subagents.js'": assetStub('export const getActiveSubagent = () => null;'),
    });
    composer.initComposer();
    const input = doc.getElementById('composer-input');
    let prevented = 0;
    const pressEnter = (props) => input.dispatch('keydown', { key: 'Enter', preventDefault() { prevented += 1; }, ...props });
    await pressEnter({ isComposing: true });
    await pressEnter({ keyCode: 229 });
    await pressEnter({ shiftKey: true });
    assert.equal(prevented, 0);
    assert.equal(submits, 0);
    await pressEnter({});
    assert.equal(prevented, 1);
    assert.equal(submits, 1);
  } finally {
    globalThis.document = previousDoc;
  }
});

test('Keyboard wraps across tabs and collapsed drawers become inert', async () => {
  const previousDoc = globalThis.document;
  const previousWindow = globalThis.window;
  const doc = mockDom([
    'app-shell', 'left-rail', 'rail-toggle', 'canvas-drawer', 'canvas-toggle', 'canvas-close-btn', 'one', 'two',
  ]);
  let handleEscape;
  doc.addEventListener = (event, callback, options) => {
    if (event === 'keydown' && options?.capture) handleEscape = callback;
  };
  globalThis.document = doc;
  globalThis.window = { innerWidth: 1280, addEventListener() {} };
  try {
    const apiStub = assetStub('export {};');
    const { wireTabKeys } = await importAsset('connect.js', { "'/assets/api.js'": apiStub });
    const { initWorkbench, setDrawerOpen } = await importAsset('workbench.js', {
      "'/assets/api.js'": apiStub,
      "'/assets/math.js'": assetStub('export const escapeHtml = String;'),
    });
    const tabs = ['one', 'two'].map((id) => doc.getElementById(id));
    let selected;
    for (const tab of tabs) {
      tab.addEventListener('click', () => { selected = tab; });
      tab.focus = () => { doc.activeElement = tab; };
    }
    wireTabKeys(tabs);
    await tabs[0].dispatch('keydown', { key: 'ArrowLeft', preventDefault() {} });
    assert.equal(selected, tabs[1]);
    assert.equal(doc.activeElement, tabs[1]);
    await tabs[1].dispatch('keydown', { key: 'Home', preventDefault() {} });
    assert.equal(selected, tabs[0]);
    assert.equal(doc.activeElement, tabs[0]);
    for (const [name, id] of [['rail', 'left-rail'], ['canvas', 'canvas-drawer']]) {
      const pane = doc.getElementById(id);
      const toggle = doc.getElementById(`${name}-toggle`);
      toggle.focus = () => { doc.activeElement = toggle; };
      pane.appendChild(tabs[0]);
      doc.activeElement = tabs[0];
      setDrawerOpen(name, false);
      assert.equal(pane.inert, true);
      assert.equal(toggle.getAttribute('aria-expanded'), 'false');
      assert.equal(doc.activeElement, toggle);
      setDrawerOpen(name, true);
      assert.equal(pane.inert, false);
      assert.equal(toggle.getAttribute('aria-expanded'), 'true');
    }
    window.innerWidth = 390;
    initWorkbench();
    const rail = doc.getElementById('left-rail');
    const canvas = doc.getElementById('canvas-drawer');
    const canvasToggle = doc.getElementById('canvas-toggle');
    const canvasClose = doc.getElementById('canvas-close-btn');
    canvas.appendChild(canvasClose);
    canvasClose.addEventListener('click', () => setDrawerOpen('canvas', false));
    canvasClose.focus = () => { doc.activeElement = canvasClose; };
    setDrawerOpen('rail', true);
    setDrawerOpen('canvas', true);
    assert.equal(rail.inert, true);
    assert.equal(canvas.inert, false);
    canvasClose.focus();
    let handled = false;
    handleEscape({
      key: 'Escape', target: canvasClose,
      preventDefault() { handled = true; }, stopPropagation() {},
    });
    assert.equal(handled, true);
    assert.equal(canvas.inert, true);
    assert.equal(doc.activeElement, canvasToggle);
  } finally {
    globalThis.document = previousDoc;
    globalThis.window = previousWindow;
  }
});

test('Stage 14/15 UI contracts: interactive rigor, session conventions, CLI steer guard, delegation & honesty cards', async () => {
  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const apiJs = fs.readFileSync(path.join(assetsDir, 'api.js'), 'utf8');
  const workbenchJs = fs.readFileSync(path.join(assetsDir, 'workbench.js'), 'utf8');
  const composerJs = fs.readFileSync(path.join(assetsDir, 'composer.js'), 'utf8');
  const appJs = fs.readFileSync(path.join(assetsDir, 'app.js'), 'utf8');
  const sourcesJs = fs.readFileSync(path.join(assetsDir, 'canvas_sources.js'), 'utf8');
  const streamJs = fs.readFileSync(path.join(assetsDir, 'stream.js'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');

  assert.match(indexHtml, /data-rigor-preset="interactive"/);
  assert.match(indexHtml, /id="pause-explore-chip"/);
  assert.match(indexHtml, /<select id="subagent-rigor-select"[\s\S]*?<option value="interactive">Interactive<\/option>/);
  assert.match(apiJs, /export function setSessionConventions\(/);
  assert.match(apiJs, /\/conventions`/);
  assert.match(workbenchJs, /api\.setSessionConventions\(sid,\s*trimmed\)/);
  assert.match(workbenchJs, /worker_input_tokens/);
  assert.match(workbenchJs, /delegation_count/);
  assert.match(workbenchJs, /honesty_warnings/);
  assert.match(composerJs, /CLI_ROUTES\s*=\s*new Set\(/);
  assert.match(composerJs, /export function setComposerRoute\(/);
  assert.match(composerJs, /defaultSendHtml/);
  assert.match(appJs, /getActiveSessionId:\s*\(\)\s*=>\s*state\.activeSessionId/);
  assert.match(appJs, /setComposerRoute\(/);
  assert.match(sourcesJs, /ansText\.length > 1500/);
  assert.match(streamJs, /pendingCps\.length === 0/);
  assert.match(fs.readFileSync(path.join(assetsDir, 'hitl.js'), 'utf8'), /branch-summary rich-prose/);
  assert.match(workbenchJs, /wireResponsiveDrawers/);
  assert.match(fs.readFileSync(path.join(assetsDir, 'layout.css'), 'utf8'), /@media \(max-width:\s*780px\)\s*\{\s*#app-shell\s*\{\s*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.ok(assetsRs.includes('"/assets/hitl.js"'));

  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const srcLinksCode = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  const srcLinksDataUrl = JSON.stringify(`data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`);
  const traceMod = await importAsset('trace_cards.js', {
    "'/assets/math.js'": mathUrl,
    "'/assets/source_links.js'": srcLinksDataUrl,
  });

  const apiStubDataUrl = JSON.stringify(`data:text/javascript;base64,${Buffer.from('export async function answerCheckpoint() { return { ok: true }; }', 'utf8').toString('base64')}`);
  const hitlMod = await importAsset('hitl.js', {
    "'/assets/api.js'": apiStubDataUrl,
    "'/assets/math.js'": mathUrl,
  });

  const prevDoc = globalThis.document;
  try {
    globalThis.document = mockDom();

    const expCard = traceMod.buildStructuredTraceCard({
      call_id: 'exp-1',
      tool: 'delegate_explore',
      status: 'ok',
      args: { problem: 'Compare Chern invariant derivations', branches: [{ approach: 'Berry' }, { approach: 'Kitaev <script>x</script>' }] },
      output: JSON.stringify({
        model: 'claude-haiku-4-5',
        branch_count: 2,
        turns: 4,
        input_tokens: 120,
        output_tokens: 45,
        branches: [
          { approach: 'Berry', ok: true, summary: 'Converged cleanly' },
          { approach: 'Kitaev', ok: false, error: 'Missing <script>y</script>' },
        ],
      }),
    });
    assert.match(expCard.innerHTML, /Parallel Explorer \(Ultramath\)/);
    assert.match(expCard.innerHTML, /2 branches/);
    assert.doesNotMatch(expCard.innerHTML, /<script>/i);

    const leanCard = traceMod.buildStructuredTraceCard({
      call_id: 'lean-1',
      tool: 'lean_check',
      status: 'ok',
      args: { path: 'Main.lean' },
      output: JSON.stringify({ status: 'verified', axioms: ['propext'], diagnostics: [] }),
    });
    assert.match(leanCard.innerHTML, /Lean 4 Checker/);
    assert.match(leanCard.innerHTML, /axioms: propext/);

    const hCard = traceMod.buildHonestyNoticeCard({
      passed: false,
      summary: 'Overclaim detected',
      findings: [{ severity: 'warning', claim: 'q.e.d.', reason: 'Symbolic step not verified <img src=x onerror=alert(1)>' }],
    });
    assert.ok(hCard);
    assert.match(hCard.innerHTML, /Honesty Linter/);
    assert.doesNotMatch(hCard.innerHTML, /<img\b/i);

    const stream = mockDom().createElement('section');
    hitlMod.appendSteerBadge(stream, 'Use gauge <script>alert(1)</script>');
    const inner = stream.children[0];
    assert.equal(inner.children[0].className, 'steer-badge-row');
    assert.doesNotMatch(inner.children[0].innerHTML, /<script>/i);
  } finally {
    globalThis.document = prevDoc;
  }
});

test('Antigravity consent requires confirmation and can be withdrawn from Settings', async (t) => {
  const calls = [];
  const notice = "Review Antigravity's terms and built-in tool access.";
  let allow = false;
  let acknowledged = false;
  let changed = 0;
  const dom = mockDom(['settings-dialog', 'connect-list']);
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  t.after(() => {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    delete globalThis.connectConsentApi;
  });
  globalThis.document = dom;
  globalThis.window = {
    confirm: (message) => {
      assert.ok(message.includes(notice));
      return allow;
    },
  };
  globalThis.connectConsentApi = {
    getConnections: async () => ({ connections: [{
      id: 'antigravity',
      label: 'Antigravity',
      found: true,
      connected: acknowledged,
      acknowledged,
      warning: notice,
      acknowledge_action: 'acknowledge',
      withdraw_action: 'withdraw',
    }] }),
    connectionAction: async (provider, action) => {
      calls.push([provider, action]);
      acknowledged = action === 'acknowledge';
    },
    getProviderSettings: async () => ({}),
  };
  const api = 'export const { getConnections, connectionAction, getProviderSettings } = globalThis.connectConsentApi;';
  const connect = await importAsset('connect.js', {
    "'/assets/api.js'": JSON.stringify(`data:text/javascript;base64,${Buffer.from(api).toString('base64')}`),
  });
  connect.initConnect({ onChanged: () => { changed += 1; } });
  await connect.openConnect();
  const warning = () => dom.getElementById('connect-list').children[0].children.find((el) => el.className === 'connect-warning');
  const button = () => warning().children.find((el) => el.className === 'connect-actions').children[0];
  assert.equal(warning().children[1].textContent, notice);
  assert.equal(button().textContent, 'I understand the risk');
  await button().click();
  assert.deepEqual(calls, []);
  allow = true;
  await button().click();
  assert.deepEqual(calls, [['antigravity', 'acknowledge']]);
  assert.equal(button().textContent, 'Withdraw acknowledgement');
  allow = false;
  await button().click();
  assert.deepEqual(calls, [['antigravity', 'acknowledge'], ['antigravity', 'withdraw']]);
  assert.equal(button().textContent, 'I understand the risk');
  assert.equal(changed, 2);
});

test('Composer retains only failed attachments and preserves the prompt after a partial upload', async (t) => {
  const ids = ['composer-pill', 'composer-input', 'composer-upload-input', 'composer-files'];
  const dom = mockDom(ids);
  const [form, input, upload, chips] = ids.map((id) => dom.getElementById(id));
  const uploads = [];
  const notices = [];
  const sent = [];
  const refreshed = [];
  const previousDocument = globalThis.document;
  t.after(() => {
    globalThis.document = previousDocument;
    delete globalThis.composerUpload;
  });
  globalThis.document = dom;
  globalThis.composerUpload = async (id, files) => {
    uploads.push([id, files]);
    if (uploads.length === 1) {
      throw Object.assign(new Error('unreadable file'), {
        status: 400,
        payload: {
          ingested: [{ origin: 'notes.txt' }],
          failed: [{ index: 1, name: 'notes.txt', error: 'unreadable file' }],
        },
      });
    }
    return { ingested: [{ origin: 'notes.txt' }], failed: [] };
  };
  const moduleUrl = (source) => JSON.stringify(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const composer = await importAsset('composer.js', {
    "'/assets/api.js'": moduleUrl('export const uploadFilesBase64 = globalThis.composerUpload;'),
    "'/assets/math.js'": JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href),
    "'/assets/effort_picker.js'": moduleUrl('export const closeEffortWindow = () => {}; export const getReasoningEffort = () => null;'),
    "'/assets/subagents.js'": moduleUrl('export const getActiveSubagent = () => null;'),
  });
  composer.initComposer({
    ensureSession: async () => 's-notes',
    getModelChoice: () => ({ provider: 'mock', model: 'test' }),
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
    onNotify: (message) => notices.push(message),
    onSourcesUpdated: (id) => refreshed.push(id),
  });
  composer.setComposerSession('s-notes');
  const file = (byte) => ({ name: 'notes.txt', size: 1, arrayBuffer: async () => Uint8Array.of(byte).buffer });
  upload.files = [file(97), file(0)];
  await upload.dispatch('change');
  input.value = 'Read these notes';
  await form.dispatch('submit');
  assert.match(notices[0], /Added .*notes\.txt.* Could not add .*notes\.txt.*unreadable file/);
  assert.deepEqual(refreshed, ['s-notes']);
  assert.equal(input.value, 'Read these notes');
  assert.equal(chips.children.length, 1);
  assert.deepEqual(sent, []);

  await form.dispatch('submit');
  assert.deepEqual(uploads[1], ['s-notes', [{ name: 'notes.txt', content_base64: 'AA==' }]]);
  assert.deepEqual(sent, [['s-notes', 'Read these notes']]);
  assert.equal(chips.children.length, 0);
});

async function initDraftComposer(t, { uploadFiles, onSendPrompt, onSteerTurn, ensureSession } = {}) {
  const ids = ['composer-pill', 'composer-input', 'composer-upload-input', 'composer-files'];
  const dom = mockDom(ids);
  const [form, input, upload, chips] = ids.map((id) => dom.getElementById(id));
  const previousDocument = globalThis.document;
  t.after(() => {
    globalThis.document = previousDocument;
    delete globalThis.composerDraftUpload;
  });
  globalThis.document = dom;
  globalThis.composerDraftUpload = uploadFiles || (async (_id, files) => ({ ingested: files, failed: [] }));
  const composer = await importAsset('composer.js', {
    "'/assets/api.js'": assetStub(`export const uploadFilesBase64 = globalThis.composerDraftUpload; // ${t.name}`),
    "'/assets/math.js'": assetStub('export const escapeHtml = String;'),
    "'/assets/effort_picker.js'": assetStub('export const closeEffortWindow = () => {}; export const getReasoningEffort = () => null;'),
    "'/assets/subagents.js'": assetStub('export const getActiveSubagent = () => null;'),
  });
  let current = null;
  composer.initComposer({
    ensureSession: ensureSession ? () => ensureSession(composer) : async () => current,
    getModelChoice: () => ({ provider: 'mock', model: 'test' }),
    onSendPrompt, onSteerTurn,
  });
  return {
    composer, input, chips,
    select(id, opts) {
      current = id;
      composer.setComposerSession(id, opts);
    },
    openFilePicker: () => upload.click(),
    stage: (name, arrayBuffer = async () => Uint8Array.of(1).buffer) => {
      upload.files = [{ name, size: 1, arrayBuffer }];
      return upload.dispatch('change');
    },
    submit: () => form.dispatch('submit'),
  };
}

test('Composer keeps Project A paper and draft out of Project B and restores them on return', async (t) => {
  const sent = [];
  const uploads = [];
  const draft = await initDraftComposer(t, {
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
    uploadFiles: async (id, files) => { uploads.push([id, files]); return { ingested: files, failed: [] }; },
  });
  draft.select('project-a-session');
  draft.input.value = 'Project A draft';
  await draft.stage('project-a-paper.pdf');
  draft.select('project-b-session');
  assert.equal(draft.input.value, '');
  assert.equal(draft.chips.children.length, 0);
  draft.input.value = 'Project B draft';
  await draft.submit();
  assert.deepEqual(sent, [['project-b-session', 'Project B draft']]);
  assert.deepEqual(uploads, []);

  draft.select('project-a-session');
  assert.equal(draft.input.value, 'Project A draft');
  assert.match(draft.chips.children[0].innerHTML, /project-a-paper\.pdf/);
  await draft.submit();
  assert.deepEqual(sent[1], ['project-a-session', 'Project A draft']);
  assert.deepEqual(uploads[0], ['project-a-session', [{ name: 'project-a-paper.pdf', content_base64: 'AQ==' }]]);
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test('A first send moves the blank draft and paper into the session it creates', async (t) => {
  const uploads = [];
  const sent = [];
  const draft = await initDraftComposer(t, {
    ensureSession: async (composer) => {
      composer.setComposerSession('first-session', { adoptDraft: true });
      return 'first-session';
    },
    uploadFiles: async (id, files) => { uploads.push([id, files[0].name]); return { ingested: files, failed: [] }; },
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
  });
  draft.input.value = 'Research prompt for my first chat';
  await draft.stage('first-paper.pdf');
  await draft.submit();
  assert.deepEqual(uploads, [['first-session', 'first-paper.pdf']]);
  assert.deepEqual(sent, [['first-session', 'Research prompt for my first chat']]);
  assert.equal(draft.input.value, '');
  draft.select(null);
  assert.equal(draft.chips.children.length, 0);
  assert.equal(draft.input.value, '');
});

test('A file picker that finishes after switching chats cannot stage the paper in the new chat', async (t) => {
  const draft = await initDraftComposer(t);
  draft.select('a');
  await draft.openFilePicker();
  draft.select('b');
  await draft.stage('a.pdf');
  assert.equal(draft.chips.children.length, 0);
  draft.select('a');
  assert.equal(draft.chips.children.length, 0);
});

test('A Send interrupted while reading a paper preserves its draft and never uploads after returning', async (t) => {
  const reading = deferred();
  const bytes = deferred();
  const uploads = [];
  const sent = [];
  const draft = await initDraftComposer(t, {
    uploadFiles: async (id, files) => { uploads.push([id, files]); return { ingested: files, failed: [] }; },
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
  });
  draft.select('a');
  draft.input.value = 'Draft A';
  await draft.stage('a.pdf', () => { reading.resolve(); return bytes.promise; });
  const submitted = draft.submit();
  await reading.promise;
  draft.select('b');
  draft.select('a');
  bytes.resolve(Uint8Array.of(1).buffer);
  await submitted;
  assert.deepEqual(uploads, []);
  assert.deepEqual(sent, []);
  assert.equal(draft.input.value, 'Draft A');
  assert.match(draft.chips.children[0].innerHTML, /a\.pdf/);
});

test('A paper receipt updates its originating draft while another draft can submit', async (t) => {
  const uploading = deferred();
  const receipt = deferred();
  const sent = [];
  const draft = await initDraftComposer(t, {
    uploadFiles: (id) => {
      assert.equal(id, 'a');
      uploading.resolve();
      return receipt.promise;
    },
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
  });
  draft.select('a');
  draft.input.value = 'Draft A';
  await draft.stage('a-success.pdf');
  await draft.stage('a-failed.pdf');
  const submitted = draft.submit();
  await uploading.promise;
  draft.select('b');
  draft.input.value = 'Draft B';
  await draft.submit();
  await draft.stage('b-next.pdf');
  receipt.resolve({ ingested: [{ origin: 'a-success.pdf' }], failed: [{ index: 1, error: 'unreadable file' }] });
  await submitted;
  assert.deepEqual(sent, [['b', 'Draft B']]);
  assert.match(draft.chips.children[0].innerHTML, /b-next\.pdf/);
  draft.select('a');
  assert.equal(draft.input.value, 'Draft A');
  assert.equal(draft.chips.children.length, 1);
  assert.match(draft.chips.children[0].innerHTML, /a-failed\.pdf/);
});

test('A failed Send restores only the originating draft after navigation', async (t) => {
  const started = deferred();
  const sent = deferred();
  const draft = await initDraftComposer(t, {
    onSendPrompt: (id, prompt) => {
      assert.deepEqual([id, prompt], ['a', 'Original prompt']);
      started.resolve();
      return sent.promise;
    },
  });
  draft.select('a');
  draft.input.value = 'Original prompt';
  const submitting = draft.submit();
  await started.promise;
  draft.select('b');
  draft.input.value = 'New draft in B';
  sent.reject(new Error('Provider rejected the request'));
  await submitting;
  assert.equal(draft.input.value, 'New draft in B');
  draft.select('a');
  assert.equal(draft.input.value, 'Original prompt');
});

test('A rejected steer restores only the originating session draft', async (t) => {
  const request = deferred();
  const steer = [];
  const draft = await initDraftComposer(t, {
    onSteerTurn: (id, prompt) => {
      steer.push([id, prompt]);
      return request.promise;
    },
  });
  draft.select('a');
  draft.composer.setComposerBusy(true);
  draft.input.value = 'Steer for A';
  const submitting = draft.submit();
  draft.select('b');
  draft.composer.setComposerBusy(false);
  draft.input.value = 'Draft B';
  request.reject(new Error('Turn finished'));
  await submitting;
  assert.deepEqual(steer, [['a', 'Steer for A']]);
  assert.equal(draft.input.value, 'Draft B');
  draft.select('a');
  assert.equal(draft.input.value, 'Steer for A');
});

test('Entering and leaving Projects cancels a deferred submit in the current chat', async (t) => {
  const session = deferred();
  const sent = [];
  const draft = await initDraftComposer(t, {
    ensureSession: () => session.promise,
    onSendPrompt: (id, prompt) => sent.push([id, prompt]),
  });
  draft.select('a');
  draft.input.value = 'Pending draft';
  const submitted = draft.submit();
  draft.composer.setComposerVisible(false);
  draft.composer.setComposerVisible(true);
  session.resolve('a');
  await submitted;
  assert.deepEqual(sent, []);
  assert.equal(draft.input.value, 'Pending draft');
});

test('First prompt can wait for EventSource to subscribe, including one reconnect', async (t) => {
  const original = globalThis.EventSource;
  t.after(() => { globalThis.EventSource = original; });
  globalThis.EventSource = class MockEventSource extends EventTarget {
    static OPEN = 1;
    static CLOSED = 2;
    readyState = 0;
  };
  const { waitForSessionEvents } = await importAsset('api.js');
  const source = new EventSource();
  let ready;
  const awaiting = waitForSessionEvents(source, 60).then((value) => { ready = value; });
  source.dispatchEvent(new Event('error'));
  await Promise.resolve();
  assert.equal(ready, undefined);
  source.readyState = EventSource.OPEN;
  source.dispatchEvent(new Event('open'));
  await awaiting;
  assert.equal(ready, true);
  assert.equal(await waitForSessionEvents(source), true);

  source.readyState = 0;
  const aborting = waitForSessionEvents(source, 60);
  source.readyState = EventSource.CLOSED;
  source.dispatchEvent(new Event('error'));
  assert.equal(await aborting, false);
  assert.equal(await waitForSessionEvents(new EventSource(), 1), false);
});

test('Escape is left to native dialogs, and accepted Stop is sent once', async (t) => {
  const previous = globalThis.document;
  t.after(() => { globalThis.document = previous; });
  const doc = mockDom(['settings-dialog', 'workspace-dialog', 'source-reader-dialog', 'composer-input',
    'composer-stop', 'topbar-stop-btn', 'composer-send']);
  const settings = doc.getElementById('settings-dialog');
  const workspace = doc.getElementById('workspace-dialog');
  const reader = doc.getElementById('source-reader-dialog');
  const stop = doc.getElementById('composer-stop');
  const stopInHeader = doc.getElementById('topbar-stop-btn');
  const summary = doc.createElement('summary');
  summary.focus = () => { doc.activeElement = summary; };
  doc.querySelector = (selector) => selector === '#research-options > summary' ? summary : null;
  const documentKeys = [];
  doc.addEventListener = (name, handler) => { if (name === 'keydown') documentKeys.push(handler); };
  globalThis.document = doc;
  const apiStub = assetStub('export {};');
  const { initConnect, openConnect } = await importAsset('connect.js', { "'/assets/api.js'": apiStub });
  const composer = await importAsset('composer.js', {
    "'/assets/api.js'": apiStub,
    "'/assets/math.js'": assetStub('export const escapeHtml = String;'),
    "'/assets/effort_picker.js'": assetStub('export const closeEffortWindow = () => {}; export const getReasoningEffort = () => null;'),
    "'/assets/subagents.js'": assetStub('export const getActiveSubagent = () => null;'),
  });
  const notices = [];
  let calls = 0;
  initConnect();
  const collapsedTrigger = doc.createElement('button');
  collapsedTrigger.closest = () => ({ querySelector: () => summary });
  doc.activeElement = collapsedTrigger;
  await openConnect();
  // Simulate the browser focusing the body after the original control becomes hidden.
  doc.activeElement = null;
  await settings.close();
  assert.equal(doc.activeElement, summary);
  composer.initComposer({ onStopTurn: async () => { calls += 1; return true; }, onNotify: (text) => notices.push(text) });
  composer.setComposerBusy(true);
  const escape = () => ({
    key: 'Escape',
    defaultPrevented: false,
    preventDefault() { this.defaultPrevented = true; },
  });
  for (const modal of [settings, workspace, reader]) {
    modal.showModal();
    const event = escape();
    documentKeys.forEach((handler) => handler(event));
    assert.equal(event.defaultPrevented, false);
    await modal.close();
  }
  assert.equal(doc.activeElement, summary);
  assert.equal(calls, 0);

  await stop.click();
  composer.setComposerBusy(true);
  assert.equal(stop.disabled, true);
  assert.equal(stopInHeader.disabled, true);
  await stopInHeader.click();
  documentKeys.forEach((handler) => handler(escape()));
  assert.equal(calls, 1);
  composer.setComposerBusy(false);
  assert.equal(stop.hidden, true);
  assert.equal(stop.disabled, false);

  const input = doc.getElementById('composer-input');
  composer.reuseComposerPrompt('Earlier failed prompt');
  assert.equal(input.value, 'Earlier failed prompt');
  composer.reuseComposerPrompt('Other prompt');
  assert.equal(input.value, 'Earlier failed prompt');
  assert.match(notices.at(-1), /current draft/);
});
