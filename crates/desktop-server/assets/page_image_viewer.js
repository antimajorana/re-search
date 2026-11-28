// F14b scanned PDF page image viewer with authenticated Blob loading and object URL cleanup.
// OCR text is strictly plain text (textContent only, never rendered as KaTeX math; zero innerHTML writes).
import { BORDER_SOLID, OCR_TAG_WORD, createBadge } from './ui_badge.js';
import { QUOTE_OCR_MATCHED_PREFIX, clampQuoteText } from './quote_chip.js';

export const OCR_MATH_NOTICE = 'OCR text is displayed as plain text and is never rendered as mathematics.';

function el(tag, className, text = null, attrs = null) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v !== undefined && v !== null) node.setAttribute(k, String(v));
    }
  }
  if (text !== null && text !== undefined) node.textContent = String(text);
  return node;
}

function setHidden(node, hidden) {
  node.hidden = Boolean(hidden);
  if (hidden) node.setAttribute('hidden', '');
  else node.removeAttribute('hidden');
}

function requireNonEmptyId(value, name) {
  const clean = String(value ?? '').trim();
  if (!clean) throw new RangeError(`${name} must be a non-empty string`);
  return clean;
}

function requirePositiveInt(value, name) {
  if (typeof value === 'boolean' || value === null || value === undefined || value === '') {
    throw new RangeError(`${name} must be a positive integer`);
  }
  const n = Number(value);
  if (!Number.isInteger(n) || n < 1) throw new RangeError(`${name} must be a positive integer`);
  return n;
}

function parseCropRect(crop) {
  const parts = Array.isArray(crop)
    ? crop
    : (typeof crop === 'string'
      ? crop.split(',').map((s) => s.trim())
      : (crop && typeof crop === 'object' ? [crop.x, crop.y, crop.w, crop.h] : null));
  if (!parts || parts.length !== 4) {
    throw new RangeError('crop must contain 4 finite numbers (x,y,w,h)');
  }
  const nums = parts.map((raw) => {
    if (typeof raw === 'boolean' || raw === null || raw === undefined || raw === '') {
      throw new RangeError('crop coordinates must be finite numbers');
    }
    const n = Number(raw);
    if (!Number.isFinite(n)) throw new RangeError('crop coordinates must be finite numbers');
    return n;
  });
  const [x, y, w, h] = nums;
  if (x < 0 || y < 0 || x >= 1 || y >= 1) {
    throw new RangeError('crop origin (x, y) must satisfy 0 <= x, y < 1');
  }
  if (w <= 0 || h <= 0) throw new RangeError('crop width and height must be positive');
  if (x + w > 1.001 || y + h > 1.001) {
    throw new RangeError('crop rectangle must lie within normalized [0, 1] page bounds');
  }
  return nums;
}

export function buildPageImageUrl(sessionId, sourceId, { page, crop, dpi } = {}) {
  const sid = requireNonEmptyId(sessionId, 'sessionId');
  const srcId = requireNonEmptyId(sourceId, 'sourceId');
  const pageNum = requirePositiveInt(page, 'page');
  const query = [`page=${pageNum}`];

  if (dpi !== undefined && dpi !== null && dpi !== '') {
    const dpiNum = typeof dpi === 'boolean' ? NaN : Number(dpi);
    if (!Number.isFinite(dpiNum) || dpiNum <= 0) {
      throw new RangeError('dpi must be a positive finite number');
    }
    query.push(`dpi=${dpiNum}`);
  }
  if (crop !== undefined && crop !== null && crop !== '') {
    query.push(`crop=${parseCropRect(crop).join(',')}`);
  }
  return `/api/sessions/${encodeURIComponent(sid)}/sources/${encodeURIComponent(srcId)}/page-image?${query.join('&')}`;
}

export function createPageImageViewer(options = {}) {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch;
  const urlApi = options.urlApi ?? globalThis.URL;
  const getToken = options.getToken ?? null;
  const onClose = typeof options.onClose === 'function' ? options.onClose : null;

  const aside = el('aside', 'page-image-viewer', null, { role: 'dialog', 'aria-label': 'Scanned page image' });
  setHidden(aside, true);

  const header = el('header', 'page-image-viewer-header');
  const headingGroup = el('div', 'page-image-viewer-heading');
  const ocrBadge = createBadge({ text: OCR_TAG_WORD, icon: 'ocr', tone: 'warn', border: BORDER_SOLID });
  const titleEl = el('h3', 'page-image-viewer-title');
  headingGroup.append(ocrBadge, titleEl);

  const controls = el('div', 'page-image-viewer-controls');
  const makeBtn = (cls, action, label) => {
    const b = el('button', cls, label, { 'data-viewer-action': action });
    b.type = 'button';
    return b;
  };
  const prevBtn = makeBtn('page-image-viewer-prev', 'prev', 'Previous');
  const nextBtn = makeBtn('page-image-viewer-next', 'next', 'Next');
  const closeBtn = makeBtn('page-image-viewer-close', 'close', 'Close');
  controls.append(prevBtn, nextBtn, closeBtn);
  header.append(headingGroup, controls);

  const noticeEl = el('p', 'page-image-viewer-notice', OCR_MATH_NOTICE);
  const quoteEl = el('blockquote', 'page-image-viewer-quote');
  setHidden(quoteEl, true);

  const stage = el('div', 'page-image-viewer-stage');
  const statusEl = el('p', 'page-image-viewer-status', '', { role: 'status', 'aria-live': 'polite' });
  const imgEl = el('img', 'page-image-viewer-img');
  setHidden(imgEl, true);
  stage.append(statusEl, imgEl);
  aside.append(header, noticeEl, quoteEl, stage);

  const state = {
    open: false, loading: false, error: null,
    sessionId: null, sourceId: null, page: null, pageCount: null,
    crop: null, dpi: null, quote: '', ocr: true, imageUrl: null, objectUrl: null,
  };
  let requestSeq = 0;

  function revokeActiveObjectUrl() {
    if (state.objectUrl) {
      if (urlApi && typeof urlApi.revokeObjectURL === 'function') urlApi.revokeObjectURL(state.objectUrl);
      state.objectUrl = null;
    }
  }

  function clearImage() {
    revokeActiveObjectUrl();
    imgEl.src = '';
    imgEl.removeAttribute('src');
    setHidden(imgEl, true);
  }

  function updateNavButtons() {
    const p = state.page;
    prevBtn.disabled = !state.open || !Number.isInteger(p) || p <= 1;
    nextBtn.disabled = !state.open || !Number.isInteger(p)
      || (Number.isInteger(state.pageCount) && p >= state.pageCount);
  }

  function setError(message) {
    state.loading = false;
    state.error = message;
    statusEl.textContent = message;
    statusEl.setAttribute('data-error', 'true');
  }

  const getState = () => ({ ...state });

  async function open(target = {}) {
    const seq = ++requestSeq;
    clearImage();

    const isOcr = target.ocr !== false;
    const clampedQuote = target.quote ? clampQuoteText(target.quote) : '';
    const pageCount = Number.isInteger(target.pageCount) && target.pageCount > 0 ? target.pageCount : null;

    Object.assign(state, {
      open: true, loading: true, error: null,
      sessionId: target.sessionId ?? null, sourceId: target.sourceId ?? null,
      pageCount, crop: target.crop ?? null, dpi: target.dpi ?? null,
      quote: clampedQuote, ocr: isOcr,
    });

    setHidden(aside, false);
    setHidden(ocrBadge, !isOcr);
    quoteEl.textContent = clampedQuote;
    setHidden(quoteEl, !clampedQuote);

    let url;
    try {
      state.page = requirePositiveInt(target.page, 'page');
      url = buildPageImageUrl(target.sessionId, target.sourceId, {
        page: state.page, crop: target.crop, dpi: target.dpi,
      });
    } catch (err) {
      titleEl.textContent = isOcr ? QUOTE_OCR_MATCHED_PREFIX : 'Page image';
      setError(err?.message || 'Invalid page image request.');
      updateNavButtons();
      return getState();
    }

    state.imageUrl = url;
    titleEl.textContent = isOcr ? `${QUOTE_OCR_MATCHED_PREFIX}, p. ${state.page}` : `Page ${state.page}`;
    const altText = `Original scanned PDF page ${state.page}`;
    imgEl.alt = altText;
    imgEl.setAttribute('alt', altText);
    statusEl.removeAttribute('data-error');
    statusEl.textContent = `Loading page ${state.page}…`;
    updateNavButtons();

    try {
      if (typeof fetchImpl !== 'function') throw new Error('fetch is unavailable');
      const headers = {};
      const rawToken = typeof getToken === 'function' ? getToken() : (typeof getToken === 'string' ? getToken : '');
      const token = String(rawToken ?? '').trim();
      if (token) headers['X-ReSearch-Token'] = token;

      const resp = await fetchImpl(url, { method: 'GET', credentials: 'same-origin', headers });
      if (seq !== requestSeq || !state.open) return getState();
      if (!resp || !resp.ok) {
        setError(`Could not load scanned page image (HTTP ${resp ? resp.status : 'error'}).`);
        return getState();
      }
      const blob = await resp.blob();
      if (seq !== requestSeq || !state.open) return getState();
      if (!urlApi || typeof urlApi.createObjectURL !== 'function') {
        throw new Error('URL.createObjectURL is unavailable');
      }
      revokeActiveObjectUrl();
      const objUrl = urlApi.createObjectURL(blob);
      state.objectUrl = objUrl;
      state.loading = false;
      state.error = null;
      imgEl.src = objUrl;
      imgEl.setAttribute('src', objUrl);
      setHidden(imgEl, false);
      statusEl.removeAttribute('data-error');
      statusEl.textContent = state.pageCount
        ? `Showing page ${state.page} of ${state.pageCount}`
        : `Showing page ${state.page}`;
      return getState();
    } catch (err) {
      if (seq !== requestSeq || !state.open) return getState();
      setError(err?.message ? `Could not load scanned page image: ${err.message}` : 'Could not load scanned page image.');
      return getState();
    }
  }

  function close() {
    requestSeq += 1;
    clearImage();
    state.open = false;
    state.loading = false;
    setHidden(aside, true);
    updateNavButtons();
    if (onClose) onClose();
    return getState();
  }

  const stepPage = (delta) => {
    if (!state.open || !Number.isInteger(state.page)) return;
    const next = state.page + delta;
    if (next < 1 || (state.pageCount != null && next > state.pageCount)) return;
    void open({
      sessionId: state.sessionId, sourceId: state.sourceId, page: next,
      dpi: state.dpi, ocr: state.ocr, pageCount: state.pageCount,
    });
  };
  prevBtn.addEventListener('click', () => stepPage(-1));
  nextBtn.addEventListener('click', () => stepPage(1));
  closeBtn.addEventListener('click', () => close());

  return { element: aside, open, close, getState };
}

export function bindQuoteChipPageImageViewer(viewer, sessionId, extraOptions = {}) {
  return (target = {}) => viewer.open({
    ...extraOptions,
    sessionId: typeof sessionId === 'function' ? sessionId(target) : (target.sessionId || target.session_id || sessionId),
    sourceId: target.sourceId || target.source_id || extraOptions.sourceId || extraOptions.source_id,
    page: target.page ?? extraOptions.page ?? 1,
    crop: target.crop ?? extraOptions.crop,
    dpi: target.dpi ?? extraOptions.dpi,
    quote: target.quote ?? extraOptions.quote ?? '',
    pageCount: target.pageCount ?? target.page_count ?? extraOptions.pageCount,
    ocr: target.ocr !== undefined ? Boolean(target.ocr) : (extraOptions.ocr !== false),
  });
}
