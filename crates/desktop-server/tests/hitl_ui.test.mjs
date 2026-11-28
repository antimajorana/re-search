import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { assetsDir, assetStub, crateRoot, importAsset, mathAssetUrl, mockDom } from './hitl_ui_helper.mjs';

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
      "'/assets/math.js'": mathAssetUrl,
      "'/assets/model_picker.js'": assetStub('export const getModelChoice = () => null;'),
      "'/assets/canvas_sources.js'": assetStub('export const openSessionSource = () => {};'),
      "'/assets/canvas_dag_detail.js'": assetStub(
        "export const renderAssumptionImpact = () => {}; export const renderDagNodeDetail = (element, entry) => { element.textContent = entry?.entry_id || ''; };",
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
      if (!path) return id === 'scratch' ? { files: [], workspace_root: null } : { files: [`${id}.tex`], workspace_root: '/paper' };
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
      "'/assets/math.js'": mathAssetUrl,
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
    await latex.refreshWorkspaceFiles('scratch');
    editor.value = 'A scratch calculation';
    assert.equal(save.disabled, true);
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
    await latex.refreshWorkspaceFiles('scratch');
    assert.equal(editor.value, 'A scratch calculation');
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
      "'/assets/math.js'": mathAssetUrl,
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
      "'/assets/math.js'": mathAssetUrl,
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
    "'/assets/stream.js'": assetStub('export const scrollToBottom = () => {};'),
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
