import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { assetsDir, assetStub, importAsset, mathAssetUrl, mockDom } from './hitl_ui_helper.mjs';

test('Antigravity renders in Settings as Antigravity (Google) with no consent box or confirm dialog', async (t) => {
  let confirmCalls = 0;
  let actionCalls = 0;
  const dom = mockDom(['settings-dialog', 'connect-list']);
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  t.after(() => {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    delete globalThis.connectAgyApi;
  });
  globalThis.document = dom;
  globalThis.window = {
    confirm: () => {
      confirmCalls += 1;
      return true;
    },
  };
  globalThis.connectAgyApi = {
    getConnections: async () => ({
      connections: [{
        id: 'antigravity',
        label: 'Antigravity (Google)',
        kind: 'subscription',
        route: 'antigravity',
        found: true,
        connected: true,
        status: 'Antigravity CLI 1.5.14 ready.',
        session_note: 'Antigravity CLI',
        // Even if an unexpected warning field appeared, connect.js renders no consent box.
        warning: 'stale-warning',
      }],
    }),
    connectionAction: async () => { actionCalls += 1; },
    getProviderSettings: async () => ({}),
  };
  const api = 'export const { getConnections, connectionAction, getProviderSettings } = globalThis.connectAgyApi;';
  const connect = await importAsset('connect.js', {
    "'/assets/api.js'": JSON.stringify(`data:text/javascript;base64,${Buffer.from(api).toString('base64')}`),
  });
  connect.initConnect();
  await connect.openConnect();
  const row = dom.getElementById('connect-list').children[0];
  assert.equal(row.getAttribute('aria-label'), 'Antigravity (Google)');
  const head = row.children[0];
  assert.equal(head.children[0].textContent, 'Antigravity (Google)');
  assert.equal(head.children[1].textContent, 'Connected');
  assert.equal(row.children.find((el) => el.className === 'connect-warning'), undefined);
  assert.equal(confirmCalls, 0);
  assert.equal(actionCalls, 0);
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
    "'/assets/math.js'": mathAssetUrl,
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
    "'/assets/math.js'": mathAssetUrl,
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
