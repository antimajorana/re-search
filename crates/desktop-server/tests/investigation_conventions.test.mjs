import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../assets/investigation_conventions.js', import.meta.url), 'utf8');
const code = source.replace("import { renderMathTextInto } from '/assets/math.js';", '');
const { conventionStatus, createConventionCatalog, filterConventions } = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));

test('catalog status distinguishes an unfinished search, selected-page absence, and partial coverage', () => {
  assert.equal(conventionStatus(null, false, false), 'No catalog recorded');
  assert.equal(conventionStatus(null, true, true), 'Reading the paper…');
  assert.equal(conventionStatus(null, true, false), 'Review stopped before the first batch');
  assert.equal(conventionStatus({items:[{}],done:true}, true, true, 1, false), '1 entry from an earlier reading · reading again…');
  assert.equal(conventionStatus({items:[{}], done:false}, true, true), '1 found so far');
  assert.equal(conventionStatus({items:[{}], done:false}, true, false), 'Incomplete catalog');
  assert.equal(conventionStatus({items:[], done:true, coverage:'paper'}, true, false, 1), 'None reported in the selected pages');
  assert.equal(conventionStatus({items:[], done:true, coverage:'partial'}, true, false), 'None located in the reviewed passages');
});

test('convention lookup matches scientific notation, meaning, scope, rationale and category', () => {
  const items = [{id:'m',category:'notation',label:'$m$',meaning:'mass',scope:'Equilibrium'},
    {id:'bands',category:'reporting',label:'Bands',meaning:'Confidence interval',scope:'Fig. 4',rationale:'Overlapping bars'}];
  for (const term of ['interval','FIG. 4','Measurement reporting','overlapping']) assert.equal(filterConventions(items,term)[0].id,'bands');
  assert.equal(filterConventions(items,'$m$')[0].id, 'm');
  assert.deepEqual(filterConventions(items,'unknown'), []);
});

test('header_hidden_with_paper_but_no_mapping', t => {
  const mk = (tag = 'div') => ({
    tagName: tag.toUpperCase(), hidden: false, open: false, disabled: false, value: '', textContent: '', className: '',
    children: [],
    setAttribute() {},
    addEventListener() {},
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = [...nodes]; },
    querySelector(sel) { return this.children.find(c => c.tagName === sel.toUpperCase()) || null; },
    querySelectorAll() { return []; },
    getBoundingClientRect() { return { top: 0 }; },
  });
  const mapEl = mk('div');
  const panelEl = mk('section');
  const prev = { document: globalThis.document };
  globalThis.document = {
    createElement: mk,
    createTextNode: s => ({ textContent: s }),
    getElementById: id => (id === 'investigation-map' ? mapEl : id === 'investigation_panel' || id === 'investigation-panel' ? panelEl : null),
    activeElement: null,
  };
  t.after(() => Object.assign(globalThis, prev));

  const host = mk('details');
  host.append(mk('summary'));
  const catalog = createConventionCatalog(host, () => {}, async () => {});

  catalog.update({ source_id: 'src-1', map_seq: null, conventions: null, conventions_requested: false }, false);
  assert.equal(host.hidden, true);

  catalog.update({ source_id: 'src-1', map_seq: 11, conventions: null, conventions_requested: false }, false);
  assert.equal(host.hidden, false);
});

test('pending_count_shown', () => {
  assert.equal(
    conventionStatus({ items: [{}], done: false, pending_items: ['metric-sign', 'fourier-norm'] }, true, false),
    'Incomplete catalog · 2 citations pending',
  );
  assert.equal(
    conventionStatus({ items: [{}], done: false, pending_items: ['metric-sign'] }, true, true),
    '1 found so far · 1 citation pending',
  );
});
