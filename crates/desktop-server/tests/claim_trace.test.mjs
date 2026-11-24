import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const assets = new URL('../assets/', import.meta.url);
const asModule = text => 'data:text/javascript;base64,' + Buffer.from(text).toString('base64');
const math = JSON.stringify(new URL('math.js', assets).href);
const source = async file => (await readFile(new URL(file, assets), 'utf8')).replace("'/assets/math.js'", math);
const inspector = asModule(await source('canvas_dag_detail.js'));
const { assumptionDependents } = await import(inspector);
const trace = (await source('claim_trace.js'))
  .replace("'/assets/api.js'", JSON.stringify(new URL('api.js', assets).href))
  .replace("'/assets/canvas_dag_detail.js'", JSON.stringify(inspector))
  .replace("'/assets/source_links.js'", JSON.stringify(asModule(await source('source_links.js'))));
const { splitClaims, linkedSteps } = await import(asModule(trace));

test('claim selection keeps exact saved excerpts, equations, citations and code', () => {
  const equation = String.raw`\[\Delta x = 3.5\] [e-9].`;
  const code = ['~~~python', 'print("one. two.")', '~~~'].join('\n');
  const phrases = ['A first claim [e-1].', 'For the change we obtain', equation, 'A text result.', code, 'The end.'];
  const answer = phrases.join('\n');
  assert.deepEqual(splitClaims(answer), phrases);
  const rows = [
    { entry_id: 'e-1', seq: 2, summary: 'A claim about a boundary.', entry_type: 'result' },
    { entry_id: 'e-9', seq: 3, latex: String.raw`\Delta x = 3.5`, entry_type: 'symbolic_computation' },
    { entry_id: 'e-99', seq: 14, summary: 'A late step.', entry_type: 'result' },
  ];
  assert.deepEqual(linkedSteps('Claim [e-1], [e-99].', rows, 5).map(r => r.entry.entry_id), ['e-1']);
  assert.deepEqual(linkedSteps('The number is 1e-9 or $e-1$.', rows, 5), []);
  assert.equal(linkedSteps(String.raw`The result is $\Delta x = 3.5$.`, rows, 5)[0].reason, 'Matching recorded equation');
  assert.equal(linkedSteps('A claim about a boundary.', rows, 5)[0].reason, 'Matching recorded statement');
});

test('assumption impact follows indirect dependencies once, including final conclusions', () => {
  const make = (entry_id, depends_on) => ({entry_id, depends_on});
  const graph = [make('a',['end']), make('b',['a']), make('c',['a']), make('end',['b','c']), make('other',[])];
  assert.deepEqual(assumptionDependents(graph, 'a').map(x => [x.entry.entry_id,x.direct,x.parent]),
    [['b',true,'a'],['c',true,'a'],['end',false,'b']]);
  assert.deepEqual(assumptionDependents(graph, 'other'), []);
});
