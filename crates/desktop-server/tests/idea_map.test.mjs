// Node tests for package X12: Map & Verdict UI over F6 graph_view.js, ui_badge.js, and quote_chip.js.
// Covers Rule V6 (all 3 judged_* confirmations + reject), quote_chip OCR/PDF/abstract rendering,
// synchronized grouped list <-> F6 GraphView <-> createNodeCardShell drawer, node flags,
// Rule S4 not_in_current_map visibility, Decision 13 stale banner, XSS safety, and Rule S8.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { renderExploreSession } from '../assets/idea.js';
import {
  CONFIRM_SAME_LABEL,
  NOT_IN_CURRENT_MAP_LABEL,
  REJECT_SAME_LABEL,
  VERDICT_STATUS_CHIPS,
  WHAT_DIFFERS_LABEL,
  adaptExploreGraphView,
  projectVerdictRow,
  renderIdeaMapSection,
  renderVerdictTable,
  resolveVerdictChip,
} from '../assets/idea_map.js';
import { createInitialIdeaState } from '../assets/idea_state.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const RULE_S8_RE = /\b(verified|new|novel|proved)\b/i;

function collectStrings(node, out = []) {
  if (!node) return out;
  if (node.tagName === '#TEXT' && node.textContent) out.push(node.textContent);
  for (const attr of ['title', 'placeholder']) {
    if (node[attr]) out.push(String(node[attr]));
  }
  const aria = typeof node.getAttribute === 'function' ? node.getAttribute('aria-label') : null;
  if (aria) out.push(aria);
  if (node.children?.length) {
    for (const c of node.children) collectStrings(c, out);
  } else if (node.tagName !== '#TEXT' && node.textContent) {
    out.push(node.textContent);
  }
  return out;
}

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    return {
      tagName: String(tag).toUpperCase(),
      parentNode: null, children: [], style: {}, type: '', title: '', value: '', placeholder: '',
      disabled: false, hidden: false, tabIndex: 0, clientWidth: 800, clientHeight: 500,
      get className() { return [...classes].join(' '); },
      set className(v) { classes.clear(); String(v || '').split(/\s+/).filter(Boolean).forEach((c) => classes.add(c)); },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, force) => (force ? classes.add(c) : classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      get textContent() { return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text; },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      setAttribute(k, v) { attrs.set(String(k), String(v)); if (k === 'class') this.className = String(v); },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      append(...nodes) {
        for (const n of nodes) {
          if (n == null) continue;
          const child = typeof n === 'string' ? Object.assign(makeEl('#text'), { textContent: n }) : n;
          child.parentNode = this;
          this.children.push(child);
        }
      },
      appendChild(n) { this.append(n); return n; },
      replaceChildren(...nodes) { this.children = []; text = ''; this.append(...nodes); },
      addEventListener(evt, fn) { listeners.set(evt, [...(listeners.get(evt) || []), fn]); },
      removeEventListener(evt, fn) { listeners.set(evt, (listeners.get(evt) || []).filter((x) => x !== fn)); },
      dispatch(evt, evObj = {}) {
        const event = { target: this, preventDefault() {}, stopPropagation() {}, ...evObj };
        for (const fn of listeners.get(evt) || []) fn(event);
      },
      click() { this.dispatch('click'); },
      findAll(pred, acc = []) { if (pred(this)) acc.push(this); for (const c of this.children) c.findAll(pred, acc); return acc; },
      findOne(pred) { return this.findAll(pred)[0] || null; },
    };
  };
  const prevDocument = globalThis.document;
  globalThis.document = {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    createTextNode: (t) => Object.assign(makeEl('#text'), { textContent: t }),
  };
  return { get innerHtmlWrites() { return innerHtmlWrites; }, makeEl, restore() { globalThis.document = prevDocument; } };
}

function sampleMapView(overrides = {}) {
  return {
    statement: { rev: 1, text: 'Lanczos coefficients grow linearly in volume-law and saturate in area-law.' },
    sharpened_idea: {
      subclaims: [
        { id: 'sc-1', text: 'Linear Lanczos growth', idea_quote: 'Lanczos coefficients grow linearly' },
        { id: 'sc-2', text: 'Area-law saturation', idea_quote: 'saturate in area-law' },
        { id: 'sc-3', text: 'Order parameter change', idea_quote: 'in volume-law' },
        { id: 'sc-4', text: 'Abstract hint part', idea_quote: 'area-law' },
        { id: 'sc-5', text: 'Open gap part', idea_quote: 'Lanczos coefficients' },
        { id: 'sc-6', text: 'Incomplete scope part', idea_quote: 'volume-law' },
      ],
    },
    evidence: [
      {
        id: 'xe-10',
        seq: 10,
        paper_key: 'arxiv:2405.01111',
        source_id: 'src-1',
        path: 'paper.pdf.txt',
        start_line: 212,
        end_line: 219,
        page: 7,
        text_basis: 'pdf_text',
        ocr_tag: null,
        quote: 'Lanczos coefficients grow linearly across the synthetic volume-law regime.',
        gist: 'Linear Lanczos growth',
      },
      {
        id: 'xe-11',
        seq: 11,
        paper_key: 'doi:10.5555/scan1988',
        source_id: 'src-2',
        path: 'scan.pdf.txt',
        start_line: 40,
        end_line: 45,
        page: 3,
        text_basis: 'scan_text_layer',
        ocr_tag: 'OCR',
        quote: 'Spread complexity saturation occurs in the synthetic scanned benchmark.',
        gist: 'Scanned spread complexity saturation',
      },
    ],
    abstract_evidence: [
      {
        id: 'xa-12',
        seq: 12,
        paper_key: 'arxiv:2405.02222',
        quote: 'Related abstract discussing synthetic area-law saturation bounds.',
        provider: 'arxiv',
      },
    ],
    verdicts: {
      map_seq: 20,
      verdicts: [
        {
          subclaim_id: 'sc-1',
          status: 'judged_same',
          confirmed_by_user: false,
          chip: 'Model judges: same',
          text: 'Model judges: same in arxiv:2405.01111.',
          overlaps: [{ paper_key: 'arxiv:2405.01111', relation: 'same', evidence_ids: ['xe-10'], difference: 'Identical free-fermion geometry' }],
        },
        {
          subclaim_id: 'sc-2',
          status: 'judged_partial',
          confirmed_by_user: false,
          chip: 'Model judges: partly the same',
          text: 'Model judges: partly the same in arxiv:2405.01111.',
          overlaps: [{ paper_key: 'arxiv:2405.01111', relation: 'partial', evidence_ids: ['xe-10'], difference: 'Non-Hermitian drift instead of projective monitoring' }],
        },
        {
          subclaim_id: 'sc-3',
          status: 'judged_other_name',
          confirmed_by_user: false,
          chip: 'Model judges: same under another name',
          text: 'Model judges: same under another name in doi:10.5555/scan1988.',
          overlaps: [{
            paper_key: 'doi:10.5555/scan1988',
            relation: 'other_name',
            evidence_ids: ['xe-11'],
            difference: 'Uses spread complexity notation on scanned pages',
            other_name: 'spread complexity saturation',
          }],
        },
        {
          subclaim_id: 'sc-4',
          status: 'abstract_only',
          confirmed_by_user: false,
          chip: 'possibly related (abstracts only)',
          text: 'Matched abstract quote indicates possible relevance only.',
          overlaps: [],
          abstract_matches: [{ evidence_id: 'xa-12', paper_key: 'arxiv:2405.02222', quote: 'Related abstract discussing synthetic area-law saturation bounds.' }],
        },
        {
          subclaim_id: 'sc-5',
          status: 'none_in_scope',
          confirmed_by_user: false,
          chip: 'not found in the searched scope',
          text: 'Not found in 18 queries on arXiv and INSPIRE-HEP.',
          overlaps: [],
        },
        {
          subclaim_id: 'sc-6',
          status: 'incomplete',
          confirmed_by_user: false,
          chip: 'not found (incomplete search)',
          text: 'Search incomplete because Semantic Scholar was rate-limited.',
          overlaps: [],
        },
      ],
    },
    where_i_looked: {
      providers: [{ name: 'semantic_scholar', label: 'Semantic Scholar', state: 'rate_limited' }],
    },
    map: {
      idea_rev: 1,
      nodes: [
        {
          id: 'k-1',
          kind: 'known_result',
          label: 'Linear Lanczos growth',
          statement: 'Linear growth in the synthetic non-Hermitian chain.',
          evidence: ['xe-10'],
          relation_to_idea: 'partial',
          papers: ['arxiv:2405.01111'],
          year: 2024,
          status: 'active',
          display_grade: 'matched_quote_full_text',
          hypotheses: ['Quadratic fermion chain'],
          regime: 'volume-law',
        },
        {
          id: 'c-1',
          kind: 'concept',
          label: 'Spread complexity',
          statement: 'Alternative name from a scanned reference.',
          evidence: ['xe-11'],
          relation_to_idea: 'same',
          papers: ['doi:10.5555/scan1988'],
          year: 1988,
          status: 'active',
          display_grade: 'matched_quote_scanned_page',
          ocr_tag: 'OCR',
          aliases: ['Krylov spread'],
        },
        {
          id: 'w-1',
          kind: 'related_work',
          label: 'Abstract-only related note',
          statement: 'Abstract mention of area-law bounds.',
          evidence: ['xa-12'],
          relation_to_idea: 'background',
          papers: ['arxiv:2405.02222'],
          paper_key: 'arxiv:2405.02222',
          status: 'active',
          display_grade: 'matched_quote_abstract',
        },
        {
          id: 'k-9',
          kind: 'known_result',
          label: 'Earlier noted node kept by Rule S4',
          statement: 'Previously pinned by the reader before re-map.',
          evidence: ['xe-10'],
          relation_to_idea: 'background',
          papers: ['arxiv:2405.01111'],
          status: 'not_in_current_map',
          display_grade: 'matched_quote_full_text',
        },
      ],
      edges: [
        { src: 'c-1', dst: 'k-1', from: 'k-1', to: 'c-1', kind: 'uses', role: 'uses', basis: 'evidence', style: 'solid' },
        { src: 'w-1', dst: 'k-1', from: 'k-1', to: 'w-1', kind: 'uses', role: 'uses', basis: 'model', style: 'dashed' },
      ],
      legend: [
        { kind: 'known_result', prefix: 'k-', label: 'Known result' },
        { kind: 'concept', prefix: 'c-', label: 'Concept' },
        { kind: 'related_work', prefix: 'w-', label: 'Related work' },
      ],
    },
    map_stale: false,
    notes: [
      { seq: 21, target_kind: 'node', target_id: 'k-9', action: 'pin', comment: '' },
    ],
    ...overrides,
  };
}

test('idea_same_needs_user_confirmation and all three judged_* statuses transition on confirm/reject (Rule V6)', () => {
  const dom = makeFakeDom();
  try {
    // Pure chip transitions across all three judged_* statuses.
    assert.equal(resolveVerdictChip('judged_same', false).text, VERDICT_STATUS_CHIPS.judged_same_unconfirmed);
    assert.equal(resolveVerdictChip('judged_same', false).border, 'dashed');
    assert.equal(resolveVerdictChip('judged_same', true).text, VERDICT_STATUS_CHIPS.judged_same_confirmed);
    assert.equal(resolveVerdictChip('judged_same', true).border, 'solid');

    assert.equal(resolveVerdictChip('judged_partial', false).text, VERDICT_STATUS_CHIPS.judged_partial_unconfirmed);
    assert.equal(resolveVerdictChip('judged_partial', true).text, VERDICT_STATUS_CHIPS.judged_partial_confirmed);

    assert.equal(resolveVerdictChip('judged_other_name', false).text, VERDICT_STATUS_CHIPS.judged_other_name_unconfirmed);
    assert.equal(resolveVerdictChip('judged_other_name', true).text, VERDICT_STATUS_CHIPS.judged_other_name_confirmed);

    assert.equal(resolveVerdictChip('abstract_only', false).text, 'possibly related (abstracts only)');
    assert.equal(resolveVerdictChip('none_in_scope', false).text, 'not found in the searched scope');
    assert.equal(resolveVerdictChip('incomplete', false).text, 'not found (incomplete search)');

    const notesSent = [];
    const openedSources = [];
    const openedPageImages = [];
    const retried = [];
    const view = sampleMapView();
    const tableEl = renderVerdictTable(view, {
      onNote: (p) => notesSent.push(p),
      onOpenSource: (t) => openedSources.push(t),
      onOpenPageImage: (t) => openedPageImages.push(t),
      onRetryService: (svc) => retried.push(svc),
    });

    const text = tableEl.textContent;
    assert.ok(text.includes(WHAT_DIFFERS_LABEL));
    assert.ok(text.includes('Model judges: same'));
    assert.ok(text.includes('Model judges: partly the same'));
    assert.ok(text.includes('Model judges: same under another name'));
    assert.ok(text.includes('possibly related (abstracts only)'));
    assert.ok(text.includes('not found in the searched scope'));
    assert.ok(text.includes('not found (incomplete search)'));
    assert.ok(text.includes('p. 7'), 'PDF page link must appear on full-text quote chip');
    assert.ok(text.includes('OCR'), 'OCR chip must appear on scanned quote');
    assert.ok(text.includes('abstract only'), 'abstract-only quote chip must appear');

    // Click "Looks the same to me" on all three judged_* rows -> posts confirm_same.
    const confirmBtns = tableEl.findAll((n) => n.getAttribute('data-action') === 'confirm-same');
    assert.equal(confirmBtns.length, 3, 'All 3 judged_* rows must expose Looks the same to me');
    for (const btn of confirmBtns) {
      assert.equal(btn.textContent, CONFIRM_SAME_LABEL);
      btn.click();
    }
    assert.deepEqual(notesSent, [
      { target_kind: 'verdict', target_id: 'sc-1', action: 'confirm_same', map_seq: 20 },
      { target_kind: 'verdict', target_id: 'sc-2', action: 'confirm_same', map_seq: 20 },
      { target_kind: 'verdict', target_id: 'sc-3', action: 'confirm_same', map_seq: 20 },
    ]);

    // Click "Not the same" on sc-2 -> posts reject_same with map_seq.
    const rejSc2 = tableEl.findOne(
      (n) => n.getAttribute('data-action') === 'reject-same' && n.getAttribute('data-subclaim-id') === 'sc-2',
    );
    assert.equal(rejSc2.textContent, REJECT_SAME_LABEL);
    rejSc2.click();
    assert.deepEqual(notesSent.at(-1), { target_kind: 'verdict', target_id: 'sc-2', action: 'reject_same', map_seq: 20 });

    // Quote chip click routing: full-text opens source (p. 7), OCR opens page image (p. 3).
    const openBtns = tableEl.findAll((n) => n.className.includes('ui-quote-chip-open'));
    assert.ok(openBtns.length >= 3);
    openBtns[0].click();
    assert.equal(openedSources.length, 1);
    assert.equal(openedSources[0].page, 7);
    const ocrChip = tableEl.findOne((n) => n.getAttribute('data-ocr') === 'true');
    assert.ok(ocrChip, 'OCR quote chip must carry data-ocr=true');
    ocrChip.findOne((n) => n.className.includes('ui-quote-chip-open')).click();
    assert.equal(openedPageImages.length, 1);
    assert.equal(openedPageImages[0].page, 3);
    assert.equal(openedPageImages[0].openPageImage, true);

    // Retry service button on incomplete row.
    const retryBtn = tableEl.findOne((n) => n.getAttribute('data-action') === 'retry-service');
    assert.ok(retryBtn);
    retryBtn.click();
    assert.deepEqual(retried, ['semantic_scholar']);

    // Re-render after user confirmation: chips flip & Rule S8 holds on confirmed table.
    const confirmedView = sampleMapView();
    confirmedView.verdicts.verdicts[0].confirmed_by_user = true;
    confirmedView.verdicts.verdicts[1].confirmed_by_user = true;
    confirmedView.verdicts.verdicts[2].confirmed_by_user = true;
    const confirmedEl = renderVerdictTable(confirmedView, {});
    assert.ok(confirmedEl.textContent.includes('found (same)'));
    assert.ok(confirmedEl.textContent.includes('found (partly)'));
    assert.ok(confirmedEl.textContent.includes('found under another name'));
    assert.equal(RULE_S8_RE.test(collectStrings(confirmedEl).join(' ')), false);

    // Reject note bound to active map_seq=20 renders "Marked not the same (you)"; older map_seq=5 is ignored.
    const rejView = sampleMapView({
      notes: [
        { seq: 6, target_kind: 'verdict', target_id: 'sc-1', action: 'reject_same', map_seq: 5 },
        { seq: 22, target_kind: 'verdict', target_id: 'sc-2', action: 'reject_same', map_seq: 20 },
      ],
    });
    const rejEl = renderVerdictTable(rejView, {});
    const sc1Row = rejEl.findOne((n) => n.getAttribute('data-subclaim-id') === 'sc-1');
    const sc2Row = rejEl.findOne((n) => n.getAttribute('data-subclaim-id') === 'sc-2');
    assert.equal(sc1Row.textContent.includes('Marked not the same'), false, 'Stale map_seq=5 rejection must not leak');
    assert.equal(sc2Row.textContent.includes('Marked not the same (you)'), true);

    const proj = projectVerdictRow(
      view.verdicts.verdicts[0],
      new Map(),
      new Map(),
      new Map([['sc-1', 'reject_same']]),
    );
    assert.equal(proj.rejectedByUser, true);
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('concept map syncs grouped list, F6 GraphView, createNodeCardShell drawer, flags, Rule S4 & Decision 13 stale banner', () => {
  const dom = makeFakeDom();
  try {
    const notesSent = [];
    const remaps = [];
    const investigated = [];
    const view = sampleMapView({
      statement: { rev: 3, text: 'Revised Lanczos idea' },
      map_stale: true,
    });
    view.map.nodes.push(
      {
        id: 'c-2', kind: 'concept', label: 'Unbacked model guess', statement: 'Model-only concept note.',
        evidence: [], relation_to_idea: 'analogue', papers: [], status: 'active', display_grade: 'model_statement',
      },
      {
        id: 'c-3', kind: 'concept', label: 'User idea concept', statement: 'Anchored to idea quote.',
        evidence: [], idea_quote: 'Lanczos coefficients', relation_to_idea: 'same', papers: [], status: 'active', display_grade: 'user_statement',
      },
    );

    const adapted = adaptExploreGraphView(view.map, view.notes, { stale: true });
    assert.equal(adapted.nodes.find((n) => n.id === 'k-1').grade, 'quote_fulltext');
    assert.equal(adapted.nodes.find((n) => n.id === 'c-1').grade, 'quote_ocr');
    assert.equal(adapted.nodes.find((n) => n.id === 'w-1').grade, 'quote_abstract');
    assert.equal(adapted.nodes.find((n) => n.id === 'c-2').grade, 'model');
    assert.equal(adapted.nodes.find((n) => n.id === 'c-3').grade, 'user');
    assert.equal(adapted.nodes.find((n) => n.id === 'k-9').dim, true);
    assert.deepEqual(adapted.nodes.find((n) => n.id === 'k-9').flags, ['pin']);
    assert.equal(adapted.nodes.every((n) => n.stale === true), true);
    assert.equal(adapted.legend[0].label, 'k- Known result', 'Legend prefix must include space');

    const root = renderIdeaMapSection(view, {
      onNote: (p) => notesSent.push(p),
      onRemap: (p) => remaps.push(p),
      onInvestigatePaper: (p) => investigated.push(p),
    });

    // Model-grade node in grouped list MUST carry dashed "(model)" badge, never solid "Found".
    const listC2 = root.findOne((n) => n.getAttribute('data-map-list-node') === 'c-2');
    const c2Badge = listC2.findOne((n) => n.className.includes('ui-badge'));
    assert.equal(c2Badge.textContent, '(model)');
    assert.equal(c2Badge.getAttribute('data-border'), 'dashed');
    assert.equal(c2Badge.getAttribute('data-icon'), 'model');
    const listC3 = root.findOne((n) => n.getAttribute('data-map-list-node') === 'c-3');
    assert.equal(listC3.findOne((n) => n.className.includes('ui-badge')).getAttribute('data-icon'), 'user');

    // Decision 13 stale banner with "Re-map against revision 3".
    const staleBanner = root.findOne((n) => n.getAttribute('data-idea-banner') === 'map-stale');
    assert.ok(staleBanner, 'Stale banner must render when statement.rev > map.idea_rev');
    const remapBtn = staleBanner.findOne((n) => n.getAttribute('data-action') === 'remap-stale');
    assert.equal(remapBtn.textContent, 'Re-map against revision 3');
    remapBtn.click();
    assert.deepEqual(remaps, [{ idea_rev: 3, reuse_evidence: true }]);

    // Rule S4: not_in_current_map user-noted node k-9 stays visible in grouped list and graph.
    const listK9 = root.findOne((n) => n.getAttribute('data-map-list-node') === 'k-9');
    assert.ok(listK9 && listK9.textContent.includes(NOT_IN_CURRENT_MAP_LABEL));
    const svgK9 = root.findOne((n) => n.getAttribute('data-node-id') === 'k-9');
    assert.ok(svgK9 && svgK9.getAttribute('data-dim') === 'true');

    // Selecting c-1 in grouped list syncs F6 GraphView + opens drawer; model properties stay in modelBlock.
    const listC1 = root.findOne((n) => n.getAttribute('data-map-list-node') === 'c-1');
    listC1.click();
    assert.equal(listC1.getAttribute('aria-selected'), 'true');
    const drawerCard = root.findOne((n) => n.getAttribute('data-drawer-node-id') === 'c-1');
    assert.ok(drawerCard && drawerCard.textContent.includes('summary (model)') && drawerCard.textContent.includes('OCR'));
    const codeBlock = drawerCard.findOne((n) => n.getAttribute('data-block') === 'code');
    const modelBlock = drawerCard.findOne((n) => n.getAttribute('data-block') === 'model');
    assert.equal(codeBlock.textContent.includes('Aliases'), false, 'Model aliases must not sit inside by-code block');
    assert.equal(modelBlock.textContent.includes('Aliases (model)'), true);
    assert.equal(root.findOne((n) => n.getAttribute('data-node-id') === 'c-1').getAttribute('aria-pressed'), 'true');

    // Selecting w-1 in F6 GraphView syncs back to grouped list and drawer.
    root.findOne((n) => n.getAttribute('data-node-id') === 'w-1').click();
    assert.equal(root.findOne((n) => n.getAttribute('data-map-list-node') === 'w-1').getAttribute('aria-selected'), 'true');
    assert.equal(listC1.getAttribute('aria-selected'), 'false');
    const drawerW1 = root.findOne((n) => n.getAttribute('data-drawer-node-id') === 'w-1');
    assert.ok(drawerW1 && drawerW1.textContent.includes('abstract only'));
    drawerW1.findOne((n) => n.getAttribute('data-action') === 'investigate-paper').click();
    assert.deepEqual(investigated, [{ paper_key: 'arxiv:2405.02222', node_id: 'w-1' }]);

    // Node flag buttons (wrong, not_relevant, pin) + comment validation.
    for (const action of ['wrong', 'not_relevant', 'pin']) {
      drawerW1.findOne((n) => n.getAttribute('data-flag-action') === action).click();
    }
    const cInput = drawerW1.findOne((n) => n.getAttribute('data-node-comment-input') === 'w-1');
    const cBtn = drawerW1.findOne((n) => n.getAttribute('data-flag-action') === 'comment');
    cInput.value = '   ';
    cBtn.click();
    assert.equal(notesSent.length, 3, 'Blank comment must not fire onNote');
    cInput.value = 'Check appendix derivation';
    cBtn.click();
    assert.deepEqual(notesSent, [
      { target_kind: 'node', target_id: 'w-1', action: 'wrong' },
      { target_kind: 'node', target_id: 'w-1', action: 'not_relevant' },
      { target_kind: 'node', target_id: 'w-1', action: 'pin' },
      { target_kind: 'node', target_id: 'w-1', action: 'comment', comment: 'Check appendix derivation' },
    ]);

    // Rule S8 & zero innerHTML sweep across all drawers (including c-1, w-1, k-9) and renderExploreSession.
    for (const nid of ['c-1', 'w-1', 'k-9', 'c-2', 'c-3']) {
      root.findOne((n) => n.getAttribute('data-map-list-node') === nid).click();
      assert.equal(RULE_S8_RE.test(collectStrings(root).join(' ')), false, `Rule S8 on drawer ${nid}`);
    }
    const host = dom.makeEl('div');
    renderExploreSession(host, createInitialIdeaState({ hasModel: true, view }));
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(RULE_S8_RE.test(collectStrings(host).join(' ')), false);
  } finally {
    dom.restore();
  }
});

test('idea_map.js & idea_verdict_ui.js have zero layout code, zero innerHTML, XSS safety, and assets.rs registration', () => {
  const dom = makeFakeDom();
  try {
    const xss = '<img src=x onerror=alert(1)><script>evil()</script>';
    const view = sampleMapView({
      verdicts: {
        map_seq: 20,
        verdicts: [{
          subclaim_id: 'sc-1',
          status: 'judged_same',
          confirmed_by_user: false,
          text: `Scoped note ${xss}`,
          overlaps: [{ paper_key: 'arxiv:2405.01111', relation: 'same', evidence_ids: ['xe-10'], difference: xss }],
        }],
      },
      map: {
        idea_rev: 1,
        nodes: [{
          id: 'k-1', kind: 'known_result', label: xss, statement: xss,
          evidence: ['xe-10'], relation_to_idea: 'same', papers: ['arxiv:2405.01111'],
          status: 'active', display_grade: 'matched_quote_full_text',
        }],
        edges: [],
        legend: [],
      },
    });
    const el = renderIdeaMapSection(view, {});
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(el.findAll((n) => n.tagName === 'IMG' || n.tagName === 'SCRIPT').length, 0);
    assert.ok(el.textContent.includes(xss));
  } finally {
    dom.restore();
  }

  for (const fname of ['idea_map.js', 'idea_verdict_ui.js']) {
    const src = fs
      .readFileSync(path.join(crateRoot, 'assets', fname), 'utf8')
      .replace(/\/\/[^\n]*/g, '');
    assert.equal(src.includes('innerHTML'), false, `${fname} must never use innerHTML`);
    assert.equal(src.includes('layoutGraphView'), false, `${fname} must contain no layout code`);
    assert.equal(src.includes('createElementNS'), false, `${fname} must delegate SVG drawing to F6`);
  }
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  assert.ok(assetsRs.includes('"/assets/idea_map.js"'), 'assets.rs must register /assets/idea_map.js');
  assert.ok(assetsRs.includes('"/assets/idea_verdict_ui.js"'), 'assets.rs must register /assets/idea_verdict_ui.js');
});
