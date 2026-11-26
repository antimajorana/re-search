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
  const fetched = (id) => [{ kind: 'literature.fetched', payload: { provider: 'arxiv', papers: [{ id }] } }];
  assert.equal(sl.sessionPaperReferences(fetched('2301.12345v1'), [src])[0].savedSource, src);
  assert.equal(sl.sessionPaperReferences(fetched('2301.12345v2'), [src])[0].savedSource, undefined);
  const correctedVersion = { ...src, arxiv_cover_id: '2301.12345v2', url: 'https://arxiv.org/pdf/2301.12345v1' };
  assert.equal(sl.sessionPaperReferences(fetched('2301.12345v2'), [correctedVersion])[0].savedSource, correctedVersion);
  assert.equal(sl.sessionPaperReferences(fetched('2301.12345v1'), [correctedVersion])[0].savedSource, undefined);
  assert.deepEqual(sl.linksForIngestedSource(correctedVersion).map(link => link.url), [
    'https://arxiv.org/abs/2301.12345v2', 'https://arxiv.org/pdf/2301.12345v2',
  ]);
  const summarySources = sl.sessionSourcesUsed({sources: [src, { source_id:'unused' }], events: [],
    investigation:{summary:{what:{anchors:[{source_id:src.source_id}]},why:{unstated:true},how:{unstated:true}}}});
  assert.deepEqual(summarySources.map(row => [row.source.source_id,row.inSummary]), [[src.source_id,true]]);
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
  const scholarId = 'a'.repeat(40);
  const papers = sl.sessionPaperReferences([
    { kind: 'literature.fetched', payload: { provider: 'crossref', hits: [
      { id: '10.1111/2501.00001', doi: '10.1111/2501.00001', title: 'Duplicate DOI' },
      { id: '10.2222/2501.00002', doi: '10.2222/2501.00002', url: 'https://doi.org/10.2222/2501.00002', title: 'DOI only' },
    ] } },
    { kind: 'literature.fetched', payload: { provider: 'arxiv', papers: [
      { id: '2501.00001v2', base_id: '2501.00001', doi: '10.1111/2501.00001', title: 'Linked paper' },
      { id: 'hep-th/9901001v2', base_id: 'hep-th/9901001' },
    ] } },
    { kind: 'literature.fetched', payload: { provider: 'semantic_scholar', hits: [
      { id: scholarId, title: 'Semantic Scholar reference' },
    ] } },
  ]);
  assert.deepEqual(papers.map(paper => paper.title), [
    'Semantic Scholar reference', 'Linked paper', 'hep-th/9901001v2', 'DOI only',
  ]);
  assert.equal(sl.linksForPaper(papers[0])[0].url, `https://www.semanticscholar.org/paper/${scholarId}`);
  assert.equal(sl.linksForPaper(papers[2])[0].url, 'https://arxiv.org/abs/hep-th/9901001v2');
  assert.deepEqual(sl.linksForPaper(papers[3]).map(link => link.url), ['https://doi.org/10.2222/2501.00002']);

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

test('Sources Used keeps final-answer citations and active proof evidence scoped to each paper', async () => {
  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const code = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8').replace("'/assets/math.js'", mathUrl);
  const { linksForPaper, sessionSourcesUsed } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
  const event = (kind, payload = {}) => ({ kind, payload });
  const reader = { source_id: 'notes-1', path: 'notes-1/counting.txt', start_line: 2 };
  const session = {
    events: [
      event('session.start', { prompt: 'Compare DOI:10.1000/user-input' }),
      event('literature.fetched', { provider: 'crossref', hits: [
        { title: 'Linked boundary result', doi: '10.1000/result' },
        { title: 'Background experiment', doi: '10.1000/unused' },
      ] }),
      event('literature.fetched', { provider: 'arxiv', papers: [
        { id: '2402.12345v3', base_id: '2402.12345', title: 'Latest version report' },
      ] }),
      event('proof.entry', { entry_id: 'e-source', summary: 'See DOI 10.1000/result.' }),
      event('proof.entry', { entry_id: 'e-file', summary: 'Check the proof against the notes.' }),
      event('proof.entry', { entry_id: 'e-spare', summary: 'Possibly use DOI 10.1000/unused.' }),
      event('proof.entry', { entry_id: 'e-final', entry_type: 'final_answer', depends_on: ['e-source', 'e-file'] }),
      event('model.response', { stop_reason: 'tool_use' }),
      event('model.text', { text: 'Consider DOI:10.1000/draft.' }),
      event('model.response', { stop_reason: 'end_turn' }),
      event('model.text', { text: 'See [the result](https://doi.org/10.1000/result) and arXiv:2402.12345v2.' }),
      event('session.end', { outcome: 'completed' }),
    ],
    sources: [{ source_id: 'notes-1', title: 'Counting proof notes', primary_path: reader.path }],
    linked_sources: [{ entry_id: 'e-file', reader }],
  };
  const used = sessionSourcesUsed(session);
  assert.deepEqual(used.map(record => record.paper.title), [
    'Linked boundary result', 'Counting proof notes', 'arXiv:2402.12345v2',
  ]);
  assert.equal(used[0].cited, true);
  assert.deepEqual([...used[0].steps], ['e-source']);
  assert.deepEqual([...used[1].readers.values()], [reader]);
  const secondReader = { ...reader, start_line: 8 };
  session.linked_sources.push({ entry_id: 'e-file', reader: secondReader });
  session.sources.push({ source_id: 'notes-2', title: 'Counting proof notes', primary_path: 'notes-2/counting.txt' });
  assert.deepEqual([...sessionSourcesUsed(session)[1].readers.values()], [reader, secondReader]);
  assert.equal(sessionSourcesUsed(session)[1].source.source_id, 'notes-1');
  assert.equal(linksForPaper(used[2].paper)[0].url, 'https://arxiv.org/abs/2402.12345v2');
  const doi = '10.1016/S0003-4916(02)00001-2';
  const bracketed = sessionSourcesUsed({ events: [
    event('model.text', { text: `See [derivation](https://doi.org/${doi}).` }),
    event('session.end', { outcome: 'completed' }),
  ] });
  assert.deepEqual(bracketed.map(record => record.paper.doi), [doi]);
  assert.equal(linksForPaper(bracketed[0].paper)[0].url, `https://doi.org/${doi}`);
  const citedPage = 'https://science.nasa.gov/mission/Euclid';
  const direct = sessionSourcesUsed({ events: [
    event('model.text', { text: `See [Euclid mission](${citedPage}), ${citedPage} and https://inspirehep.net/literature/123.` }),
    event('session.end', { outcome: 'completed' }),
  ] });
  assert.deepEqual(direct.map(({ paper }) => paper.title), ['Euclid mission', 'https://inspirehep.net/literature/123']);
  assert.equal(direct[0].paper.url, citedPage);
  assert.deepEqual(linksForPaper(direct[0].paper), []);
  assert.equal(linksForPaper(direct[1].paper)[0].url, direct[1].paper.url);
});

test('Project library normalizes citations and finds excerpts across session sources', async () => {
  const moduleUrl = (code) => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
  const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;
  const linksCode = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", JSON.stringify(mathUrl));
  const libraryCode = fs.readFileSync(path.join(assetsDir, 'project_library.js'), 'utf8')
    .replace("'/assets/api.js'", JSON.stringify(moduleUrl('export {};')))
    .replace("'/assets/library_send_back.js'", JSON.stringify(pathToFileURL(path.join(assetsDir, 'library_send_back.js')).href))
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
