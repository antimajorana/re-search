import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const assetsDir = new URL('../assets/', import.meta.url);
const require = createRequire(import.meta.url);
const katex = require(fileURLToPath(new URL('../vendor/katex/katex.min.js', import.meta.url)));

function createElement(tag = 'div') {
  const listeners = new Map();
  const attrs = new Map();
  let text = '';
  let html = '';
  return {
    tagName: String(tag).toUpperCase(),
    dataset: {},
    children: [],
    hidden: false,
    disabled: false,
    open: false,
    value: '',
    className: '',
    classList: { toggle() {} },
    get textContent() {
      if (this.children.length) return this.children.map(c => c.textContent ?? '').join('');
      return text;
    },
    set textContent(v) {
      this.children = [];
      text = String(v ?? '');
      html = '';
    },
    get innerHTML() {
      if (html) return html;
      if (this.children.length) return this.children.map(c => c.innerHTML || c.textContent || '').join('');
      return text;
    },
    set innerHTML(v) {
      this.children = [];
      html = String(v ?? '');
      text = html.replace(/<[^>]+>/g, '');
    },
    setAttribute(name, val) { attrs.set(name, String(val)); },
    getAttribute(name) { return attrs.get(name); },
    focus() { globalThis.document.activeElement = this; },
    addEventListener(name, fn) { listeners.set(name, fn); },
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
    reset() {},
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    querySelectorAll(sel) {
      const out = [];
      const match = node => {
        if (!node.tagName) return false;
        if (sel.startsWith('.')) return String(node.className || '').split(/\s+/).includes(sel.slice(1));
        return node.tagName === sel.toUpperCase();
      };
      const visit = node => {
        for (const child of node.children || []) {
          if (match(child)) out.push(child);
          visit(child);
        }
      };
      visit(this);
      return out;
    },
    async click() {
      return listeners.get('click')?.({ currentTarget: this, preventDefault() {} });
    },
  };
}

function createTextNode(value) {
  return { textContent: String(value ?? ''), innerHTML: String(value ?? ''), children: [] };
}

function dataUrl(code) {
  return 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
}

async function setupHarness(t, hooks = {}) {
  const ids = [
    'app-shell', 'investigation-map', 'investigation-conventions', 'new-investigation-btn',
    'show-paper-map', 'show-investigation-chat', 'investigation-file', 'investigation-existing',
    'investigation-original', 'investigation-setup', 'investigation-pages', 'investigation-panel',
    'investigation-nav', 'investigation-status', 'investigation-origin', 'investigation-question',
    'investigation-page-start', 'investigation-page-end', 'investigation-existing-wrap',
  ];
  const byId = new Map(ids.map(id => [id, createElement('div')]));
  byId.get('investigation-conventions').append(createElement('summary'));
  byId.get('investigation-file').files = [];
  byId.get('investigation-existing').options = [];
  byId.get('investigation-existing').add = opt => byId.get('investigation-existing').options.push(opt);
  byId.get('investigation-status').after = (...nodes) => {
    for (const node of nodes) { if (node?.id) byId.set(node.id, node); }
  };
  const submitBtn = createElement('button');
  byId.get('investigation-setup').append(submitBtn);
  byId.get('investigation-origin').hidden = true;

  const prev = { document: globalThis.document, window: globalThis.window, Option: globalThis.Option };
  globalThis.document = {
    getElementById: id => byId.get(id),
    createElement,
    createTextNode,
    addEventListener() {},
    activeElement: null,
  };
  globalThis.window = { katex };
  globalThis.Option = function Option(label, value) { return { label, value }; };
  t.after(() => Object.assign(globalThis, prev));

  const mathHref = new URL('math.js', assetsDir).href;
  const stateHref = new URL('investigation_state.js', assetsDir).href;
  let source = await readFile(new URL('investigation.js', assetsDir), 'utf8');
  const stubs = {
    'api.js': 'export const getSession = async () => ({}); export const findInvestigationConventions = async () => {};',
    'composer.js': 'export const arrayBufferToBase64 = () => ""; export const setComposerPlaceholder = () => {};',
    'investigation_map.js': `export const DEFAULT_PAPER_REQUEST = 'Identify'; export const createPaperMap = () => ({ reset() {}, update() {}, updateSummary() {}, renderAnchors() {}, openNode: (...a) => globalThis.__originMapOpen?.(...a) ?? true });`,
    'pdf_preview.js': 'export const openOriginalPdf = () => {};',
    'investigation_conventions.js': 'export const createConventionCatalog = () => ({ reset() {}, update() {} });',
    'investigation_answer.js': 'export const createAnswerCard = () => ({ reset() {}, update() {} });',
    'source_links.js': 'export const citedPaperLinks = () => [];',
  };
  source = source.replace("'/assets/math.js'", JSON.stringify(mathHref));
  source = source.replace("'/assets/investigation_state.js'", JSON.stringify(stateHref));
  for (const [name, stub] of Object.entries(stubs)) {
    source = source.replace(`'/assets/${name}'`, JSON.stringify(dataUrl(stub)));
  }

  const mod = await import(dataUrl(source));
  const calls = { opened: [], openedSources: [], usedQuestions: [], mapOpened: [] };
  globalThis.__originMapOpen = (seq, nodeId) => {
    calls.mapOpened.push([seq, nodeId]);
    return hooks.openNode ? hooks.openNode(seq, nodeId) : true;
  };
  t.after(() => { delete globalThis.__originMapOpen; });

  mod.initInvestigation({
    create: async () => {},
    open: async id => {
      calls.opened.push(id);
      if (hooks.onOpen) await hooks.onOpen(id, mod);
    },
    useQuestion: q => calls.usedQuestions.push(q),
    requestOptions: () => ({}),
    send: async () => {},
    openSource: async (id, anchor) => { calls.openedSources.push([id, anchor]); },
  });

  return { mod, originEl: byId.get('investigation-origin'), calls };
}

test('linked Research origin renders collapsed math conventions and opens exact passages or parent session', async t => {
  let allowParentSwitch = true;
  const parentData = { mode: 'investigation', investigation: { map_seq: 7 } };
  const { mod, originEl, calls } = await setupHarness(t, {
    onOpen: async (id, m) => {
      if (allowParentSwitch) m.showInvestigationSession(id, parentData);
    },
  });

  const metricAnchor = { source_id: 'src-1', path: 'paper.tex', label: 'Sec. 2', start_line: 14, end_line: 14, excerpt: 'We take (-,+,+,+).' };
  const diffAnchor = { source_id: 'src-1', path: 'app.tex', start_line: 20, end_line: 23, excerpt: 'Let d denote...' };
  const claimAnchor = { source_id: 'src-1', path: 'paper.tex', label: 'Theorem 1', start_line: 40, end_line: 45, excerpt: 'Main bound.' };

  mod.showInvestigationSession('s-child-1', {
    mode: 'research',
    investigation: {
      origin: {
        parent_session: 's-parent',
        node_id: 'thm-1',
        map_seq: 7,
        claim: 'Energy satisfies $E \\ge 0$.',
        question: 'Does positivity hold on $\\partial M$?',
        assumptions: ['Boundary terms vanish at $r \\to \\infty$.'],
        paper_conventions: [
          { id: 'c1', category: 'sign', label: '$\\eta_{\\mu\\nu}$', meaning: 'Metric signature $(-,+,+,+)$', scope: 'Bulk $\\mathcal{M}$', anchors: [metricAnchor] },
          { id: 'c2', category: 'notation', label: '\\mathrm{d}', meaning: 'Exterior derivative', scope: 'Forms', anchors: [diffAnchor] },
          { id: 'c3', category: 'units', label: '$c = 1$', meaning: 'Natural units', scope: 'All sections', anchors: [] },
        ],
        sources: [claimAnchor],
      },
    },
  });

  assert.equal(originEl.hidden, false);
  const outer = originEl.children[0];
  assert.equal(outer.tagName, 'DETAILS');
  assert.equal(outer.open, false, 'outer origin disclosure starts collapsed');
  assert.equal(outer.children[0].textContent, 'From a paper investigation');

  const subDetails = outer.children.filter(c => c.tagName === 'DETAILS');
  assert.equal(subDetails.length, 2, 'has collapsed Paper conventions and Source passages subsections');
  const [convDetails, sourceDetails] = subDetails;
  assert.equal(convDetails.open, false, 'Paper conventions subsection starts collapsed');
  assert.equal(convDetails.children[0].textContent, 'Paper conventions (3)');
  assert.equal(sourceDetails.children[0].textContent, 'Source passages (1)');

  const items = convDetails.querySelector('ul').children;
  assert.equal(items.length, 3);
  assert.match(items[0].querySelector('strong').innerHTML, /katex/, 'math label rendered with KaTeX');
  assert.match(items[0].querySelector('span').innerHTML, /katex/, 'math meaning rendered with KaTeX');
  assert.match(items[0].querySelector('small').innerHTML, /Scope · .*katex/, 'math scope rendered with KaTeX');
  assert.match(items[1].querySelector('strong').innerHTML, /katex/, 'bare TeX notation label wrapped and rendered with KaTeX');

  const passageBtns0 = items[0].querySelectorAll('button');
  assert.equal(passageBtns0.length, 1);
  assert.equal(passageBtns0[0].textContent, 'Sec. 2 · line 14');
  assert.equal(items[1].querySelector('button').textContent, 'app.tex · lines 20–23');
  assert.equal(items[2].querySelectorAll('button').length, 0, 'anchorless convention relies on disclosure parent button');

  allowParentSwitch = false;
  await passageBtns0[0].click();
  assert.deepEqual(calls.opened, ['s-parent']);
  assert.equal(calls.openedSources.length, 0, 'does not open source if parent switch was aborted');

  allowParentSwitch = true;
  await passageBtns0[0].click();
  assert.deepEqual(calls.openedSources, [['s-parent', metricAnchor]]);

  // Switch back to child and test parent session button from the same disclosure
  mod.showInvestigationSession('s-child-1', {
    mode: 'research',
    investigation: {
      origin: {
        parent_session: 's-parent',
        node_id: 'thm-1',
        map_seq: 7,
        claim: 'Energy satisfies $E \\ge 0$.',
        question: 'Does positivity hold on $\\partial M$?',
        paper_conventions: [{ id: 'c3', category: 'units', label: '$c = 1$', meaning: 'Natural units', scope: 'Global', anchors: [] }],
      },
    },
  });
  const topButtons = originEl.children[0].children.filter(c => c.tagName === 'BUTTON');
  assert.deepEqual(topButtons.map(b => b.textContent), ['Use research question', 'Open original claim']);
  await topButtons[0].click();
  assert.deepEqual(calls.usedQuestions, ['Does positivity hold on $\\partial M$?']);
  await topButtons[1].click();
  assert.deepEqual(calls.mapOpened, [[7, 'thm-1']]);
});

test('catalog boundaries (0, 8, >8) and switching between two linked sessions from the same parent', async t => {
  const { mod, originEl, calls } = await setupHarness(t, {
    onOpen: (id, m) => m.showInvestigationSession(id, { mode: 'investigation', investigation: {} }),
  });
  const makeConventions = (count, prefix) => Array.from({ length: count }, (_, i) => ({
    id: `${prefix}-${i + 1}`,
    category: 'notation',
    label: `$x_{${i + 1}}$`,
    meaning: `${prefix} coordinate ${i + 1}`,
    scope: `Sec. ${i + 1}`,
    anchors: [{ source_id: 'src-1', path: 'p.tex', start_line: i + 1, end_line: i + 1 }],
  }));

  // 0 conventions: no Paper conventions subsection
  mod.showInvestigationSession('s-linked-empty', {
    mode: 'research',
    investigation: { origin: { parent_session: 's-same-parent', claim: 'Empty catalog claim', paper_conventions: [] } },
  });
  assert.equal(originEl.children[0].querySelectorAll('details').length, 0);
  assert.equal(originEl.children[0].querySelector('button').textContent, 'Open original investigation');

  // Exact upper bound: 8 conventions -> all 8 shown, no overflow notice
  const originEight = {
    parent_session: 's-same-parent',
    node_id: 'claim-a',
    map_seq: 3,
    claim: 'First linked claim $A$',
    question: 'Question A?',
    paper_conventions: makeConventions(8, 'Alpha'),
  };
  mod.showInvestigationSession('s-linked-a', { mode: 'research', investigation: { origin: originEight } });
  let conv = originEl.children[0].querySelector('details');
  assert.equal(conv.children[0].textContent, 'Paper conventions (8)');
  assert.equal(conv.querySelector('ul').children.length, 8);
  assert.equal(conv.querySelector('.investigation-meta'), null);

  // Preserve open state on identical refresh of the same session
  originEl.children[0].open = true;
  conv.open = true;
  mod.showInvestigationSession('s-linked-a', { mode: 'research', investigation: { origin: structuredClone(originEight) } });
  assert.equal(originEl.children[0].open, true);
  assert.equal(originEl.children[0].querySelector('details').open, true);

  // Switch to a SECOND linked session from the SAME parent with 11 conventions (>8 bound)
  const originEleven = {
    parent_session: 's-same-parent',
    node_id: 'claim-b',
    map_seq: 3,
    claim: 'Second linked claim $B$',
    question: 'Question B?',
    paper_conventions: makeConventions(11, 'Beta'),
  };
  mod.showInvestigationSession('s-linked-b', { mode: 'research', investigation: { origin: originEleven } });
  assert.match(originEl.children[0].children[1].textContent, /Second linked claim/);
  conv = originEl.children[0].querySelector('details');
  assert.equal(conv.open, false);
  assert.equal(conv.children[0].textContent, 'Paper conventions (11)');
  assert.equal(conv.querySelector('ul').children.length, 8);
  assert.match(conv.querySelector('ul').children[0].textContent, /Beta coordinate 1/);
  assert.equal(
    conv.querySelector('.investigation-meta')?.textContent,
    'Showing 8 of 11 conventions.',
  );
  const topButtons = originEl.children[0].children.filter(c => c.tagName === 'BUTTON');
  assert.deepEqual(topButtons.map(b => b.textContent), ['Use research question', 'Open original claim']);
  await topButtons[0].click();
  assert.deepEqual(calls.usedQuestions, ['Question B?']);
  const fullCatalog = conv.querySelectorAll('button').find(b => b.textContent === 'Open full catalog in Investigation');
  await fullCatalog.click();
  const catalog = document.getElementById('investigation-conventions');
  assert.equal(catalog.open, true);
  assert.equal(document.activeElement, catalog.querySelector('summary'));
  assert.deepEqual(calls.opened, ['s-same-parent']);
  mod.showInvestigationSession('s-linked-b', { mode: 'research', investigation: { origin: originEleven } });

  // Same-parent update without sessionId reset (renderOrigin's own signature cache) + HTML safety
  const originEscaped = {
    ...originEight,
    claim: 'Updated same-parent claim $C$',
    paper_conventions: [{
      id: 'xss',
      category: 'other',
      label: '<img src=x onerror=1> $\\alpha$',
      meaning: 'Safe <b>meaning</b> $\\beta$',
      scope: 'Safe <script>1</script> $\\gamma$',
      anchors: [{ source_id: 'src-1', path: 'p.tex', start_line: 9 }],
    }],
  };
  mod.showInvestigationSession('s-linked-b', { mode: 'research', investigation: { origin: originEscaped } });
  assert.match(originEl.children[0].children[1].textContent, /Updated same-parent claim/);
  const escapedRow = originEl.children[0].querySelector('details').querySelector('ul').children[0];
  assert.doesNotMatch(escapedRow.innerHTML, /<img|<script|<b>/);
  assert.equal(escapedRow.querySelector('button').textContent, 'p.tex · line 9');
});
