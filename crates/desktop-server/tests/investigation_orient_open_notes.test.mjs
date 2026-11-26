import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import {
  AUTHORS_CLAIM_LABELS,
  MAIN_RESULT_MODEL_LABEL,
  MAIN_RESULT_USER_LABEL,
  NO_EXPLICIT_CUE_TEXT,
  PIN_ACTION_LABEL,
  RIGHT_SLOT_V16_LABEL,
  UNPIN_ACTION_LABEL,
  formatHeadingCiteLine,
  renderInvestigationBadges,
  resolveEffectiveMainResult,
} from '../assets/investigation_badges.js';
import { renderInvestigationNodeCard } from '../assets/investigation_node.js';
import * as apiExports from '../assets/api.js';
import {
  extractAuthorsClaimLabel,
  inspectMapviewCapabilities,
  isMainResultNode,
} from '../assets/investigation_wholemap_model.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const byId = new Map();
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    let idVal = '';
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
      placeholder: '',
      maxLength: 0,
      hidden: false,
      disabled: false,
      checked: false,
      tabIndex: 0,
      clientWidth: 800,
      clientHeight: 500,
      get id() { return idVal; },
      set id(v) {
        if (idVal && byId.get(idVal) === el) byId.delete(idVal);
        idVal = String(v || '');
        if (idVal) byId.set(idVal, el);
      },
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        contains: (c) => classes.has(c),
        toggle: (c, force) => {
          const next = force === undefined ? !classes.has(c) : Boolean(force);
          if (next) classes.add(c); else classes.delete(c);
          return next;
        },
      },
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; if (k === 'id') this.id = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      insertBefore(node) { this.children.unshift(node); return node; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      removeEventListener(type, fn) {
        const list = listeners.get(type);
        if (list) listeners.set(type, list.filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, stopPropagation() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      focus() { this._focused = true; },
      getBoundingClientRect() { return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight }; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const m = /^\[([^=\]]+)(?:="([^"]*)")?\]$/.exec(sel);
          if (m) return m[2] === undefined ? n.attributes.has(m[1]) : n.getAttribute(m[1]) === m[2];
          return n.tagName.toLowerCase() === sel.toLowerCase();
        };
        const walk = (n) => {
          for (const c of n.children || []) {
            if (match(c)) out.push(c);
            walk(c);
          }
        };
        walk(this);
        return out;
      },
    };
    return el;
  };
  return {
    document: {
      createElement: (t) => makeEl(t, null),
      createElementNS: (ns, t) => makeEl(t, ns),
      getElementById: (id) => byId.get(id) || null,
    },
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
  };
}

function collectAllStrings(el, out = []) {
  if (!el) return out;
  if (el.textContent) out.push(el.textContent);
  if (el.innerHTML) out.push(el.innerHTML);
  if (el.title) out.push(el.title);
  for (const [, v] of el.attributes || []) out.push(v);
  for (const c of el.children || []) collectAllStrings(c, out);
  return out;
}

test('V7 investigation_badges renders authors half-badge, cues, heading cites, quote popover, V16 slot, and pin toggle', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const pinCalls = [];
  const quoteOpens = [];

  const node = {
    id: 'n-thm1111111',
    kind: 'theorem',
    label: 'Theorem 1.1: Decay Estimate',
    role: 'main_result',
    effective_is_main_result: true,
    head_cites: ['NS06'],
    authors_claim: {
      present: true,
      label: 'new',
      text: AUTHORS_CLAIM_LABELS.new,
      cues: ['we prove', 'our main'],
      cue_line: 'Cues in quoted lines: we prove, our main',
      heading_cite_line: 'cites [NS06] in its heading',
      relation_note: 'extends [NS06] to stretched decay',
      attributed_to: ['NS06'],
      quotes: [{
        source_id: 'src-1',
        path: 'main.tex',
        start_line: 14,
        end_line: 16,
        quote: 'We prove our main decay estimate extending [NS06].',
      }],
    },
  };

  const ctrl = renderInvestigationBadges(host, node, {
    onTogglePin: (n, nextPin) => pinCalls.push({ id: n.id, nextPin }),
    onOpenQuote: (target) => quoteOpens.push(target),
  });

  const left = host.querySelector('[data-half="left"]');
  const right = host.querySelector('[data-half="right"]');
  assert.ok(left, 'left half-badge present');
  assert.ok(right, 'right V16 placeholder half-badge present');
  assert.equal(left.textContent, AUTHORS_CLAIM_LABELS.new);
  assert.equal(right.textContent, RIGHT_SLOT_V16_LABEL);
  assert.equal(right.getAttribute('data-v16-slot'), 'prior-work-explore');
  assert.equal(right.getAttribute('aria-disabled'), 'true');
  assert.ok(!right.textContent.toLowerCase().includes('new'));
  assert.ok(!right.textContent.toLowerCase().includes('novel'));

  const headCite = host.querySelector('.inv-v7-heading-cites');
  assert.ok(headCite.textContent.includes('cites [NS06] in its heading'));
  assert.ok(!headCite.textContent.toLowerCase().includes('attributed'));

  // Popover starts collapsed, toggles open on left half-badge click.
  const popover = host.querySelector('.inv-v7-claim-popover');
  assert.equal(popover.hidden, true);
  left.click();
  assert.equal(popover.hidden, false);
  assert.ok(popover.textContent.includes('Cues in quoted lines: we prove, our main'));
  assert.ok(popover.textContent.includes('extends [NS06] to stretched decay (model)'));

  const quoteBtn = popover.querySelector('.ui-quote-chip-open');
  assert.ok(quoteBtn);
  quoteBtn.click();
  assert.equal(quoteOpens.length, 1);
  assert.equal(quoteOpens[0].start_line, 14);

  // Pin toggle flips main_result -> not_main_result.
  const pinBtn = host.querySelector('.inv-v7-pin-btn');
  assert.equal(pinBtn.textContent, UNPIN_ACTION_LABEL);
  pinBtn.click();
  assert.deepEqual(pinCalls, [{ id: 'n-thm1111111', nextPin: 'not_main_result' }]);
  assert.ok(host.textContent.includes(MAIN_RESULT_MODEL_LABEL));

  // Verify user-pinned display and no-cue wording on another node.
  const noCueHost = dom.makeEl('div');
  renderInvestigationBadges(noCueHost, {
    id: 'n-lem2222222',
    pin: 'main_result',
    pinned: true,
    authors_claim: { label: 'background', cues: [], quotes: [] },
  }, { initialPopoverOpen: true, onTogglePin: () => {} });
  assert.ok(noCueHost.textContent.includes(MAIN_RESULT_USER_LABEL));
  assert.ok(noCueHost.textContent.includes(NO_EXPLICIT_CUE_TEXT));
  assert.equal(formatHeadingCiteLine(['K1', 'K2', 'K1']), 'cites [K1, K2] in its heading');

  // Unpinned non-main node shows "Pin as main result".
  const unpinHost = dom.makeEl('div');
  renderInvestigationBadges(unpinHost, { id: 'n-lem3333333', role: 'supporting' }, { onTogglePin: () => {} });
  assert.equal(unpinHost.querySelector('.inv-v7-pin-btn').textContent, PIN_ACTION_LABEL);

  // Wording property & XSS check.
  const xssHost = dom.makeEl('div');
  renderInvestigationBadges(xssHost, {
    id: 'n-xss',
    head_cites: ['<img src=x onerror=alert(1)>'],
    authors_claim: {
      label: 'restated',
      relation_note: '<script>alert(2)</script>',
      cues: [],
      quotes: [{ source_id: 's1', path: 'a.tex', start_line: 1, end_line: 1, quote: '<svg onload=alert(3)>' }],
    },
  }, { initialPopoverOpen: true });
  assert.equal(dom.innerHtmlWrites, 0, 'zero innerHTML writes');
  assert.equal(xssHost.querySelectorAll('script').length, 0);
  assert.equal(xssHost.querySelectorAll('img').length, 0);

  const allText = collectAllStrings(host).concat(collectAllStrings(noCueHost)).join('\n').toLowerCase();
  assert.ok(!allText.includes('verified'), 'Investigate UI must never show "verified"');
  assert.ok(!allText.includes('branch'), 'New Investigate UI drops the word "branch"');
  assert.ok(ctrl);
});

test('V7 enriched mapview automatically activates V13 Main results and Authors label filters', () => {
  const mv = {
    main_result_tags: ['n-thm1'],
    nodes: [
      {
        id: 'n-thm1',
        kind: 'theorem',
        label: 'Theorem 1',
        group: 'Section 1',
        role: 'main_result',
        effective_is_main_result: true,
        authors_claim: { label: 'new', text: AUTHORS_CLAIM_LABELS.new },
      },
      {
        id: 'n-lem2',
        kind: 'lemma',
        label: 'Lemma 2',
        group: 'Section 1',
        role: 'main_result',
        pin: 'not_main_result',
        pinned: false,
        unpinned: true,
        effective_is_main_result: false,
        authors_claim: { label: 'restated', text: AUTHORS_CLAIM_LABELS.restated },
      },
    ],
    edges: [],
    skip_plan: { empty: false, main_result_tags: ['n-thm1'], basis: 'explicit', sections: [] },
  };
  const caps = inspectMapviewCapabilities(mv);
  assert.equal(caps.hasMainResults, true);
  assert.equal(caps.hasAuthorsClaims, true);
  assert.deepEqual(caps.availableAuthorsLabels, ['new', 'restated']);
  assert.equal(isMainResultNode(mv.nodes[0], caps.mainResultIds), true);
  assert.equal(isMainResultNode(mv.nodes[1], caps.mainResultIds), false, 'reader pin=not_main_result overrides model role');
  assert.equal(extractAuthorsClaimLabel(mv.nodes[0]), 'new');
  assert.equal(extractAuthorsClaimLabel(mv.nodes[1]), 'restated');
  assert.equal(resolveEffectiveMainResult({ role: 'main_result', effective_is_main_result: false, stale: true }), false);
});

test('V7 api.js exports and investigation_node mounts .inv-node-badges-slot cleanly without V8/V10', () => {
  for (const fn of ['startInvestigationTask', 'saveInvestigationNote', 'getInvestigateSettings', 'putInvestigateSettings']) {
    assert.equal(typeof apiExports[fn], 'function', `api.js must export ${fn}`);
  }
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const host = dom.makeEl('div');
  const pinCalls = [];
  const quoteCalls = [];
  const pageCalls = [];
  const mv = {
    nodes: [{
      id: 'n-thm1',
      kind: 'theorem',
      label: 'Theorem 1: Main',
      fp: 'fp-1',
      role: 'main_result',
      effective_is_main_result: true,
      page_basis: 'scan_text_layer',
      head_cites: ['Toy95'],
      authors_claim: {
        label: 'new',
        text: AUTHORS_CLAIM_LABELS.new,
        cues: ['we prove'],
        quotes: [{ source_id: 'src-1', path: 'paper.pdf.txt', start_line: 3, end_line: 4, page: 2, quote: 'We prove...' }],
      },
      spans: [{ path: 'paper.pdf.txt', start_line: 3, end_line: 5 }],
    }],
    edges: [],
    tasks: [],
  };
  const ctrl = renderInvestigationNodeCard(host, mv, 'n-thm1', {
    nodeText: 'Statement text.',
    modelConnected: true,
    onTogglePin: (n, pin) => pinCalls.push({ id: n.id, pin }),
    onOpenQuote: (q) => quoteCalls.push(q),
    onOpenPageImage: (q) => pageCalls.push(q),
  });
  assert.ok(ctrl.badgesCtrl);
  assert.ok(host.querySelector('.inv-node-badges-slot'));
  assert.ok(host.querySelector('.inv-v7-badges'));
  assert.equal(host.querySelector('.inv-v8-model'), null);
  assert.equal(host.querySelector('.inv-v10-notes'), null);
  ctrl.badgesCtrl.togglePopover(true);
  host.querySelector('.ui-quote-chip-open').click();
  assert.equal(pageCalls.length, 1, 'OCR quote chip invokes onOpenPageImage');
  assert.equal(quoteCalls.length, 0);
  host.querySelector('.inv-v7-pin-btn').click();
  assert.deepEqual(pinCalls, [{ id: 'n-thm1', pin: 'not_main_result' }]);
  assert.equal(dom.innerHtmlWrites, 0);
});

async function loadShellController() {
  const shellUrl = new URL('../assets/investigation_v3_shell.js', import.meta.url);
  const src = await readFile(shellUrl, 'utf8');
  const rewritten = src.replace(/(['"])\/assets\/([^'"]+)\1/g, (_, _q, file) =>
    JSON.stringify(new URL(`../assets/${file}`, import.meta.url).href));
  const dataUrl = `data:text/javascript;base64,${Buffer.from(rewritten, 'utf8').toString('base64')}`;
  return import(dataUrl);
}

test('V7 investigation_v3_shell wires orient_auto_start, Orient button, pin toggle, quote open, and safe ask/note', async () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = { addEventListener() {}, removeEventListener() {} };

  const shellHost = dom.makeEl('div');
  shellHost.id = 'investigation-v3-shell';
  const composerInput = dom.makeEl('textarea');
  composerInput.id = 'composer-input';

  const { createInvestigationV3Controller } = await loadShellController();

  const taskCalls = [];
  const noteCalls = [];
  const openSourceCalls = [];
  let refreshCount = 0;
  let mvFetchCount = 0;
  let currentMv = {
    etag: 'h1',
    skeleton_seq: 4,
    source_id: 'src-1',
    orient_auto_start: true,
    tasks: [],
    nodes: [{
      id: 'n-thm1',
      kind: 'theorem',
      label: 'Theorem 1: Main',
      fp: 'fp-1',
      role: 'main_result',
      effective_is_main_result: true,
      head_cites: ['Toy95'],
      authors_claim: {
        label: 'new',
        text: AUTHORS_CLAIM_LABELS.new,
        cues: ['we prove'],
        quotes: [{ source_id: 'src-1', path: 'main.tex', start_line: 10, end_line: 11, quote: 'We prove main bound.' }],
      },
      spans: [{ path: 'main.tex', start_line: 10, end_line: 12 }],
    }],
    edges: [],
    bib: [],
    symbols: [],
  };

  const ctrl = createInvestigationV3Controller({
    api: {
      getInvestigationMapview: async () => { mvFetchCount += 1; return currentMv; },
      getInvestigationNodeText: async () => ({ text: 'Statement line.' }),
      startInvestigationTask: async (sid, payload) => { taskCalls.push({ sid, payload }); return { status: 'running' }; },
      saveInvestigationNote: async (sid, payload) => { noteCalls.push({ sid, payload }); return { seq: 7, ...payload }; },
    },
  });

  const sessionData = {
    mode: 'investigation',
    investigation: { source_id: 'src-1', skeleton_seq: 4 },
    sources: [{ source_id: 'src-1', format: 'tex' }],
    events: [{ seq: 4 }],
  };
  const hooks = {
    isModelConnected: () => true,
    openSource: (sid, target) => openSourceCalls.push({ sid, target }),
    refresh: async () => { refreshCount += 1; await ctrl.syncSession('s1', sessionData, hooks); },
  };

  const ok = await ctrl.syncSession('s1', sessionData, hooks);
  assert.equal(ok, true);
  await new Promise((r) => setTimeout(r, 10));

  // orient_auto_start fires once when modelConnected=true and no prior orient task exists.
  assert.equal(taskCalls.length, 1);
  assert.deepEqual(taskCalls[0], { sid: 's1', payload: { task: 'orient' } });
  assert.ok(refreshCount >= 1);
  assert.ok(mvFetchCount >= 2, 'refreshMv invalidates activeKey so updated mapview loads');

  // Manual "Orient paper" button starts orient.
  const orientBtn = shellHost.querySelector('[data-action="orient"]');
  assert.ok(orientBtn);
  assert.equal(orientBtn.disabled, false);
  orientBtn.click();
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(taskCalls.length, 2);

  // Pin toggle sends saveInvestigationNote and refreshes.
  const pinBtn = shellHost.querySelector('.inv-v7-pin-btn');
  assert.ok(pinBtn);
  pinBtn.click();
  await new Promise((r) => setTimeout(r, 10));
  assert.deepEqual(noteCalls, [{ sid: 's1', payload: { tag: 'n-thm1', node_fp: 'fp-1', pin: 'not_main_result' } }]);

  // Quote chip in popover opens source via hooks.openSource.
  shellHost.querySelector('[data-half="left"]').click();
  shellHost.querySelector('.ui-quote-chip-open').click();
  assert.equal(openSourceCalls.length, 1);
  assert.equal(openSourceCalls[0].target.start_line, 10);

  // Safe ask/note actions focus composer without V10 mounted.
  const noteActionBtn = shellHost.querySelector('[data-action="note"]');
  noteActionBtn.click();
  assert.ok(composerInput.value.includes('n-thm1'));
  assert.equal(composerInput._focused, true);

  // When orient_auto_start is false or model disconnected, auto-start does not trigger.
  currentMv = { ...currentMv, orient_auto_start: false };
  const callsBefore = taskCalls.length;
  await ctrl.syncSession('s2', { ...sessionData, events: [{ seq: 1 }] }, { ...hooks, isModelConnected: () => false });
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(taskCalls.length, callsBefore);
  assert.equal(shellHost.querySelector('[data-action="orient"]').disabled, true);
  assert.equal(dom.innerHtmlWrites, 0);
});

test('V7 badges render ocr page_basis, node.gist (model), and changed_since_last badge', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  const slot = dom.makeEl('div');
  const pageImgs = [];
  renderInvestigationBadges(slot, {
    id: 'n-ocr',
    page_basis: 'ocr',
    gist: 'Spectral gap bound holds under exponential damping.',
    changed_since_last: { label: 'Changed since you last opened this', changes: [{ field: 'gist' }] },
    effective_is_main_result: true,
    authors_claim: {
      present: true,
      label: 'new',
      text: 'Authors: New (model)',
      cue_line: 'Cues in quoted lines: we prove',
      quotes: [{ quote: 'We prove positive gap.', source_id: 's1', path: 'p.txt', start_line: 12, end_line: 12, page: 4 }],
    },
  }, { onOpenPageImage: (t) => pageImgs.push(t), initialPopoverOpen: true });

  const text = slot.textContent;
  assert.match(text, /Changed since you last opened this/);
  assert.match(text, /Spectral gap bound holds under exponential damping\. \(model\)/);
  assert.match(text, /matched in OCR text, p\. 4/);
  assert.doesNotMatch(text, /\bverified\b/i);
  slot.querySelector('.ui-quote-chip-open').click();
  assert.equal(pageImgs.length, 1);
  assert.equal(pageImgs[0].openPageImage, true);
  assert.equal(dom.innerHtmlWrites, 0);
});

test('V7 Orient UI renders RunBar (Paused not Partial), model choices, Auto-orient toggle, and model watcher', async () => {
  const dom = makeFakeDom();
  globalThis.window = { addEventListener() {}, removeEventListener() {} };
  const prevGetById = dom.document.getElementById;
  const composerModel = dom.makeEl('select');
  composerModel.hidden = true;
  composerModel.value = '__none__';
  const composerConnect = dom.makeEl('button');
  composerConnect.hidden = false;
  const setupEl = dom.makeEl('div');
  const shellHost = dom.makeEl('div');
  const byId = {
    'investigation-v3-shell': shellHost, 'investigation-setup': setupEl,
    'composer-model': composerModel, 'composer-connect': composerConnect,
  };
  dom.document.getElementById = (id) => byId[id] || prevGetById?.(id) || null;
  globalThis.document = dom.document;

  const taskCalls = [];
  const stopCalls = [];
  const settingCalls = [];
  let mv = {
    etag: 'h-v7-ui',
    source_id: 'src1', skeleton_seq: 1, builder: 'tex-1', orient_auto_start: true,
    nodes: [{ id: 'n1', kind: 'theorem', label: 'Thm 1', order: 0, fp: 'fp1' }],
    tasks: [{ task: 'orient', run_state: 'partial', reason: 'step cap reached', model_calls: { used: 8, cap: 8 } }],
    edges: [], bib: [], symbols: [],
  };
  const apiStub = {
    async getInvestigationMapview() { return mv; },
    async getInvestigationNodeText() { return { text: 'Thm body.' }; },
    async startInvestigationTask(id, body) { taskCalls.push({ id, body }); return {}; },
    async stopTurn(id) { stopCalls.push(id); return {}; },
    async putInvestigateSettings(body) { settingCalls.push(body); return body; },
  };
  const { createInvestigationV3Controller } = await loadShellController();
  const ctrl = createInvestigationV3Controller({ api: apiStub });
  const isModelConnected = () => !composerModel.hidden && composerModel.value !== '__none__' && composerConnect.hidden;
  const hooks = {
    isModelConnected,
    getTaskModelOptions: () => ({ provider: 'openai', model: 'gpt-5.4' }),
    refresh: async () => {},
  };
  const sessionData = { mode: 'investigation', investigation: { source_id: 'src1', skeleton_seq: 1 }, events: [{ seq: 1 }] };
  await ctrl.syncSession('s3', sessionData, hooks);
  assert.match(shellHost.textContent, /Paused: step cap reached/);
  assert.doesNotMatch(shellHost.textContent, /\bPartial\b/i);
  shellHost.querySelector('[data-run-action="continue"]').click();
  assert.deepEqual(taskCalls[0], { id: 's3', body: { task: 'orient', provider: 'openai', model: 'gpt-5.4' } });

  // Toggle Auto-orient checkbox -> persists via putInvestigateSettings.
  const autoBox = shellHost.querySelector('[data-setting="orient_auto_start"]');
  assert.equal(autoBox.checked, true);
  autoBox.checked = false;
  await autoBox.dispatch('change');
  assert.deepEqual(settingCalls, [{ orient_auto_start: false }]);

  // Running orient disables Orient button and Stop button invokes stopTurn.
  mv = { ...mv, tasks: [{ task: 'orient', run_state: 'running', model_calls: { used: 1, cap: 8 } }] };
  await ctrl.syncSession('s3', { ...sessionData, events: [{ seq: 2 }] }, hooks);
  assert.equal(shellHost.querySelector('[data-action="orient"]').disabled, true);
  shellHost.querySelector('[data-run-action="stop"]').click();
  assert.deepEqual(stopCalls, ['s3']);

  // Boot race: initial sync while model disconnected -> auto-starts once model connects via change.
  mv = { ...mv, orient_auto_start: true, tasks: [] };
  composerModel.hidden = true;
  composerConnect.hidden = false;
  await ctrl.syncSession('s4', { ...sessionData, events: [{ seq: 3 }] }, hooks);
  assert.equal(taskCalls.length, 1);
  assert.equal(shellHost.querySelector('[data-action="orient"]').disabled, true);
  composerModel.hidden = false;
  composerModel.value = 'openai::gpt-5.4';
  composerConnect.hidden = true;
  composerModel.dispatch('change');
  assert.equal(taskCalls.length, 2);
  assert.equal(taskCalls[1].id, 's4');
  assert.equal(dom.innerHtmlWrites, 0);
});
