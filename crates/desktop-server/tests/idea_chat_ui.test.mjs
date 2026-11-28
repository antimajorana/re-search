import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ASSETS_DIR = path.resolve(__dirname, '../assets');

function installMiniDom() {
  class MiniNode {
    constructor(nodeType, tag, text = '') {
      this.nodeType = nodeType;
      this.tagName = tag ? String(tag).toUpperCase() : '';
      this.className = '';
      this.children = [];
      this.childNodes = this.children;
      this.attrs = new Map();
      this.listeners = new Map();
      this._text = String(text);
      this.nodeValue = nodeType === 3 ? this._text : null;
      this.type = '';
      this.title = '';
      this.disabled = false;
      this.parentNode = null;
    }
    get textContent() {
      if (this.nodeType === 3 || this.children.length === 0) return this._text;
      return this._text + this.children.map((c) => c.textContent).join('');
    }
    set textContent(v) {
      this._text = String(v ?? '');
      this.nodeValue = this.nodeType === 3 ? this._text : null;
      this.children.length = 0;
    }
    append(...nodes) {
      for (const n of nodes) {
        if (n == null) continue;
        const child = typeof n === 'string' ? new MiniNode(3, '', n) : n;
        child.parentNode = this;
        this.children.push(child);
      }
    }
    appendChild(n) {
      this.append(n);
      return n;
    }
    replaceChildren(...nodes) {
      this.children.length = 0;
      this.append(...nodes);
    }
    insertBefore(newNode, refNode) {
      const idx = this.children.indexOf(refNode);
      newNode.parentNode = this;
      if (idx >= 0) this.children.splice(idx, 0, newNode);
      else this.children.push(newNode);
    }
    removeChild(child) {
      const idx = this.children.indexOf(child);
      if (idx >= 0) this.children.splice(idx, 1);
      child.parentNode = null;
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
    createElement: (tag) => new MiniNode(1, tag),
    createElementNS: (_ns, tag) => new MiniNode(1, tag),
    createTextNode: (t) => new MiniNode(3, '', t),
  };
}

test('renderExploreChatCitations renders F6 multi-paper matched chips, OCR page chip, and unread warnings', async () => {
  installMiniDom();
  const { renderExploreChatCitations } = await import('../assets/idea_chat_ui.js');

  const opened = [];
  const openedImages = [];
  const citations = [
    {
      raw: '[arxiv:2501.00001: L2-3]',
      paper_ref: 'arxiv:2501.00001',
      paper_key: 'arxiv:2501.00001',
      source_id: 'src-p1',
      path: 'src-p1/paper1.tex',
      start_line: 2,
      end_line: 3,
      matched: true,
      status: 'matched',
      text_basis: 'tex',
    },
    {
      raw: '[src-p2: L1-3]',
      paper_ref: 'src-p2',
      paper_key: 'arxiv:2501.00002',
      source_id: 'src-p2',
      path: 'src-p2/paper2.txt',
      start_line: 1,
      end_line: 3,
      matched: true,
      status: 'matched',
      page: 1,
      ocr_tag: 'OCR',
      text_basis: 'scan_text_layer',
    },
    {
      raw: '[arxiv:2501.00002: L9-10]',
      paper_ref: 'arxiv:2501.00002',
      paper_key: 'arxiv:2501.00002',
      source_id: 'src-p2',
      start_line: 9,
      end_line: 10,
      matched: false,
      status: 'unread',
      reason: 'lines not read from this paper in this conversation',
    },
    {
      raw: '[arxiv:9999.99999: L1-2]',
      paper_ref: 'arxiv:9999.99999',
      start_line: 1,
      end_line: 2,
      matched: false,
      status: 'unread',
      reason: 'cited paper is not ingested in this session',
    },
  ];

  const section = renderExploreChatCitations(citations, {
    onOpen: (target) => opened.push(target),
    onOpenPageImage: (target) => openedImages.push(target),
  });
  assert.ok(section, 'citation panel should render');
  assert.equal(section.getAttribute('data-idea-section'), 'chat-citations');

  const fullText = section.textContent;
  assert.match(fullText, /2 matched/);
  assert.match(fullText, /2 unread/);
  assert.ok(!fullText.toLowerCase().includes('verified'), `Rule S8: found 'verified' in: ${fullText}`);

  const matchedChips = section.findAll((n) => n.getAttribute('data-chat-cite-status') === 'matched');
  assert.equal(matchedChips.length, 2);
  assert.equal(matchedChips[0].getAttribute('data-source-id'), 'src-p1');
  assert.equal(matchedChips[1].getAttribute('data-source-id'), 'src-p2');
  assert.equal(matchedChips[1].getAttribute('data-ocr'), 'true');

  // Clicking the TeX chip invokes onOpen; clicking the OCR chip invokes onOpenPageImage.
  const openBtns = section.findAll((n) => n.className === 'ui-quote-chip-open');
  assert.equal(openBtns.length, 2);
  openBtns[0].click();
  openBtns[1].click();
  assert.equal(opened.length, 1);
  assert.equal(opened[0].source_id, 'src-p1');
  assert.equal(opened[0].start_line, 2);
  assert.equal(openedImages.length, 1);
  assert.equal(openedImages[0].source_id, 'src-p2');
  assert.equal(openedImages[0].page, 1);

  const warnings = section.findAll((n) => n.getAttribute('data-chat-cite-status') === 'unread');
  assert.equal(warnings.length, 2);
  assert.equal(warnings[0].getAttribute('data-border'), 'dashed');
  assert.match(warnings[0].textContent, /lines not read/);
  assert.match(warnings[1].textContent, /not ingested/);
});

test('linkExploreChatCitations links matched spans, warns unread spans, and skips code/katex', async () => {
  installMiniDom();
  const { linkExploreChatCitations } = await import('../assets/idea_chat_ui.js');

  const root = document.createElement('div');
  const p = document.createElement('p');
  p.append(
    document.createTextNode('Claim A [arxiv:2501.00001: L2-3] and unread [src-p2: L9-10].'),
  );
  const code = document.createElement('code');
  code.append(document.createTextNode('[arxiv:2501.00001: L2-3]'));
  root.append(p, code);

  const clicked = [];
  linkExploreChatCitations(
    root,
    [
      {
        paper_ref: 'arxiv:2501.00001',
        source_id: 'src-p1',
        path: 'src-p1/paper1.tex',
        start_line: 2,
        end_line: 3,
        matched: true,
        status: 'matched',
      },
      {
        paper_ref: 'src-p2',
        source_id: 'src-p2',
        start_line: 9,
        end_line: 10,
        matched: false,
        status: 'unread',
        reason: 'lines not read from this paper in this conversation',
      },
    ],
    { onOpen: (entry) => clicked.push(entry) },
  );

  const btns = p.findAll((n) => n.tagName === 'BUTTON');
  assert.equal(btns.length, 1);
  assert.equal(btns[0].textContent, 'L2-3');
  btns[0].click();
  assert.equal(clicked.length, 1);
  assert.equal(clicked[0].source_id, 'src-p1');

  const unreadSpans = p.findAll((n) => n.getAttribute('data-chat-cite-status') === 'unread');
  assert.equal(unreadSpans.length, 1);
  assert.match(unreadSpans[0].textContent, /unread/);

  // Code block was untouched (no button inside <code>).
  assert.equal(code.findAll((n) => n.tagName === 'BUTTON').length, 0);

  // Carbon B1 & M4 regressions: semicolon-grouped `[src-a: L1-2; src-b: L3-4]` + inline OCR `onOpenPageImage`.
  const semiP = document.createElement('p');
  semiP.append(
    document.createTextNode('Combined [src-a: L1-2; src-b: L3-4; src-c: L9] in one bracket.'),
  );
  const openedStd = [];
  const openedOcr = [];
  linkExploreChatCitations(
    semiP,
    [
      { paper_ref: 'src-a', source_id: 'src-a', start_line: 1, end_line: 2, matched: true, status: 'matched' },
      { paper_ref: 'src-b', source_id: 'src-b', start_line: 3, end_line: 4, matched: true, status: 'matched', page: 2, ocr_tag: 'OCR', text_basis: 'scan_text_layer' },
      { paper_ref: 'src-c', source_id: 'src-c', start_line: 9, end_line: 9, matched: false, status: 'unread', reason: 'not read' },
    ],
    {
      onOpen: (e) => openedStd.push(e),
      onOpenPageImage: (e) => openedOcr.push(e),
    },
  );
  const semiBtns = semiP.findAll((n) => n.tagName === 'BUTTON');
  assert.equal(semiBtns.length, 2, 'both matched segments inside semicolon bracket must become buttons');
  assert.equal(semiBtns[1].getAttribute('data-ocr'), 'true');
  semiBtns[0].click();
  semiBtns[1].click();
  assert.equal(openedStd.length, 1);
  assert.equal(openedStd[0].source_id, 'src-a');
  assert.equal(openedOcr.length, 1);
  assert.equal(openedOcr[0].source_id, 'src-b');
  assert.equal(openedOcr[0].openPageImage, true);
  assert.equal(semiP.findAll((n) => n.getAttribute('data-chat-cite-status') === 'unread').length, 1);
});

test('mountExploreChatCitations escapes XSS payloads, handles SSE updates, and obeys Rule S8', async () => {
  installMiniDom();
  const { mountExploreChatCitations, EVENT_IDEA_CHAT_CITATIONS } = await import('../assets/idea_chat_ui.js');

  const host = document.createElement('div');
  const xssPayload = '<img src=x onerror=alert(1)>';
  const ctrl = mountExploreChatCitations(host, {
    events: [
      {
        seq: 12,
        kind: EVENT_IDEA_CHAT_CITATIONS,
        payload: {
          turn_seq: 5,
          matched_count: 0,
          unread_count: 1,
          citations: [
            {
              raw: `[${xssPayload}: L1-2]`,
              paper_ref: xssPayload,
              start_line: 1,
              end_line: 2,
              matched: false,
              status: 'unread',
              reason: `Bad paper ${xssPayload}`,
            },
          ],
        },
      },
    ],
  });

  // XSS string stays plain textContent with zero <img> element created.
  assert.equal(host.findAll((n) => n.tagName === 'IMG').length, 0);
  assert.ok(host.textContent.includes(xssPayload));
  assert.ok(!host.textContent.toLowerCase().includes('verified'));

  // Live SSE update replaces with a matched chip cleanly.
  ctrl.applySseEvent(EVENT_IDEA_CHAT_CITATIONS, {
    turn_seq: 9,
    matched_count: 1,
    unread_count: 0,
    citations: [
      {
        raw: '[src-p1: L2-3]',
        paper_ref: 'src-p1',
        source_id: 'src-p1',
        path: 'src-p1/paper1.tex',
        start_line: 2,
        end_line: 3,
        matched: true,
        status: 'matched',
      },
    ],
  });
  assert.match(host.textContent, /1 matched/);
  assert.ok(!host.textContent.toLowerCase().includes('verified'));

  // Static audit of `idea_chat_ui.js`: zero innerHTML writes, zero Rule S8 "verified" in code.
  const src = fs.readFileSync(path.join(ASSETS_DIR, 'idea_chat_ui.js'), 'utf8');
  assert.ok(!src.includes('innerHTML'), 'idea_chat_ui.js must never write innerHTML');
  const nonComment = src.replace(/\/\/.*$/gm, '');
  assert.ok(!/\bverified\b/i.test(nonComment), 'Rule S8: zero "verified" in idea_chat_ui.js code');
});
