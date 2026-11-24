import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

class Node {
  children = [];
  events = {};
  textContent = '';
  value = '';
  disabled = false;

  constructor(tag) { this.tag = tag; }
  addEventListener(type, listener) { this.events[type] = listener; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  set innerHTML(_value) { assert.fail('Lean source must be rendered as text'); }
}

test('Formal Library requires a workspace and labels the exact declaration source', async () => {
  const elements = new Map();
  const form = new Node('form');
  const button = new Node('button');
  form.querySelector = () => button;
  elements.set('formal-library-form', form);
  for (const id of ['query', 'status', 'results']) {
    elements.set('formal-library-' + id, new Node(id));
  }
  const query = elements.get('formal-library-query');
  const status = elements.get('formal-library-status');
  const results = elements.get('formal-library-results');
  globalThis.document = {
    getElementById: id => elements.get(id),
    createElement: tag => new Node(tag),
  };

  const requests = [];
  globalThis.formalLibraryApi = (id, args) =>
    new Promise(resolve => requests.push({ id, args, resolve }));
  const url = code => 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
  const api = url('export const searchFormalLibrary = globalThis.formalLibraryApi;');
  const source = readFileSync(new URL('../assets/formal_library.js', import.meta.url), 'utf8')
    .replace("'/assets/api.js'", JSON.stringify(api));
  const { initFormalLibrary, setFormalLibraryWorkspace } = await import(url(source));

  initFormalLibrary();
  assert.equal(query.disabled, true);
  assert.match(status.textContent, /Attach a Lean workspace/);
  setFormalLibraryWorkspace('previous', '/physics/workspace');
  assert.equal(query.disabled, false);

  query.value = 'Boost';
  const first = form.events.submit({ preventDefault() {} });
  assert.equal(button.disabled, true);
  assert.equal(requests[0].id, 'previous');

  setFormalLibraryWorkspace('active', '/physics/workspace');
  assert.equal(button.disabled, false);
  assert.equal(query.value, '');
  requests[0].resolve({ results: [{ name: 'Old declaration' }] });
  await first;
  assert.deepEqual(results.children, []);

  query.value = '  Boost  ';
  const second = form.events.submit({ preventDefault() {} });
  assert.equal(requests[1].id, 'active');
  assert.deepEqual(requests[1].args, { query: 'Boost' });
  const signature = 'theorem Boost (x : ℝ) : x < x';
  requests[1].resolve({
    status: 'lexical_unverified',
    results: [{
      qualified_name: 'Relativity.Boost', name: 'Boost', signature,
      path: 'Physlib/Relativity.lean', line: 4, module: 'Physlib.Relativity',
    }],
    scan_truncated: true,
  });
  await second;
  assert.match(status.textContent, /Some sources could not be searched/);
  const card = results.children[0];
  assert.ok(card.children.some(node => node.textContent === 'Declaration found'));
  assert.ok(card.children.some(node => node.textContent.includes('Physlib/Relativity.lean:4')));
  assert.equal(card.children.find(node => node.tag === 'pre').textContent, signature);
  assert.ok(card.children.some(node => node.textContent === 'Copy Lean name'));
});
