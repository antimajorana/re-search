import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('../assets/investigation_map.js', import.meta.url), 'utf8');
const graphSource = await readFile(new URL('../assets/investigation_graph.js', import.meta.url), 'utf8');
const graphUrl = 'data:text/javascript;base64,' + Buffer.from(graphSource).toString('base64');
const { mappedQuestionMatches, paperQuestionFocus, projectedPaperGraph } = await import(graphUrl);
const local = source.replace("import { renderMathTextInto } from '/assets/math.js';", '').replace('/assets/investigation_graph.js', graphUrl);
const { visibleMapNodes, claimNeighbors } = await import('data:text/javascript;base64,' + Buffer.from(local).toString('base64'));
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
