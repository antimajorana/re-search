import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const require = createRequire(import.meta.url);
const katex = require(fileURLToPath(new URL('../vendor/katex/katex.min.js', import.meta.url)));
const mathUrl = pathToFileURL(fileURLToPath(new URL('../assets/math.js', import.meta.url))).href;
const source = await readFile(new URL('../assets/investigation_map.js', import.meta.url), 'utf8');
const graphUrl = pathToFileURL(fileURLToPath(new URL('../assets/investigation_graph.js', import.meta.url))).href;
const { mappedQuestionMatches, paperQuestionFocus, projectedPaperGraph, resultDependencies, savedFocusMap } = await import(graphUrl);
const local = source.replace("'/assets/math.js'", JSON.stringify(mathUrl)).replace('/assets/investigation_graph.js', graphUrl);
const { visibleMapNodes, claimNeighbors, foldSearchSpan, renderAnchorExcerptInto, createPaperMap } = await import('data:text/javascript;base64,' + Buffer.from(local).toString('base64'));
const nodes = [
  { id: 'mass', kind: 'assumption', sentence: 'Assume a unit mass.', main: false },
  { id: 'force', kind: 'result', sentence: 'Force equals acceleration.', main: true },
  { id: 'measure', kind: 'measurement', sentence: 'The force is measured with a spring.', main: false },
];
const map = { nodes, edges: [
  { from: 'mass', to: 'force', kind: 'depends_on', basis: 'inferred' },
  { from: 'measure', to: 'force', kind: 'supports', basis: 'explicit' },
] };

test('paper overview keeps assumptions and main results while searching the complete map', () => {
  assert.deepEqual(visibleMapNodes(nodes, false, '').map(n => n.id), ['mass', 'force']);
  assert.equal(visibleMapNodes(nodes, true, '').length, 3);
  assert.deepEqual(visibleMapNodes(nodes, false, 'SPRING').map(n => n.id), ['measure']);
  assert.deepEqual(visibleMapNodes(nodes, true, 'not found'), []);
});

test('upstream and downstream follow supporting-to-dependent edge direction and retain provenance', () => {
  const result = claimNeighbors(map, 'force');
  assert.deepEqual(result.incoming.map(x => [x.node.id, x.edge.basis]), [['mass', 'inferred'], ['measure', 'explicit']]);
  assert.deepEqual(result.outgoing, []);
  assert.deepEqual(claimNeighbors(map, 'mass').outgoing.map(x => x.node.id), ['force']);
});

test('contracted branches show hidden statements and preserve shared upstream nodes without duplicating them', () => {
  const graph = { nodes: [...nodes, { id: 'second', sentence: 'A second conclusion.', main: true }],
    edges: [
      { from: 'mass', to: 'measure', basis: 'explicit', kind: 'supports' },
      { from: 'measure', to: 'force', basis: 'inferred', kind: 'depends_on' },
      { from: 'mass', to: 'force', basis: 'explicit', kind: 'supports' },
      { from: 'measure', to: 'second', basis: 'explicit', kind: 'supports' },
    ] };
  const visible = ['mass', 'force', 'second'];
  const collapsed = projectedPaperGraph(graph, visible);
  assert.deepEqual(collapsed.rows[0], ['mass']);
  assert.deepEqual(new Set(collapsed.rows[1]), new Set(['force', 'second']));
  const mixed = collapsed.edges.find(e => e.to === 'force');
  assert.deepEqual(mixed.via, ['measure']);
  assert.equal(mixed.inferred, true);
  assert.equal(mixed.explicit, true);
  assert.equal(collapsed.edges.find(e => e.to === 'second').inferred, false);
  assert.deepEqual(collapsed.neighbors.get('mass'), ['measure']);
  const full = projectedPaperGraph(graph, [...visible, ...mixed.via]);
  assert.equal(full.edges.length, 4);
  assert.ok(full.edges.every(e => e.via.length === 0));
});

test('reading focus uses overlap with recorded questions and never infers a match from the claim alone', () => {
  const question = 'How was the spring calibration tested?';
  const matches = mappedQuestionMatches([
    { id: 'cal', questions: ['Does spring calibration affect the uncertainty?'] },
    { id: 'guess', sentence: question, questions: [] },
    { id: 'single', questions: ['What spring was used?'] },
  ], question);
  assert.deepEqual([...matches.keys()], ['cal']);
  assert.equal(mappedQuestionMatches(nodes, '').size, 0);
});

test('model-selected answer IDs take precedence over lexical matches, including an explicitly empty selection', () => {
  const question = 'Are test particles stationary?';
  const map = { nodes: [{id:'words', questions:[question]}, {id:'answer', questions:[]}] };
  assert.deepEqual([...paperQuestionFocus(map, question).keys()], ['words']);
  assert.deepEqual([...paperQuestionFocus({...map, answer_node_ids:['answer']}, question).keys()], ['answer']);
  assert.deepEqual([...paperQuestionFocus({...map, answer_node_ids:[]}, question).keys()], []);
});

test('a result branch excludes unrelated conclusions and keeps separately saved focuses independent', () => {
  const node = id => ({id});
  const edge = (from,to) => ({from,to,kind:'supports'});
  const root = 'force';
  const base = {seq: 6, kind: 'investigation.map', payload: {edges:[]}};
  const branch = (seq, request_seq, edges) => ({seq,kind:'investigation.branch',payload:{node_id:root,map_seq:6,request_seq,edges}});
  const one = branch(8,7,[edge('assumption','calibration'),edge('calibration',root)]);
  const two = branch(10,9,[edge('assumption','control'),edge('control',root)]);
  const map = {nodes:['assumption','calibration','control',root,'another'].map(node),
    edges:[...one.payload.edges,...two.payload.edges,edge('control','another')]};
  assert.deepEqual(resultDependencies(map, 'another').nodes.map(n => n.id), ['assumption','control','another']);
  assert.deepEqual(savedFocusMap(map,root,6,7,[base,one,two]).nodes.map(n => n.id), ['assumption','calibration',root]);
  assert.deepEqual(savedFocusMap(map,root,6,9,[base,one,two]).nodes.map(n => n.id), ['assumption','control',root]);
});

test('focus membership follows edges between existing statements, including a later focus that deepens an older route', () => {
  const edge = (from, to) => ({from,to,kind:'supports'});
  const base = {seq:1, kind:'investigation.map', payload:{edges:[edge('readout','result'), edge('control','result')]}};
  const record = (seq, added) => ({seq, kind:'investigation.branch', payload:{map_seq:1, node_id:'result', request_seq:seq-1, nodes:[], edges:added}});
  const one = record(3,[edge('unit','readout')]), two = record(5,[edge('unit','control')]);
  const deepen = record(7,[edge('calibration','readout')]), limited = record(9,[]);
  const events = [base,one,two,deepen,limited];
  const map = { nodes:['unit','readout','control','calibration','result'].map(id => ({id})), edges:events.flatMap(e=>e.payload.edges) };
  const pairs = seq => savedFocusMap(map,'result',1,seq,events).edges.map(e=>e.from+'→'+e.to).sort();
  assert.deepEqual(pairs(2), ['readout→result','unit→readout']);
  assert.deepEqual(pairs(4), ['control→result','unit→control']);
  assert.deepEqual(pairs(6), ['calibration→readout','readout→result']);
  assert.deepEqual(pairs(8), []);
});

function makeFakeDom() {
  const mk = (tag = 'div') => {
    let text = '', html = '', innerHtmlWrites = 0;
    const node = {
      tagName: tag.toUpperCase(), className: '', dataset: {}, children: [], type: '', hidden: false,
      get innerHtmlWrites() { return innerHtmlWrites; },
      get textContent() { return this.children.length ? this.children.map(c => c.textContent ?? '').join('') : text; },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html || (this.children.length ? this.children.map(c => c.innerHTML || c.textContent || '').join('') : text); },
      set innerHTML(v) { innerHtmlWrites++; this.children = []; html = String(v ?? ''); text = html.replace(/<[^>]+>/g, ''); },
      append(...nodes) { this.children.push(...nodes); },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener() {},
      setAttribute() {},
      getBoundingClientRect() { return { top: 0 }; },
      querySelectorAll(sel) {
        const out = [];
        const visit = n => {
          for (const c of n.children || []) {
            if (sel.startsWith('.') ? String(c.className || '').split(/\s+/).includes(sel.slice(1)) : c.tagName === sel.toUpperCase()) out.push(c);
            visit(c);
          }
        };
        visit(this);
        return out;
      },
    };
    return node;
  };
  const prevDoc = globalThis.document, prevWin = globalThis.window;
  globalThis.window = { katex };
  globalThis.document = {
    createElement: mk,
    getElementById: () => mk('div'),
  };
  return {
    mk,
    restore() { globalThis.document = prevDoc; globalThis.window = prevWin; },
  };
}

test('quote_mark_uses_text_nodes', () => {
  const dom = makeFakeDom();
  try {
    const quoteEl = dom.mk('blockquote');
    renderAnchorExcerptInto(quoteEl, 'We assume\nstrictly positive mass in this model.', 'strictly positive mass');
    assert.equal(quoteEl.innerHtmlWrites, 0);
    const marks = quoteEl.querySelectorAll('mark');
    assert.equal(marks.length, 1);
    assert.equal(marks[0].innerHtmlWrites, 0);
    assert.equal(marks[0].textContent, 'strictly positive mass');
    assert.equal(quoteEl.textContent, 'We assume\nstrictly positive mass in this model.');
  } finally {
    dom.restore();
  }
});

test('quote_with_html_chars_escaped', () => {
  const dom = makeFakeDom();
  try {
    const payload = '<img src=x onerror=alert(1)> & <b>bold</b>';
    const quoteEl = dom.mk('blockquote');
    renderAnchorExcerptInto(quoteEl, 'Before ' + payload + ' after.', payload);
    const marks = quoteEl.querySelectorAll('mark');
    assert.equal(marks.length, 1);
    assert.equal(marks[0].innerHtmlWrites, 0);
    assert.equal(marks[0].textContent, payload);
    // Also verify when math is present alongside HTML chars inside the marked segment.
    const mathPayload = 'For $x < y$ and <script>alert(1)</script> end';
    renderAnchorExcerptInto(quoteEl, mathPayload, '$x < y$ and <script>alert(1)</script>');
    const mark2 = quoteEl.querySelectorAll('mark')[0];
    assert.ok(!mark2.innerHTML.includes('<script>'), 'script tag must be escaped: ' + mark2.innerHTML);
    assert.ok(mark2.innerHTML.includes('&lt;script&gt;'), 'expected escaped script tag: ' + mark2.innerHTML);
  } finally {
    dom.restore();
  }
});

test('mark_widened_to_math_span', () => {
  const dom = makeFakeDom();
  try {
    const excerpt = 'We define $E = mc^2$ in natural units.';
    const span = foldSearchSpan(excerpt, 'define $E = mc');
    assert.equal(excerpt.slice(span[0], span[1]), 'define $E = mc^2$');
    const quoteEl = dom.mk('blockquote');
    renderAnchorExcerptInto(quoteEl, excerpt, 'define $E = mc');
    const mark = quoteEl.querySelectorAll('mark')[0];
    assert.ok(mark.innerHTML.includes('katex'), 'widened math span should render via KaTeX: ' + mark.innerHTML);
    assert.equal(quoteEl.children[0].textContent, 'We ');
    assert.equal(quoteEl.children[2].textContent, ' in natural units.');
  } finally {
    dom.restore();
  }
});

test('corrected_note_rendered', () => {
  const dom = makeFakeDom();
  try {
    const host = dom.mk('div');
    const mapApi = createPaperMap(host, { openSource() {}, openPdf: () => false, citedPaperLinks: () => [], act() {} });
    const container = dom.mk('div');
    mapApi.renderAnchors(container, [{
      source_id: 'paper', path: 'paper/main.txt', start_line: 20, end_line: 22,
      excerpt: 'Line twenty text here.', quote: 'twenty text', kind: 'passage',
      cited_range: [12, 16], range_basis: ['relocated'],
    }]);
    const notes = container.querySelectorAll('.investigation-anchor-note');
    assert.equal(notes.length, 1);
    assert.equal(notes[0].textContent, 'lines corrected by the app (cited 12–16)');
  } finally {
    dom.restore();
  }
});
