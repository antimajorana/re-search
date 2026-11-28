// Paper investigation entry card, Stage A open progress, and Stage B metadata card (Investigate V4).
// Uses textContent and DOM builders exclusively; no untrusted HTML parsing.

import { createBadge } from './ui_badge.js';

// Design wording constants (00-overview.md 5.1, investigate.md 3, inv-a-v2-v5.md).
export const INVESTIGATE_CARD_TITLE = 'Investigate a paper';
export const INVESTIGATE_OPEN_VERB = 'Open';
export const INVESTIGATE_PLACEHOLDER = 'Paste arXiv id, DOI, link, or pick from library';
export const NEEDS_FILE_MESSAGE = 'This DOI has no arXiv version and is not in your library; drop the PDF or TeX.';
export const NO_MODEL_NOTE = 'Works without a model: builds the outline, results list, and structure map from source.';
export const CHOOSE_FILE_LABEL = 'Choose file';
export const DROP_OR_CHOOSE_FILE_LABEL = 'Drop or choose PDF / TeX';
export const RECENT_LIBRARY_HEADING = 'Recent library papers';
export const OPEN_FAILED_FALLBACK = 'Could not open paper';

export const OPEN_STEP_LABELS = Object.freeze({
  queued: 'Queued',
  metadata: 'Fetching metadata',
  library_lookup: 'Opening from library',
  eprint_tex: 'Fetching TeX source',
  pdf_fallback: 'Fetching PDF',
  ingest: 'Ingesting paper files',
  skeleton: 'Building structure map',
  done: 'Structure ready',
  needs_file: 'File needed',
  failed: 'Could not open paper',
});

// Pure paste-detection patterns (00-overview.md 5.1: only unambiguous paper ids/links match).
const CANONICAL_PAPER_KEY_RE = /^(arxiv|doi|inspire|s2|openalex|file):[^\s]+$/;
const ARXIV_URL_RE = /^https?:\/\/(?:export\.)?arxiv\.org\/(?:abs|pdf|e-print)\/[^\s]+$/i;
const ARXIV_ID_RE = /^(?:arxiv:)?(?:\d{4}\.\d{4,5}|[a-z-]+(?:\.[a-z-]+)?\/\d{7})(?:v[1-9]\d*)?$/i;
const DOI_RE = /^(?:https?:\/\/(?:dx\.)?doi\.org\/|doi:)?10\.\d{4,9}\/\S+$/i;

const STATUS_BADGE_SPECS = Object.freeze({
  running: Object.freeze({ icon: 'running', tone: 'info', fallbackText: 'Opening paper' }),
  done: Object.freeze({ icon: 'code', tone: 'ok', fallbackText: OPEN_STEP_LABELS.done }),
  needs_file: Object.freeze({ icon: 'warn', tone: 'warn', fallbackText: OPEN_STEP_LABELS.needs_file }),
  failed: Object.freeze({ icon: 'error', tone: 'err', fallbackText: OPEN_STEP_LABELS.failed }),
});

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function detectPaperInput(raw) {
  if (typeof raw !== 'string') return null;
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (CANONICAL_PAPER_KEY_RE.test(trimmed)) return { kind: 'library_key', value: trimmed };
  if (ARXIV_URL_RE.test(trimmed)) return { kind: 'url', value: trimmed };
  if (ARXIV_ID_RE.test(trimmed)) return { kind: 'arxiv_id', value: trimmed };
  if (DOI_RE.test(trimmed)) return { kind: 'doi', value: trimmed };
  return null;
}

function formatAuthors(authors) {
  if (typeof authors === 'string') return authors.trim();
  if (!Array.isArray(authors)) return '';
  return authors
    .map((a) => (typeof a === 'string' ? a : String(a?.name ?? a?.full_name ?? '')).trim())
    .filter(Boolean)
    .join(', ');
}

function normalizeVersions(metadata) {
  if (Array.isArray(metadata.versions) && metadata.versions.length > 0) {
    return metadata.versions
      .map((v) => (typeof v === 'object' && v ? String(v.version ?? v.label ?? v.id ?? '') : String(v ?? '')).trim())
      .filter(Boolean);
  }
  const single = metadata.version != null ? String(metadata.version).trim() : '';
  return single ? [single] : [];
}

export function renderMetadataCard(host, metadata) {
  if (!host) return null;
  if (!metadata || typeof metadata !== 'object') {
    host.replaceChildren();
    return null;
  }

  const card = el('article', 'inv-meta-card');
  card.appendChild(el('h3', 'inv-meta-title', String(metadata.title ?? '').trim() || 'Untitled paper'));

  const authorsText = formatAuthors(metadata.authors);
  if (authorsText) card.appendChild(el('p', 'inv-meta-authors', authorsText));

  const idRow = el('div', 'inv-meta-ids');
  const paperKey = String(metadata.paper_key ?? metadata.key ?? '').trim();
  const arxivId = String(metadata.arxiv_id ?? '').trim();
  const doi = String(metadata.doi ?? '').trim();
  const licence = String(metadata.licence ?? metadata.license ?? '').trim();

  if (paperKey) idRow.appendChild(el('span', 'inv-meta-key', paperKey));
  if (arxivId && arxivId !== paperKey) idRow.appendChild(el('span', 'inv-meta-arxiv', arxivId));
  if (doi && doi !== paperKey) idRow.appendChild(el('span', 'inv-meta-doi', doi));

  const versions = normalizeVersions(metadata);
  if (versions.length === 1) {
    idRow.appendChild(createBadge({ icon: 'info', tone: 'neutral', text: versions[0] }));
  } else if (versions.length > 1) {
    const list = el('ul', 'inv-meta-versions');
    const activeVersion = String(metadata.version ?? '').trim();
    for (const ver of versions) {
      const item = el('li', 'inv-meta-version', ver);
      if (activeVersion && ver === activeVersion) item.classList.add('is-current');
      list.appendChild(item);
    }
    idRow.appendChild(list);
  }

  if (licence) idRow.appendChild(createBadge({ icon: 'info', tone: 'muted', text: licence }));
  if ((idRow.children?.length || idRow.childNodes?.length || 0) > 0) card.appendChild(idRow);

  const abstractText = String(metadata.abstract ?? metadata.abstract_text ?? metadata.summary ?? '').trim();
  if (abstractText) card.appendChild(el('p', 'inv-meta-abstract', abstractText));

  host.replaceChildren(card);
  return card;
}

function labelForStep(step) {
  const key = String(step ?? '').trim();
  return key ? (OPEN_STEP_LABELS[key] || key) : '';
}

function normalizeHistoryEntry(entry) {
  if (typeof entry === 'string') return { step: entry.trim(), detail: '' };
  if (!entry || typeof entry !== 'object') return { step: '', detail: '' };
  return {
    step: String(entry.step ?? entry.stage ?? entry.name ?? '').trim(),
    detail: String(entry.detail ?? entry.message ?? entry.note ?? '').trim(),
  };
}

export function renderOpenProgress(host, snapshot, opts = {}) {
  if (!host) return null;
  if (!snapshot || typeof snapshot !== 'object') {
    host.replaceChildren();
    return null;
  }

  const status = String(snapshot.status ?? 'running').trim();
  const spec = STATUS_BADGE_SPECS[status] || { icon: 'info', tone: 'neutral', fallbackText: status || 'Queued' };
  const badgeText = status === 'running'
    ? (labelForStep(snapshot.step ?? snapshot.current_step) || spec.fallbackText)
    : spec.fallbackText;

  const panel = el('section', 'inv-progress');
  panel.setAttribute('data-status', status);
  panel.setAttribute('aria-live', 'polite');

  const header = el('div', 'inv-progress-header');
  header.appendChild(createBadge({ icon: spec.icon, tone: spec.tone, text: badgeText }));

  const budgetLimit = Number(snapshot.fetch_budget_limit ?? 0);
  if (Number.isFinite(budgetLimit) && budgetLimit > 0) {
    const rawUsed = Number(snapshot.fetch_budget_used ?? snapshot.external_fetches_used ?? snapshot.external_fetches ?? 0);
    const budgetUsed = Number.isFinite(rawUsed) && rawUsed >= 0 ? rawUsed : 0;
    header.appendChild(el('span', 'inv-progress-budget', `${budgetUsed} / ${budgetLimit} external fetches`));
  }
  panel.appendChild(header);

  const history = Array.isArray(snapshot.history) ? snapshot.history.map(normalizeHistoryEntry).filter((h) => h.step) : [];
  if (history.length > 0) {
    const list = el('ol', 'inv-progress-steps');
    for (const item of history) {
      const li = el('li', 'inv-progress-step');
      li.setAttribute('data-step', item.step);
      li.appendChild(el('span', 'inv-progress-step-label', labelForStep(item.step)));
      if (item.detail) li.appendChild(el('span', 'inv-progress-step-detail', item.detail));
      list.appendChild(li);
    }
    panel.appendChild(list);
  }

  if (status === 'needs_file') {
    const box = el('div', 'inv-progress-needs-file');
    box.appendChild(el('p', 'inv-progress-message', String(snapshot.needs_file_reason ?? '').trim() || NEEDS_FILE_MESSAGE));
    const fileBtn = el('button', 'inv-progress-file-btn', DROP_OR_CHOOSE_FILE_LABEL);
    fileBtn.type = 'button';
    fileBtn.addEventListener('click', () => opts.onRequestFile?.());
    box.appendChild(fileBtn);
    panel.appendChild(box);
  } else if (status === 'failed') {
    panel.appendChild(el('p', 'inv-progress-error', String(snapshot.error ?? '').trim() || OPEN_FAILED_FALLBACK));
  }

  host.replaceChildren(panel);
  return panel;
}

export function renderInvestigationEntry(host, opts = {}) {
  if (!host) return null;

  const card = el('section', 'inv-entry-card');
  card.appendChild(el('h2', 'inv-entry-title', INVESTIGATE_CARD_TITLE));
  if (!opts.modelConnected) card.appendChild(el('p', 'inv-entry-no-model', NO_MODEL_NOTE));

  const form = el('form', 'inv-entry-form');
  const input = el('input', 'inv-entry-input');
  input.type = 'text';
  input.placeholder = INVESTIGATE_PLACEHOLDER;
  input.setAttribute('aria-label', INVESTIGATE_PLACEHOLDER);

  const openBtn = el('button', 'inv-entry-open-btn', INVESTIGATE_OPEN_VERB);
  openBtn.type = 'submit';
  const fileBtn = el('button', 'inv-entry-file-btn', CHOOSE_FILE_LABEL);
  fileBtn.type = 'button';
  form.append(input, openBtn, fileBtn);

  const libraryEl = el('div', 'inv-entry-library');
  const progressEl = el('div', 'inv-entry-progress');
  const metaEl = el('div', 'inv-entry-meta');
  card.append(form, libraryEl, progressEl, metaEl);

  let isBusy = false;

  const syncBusy = () => {
    input.disabled = isBusy;
    openBtn.disabled = isBusy;
    fileBtn.disabled = isBusy;
    card.classList.toggle('is-busy', isBusy);
    card.setAttribute('aria-busy', isBusy ? 'true' : 'false');
    for (const btn of libraryEl.querySelectorAll?.('button') ?? []) btn.disabled = isBusy;
  };

  const renderLibrary = (papers) => {
    const valid = (Array.isArray(papers) ? papers : []).filter((p) => p && typeof p.key === 'string' && p.key.trim());
    if (valid.length === 0) {
      libraryEl.hidden = true;
      libraryEl.replaceChildren();
      return;
    }
    libraryEl.hidden = false;
    const list = el('ul', 'inv-entry-library-list');
    for (const paper of valid) {
      const key = paper.key.trim();
      const li = el('li', 'inv-entry-library-item');
      const btn = el('button', 'inv-entry-library-btn');
      btn.type = 'button';
      btn.disabled = isBusy;
      btn.setAttribute('data-paper-key', key);
      btn.append(
        el('span', 'inv-entry-library-title', String(paper.title ?? '').trim() || key),
        el('span', 'inv-entry-library-key', key),
      );
      btn.addEventListener('click', () => opts.onOpenLibraryKey?.(key));
      li.appendChild(btn);
      list.appendChild(li);
    }
    libraryEl.replaceChildren(el('h3', 'inv-entry-library-heading', RECENT_LIBRARY_HEADING), list);
  };

  const handleSubmit = (ev) => {
    ev?.preventDefault?.();
    if (isBusy) return;
    const trimmed = input.value.trim();
    if (!trimmed) return;
    const detected = detectPaperInput(trimmed);
    opts.onOpenInput?.(detected ? { [detected.kind]: detected.value, input: trimmed } : { input: trimmed });
  };

  const handlePaste = (ev) => {
    const text = ev?.clipboardData?.getData?.('text/plain') ?? ev?.clipboardData?.getData?.('text') ?? '';
    const detected = detectPaperInput(text);
    if (detected) opts.onDetectPaste?.(detected);
  };

  const handleFileClick = () => { if (!isBusy) opts.onRequestFile?.(); };

  form.addEventListener('submit', handleSubmit);
  input.addEventListener('paste', handlePaste);
  fileBtn.addEventListener('click', handleFileClick);

  renderLibrary(opts.recentPapers);
  host.replaceChildren(card);

  return {
    setSnapshot(snapshot) { renderOpenProgress(progressEl, snapshot, { onRequestFile: opts.onRequestFile }); },
    setMetadata(metadata) { renderMetadataCard(metaEl, metadata); },
    setRecentPapers(papers) { renderLibrary(papers); },
    setBusy(busy) { isBusy = Boolean(busy); syncBusy(); },
    focusInput() { input.focus?.(); },
    destroy() {
      form.removeEventListener('submit', handleSubmit);
      input.removeEventListener('paste', handlePaste);
      fileBtn.removeEventListener('click', handleFileClick);
      host.replaceChildren();
    },
  };
}
