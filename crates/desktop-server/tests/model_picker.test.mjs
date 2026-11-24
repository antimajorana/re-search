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
