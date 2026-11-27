import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  WRITEUP_CHECK_TABLE_NOTE,
  WRITEUP_FORMATS,
  fetchWriteup,
  mountWriteupButton,
  normalizeWriteupFormat,
  parseContentDispositionFilename,
  writeupUrl,
} from '../assets/writeup_btn.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');

function createMockDom() {
  const clickedLinks = [];

  function matchesSelector(node, sel) {
    const attr = /^\[([^=\]]+)="([^"]+)"\]$/.exec(sel);
    if (attr) return node.getAttribute(attr[1]) === attr[2];
    return node.tagName === sel.toUpperCase();
  }

  function findFirst(root, sel) {
    for (const child of root.children) {
      if (matchesSelector(child, sel)) return child;
      const nested = findFirst(child, sel);
      if (nested) return nested;
    }
    return null;
  }

  function makeElement(tagName) {
    const attrs = new Map();
    const listeners = new Map();
    const children = [];
    let _textContent = '';

    const el = {
      tagName: String(tagName || 'div').toUpperCase(),
      children,
      parentNode: null,
      ownerDocument: null,
      className: '',
      type: '',
      value: '',
      checked: false,
      disabled: false,
      title: '',
      href: '',
      download: '',
      get textContent() {
        if (children.length === 0) return _textContent;
        return _textContent + children.map((c) => c.textContent).join('');
      },
      set textContent(v) {
        children.length = 0;
        _textContent = v === undefined || v === null ? '' : String(v);
      },
      get innerHTML() {
        throw new Error('innerHTML read forbidden');
      },
      set innerHTML(_) {
        throw new Error('innerHTML write forbidden');
      },
      set outerHTML(_) {
        throw new Error('outerHTML write forbidden');
      },
      insertAdjacentHTML() {
        throw new Error('insertAdjacentHTML forbidden');
      },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
      },
      getAttribute(k) {
        return attrs.has(String(k)) ? attrs.get(String(k)) : null;
      },
      appendChild(child) {
        child.parentNode = el;
        children.push(child);
        return child;
      },
      remove() {
        if (!el.parentNode) return;
        const idx = el.parentNode.children.indexOf(el);
        if (idx >= 0) el.parentNode.children.splice(idx, 1);
        el.parentNode = null;
      },
      querySelector(sel) {
        return findFirst(el, sel);
      },
      addEventListener(evt, fn) {
        if (!listeners.has(evt)) listeners.set(evt, []);
        listeners.get(evt).push(fn);
      },
      click() {
        if (el.tagName === 'A') {
          clickedLinks.push({ href: el.href, download: el.download });
        }
        for (const fn of listeners.get('click') || []) fn({ target: el });
      },
    };
    return el;
  }

  const doc = {
    body: makeElement('body'),
    createElement(tag) {
      const node = makeElement(tag);
      node.ownerDocument = doc;
      return node;
    },
  };
  doc.body.ownerDocument = doc;
  return { doc, clickedLinks };
}

test('writeupUrl and filename helpers cover tex/md/bib and inline_marks', () => {
  assert.equal(WRITEUP_FORMATS.length, 3);
  assert.equal(normalizeWriteupFormat('MD'), 'md');
  assert.equal(normalizeWriteupFormat('bibtex'), 'bib');
  assert.equal(normalizeWriteupFormat('unknown'), 'tex');

  assert.equal(
    writeupUrl('s-1/a', { format: 'tex', inlineMarks: true }),
    '/api/sessions/s-1%2Fa/writeup?format=tex&inline_marks=1',
  );
  assert.equal(
    writeupUrl('s-1', { format: 'md', inlineMarks: false }),
    '/api/sessions/s-1/writeup?format=md&inline_marks=0',
  );
  assert.equal(
    parseContentDispositionFilename('attachment; filename="s-1-writeup.bib"', 'fallback.bib'),
    's-1-writeup.bib',
  );
  assert.equal(
    parseContentDispositionFilename('attachment; filename="../../evil.tex"', 'fallback.tex'),
    'evil.tex',
  );
});

test('mountWriteupButton sends X-ReSearch-Token, downloads blob, revokes URL, and never uses innerHTML', async () => {
  const { doc, clickedLinks } = createMockDom();
  const host = doc.createElement('div');
  const calls = [];
  const createdUrls = [];
  const revokedUrls = [];
  const scheduled = [];

  const mockUrlApi = {
    createObjectURL(blob) {
      const u = `blob:mock-${createdUrls.length + 1}`;
      createdUrls.push({ u, blob });
      return u;
    },
    revokeObjectURL(u) {
      revokedUrls.push(u);
    },
  };

  const mockFetch = async (url, init) => {
    calls.push({ url, init });
    return {
      ok: true,
      status: 200,
      headers: {
        get(name) {
          const n = name.toLowerCase();
          if (n === 'content-type') return 'text/markdown; charset=utf-8';
          if (n === 'content-disposition') return 'attachment; filename="s-42-writeup.md"';
          return null;
        },
      },
      async blob() {
        return new Blob(['# Synthetic Writeup\n'], { type: 'text/markdown' });
      },
    };
  };

  const ctrl = mountWriteupButton(host, {
    sessionId: 's-42',
    document: doc,
    fetchFn: mockFetch,
    getToken: () => 'tok-header-secret',
    urlApi: mockUrlApi,
    scheduleRevoke: (fn) => scheduled.push(fn),
  });

  assert.ok(ctrl);
  assert.equal(ctrl.noteEl.textContent, WRITEUP_CHECK_TABLE_NOTE);
  ctrl.formatSelect.value = 'md';
  ctrl.inlineCheckbox.checked = false;
  await ctrl.download();

  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, '/api/sessions/s-42/writeup?format=md&inline_marks=0');
  assert.equal(calls[0].init.method, 'GET');
  assert.equal(calls[0].init.headers['X-ReSearch-Token'], 'tok-header-secret');
  assert.equal(clickedLinks.length, 1);
  assert.equal(clickedLinks[0].href, 'blob:mock-1');
  assert.equal(clickedLinks[0].download, 's-42-writeup.md');
  assert.equal(ctrl.statusEl.textContent, 'Saved s-42-writeup.md');

  // Revocation callback releases blob URL.
  assert.equal(scheduled.length, 1);
  scheduled[0]();
  assert.deepEqual(revokedUrls, ['blob:mock-1']);

  ctrl.destroy();
  assert.equal(host.children.length, 0);
});

test('mountWriteupButton renders Gate G7 and hostile HTML errors safely via textContent', async () => {
  const { doc } = createMockDom();
  const host = doc.createElement('div');
  const hostile = 'Solve write-up refused (Gate G7): <script>alert(1)</script>';

  const ctrl = mountWriteupButton(host, {
    sessionId: 's-fail',
    document: doc,
    getToken: () => 'tok-123',
    fetchFn: async () => ({
      ok: false,
      status: 409,
      async text() {
        return JSON.stringify({ error: hostile });
      },
    }),
  });

  await ctrl.download();
  assert.equal(ctrl.statusEl.textContent, hostile);
  assert.ok(ctrl.statusEl.className.includes('error'));
});

test('fetchWriteup rejects missing sessionId and assets.rs registers /assets/writeup_btn.js', async () => {
  await assert.rejects(() => fetchWriteup(''), /Select a session/);
  const assetsRs = fs.readFileSync(
    path.join(REPO_ROOT, 'crates', 'desktop-server', 'src', 'assets.rs'),
    'utf8',
  );
  assert.ok(assetsRs.includes('"/assets/writeup_btn.js"'));
  const authRs = fs.readFileSync(
    path.join(REPO_ROOT, 'crates', 'desktop-server', 'src', 'auth.rs'),
    'utf8',
  );
  assert.ok(!authRs.includes('writeup'), 'writeup route must NEVER be added to auth.rs cookie routes');
});
