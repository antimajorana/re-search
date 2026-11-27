import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  ALLOWED_CROP_MEDIA_TYPES,
  DEFAULT_EQUATION_TRANSCRIPTION_ENABLED,
  MODEL_TRANSCRIBED_LABEL,
  TRANSCRIPTION_AGREED_NOTE,
  TRANSCRIPTION_DISAGREED_NOTE,
  TRANSCRIPTION_NON_QUOTABLE_NOTE,
  mountFormulaTranscriptionPanel,
  renderFormulaTranscriptionCard,
  requestEquationTranscription,
  safeCropDataUrl,
  transcribeEquationsUrl,
} from '../assets/formula_transcription.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const SAMPLE_PNG_B64 = Buffer.from([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
  0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
]).toString('base64');

function createStrictMockDom() {
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
      checked: false,
      disabled: false,
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
      dispatch(evt) {
        for (const fn of listeners.get(evt) || []) fn({ target: el });
      },
    };
    return el;
  }

  const doc = {
    createElement(tag) {
      const node = makeElement(tag);
      node.ownerDocument = doc;
      return node;
    },
  };
  return { doc };
}

test('constants, safeCropDataUrl, and URL encoding enforce Owner Decision 2026-11-26', () => {
  assert.equal(DEFAULT_EQUATION_TRANSCRIPTION_ENABLED, false);
  assert.equal(MODEL_TRANSCRIBED_LABEL, 'model-transcribed');
  assert.deepEqual(ALLOWED_CROP_MEDIA_TYPES, ['image/png', 'image/jpeg', 'image/webp']);

  assert.equal(
    safeCropDataUrl('image/png', SAMPLE_PNG_B64),
    `data:image/png;base64,${SAMPLE_PNG_B64}`,
  );
  assert.equal(safeCropDataUrl('image/svg+xml', SAMPLE_PNG_B64), null);
  assert.equal(safeCropDataUrl('text/html', SAMPLE_PNG_B64), null);
  assert.equal(safeCropDataUrl('javascript:alert(1)', SAMPLE_PNG_B64), null);
  assert.equal(safeCropDataUrl('image/png', 'not valid base64!'), null);
  assert.equal(safeCropDataUrl('image/png', ''), null);

  assert.equal(
    transcribeEquationsUrl('s-1/a', 'src 2'),
    '/api/sessions/s-1%2Fa/sources/src%202/transcribe-equations',
  );
});

test('requestEquationTranscription refuses unless explicit optIn === true', async () => {
  let called = false;
  const mockFetch = async () => {
    called = true;
    return { ok: true, status: 200, async json() { return {}; } };
  };

  await assert.rejects(
    () =>
      requestEquationTranscription('s-1', 'paper1', {
        page: 1,
        fetchFn: mockFetch,
        token: 'tok',
      }),
    /off by default/,
  );
  await assert.rejects(
    () =>
      requestEquationTranscription('s-1', 'paper1', {
        page: 1,
        optIn: false,
        fetchFn: mockFetch,
        token: 'tok',
      }),
    /off by default/,
  );
  assert.equal(called, false, 'fetch must not run when optIn is not explicitly true');
});

test('renderFormulaTranscriptionCard renders crop beside model-transcribed draft with XSS safety', () => {
  const { doc } = createStrictMockDom();
  const hostileLatex = '\\frac{a}{b} <script>alert(1)</script><img src=x onerror=alert(2)>';
  const card = renderFormulaTranscriptionCard(
    {
      source_id: 'paper1',
      page: 3,
      crop: [10, 20, 300, 90],
      model: 'gpt-4o<svg onload=alert(1)>',
      latex_draft: hostileLatex,
      label: 'forged-verified-label',
      agreed_with_text_layer: false,
    },
    {
      document: doc,
      cropMediaType: 'image/png',
      cropBase64: SAMPLE_PNG_B64,
    },
  );

  assert.ok(card);
  // Owner-mandated label is enforced even if untrusted record attempted to forge it.
  assert.equal(card.getAttribute('data-label'), MODEL_TRANSCRIBED_LABEL);
  assert.equal(card.getAttribute('data-quotable'), 'false');
  assert.equal(card.getAttribute('data-agreed-with-text-layer'), 'false');

  const labelEl = card.querySelector('[data-testid="transcription-label"]');
  assert.equal(labelEl.textContent, MODEL_TRANSCRIBED_LABEL);

  const img = card.querySelector('[data-testid="transcription-crop-img"]');
  assert.ok(img);
  assert.equal(img.getAttribute('src'), `data:image/png;base64,${SAMPLE_PNG_B64}`);
  assert.equal(img.getAttribute('alt'), 'Equation crop from page 3');

  const draftEl = card.querySelector('[data-testid="transcription-latex-draft"]');
  assert.equal(draftEl.textContent, hostileLatex);

  const noteEl = card.querySelector('[data-testid="transcription-honesty-note"]');
  assert.equal(noteEl.textContent, TRANSCRIPTION_DISAGREED_NOTE);

  // Agreeing text layer still keeps data-quotable="false" and directs user to text-layer quote.
  const agreedCard = renderFormulaTranscriptionCard(
    {
      page: 1,
      model: 'claude-sonnet-4-5',
      latex_draft: 'E = m c^2',
      agreed_with_text_layer: true,
    },
    {
      document: doc,
      cropMediaType: 'image/svg+xml',
      cropBase64: SAMPLE_PNG_B64,
    },
  );
  assert.equal(agreedCard.getAttribute('data-quotable'), 'false');
  assert.equal(agreedCard.getAttribute('data-agreed-with-text-layer'), 'true');
  assert.equal(
    agreedCard.querySelector('[data-testid="transcription-honesty-note"]').textContent,
    TRANSCRIPTION_AGREED_NOTE,
  );
  // Disallowed SVG MIME falls back to safe placeholder with zero <img>.
  assert.equal(agreedCard.querySelector('[data-testid="transcription-crop-img"]'), null);
  assert.ok(agreedCard.querySelector('[data-testid="transcription-crop-missing"]'));
});

test('mountFormulaTranscriptionPanel is off by default, sends X-ReSearch-Token after opt-in, and renders card', async () => {
  const { doc } = createStrictMockDom();
  const host = doc.createElement('div');
  const calls = [];

  const mockFetch = async (url, init) => {
    calls.push({ url, init });
    return {
      ok: true,
      status: 200,
      async json() {
        return {
          label: MODEL_TRANSCRIBED_LABEL,
          quotable: false,
          transcription: {
            source_id: 'paper1',
            page: 2,
            crop: [5, 10, 200, 60],
            model: 'gpt-4o',
            latex_draft: 'H = p^2 / (2 m)',
            label: MODEL_TRANSCRIBED_LABEL,
            agreed_with_text_layer: false,
          },
        };
      },
    };
  };

  const panel = mountFormulaTranscriptionPanel(host, {
    sessionId: 's-42',
    sourceId: 'paper1',
    document: doc,
    fetchFn: mockFetch,
    getToken: () => 'tok-vision-secret',
  });

  assert.ok(panel);
  assert.equal(panel.optInCheckbox.checked, false);
  assert.equal(panel.transcribeBtn.disabled, true);
  assert.equal(
    panel.root.querySelector('[data-testid="transcription-disclaimer"]').textContent,
    TRANSCRIPTION_NON_QUOTABLE_NOTE,
  );

  // Attempting to transcribe before opt-in refuses without calling fetch.
  const denied = await panel.transcribe({
    page: 2,
    imageMediaType: 'image/png',
    imageBase64: SAMPLE_PNG_B64,
  });
  assert.equal(denied, null);
  assert.equal(calls.length, 0);

  // User checks explicit opt-in -> button enables -> POST sends header token & renders card.
  panel.optInCheckbox.checked = true;
  panel.optInCheckbox.dispatch('change');
  assert.equal(panel.transcribeBtn.disabled, false);

  const res = await panel.transcribe({
    page: 2,
    crop: [5, 10, 200, 60],
    imageMediaType: 'image/png',
    imageBase64: SAMPLE_PNG_B64,
    provider: 'openai',
    model: 'gpt-4o',
  });
  assert.ok(res);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, '/api/sessions/s-42/sources/paper1/transcribe-equations');
  assert.equal(calls[0].init.method, 'POST');
  assert.equal(calls[0].init.headers['X-ReSearch-Token'], 'tok-vision-secret');
  const sentBody = JSON.parse(calls[0].init.body);
  assert.equal(sentBody.opt_in, true);
  assert.equal(sentBody.page, 2);

  assert.equal(panel.cardHost.children.length, 1);
  const card = panel.cardHost.children[0];
  assert.equal(card.getAttribute('data-label'), MODEL_TRANSCRIBED_LABEL);
  assert.equal(card.getAttribute('data-quotable'), 'false');

  panel.destroy();
  assert.equal(host.children.length, 0);
});

test('assets.rs registers /assets/formula_transcription.js and auth.rs never exempts transcribe-equations', () => {
  const assetsRs = fs.readFileSync(
    path.join(REPO_ROOT, 'crates', 'desktop-server', 'src', 'assets.rs'),
    'utf8',
  );
  assert.ok(assetsRs.includes('"/assets/formula_transcription.js"'));
  const authRs = fs.readFileSync(
    path.join(REPO_ROOT, 'crates', 'desktop-server', 'src', 'auth.rs'),
    'utf8',
  );
  assert.ok(!authRs.includes('transcribe-equations'));
});
