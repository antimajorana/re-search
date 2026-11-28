import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  READ_ONLY_BANNER_TEXT,
  destroyInvestigationWriteup,
  formatDroppedQuotesBanner,
  mountInvestigationShare,
  syncInvestigationEntryShare,
} from '../assets/investigation_share_ui.js';

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
      files: [],
      type: '',
      value: '',
      checked: false,
      hidden: false,
      disabled: false,
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      remove() {},
      addEventListener(type, fn) {
        (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn);
      },
      async dispatchAsync(type, event = {}) {
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) await fn(ev);
      },
      click() { void this.dispatchAsync('click'); },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const walk = (n) => {
          for (const ch of n.children || []) {
            if (!ch || typeof ch !== 'object') continue;
            if (matchesSel(ch, sel)) out.push(ch);
            walk(ch);
          }
        };
        walk(this);
        return out;
      },
    };
    return el;
  };

  function matchesSel(node, sel) {
    if (sel.startsWith('.')) return node.className.split(/\s+/).includes(sel.slice(1));
    const attrM = sel.match(/^\[([^=]+)="([^"]+)"\]$/);
    if (attrM) return node.getAttribute(attrM[1]) === attrM[2];
    return node.tagName === sel.toUpperCase();
  }

  return {
    document: { createElement: (t) => makeEl(t), body: makeEl('body') },
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
  };
}

test('export controls default opt-in checkboxes off, honor opt-ins, and pass API options cleanly', async () => {
  const dom = makeFakeDom();
  const prevDoc = globalThis.document;
  globalThis.document = dom.document;
  try {
    const calls = [];
    const toolbar = dom.makeEl('div');
    const centerCol = dom.makeEl('div');
    const ui = mountInvestigationShare({
      toolbar,
      centerCol,
      sessionId: 'sess-1',
      sessionData: { events: [] },
      apiMod: {
        async exportInvestigationFile(sid, opts) {
          calls.push({ sid, opts });
          return { blob: null, filename: 'out.rsmap' };
        },
        async importInvestigationMap() { return {}; },
      },
      hooks: { refresh: async () => {}, showStatus: () => {} },
    });

    assert.equal(ui.stmtCheck.checked, false);
    assert.equal(ui.notesCheck.checked, false);

    ui.exportRsmapBtn.click();
    await Promise.resolve();
    assert.deepEqual(calls[0], {
      sid: 'sess-1',
      opts: { format: 'rsmap', includeStatements: false, includeNotes: false },
    });

    ui.stmtCheck.checked = true;
    ui.notesCheck.checked = true;
    ui.exportHtmlBtn.click();
    await Promise.resolve();
    assert.deepEqual(calls[1], {
      sid: 'sess-1',
      opts: { format: 'html', includeStatements: true, includeNotes: true },
    });

    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    globalThis.document = prevDoc;
  }
});

test('import displays dropped-quote and read-only banners without innerHTML or verified text', async () => {
  const dom = makeFakeDom();
  const prevDoc = globalThis.document;
  globalThis.document = dom.document;
  try {
    const entryHost = dom.makeEl('div');
    const entryCard = dom.makeEl('section');
    entryCard.className = 'inv-entry-card';
    entryHost.appendChild(entryCard);

    syncInvestigationEntryShare(entryHost, {
      sessionId: 'sess-ro',
      sessionData: {
        events: [{
          kind: 'investigation.imported',
          payload: {
            source_match: false,
            read_only: true,
            banner: READ_ONLY_BANNER_TEXT,
            preview: {
              paper_key: 'arxiv:2402.00001',
              version: 'v2',
              title: '<script>alert(1)</script> Stability',
              gists: [{ tag: 'n-abc', role: 'main_result', gist: 'Bounded <img src=x>' }],
            },
          },
        }],
      },
      hooks: { refresh: async () => {}, showStatus: () => {} },
    });

    const preview = entryHost.querySelector('[data-testid="inv-share-ro-preview"]');
    assert.ok(preview.textContent.includes('quotes not matched on this machine'));
    assert.ok(preview.textContent.includes('<script>alert(1)</script>'));
    assert.equal(dom.innerHtmlWrites, 0);

    const toolbar = dom.makeEl('div');
    const centerCol = dom.makeEl('div');
    const ui = mountInvestigationShare({
      toolbar,
      centerCol,
      sessionId: 'sess-tamper',
      sessionData: { events: [] },
      apiMod: {
        async exportInvestigationFile() { return {}; },
        async importInvestigationMap() {
          return {
            source_match: true,
            items_matched: 4,
            items_dropped: 2,
            banner: formatDroppedQuotesBanner(2),
          };
        },
      },
      hooks: { refresh: async () => {}, showStatus: () => {} },
    });

    ui.fileInput.files = [{ text: async () => JSON.stringify({ format: 'rsmap', paper_key: 'arxiv:2402.00001' }) }];
    await ui.fileInput.dispatchAsync('change');
    assert.equal(ui.bannerHost.textContent, 'Imported, 2 quotes could not be matched');
    assert.equal(dom.innerHtmlWrites, 0);

    const src = readFileSync(new URL('../assets/investigation_share_ui.js', import.meta.url), 'utf8');
    assert.ok(!/verified/i.test(src), 'investigation_share_ui.js must never contain "verified"');
    assert.ok(!src.includes('innerHTML'), 'investigation_share_ui.js must never use innerHTML');
  } finally {
    globalThis.document = prevDoc;
  }
});

test('mountInvestigationShare mounts F15 write-up control with X-ReSearch-Token and cancels stale remount downloads', async () => {
  const dom = makeFakeDom();
  const prevDoc = globalThis.document;
  globalThis.document = dom.document;
  try {
    const requests = [];
    const createdUrls = [];
    const revokedUrls = [];
    const statuses = [];
    const urlApi = {
      createObjectURL(blob) {
        const u = `blob:inv-${createdUrls.length + 1}`;
        createdUrls.push({ u, blob });
        return u;
      },
      revokeObjectURL(u) { revokedUrls.push(u); },
    };

    const toolbar = dom.makeEl('div');
    const centerCol = dom.makeEl('div');
    const ui1 = mountInvestigationShare({
      toolbar,
      centerCol,
      sessionId: 's-inv-ui-1',
      sessionData: { events: [] },
      apiMod: {
        getCsrfToken: () => 'inv-csrf-tok',
        async exportInvestigationFile() { return {}; },
        async importInvestigationMap() { return {}; },
      },
      hooks: { refresh: async () => {}, showStatus: (m) => statuses.push(m) },
      fetchFn: async (url, init) => {
        requests.push({ url, init });
        return {
          ok: true,
          status: 200,
          headers: {
            get(name) {
              const n = name.toLowerCase();
              if (n === 'content-type') return 'text/markdown; charset=utf-8';
              if (n === 'content-disposition') return 'attachment; filename="s-inv-ui-1-writeup.md"';
              return null;
            },
          },
          blob: async () => new Blob(['# Reading Notes\n'], { type: 'text/markdown' }),
        };
      },
      urlApi,
      scheduleRevoke: (fn) => fn(),
    });

    assert.ok(ui1.writeupHost, 'toolbar must mount inv-writeup-host');
    assert.ok(ui1.writeupCtrl, 'toolbar must mount writeup_btn controller');
    ui1.writeupCtrl.formatSelect.value = 'md';
    ui1.writeupCtrl.inlineCheckbox.checked = false;
    await ui1.writeupCtrl.download();

    assert.equal(requests.length, 1);
    assert.equal(requests[0].url, '/api/sessions/s-inv-ui-1/writeup?format=md&inline_marks=0');
    assert.equal(requests[0].init.headers['X-ReSearch-Token'], 'inv-csrf-tok');
    assert.equal(createdUrls.length, 1);
    assert.deepEqual(revokedUrls, ['blob:inv-1']);
    assert.ok(statuses.some((s) => s.includes('s-inv-ui-1-writeup.md')));

    // Switching sessions creates a fresh toolbar node (matching investigation_v3_shell renderShell)
    // and must STILL destroy the previous controller so its in-flight download cannot trigger a save.
    let resolveSlow;
    const toolbarOld = dom.makeEl('div');
    const toolbarNew = dom.makeEl('div');
    const uiSlow = mountInvestigationShare({
      toolbar: toolbarOld,
      centerCol,
      sessionId: 's-inv-old',
      sessionData: { events: [] },
      apiMod: { getCsrfToken: () => 'inv-csrf-tok' },
      hooks: { showStatus: () => {} },
      fetchFn: () => new Promise((resolve) => { resolveSlow = resolve; }),
      urlApi,
      scheduleRevoke: (fn) => fn(),
    });
    const pending = uiSlow.writeupCtrl.download();
    mountInvestigationShare({
      toolbar: toolbarNew,
      centerCol,
      sessionId: 's-inv-new',
      sessionData: { events: [] },
      apiMod: { getCsrfToken: () => 'inv-csrf-tok' },
      hooks: { showStatus: () => {} },
      fetchFn: async () => ({ ok: true, status: 200, headers: { get: () => '' }, blob: async () => new Blob(['']) }),
      urlApi,
      scheduleRevoke: (fn) => fn(),
    });
    resolveSlow({
      ok: true,
      status: 200,
      headers: { get: () => 'attachment; filename="s-inv-old-writeup.tex"' },
      blob: async () => new Blob(['old'], { type: 'application/x-tex' }),
    });
    await pending;
    assert.equal(createdUrls.length, 1, 'stale download after fresh-toolbar session switch must be dropped');

    // Leaving Investigate mode calls destroyInvestigationWriteup and drops any in-flight download.
    let resolveLeave;
    const uiLeave = mountInvestigationShare({
      toolbar: dom.makeEl('div'),
      centerCol,
      sessionId: 's-inv-leave',
      sessionData: { events: [] },
      apiMod: { getCsrfToken: () => 'inv-csrf-tok' },
      hooks: { showStatus: () => {} },
      fetchFn: () => new Promise((resolve) => { resolveLeave = resolve; }),
      urlApi,
      scheduleRevoke: (fn) => fn(),
    });
    const pendingLeave = uiLeave.writeupCtrl.download();
    destroyInvestigationWriteup();
    resolveLeave({
      ok: true,
      status: 200,
      headers: { get: () => 'attachment; filename="s-inv-leave-writeup.tex"' },
      blob: async () => new Blob(['leave'], { type: 'application/x-tex' }),
    });
    await pendingLeave;
    assert.equal(createdUrls.length, 1, 'download after destroyInvestigationWriteup must be dropped');
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    destroyInvestigationWriteup();
    globalThis.document = prevDoc;
  }
});
