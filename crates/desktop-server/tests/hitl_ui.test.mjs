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

test('Proof graph stays with the selected session and forks at the displayed sequence', async () => {
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
    dag.selectNodeById('e-b');
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

test('Late LaTeX file responses cannot replace or be saved into a different session', async () => {
  let finishOldFile;
  const saved = [];
  const previousDoc = globalThis.document;
  const doc = mockDom(['latex-file-select', 'latex-editor', 'latex-save-btn']);
  globalThis.document = doc;
  globalThis.canvasTestApi = {
    getWorkspace: async (id, path) => {
      if (!path) return { files: [`${id}.tex`] };
      if (id === 'old') return new Promise((resolve) => { finishOldFile = resolve; });
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
    await save.click();
    assert.deepEqual(saved, [['current', { path: 'current.tex', search: 'Current proof', replace: 'Revised proof', content: 'Revised proof' }]]);
    await latex.refreshWorkspaceFiles(null);
    assert.equal(editor.value, '');
    assert.equal(save.disabled, true);
  } finally {
    globalThis.document = previousDoc;
    delete globalThis.canvasTestApi;
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
