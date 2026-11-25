import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');
const require = createRequire(import.meta.url);
const katex = require(path.join(crateRoot, 'vendor', 'katex', 'katex.min.js'));

const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
const mathMod = await import(mathUrl);
const stateMod = await import(pathToFileURL(path.join(assetsDir, 'investigation_state.js')).href);

const srcLinksCode = fs
  .readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
  .replace("'/assets/math.js'", JSON.stringify(mathUrl));
const srcLinksMod = await import(
  `data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`
);

test('Paper view math renderer renders single-dollar $...$ inline math despite INVESTIGATION_SYSTEM_PROMPT claiming it does not', () => {
  const rustPrompt = fs.readFileSync(
    path.join(crateRoot, 'src', 'investigation_system_prompt.rs'),
    'utf8'
  );
  const inlineHtml = mathMod.mathTextToHtml('In Section 2, $\\eta_\\mu$ denotes the normal.', { katex });
  const richHtml = mathMod.markdownToRenderedHtml('The Hamiltonian $H(k) = d(k)\\cdot\\sigma$ is gapped.', { katex });

  assert.ok(inlineHtml.includes('class="katex"'), `mathTextToHtml should render $...$ via KaTeX: ${inlineHtml}`);
  assert.ok(richHtml.includes('class="katex"'), `markdownToRenderedHtml should render $...$ via KaTeX: ${richHtml}`);

  assert.ok(
    !rustPrompt.includes('single dollar delimiters do not render in the Paper view'),
    'INVESTIGATION_SYSTEM_PROMPT falsely claims single-dollar delimiters do not render in Paper view'
  );
});

test('Settings UI #paper-readers-enabled represents 3-state Option<bool> including explicit false override', async () => {
  const puts = [];
  const apiShim = `
    export async function listSubagents() {
      return {
        builtin: [{ role_key: 'paper_reader', name: 'Paper Reader', summary: 'Reads slices.' }],
        custom: [],
      };
    }
    export async function getProviderSettings() {
      return {
        agent_models: {},
        paper_readers_enabled: false,
        paper_reader_max_parallel: 3,
      };
    }
    export async function getModels() { return { providers: [] }; }
    export async function updateProviderSettings(body) {
      globalThis.__auditPuts.push(body);
      return body;
    }
  `;
  globalThis.__auditPuts = puts;
  const apiUrl = `data:text/javascript;base64,${Buffer.from(apiShim, 'utf8').toString('base64')}`;
  const subCode = fs
    .readFileSync(path.join(assetsDir, 'subagents.js'), 'utf8')
    .replace("'/assets/api.js'", JSON.stringify(apiUrl))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const subMod = await import(
    `data:text/javascript;base64,${Buffer.from(subCode, 'utf8').toString('base64')}`
  );

  const createdById = new Map();
  function makeEl(tag = 'div') {
    const listeners = {};
    let idVal = '';
    return {
      tagName: String(tag).toUpperCase(),
      get id() { return idVal; },
      set id(v) { idVal = String(v); if (idVal) createdById.set(idVal, this); },
      value: '',
      checked: false,
      disabled: false,
      hidden: false,
      dataset: {},
      style: {},
      className: '',
      innerHTML: '',
      textContent: '',
      children: [],
      setAttribute() {},
      append(...nodes) { this.children.push(...nodes); },
      appendChild(c) { this.children.push(c); return c; },
      replaceChildren(...nodes) { this.children = [...nodes]; },
      querySelector() { return null; },
      querySelectorAll() { return []; },
      addEventListener(ev, fn) { listeners[ev] = fn; },
      async trigger(ev) { await listeners[ev]?.({ preventDefault() {} }); },
    };
  }
  const builtinBox = makeEl('div');
  builtinBox.id = 'builtin-agents-list';
  const prevDoc = globalThis.document;
  const prevOpt = globalThis.Option;
  try {
    globalThis.Option = function Option(text, val) {
      const o = makeEl('option');
      o.textContent = text;
      o.value = val;
      return o;
    };
    globalThis.document = {
      getElementById: id => createdById.get(id) || null,
      createElement: tag => makeEl(tag),
      createTextNode: text => ({ nodeType: 3, textContent: String(text) }),
    };
    await subMod.loadSubagents();

    const ctrl = createdById.get('paper-readers-enabled');
    assert.ok(ctrl, 'expected #paper-readers-enabled control in builtin-agents-list');
    ctrl.checked = false;
    await ctrl.trigger('change');
    const sent = puts.at(-1);
    assert.equal(
      sent?.paper_readers_enabled,
      false,
      `UI checkbox cannot represent or save paper_readers_enabled=false (sent ${JSON.stringify(sent?.paper_readers_enabled)} instead)`
    );
  } finally {
    globalThis.document = prevDoc;
    globalThis.Option = prevOpt;
    delete globalThis.__auditPuts;
  }
});

test('paperViewState shows Stopped status when a conventions or branch turn is stopped on a mapped paper', () => {
  const inv = {
    source_id: 's1',
    map_seq: 2,
    mapping_status: 'mapped',
    map: { nodes: [{ id: 'n1' }], edges: [] },
    answers: [],
  };
  const session = {
    running: false,
    events: [
      { seq: 1, kind: 'investigation.mapping_requested', payload: { source_id: 's1' } },
      { seq: 2, kind: 'investigation.map', payload: { nodes: [{ id: 'n1' }], edges: [] } },
      { seq: 3, kind: 'session.end', payload: { outcome: 'completed' } },
      { seq: 4, kind: 'investigation.conventions_resumed', payload: { map_seq: 2, scope: 'paper' } },
      { seq: 5, kind: 'session.abort', payload: { reason: 'stopped by user', stopped: true } },
    ],
  };
  const view = stateMod.paperViewState(inv, session);
  assert.match(
    view.statusText,
    /Stopped/i,
    `Stopping a conventions turn after a map exists loses the Stopped status message: ${JSON.stringify(view)}`
  );
});

test('sessionSourcesUsed counts papers cited via [path:Lstart-end] in question-first investigation answers', () => {
  const session = {
    events: [
      { seq: 1, kind: 'session.start', payload: { prompt: 'What vanishes?' } },
      {
        seq: 2,
        kind: 'source.ingested',
        payload: {
          source_id: 'src-1',
          title: 'Notes',
          primary_path: 'src-1/notes.txt',
          files: ['src-1/notes.txt'],
        },
      },
      { seq: 3, kind: 'investigation.paper', payload: { source_id: 'src-1' } },
      { seq: 4, kind: 'investigation.question_requested', payload: { source_id: 'src-1', question: 'What vanishes?' } },
      { seq: 5, kind: 'model.response', payload: {} },
      { seq: 6, kind: 'model.text', payload: { text: 'The endpoint variation vanishes [L2].' } },
      {
        seq: 7,
        kind: 'investigation.answer',
        payload: {
          request_seq: 4,
          source_id: 'src-1',
          question: 'What vanishes?',
          answer: 'The endpoint variation vanishes [L2].',
          cites: [{ source_id: 'src-1', path: 'src-1/notes.txt', start: 2, end: 2, read: true }],
        },
      },
      { seq: 8, kind: 'session.end', payload: { outcome: 'completed' } },
    ],
    sources: [
      {
        source_id: 'src-1',
        title: 'Notes',
        primary_path: 'src-1/notes.txt',
        files: ['src-1/notes.txt'],
      },
    ],
    investigation: {
      source_id: 'src-1',
      answers: [
        {
          request_seq: 4,
          answer: 'The endpoint variation vanishes [L2].',
          cites: [{ source_id: 'src-1', path: 'src-1/notes.txt', start: 2, end: 2, read: true }],
        },
      ],
    },
  };

  const used = srcLinksMod.sessionSourcesUsed(session);
  assert.equal(
    used.length,
    1,
    'Question-first Investigation answer citations do not mark the primary paper as used in Sources & Literature'
  );
});
