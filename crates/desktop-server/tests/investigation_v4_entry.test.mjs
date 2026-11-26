import assert from 'node:assert/strict';
import test from 'node:test';

import {
  INVESTIGATE_CARD_TITLE,
  INVESTIGATE_OPEN_VERB,
  NEEDS_FILE_MESSAGE,
  detectPaperInput,
  renderInvestigationEntry,
  renderMetadataCard,
  renderOpenProgress,
} from '../assets/investigation_entry.js';

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
      style: {},
      type: '',
      value: '',
      placeholder: '',
      title: '',
      hidden: false,
      disabled: false,
      focused: false,
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
      removeAttribute(k) { attrs.delete(k); },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      removeEventListener(type, fn) {
        if (listeners.has(type)) listeners.set(type, listeners.get(type).filter((f) => f !== fn));
      },
      dispatch(type, event = {}) {
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      focus() { this.focused = true; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => sel.startsWith('.')
          ? n.classList.contains(sel.slice(1))
          : n.tagName.toLowerCase() === sel.toLowerCase();
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

test('detectPaperInput recognizes arXiv ids, URLs, DOIs, and canonical keys without guessing prose', () => {
  assert.deepEqual(detectPaperInput('1102.0842'), { kind: 'arxiv_id', value: '1102.0842' });
  assert.deepEqual(detectPaperInput('2103.01234v2'), { kind: 'arxiv_id', value: '2103.01234v2' });
  assert.deepEqual(detectPaperInput('https://arxiv.org/abs/1102.0842v3'), { kind: 'url', value: 'https://arxiv.org/abs/1102.0842v3' });
  assert.deepEqual(detectPaperInput('10.1103/PhysRevLett.116.061102'), { kind: 'doi', value: '10.1103/PhysRevLett.116.061102' });
  assert.deepEqual(detectPaperInput('arxiv:1102.0842'), { kind: 'library_key', value: 'arxiv:1102.0842' });
  assert.equal(detectPaperInput('Prove that every finite weighted graph has a spectral gap'), null);
  assert.equal(detectPaperInput(''), null);
});

test('renderInvestigationEntry handles submit, library click, needs_file, metadata, XSS, and zero verified word', () => {
  const dom = makeFakeDom();
  globalThis.document = dom.document;
  globalThis.window = {};

  const host = dom.makeEl('div');
  let submitted = null;
  let pickedKey = null;
  let requestedFile = 0;

  const ctrl = renderInvestigationEntry(host, {
    modelConnected: false,
    recentPapers: [{ key: 'arxiv:2601.00077', title: 'Library Cached Paper' }],
    onOpenInput: (payload) => { submitted = payload; },
    onOpenLibraryKey: (key) => { pickedKey = key; },
    onRequestFile: () => { requestedFile += 1; },
  });

  assert.ok(host.textContent.includes(INVESTIGATE_CARD_TITLE));
  assert.ok(host.textContent.includes(INVESTIGATE_OPEN_VERB));

  const input = host.querySelector('.inv-entry-input');
  const form = host.querySelector('form');
  input.value = '1102.0842';
  form.dispatch('submit');
  assert.equal(submitted.arxiv_id, '1102.0842');

  const libBtn = host.querySelector('.inv-entry-library-btn') || host.querySelectorAll('button').find((b) => b.textContent.includes('Library Cached Paper'));
  assert.ok(libBtn);
  libBtn.click();
  assert.equal(pickedKey, 'arxiv:2601.00077');

  const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
  ctrl.setMetadata({
    paper: { key: 'arxiv:2601.00055', version: 'v2' },
    title: `Paper ${xss}`,
    authors: [`A. Author ${xss}`],
    abstract_text: `Abstract ${xss}`,
    license: 'arXiv non-exclusive',
  });
  ctrl.setSnapshot({
    status: 'needs_file',
    step: 'needs_file',
    history: ['queued', 'metadata', 'needs_file'],
    fetch_budget_used: 1,
    fetch_budget_limit: 8,
    needs_file_reason: '',
  });
  assert.ok(host.textContent.includes(NEEDS_FILE_MESSAGE));
  assert.ok(host.textContent.includes('1 / 8 external fetches'));
  assert.equal(host.querySelectorAll('script').length, 0);
  assert.equal(host.querySelectorAll('img').length, 0);
  assert.equal(dom.innerHtmlWrites, 0);
  assert.equal(host.textContent.toLowerCase().includes('verified'), false);

  // Standalone progress + metadata smoke check
  const pHost = dom.makeEl('div');
  renderOpenProgress(pHost, { status: 'running', step: 'eprint_tex', history: ['queued', 'metadata', 'eprint_tex'] });
  assert.ok(pHost.textContent.includes('Fetching TeX source'));
  const mHost = dom.makeEl('div');
  renderMetadataCard(mHost, null);
  assert.equal(mHost.children.length, 0);

  const dropBtn = host.querySelectorAll('button').find((b) => b.textContent.includes('Drop or choose'));
  assert.ok(dropBtn);
  dropBtn.click();
  assert.equal(requestedFile, 1);
  ctrl.destroy();
});
