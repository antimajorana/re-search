import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

test('a delayed startup discovery cannot erase a model loaded by a newer refresh', async (t) => {
  const source = await readFile(new URL('../assets/model_picker.js', import.meta.url), 'utf8');
  const oldWindow = globalThis.window;
  const pending = [];
  globalThis.window = { localStorage: { getItem() { return null; }, setItem() {} } };
  globalThis.modelDiscoveryFixture = () => new Promise(resolve => pending.push(resolve));
  t.after(() => { globalThis.window = oldWindow; delete globalThis.modelDiscoveryFixture; });
  const code = source.replace("import * as api from '/assets/api.js';", 'const api = {getModels: globalThis.modelDiscoveryFixture};');
  const { loadModels, getModelChoice } = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  const startup = loadModels(false);
  const refreshed = loadModels(true);
  pending[1]({ providers:[{ id:'antigravity', models:[{id:'gemini-carbon'}] }] });
  await refreshed;
  pending[0]({ providers:[] });
  await startup;
  assert.deepEqual(getModelChoice(), { provider:'antigravity', model:'gemini-carbon' });
});

test('model_picker renders Antigravity (Google) label and clean session note', async (t) => {
  const source = await readFile(new URL('../assets/model_picker.js', import.meta.url), 'utf8');
  const oldWindow = globalThis.window;
  const oldDocument = globalThis.document;
  const oldOption = globalThis.Option;
  const oldSetInterval = globalThis.setInterval;
  const makeNode = (tag) => {
    const classes = new Set();
    return {
      tagName: tag.toUpperCase(),
      children: [],
      hidden: false,
      disabled: false,
      label: '',
      value: '',
      textContent: '',
      classList: { toggle: (c, f) => (f ? classes.add(c) : classes.delete(c)) },
      addEventListener() {},
      append(...nodes) { this.children.push(...nodes); },
      replaceChildren(...nodes) { this.children = nodes; },
    };
  };
  const selectEl = makeNode('select');
  const noteEl = makeNode('span');
  globalThis.window = {
    localStorage: { getItem() { return null; }, setItem() {} },
    addEventListener() {},
  };
  globalThis.document = {
    hidden: false,
    getElementById: (id) => (id === 'composer-model' ? selectEl : id === 'composer-route-note' ? noteEl : null),
    createElement: makeNode,
    addEventListener() {},
    dispatchEvent() {},
  };
  globalThis.Option = class MockOption {
    constructor(text, value) { this.text = text; this.value = value; this.disabled = false; this.selected = false; }
  };
  globalThis.setInterval = () => ({ unref() {} });
  globalThis.modelPickerLabelFixture = async () => ({
    providers: [{
      id: 'antigravity',
      label: 'Antigravity (Google)',
      route: 'antigravity',
      session_note: 'Antigravity CLI',
      models: [{ id: 'gemini-3.8-flash-medium', label: 'Gemini 3.8 Flash (Medium)', default: true }],
    }],
  });
  t.after(() => {
    globalThis.window = oldWindow;
    globalThis.document = oldDocument;
    globalThis.Option = oldOption;
    globalThis.setInterval = oldSetInterval;
    delete globalThis.modelPickerLabelFixture;
  });
  const code = source.replace("import * as api from '/assets/api.js';", 'const api = { getModels: globalThis.modelPickerLabelFixture };');
  const picker = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
  picker.initModelPicker();
  await picker.loadModels(true);
  assert.equal(selectEl.children[0].label, 'Antigravity (Google)');
  assert.equal(noteEl.textContent, 'Antigravity CLI');
  picker.applySession('s-agy', { route: 'antigravity', route_label: 'Antigravity (Google)', events: [{ kind: 'session.start' }], messages: [] });
  assert.equal(noteEl.textContent, 'Antigravity CLI');
});
