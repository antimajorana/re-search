import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const scopeMod = await import(new URL('../assets/investigation_scope.js', import.meta.url));
const { scopeChipData, createScopeChip } = scopeMod;

const convSource = await readFile(new URL('../assets/investigation_conventions.js', import.meta.url), 'utf8');
const convCode = convSource.replace(
  "import { renderMathTextInto } from '/assets/math.js';",
  'const renderMathTextInto = (el, text) => { el.textContent = String(text ?? ""); };',
);
const { conventionStatus, createConventionCatalog } = await import(
  'data:text/javascript;base64,' + Buffer.from(convCode).toString('base64')
);

function makeElement(tag = 'div') {
  const attrs = new Map();
  const listeners = new Map();
  let text = '';
  return {
    tagName: tag.toUpperCase(),
    id: '',
    className: '',
    dataset: {},
    children: [],
    hidden: false,
    disabled: false,
    open: false,
    value: '',
    scrollTop: 0,
    isConnected: true,
    afterInserted: [],
    get textContent() {
      return this.children.length ? this.children.map(c => c.textContent ?? '').join('') : text;
    },
    set textContent(v) {
      this.children = [];
      text = String(v ?? '');
    },
    get innerHTML() {
      throw new Error('innerHTML must not be used for untrusted scope or convention text');
    },
    set innerHTML(_v) {
      throw new Error('innerHTML must not be set for untrusted scope or convention text');
    },
    setAttribute(k, v) { attrs.set(k, String(v)); },
    getAttribute(k) { return attrs.get(k); },
    removeAttribute(k) { attrs.delete(k); },
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = [...nodes]; text = ''; },
    after(...nodes) { this.afterInserted.push(...nodes); },
    addEventListener(type, fn) { listeners.set(type, fn); },
    getBoundingClientRect() { return { top: 0 }; },
    focus() {},
    querySelector(sel) {
      const visit = node => {
        for (const c of node.children || []) {
          if (c.tagName === sel.toUpperCase()) return c;
          const found = visit(c);
          if (found) return found;
        }
        return null;
      };
      return visit(this);
    },
    querySelectorAll(sel) {
      const out = [];
      const visit = node => {
        for (const c of node.children || []) {
          if (c.tagName === sel.toUpperCase()) out.push(c);
          visit(c);
        }
      };
      visit(this);
      return out;
    },
    async click() { return listeners.get('click')?.({ currentTarget: this, preventDefault() {} }); },
  };
}

test('scopeChipData formats resolved targets, ranges and pages and ignores unresolved scope', () => {
  assert.equal(scopeChipData(null), null);
  assert.equal(scopeChipData({ status: 'unresolved', reason: 'none' }), null);

  const single = scopeChipData({
    status: 'resolved',
    label: 'Section V.D.4 (lines 2687–3105)',
    targets: [{
      kind: 'section',
      label: 'Section V.D.4',
      title: 'Dislocations and Twist Defects',
      start_line: 2687,
      end_line: 3105,
      page_start: 18,
      page_end: 21,
    }],
    ranges: [[2687, 3105]],
  });
  assert.equal(single.text, 'Scoped to Section V.D.4');
  assert.equal(single.detail, 'lines 2687–3105 · pp. 18–21');
  assert.match(single.tooltip, /Dislocations and Twist Defects/);

  const multi = scopeChipData({
    status: 'resolved',
    label: 'Section V.D.4, Eq. (52)',
    targets: [
      { kind: 'section', label: 'Section V.D.4', start_line: 100, end_line: 140 },
      { kind: 'equation', label: 'Eq. (52)', start_line: 110, end_line: 150 },
    ],
    ranges: [[100, 150]],
  });
  assert.equal(multi.text, 'Scoped to Section V.D.4, Eq. (52)');
  assert.equal(multi.detail, 'lines 100–150');
});

test('createScopeChip renders untrusted labels via textContent only and toggles visibility', t => {
  const prevDoc = globalThis.document;
  globalThis.document = {
    createElement: makeElement,
    createTextNode: s => ({ textContent: String(s ?? ''), children: [] }),
  };
  t.after(() => { globalThis.document = prevDoc; });

  const statusAnchor = makeElement('div');
  const chip = createScopeChip(statusAnchor);
  assert.equal(statusAnchor.afterInserted.length, 1);
  assert.equal(chip.element.hidden, true);

  chip.update({
    status: 'resolved',
    targets: [{
      label: 'Section V.D.4',
      title: '<img src=x onerror=alert(1)>',
      start_line: 40,
      end_line: 90,
      page_start: 3,
      page_end: 3,
    }],
    ranges: [[40, 90]],
  });
  assert.equal(chip.element.hidden, false);
  assert.equal(chip.element.textContent, 'Scoped to Section V.D.4 · lines 40–90 · p. 3');
  assert.equal(chip.element.getAttribute('title'), 'Section V.D.4: <img src=x onerror=alert(1)>');

  chip.update({ status: 'unresolved', reason: 'not found' });
  assert.equal(chip.element.hidden, true);
  assert.equal(chip.element.textContent, '');

  chip.update({ status: 'resolved', label: 'Appendix B', targets: [], ranges: [[500, 560]] });
  assert.equal(chip.element.hidden, false);
  chip.reset();
  assert.equal(chip.element.hidden, true);
});

test('section-scoped conventions catalog shows section catalog label and Catalog all conventions button', async t => {
  assert.equal(
    conventionStatus({ items: [{}], done: true, coverage: 'paper', coverage_scope: 'section' }, true, false),
    '1 entry · section catalog',
  );
  assert.equal(
    conventionStatus({ items: [{}, {}], done: true, coverage: 'partial', coverage_scope: 'section' }, true, false),
    '2 entries · section catalog · partial coverage',
  );
  assert.equal(
    conventionStatus({ items: [], done: true, coverage: 'paper', coverage_scope: 'section' }, true, false),
    'None reported in the section · section catalog',
  );
  assert.equal(
    conventionStatus({ items: [{}, {}], done: true, coverage: 'paper', coverage_scope: 'paper' }, true, false),
    '2 entries',
  );

  const prevDoc = globalThis.document;
  const host = makeElement('details');
  const summary = makeElement('summary');
  host.append(summary);
  const mapEl = makeElement('div');
  const panelEl = makeElement('section');
  globalThis.document = {
    createElement: makeElement,
    createTextNode: s => ({ textContent: String(s ?? ''), children: [] }),
    getElementById: id => (id === 'investigation-map' ? mapEl : id === 'investigation-panel' ? panelEl : null),
    activeElement: null,
  };
  t.after(() => { globalThis.document = prevDoc; });

  const requestedMaps = [];
  const catalog = createConventionCatalog(host, () => {}, async mapSeq => {
    requestedMaps.push(mapSeq);
  });

  catalog.update({
    map_seq: 19,
    source_id: 'src-1',
    conventions_requested: true,
    conventions_is_current: true,
    conventions: {
      done: true,
      coverage: 'paper',
      coverage_scope: 'section',
      limitations: [],
      items: [{ id: 'b', category: 'notation', label: 'b', meaning: 'Burgers vector', scope: 'Sec. V.D.4', basis: 'explicit', anchors: [] }],
    },
  }, false);

  assert.equal(summary.textContent, 'Conventions · 1 entry · section catalog');
  const resumeBtn = host.querySelector('button');
  assert.equal(resumeBtn.hidden, false);
  assert.equal(resumeBtn.textContent, 'Catalog all conventions');

  await resumeBtn.click();
  assert.deepEqual(requestedMaps, [19]);

  // Once upgraded to paper scope, the resume button hides and the section label disappears.
  catalog.update({
    map_seq: 19,
    source_id: 'src-1',
    conventions_requested: true,
    conventions_is_current: true,
    conventions: {
      done: true,
      coverage: 'paper',
      coverage_scope: 'paper',
      limitations: [],
      items: [
        { id: 'b', category: 'notation', label: 'b', meaning: 'Burgers vector', scope: 'Sec. V.D.4', basis: 'explicit', anchors: [] },
        { id: 'g', category: 'notation', label: 'G', meaning: 'Symmetry group', scope: 'Paper', basis: 'explicit', anchors: [] },
      ],
    },
  }, false);

  assert.equal(summary.textContent, 'Conventions · 2 entries');
  assert.equal(resumeBtn.hidden, true);
});
