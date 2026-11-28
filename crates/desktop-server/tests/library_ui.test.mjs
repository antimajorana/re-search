import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  ATTACH_PDF_LABEL,
  ATTACH_SESSION_LABEL,
  BACKUP_MARKS_LABEL,
  FULLTEXT_BADGE_LABEL,
  LIBRARY_API_BASE,
  LIBRARY_INDEX_NOTICE,
  MARK_STATUSES,
  OWNED_BADGE_LABEL,
  REBUILD_INDEX_LABEL,
  RESTORE_MARKS_LABEL,
  SAVE_TO_PROJECT_LABEL,
  SEEN_ROLES,
  buildLibraryQueryString,
  buildProjectSourcePayload,
  encodePaperKey,
  libraryPaperUrl,
  mountLibraryPane,
} from '../assets/library.js';
import {
  SOURCES_API_BASE,
  SOURCES_READ_ONLY_NOTICE,
  SOURCE_KIND_LABELS,
  sourceUrl,
} from '../assets/library_sources.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    let childNodes = [];
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const htmlCollection = {
      get length() { return childNodes.length; },
      item(i) { return childNodes[i] || null; },
      [Symbol.iterator]() { return childNodes[Symbol.iterator](); },
    };
    const el = {
      tagName: String(tag).toUpperCase(),
      get children() { return htmlCollection; },
      get firstChild() { return childNodes[0] || null; },
      type: '',
      value: '',
      checked: false,
      placeholder: '',
      accept: '',
      href: '',
      download: '',
      files: [],
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
        return childNodes.length ? childNodes.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { childNodes = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; childNodes = []; html = String(v ?? ''); },
      attributes: attrs,
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      appendChild(node) { childNodes.push(node); return node; },
      removeChild(node) { childNodes = childNodes.filter((c) => c !== node); return node; },
      append(...nodes) { for (const n of nodes) childNodes.push(n); },
      replaceChildren(...nodes) { childNodes = [...nodes]; },
      addEventListener(type, fn) {
        (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn);
      },
      async trigger(type, extra = {}) {
        if (this.disabled) return;
        for (const fn of listeners.get(type) || []) {
          await fn({ preventDefault() {}, target: this, ...extra });
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
  globalThis.document = {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function makeFakeFetch(handler) {
  const calls = [];
  const fetchImpl = async (url, init = {}) => {
    const method = (init.method || 'GET').toUpperCase();
    const body = init.body ? JSON.parse(init.body) : undefined;
    const headers = init.headers || {};
    calls.push({ method, url, body, headers });
    const res = await handler(method, url, body, headers);
    const status = res?.status ?? 200;
    const payload = res?.body !== undefined ? res.body : res;
    return {
      ok: status >= 200 && status < 300,
      status,
      async text() { return payload === undefined ? '' : JSON.stringify(payload); },
    };
  };
  return { calls, fetchImpl };
}

test('encodePaperKey and URL builders encode `/` and `+` in PaperKeys and source IDs', () => {
  const doiPlus = 'doi:10.1000/a+b/c+d';
  const oldArxiv = 'arxiv:hep-th/9901001';
  assert.equal(encodePaperKey(doiPlus), 'doi%3A10.1000%2Fa%2Bb%2Fc%2Bd');
  assert.equal(libraryPaperUrl(doiPlus), '/api/library/doi%3A10.1000%2Fa%2Bb%2Fc%2Bd');
  assert.equal(libraryPaperUrl(doiPlus, '/mark'), '/api/library/doi%3A10.1000%2Fa%2Bb%2Fc%2Bd/mark');
  assert.equal(libraryPaperUrl(oldArxiv, '/attach'), '/api/library/arxiv%3Ahep-th%2F9901001/attach');
  assert.equal(sourceUrl('vault+a/1', '/sync'), '/api/sources/vault%2Ba%2F1/sync');

  const qs = buildLibraryQueryString({
    q: 'krylov + chain', title: 'Lanczos', author: 'Ada', year: '2024', status: 'key', role: 'read',
  });
  assert.ok(qs.startsWith('?'));
  const params = new URLSearchParams(qs.slice(1));
  assert.equal(params.get('q'), 'krylov + chain');
  assert.equal(params.get('title'), 'Lanczos');
  assert.equal(params.get('author'), 'Ada');
  assert.equal(params.get('year'), '2024');
  assert.equal(params.get('status'), 'key');
  assert.equal(params.get('role'), 'read');
});

test('Rule S8: zero "verified", "new", or "novel" in exported Library UI labels and constants', () => {
  const labels = [
    OWNED_BADGE_LABEL,
    FULLTEXT_BADGE_LABEL,
    ATTACH_PDF_LABEL,
    ATTACH_SESSION_LABEL,
    REBUILD_INDEX_LABEL,
    BACKUP_MARKS_LABEL,
    RESTORE_MARKS_LABEL,
    SAVE_TO_PROJECT_LABEL,
    LIBRARY_INDEX_NOTICE,
    SOURCES_READ_ONLY_NOTICE,
    ...MARK_STATUSES,
    ...SEEN_ROLES,
    ...Object.values(SOURCE_KIND_LABELS),
  ];
  for (const text of labels) {
    assert.ok(!/\b(verified|new|novel|novelty|proved)\b/i.test(text), `forbidden word in label: ${text}`);
  }
});

test('Library pane search/filter, detail drawer, mark update, Save to project, Attach PDF, rebuild, backup/restore, and F13c send-back', async () => {
  const dom = makeFakeDom();
  try {
    const plusKey = 'doi:10.1000/a+b';
    const hostileTitle = '<script>alert(1)</script> Synthetic Gauge Chain';
    const hostileQuote = '<img src=x onerror=alert(2)> Matching synthetic equation quote.';
    const detailFixture = {
      paper: {
        key: plusKey,
        ids: { doi: '10.1000/a+b' },
        title: hostileTitle,
        authors: ['<b>Ada Synthetic</b>'],
        year: 2024,
        bibtex: '@article{ada2024synth, title={Synthetic}}',
        flags: { owned: true },
      },
      aliases: [plusKey],
      fulltext: [{ key: plusKey, format: 'pdf', origin: 'user_library', raw_blob: 'a'.repeat(64), bytes: 1024 }],
      seen: [{ key: plusKey, session_id: 'exp-sess-1', role: 'read', seen_ms: 100 }],
      skeletons: [],
      findings: [{
        key: plusKey, session_id: 'exp-sess-1', evidence_seq: 9,
        role: 'states_result', gist: 'Model summary <i>here</i>', quote: hostileQuote,
        cited_range: [12, 14], recorded_ms: 200,
      }],
      paper_text: [],
      source_items: [{
        source_id: 'kandinsky-main', external_id: 'ada2024synth',
        citekey: '<svg onload=alert(3)>ada2024synth', state: 'present', version: 'v1',
      }],
      user_mark: { key: plusKey, status: 'skimmed', tags: ['lattice'], note: 'Check appendix' },
    };

    const savedProjectCalls = [];
    const downloadedBackups = [];
    const { calls, fetchImpl } = makeFakeFetch((method, url, body) => {
      if (method === 'GET' && url.startsWith(LIBRARY_API_BASE) && !url.includes('/marks/') && url.split('?')[0] === LIBRARY_API_BASE) {
        return {
          papers: [{
            paper: detailFixture.paper,
            user_mark: detailFixture.user_mark,
            has_fulltext: true,
            owned: true,
            seen_roles: ['read'],
            seen_sessions: ['exp-sess-1'],
            finding_count: 1,
            skeleton_count: 0,
          }],
          count: 1,
        };
      }
      if (method === 'GET' && url === libraryPaperUrl(plusKey)) return detailFixture;
      if (method === 'POST' && url === libraryPaperUrl(plusKey, '/mark')) {
        return { ok: true, mark: { key: plusKey, ...body } };
      }
      if (method === 'DELETE' && url === libraryPaperUrl(plusKey, '/mark')) {
        return { ok: true, deleted: true, key: plusKey };
      }
      if (method === 'POST' && url === libraryPaperUrl(plusKey, '/attach')) {
        return { ok: true, ingested: { source_id: 'src-1' } };
      }
      if (method === 'POST' && url === '/api/sessions/exp-sess-1/idea/read') {
        return { ok: true, paper: { key: plusKey } };
      }
      if (method === 'POST' && url === '/api/library/rebuild') {
        return { ok: true, report: { papers: 1, fulltext_entries: 1, user_marks_restored: 1 } };
      }
      if (method === 'GET' && url === '/api/library/marks/backup') {
        return { schema_version: 1, sha256: '0'.repeat(64), marks: [detailFixture.user_mark] };
      }
      if (method === 'POST' && url === '/api/library/marks/restore') {
        return { ok: true, restored: 1 };
      }
      if (method === 'POST' && url === '/api/sources/send/kandinsky') {
        return { ok: true, mode: 'deep_link', target: 'doi:10.1000/a+b', deep_link: 'kandinsky://capture-paper?target=doi%3A10.1000%2Fa%2Bb' };
      }
      throw new Error(`Unexpected fetch: ${method} ${url}`);
    });

    const host = dom.makeEl('div');
    const pane = mountLibraryPane(host, {
      autoLoad: false,
      fetchImpl,
      getToken: () => 'tok-test-123',
      getActiveSessionId: () => 'exp-sess-1',
      getActiveSessionMode: () => 'explore',
      listProjects: async () => ({ projects: [{ id: 'p-1', title: 'Gauge Study' }] }),
      saveProjectSource: async (pid, payload) => {
        savedProjectCalls.push({ pid, payload });
        return { ok: true };
      },
      onDownloadBackup: (text, filename) => downloadedBackups.push({ text, filename }),
    });

    // 1. Search/filter sends encoded query params and token header.
    await pane.loadPapers({ q: 'gauge', author: 'Ada', year: '2024', status: 'key', role: 'read' });
    const searchCall = calls.find((c) => c.method === 'GET' && c.url.startsWith('/api/library?'));
    assert.ok(searchCall, 'expected GET /api/library with filters');
    assert.equal(searchCall.headers['X-ReSearch-Token'], 'tok-test-123');
    assert.ok(searchCall.url.includes('q=gauge'));
    assert.ok(searchCall.url.includes('author=Ada'));
    assert.ok(searchCall.url.includes('status=key'));
    assert.ok(searchCall.url.includes('role=read'));
    assert.ok(host.textContent.includes(OWNED_BADGE_LABEL));

    // 2. Open paper detail drawer uses encodeURIComponent for `/` and `+` in key.
    await pane.openPaperDetail(plusKey);
    assert.ok(calls.some((c) => c.method === 'GET' && c.url === '/api/library/doi%3A10.1000%2Fa%2Bb'));
    assert.ok(host.textContent.includes(hostileTitle));
    assert.ok(host.textContent.includes(hostileQuote));
    assert.ok(host.textContent.includes('kandinsky-main'));
    assert.ok(host.textContent.includes('exp-sess-1 (read)'));

    // 3. F13c send-back actions are mounted inside detail drawer and never auto-fire on render.
    assert.ok(!calls.some((c) => c.url.startsWith('/api/sources/send/')));
    const kandBtn = pane.el.querySelector('.send-back-kandinsky-btn');
    assert.ok(kandBtn, 'Send to Kandinsky button must be mounted');
    await kandBtn.trigger('click');
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/sources/send/kandinsky'));

    // 4. Mark update uses encodeURIComponent(key).
    await pane.saveMark(plusKey, { status: 'key', tags: ['important', 'gauge'], note: 'Core reference' });
    const markCall = calls.find((c) => c.method === 'POST' && c.url === '/api/library/doi%3A10.1000%2Fa%2Bb/mark');
    assert.ok(markCall);
    assert.deepEqual(markCall.body, { status: 'key', tags: ['important', 'gauge'], note: 'Core reference' });

    // 5. Save to project calls saveProjectSource with strict SourceInput payload.
    const projPayload = buildProjectSourcePayload(detailFixture);
    assert.deepEqual(Object.keys(projPayload).sort(), ['bibtex', 'doi', 'notes', 'quote', 'title', 'url']);
    await pane.saveDetailToProject('p-1');
    assert.equal(savedProjectCalls.length, 1);
    assert.equal(savedProjectCalls[0].pid, 'p-1');
    assert.equal(savedProjectCalls[0].payload.doi, '10.1000/a+b');

    // 6. Attach to session & Attach PDF upload action.
    await pane.attachToSession(plusKey);
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/library/doi%3A10.1000%2Fa%2Bb/attach'));

    await pane.attachPdfFile(plusKey, { name: 'synth.pdf', bytes: new Uint8Array([37, 80, 68, 70]) });
    const uploadCall = calls.find((c) => c.method === 'POST' && c.url === '/api/sessions/exp-sess-1/idea/read');
    assert.ok(uploadCall);
    assert.equal(uploadCall.body.attach_pdf.paper_key, plusKey);
    assert.equal(uploadCall.body.attach_pdf.file_name, 'synth.pdf');
    assert.equal(uploadCall.body.attach_pdf.bytes_base64, 'JVBERg==');

    // 7. Rebuild index + mark backup download & restore.
    await pane.rebuildIndex();
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/library/rebuild'));
    await pane.downloadMarksBackup();
    assert.equal(downloadedBackups.length, 1);
    await pane.restoreMarksBackup(downloadedBackups[0].text);
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/library/marks/restore'));

    // 8. Zero innerHTML writes (XSS safe) and zero "verified" anywhere in rendered UI text.
    assert.equal(dom.innerHtmlWrites, 0, 'library.js must never write innerHTML');
    assert.ok(!/\bverified\b/i.test(host.textContent), `rendered UI must not contain "verified": ${host.textContent}`);
  } finally {
    dom.restore();
  }
});

test('buildProjectSourcePayload preserves old-style arXiv slash, strips PDF form-feeds, and filters invalid DOIs', () => {
  const oldArxiv = buildProjectSourcePayload({
    paper: {
      key: 'arxiv:hep-th/9901001',
      ids: { arxiv: 'hep-th/9901001', doi: 'not-a-doi' },
      title: 'Old\x0cStyle\x07 ArXiv Title',
    },
    findings: [{ quote: 'Page 1\x0cPage 2 quote' }],
    user_mark: { note: 'Line1\nLine2\x00' },
  });
  assert.equal(oldArxiv.url, 'https://arxiv.org/abs/hep-th/9901001');
  assert.equal(oldArxiv.doi, undefined);
  assert.equal(oldArxiv.title, 'Old Style ArXiv Title');
  assert.equal(oldArxiv.quote, 'Page 1 Page 2 quote');
  assert.equal(oldArxiv.notes, 'Line1\nLine2');
});

test('F13a Sources panel supports preview -> accept, sync, duplicates, remove with exact Rust schemas, zero innerHTML, and zero "verified"', async () => {
  const dom = makeFakeDom();
  try {
    const hostileRoot = '/tmp/<img src=x onerror=alert(9)>/vault';
    const registered = [];
    const { calls, fetchImpl } = makeFakeFetch((method, url, body) => {
      if (method === 'GET' && url === SOURCES_API_BASE) {
        return { sources: registered, count: registered.length };
      }
      if (method === 'POST' && url === SOURCES_API_BASE) {
        const src = { id: body.id || 'vault-1', kind: body.kind, root: body.root, state: 'synced' };
        if (body.accept) registered.push({ source: src, item_count: 3 });
        return {
          ok: true,
          accepted: Boolean(body.accept),
          source: src,
          report: {
            source_id: src.id,
            kind: src.kind,
            dry_run: !body.accept,
            total_items: 3,
            with_pdf: 2,
            with_tex: 1,
            added: 3,
            updated: 0,
            moved: 1,
            missing: 0,
            unchanged: 0,
            exact_duplicates: 1,
            possible_duplicates: 1,
            unreadable: 1,
            warnings: [{ path: 'broken.pdf', reason: 'encrypted stream' }],
            duplicate_groups: [{ id: 'dup-1', confidence: 'exact', signals: ['doi'], members: [] }],
            items: [],
          },
        };
      }
      if (method === 'POST' && url === '/api/sources/vault_1/sync') {
        return { ok: true, report: { total_items: 3, updated: 1, duplicate_groups: [] } };
      }
      if (method === 'POST' && url === '/api/sources/sync') {
        return { ok: true, count: registered.length, reports: [] };
      }
      if (method === 'GET' && url === '/api/sources/duplicates') {
        return {
          duplicates: {
            exact_groups: [{
              id: 'dup-exact-1',
              confidence: 'exact',
              signals: ['doi', 'arxiv'],
              canonical_key: 'arxiv:2401.00001',
              members: [{
                source_id: 'vault_1', external_id: 'k1', citekey: 'ada2024',
                title: '<script>dup</script>', item_kind: 'article',
              }],
            }],
            possible_groups: [{
              id: 'dup-fuzzy-1',
              confidence: 'possible',
              signals: ['fuzzy_title_author_year'],
              members: [{
                source_id: 'vault_1', external_id: 'k2', title: 'Fuzzy match', item_kind: 'article',
              }],
            }],
          },
        };
      }
      if (method === 'DELETE' && url === '/api/sources/vault_1') {
        registered.length = 0;
        return { ok: true, deleted: 'vault_1' };
      }
      if (method === 'GET' && url.startsWith(LIBRARY_API_BASE)) {
        return { papers: [], count: 0 };
      }
      throw new Error(`Unexpected fetch: ${method} ${url}`);
    });

    const host = dom.makeEl('div');
    const pane = mountLibraryPane(host, { autoLoad: false, fetchImpl, getToken: () => 'tok-src' });
    pane.setTab('sources');

    // Step 1: Preview (accept: false) renders ScanWarning objects cleanly (never [object Object]).
    await pane.sourcesPanel.previewSource({ kind: 'kandinsky_vault', root: hostileRoot, id: 'vault_1', hash_files: false });
    const prevCall = calls.find((c) => c.method === 'POST' && c.url === SOURCES_API_BASE && c.body.accept === false);
    assert.ok(prevCall, 'preview must POST /api/sources with accept: false');
    assert.equal(registered.length, 0);
    assert.ok(host.textContent.includes('broken.pdf: encrypted stream'));
    assert.ok(host.textContent.includes('1 exact / 1 possible duplicates'));
    assert.ok(!host.textContent.includes('[object Object]'));

    // Step 2: Explicit accept (accept: true) registers and indexes source.
    await pane.sourcesPanel.acceptPreview();
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === SOURCES_API_BASE && c.body.accept === true));
    assert.equal(registered.length, 1);

    // Step 3: Sync single source and sync all.
    await pane.sourcesPanel.syncSource('vault_1');
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/sources/vault_1/sync'));
    await pane.sourcesPanel.syncAll();
    assert.ok(calls.some((c) => c.method === 'POST' && c.url === '/api/sources/sync'));

    // Step 4: Duplicates report renders exact_groups & possible_groups members cleanly.
    await pane.sourcesPanel.loadDuplicates();
    assert.ok(host.textContent.includes('Exact match (doi, arxiv)'));
    assert.ok(host.textContent.includes('Possible duplicate (not auto-merged)'));
    assert.ok(host.textContent.includes('<script>dup</script>'));
    assert.ok(host.textContent.includes('Fuzzy match'));

    // Step 5: Remove source.
    await pane.sourcesPanel.removeSource('vault_1');
    assert.ok(calls.some((c) => c.method === 'DELETE' && c.url === '/api/sources/vault_1'));

    assert.equal(dom.innerHtmlWrites, 0, 'library_sources.js must never write innerHTML');
    assert.ok(!/\bverified\b/i.test(host.textContent), 'sources panel must not contain "verified"');
  } finally {
    dom.restore();
  }
});

test('Library pane surfaces request/empty-PDF errors visibly, gates Attach-to-session on cached blobs, and registers assets', async () => {
  const dom = makeFakeDom();
  try {
    const fetchImpl = async (url, init = {}) => {
      const method = String(init.method || 'GET').toUpperCase();
      if (method === 'GET' && url === '/api/library/arxiv%3A2401.00001') {
        return {
          ok: true,
          status: 200,
          text: async () => JSON.stringify({
            paper: { key: 'arxiv:2401.00001', title: 'Uncached Paper', ids: { arxiv: '2401.00001' } },
            seen: [],
            fulltext: [{ origin: 'user_library', source_id: 's1' }],
            findings: [],
            source_items: [],
          }),
        };
      }
      if (method === 'POST' && url === '/api/library/arxiv%3A2401.00001/mark') {
        return { ok: false, status: 500, text: async () => JSON.stringify({ error: 'disk read-only' }) };
      }
      return { ok: true, status: 200, text: async () => JSON.stringify({ papers: [] }) };
    };
    const host = dom.makeEl('div');
    const pane = mountLibraryPane(host, {
      autoLoad: false,
      fetchImpl,
      getToken: () => 'tok-err',
      getActiveSessionId: () => 's-err',
      listProjects: async () => ({ projects: [] }),
    });
    await pane.openPaperDetail('arxiv:2401.00001');
    assert.ok(!host.textContent.includes(ATTACH_SESSION_LABEL), 'Attach to session must be hidden when no blob is cached');

    await assert.rejects(
      () => pane.saveMark('arxiv:2401.00001', { status: 'key', tags: [], note: '' }),
      /disk read-only/,
    );
    assert.ok(host.textContent.includes('Save mark failed: disk read-only'));

    await assert.rejects(
      () => pane.attachPdfFile('arxiv:2401.00001', { name: 'empty.pdf', bytes: new Uint8Array(0) }),
      /empty/i,
    );
    assert.ok(host.textContent.includes('Attach PDF failed:'));

    const assetsRs = readFileSync(new URL('../src/assets.rs', import.meta.url), 'utf8');
    assert.ok(assetsRs.includes('"/assets/library.js"'), 'assets.rs must register /assets/library.js');
    assert.ok(assetsRs.includes('"/assets/library_sources.js"'), 'assets.rs must register /assets/library_sources.js');
  } finally {
    dom.restore();
  }
});

