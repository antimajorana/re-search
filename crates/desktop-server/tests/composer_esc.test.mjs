import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const assetsDir = new URL('../assets/', import.meta.url);

function dataUrl(code) {
  return 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
}

function makeEl(id, props = {}) {
  const listeners = new Map();
  return {
    id,
    hidden: false,
    disabled: false,
    open: false,
    value: '',
    placeholder: props.placeholder || '',
    innerHTML: props.innerHTML || '',
    textContent: '',
    style: {},
    classList: { add() {}, remove() {} },
    setAttribute() {},
    getAttribute() { return null; },
    querySelector() { return null; },
    querySelectorAll() { return []; },
    addEventListener(name, fn) { listeners.set(name, fn); },
    focus() {},
    ...props,
  };
}

async function loadComposer(t) {
  const docListeners = new Map();
  const byId = new Map([
    ['composer-pill', makeEl('composer-pill')],
    ['composer-input', makeEl('composer-input', { placeholder: 'Default placeholder' })],
    ['composer-send', makeEl('composer-send', { innerHTML: 'Send' })],
    ['composer-stop', makeEl('composer-stop', { hidden: true })],
    ['topbar-stop-btn', makeEl('topbar-stop-btn', { hidden: true })],
    ['effort-popover', makeEl('effort-popover', { hidden: true })],
    ['project-overview', makeEl('project-overview', { hidden: true })],
    ['conventions-popover', makeEl('conventions-popover', { hidden: true })],
  ]);
  let openDialog = null;

  const prev = { document: globalThis.document };
  globalThis.document = {
    getElementById: id => byId.get(id) || null,
    querySelector: sel => (sel === 'dialog[open]' ? openDialog : null),
    addEventListener(name, fn) { docListeners.set(name, fn); },
  };
  t.after(() => Object.assign(globalThis, prev));

  let source = await readFile(new URL('composer.js', assetsDir), 'utf8');
  const stubs = {
    'api.js': 'export const pickWorkspaceFolder = async () => ({ supported: false }); export const uploadFilesBase64 = async () => ({});',
    'math.js': 'export const escapeHtml = s => String(s);',
    'effort_picker.js': 'export const closeEffortWindow = () => {}; export const getReasoningEffort = () => "default";',
    'subagents.js': 'export const getActiveSubagent = () => null;',
  };
  for (const [name, stub] of Object.entries(stubs)) {
    source = source.replace(`'/assets/${name}'`, JSON.stringify(dataUrl(stub)));
  }

  const mod = await import(dataUrl(source));
  let stopCalls = 0;
  mod.initComposer({
    onStopTurn: async () => { stopCalls += 1; return true; },
  });
  mod.setComposerBusy(true);

  const fireEsc = target => {
    let prevented = false;
    docListeners.get('keydown')?.({
      key: 'Escape',
      defaultPrevented: false,
      repeat: false,
      isComposing: false,
      keyCode: 27,
      target,
      preventDefault() { prevented = true; },
    });
    return prevented;
  };

  return {
    mod,
    byId,
    fireEsc,
    setOpenDialog: d => { openDialog = d; },
    getStopCalls: () => stopCalls,
  };
}

test('Esc in #investigation-panel input, textarea, or select does not stop a running turn', async t => {
  const { fireEsc, getStopCalls } = await loadComposer(t);
  for (const tag of ['INPUT', 'TEXTAREA', 'SELECT']) {
    const target = {
      tagName: tag,
      closest: sel => (sel === '#investigation-panel :is(input,textarea,select)' ? target : null),
    };
    assert.equal(fireEsc(target), false, `Esc in ${tag} must not preventDefault`);
  }
  assert.equal(getStopCalls(), 0);
});

test('Esc while any dialog[open] is present does not stop a running turn', async t => {
  const { fireEsc, setOpenDialog, getStopCalls } = await loadComposer(t);
  setOpenDialog({ tagName: 'DIALOG', open: true });
  const composerTarget = { tagName: 'TEXTAREA', closest: () => null };
  assert.equal(fireEsc(composerTarget), false);
  assert.equal(getStopCalls(), 0);
});

test('Esc in the main composer while running calls stop and placeholder override works', async t => {
  const { mod, byId, fireEsc, getStopCalls } = await loadComposer(t);
  const composerTarget = { tagName: 'TEXTAREA', closest: () => null };
  assert.equal(fireEsc(composerTarget), true);
  assert.equal(getStopCalls(), 1);

  mod.setComposerBusy(false);
  mod.setComposerPlaceholder('Ask a follow-up about this paper…');
  assert.equal(byId.get('composer-input').placeholder, 'Ask a follow-up about this paper…');
  mod.setComposerPlaceholder(null);
  assert.equal(byId.get('composer-input').placeholder, 'Default placeholder');
});
