import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../assets/app.js', import.meta.url), 'utf8');
const returningSession = { id: 'existing', title: 'Existing research', messages: [] };

function deferred() {
  let resolve;
  const promise = new Promise((settle) => { resolve = settle; });
  return { promise, resolve };
}

async function startApp(t, { listSessions, createSession, loadModels, waitForSessionEvents, width = 1280 }) {
  const mockKey = '__navigationTest_' + t.name.replace(/\W/g, '_');
  const elements = new Map();
  const sessionReads = [];
  const modelSelections = [];
  const draftSelections = [];
  const sent = [];
  const projectReady = deferred();
  let boot;
  let selectFromRail;
  const element = (id) => {
    if (!elements.has(id)) {
      const attrs = new Map(), listeners = new Map();
      elements.set(id, {
        hidden: true, classList: { add() {}, remove() {}, toggle() {} }, style: {},
        setAttribute: (name, value) => attrs.set(name, value),
        getAttribute: (name) => attrs.get(name),
        addEventListener: (name, fn) => listeners.set(name, fn),
        click: () => listeners.get('click')?.(),
        focus() { document.activeElement = this; },
      });
    }
    return elements.get(id);
  };
  const shell = element('app-shell');
  shell.setAttribute('data-canvas-open', 'false');
  const previous = { document: globalThis.document, window: globalThis.window };
  globalThis.document = {
    documentElement: element('html'), getElementById: element,
    querySelector() { return null; }, querySelectorAll() { return []; },
    addEventListener(name, handler) { if (name === 'DOMContentLoaded') boot = handler; },
  };
  globalThis.window = {
    innerWidth: width, localStorage: { getItem() {}, setItem() {} },
    addEventListener() {}, confirm: () => true,
  };
  const mock = {
    initAuth: async () => true,
    getHealth: async () => ({ ok: true }),
    listSessions,
    createSession,
    loadModels,
    deleteSession: async () => {},
    getSession: async (id) => {
      sessionReads.push(id);
      return { ...returningSession, id };
    },
    openSessionEvents: () => ({ close() {} }),
    waitForSessionEvents,
    sendMessage: (id, payload) => { sent.push([id, payload]); },
    initComposer: (callbacks) => { mock.composer = callbacks; },
    setComposerSession: (id, options) => { draftSelections.push([id, options]); },
    initProjects(callbacks) {
      mock.onVisibilityChange = callbacks.onVisibilityChange;
      mock.onProjectSelected = callbacks.onProjectSelected;
      mock.onCreateSession = callbacks.onCreateSession;
      projectReady.resolve();
    },
    openProjects() {
      element('project-overview').hidden = false;
      mock.onVisibilityChange(true);
    },
    closeProjects() {
      if (element('project-overview').hidden) return;
      element('project-overview').hidden = true;
      mock.onVisibilityChange(false);
    },
    initStream: (options) => { mock.openFromWelcome = options.onOpenProjects; },
    renderSessionRailList(_element, options) {
      selectFromRail = options.onSelect;
      mock.deleteFromRail = options.onDelete;
    },
    setDrawerOpen: (drawer, open) => shell.setAttribute('data-' + drawer + '-open', String(open)),
    applySession: (id) => modelSelections.push(id),
  };
  globalThis[mockKey] = mock;
  t.after(() => {
    Object.assign(globalThis, previous);
    delete globalThis[mockKey];
  });
  const dataUrl = (code) => 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
  const code = source.replace(/import\s+([\s\S]*?)\s+from '\/assets\/([^']+)';/g, (_, imports) => {
    const exports = imports.startsWith('*')
      ? ['initAuth', 'getHealth', 'listSessions', 'getSession', 'openSessionEvents', 'createSession', 'deleteSession',
         'waitForSessionEvents', 'sendMessage']
      : imports.match(/[A-Za-z_]\w*/g);
    const stub = exports.map((name) =>
      `export function ${name}(...args) { return globalThis[${JSON.stringify(mockKey)}].${name}?.(...args); }`,
    ).join('\n');
    return `import ${imports} from '${dataUrl(stub)}';`;
  });
  await import(dataUrl(code));
  const booted = boot();
  await projectReady.promise;
  return {
    mock, booted, element, sessionReads, modelSelections, draftSelections, sent,
    selectFromRail: (id) => selectFromRail(id),
    canvasOpen: () => shell.getAttribute('data-canvas-open'),
  };
}

test('Startup respects Projects opened before the session list arrives', async (t) => {
  const list = deferred();
  const app = await startApp(t, { listSessions: () => list.promise });
  app.mock.openFromWelcome();
  list.resolve([returningSession]);
  await app.booted;
  assert.equal(app.element('project-overview').hidden, false);
  assert.deepEqual(app.sessionReads, []);

  await app.selectFromRail(returningSession.id);
  assert.equal(app.element('project-overview').hidden, true);
  assert.equal(document.activeElement, app.element('active-session-title'));
  assert.deepEqual(app.sessionReads, [returningSession.id]);
});

test('New session completion preserves Projects entered while creating it', async (t) => {
  const create = deferred();
  const app = await startApp(t, { listSessions: async () => [], createSession: () => create.promise });
  await app.booted;
  const newSession = app.element('new-research-btn').click();
  app.mock.openProjects();
  create.resolve({ id: 'new' });
  await newSession;
  assert.equal(app.element('project-overview').hidden, false);
  assert.deepEqual(app.sessionReads, []);
});

test('Creating a linked session keeps a project selected during the request open', async (t) => {
  const create = deferred();
  const app = await startApp(t, { listSessions: async () => [], createSession: () => create.promise });
  await app.booted;
  app.mock.openProjects();
  const newSession = app.mock.onCreateSession('previous-project');
  app.mock.onProjectSelected();
  create.resolve({ id: 'new-project-session' });
  await newSession;
  assert.equal(app.element('project-overview').hidden, false);
  assert.deepEqual(app.sessionReads, []);
});

test('Startup opens the desktop Canvas for a returning session while model loading continues', async (t) => {
  const models = deferred();
  const app = await startApp(t, { listSessions: async () => [returningSession], loadModels: () => models.promise });
  await new Promise(setImmediate);
  assert.deepEqual(app.sessionReads, [returningSession.id]);
  assert.equal(app.canvasOpen(), 'true');
  models.resolve();
  await app.booted;
});

test('Startup preserves a Canvas explicitly opened from a blank desktop', async (t) => {
  const list = deferred();
  const app = await startApp(t, { listSessions: () => list.promise });
  assert.equal(app.canvasOpen(), 'false');
  app.element('canvas-toggle').click();
  list.resolve([]);
  await app.booted;
  assert.equal(app.canvasOpen(), 'true');
});

test('Deleting the last session clears the session model from the blank workbench', async (t) => {
  let list = [returningSession];
  const app = await startApp(t, { listSessions: async () => list });
  await app.booted;
  assert.deepEqual(app.modelSelections, [returningSession.id]);
  list = [];
  await app.mock.deleteFromRail(returningSession);
  assert.deepEqual(app.modelSelections, [returningSession.id, null]);
  assert.equal(app.element('active-session-title').textContent, 'New research');
});

test('App adopts the blank draft only for the chat created by the composer', async (t) => {
  const app = await startApp(t, {
    listSessions: async () => [], createSession: async () => ({ id: 'first-send' }),
  });
  await app.booted;
  assert.equal(await app.mock.composer.ensureSession(), 'first-send');
  assert.deepEqual(app.draftSelections, [[null, undefined], ['first-send', { adoptDraft: true }]]);
});

test('App cancels a Send when Projects opens during live connection setup', async (t) => {
  const connected = deferred();
  const app = await startApp(t, {
    listSessions: async () => [returningSession], waitForSessionEvents: () => connected.promise,
  });
  await app.booted;
  const sending = app.mock.composer.onSendPrompt(returningSession.id, 'Still a draft', {});
  app.mock.openProjects();
  connected.resolve(true);
  await assert.rejects(sending, /research view changed/);
  assert.deepEqual(app.sent, []);
});
