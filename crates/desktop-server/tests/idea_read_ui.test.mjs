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

test('formatReadRowLabel covers every X9/X10 reading-queue row state accurately', async () => {
  const { formatReadRowLabel, READ_ROW_LABELS } = await import('../assets/idea_read_ui.js');
  assert.equal(formatReadRowLabel({ row_state: 'queued' }), 'queued');
  assert.equal(formatReadRowLabel({ row_state: 'fetching' }), 'fetching');
  assert.equal(formatReadRowLabel({ row_state: 'from_library' }), 'from your library');
  assert.equal(
    formatReadRowLabel({ row_state: 'extracting', page: 3, total_pages: 12 }),
    'extracting text (page 3 of 12)',
  );
  assert.equal(formatReadRowLabel({ row_state: 'reading' }), 'reading');
  assert.equal(
    formatReadRowLabel({ row_state: 'read_passages', lines_sent: 410, total_lines: 2300, quote_count: 4 }),
    'read: passages (410 of 2300 lines), 4 quotes',
  );
  assert.equal(formatReadRowLabel({ row_state: 'read_full', quote_count: 0 }), 'read: full');
  assert.equal(formatReadRowLabel({ row_state: 'nothing_relevant' }), 'nothing relevant');
  assert.equal(
    formatReadRowLabel({ row_state: 'unavailable' }),
    'unavailable: not on arXiv and not in your library; attach PDF',
  );
  assert.equal(READ_ROW_LABELS.scan_ocr, 'scan: quotes are from OCR text');
});

test('renderPaperReadRow shows Attach PDF button, OCR tag, and matched quotes without innerHTML', async () => {
  installMiniDom();
  const { renderPaperReadRow } = await import('../assets/idea_read_ui.js');
  const { renderCandidateList } = await import('../assets/idea_search.js');

  let attachedKey = null;
  const unavail = renderPaperReadRow(
    { key: 'doi:10.5555/paywalled', row_state: 'unavailable' },
    { onAttachPdf: (k) => { attachedKey = k; } },
  );
  const btn = unavail.findAll((n) => n.getAttribute('data-action') === 'attach-pdf')[0];
  assert.ok(btn, 'Attach PDF button must be rendered for unavailable paper');
  assert.equal(btn.textContent, 'Attach PDF');
  btn.click();
  assert.equal(attachedKey, 'doi:10.5555/paywalled');

  const scannedCard = renderCandidateList(
    [{ key: 'arxiv:2501.10002', title: 'Synthetic Scanned Paper', authors: ['Ada Example'], year: 2025 }],
    {
      highlightTerms: ['Synthetic'],
      papers: [{
        key: 'arxiv:2501.10002',
        row_state: 'read_passages',
        text_basis: 'scan_text_layer',
        lines_sent: 120,
        total_lines: 600,
        quote_count: 1,
        quotes: [{
          id: 'xe-42',
          page: 3,
          start_line: 45,
          end_line: 47,
          text_basis: 'scan_text_layer',
          ocr_tag: 'OCR',
          quote: 'Lanczos coefficients grow linearly in this synthetic scan.',
          gist: 'Linear Lanczos growth',
        }],
      }],
    },
  );
  const fullText = scannedCard.textContent;
  assert.ok(fullText.includes('read: passages (120 of 600 lines), 1 quote'), fullText);
  assert.ok(fullText.includes('scan: quotes are from OCR text'), fullText);
  assert.ok(fullText.includes('p. 3'), fullText);
  assert.ok(fullText.includes('OCR'), fullText);
  assert.ok(fullText.includes('Linear Lanczos growth (model)'), fullText);

  // Rule S8 & security check on idea_read_ui.js executable code: zero innerHTML and zero forbidden words.
  const src = fs
    .readFileSync(path.join(ASSETS_DIR, 'idea_read_ui.js'), 'utf8')
    .replace(/\/\/[^\n]*/g, '');
  assert.equal(src.includes('innerHTML'), false, 'idea_read_ui.js must never touch innerHTML');
  for (const forbidden of ['"verified"', "'verified'", '"novel"', "'novel'", '"proved"', "'proved'"]) {
    assert.equal(src.toLowerCase().includes(forbidden), false, `Rule S8 forbids ${forbidden}`);
  }
});

test('ideaReducer folds idea.read_progress, idea.paper, and evidence.quote SSE events', async () => {
  const { createInitialIdeaState, ideaReducer } = await import('../assets/idea_state.js');
  let st = createInitialIdeaState({
    sessionId: 's-read-sse',
    view: { papers: [], evidence: [], stages: [] },
  });
  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.read_progress',
    payload: { key: 'arxiv:2501.10002', row_state: 'extracting', page: 2, total_pages: 5 },
  });
  assert.equal(st.view.papers.length, 1);
  assert.equal(st.view.papers[0].row_state, 'extracting');
  assert.equal(st.view.papers[0].page, 2);

  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'idea.paper',
    payload: { key: 'arxiv:2501.10002', status: 'read_passages', text_basis: 'pdf_text', lines_sent: 80, total_lines: 200 },
  });
  assert.equal(st.view.papers[0].row_state, 'read_passages');
  assert.equal(st.view.papers[0].lines_sent, 80);

  st = ideaReducer(st, {
    type: 'APPLY_SSE_EVENT',
    kind: 'evidence.quote',
    payload: { id: 'xe-9', paper_key: 'arxiv:2501.10002', quote: 'Synthetic quote', page: 2 },
  });
  assert.equal(st.view.evidence.length, 1);
  assert.equal(st.view.papers[0].quote_count, 1);
  assert.equal(st.view.papers[0].quotes[0].id, 'xe-9');
});
