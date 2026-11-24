import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

test('Stage 16 Sources title & deduplicated Origin links + Proof DAG controls & CSS safety', async () => {
  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const srcLinksCode = fs
    .readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  const srcLinksUrl = `data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`;
  const sl = await import(srcLinksUrl);

  const src = {
    source_id: 'arxiv_2301_12345v1-abcd1234',
    title: 'Two-Dimensional Lieb-Schultz-Mattis Obstructions',
    origin: 'https://export.arxiv.org/e-print/2301.12345v1',
    primary_path: 'arxiv_2301_12345v1-abcd1234/main.tex',
  };
  assert.equal(sl.resolveSourceTitle(src), 'Two-Dimensional Lieb-Schultz-Mattis Obstructions');
  const originInfo = sl.resolveOriginInfo(src);
  assert.equal(originInfo.url, 'https://arxiv.org/abs/2301.12345v1');
  const extra = sl.linksForIngestedSource(src).filter((lk) => lk.url !== originInfo.url);
  assert.equal(extra.some((lk) => lk.url === originInfo.url), false, 'Origin link must not repeat in chips');
  assert.equal(extra.some((lk) => lk.label === 'PDF'), true, 'Secondary PDF link preserved');
  assert.equal(extra.find((lk) => lk.label === 'PDF').url, 'https://arxiv.org/pdf/2301.12345v1');
  const versions = sl.linksForPaper({
    url: 'https://arxiv.org/abs/hep-th/9901001', base_id: 'hep-th/9901001', id: 'hep-th/9901001v2',
  });
  assert.deepEqual(versions.map((link) => link.url), [
    'https://arxiv.org/abs/hep-th/9901001v2', 'https://arxiv.org/pdf/hep-th/9901001v2',
  ]);

  // Legacy/untitled source must fall back to clean filename, never raw URL.
  const untitled = {
    source_id: 'paper-1234abcd',
    origin: 'https://export.arxiv.org/e-print/2402.00001v1',
    primary_path: 'paper-1234abcd/article.tex',
  };
  assert.equal(sl.resolveSourceTitle(untitled), 'article.tex');

  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  const dagCss = fs.readFileSync(path.join(assetsDir, 'dag.css'), 'utf8');
  for (const id of ['dag-viewport', 'dag-zoom-out', 'dag-zoom-reset', 'dag-zoom-in', 'dag-zoom-fit']) {
    assert.match(indexHtml, new RegExp(`id="${id}"`), `Missing #${id}`);
  }
  for (const asset of ['/assets/dag.css', '/assets/canvas_dag_detail.js']) {
    assert.ok(assetsRs.includes(asset), `Missing ${asset} in assets.rs`);
  }
  assert.equal(/\.(?:dag-eq-box|dag-explanation-box|verdict-recovery-body|rich-prose)\s+(?:span|div|\*)\b/.test(dagCss), false);
});

test('Project library normalizes citations and finds excerpts across session sources', async () => {
  const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
  const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
  const linksCode = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const libraryCode = fs.readFileSync(path.join(assetsDir, 'project_library.js'), 'utf8')
    .replace("'/assets/api.js'", JSON.stringify(moduleUrl('export {};')))
    .replace("'/assets/project_edits.js'", JSON.stringify(pathToFileURL(path.join(assetsDir, 'project_edits.js')).href))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/source_links.js'", JSON.stringify(moduleUrl(linksCode)));
  const { normalizeProjectReference, filterProjectSources } = await import(moduleUrl(libraryCode));
  assert.deepEqual(normalizeProjectReference('arXiv:hep-th/9901001v2'), {
    url: 'https://arxiv.org/abs/hep-th/9901001v2',
    doi: '',
  });
  assert.deepEqual(normalizeProjectReference(' DOI: 10.1103/PhysRev.47.777 '), {
    url: 'https://doi.org/10.1103/PhysRev.47.777',
    doi: '10.1103/PhysRev.47.777',
  });
  assert.deepEqual(normalizeProjectReference('http://dx.doi.org/10.1234/foo%23bar'), {
    url: 'https://doi.org/10.1234/foo%23bar',
    doi: '10.1234/foo#bar',
  });
  assert.equal(normalizeProjectReference('garbage 2501.00001'), null);
  assert.equal(normalizeProjectReference('javascript:alert(1)'), null);
  assert.equal(normalizeProjectReference('https://arxiv.org@example.com/abs/2501.00001'), null);
  assert.deepEqual(normalizeProjectReference('https://example.org/papers/topology'), {
    url: 'https://example.org/papers/topology',
    doi: '',
  });

  const first = {
    title: 'Boundary spectrum', session_id: 'session-one', location: 'Eq. (12)',
    quote: 'The Berry connection is locally flat.', url: 'https://arxiv.org/abs/2501.00001',
  };
  const second = {
    title: 'Berry curvature', session_id: 'session-two', notes: 'Use the opposite orientation.',
    bibtex: '@article{complement, title={Berry curvature}}',
  };
  assert.deepEqual(filterProjectSources([first, second], 'BERRY'), [first, second]);
  assert.deepEqual(filterProjectSources([first, second], 'berry eq. (12)'), [first]);
  assert.deepEqual(filterProjectSources([first, second], 'arXiv:2501.00001'), [first]);
  assert.deepEqual(filterProjectSources([first, second], 'session-two complement'), [second]);
  assert.deepEqual(filterProjectSources([first, second], 'missing'), []);
});

test('Sources passage search shows exact provenance and discards replies from a previous session', async (t) => {
  const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
  const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
  const linksCode = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const assignments = {};
  const createElement = (tag) => ({
    tag, children: [], textContent: '', listeners: {},
    addEventListener(type, listener) { this.listeners[type] = listener; },
    append(...children) { this.children.push(...children); },
    appendChild(child) { this.children.push(child); return child; },
    replaceChildren(...children) { this.children = children; this.textContent = ''; },
    querySelector() { return null; },
    getBoundingClientRect() { return { top: 0 }; },
  });
  const elements = Object.fromEntries(['form', 'query', 'status', 'results'].map((name) => [
    `paper-passage-${name}`, createElement(name),
  ]));
  elements['source-reader-dialog'] = {
    ...createElement('dialog'), open: false,
    showModal() { this.open = true; },
    close() { this.open = false; this.listeners.close?.(); },
  };
  elements['source-reader-body'] = { ...createElement('section'), clientHeight: 100 };
  elements['source-reader-status'] = createElement('p');
  elements['literature-search-form'] = createElement('form');
  elements['lit-query-input'] = createElement('input');
  elements['literature-results'] = createElement('section');
  const previousDocument = globalThis.document;
  const previousNavigator = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  t.after(() => {
    globalThis.document = previousDocument;
    if (previousNavigator) Object.defineProperty(globalThis, 'navigator', previousNavigator);
    else delete globalThis.navigator;
    delete globalThis.paperPassageMock;
  });
  globalThis.document = { createElement, getElementById: (id) => elements[id] || null, querySelector: () => null };
  const read = [];
  const copied = [];
  let fileDigest = 'b'.repeat(64);
  const literature = new Map();
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: { clipboard: { writeText: async (text) => copied.push(text) } },
  });
  globalThis.paperPassageMock = {
    searchPaperPassages(session, query) {
      assignments[session] = { query };
      return new Promise((resolve) => { assignments[session].resolve = resolve; });
    },
    async getSession() {
      return { sources: [{ source_id: 'entropy', primary_path: 'entropy/main.tex', files: ['entropy/appendix.tex'] }] };
    },
    async readSourceFile(session, file) {
      read.push([session, file]);
      return { content: 'One\nTwo\nThree\nFour\nFive', file_sha256: fileDigest };
    },
    searchLiterature(_session, { query }) {
      return new Promise((resolve) => literature.set(query, resolve));
    },
  };
  const code = fs.readFileSync(path.join(assetsDir, 'canvas_sources.js'), 'utf8')
    .replace("'/assets/api.js'", JSON.stringify(moduleUrl('export const { searchPaperPassages, searchLiterature, getSession, readSourceFile } = globalThis.paperPassageMock;')))
    .replace("'/assets/math.js'", JSON.stringify(mathUrl))
    .replace("'/assets/model_picker.js'", JSON.stringify(moduleUrl('export function getModelChoice() { return null; }')))
    .replace("'/assets/source_links.js'", JSON.stringify(moduleUrl(linksCode)));
  const { initSourcesAndSidetalk, refreshSourcesAndSidetalk } = await import(moduleUrl(code));
  initSourcesAndSidetalk();
  await refreshSourcesAndSidetalk('first');
  const submit = elements['paper-passage-form'].listeners.submit;
  elements['paper-passage-query'].value = 'Discard this search';
  const oldSearch = submit({ preventDefault() {} });
  await refreshSourcesAndSidetalk('current');
  assert.equal(elements['paper-passage-query'].value, '');
  elements['paper-passage-query'].value = ' Spectral entropy ';
  const activeSearch = submit({ preventDefault() {} });
  assert.equal(assignments.current.query, 'Spectral entropy');
  const excerpt = '<article>Entropy at α = 2</article>\r\n';
  const receipt = {
    passages: [{
      source_id: 'entropy', title: 'Entropy note', path: 'entropy/appendix.tex', source_seq: 7,
      start_line: 3, end_line: 4, start_byte: 28, end_byte: 67, excerpt,
      source_sha256: 'a'.repeat(64), file_sha256: 'b'.repeat(64),
    }],
    coverage: { files_searched: 1, matching_passages: 1, omissions: { hash_mismatch: 1 } },
  };
  assignments.current.resolve(receipt);
  await activeSearch;
  assignments.first.resolve({ passages: [] });
  await oldSearch;
  const [hit] = elements['paper-passage-results'].children;
  assert.equal(hit.children[1].textContent, 'entropy/appendix.tex:L3–L4');
  assert.equal(hit.children[2].tag, 'pre');
  assert.equal(hit.children[2].textContent, excerpt);
  assert.match(hit.children[3].children[1].textContent, /b{64}$/);
  assert.match(elements['paper-passage-status'].textContent, /changed content/);
  const [open, copy] = hit.children[4].children;
  await open.listeners.click();
  assert.deepEqual(read, [['current', 'entropy/appendix.tex']]);
  assert.equal(elements['source-reader-dialog'].open, true);
  assert.equal(elements['source-reader-status'].textContent, 'Lines 3–4');
  const marked = elements['source-reader-body'].children[0].children[1];
  assert.equal(marked.tag, 'mark');
  assert.equal(marked.textContent, 'Three\nFour\n');
  fileDigest = 'c'.repeat(64);
  await open.listeners.click();
  assert.match(elements['source-reader-status'].textContent, /does not match the SHA-256/);
  assert.deepEqual(elements['source-reader-body'].children, []);
  await copy.listeners.click();
  assert.equal(copied.length, 1);
  assert.ok(copied[0].includes('entropy/appendix.tex:L3–L4 · bytes 28–67'));
  assert.ok(copied[0].endsWith(excerpt));

  const submitLiterature = elements['literature-search-form'].listeners.submit;
  const litQuery = elements['lit-query-input'];
  litQuery.value = 'Old literature';
  const first = submitLiterature({ preventDefault() {} });
  litQuery.value = 'New literature';
  const second = submitLiterature({ preventDefault() {} });
  literature.get('New literature')({ results: [{ title: 'New result' }] });
  await second;
  literature.get('Old literature')({ results: [{ title: 'Old result' }] });
  await first;
  const [paper] = elements['literature-results'].children;
  assert.match(paper.innerHTML, /New result/);
});
