import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('../assets/investigation_map.js', import.meta.url), 'utf8');
const local = source.replace("import { renderMathTextInto } from '/assets/math.js';", '');
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
