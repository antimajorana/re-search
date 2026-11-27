// Opt-in PDF equation vision transcription UI (package F14c, Owner Decision 2026-11-26).
// Strictly DOM-based (`createElement` / `textContent` only; zero `innerHTML`).
//
// Honesty constraints:
// 1. Off by default (`DEFAULT_EQUATION_TRANSCRIPTION_ENABLED === false`).
// 2. Every draft card is unconditionally labelled `"model-transcribed"` beside the page crop.
// 3. Always marked non-quotable (`data-quotable="false"`): never a matched quote unless the
//    pinned text layer agrees and its text lines are verified via `quote_core::accept_quote`.

export const DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false;
export const MODEL_TRANSCRIBED_LABEL = 'model-transcribed';
export const ALLOWED_CROP_MEDIA_TYPES = Object.freeze(['image/png', 'image/jpeg', 'image/webp']);

export const TRANSCRIPTION_OPT_IN_LABEL =
  'Send equation page crop to my vision model (off by default)';
export const TRANSCRIPTION_NON_QUOTABLE_NOTE =
  'Model-transcribed draft is never a matched quote unless the pinned text layer agrees.';
export const TRANSCRIPTION_AGREED_NOTE =
  'Pinned text layer agrees on this page — cite the matched text-layer quote, not this draft.';
export const TRANSCRIPTION_DISAGREED_NOTE =
  'Pinned text layer unavailable or disagrees — cannot be cited as a matched quote.';

const STRICT_BASE64_RE = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function resolveDoc(container, opts) {
  if (opts?.document) return opts.document;
  if (container?.ownerDocument) return container.ownerDocument;
  if (typeof document !== 'undefined') return document;
  return null;
}

function resolveToken(opts) {
  if (typeof opts?.getToken === 'function') {
    const t = String(opts.getToken() || '').trim();
    if (t) return t;
  }
  if (opts?.token) {
    const t = String(opts.token).trim();
    if (t) return t;
  }
  if (typeof document !== 'undefined' && typeof document.querySelector === 'function') {
    const meta = document.querySelector('meta[name="re-search-csrf"]');
    if (meta && typeof meta.content === 'string' && meta.content.trim()) {
      return meta.content.trim();
    }
  }
  try {
    if (typeof window !== 'undefined' && window?.sessionStorage) {
      return String(window.sessionStorage.getItem('re-search-token') || '').trim();
    }
  } catch {
    // sessionStorage unavailable
  }
  return '';
}

/**
 * Validate a crop image MIME type and strict base64 payload before constructing a `data:` URI.
 * Rejects SVG, HTML, javascript: URLs, whitespace, and malformed base64.
 */
export function safeCropDataUrl(mediaType, base64) {
  const mt = String(mediaType || '').trim();
  if (!ALLOWED_CROP_MEDIA_TYPES.includes(mt)) return null;
  const b64 = String(base64 || '');
  if (!b64 || b64.length % 4 !== 0 || !STRICT_BASE64_RE.test(b64)) return null;
  return `data:${mt};base64,${b64}`;
}

export function transcribeEquationsUrl(sessionId, sourceId) {
  const sid = encodeURIComponent(String(sessionId || '').trim());
  const src = encodeURIComponent(String(sourceId || '').trim());
  return `/api/sessions/${sid}/sources/${src}/transcribe-equations`;
}

async function readErrorMessage(res) {
  try {
    if (typeof res.text === 'function') {
      const text = await res.text();
      if (text) {
        try {
          const parsed = JSON.parse(text);
          if (parsed && typeof parsed.error === 'string' && parsed.error.trim()) {
            return parsed.error.trim();
          }
        } catch {
          return text.trim();
        }
      }
    } else if (typeof res.json === 'function') {
      const parsed = await res.json();
      if (parsed && typeof parsed.error === 'string' && parsed.error.trim()) {
        return parsed.error.trim();
      }
    }
  } catch {
    // Fall back to HTTP status.
  }
  return `Equation transcription request failed (HTTP ${res?.status ?? 0})`;
}

export async function requestEquationTranscription(sessionId, sourceId, opts = {}) {
  const sid = String(sessionId || '').trim();
  const src = String(sourceId || '').trim();
  if (!sid || !src) {
    throw new Error('Select a session and PDF source before transcribing equations.');
  }
  if (opts.optIn !== true) {
    throw new Error(
      'Equation vision transcription is off by default; explicitly opt in before sending page images.',
    );
  }
  const page = Number(opts.page);
  if (!Number.isInteger(page) || page < 1) {
    throw new Error('Provide a 1-based page number (>= 1).');
  }
  const fetchFn = opts.fetchFn || (typeof fetch === 'function' ? fetch : null);
  if (typeof fetchFn !== 'function') {
    throw new Error('Equation transcription fetch unavailable.');
  }

  const payload = { page, opt_in: true };
  if (Array.isArray(opts.crop) && opts.crop.length === 4) {
    payload.crop = opts.crop.map((n) => Math.floor(Number(n) || 0));
  }
  if (opts.imageMediaType !== undefined || opts.imageBase64 !== undefined) {
    if (!safeCropDataUrl(opts.imageMediaType, opts.imageBase64)) {
      throw new Error('Crop image must be valid PNG, JPEG, or WebP base64.');
    }
    payload.image_media_type = String(opts.imageMediaType).trim();
    payload.image_base64 = String(opts.imageBase64);
  }
  if (opts.provider) payload.provider = String(opts.provider).trim();
  if (opts.model) payload.model = String(opts.model).trim();
  if (opts.runId) payload.run_id = String(opts.runId).trim();
  if (Number.isInteger(opts.budgetCap) && opts.budgetCap >= 0) {
    payload.budget_cap = opts.budgetCap;
  }
  if (opts.promptHint) payload.prompt_hint = String(opts.promptHint).trim();

  const headers = { 'Content-Type': 'application/json' };
  const token = resolveToken(opts);
  if (token) headers['X-ReSearch-Token'] = token;

  const res = await fetchFn(transcribeEquationsUrl(sid, src), {
    method: 'POST',
    credentials: 'same-origin',
    headers,
    body: JSON.stringify(payload),
  });
  if (!res || !res.ok) {
    const err = new Error(await readErrorMessage(res));
    err.status = res?.status ?? 0;
    throw err;
  }
  return typeof res.json === 'function' ? await res.json() : JSON.parse(await res.text());
}

/**
 * Render a side-by-side equation transcription card: page crop on the left, `"model-transcribed"`
 * LaTeX draft on the right. Uses strictly `textContent` and validated `data:` image URLs.
 */
export function renderFormulaTranscriptionCard(record = {}, opts = {}) {
  const doc = resolveDoc(null, opts);
  if (!doc) return null;

  const page = Number.isInteger(record.page) && record.page >= 1 ? record.page : null;
  const agreed = record.agreed_with_text_layer === true;
  const modelName = String(record.model || '').trim();
  const latexDraft = String(record.latex_draft ?? '');
  const cropUrl = safeCropDataUrl(
    opts.cropMediaType ?? record.image_media_type,
    opts.cropBase64 ?? record.image_base64,
  );

  const card = el(doc, 'section', 'formula-transcription-card');
  card.setAttribute('data-testid', 'formula-transcription-card');
  // Always stamped with the canonical owner-mandated label regardless of untrusted input.
  card.setAttribute('data-label', MODEL_TRANSCRIBED_LABEL);
  card.setAttribute('data-quotable', 'false');
  card.setAttribute('data-agreed-with-text-layer', agreed ? 'true' : 'false');

  const header = el(doc, 'div', 'formula-transcription-header');
  const badge = el(doc, 'span', 'formula-transcription-badge', MODEL_TRANSCRIBED_LABEL);
  badge.setAttribute('data-testid', 'transcription-label');
  header.appendChild(badge);

  const metaParts = [];
  if (page !== null) metaParts.push(`p. ${page}`);
  if (Array.isArray(record.crop) && record.crop.length === 4) {
    metaParts.push(`crop ${record.crop.join(',')}`);
  }
  if (modelName) metaParts.push(modelName);
  if (metaParts.length > 0) {
    const metaEl = el(doc, 'span', 'formula-transcription-meta', metaParts.join(' · '));
    metaEl.setAttribute('data-testid', 'transcription-meta');
    header.appendChild(metaEl);
  }

  const agreeBadge = el(
    doc,
    'span',
    agreed ? 'formula-transcription-agree formula-transcription-agree--yes' : 'formula-transcription-agree formula-transcription-agree--no',
    agreed ? 'Text layer agrees (cite text layer)' : 'Not a matched quote',
  );
  agreeBadge.setAttribute('data-testid', 'transcription-agreement');
  header.appendChild(agreeBadge);
  card.appendChild(header);

  const body = el(doc, 'div', 'formula-transcription-split');
  body.setAttribute('data-testid', 'transcription-split');

  const cropPane = el(doc, 'div', 'formula-transcription-crop-pane');
  cropPane.setAttribute('data-testid', 'transcription-crop-pane');
  if (cropUrl) {
    const img = el(doc, 'img', 'formula-transcription-crop-img');
    img.setAttribute('data-testid', 'transcription-crop-img');
    img.setAttribute('src', cropUrl);
    img.setAttribute('alt', page !== null ? `Equation crop from page ${page}` : 'Equation page crop');
    cropPane.appendChild(img);
  } else {
    const fallback = el(doc, 'div', 'formula-transcription-crop-missing', 'Page crop unavailable');
    fallback.setAttribute('data-testid', 'transcription-crop-missing');
    cropPane.appendChild(fallback);
  }
  body.appendChild(cropPane);

  const draftPane = el(doc, 'div', 'formula-transcription-draft-pane');
  draftPane.setAttribute('data-testid', 'transcription-draft-pane');
  const draftPre = el(doc, 'pre', 'formula-transcription-latex-pre');
  const draftCode = el(doc, 'code', 'formula-transcription-latex-code', latexDraft);
  draftCode.setAttribute('data-testid', 'transcription-latex-draft');
  draftPre.appendChild(draftCode);
  draftPane.appendChild(draftPre);

  const honestyNote = el(
    doc,
    'p',
    'formula-transcription-note',
    agreed ? TRANSCRIPTION_AGREED_NOTE : TRANSCRIPTION_DISAGREED_NOTE,
  );
  honestyNote.setAttribute('data-testid', 'transcription-honesty-note');
  draftPane.appendChild(honestyNote);

  body.appendChild(draftPane);
  card.appendChild(body);
  return card;
}

/**
 * Mount an interactive off-by-default equation transcription panel into `container`.
 */
export function mountFormulaTranscriptionPanel(container, opts = {}) {
  const doc = resolveDoc(container, opts);
  if (!container || !doc) return null;

  let currentSessionId = String(opts.sessionId || '').trim();
  let currentSourceId = String(opts.sourceId || '').trim();
  let busy = false;
  let destroyed = false;
  let reqSeq = 0;

  const existing = container.querySelector?.('[data-testid="formula-transcription-panel"]');
  existing?.remove?.();

  const root = el(doc, 'div', 'formula-transcription-panel');
  root.setAttribute('data-testid', 'formula-transcription-panel');
  root.setAttribute('data-default-enabled', String(DEFAULT_EQUATION_TRANSCRIPTION_ENABLED));

  const optInLabel = el(doc, 'label', 'formula-transcription-optin-label');
  const optInCheckbox = el(doc, 'input', 'formula-transcription-optin');
  optInCheckbox.type = 'checkbox';
  // Strictly off by default unless caller explicitly seeds persisted user opt-in.
  optInCheckbox.checked = opts.initialOptIn === true ? true : DEFAULT_EQUATION_TRANSCRIPTION_ENABLED;
  optInCheckbox.setAttribute('data-testid', 'transcription-opt-in');
  optInLabel.appendChild(optInCheckbox);
  optInLabel.appendChild(el(doc, 'span', null, TRANSCRIPTION_OPT_IN_LABEL));

  const transcribeBtn = el(doc, 'button', 'formula-transcription-btn', 'Transcribe equation');
  transcribeBtn.type = 'button';
  transcribeBtn.setAttribute('data-action', 'transcribe-equation');

  const statusEl = el(doc, 'span', 'formula-transcription-status');
  statusEl.setAttribute('role', 'status');
  statusEl.setAttribute('data-testid', 'transcription-status');

  const disclaimerEl = el(doc, 'p', 'formula-transcription-disclaimer', TRANSCRIPTION_NON_QUOTABLE_NOTE);
  disclaimerEl.setAttribute('data-testid', 'transcription-disclaimer');

  const cardHost = el(doc, 'div', 'formula-transcription-cards');
  cardHost.setAttribute('data-testid', 'transcription-card-host');

  function syncButtonState() {
    transcribeBtn.disabled =
      busy || !optInCheckbox.checked || !currentSessionId || !currentSourceId;
  }

  function setStatus(text, isError = false) {
    statusEl.textContent = text ? String(text) : '';
    statusEl.className = isError
      ? 'formula-transcription-status formula-transcription-status--error'
      : 'formula-transcription-status';
  }

  optInCheckbox.addEventListener('change', () => {
    syncButtonState();
  });

  async function transcribe(params = {}) {
    if (destroyed || busy) return null;
    if (!optInCheckbox.checked) {
      setStatus('Enable opt-in before sending page crops to a vision model.', true);
      return null;
    }
    const seq = ++reqSeq;
    const sid = currentSessionId;
    const src = currentSourceId;
    busy = true;
    syncButtonState();
    setStatus('Transcribing equation crop...');
    try {
      const data = await requestEquationTranscription(sid, src, {
        ...params,
        optIn: true,
        fetchFn: opts.fetchFn,
        getToken: opts.getToken,
        token: opts.token,
      });
      if (destroyed || seq !== reqSeq || sid !== currentSessionId || src !== currentSourceId) {
        return null;
      }
      const record = data?.transcription || data;
      const card = renderFormulaTranscriptionCard(record, {
        document: doc,
        cropMediaType: params.imageMediaType,
        cropBase64: params.imageBase64,
      });
      if (card) cardHost.appendChild(card);
      setStatus(`Recorded ${MODEL_TRANSCRIBED_LABEL} draft (not a matched quote).`);
      opts.onTranscribed?.(data);
      return data;
    } catch (err) {
      if (destroyed || seq !== reqSeq || sid !== currentSessionId || src !== currentSourceId) {
        return null;
      }
      setStatus(err?.message || 'Equation transcription failed.', true);
      opts.onError?.(err);
      return null;
    } finally {
      if (seq === reqSeq) {
        busy = false;
        if (!destroyed) syncButtonState();
      }
    }
  }

  syncButtonState();
  root.appendChild(optInLabel);
  root.appendChild(transcribeBtn);
  root.appendChild(statusEl);
  root.appendChild(disclaimerEl);
  root.appendChild(cardHost);
  container.appendChild(root);

  return {
    root,
    optInCheckbox,
    transcribeBtn,
    statusEl,
    cardHost,
    transcribe,
    update(next = {}) {
      if (destroyed) return;
      if (next.sessionId !== undefined || next.sourceId !== undefined) {
        const nextSid = next.sessionId !== undefined ? String(next.sessionId || '').trim() : currentSessionId;
        const nextSrc = next.sourceId !== undefined ? String(next.sourceId || '').trim() : currentSourceId;
        if (nextSid !== currentSessionId || nextSrc !== currentSourceId) {
          currentSessionId = nextSid;
          currentSourceId = nextSrc;
          reqSeq += 1;
          busy = false;
          setStatus('');
        }
      }
      if (next.optIn !== undefined) {
        optInCheckbox.checked = Boolean(next.optIn);
      }
      syncButtonState();
    },
    destroy() {
      destroyed = true;
      reqSeq += 1;
      root.remove?.();
    },
  };
}
