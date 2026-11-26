import assert from 'node:assert/strict';
import test from 'node:test';

import {
  EXPORT_BIBTEX_ENDPOINT,
  EXPORT_BIBTEX_LABEL,
  SEND_KANDINSKY_ENDPOINT,
  SEND_TO_KANDINSKY_LABEL,
  buildKandinskyDeepLink,
  mountPaperSendBackActions,
  validateClientCaptureTarget,
} from '../assets/library_send_back.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      children: [],
      type: '',
      value: '',
      placeholder: '',
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
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      appendChild(node) { this.children.push(node); return node; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      async clickAsync() {
        if (this.disabled) return;
        for (const fn of listeners.get('click') || []) {
          await fn({ preventDefault() {}, target: this });
        }
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          return n.tagName === sel.toUpperCase();
        };
        const visit = (n) => {
          for (const c of n.children || []) {
            if (match(c)) out.push(c);
            visit(c);
          }
        };
        visit(this);
        return out;
      },
    };
    return el;
  };
  const prevDoc = globalThis.document;
  globalThis.document = { createElement: (tag) => makeEl(tag) };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

test('validateClientCaptureTarget accepts arXiv/DOI/HTTPS and rejects injection/bad schemes', () => {
  assert.equal(validateClientCaptureTarget('arxiv:2405.01234v2').target, 'arXiv:2405.01234v2');
  assert.equal(validateClientCaptureTarget('doi:10.9999/Synth.1').target, 'doi:10.9999/synth.1');
  assert.equal(
    validateClientCaptureTarget('https://arxiv.org/abs/2405.01234').target,
    'https://arxiv.org/abs/2405.01234',
  );

  for (const bad of [
    '',
    '-e',
    '--config=/tmp/x',
    'arXiv:2405.01234;id',
    'arXiv:2405.01234|cat',
    'arXiv:$(whoami)',
    'arXiv:`id`',
    'arXiv:2405.01234\nfoo',
    'file:///etc/passwd',
    'javascript:alert(1)',
    'http://arxiv.org/abs/2405.01234',
  ]) {
    assert.equal(validateClientCaptureTarget(bad).ok, false, `must reject ${bad}`);
  }

  assert.equal(
    buildKandinskyDeepLink('doi:10.9999/a+b', 'Spin & Gauge'),
    'kandinsky://capture-paper?target=doi%3A10.9999%2Fa%2Bb&title=Spin%20%26%20Gauge',
  );
});

test('library_send_back escapes untrusted title/citekey with zero innerHTML writes and fires only on click', async () => {
  const dom = makeFakeDom();
  try {
    const calls = [];
    const deepLinks = [];
    const slot = dom.makeEl('div');
    const hostilePaper = {
      key: 'arxiv:2407.09999',
      version: 'v1',
      ids: { arxiv: '2407.09999' },
      citekey: '<script>alert(1)</script>',
      title: '<img src=x onerror=alert(2)> Synthetic Title',
    };

    const mounted = mountPaperSendBackActions(slot, hostilePaper, {
      defaultOutputPath: '/tmp/exports/out.bib',
      onDeepLink: (url) => deepLinks.push(url),
      postJson: async (path, body) => {
        calls.push({ path, body });
        if (path === EXPORT_BIBTEX_ENDPOINT) {
          return { ok: true, written: true, count: 1, citekeys: [hostilePaper.citekey], output_path: body.output_path };
        }
        return {
          ok: true,
          mode: 'deep_link',
          target: 'arXiv:2407.09999v1',
          deep_link: 'kandinsky://capture-paper?target=arXiv%3A2407.09999v1',
        };
      },
    });

    // Rendering alone triggers ZERO backend calls and ZERO innerHTML writes.
    assert.equal(calls.length, 0);
    assert.equal(dom.innerHtmlWrites, 0);

    const exportBtn = mounted.exportCtrl.querySelector('.send-back-export-btn');
    const kandBtn = mounted.kandinskyCtrl.querySelector('.send-back-kandinsky-btn');
    assert.equal(exportBtn.textContent, EXPORT_BIBTEX_LABEL);
    assert.equal(kandBtn.textContent, SEND_TO_KANDINSKY_LABEL);
    assert.ok(slot.textContent.includes('<img src=x onerror=alert(2)>'));

    // Explicit click opens BibTeX dialog, second explicit click writes.
    await exportBtn.clickAsync();
    assert.equal(calls.length, 0);
    const confirmBtn = mounted.exportCtrl.querySelector('.send-back-confirm-export');
    await confirmBtn.clickAsync();
    assert.equal(calls.length, 1);
    assert.equal(calls[0].path, EXPORT_BIBTEX_ENDPOINT);
    assert.deepEqual(calls[0].body, {
      paper_keys: ['arxiv:2407.09999'],
      output_path: '/tmp/exports/out.bib',
    });

    // Explicit click sends to Kandinsky.
    await kandBtn.clickAsync();
    assert.equal(calls.length, 2);
    assert.equal(calls[1].path, SEND_KANDINSKY_ENDPOINT);
    assert.equal(calls[1].body.user_initiated, true);
    assert.equal(deepLinks.length, 1);
    assert.equal(dom.innerHtmlWrites, 0, 'library_send_back.js must never touch innerHTML');
  } finally {
    dom.restore();
  }
});
