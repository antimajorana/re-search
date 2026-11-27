// Reusable F15 write-up download control (`GET /api/sessions/{id}/writeup`).
// Strictly DOM-based (`createElement` / `textContent` only; zero `innerHTML`).
// Always authenticates with `X-ReSearch-Token` header and downloads via Blob URLs
// (never bare `<a href="/api/...">` links).

export const WRITEUP_FORMATS = Object.freeze([
  Object.freeze({ id: 'tex', label: 'TeX (.tex)' }),
  Object.freeze({ id: 'md', label: 'Markdown (.md)' }),
  Object.freeze({ id: 'bib', label: 'BibTeX (.bib)' }),
]);

export const WRITEUP_CHECK_TABLE_NOTE =
  'Status line and "How each statement was checked" table are always included.';

export function normalizeWriteupFormat(raw) {
  const norm = String(raw || '').trim().toLowerCase();
  if (norm === 'md' || norm === 'markdown') return 'md';
  if (norm === 'bib' || norm === 'bibtex') return 'bib';
  return 'tex';
}

export function writeupUrl(sessionId, { format = 'tex', inlineMarks = true } = {}) {
  const sid = encodeURIComponent(String(sessionId || ''));
  const fmt = normalizeWriteupFormat(format);
  const marks = inlineMarks === false ? '0' : '1';
  return `/api/sessions/${sid}/writeup?format=${fmt}&inline_marks=${marks}`;
}

export function safeWriteupFilename(name, fallback = 'writeup.tex') {
  const leaf = String(name || '').replace(/\\/g, '/').split('/').pop() || '';
  const cleaned = leaf.replace(/[\x00-\x1f\x7f<>:"/\\|?*]+/g, '_').trim();
  if (cleaned && cleaned !== '.' && cleaned !== '..') return cleaned;
  return fallback;
}

export function parseContentDispositionFilename(header, fallback) {
  if (!header) return safeWriteupFilename(fallback, 'writeup.tex');
  const raw = String(header);
  const quoted = /filename="([^"]+)"/i.exec(raw);
  if (quoted && quoted[1]) return safeWriteupFilename(quoted[1], fallback);
  const bare = /filename=([^;\s]+)/i.exec(raw);
  if (bare && bare[1]) return safeWriteupFilename(bare[1], fallback);
  return safeWriteupFilename(fallback, 'writeup.tex');
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

function readHeader(res, name) {
  if (!res?.headers) return '';
  if (typeof res.headers.get === 'function') return res.headers.get(name) || '';
  const target = name.toLowerCase();
  for (const [k, v] of Object.entries(res.headers)) {
    if (k.toLowerCase() === target) return String(v || '');
  }
  return '';
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
    // Fall back to HTTP status message.
  }
  return `Write-up request failed (HTTP ${res?.status ?? 0})`;
}

export async function fetchWriteup(sessionId, opts = {}) {
  const sid = String(sessionId || '').trim();
  if (!sid) throw new Error('Select a session before creating a write-up.');
  const fetchFn = opts.fetchFn || (typeof fetch === 'function' ? fetch : null);
  if (typeof fetchFn !== 'function') throw new Error('Write-up fetch unavailable.');

  const format = normalizeWriteupFormat(opts.format);
  const inlineMarks = opts.inlineMarks !== false;
  const url = writeupUrl(sid, { format, inlineMarks });
  const headers = {};
  const token = resolveToken(opts);
  if (token) headers['X-ReSearch-Token'] = token;

  const res = await fetchFn(url, {
    method: 'GET',
    credentials: 'same-origin',
    headers,
  });
  if (!res || !res.ok) {
    const err = new Error(await readErrorMessage(res));
    err.status = res?.status ?? 0;
    throw err;
  }

  const contentType = readHeader(res, 'content-type') || 'text/plain; charset=utf-8';
  const fallbackName = safeWriteupFilename(`${sid}-writeup.${format}`, `writeup.${format}`);
  const filename = parseContentDispositionFilename(
    readHeader(res, 'content-disposition'),
    fallbackName,
  );

  let blob;
  if (typeof res.blob === 'function') {
    blob = await res.blob();
  } else if (typeof res.arrayBuffer === 'function' && typeof Blob !== 'undefined') {
    blob = new Blob([await res.arrayBuffer()], { type: contentType });
  } else if (typeof res.text === 'function' && typeof Blob !== 'undefined') {
    blob = new Blob([await res.text()], { type: contentType });
  } else {
    throw new Error('Write-up response missing body.');
  }

  return { blob, filename, contentType, format };
}

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function mountWriteupButton(container, opts = {}) {
  const doc = resolveDoc(container, opts);
  if (!container || !doc) return null;

  const urlApi = opts.urlApi || (typeof URL !== 'undefined' ? URL : null);
  const scheduleRevoke =
    opts.scheduleRevoke ||
    ((fn) => {
      if (typeof setTimeout === 'function') setTimeout(fn, 1000);
      else fn();
    });

  let currentSessionId = String(opts.sessionId || '').trim();
  let busy = false;
  let destroyed = false;
  const trackedUrls = new Set();

  const existing = container.querySelector?.('[data-testid="writeup-btn-bar"]');
  existing?.remove?.();

  const root = el(doc, 'div', 'writeup-btn-bar');
  root.setAttribute('data-testid', 'writeup-btn-bar');

  const formatSelect = el(doc, 'select', 'writeup-format-select');
  formatSelect.setAttribute('data-testid', 'writeup-format');
  formatSelect.setAttribute('aria-label', 'Write-up format');
  for (const item of WRITEUP_FORMATS) {
    const opt = el(doc, 'option', null, item.label);
    opt.value = item.id;
    formatSelect.appendChild(opt);
  }
  formatSelect.value = normalizeWriteupFormat(opts.format);

  const inlineLabel = el(doc, 'label', 'writeup-inline-label');
  inlineLabel.title = WRITEUP_CHECK_TABLE_NOTE;
  const inlineCheckbox = el(doc, 'input', 'writeup-inline-checkbox');
  inlineCheckbox.type = 'checkbox';
  inlineCheckbox.checked = opts.inlineMarks !== false;
  inlineCheckbox.setAttribute('data-testid', 'writeup-inline-marks');
  inlineLabel.appendChild(inlineCheckbox);
  inlineLabel.appendChild(el(doc, 'span', null, 'Inline check badges'));

  const button = el(doc, 'button', 'writeup-download-btn', 'Write up');
  button.type = 'button';
  button.setAttribute('data-action', 'download-writeup');
  button.disabled = Boolean(opts.disabled) || !currentSessionId;

  const statusEl = el(doc, 'span', 'writeup-status');
  statusEl.setAttribute('role', 'status');
  statusEl.setAttribute('data-testid', 'writeup-status');

  const noteEl = el(doc, 'span', 'writeup-note', WRITEUP_CHECK_TABLE_NOTE);
  noteEl.setAttribute('data-testid', 'writeup-table-note');

  function setStatus(text, isError = false) {
    statusEl.textContent = text ? String(text) : '';
    statusEl.className = isError ? 'writeup-status writeup-status--error' : 'writeup-status';
  }

  function revokeTracked() {
    if (!urlApi || typeof urlApi.revokeObjectURL !== 'function') {
      trackedUrls.clear();
      return;
    }
    for (const u of trackedUrls) {
      try {
        urlApi.revokeObjectURL(u);
      } catch {
        // Ignore cleanup failures in tests.
      }
    }
    trackedUrls.clear();
  }

  function triggerDownload(blob, filename) {
    if (!urlApi || typeof urlApi.createObjectURL !== 'function') return;
    const objUrl = urlApi.createObjectURL(blob);
    trackedUrls.add(objUrl);
    const link = doc.createElement('a');
    link.href = objUrl;
    link.download = filename;
    doc.body?.appendChild?.(link);
    link.click?.();
    link.remove?.();
    scheduleRevoke(() => {
      trackedUrls.delete(objUrl);
      try {
        urlApi.revokeObjectURL?.(objUrl);
      } catch {
        // Ignore cleanup failures.
      }
    });
  }

  async function onDownload() {
    if (destroyed || busy) return;
    if (!currentSessionId) {
      setStatus('Select a session first.', true);
      return;
    }
    busy = true;
    button.disabled = true;
    setStatus('Building write-up...');
    try {
      const result = await fetchWriteup(currentSessionId, {
        format: formatSelect.value,
        inlineMarks: Boolean(inlineCheckbox.checked),
        fetchFn: opts.fetchFn,
        getToken: opts.getToken,
        token: opts.token,
      });
      if (destroyed) return;
      triggerDownload(result.blob, result.filename);
      setStatus(`Saved ${result.filename}`);
      opts.onStatus?.(result);
    } catch (err) {
      if (destroyed) return;
      const msg = err?.message || 'Could not generate write-up.';
      setStatus(msg, true);
      opts.onError?.(err);
    } finally {
      busy = false;
      if (!destroyed) {
        button.disabled = Boolean(opts.disabled) || !currentSessionId;
      }
    }
  }

  button.addEventListener('click', () => {
    void onDownload();
  });

  root.appendChild(formatSelect);
  root.appendChild(inlineLabel);
  root.appendChild(button);
  root.appendChild(statusEl);
  root.appendChild(noteEl);
  container.appendChild(root);

  return {
    root,
    button,
    formatSelect,
    inlineCheckbox,
    statusEl,
    noteEl,
    download: onDownload,
    update(next = {}) {
      if (destroyed) return;
      if (next.sessionId !== undefined) {
        currentSessionId = String(next.sessionId || '').trim();
      }
      if (next.format !== undefined) {
        formatSelect.value = normalizeWriteupFormat(next.format);
      }
      if (next.inlineMarks !== undefined) {
        inlineCheckbox.checked = Boolean(next.inlineMarks);
      }
      if (next.disabled !== undefined) {
        opts.disabled = Boolean(next.disabled);
      }
      button.disabled = busy || Boolean(opts.disabled) || !currentSessionId;
    },
    destroy() {
      destroyed = true;
      revokeTracked();
      root.remove?.();
    },
  };
}
