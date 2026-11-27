import assert from 'node:assert/strict';
import test from 'node:test';

import { createInvestigationContextDrawer } from '../assets/investigation_context.js';
import {
  COMPARE_HYPOTHESES_BTN_LABEL,
  COMPARE_HYPOTHESES_FOCUS,
  HIDE_UNCHECKED_LABEL,
  HIDE_UNCHECKED_STORAGE_KEY,
  PREREQUISITES_BTN_LABEL,
  PREREQUISITES_FOCUS,
  filterContextCards,
  readHideUncheckedPref,
  renderContextCardsSection,
  resolveCardProvenance,
  syncContextCardsDrawer,
  writeHideUncheckedPref,
} from '../assets/investigation_context_cards.js';
import { BORDER_DASHED, BORDER_SOLID, PROVENANCE_STYLES } from '../assets/ui_badge.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      dataset: {},
      style: {},
      type: '',
      value: '',
      title: '',
      checked: false,
      hidden: false,
      disabled: false,
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      setAttribute(k, v) { attrs.set(k, String(v)); },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      hasAttribute(k) { return attrs.has(k); },
      appendChild(c) { el.children.push(c); return c; },
      append(...cs) {
        for (const c of cs) {
          el.children.push(typeof c === 'string' ? { textContent: c, children: [] } : c);
        }
      },
      replaceChildren(...cs) { el.children = [...cs]; },
      addEventListener(t, fn) {
        if (!listeners.has(t)) listeners.set(t, []);
        listeners.get(t).push(fn);
      },
      dispatch(t, ev = {}) {
        for (const fn of listeners.get(t) || []) fn({ target: el, ...ev });
      },
      click() {
        if (!el.disabled) el.dispatch('click');
      },
      set textContent(v) { text = String(v ?? ''); },
      get textContent() { return text; },
      set innerHTML(_v) { innerHtmlWrites += 1; },
      get innerHTML() { return ''; },
    };
    return el;
  };
  return {
    document: {
      createElement: (tag) => makeEl(tag),
      createElementNS: (ns, tag) => makeEl(tag, ns),
    },
    innerHtmlCount: () => innerHtmlWrites,
  };
}

function collectText(node) {
  if (!node) return '';
  let out = node.textContent || '';
  for (const child of node.children || []) out += `\n${collectText(child)}`;
  return out;
}

function findAll(root, pred) {
  const out = [];
  const walk = (n) => {
    if (!n) return;
    if (pred(n)) out.push(n);
    for (const c of n.children || []) walk(c);
  };
  walk(root);
  return out;
}

test('renders three provenance styles, quote chips, hypothesis anchors, and zero verified or innerHTML', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  writeHideUncheckedPref(false);

  const openedQuotes = [];
  const startedTasks = [];
  const mv = {
    paper: { title: 'Test Paper' },
    tasks: [{
      request_seq: 11,
      task: 'context',
      tags: ['n-thm1'],
      focus: PREREQUISITES_FOCUS,
      run_state: 'done',
      model_calls: 2,
      cap: 8,
      saved_items: 3,
    }],
    nodes: [{
      id: 'n-thm1',
      label: 'Theorem 1',
      context_cards: [
        {
          type: 'symbol',
          title: 'Spectral gap gamma',
          body: 'Fixed positive spectral gap constant.',
          origin: 'paper',
          anchors: [{ source_id: 'src1', path: 'paper.tex', start_line: 8, end_line: 8, quote: 'spectral gap' }],
        },
        {
          type: 'cited_result',
          title: 'Smith24 Exponential Bound',
          body: 'External theorem assumes continuous boundary weight.',
          origin: 'external',
          bib_key: 'Smith24',
          short_ref: 'Smith 2024, Thm. 1',
          anchors: [{ source_id: 'src_ext', path: 'smith24.tex', start_line: 12, end_line: 12, quote: 'semigroup decays' }],
          hypotheses: [{
            text: 'Continuous boundary weight',
            anchor: { source_id: 'src_ext', path: 'smith24.tex', start_line: 11, end_line: 11, quote: 'boundary weight is continuous' },
          }],
        },
        {
          type: 'prerequisite',
          title: 'Standard Gronwall lemma',
          body: 'Basic ODE bound.',
          origin: 'model_background',
          anchors: [],
        },
      ],
    }],
  };

  const section = renderContextCardsSection({
    mv,
    selectedTag: 'n-thm1',
    modelConnected: true,
    onStartContextTask: (focus, tag) => startedTasks.push({ focus, tag }),
    onOpenQuote: (t) => openedQuotes.push(t),
  });

  assert.equal(dom.innerHtmlCount(), 0);
  const fullText = collectText(section);
  assert.equal(fullText.toLowerCase().includes('verified'), false);
  assert.match(fullText, /From this paper/);
  assert.match(fullText, /Outside this paper: Smith 2024, Thm\. 1/);
  assert.match(fullText, /Model background, not checked/);

  const cards = findAll(section, (n) => n.classList?.contains('inv-ctx-card'));
  assert.equal(cards.length, 3);
  assert.equal(cards[0].getAttribute('data-border'), BORDER_SOLID);
  assert.equal(cards[1].getAttribute('data-border'), BORDER_SOLID);
  assert.equal(cards[2].getAttribute('data-border'), BORDER_DASHED);
  assert.equal(resolveCardProvenance({ origin: 'paper' }).key, PROVENANCE_STYLES.from_paper.key);

  const quoteBtns = findAll(section, (n) => n.classList?.contains('ui-quote-chip-open'));
  assert.equal(quoteBtns.length, 3); // 1 paper + 1 external hypothesis + 1 external anchor
  quoteBtns[1].click();
  assert.equal(openedQuotes[0].source_id, 'src_ext');

  const prereqBtn = findAll(section, (n) => n.getAttribute('data-context-focus') === PREREQUISITES_FOCUS)[0];
  const cmpBtn = findAll(section, (n) => n.getAttribute('data-context-focus') === COMPARE_HYPOTHESES_FOCUS)[0];
  assert.equal(prereqBtn.textContent, PREREQUISITES_BTN_LABEL);
  assert.equal(cmpBtn.textContent, COMPARE_HYPOTHESES_BTN_LABEL);
  prereqBtn.click();
  cmpBtn.click();
  assert.deepEqual(startedTasks, [
    { focus: PREREQUISITES_FOCUS, tag: 'n-thm1' },
    { focus: COMPARE_HYPOTHESES_FOCUS, tag: 'n-thm1' },
  ]);
});

test('Hide unchecked toggle hides only model_background cards and persists across remounts', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const store = new Map();
  globalThis.localStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => { store.set(k, String(v)); },
  };
  writeHideUncheckedPref(false);

  const sampleCards = [
    { type: 'symbol', title: 'S', body: 'Paper symbol.', origin: 'paper', anchors: [{ source_id: 's1', path: 'a.tex', start_line: 1, end_line: 1, quote: 'gap' }] },
    { type: 'cited_result', title: 'E', body: 'Ext.', origin: 'external', anchors: [{ source_id: 's2', path: 'b.tex', start_line: 2, end_line: 2, quote: 'decay' }] },
    { type: 'prerequisite', title: 'B1', body: 'BG 1', origin: 'model_background', anchors: [] },
    { type: 'prerequisite', title: 'B2', body: 'BG 2', origin: 'model_background', anchors: [] },
  ];
  assert.equal(filterContextCards(sampleCards, { hideUnchecked: true }).length, 2);

  const drawer = createInvestigationContextDrawer(null, {});
  const mv = { paper: { title: 'P' }, tasks: [], nodes: [{ id: 'n-1', label: 'Thm 1', context_cards: sampleCards }] };
  syncContextCardsDrawer(drawer, { sessionId: 's-1', mv, selectedTag: 'n-1', modelConnected: false });

  let mountedCards = findAll(drawer.element, (n) => n.classList?.contains('inv-ctx-card'));
  assert.equal(mountedCards.length, 4);

  const checkbox = findAll(drawer.element, (n) => n.classList?.contains('inv-ctx-hide-checkbox'))[0];
  assert.equal(checkbox.getAttribute('aria-label'), HIDE_UNCHECKED_LABEL);
  checkbox.checked = true;
  checkbox.dispatch('change');

  assert.equal(store.get(HIDE_UNCHECKED_STORAGE_KEY), '1');
  assert.equal(readHideUncheckedPref(), true);
  mountedCards = findAll(drawer.element, (n) => n.classList?.contains('inv-ctx-card'));
  assert.equal(mountedCards.length, 2);
  assert.deepEqual(mountedCards.map((c) => c.getAttribute('data-origin')), ['paper', 'external']);

  // Re-mounting into a fresh drawer instance preserves Hide unchecked state.
  const drawer2 = createInvestigationContextDrawer(null, {});
  syncContextCardsDrawer(drawer2, { sessionId: 's-1', mv, selectedTag: 'n-1', modelConnected: true });
  const mountedAfterRemount = findAll(drawer2.element, (n) => n.classList?.contains('inv-ctx-card'));
  assert.equal(mountedAfterRemount.length, 2);
  assert.equal(collectText(drawer2.element).includes('2 unchecked model background cards hidden.'), true);
  assert.equal(dom.innerHtmlCount(), 0);
});

test('escapes XSS payloads via textContent with zero innerHTML and wires RunBar stop/retry', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  writeHideUncheckedPref(false);

  const actions = [];
  const mv = {
    paper: { title: '<script>alert(0)</script>' },
    tasks: [{
      request_seq: 19,
      task: 'context',
      tags: ['n-1'],
      focus: COMPARE_HYPOTHESES_FOCUS,
      run_state: 'paused',
      model_calls: 3,
      cap: 8,
      saved_items: 1,
    }],
    nodes: [{
      id: 'n-1',
      label: 'Thm 1',
      context_cards: [{
        type: 'cited_result',
        title: '<img src=x onerror=alert(1)>',
        body: '<svg onload=alert(2)>',
        origin: 'external',
        short_ref: '<script>bad</script>',
        anchors: [{ source_id: 's2', path: 'b.tex', start_line: 1, end_line: 1, quote: '<b>q</b>' }],
      }],
    }],
  };

  const sec = renderContextCardsSection({
    mv,
    selectedTag: 'n-1',
    modelConnected: true,
    onStartContextTask: (focus, tag) => actions.push(['start', focus, tag]),
    onStopTask: () => actions.push(['stop']),
  });
  assert.equal(dom.innerHtmlCount(), 0);
  const allEls = findAll(sec, () => true);
  assert.equal(allEls.some((e) => ['SCRIPT', 'IMG', 'B'].includes(e.tagName)), false);
  const text = collectText(sec);
  assert.equal(text.includes('<img src=x onerror=alert(1)>'), true);

  const continueBtn = findAll(sec, (n) => n.tagName === 'BUTTON' && n.textContent === 'Continue')[0];
  assert.ok(continueBtn);
  continueBtn.click();
  assert.deepEqual(actions, [['start', COMPARE_HYPOTHESES_FOCUS, 'n-1']]);
});

