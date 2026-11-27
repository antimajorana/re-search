import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  DEFAULT_OCR_ENABLED,
  OCR_INSTALL_PROMPT,
  createOcrSettingsPanel,
  createOcrStatusBanner,
  loadOcrSettings,
  normalizeOcrStatus,
  saveOcrSettings,
} from '../assets/ocr_settings.js';
import {
  OCR_MATH_NOTICE,
  bindQuoteChipPageImageViewer,
  buildPageImageUrl,
  createPageImageViewer,
} from '../assets/page_image_viewer.js';
import { MAX_QUOTE_CHARS, createQuoteChip } from '../assets/quote_chip.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    return {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      dataset: {},
      style: {},
      type: '', name: '', value: '', checked: false, src: '', alt: '',
      placeholder: '', title: '', hidden: false, disabled: false,
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
      get textContent() { return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text; },
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
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) fn(ev);
      },
      click() { if (!this.disabled) this.dispatch('click'); },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const attr = sel.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (attr) {
            const val = n.getAttribute(attr[1]);
            return attr[2] === undefined ? val !== null : val === attr[2];
          }
          return n.tagName === sel.toUpperCase();
        };
        const visit = (n) => { for (const c of n.children || []) { if (match(c)) out.push(c); visit(c); } };
        visit(this);
        return out;
      },
    };
  };
  const prevDoc = globalThis.document;
  globalThis.document = { createElement: (tag) => makeEl(tag, null), createElementNS: (ns, tag) => makeEl(tag, ns) };
  return { makeEl, get innerHtmlWrites() { return innerHtmlWrites; }, restore() { globalThis.document = prevDoc; } };
}

function walkTree(node, fn) {
  fn(node);
  for (const child of node.children || []) walkTree(child, fn);
}

function makeFakeUrlApi() {
  let seq = 0;
  const created = [];
  const revoked = [];
  return {
    created,
    revoked,
    createObjectURL(blob) { const url = `blob:test/${++seq}/${blob?.size ?? 0}`; created.push(url); return url; },
    revokeObjectURL(url) { revoked.push(url); },
  };
}

test('ocr_settings: missing vs available status, default enabled=true, save/refresh, and API helpers', async () => {
  const dom = makeFakeDom();
  try {
    assert.equal(DEFAULT_OCR_ENABLED, true);
    assert.equal(OCR_INSTALL_PROMPT, 'Install Tesseract to enable OCR');

    const missing = normalizeOcrStatus({ available: false });
    assert.equal(missing.enabled, true);
    assert.equal(missing.available, false);
    assert.equal(missing.message, 'Install Tesseract to enable OCR');

    const missingBanner = createOcrStatusBanner(missing);
    assert.equal(missingBanner.getAttribute('data-ocr-available'), 'false');
    assert.match(missingBanner.textContent, /Install Tesseract to enable OCR/);
    assert.ok(missingBanner.querySelector('.ui-badge'));

    const saved = [];
    let refreshed = 0;
    const panel = createOcrSettingsPanel({
      onSave: (v) => saved.push(v),
      onRefresh: () => { refreshed += 1; },
    });
    assert.equal(panel.element.getAttribute('data-ocr-available'), 'false');
    assert.equal(panel.getValues().enabled, true);
    assert.match(panel.element.textContent, /Install Tesseract to enable OCR/);

    panel.update({
      enabled: true,
      available: true,
      tesseract_path: '/usr/bin/tesseract',
      tessdata_dir: '/usr/share/tessdata',
      langs: ['eng', 'deu', 'fra'],
      version: '5.3.4',
    });
    assert.equal(panel.element.getAttribute('data-ocr-available'), 'true');
    assert.match(panel.element.textContent, /5\.3\.4/);
    assert.match(panel.element.textContent, /3 languages/);
    assert.equal(panel.element.querySelector('[data-ocr-detected="tesseract_path"]').textContent, '/usr/bin/tesseract');

    panel.element.querySelector('[data-ocr-field="tesseract_path"]').value = '  /opt/bin/tesseract  ';
    panel.element.querySelector('[data-ocr-field="tessdata_dir"]').value = '  /opt/tessdata  ';
    panel.element.querySelector('[data-ocr-field="langs"]').value = ' eng, deu , eng, spa ';
    panel.element.querySelector('[data-ocr-field="enabled"]').checked = false;

    panel.element.querySelector('[data-ocr-action="save"]').click();
    panel.element.querySelector('[data-ocr-action="refresh"]').click();
    assert.equal(refreshed, 1);
    assert.deepEqual(saved, [{
      enabled: false,
      tesseract_path: '/opt/bin/tesseract',
      tessdata_dir: '/opt/tessdata',
      langs: ['eng', 'deu', 'spa'],
    }]);

    panel.setBusy(true);
    assert.equal(panel.element.querySelector('[data-ocr-action="save"]').disabled, true);
    panel.setFeedback('Saved locally.', false);
    assert.equal(panel.element.querySelector('.ocr-settings-feedback').textContent, 'Saved locally.');

    const apiCalls = [];
    const fakeApi = async (url, init) => {
      apiCalls.push({ url, init });
      return { ok: true, json: async () => ({ enabled: true, available: true, version: '5.4.0', langs: ['eng'] }) };
    };
    const loaded = await loadOcrSettings(fakeApi);
    assert.equal(loaded.version, '5.4.0');
    await saveOcrSettings(fakeApi, { enabled: true, tesseract_path: ' /bin/tess ', tessdata_dir: '', langs: 'eng, fra' });
    assert.equal(apiCalls[0].url, '/api/settings/ocr');
    assert.equal(apiCalls[0].init.method, 'GET');
    assert.equal(apiCalls[1].url, '/api/settings/ocr');
    assert.equal(apiCalls[1].init.method, 'PUT');
    assert.deepEqual(JSON.parse(apiCalls[1].init.body), {
      enabled: true, tesseract_path: '/bin/tess', tessdata_dir: '', langs: ['eng', 'fra'],
    });
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('ocr_settings: XSS payloads in path/message/version write zero innerHTML and inject zero elements', () => {
  const dom = makeFakeDom();
  try {
    const xss1 = '<img src=x onerror=alert(1)>';
    const xss2 = '<script>alert("xss")</script>';
    const panel = createOcrSettingsPanel({
      initial: {
        available: false,
        tesseract_path: xss1,
        tessdata_dir: xss2,
        version: xss1,
        message: xss2,
        langs: [xss1],
      },
    });
    panel.setFeedback(xss1, true);
    assert.equal(dom.innerHtmlWrites, 0);
    const forbidden = new Set(['IMG', 'SCRIPT', 'IFRAME', 'A', 'OBJECT', 'EMBED']);
    walkTree(panel.element, (el) => {
      assert.equal(forbidden.has(el.tagName), false, `unexpected injected tag <${el.tagName}>`);
      for (const [k] of el.attributes.entries()) assert.equal(/^on/i.test(k), false);
    });
    assert.ok(panel.element.textContent.includes(OCR_INSTALL_PROMPT));
    assert.ok(panel.element.textContent.includes(xss1));
    assert.ok(panel.element.textContent.includes(xss2));
  } finally {
    dom.restore();
  }
});

test('page_image_viewer: URL encoding/validation, auth header, Blob URL lifecycle, XSS quotes, and errors', async () => {
  const dom = makeFakeDom();
  try {
    assert.equal(
      buildPageImageUrl('sess/1', 'src 2', { page: 3, dpi: 150, crop: [10, 20, 300, 400] }),
      '/api/sessions/sess%2F1/sources/src%202/page-image?page=3&dpi=150&crop=10,20,300,400',
    );
    assert.throws(() => buildPageImageUrl('s1', 'src1', { page: 0 }), RangeError);
    assert.throws(() => buildPageImageUrl('s1', 'src1', { page: 1.5 }), RangeError);
    assert.throws(() => buildPageImageUrl('s1', 'src1', { page: 1, dpi: -10 }), RangeError);
    assert.throws(() => buildPageImageUrl('s1', 'src1', { page: 1, crop: [0, 0, NaN, 10] }), RangeError);
    assert.throws(() => buildPageImageUrl('s1', 'src1', { page: 1, crop: [0, 0, 0, 10] }), RangeError);

    const requests = [];
    let failNext = false;
    const urlApi = makeFakeUrlApi();
    const viewer = createPageImageViewer({
      getToken: () => 'secret-csrf-token',
      urlApi,
      fetchImpl: async (url, init) => {
        requests.push({ url, init });
        if (failNext) return { ok: false, status: 404 };
        return { ok: true, status: 200, blob: async () => ({ size: 128, type: 'image/png' }) };
      },
    });

    const xssQuote = '<img src=x onerror=alert(1)> $$\\frac{1}{2}$$ ' + 'Q'.repeat(350);
    const st1 = await viewer.open({
      sessionId: 'sess-1', sourceId: 'src-1', page: 4, pageCount: 10, quote: xssQuote, ocr: true,
    });
    assert.equal(st1.open, true);
    assert.equal(requests[0].url, '/api/sessions/sess-1/sources/src-1/page-image?page=4');
    assert.equal(requests[0].init.headers['X-ReSearch-Token'], 'secret-csrf-token');
    assert.equal(urlApi.created.length, 1);
    assert.equal(urlApi.revoked.length, 0);
    assert.equal(viewer.element.querySelector('.page-image-viewer-title').textContent, 'matched in OCR text, p. 4');
    assert.equal(viewer.element.querySelector('.ui-badge-text').textContent, 'OCR');
    assert.equal(viewer.element.querySelector('.page-image-viewer-notice').textContent, OCR_MATH_NOTICE);
    const quoteEl = viewer.element.querySelector('.page-image-viewer-quote');
    assert.equal(quoteEl.textContent.length, MAX_QUOTE_CHARS);
    assert.ok(quoteEl.textContent.startsWith('<img src=x onerror=alert(1)>'));
    assert.equal(viewer.element.querySelector('img').getAttribute('alt'), 'Original scanned PDF page 4');
    assert.equal(viewer.element.querySelector('img').src, urlApi.created[0]);

    // Re-open revokes previous object URL.
    await viewer.open({ sessionId: 'sess-1', sourceId: 'src-1', page: 5, ocr: false });
    assert.deepEqual(urlApi.revoked, [urlApi.created[0]]);
    assert.equal(viewer.element.querySelector('.page-image-viewer-title').textContent, 'Page 5');

    // Error handling revokes active URL and reports honest error.
    failNext = true;
    const errState = await viewer.open({ sessionId: 'sess-1', sourceId: 'src-1', page: 6 });
    assert.match(errState.error, /HTTP 404/);
    assert.deepEqual(urlApi.revoked, [urlApi.created[0], urlApi.created[1]]);
    assert.equal(viewer.element.querySelector('img').hidden, true);

    // Close hides viewer and ensures no leaked object URLs.
    failNext = false;
    await viewer.open({ sessionId: 'sess-1', sourceId: 'src-1', page: 2 });
    const thirdUrl = urlApi.created[2];
    viewer.close();
    assert.equal(viewer.element.hidden, true);
    assert.ok(urlApi.revoked.includes(thirdUrl));
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('integration: quote_chip wired via bindQuoteChipPageImageViewer opens scanned page image', async () => {
  const dom = makeFakeDom();
  try {
    const fetched = [];
    const urlApi = makeFakeUrlApi();
    const viewer = createPageImageViewer({
      getToken: () => 'tok-42',
      urlApi,
      fetchImpl: async (url, init) => {
        fetched.push({ url, init });
        return { ok: true, status: 200, blob: async () => ({ size: 64 }) };
      },
    });
    const spec = { ocr: true, page: 4, source_id: 'src-1', quote: 'Theorem 2.1 (scanned proof excerpt)' };
    const chip = createQuoteChip(spec, {
      onOpenPageImage: bindQuoteChipPageImageViewer(viewer, 'sess-1', { quote: spec.quote }),
    });
    chip.querySelector('.ui-quote-chip-open').click();
    await new Promise((r) => setTimeout(r, 0));

    assert.equal(fetched.length, 1);
    assert.equal(fetched[0].url, '/api/sessions/sess-1/sources/src-1/page-image?page=4');
    assert.equal(fetched[0].init.headers['X-ReSearch-Token'], 'tok-42');
    assert.equal(viewer.element.hidden, false);
    assert.equal(viewer.element.querySelector('.page-image-viewer-title').textContent, 'matched in OCR text, p. 4');
    assert.equal(viewer.element.querySelector('.page-image-viewer-quote').textContent, spec.quote);
    assert.equal(dom.innerHtmlWrites, 0);

    const here = path.dirname(fileURLToPath(import.meta.url));
    for (const file of ['ocr_settings.js', 'page_image_viewer.js']) {
      const src = fs.readFileSync(path.resolve(here, '..', 'assets', file), 'utf8');
      assert.equal(/innerHTML\s*=/.test(src), false, `${file} must never assign innerHTML`);
    }
  } finally {
    dom.restore();
  }
});
