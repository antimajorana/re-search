// Opt-in paper send-back UI controls (package F13c, overview 2.11 & decision 8.3 #5).
// Mounts "Export BibTeX" and "Send to Kandinsky" into the library paper-action slot
// (F13a assets/library_sources.js and X17 assets/library.js).
// Security invariant: zero `innerHTML` writes anywhere; all untrusted text uses `textContent`;
// actions run strictly on explicit user click (never on render or sync).

// Overview 2.11 & Decision 8.3 #5: opt-in labels and endpoints.
export const EXPORT_BIBTEX_LABEL = 'Export BibTeX';
export const SEND_TO_KANDINSKY_LABEL = 'Send to Kandinsky';
export const EXPORT_BIBTEX_ENDPOINT = '/api/sources/export/bibtex';
export const SEND_KANDINSKY_ENDPOINT = '/api/sources/send/kandinsky';
export const KANDINSKY_DEEP_LINK_PREFIX = 'kandinsky://capture-paper?target=';
export const SEND_BACK_NOTICE =
  'Opt-in only: re:Search never writes files into a Kandinsky vault or Zotero database.';

const FORBIDDEN_TARGET_RE = /[^\x21-\x7e]|[;|&$`<>()'"\\!*]/;
const MAX_TARGET_LEN = 512;

/**
 * Validate a Kandinsky capture target client-side before sending or building a deep link.
 * Mirrors `research_tools::source_export::validate_kandinsky_capture_target`.
 */
export function validateClientCaptureTarget(raw) {
  const value = String(raw ?? '').trim();
  if (!value || value.length > MAX_TARGET_LEN) {
    return { ok: false, error: 'Target must be 1..=512 characters.' };
  }
  if (value.startsWith('-')) {
    return { ok: false, error: 'Target must not start with `-`.' };
  }
  if (FORBIDDEN_TARGET_RE.test(value)) {
    return { ok: false, error: 'Target contains disallowed whitespace, control, or shell characters.' };
  }
  const low = value.toLowerCase();
  if (low.startsWith('arxiv:')) {
    const rest = value.slice(6);
    if (!rest || !/^[A-Za-z0-9./-]+$/.test(rest)) {
      return { ok: false, error: 'Invalid arXiv identifier.' };
    }
    return { ok: true, target: `arXiv:${rest}` };
  }
  if (low.startsWith('doi:')) {
    const rest = value.slice(4);
    if (!rest || !rest.startsWith('10.') || !rest.includes('/')) {
      return { ok: false, error: 'Invalid DOI identifier.' };
    }
    return { ok: true, target: `doi:${rest.toLowerCase()}` };
  }
  if (value.startsWith('https://')) {
    const after = value.slice('https://'.length);
    const authority = after.split(/[/?#]/, 1)[0] || '';
    if (!authority || authority.includes('@') || authority.includes('..')) {
      return { ok: false, error: 'Invalid HTTPS authority.' };
    }
    const host = authority.split(':', 1)[0];
    if (!host || !host.includes('.') || host.startsWith('.') || host.endsWith('.') || !/^[A-Za-z0-9.-]+$/.test(host)) {
      return { ok: false, error: 'Invalid HTTPS host.' };
    }
    return { ok: true, target: value };
  }
  return { ok: false, error: 'Target must start with arXiv:, doi:, or https://.' };
}

/**
 * Derive a candidate capture target string from a library paper object.
 */
export function derivePaperCaptureTarget(paper) {
  if (!paper || typeof paper !== 'object') return null;
  const ids = paper.ids && typeof paper.ids === 'object' ? paper.ids : {};
  const key = String(paper.key ?? '').trim();
  if (ids.arxiv) {
    const ver = paper.version ? String(paper.version).trim() : '';
    return `arXiv:${String(ids.arxiv).trim()}${ver}`;
  }
  if (key.toLowerCase().startsWith('arxiv:')) {
    const ver = paper.version ? String(paper.version).trim() : '';
    return `arXiv:${key.slice(6)}${ver}`;
  }
  if (ids.doi) {
    return `doi:${String(ids.doi).trim()}`;
  }
  if (key.toLowerCase().startsWith('doi:')) {
    return `doi:${key.slice(4)}`;
  }
  return null;
}

/**
 * Build a validated `kandinsky://capture-paper?...` deep link with RFC 3986 encoding.
 */
export function buildKandinskyDeepLink(rawTarget, title = '') {
  const check = validateClientCaptureTarget(rawTarget);
  if (!check.ok) {
    throw new Error(check.error);
  }
  let url = `${KANDINSKY_DEEP_LINK_PREFIX}${encodeURIComponent(check.target)}`;
  const cleanTitle = String(title ?? '').replace(/[\x00-\x1f\x7f]/g, '').trim().slice(0, 200);
  if (cleanTitle) {
    url += `&title=${encodeURIComponent(cleanTitle)}`;
  }
  return url;
}

async function defaultPostJson(path, body, options = {}) {
  if (typeof options.postJson === 'function') {
    return options.postJson(path, body);
  }
  const fetchImpl = options.fetchImpl || globalThis.fetch;
  if (typeof fetchImpl !== 'function') {
    throw new Error('HTTP client unavailable.');
  }
  const headers = { 'Content-Type': 'application/json' };
  const token = typeof options.getToken === 'function' ? String(options.getToken() || '').trim() : '';
  if (token) {
    headers['X-ReSearch-Token'] = token;
  }
  const resp = await fetchImpl(path, {
    method: 'POST',
    credentials: 'same-origin',
    headers,
    body: JSON.stringify(body),
  });
  const text = await resp.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { raw: text };
    }
  }
  if (!resp.ok) {
    throw new Error((data && (data.error || data.message)) || `HTTP ${resp.status}`);
  }
  return data;
}

/**
 * Create an opt-in "Export BibTeX" control for one or more selected library papers.
 * Opens an inline destination form on explicit click; never runs automatically.
 */
export function createExportBibtexControl(papers, options = {}) {
  const list = Array.isArray(papers) ? papers : [papers].filter(Boolean);
  const paperKeys = list
    .map((p) => (typeof p === 'string' ? p.trim() : String(p?.key ?? '').trim()))
    .filter(Boolean);

  const wrap = document.createElement('div');
  wrap.className = 'send-back-control send-back-export-bibtex';

  const triggerBtn = document.createElement('button');
  triggerBtn.type = 'button';
  triggerBtn.className = 'pill-btn sm send-back-export-btn';
  triggerBtn.textContent = EXPORT_BIBTEX_LABEL;
  triggerBtn.disabled = paperKeys.length === 0;
  wrap.appendChild(triggerBtn);

  const panel = document.createElement('div');
  panel.className = 'send-back-dialog';
  panel.hidden = true;

  const heading = document.createElement('div');
  heading.className = 'send-back-dialog-title';
  const summaryLabel = list.length === 1 && list[0]?.citekey
    ? `${EXPORT_BIBTEX_LABEL}: ${String(list[0].citekey)}`
    : `${EXPORT_BIBTEX_LABEL} (${paperKeys.length})`;
  heading.textContent = summaryLabel;
  panel.appendChild(heading);

  const notice = document.createElement('p');
  notice.className = 'send-back-notice';
  notice.textContent = SEND_BACK_NOTICE;
  panel.appendChild(notice);

  const pathInput = document.createElement('input');
  pathInput.type = 'text';
  pathInput.className = 'text-input send-back-path-input';
  pathInput.placeholder = 'Output .bib file path (outside read-only sources)';
  pathInput.value = String(options.defaultOutputPath ?? '');
  panel.appendChild(pathInput);

  const actionsRow = document.createElement('div');
  actionsRow.className = 'send-back-actions-row';

  const confirmBtn = document.createElement('button');
  confirmBtn.type = 'button';
  confirmBtn.className = 'pill-btn sm send-back-confirm-export';
  confirmBtn.textContent = 'Write .bib';
  actionsRow.appendChild(confirmBtn);

  const cancelBtn = document.createElement('button');
  cancelBtn.type = 'button';
  cancelBtn.className = 'pill-btn sm send-back-cancel-export';
  cancelBtn.textContent = 'Cancel';
  actionsRow.appendChild(cancelBtn);
  panel.appendChild(actionsRow);

  const statusEl = document.createElement('div');
  statusEl.className = 'send-back-status';
  panel.appendChild(statusEl);
  wrap.appendChild(panel);

  triggerBtn.addEventListener('click', () => {
    panel.hidden = !panel.hidden;
    statusEl.textContent = '';
  });

  cancelBtn.addEventListener('click', () => {
    panel.hidden = true;
    statusEl.textContent = '';
  });

  confirmBtn.addEventListener('click', async () => {
    const outPath = String(pathInput.value ?? '').trim();
    const payload = { paper_keys: paperKeys };
    if (outPath) {
      payload.output_path = outPath;
    }
    confirmBtn.disabled = true;
    statusEl.textContent = 'Exporting BibTeX...';
    try {
      const res = await defaultPostJson(EXPORT_BIBTEX_ENDPOINT, payload, options);
      const citeSummary = Array.isArray(res?.citekeys) ? res.citekeys.join(', ') : '';
      statusEl.textContent = res?.written
        ? `Exported ${res.count ?? paperKeys.length} entry(s) (${citeSummary}) to ${res.output_path}.`
        : `Formatted ${res?.count ?? paperKeys.length} entry(s) (${citeSummary}).`;
      if (typeof options.onExported === 'function') {
        options.onExported(res);
      }
    } catch (err) {
      statusEl.textContent = `Export failed: ${err?.message || String(err)}`;
    } finally {
      confirmBtn.disabled = false;
    }
  });

  return wrap;
}

/**
 * Create an opt-in "Send to Kandinsky" button for a single library paper.
 * Invokes `POST /api/sources/send/kandinsky` strictly on explicit user click.
 */
export function createSendToKandinskyControl(paper, options = {}) {
  const wrap = document.createElement('div');
  wrap.className = 'send-back-control send-back-kandinsky';

  const candidateTarget = derivePaperCaptureTarget(paper);
  const paperKey = String(paper?.key ?? '').trim();
  const paperTitle = String(paper?.title ?? '').trim();
  const paperCitekey = String(paper?.citekey ?? '').trim();

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'pill-btn sm send-back-kandinsky-btn';
  btn.textContent = SEND_TO_KANDINSKY_LABEL;
  btn.disabled = !candidateTarget && !paperKey;
  if (paperTitle || paperCitekey) {
    btn.setAttribute(
      'aria-label',
      `${SEND_TO_KANDINSKY_LABEL}: ${paperTitle || paperCitekey}`,
    );
  }
  wrap.appendChild(btn);

  const statusEl = document.createElement('span');
  statusEl.className = 'send-back-status';
  wrap.appendChild(statusEl);

  btn.addEventListener('click', async () => {
    const mode = options.mode === 'cli' ? 'cli' : 'deep_link';
    const payload = { mode, user_initiated: true };
    if (paperKey) {
      payload.paper_key = paperKey;
    } else if (candidateTarget) {
      const check = validateClientCaptureTarget(candidateTarget);
      if (!check.ok) {
        statusEl.textContent = `Cannot send: ${check.error}`;
        return;
      }
      payload.target = check.target;
    } else {
      statusEl.textContent = 'Paper has no arXiv id or DOI for Kandinsky.';
      return;
    }
    if (paperTitle) {
      payload.title = paperTitle;
    }

    btn.disabled = true;
    statusEl.textContent = 'Preparing Kandinsky handoff...';
    try {
      const res = await defaultPostJson(SEND_KANDINSKY_ENDPOINT, payload, options);
      statusEl.textContent = mode === 'cli'
        ? `Sent ${res?.target ?? ''} via Kandinsky CLI.`
        : `Ready: ${res?.deep_link ?? ''}`;
      if (mode === 'deep_link' && res?.deep_link && typeof options.onDeepLink === 'function') {
        options.onDeepLink(res.deep_link, res);
      }
      if (typeof options.onSent === 'function') {
        options.onSent(res);
      }
    } catch (err) {
      statusEl.textContent = `Send failed: ${err?.message || String(err)}`;
    } finally {
      btn.disabled = false;
    }
  });

  return wrap;
}

/**
 * Mount both F13c opt-in paper actions ("Export BibTeX" and "Send to Kandinsky") into
 * a paper-action slot inside `assets/library_sources.js` or `assets/library.js`.
 */
export function mountPaperSendBackActions(slotEl, paper, options = {}) {
  if (!slotEl || typeof slotEl.appendChild !== 'function') {
    throw new Error('mountPaperSendBackActions requires a valid container element.');
  }
  const group = document.createElement('div');
  group.className = 'library-send-back-slot';
  group.setAttribute('data-opt-in', 'true');

  if (paper?.title || paper?.citekey) {
    const meta = document.createElement('span');
    meta.className = 'send-back-paper-label';
    meta.textContent = paper.citekey ? `${paper.citekey} — ${paper.title || ''}` : String(paper.title);
    group.appendChild(meta);
  }

  const exportCtrl = createExportBibtexControl([paper], options);
  const kandinskyCtrl = createSendToKandinskyControl(paper, options);
  group.appendChild(exportCtrl);
  group.appendChild(kandinskyCtrl);
  slotEl.appendChild(group);

  return { group, exportCtrl, kandinskyCtrl };
}
