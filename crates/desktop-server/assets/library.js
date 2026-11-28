// Full global Library pane UI (package X17: F10 `/api/library`, F13a `/api/sources`,
// F13c `library_send_back.js`, and existing `saveProjectSource` in `api.js`).
// Governance: `00-overview.md` 2.7, 2.11, 3.2 row 20, 5.1; `explore.md` 4.3, 5.4, Rule S8.
// Security invariants:
// - ALWAYS wrap every `PaperKey` with `encodeURIComponent(key)` (`http_srv.rs` decodes `+` to ` `).
// - ZERO `innerHTML` writes anywhere; all paper, finding, and source fields use `textContent`.
// - Rule S8: ZERO `"verified"`, `"new"`, or `"novel"` in any UI label or constant.

import { getCsrfToken, listProjects, saveProjectSource } from './api.js';
import { mountPaperSendBackActions } from './library_send_back.js';
import {
  clearChildren, createLibrarySourcesPanel, domEl as el,
  makeBtn, makeInput, makeSelect, requestJsonWithToken,
} from './library_sources.js';
import { createBadge } from './ui_badge.js';

// Named constants for owner-accepted decisions (`00-overview.md` 2.7, 2.10, 3.2 row 20, 8.2 #5).
export const LIBRARY_API_BASE = '/api/library';
export const MARK_STATUSES = Object.freeze(['unread', 'skimmed', 'key', 'irrelevant']);
export const SEEN_ROLES = Object.freeze([
  'candidate', 'triaged', 'read', 'investigated', 'cited_lookup', 'solve_source',
]);
export const OWNED_BADGE_LABEL = 'in your library';
export const FULLTEXT_BADGE_LABEL = 'full text cached';
export const ATTACH_PDF_LABEL = 'Attach PDF';
export const ATTACH_SESSION_LABEL = 'Attach to session';
export const REBUILD_INDEX_LABEL = 'Rebuild index';
export const BACKUP_MARKS_LABEL = 'Download mark backup';
export const RESTORE_MARKS_LABEL = 'Restore mark backup';
export const SAVE_TO_PROJECT_LABEL = 'Save to project';
export const LIBRARY_INDEX_NOTICE =
  'Rebuildable local index over session blobs and your read-only sources; never evidence.';

export function encodePaperKey(key) {
  return encodeURIComponent(String(key ?? '').trim());
}

export function libraryPaperUrl(key, suffix = '') {
  return `${LIBRARY_API_BASE}/${encodePaperKey(key)}${suffix}`;
}

export function buildLibraryQueryString(filters = {}) {
  const qs = new URLSearchParams();
  for (const field of ['q', 'title', 'author', 'year', 'key', 'status', 'role', 'limit', 'offset']) {
    const raw = filters[field];
    if (raw !== undefined && raw !== null && String(raw).trim() !== '') {
      qs.set(field, String(raw).trim());
    }
  }
  const s = qs.toString();
  return s ? `?${s}` : '';
}

function stripDisallowedControls(raw) {
  return String(raw ?? '')
    .replace(/\x0c/g, ' ')
    .replace(/[\x00-\x08\x0b\x0e-\x1f\x7f-\u009f]/g, '')
    .trim();
}

/**
 * Build a `POST /api/projects/{id}/sources` (`SourceInput`) payload from a `PaperDetail`.
 * Strictly emits only fields accepted by `projects.rs` (`#[serde(deny_unknown_fields)]`).
 */
export function buildProjectSourcePayload(detail) {
  const paper = detail?.paper || detail || {};
  const ids = paper.ids && typeof paper.ids === 'object' ? paper.ids : {};
  const rawTitle = stripDisallowedControls(paper.title || paper.key || 'Untitled paper').slice(0, 256);
  const payload = { title: rawTitle || 'Untitled paper' };
  const cleanDoi = stripDisallowedControls(ids.doi || '');
  const cleanArxiv = stripDisallowedControls(ids.arxiv || '');
  if (/^10\.\d{4,}\/[^\s\\<>"']+$/.test(cleanDoi)) {
    payload.doi = cleanDoi.slice(0, 512);
    payload.url = `https://doi.org/${cleanDoi.split('/').map(encodeURIComponent).join('/')}`.slice(0, 2048);
  } else if (cleanArxiv && !/[\s\\<>"']/.test(cleanArxiv)) {
    const arxivPath = cleanArxiv.split('/').map(encodeURIComponent).join('/');
    payload.url = `https://arxiv.org/abs/${arxivPath}`.slice(0, 2048);
  }
  const cleanBib = stripDisallowedControls(paper.bibtex || '');
  if (cleanBib) payload.bibtex = cleanBib.slice(0, 8192);
  const noteText = stripDisallowedControls(detail?.user_mark?.note || '');
  if (noteText) payload.notes = noteText.slice(0, 4000);
  const firstFinding = Array.isArray(detail?.findings) ? detail.findings[0] : null;
  const cleanQuote = stripDisallowedControls(firstFinding?.quote || '');
  if (cleanQuote) payload.quote = cleanQuote.slice(0, 300);
  return payload;
}

function bytesToBase64(bytes) {
  if (typeof Buffer !== 'undefined') return Buffer.from(bytes).toString('base64');
  let bin = '';
  for (let i = 0; i < bytes.length; i += 1) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

async function readFileAsBase64(file) {
  if (file && typeof file.arrayBuffer === 'function') {
    return bytesToBase64(new Uint8Array(await file.arrayBuffer()));
  }
  if (file && file.bytes instanceof Uint8Array) return bytesToBase64(file.bytes);
  if (file && typeof file.bytes_base64 === 'string') return file.bytes_base64;
  throw new Error('Unable to read file bytes.');
}

async function readFileAsText(file) {
  if (typeof file === 'string') return file;
  if (file && typeof file.text === 'function') return file.text();
  if (file && file.bytes instanceof Uint8Array) return new TextDecoder().decode(file.bytes);
  throw new Error('Unable to read backup file text.');
}

function sendBackPaperFromDetail(detail) {
  const paper = detail?.paper || {};
  const firstItem = Array.isArray(detail?.source_items)
    ? detail.source_items.find((it) => it?.citekey) || detail.source_items[0] : null;
  return {
    key: String(paper.key || ''),
    version: firstItem?.version || undefined,
    ids: paper.ids || {},
    title: String(paper.title || ''),
    citekey: String(firstItem?.citekey || ''),
  };
}

/** Mount the full Library pane UI into `container` (package X17). */
export function mountLibraryPane(container, options = {}) {
  if (!container || typeof container.appendChild !== 'function') {
    throw new Error('mountLibraryPane requires a valid container element.');
  }
  const getToken = typeof options.getToken === 'function' ? options.getToken : getCsrfToken;
  const fetchImpl = options.fetchImpl || globalThis.fetch;
  const projectsListFn = typeof options.listProjects === 'function' ? options.listProjects : listProjects;
  const projectSaveFn = typeof options.saveProjectSource === 'function'
    ? options.saveProjectSource : saveProjectSource;
  const reqOpts = { requestJson: options.requestJson, fetchImpl, getToken };
  const requestJson = (method, path, body) => requestJsonWithToken(method, path, body, reqOpts);

  const state = {
    tab: 'papers',
    filters: { q: '', title: '', author: '', year: '', status: '', role: '' },
    papers: [],
    selectedKey: null,
    detail: null,
    projects: [],
  };

  const root = el('section', 'library-pane');
  root.setAttribute('data-library-pane', 'true');
  const setStatus = (msg) => { statusEl.textContent = String(msg ?? ''); };

  const restoreInput = makeInput('file', 'library-restore-input');
  restoreInput.accept = '.json,application/json';
  restoreInput.hidden = true;

  const topBar = el('div', 'project-library-toolbar library-pane-header');
  topBar.appendChild(el('h2', 'library-pane-title', 'Library'));
  const tabRow = el('div', 'project-library-actions library-pane-tabs');
  tabRow.appendChild(makeBtn('pill-btn sm primary library-tab-papers', 'Papers', () => setTab('papers')));
  tabRow.appendChild(makeBtn('pill-btn sm library-tab-sources', 'Sources', () => setTab('sources')));
  tabRow.appendChild(makeBtn('pill-btn sm library-rebuild-btn', REBUILD_INDEX_LABEL, () => {
    rebuildIndex().catch((e) => setStatus(`Rebuild failed: ${e?.message || e}`));
  }));
  tabRow.appendChild(makeBtn('pill-btn sm library-backup-btn', BACKUP_MARKS_LABEL, () => {
    downloadMarksBackup().catch((e) => setStatus(`Backup failed: ${e?.message || e}`));
  }));
  tabRow.appendChild(makeBtn('pill-btn sm library-restore-btn', RESTORE_MARKS_LABEL, () => {
    if (typeof restoreInput.click === 'function') restoreInput.click();
  }));
  tabRow.appendChild(restoreInput);
  topBar.appendChild(tabRow);
  root.appendChild(topBar);
  root.appendChild(el('p', 'project-library-status library-pane-notice', LIBRARY_INDEX_NOTICE));

  const statusEl = el('p', 'project-library-status library-pane-status');
  statusEl.setAttribute('role', 'status');
  root.appendChild(statusEl);

  const papersSection = el('div', 'library-papers-section');
  const filterForm = el('form', 'project-library-toolbar library-filter-form');
  const qInput = makeInput('search', 'text-input library-filter-q', 'Search title, abstract, full text, findings');
  const titleInput = makeInput('text', 'text-input library-filter-title', 'Title');
  const authorInput = makeInput('text', 'text-input library-filter-author', 'Author');
  const yearInput = makeInput('text', 'text-input library-filter-year', 'Year');
  const statusSelect = makeSelect('text-input library-filter-status', [
    ['', 'Any mark status'], ...MARK_STATUSES.map((s) => [s, s]),
  ]);
  const roleSelect = makeSelect('text-input library-filter-role', [
    ['', 'Any session role'], ...SEEN_ROLES.map((r) => [r, r.replace(/_/g, ' ')]),
  ]);
  const searchBtn = makeBtn('pill-btn sm primary library-search-btn', 'Search', null, 'submit');
  const clearBtn = makeBtn('pill-btn sm library-clear-btn', 'Clear', () => {
    for (const c of [qInput, titleInput, authorInput, yearInput, statusSelect, roleSelect]) c.value = '';
    loadPapers().catch(() => {});
  });
  for (const ctrl of [qInput, titleInput, authorInput, yearInput, statusSelect, roleSelect, searchBtn, clearBtn]) {
    filterForm.appendChild(ctrl);
  }
  papersSection.appendChild(filterForm);

  const countEl = el('p', 'project-library-status library-count');
  const listEl = el('div', 'project-library-list library-paper-list');
  const drawerEl = el('aside', 'paper-card library-detail-drawer');
  drawerEl.hidden = true;
  papersSection.appendChild(countEl);
  papersSection.appendChild(listEl);
  papersSection.appendChild(drawerEl);
  root.appendChild(papersSection);

  const sourcesHost = el('div', 'library-sources-host');
  sourcesHost.hidden = true;
  root.appendChild(sourcesHost);
  const sourcesPanel = createLibrarySourcesPanel(sourcesHost, {
    fetchImpl, getToken, requestJson, onSourcesChanged: () => loadPapers(),
  });
  container.appendChild(root);

  function setTab(nextTab) {
    state.tab = nextTab === 'sources' ? 'sources' : 'papers';
    papersSection.hidden = state.tab !== 'papers';
    sourcesHost.hidden = state.tab !== 'sources';
    if (state.tab === 'sources') sourcesPanel.loadSources().catch(() => {});
  }

  function readFiltersFromInputs() {
    state.filters = {
      q: String(qInput.value || '').trim(),
      title: String(titleInput.value || '').trim(),
      author: String(authorInput.value || '').trim(),
      year: String(yearInput.value || '').trim(),
      status: String(statusSelect.value || '').trim(),
      role: String(roleSelect.value || '').trim(),
    };
    return state.filters;
  }

  function renderPaperList() {
    clearChildren(listEl);
    countEl.textContent = state.papers.length === 0
      ? 'No matching papers in library.'
      : `${state.papers.length} ${state.papers.length === 1 ? 'paper' : 'papers'}`;
    for (const summary of state.papers) {
      const p = summary.paper || {};
      const key = String(p.key || '');
      const card = el('article', 'paper-card library-paper-row');
      card.setAttribute('data-paper-key', key);
      card.appendChild(makeBtn('library-paper-open-btn', p.title || key, () => { openPaperDetail(key).catch(() => {}); }));
      const authors = Array.isArray(p.authors) && p.authors.length ? p.authors.join(', ') : '';
      card.appendChild(el('p', 'project-library-locator', [authors, p.year || '', key].filter(Boolean).join(' · ')));

      const badges = el('div', 'project-library-actions library-paper-badges');
      if (summary.owned) badges.appendChild(createBadge({ text: OWNED_BADGE_LABEL, icon: 'pin', tone: 'ok', border: 'solid' }));
      if (summary.has_fulltext) badges.appendChild(createBadge({ text: FULLTEXT_BADGE_LABEL, icon: 'code', tone: 'info', border: 'solid' }));
      if (summary.user_mark?.status) {
        badges.appendChild(createBadge({ text: `mark: ${summary.user_mark.status}`, icon: 'user', tone: 'neutral', border: 'solid' }));
      }
      if (summary.finding_count > 0) {
        badges.appendChild(createBadge({
          text: `${summary.finding_count} matched ${summary.finding_count === 1 ? 'finding' : 'findings'}`,
          icon: 'quote', tone: 'ok', border: 'solid',
        }));
      }
      card.appendChild(badges);
      listEl.appendChild(card);
    }
  }

  function renderDetailDrawer() {
    clearChildren(drawerEl);
    const detail = state.detail;
    if (!detail || !detail.paper) { drawerEl.hidden = true; return; }
    drawerEl.hidden = false;
    const paper = detail.paper;
    const key = String(paper.key || '');
    drawerEl.setAttribute('data-detail-key', key);

    const head = el('div', 'project-library-toolbar library-detail-header');
    head.appendChild(el('h3', 'library-detail-title', paper.title || key));
    head.appendChild(makeBtn('pill-btn sm library-detail-close', 'Close', () => {
      state.selectedKey = null;
      state.detail = null;
      renderDetailDrawer();
    }));
    drawerEl.appendChild(head);

    const authors = Array.isArray(paper.authors) ? paper.authors.join(', ') : '';
    drawerEl.appendChild(el('p', 'project-library-locator', [authors, paper.year || '', key].filter(Boolean).join(' · ')));

    const srcItems = Array.isArray(detail.source_items) ? detail.source_items : [];
    const owned = Boolean(paper.flags?.owned) || srcItems.length > 0;
    const badgeRow = el('div', 'project-library-actions');
    if (owned) badgeRow.appendChild(createBadge({ text: OWNED_BADGE_LABEL, icon: 'pin', tone: 'ok', border: 'solid' }));
    if (Array.isArray(detail.fulltext) && detail.fulltext.length > 0) {
      badgeRow.appendChild(createBadge({ text: FULLTEXT_BADGE_LABEL, icon: 'code', tone: 'info', border: 'solid' }));
    }
    drawerEl.appendChild(badgeRow);

    if (srcItems.length) {
      const provSec = el('div', 'library-detail-provenance');
      provSec.appendChild(el('h4', '', 'Source provenance'));
      for (const it of srcItems) {
        provSec.appendChild(el('p', 'project-library-locator', [
          `source: ${it.source_id}`,
          it.citekey ? `citekey: ${it.citekey}` : `id: ${it.external_id}`,
          `state: ${it.state}`,
        ].join(' · ')));
      }
      drawerEl.appendChild(provSec);
    }

    const seenSec = el('div', 'library-detail-seen');
    seenSec.appendChild(el('h4', '', 'Sessions where seen'));
    const seenList = Array.isArray(detail.seen) ? detail.seen : [];
    if (!seenList.length) {
      seenSec.appendChild(el('p', 'project-library-status', 'Not linked to any session yet.'));
    } else {
      for (const s of seenList) {
        const row = el('div', 'project-library-actions');
        row.appendChild(el('span', 'project-library-locator', `${s.session_id} (${s.role})`));
        if (typeof options.onOpenSession === 'function') {
          row.appendChild(makeBtn('pill-btn sm', 'Open session', () => options.onOpenSession(s.session_id)));
        }
        seenSec.appendChild(row);
      }
    }
    drawerEl.appendChild(seenSec);

    const findSec = el('div', 'library-detail-findings');
    findSec.appendChild(el('h4', '', 'Matched findings'));
    const findings = Array.isArray(detail.findings) ? detail.findings : [];
    if (!findings.length) {
      findSec.appendChild(el('p', 'project-library-status', 'No matched findings recorded.'));
    } else {
      for (const f of findings) {
        const box = el('div', 'paper-card library-finding-item');
        const range = Array.isArray(f.cited_range) ? `lines ${f.cited_range[0]}-${f.cited_range[1]}` : '';
        box.appendChild(el('p', 'project-library-locator', [f.role, range, f.session_id].filter(Boolean).join(' · ')));
        if (f.quote) box.appendChild(el('blockquote', '', String(f.quote)));
        if (f.gist) box.appendChild(el('p', 'project-library-status', `${f.gist} (model)`));
        findSec.appendChild(box);
      }
    }
    drawerEl.appendChild(findSec);

    const markForm = el('form', 'project-library-form library-mark-form');
    markForm.appendChild(el('h4', '', 'Your mark & notes'));
    const markSelect = makeSelect('text-input library-mark-status', MARK_STATUSES.map((s) => [s, s]), detail.user_mark?.status || 'unread');
    const tagsInput = makeInput('text', 'text-input library-mark-tags');
    tagsInput.value = Array.isArray(detail.user_mark?.tags) ? detail.user_mark.tags.join(', ') : '';
    const noteInput = el('textarea', 'text-input library-mark-note');
    noteInput.value = String(detail.user_mark?.note || '');
    for (const [lbl, node] of [['Status', markSelect], ['Tags (comma-separated)', tagsInput], ['Notes', noteInput]]) {
      const f = el('label', 'project-library-field', lbl);
      f.appendChild(node);
      markForm.appendChild(f);
    }
    const markBtns = el('div', 'project-library-actions');
    markBtns.appendChild(makeBtn('pill-btn sm primary library-mark-save-btn', 'Save mark', null, 'submit'));
    markBtns.appendChild(makeBtn('pill-btn sm library-mark-clear-btn', 'Clear mark', () => { deleteMark(key).catch(() => {}); }));
    markForm.appendChild(markBtns);
    markForm.addEventListener('submit', (ev) => {
      ev?.preventDefault?.();
      const tags = String(tagsInput.value || '').split(',').map((t) => t.trim()).filter(Boolean);
      saveMark(key, { status: markSelect.value, tags, note: String(noteInput.value || '') }).catch(() => {});
    });
    drawerEl.appendChild(markForm);

    const actionsRow = el('div', 'project-library-actions library-detail-actions');
    const attachPdfInput = makeInput('file', 'library-attach-pdf-input');
    attachPdfInput.accept = '.pdf,application/pdf';
    attachPdfInput.hidden = true;
    attachPdfInput.addEventListener('change', () => {
      const file = attachPdfInput.files?.[0];
      attachPdfInput.value = '';
      if (file) attachPdfFile(key, file).catch(() => {});
    });
    actionsRow.appendChild(makeBtn('pill-btn sm library-attach-pdf-btn', ATTACH_PDF_LABEL, () => {
      if (typeof attachPdfInput.click === 'function') attachPdfInput.click();
    }));
    actionsRow.appendChild(attachPdfInput);

    const hasCachedBlob = Array.isArray(detail.fulltext)
      && detail.fulltext.some((f) => Boolean(f?.raw_blob || f?.file_sha256));
    if (hasCachedBlob) {
      actionsRow.appendChild(makeBtn('pill-btn sm library-attach-session-btn', ATTACH_SESSION_LABEL, () => {
        attachToSession(key).catch(() => {});
      }));
    }

    const projSelect = makeSelect('text-input library-project-select', [
      ['', 'Select project...'], ...state.projects.map((p) => [p.id, p.title || p.id]),
    ]);
    actionsRow.appendChild(projSelect);
    actionsRow.appendChild(makeBtn('pill-btn sm library-save-project-btn', SAVE_TO_PROJECT_LABEL, () => {
      saveDetailToProject(projSelect.value, detail).catch(() => {});
    }));
    actionsRow.appendChild(makeBtn('pill-btn sm library-delete-paper-btn', 'Remove from index', () => {
      deletePaper(key).catch(() => {});
    }));
    drawerEl.appendChild(actionsRow);

    const sendBackSlot = el('div', 'library-detail-send-back');
    mountPaperSendBackActions(sendBackSlot, sendBackPaperFromDetail(detail), {
      getToken, fetchImpl, postJson: (path, body) => requestJson('POST', path, body), onDeepLink: options.onKandinskyDeepLink,
    });
    drawerEl.appendChild(sendBackSlot);
  }

  async function withStatusError(actionLabel, fn) {
    try { return await fn(); } catch (err) {
      setStatus(`${actionLabel} failed: ${err?.message || String(err)}`);
      throw err;
    }
  }

  async function loadPapers(overrideFilters = null) {
    const filters = overrideFilters || readFiltersFromInputs();
    state.filters = { ...state.filters, ...filters };
    return withStatusError('Load library', async () => {
      const data = await requestJson('GET', `${LIBRARY_API_BASE}${buildLibraryQueryString(state.filters)}`);
      state.papers = Array.isArray(data?.papers) ? data.papers : [];
      renderPaperList();
      return state.papers;
    });
  }

  async function loadProjectsList() {
    try {
      const res = await projectsListFn();
      state.projects = Array.isArray(res?.projects) ? res.projects : [];
    } catch { state.projects = []; }
    return state.projects;
  }

  async function openPaperDetail(key) {
    return withStatusError(`Load paper ${key}`, async () => {
      const [detail] = await Promise.all([requestJson('GET', libraryPaperUrl(key)), loadProjectsList()]);
      state.selectedKey = key;
      state.detail = detail;
      renderDetailDrawer();
      return detail;
    });
  }

  async function saveMark(key, mark) {
    return withStatusError('Save mark', async () => {
      const res = await requestJson('POST', libraryPaperUrl(key, '/mark'), mark);
      setStatus(`Saved mark (${mark.status}) for ${key}.`);
      if (state.detail && state.selectedKey === key) { state.detail.user_mark = res?.mark || mark; renderDetailDrawer(); }
      await loadPapers();
      return res;
    });
  }

  async function deleteMark(key) {
    return withStatusError('Clear mark', async () => {
      const res = await requestJson('DELETE', libraryPaperUrl(key, '/mark'));
      setStatus(`Cleared mark for ${key}.`);
      if (state.detail && state.selectedKey === key) { state.detail.user_mark = null; renderDetailDrawer(); }
      await loadPapers();
      return res;
    });
  }

  async function attachToSession(key, sessionIdOverride = null) {
    return withStatusError('Attach to session', async () => {
      const sid = sessionIdOverride || (typeof options.getActiveSessionId === 'function' ? options.getActiveSessionId() : '');
      if (!sid) { setStatus('Open or select a session before attaching a library paper.'); return null; }
      const res = await requestJson('POST', libraryPaperUrl(key, '/attach'), { session_id: sid });
      setStatus(`Attached ${key} to session ${sid}.`);
      return res;
    });
  }

  async function attachPdfFile(key, file, sessionIdOverride = null) {
    return withStatusError('Attach PDF', async () => {
      const sid = sessionIdOverride || (typeof options.getActiveSessionId === 'function' ? options.getActiveSessionId() : '');
      if (typeof options.onAttachPdf === 'function') {
        const res = await options.onAttachPdf(key, file, sid);
        setStatus(`Attached PDF for ${key}.`);
        return res;
      }
      if (!sid) { setStatus('Open or select a session before attaching a PDF.'); return null; }
      const b64 = await readFileAsBase64(file);
      if (!b64) throw new Error('Attached PDF file is empty.');
      const fileName = String(file?.name || 'paper.pdf');
      const mode = typeof options.getActiveSessionMode === 'function' ? options.getActiveSessionMode() : '';
      const res = mode === 'explore'
        ? await requestJson('POST', `/api/sessions/${encodeURIComponent(sid)}/idea/read`, {
          attach_pdf: { paper_key: key, file_name: fileName, bytes_base64: b64 },
        })
        : await requestJson('POST', `/api/sessions/${encodeURIComponent(sid)}/upload`, {
          files: [{ name: fileName, bytes_base64: b64 }],
        });
      setStatus(`Uploaded PDF (${fileName}) for ${key} into session ${sid}.`);
      return res;
    });
  }

  async function saveDetailToProject(projectId, detail = state.detail) {
    return withStatusError('Save to project', async () => {
      if (!projectId) { setStatus('Choose a project before saving.'); return null; }
      const payload = buildProjectSourcePayload(detail);
      const res = await projectSaveFn(projectId, payload);
      setStatus(`Saved "${payload.title}" to project ${projectId}.`);
      return res;
    });
  }

  async function deletePaper(key) {
    return withStatusError('Remove paper', async () => {
      const res = await requestJson('DELETE', libraryPaperUrl(key));
      setStatus(`Removed ${key} from library index.`);
      if (state.selectedKey === key) { state.selectedKey = null; state.detail = null; renderDetailDrawer(); }
      await loadPapers();
      return res;
    });
  }

  async function rebuildIndex() {
    setStatus('Rebuilding library index...');
    return withStatusError('Rebuild index', async () => {
      const res = await requestJson('POST', `${LIBRARY_API_BASE}/rebuild`, {});
      const r = res?.report || {};
      setStatus(`Index rebuilt: ${r.papers ?? 0} papers, ${r.fulltext_entries ?? 0} cached full texts, ${r.user_marks_restored ?? 0} marks restored.`);
      await loadPapers();
      return res;
    });
  }

  async function downloadMarksBackup() {
    return withStatusError('Backup marks', async () => {
      const backup = await requestJson('GET', `${LIBRARY_API_BASE}/marks/backup`);
      const text = JSON.stringify(backup, null, 2);
      const filename = 're-search-library-marks-backup.json';
      if (typeof options.onDownloadBackup === 'function') {
        options.onDownloadBackup(text, filename, backup);
      } else if (typeof Blob !== 'undefined' && typeof URL?.createObjectURL === 'function') {
        const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
        const a = el('a', '');
        a.href = url;
        a.download = filename;
        if (typeof a.click === 'function') a.click();
        URL.revokeObjectURL?.(url);
      }
      const n = Array.isArray(backup?.marks) ? backup.marks.length : 0;
      setStatus(`Downloaded mark backup (${n} ${n === 1 ? 'mark' : 'marks'}).`);
      return backup;
    });
  }

  async function restoreMarksBackup(fileOrJson) {
    return withStatusError('Restore marks', async () => {
      const raw = await readFileAsText(fileOrJson);
      let parsed;
      try { parsed = JSON.parse(raw); } catch { throw new Error('Backup file is not valid JSON.'); }
      const res = await requestJson('POST', `${LIBRARY_API_BASE}/marks/restore`, parsed);
      setStatus(`Restored ${res?.restored ?? 0} user marks.`);
      await loadPapers();
      return res;
    });
  }

  restoreInput.addEventListener('change', () => {
    const file = restoreInput.files?.[0];
    restoreInput.value = '';
    if (file) restoreMarksBackup(file).catch(() => {});
  });
  filterForm.addEventListener('submit', (ev) => { ev?.preventDefault?.(); loadPapers().catch(() => {}); });
  if (options.autoLoad !== false) loadPapers().catch(() => {});

  return {
    el: root, sourcesPanel, setTab, loadPapers, openPaperDetail, saveMark, deleteMark,
    attachToSession, attachPdfFile, saveDetailToProject, deletePaper,
    rebuildIndex, downloadMarksBackup, restoreMarksBackup, getState: () => ({ ...state }),
  };
}

export const createLibraryPane = mountLibraryPane;
