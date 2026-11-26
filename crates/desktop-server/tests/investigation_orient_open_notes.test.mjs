import assert from 'node:assert/strict';
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
} from '../assets/investigation_badges.js';
import {
  extractAuthorsClaimLabel,
  inspectMapviewCapabilities,
  isMainResultNode,
} from '../assets/investigation_wholemap_model.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
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
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, stopPropagation() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      focus() { this._focused = true; },
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
});
