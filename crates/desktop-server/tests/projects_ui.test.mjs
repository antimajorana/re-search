import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const assets = new URL('../assets/', import.meta.url);

function element() {
  const handlers = new Map();
  return {
    attributes: new Map(),
    dataset: {},
    children: [],
    value: '',
    textContent: '',
    hidden: false,
    disabled: false,
    setAttribute(name, value) { this.attributes.set(name, value); },
    getAttribute(name) { return this.attributes.get(name); },
    removeAttribute(name) { this.attributes.delete(name); },
    addEventListener(event, callback) { handlers.set(event, callback); },
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = nodes; },
    focus() { globalThis.document.activeElement = this; },
    async trigger(event) { return handlers.get(event)?.({ currentTarget: this, preventDefault() {} }); },
  };
}

function moduleUrl(source) {
  return JSON.stringify(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
}

test('Project overview saves notes and links proof steps to papers', async (t) => {
  const html = await readFile(new URL('index.html', assets), 'utf8');
  const nodes = new Map([...html.matchAll(/\bid="(app-shell|projects?-[^"]+)"/g)].map(([, id]) => [id, element()]));
  const el = (id) => nodes.get(id);
  const project = (id, title, extras = {}) => ({
    id, title, revision: 1, question: '', conventions: '', session_ids: [], items: [], sources: [], ...extras,
  });
  const paper = { id: 'paper-1', title: 'Bounds on the spectral gap' };
  const records = [project('p-old', 'Spectral gap', { sources: [paper] })];
  const sessions = [{ id: 's-one', title: 'Check the recurrence' }];
  const opened = [];
  const refreshed = [];
  const changes = [];
  const exports = ['listProjects', 'createProject', 'getProject', 'updateProject', 'linkProjectSession', 'addProjectItem', 'deleteProjectItem'];
  const tabs = [...html.matchAll(/id="(project-tab-[^"]+)" data-project-tab="([^"]+)"[^>]*aria-controls="([^"]+)"/g)]
    .map(([, id, name, controls]) => {
      const node = el(id);
      node.dataset.projectTab = name;
      node.setAttribute('aria-controls', controls);
      return node;
    });
  el('project-tabs').querySelectorAll = () => tabs;
  const savedValues = element();
  el('project-edit-conflicts').querySelector = () => savedValues;
  el('project-overview').hidden = true;
  for (const id of ['project-finding-form', 'project-open-question-form']) {
    const submit = element();
    el(id).querySelector = () => submit;
  }

  const before = { document: globalThis.document, window: globalThis.window };
  globalThis.document = { getElementById: el, createElement: element, activeElement: null };
  globalThis.window = { localStorage: { getItem: () => null, setItem() {} }, confirm: () => true };
  const find = (id) => records.find((record) => record.id === id);
  const apply = (id, update) => {
    const record = find(id);
    update(record);
    record.revision += 1;
    return structuredClone(record);
  };
  globalThis.projectTest = {
    listProjects: () => ({ projects: structuredClone(records) }),
    createProject: (payload) => {
      const created = project('p-new', payload.title, payload);
      records.push(created);
      return structuredClone(created);
    },
    getProject: (id) => structuredClone(find(id)),
    updateProject: (id, { revision, ...payload }) => {
      if (revision !== find(id).revision) throw Object.assign(new Error('Project changed'), { status: 409 });
      return apply(id, (record) => Object.assign(record, payload));
    },
    linkProjectSession: (id, sid) => {
      sessions[0].project_id = id;
      return apply(id, (record) => record.session_ids.push(sid));
    },
    addProjectItem: (id, payload) => apply(id, (record) => {
      record.items.push({ id: `i-${record.items.length + 1}`, ...payload });
    }),
    deleteProjectItem: (id, item) => apply(id, (record) => {
      record.items = record.items.filter((note) => note.id !== item);
    }),
    focusProjectSource: (id) => changes.push(id),
  };
  t.after(() => {
    Object.assign(globalThis, before);
    delete globalThis.projectTest;
  });

  let source = await readFile(new URL('projects.js', assets), 'utf8');
  source = source.replace("'/assets/project_edits.js'", JSON.stringify(new URL('project_edits.js', assets).href));
  for (const [name, stub] of Object.entries({
    'api.js': exports.map((name) => `export const ${name} = globalThis.projectTest.${name};`).join('\n'),
    'connect.js': 'export const wireTabKeys = () => {};',
    'math.js': 'export const renderRichInto = (node, text) => { node.textContent = text; }; export const renderMathTextInto = renderRichInto;',
    'project_library.js': 'export const focusProjectSource = globalThis.projectTest.focusProjectSource; export const initProjectLibrary = () => {}; export const renderProjectLibrary = () => {};',
    'project_validation.js': 'export const initProjectValidation = () => {}; export const loadProjectValidation = () => {}; export const setValidationProject = () => {};',
    'experiments.js': 'export const initProjectExperiments = () => {}; export const loadProjectExperiments = () => {}; export const refreshExperimentSessions = () => {}; export const setExperimentProject = () => {};',
  })) source = source.replace(`'/assets/${name}'`, moduleUrl(stub));
  const ui = await import(JSON.parse(moduleUrl(source)));
  ui.initProjects({
    getActiveProjectId: () => null,
    getSessions: () => sessions,
    onNotify: () => {},
    onSessionsChanged: () => ui.refreshProjectSessions(),
    onEvidenceChanged: (id) => refreshed.push(id),
    onOpenSession: (...ref) => opened.push(ref),
  });

  await el('projects-open-btn').trigger('click');
  assert.equal(el('project-title').textContent, 'Spectral gap');
  await el('project-create-btn').trigger('click');
  el('project-title-input').value = 'Finite-size corrections';
  el('project-question-input').value = 'Which boundary condition fixes the leading term?';
  el('project-conventions-input').value = 'Set $J=1$.';
  await el('project-edit-form').trigger('submit');
  assert.equal(el('project-title').textContent, 'Finite-size corrections');
  assert.equal(el('project-conventions').textContent, 'Set $J=1$.');
  await el('project-edit-btn').trigger('click');
  el('project-question-input').value = 'Find the correction for open boundaries.';
  apply('p-new', (record) => { record.conventions = 'Set $J = 2$.'; });
  await el('project-edit-form').trigger('submit');
  assert.equal(find('p-new').question, 'Find the correction for open boundaries.');
  assert.equal(find('p-new').conventions, 'Set $J = 2$.');

  await el('project-edit-btn').trigger('click');
  el('project-question-input').value = 'Check complex boundary conditions.';
  apply('p-new', (record) => { record.question = 'How does a twist change the gap?'; });
  await el('project-edit-form').trigger('submit');
  assert.equal(el('project-edit-form').hidden, false);
  assert.equal(el('project-question-input').value, 'Check complex boundary conditions.');
  assert.equal(savedValues.children.at(-1).textContent, 'How does a twist change the gap?');
  assert.equal(find('p-new').question, 'How does a twist change the gap?');
  await el('project-edit-form').trigger('submit');
  assert.equal(find('p-new').question, 'Check complex boundary conditions.');

  el('project-finding-text').value = 'The periodic ansatz misses a boundary term.';
  el('project-finding-kind').value = 'failed_approach';
  await el('project-finding-form').trigger('submit');
  el('project-open-question-text').value = 'Does the correction persist at nonzero temperature?';
  await el('project-open-question-form').trigger('submit');
  assert.deepEqual(find('p-new').items.map((note) => note.kind), ['failed_approach', 'open_question']);
  assert.equal(el('project-finding-text').value, '');

  await ui.promoteProjectStep('s-one', 'p-old', { entry_id: 'e-3', summary: 'The gap scales as $1/L$.' });
  assert.equal(el('project-title').textContent, 'Spectral gap');
  assert.equal(el('project-findings-panel').hidden, false);
  el('project-promote-source').value = 'paper-1';
  await el('project-promote-source').trigger('change');
  el('project-promote-location').value = 'Theorem 2';
  el('project-promote-quote').value = 'Take the thermodynamic limit after rescaling.';
  await el('project-promote-form').trigger('submit');
  assert.deepEqual(find('p-old').session_ids, ['s-one']);
  assert.deepEqual(find('p-old').items, [{
    id: 'i-1', kind: 'finding', text: 'The gap scales as $1/L$.', session_id: 's-one', entry_id: 'e-3',
    source_id: 'paper-1', location: 'Theorem 2', quote: 'Take the thermodynamic limit after rescaling.',
  }]);
  assert.deepEqual(refreshed, ['s-one']);
  const [claim, citation] = el('project-finding-list').children[0].children.at(-1).children;
  await claim.trigger('click');
  assert.deepEqual(opened, [['s-one', 'e-3']]);
  await citation.trigger('click');
  assert.equal(el('project-sources-panel').hidden, false);
  assert.deepEqual(changes, ['paper-1']);
});
