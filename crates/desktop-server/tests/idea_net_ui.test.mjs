import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ASSETS_DIR = path.resolve(__dirname, '../assets');

function installMiniDom() {
  class MiniEl {
    constructor(tag) {
      this.tagName = String(tag).toUpperCase();
      this.className = '';
      this.children = [];
      this.attrs = new Map();
      this.listeners = new Map();
      this._text = '';
      this.type = '';
      this.checked = false;
      this.disabled = false;
    }
    get textContent() {
      if (this.children.length === 0) return this._text;
      return this._text + this.children.map((c) => c.textContent).join('');
    }
    set textContent(v) {
      this._text = String(v ?? '');
      this.children = [];
    }
    set innerHTML(_v) {
      throw new Error('XSS VIOLATION: innerHTML must never be written');
    }
    append(...nodes) {
      for (const n of nodes) {
        if (n == null) continue;
        if (typeof n === 'string') {
          const span = new MiniEl('span');
          span.textContent = n;
          this.children.push(span);
        } else {
          this.children.push(n);
        }
      }
    }
    appendChild(n) {
      this.append(n);
      return n;
    }
    setAttribute(k, v) {
      this.attrs.set(String(k), String(v));
    }
    getAttribute(k) {
      return this.attrs.has(k) ? this.attrs.get(k) : null;
    }
    addEventListener(evt, fn) {
      const list = this.listeners.get(evt) || [];
      list.push(fn);
      this.listeners.set(evt, list);
    }
    click() {
      for (const fn of this.listeners.get('click') || []) fn({ target: this });
    }
    findAll(pred, acc = []) {
      if (pred(this)) acc.push(this);
      for (const c of this.children) c.findAll(pred, acc);
      return acc;
    }
  }
  globalThis.document = {
    createElement: (tag) => new MiniEl(tag),
    createElementNS: (_ns, tag) => new MiniEl(tag),
    createTextNode: (t) => {
      const n = new MiniEl('#text');
      n.textContent = t;
      return n;
    },
  };
}

test('renderCitedSinceControl shows 0 model calls and open-question "cited since" != "answered" note', async () => {
  installMiniDom();
  const {
    renderCitedSinceControl,
    CITED_SINCE_BUTTON_LABEL,
    CITED_SINCE_NOT_ANSWERED_NOTE,
  } = await import('../assets/idea_net_ui.js');

  let clicked = null;
  const openQ = renderCitedSinceControl(
    { node_id: 'q-1', paper_key: 'arxiv:2401.00001', kind: 'open_question', stated_year: 2024 },
    { onCitedSince: (payload) => { clicked = payload; } },
  );
  assert.ok(openQ);
  assert.ok(openQ.textContent.includes('0 model calls'), openQ.textContent);
  assert.ok(openQ.textContent.includes(CITED_SINCE_NOT_ANSWERED_NOTE), openQ.textContent);
  const caveat = openQ.findAll((n) => n.getAttribute('data-cited-since-caveat') === 'true');
  assert.equal(caveat.length, 1);

  const btn = openQ.findAll((n) => n.getAttribute('data-action') === 'cited-since')[0];
  assert.equal(btn.textContent, CITED_SINCE_BUTTON_LABEL);
  btn.click();
  assert.deepEqual(clicked, {
    paper_key: 'arxiv:2401.00001',
    node_id: 'q-1',
    since_year: 2024,
    is_open_question: true,
  });

  // Non-open-question papers omit the open-question caveat.
  const regular = renderCitedSinceControl(
    { key: 'arxiv:2401.00002', year: 2024, isOpenQuestion: false },
    { onCitedSince: () => {} },
  );
  assert.equal(regular.findAll((n) => n.getAttribute('data-cited-since-caveat') === 'true').length, 0);
});

test('renderWhereILooked embeds unread core citations, cited-since runs, and Thorough Round 2 progress safely (XSS + Rule S8)', async () => {
  installMiniDom();
  const { renderWhereILooked } = await import('../assets/idea_search.js');
  const {
    UNREAD_CORE_HEADING,
    ROUND2_HEADING,
    CITED_SINCE_NOT_ANSWERED_NOTE,
  } = await import('../assets/idea_net_ui.js');

  const xssTitle = '<img src=x onerror=alert(1)> Synthetic <script>evil()</script> Paper';
  let citedCall = null;
  let round2Clicked = 0;
  let moreNode = null;

  const panel = renderWhereILooked(
    {
      providers: [{ name: 'inspire', label: 'INSPIRE-HEP', state: 'ok', queries_ok: 2, queries_failed: 0, hits: 3 }],
      library: { hits: 1, owned_hits: 0 },
      citation_expansion: { seeds: 2, hops: 1 },
      citation_net: {
        seeds: ['arxiv:2401.00001', 'arxiv:2401.00002'],
        hops: 1,
        edges_count: 5,
        unread_core_citations: [
          {
            key: 'arxiv:2103.00099',
            title: xssTitle,
            authors: ['<b>Ada Example</b>'],
            year: 2021,
            citation_count: 42,
            cited_by_core_count: 2,
            cited_by_core: ['arxiv:2401.00001', 'arxiv:2401.00002'],
            in_network_rank: 4,
          },
        ],
        cited_since_runs: [
          {
            target_key: 'arxiv:2401.00001',
            node_id: 'q-1',
            since_year: 2024,
            is_open_question: true,
            note: CITED_SINCE_NOT_ANSWERED_NOTE,
            citing_papers: [{ key: 'arxiv:2505.00011', title: xssTitle, year: 2025 }],
          },
        ],
      },
      round2: {
        eligible: true,
        preset: 'thorough',
        preset_label: 'Thorough',
        status: 'stopped_at_cap',
        stopped_at_cap: true,
        reason: 'Query budget cap (36) reached before Thorough Round 2',
        ceilings: { max_queries: 36, max_screened: 80, max_papers: 12, calls_per_paper: 4, max_model_calls: 55 },
        usage: { queries_used: 36, screened_used: 40, papers_fetched: 6, model_calls_used: 18 },
        thin_statements: [
          {
            target_kind: 'node',
            target_id: 'q-1',
            label: xssTitle,
            summary: 'Open question has not had later citations checked yet.',
          },
        ],
      },
      not_searched: ['books and monographs'],
    },
    {
      onCitedSince: (p) => { citedCall = p; },
      onRunRound2: () => { round2Clicked += 1; },
      onSearchMoreNode: (nid) => { moreNode = nid; },
    },
  );

  const text = panel.textContent;
  assert.ok(text.includes('Citation expansion: 2 seeds, 1 hop, 5 citation links'), text);
  assert.ok(text.includes(`${UNREAD_CORE_HEADING} (1)`), text);
  assert.ok(text.includes('cited by 2 core papers'), text);
  assert.ok(text.includes('in-network rank 4'), text);
  assert.ok(text.includes(CITED_SINCE_NOT_ANSWERED_NOTE), text);
  assert.ok(text.includes(ROUND2_HEADING), text);
  assert.ok(text.includes('Paused: budget cap reached'), text);
  assert.ok(text.includes('36/36 queries'), text);
  assert.ok(text.includes('18/55 model calls'), text);
  // XSS payload is preserved strictly as plain text, with zero script/img elements created.
  assert.ok(text.includes(xssTitle), 'XSS text must be rendered literally via textContent');
  assert.equal(panel.findAll((n) => n.tagName === 'SCRIPT' || n.tagName === 'IMG').length, 0);

  // Trigger interactive buttons.
  panel.findAll((n) => n.getAttribute('data-action') === 'cited-since')[0].click();
  assert.equal(citedCall.paper_key, 'arxiv:2103.00099');
  panel.findAll((n) => n.getAttribute('data-action') === 'run-round2')[0].click();
  assert.equal(round2Clicked, 1);
  panel.findAll((n) => n.getAttribute('data-action') === 'search-more-node')[0].click();
  assert.equal(moreNode, 'q-1');
});

test('idea_net_ui.js and idea_search.js have zero innerHTML and zero Rule S8 forbidden words', () => {
  for (const fname of ['idea_net_ui.js', 'idea_search.js']) {
    const raw = fs.readFileSync(path.join(ASSETS_DIR, fname), 'utf8');
    const codeOnly = raw.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');
    for (const sink of ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write']) {
      assert.equal(codeOnly.includes(sink), false, `${fname} must never touch ${sink}`);
    }
    for (const badWord of ['verified', 'novel', 'proved']) {
      assert.equal(
        new RegExp(`\\b${badWord}\\b`, 'i').test(codeOnly),
        false,
        `${fname} violates Rule S8 with forbidden word '${badWord}'`,
      );
    }
  }
});

test('audit M1/M5: cited-since filters pre-cutoff rows, ignores invalid sinceYear, and disables Round 2 while running', async () => {
  installMiniDom();
  const { renderCitedSinceControl, renderCitationNetPanel } = await import('../assets/idea_net_ui.js');

  let payload = null;
  const ctrl = renderCitedSinceControl(
    { key: 'arxiv:2401.00001', since_year: '', isOpenQuestion: false },
    { onCitedSince: (p) => { payload = p; } },
  );
  ctrl.findAll((n) => n.getAttribute('data-action') === 'cited-since')[0].click();
  assert.equal(payload.since_year, undefined);
  assert.equal(ctrl.textContent.includes('since '), false);
  assert.equal(ctrl.textContent.includes('NaN'), false);

  const extras = renderCitationNetPanel(
    {
      citation_expansion: { seeds: 1, hops: 1 },
      citation_net: {
        edges_count: 2,
        cited_since_runs: [
          {
            target_key: 'arxiv:2401.00001',
            since_year: '',
            citing_papers: [
              { key: 'arxiv:2301.00001', title: 'Old Pre-Cutoff Paper', year: 2023, matches_since_year: false },
              { key: 'arxiv:2505.00011', title: 'Post-Cutoff Paper', year: 2025, matches_since_year: true },
            ],
          },
        ],
      },
      round2: {
        eligible: true,
        preset: 'thorough',
        preset_label: 'Thorough',
        status: 'running',
        ceilings: { max_queries: 36, max_screened: 80, max_papers: 12, calls_per_paper: 4, max_model_calls: 55 },
        usage: { queries_used: 4, screened_used: 5, papers_fetched: 1, model_calls_used: 2 },
        thin_statements: [],
      },
    },
    { onRunRound2: () => {} },
  );

  const text = extras.textContent;
  assert.ok(text.includes('Post-Cutoff Paper'), text);
  assert.equal(text.includes('Old Pre-Cutoff Paper'), false, 'matches_since_year:false must be filtered');
  assert.equal(text.includes('NaN'), false, 'empty since_year must never produce NaN');
  assert.equal(text.includes('since )'), false, 'empty since_year must never render blank since label');
  const r2Btn = extras.findAll((n) => n.getAttribute('data-action') === 'run-round2')[0];
  assert.equal(r2Btn.disabled, true, 'Round 2 button must be disabled while status === running');
});

