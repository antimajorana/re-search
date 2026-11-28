// User paper sources and read-only sync panel (F13a `/api/sources`, package X17).
// Governance: `00-overview.md` 2.11, `explore.md` 4.3, `agents/exp-x17-library.md`.
// Security invariants: zero `innerHTML` writes anywhere (all untrusted paths/titles use
// `textContent`); every source id is wrapped with `encodeURIComponent`; Rule S8 zero "verified".

export const SOURCES_API_BASE = '/api/sources';
export const SOURCE_KINDS = Object.freeze(['kandinsky_vault', 'folder', 'bibtex', 'zotero']);
export const SOURCE_KIND_LABELS = Object.freeze({
  kandinsky_vault: 'Kandinsky vault',
  folder: 'Folder',
  bibtex: 'BibTeX file',
  zotero: 'Zotero database',
});
export const SOURCES_READ_ONLY_NOTICE =
  'Read-only sync: re:Search indexes your sources in place without editing source files.';

export function sourceUrl(id, suffix = '') {
  return `${SOURCES_API_BASE}/${encodeURIComponent(String(id ?? '').trim())}${suffix}`;
}

export function domEl(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function clearChildren(node) {
  if (typeof node.replaceChildren === 'function') {
    node.replaceChildren();
    return;
  }
  while (node.firstChild && typeof node.removeChild === 'function') {
    node.removeChild(node.firstChild);
  }
  if (Array.isArray(node.children)) node.children.length = 0;
}

export function makeBtn(className, label, onClick, type = 'button') {
  const b = domEl('button', className, label);
  b.type = type;
  if (onClick) b.addEventListener('click', onClick);
  return b;
}

export function makeInput(type, className, placeholder = '') {
  const inp = domEl('input', className);
  inp.type = type;
  if (placeholder) inp.placeholder = placeholder;
  return inp;
}

export function makeSelect(className, entries, selected = '') {
  const sel = domEl('select', className);
  for (const [val, label] of entries) {
    const opt = domEl('option', '', label);
    opt.value = val;
    sel.appendChild(opt);
  }
  sel.value = selected;
  return sel;
}

export async function requestJsonWithToken(method, path, body, options = {}) {
  if (typeof options.requestJson === 'function') {
    return options.requestJson(method, path, body);
  }
  const fetchImpl = options.fetchImpl || globalThis.fetch;
  if (typeof fetchImpl !== 'function') throw new Error('HTTP client unavailable.');
  const headers = {};
  const token = typeof options.getToken === 'function' ? String(options.getToken() || '').trim() : '';
  if (token) headers['X-ReSearch-Token'] = token;
  const init = { method, credentials: 'same-origin', headers };
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = typeof body === 'string' ? body : JSON.stringify(body);
  }
  const resp = await fetchImpl(path, init);
  const text = await resp.text();
  let data = null;
  if (text) {
    try { data = JSON.parse(text); } catch { data = { raw: text }; }
  }
  if (!resp.ok) {
    const err = new Error((data && (data.error || data.message)) || `HTTP ${resp.status}`);
    err.status = resp.status;
    throw err;
  }
  return data;
}

const el = domEl;
const defaultRequestJson = requestJsonWithToken;

function formatScanWarning(warning) {
  if (typeof warning === 'string') return warning;
  if (warning && typeof warning === 'object') {
    const p = String(warning.path || '').trim();
    const r = String(warning.reason || '').trim();
    if (p && r) return `${p}: ${r}`;
    return p || r || 'Unreadable source entry';
  }
  return String(warning ?? '');
}

function formatSyncReport(report) {
  if (!report || typeof report !== 'object') return 'Scan finished.';
  const parts = [];
  if (report.total_items != null) parts.push(`${report.total_items} items`);
  if (report.with_pdf != null) parts.push(`${report.with_pdf} with PDF`);
  if (report.with_tex != null) parts.push(`${report.with_tex} with TeX`);
  if (report.added != null) parts.push(`${report.added} added`);
  if (report.updated != null) parts.push(`${report.updated} updated`);
  if (report.moved != null && report.moved > 0) parts.push(`${report.moved} moved`);
  if (report.missing != null && report.missing > 0) parts.push(`${report.missing} missing`);
  const exactDups = Number(report.exact_duplicates || 0);
  const possibleDups = Number(report.possible_duplicates || 0);
  const groupCount = Array.isArray(report.duplicate_groups)
    ? report.duplicate_groups.length : Number(report.duplicate_groups || 0);
  if (exactDups > 0 || possibleDups > 0) {
    parts.push(`${exactDups} exact / ${possibleDups} possible duplicates`);
  } else if (groupCount > 0) {
    parts.push(`${groupCount} duplicate ${groupCount === 1 ? 'group' : 'groups'}`);
  }
  if (report.unreadable != null && report.unreadable > 0) {
    parts.push(`${report.unreadable} unreadable`);
  }
  return parts.length ? parts.join(' · ') : 'Sync finished.';
}

/**
 * Mount the F13a `/api/sources` management panel (preview -> accept, sync, duplicates, remove).
 */
export function createLibrarySourcesPanel(container, options = {}) {
  if (!container || typeof container.appendChild !== 'function') {
    throw new Error('createLibrarySourcesPanel requires a valid container element.');
  }
  const req = (method, path, body) => defaultRequestJson(method, path, body, options);
  const state = { sources: [], pendingPreview: null, duplicates: null, busy: false };

  const root = el('section', 'library-sources-panel');
  root.setAttribute('data-library-sources', 'true');

  const header = el('div', 'project-library-toolbar');
  header.appendChild(el('h3', 'library-sources-heading', 'Your paper sources'));
  const topActions = el('div', 'project-library-actions');
  const syncAllBtn = el('button', 'pill-btn sm library-sources-sync-all', 'Sync all');
  syncAllBtn.type = 'button';
  const dupesBtn = el('button', 'pill-btn sm library-sources-duplicates-btn', 'Duplicates');
  dupesBtn.type = 'button';
  topActions.appendChild(syncAllBtn);
  topActions.appendChild(dupesBtn);
  header.appendChild(topActions);
  root.appendChild(header);

  root.appendChild(el('p', 'project-library-status library-sources-notice', SOURCES_READ_ONLY_NOTICE));

  // Add source form (preview first -> explicit accept, overview 2.11 sync rule 5).
  const form = el('form', 'project-library-form library-sources-form');
  const kindLabel = el('label', 'project-library-field', 'Source kind');
  const kindSelect = el('select', 'text-input library-source-kind');
  for (const kind of SOURCE_KINDS) {
    const opt = el('option', '', SOURCE_KIND_LABELS[kind] || kind);
    opt.value = kind;
    kindSelect.appendChild(opt);
  }
  kindLabel.appendChild(kindSelect);
  form.appendChild(kindLabel);

  const rootLabel = el('label', 'project-library-field', 'Root path');
  const rootInput = el('input', 'text-input library-source-root');
  rootInput.type = 'text';
  rootInput.placeholder = 'Path to Kandinsky vault, PDF folder, .bib file, or Zotero sqlite';
  rootLabel.appendChild(rootInput);
  form.appendChild(rootLabel);

  const idLabel = el('label', 'project-library-field', 'Optional source ID');
  const idInput = el('input', 'text-input library-source-id');
  idInput.type = 'text';
  idInput.placeholder = 'Auto-generated if blank';
  idLabel.appendChild(idInput);
  form.appendChild(idLabel);

  const hashLabel = el('label', 'library-source-hash-label');
  const hashInput = el('input', 'library-source-hash');
  hashInput.type = 'checkbox';
  hashLabel.appendChild(hashInput);
  hashLabel.appendChild(el('span', '', ' Hash files during scan'));
  form.appendChild(hashLabel);

  const formActions = el('div', 'project-library-actions');
  const previewBtn = el('button', 'pill-btn sm primary library-source-preview-btn', 'Preview scan');
  previewBtn.type = 'submit';
  formActions.appendChild(previewBtn);
  form.appendChild(formActions);
  root.appendChild(form);

  const previewBox = el('div', 'paper-card library-source-preview');
  previewBox.hidden = true;
  root.appendChild(previewBox);

  const statusEl = el('p', 'project-library-status library-sources-status');
  statusEl.setAttribute('role', 'status');
  root.appendChild(statusEl);

  const listEl = el('div', 'project-library-list library-sources-list');
  root.appendChild(listEl);

  const dupesBox = el('div', 'library-sources-duplicates');
  dupesBox.hidden = true;
  root.appendChild(dupesBox);

  container.appendChild(root);

  function setStatus(msg) {
    statusEl.textContent = String(msg ?? '');
  }

  function renderPreview() {
    clearChildren(previewBox);
    if (!state.pendingPreview) {
      previewBox.hidden = true;
      return;
    }
    previewBox.hidden = false;
    const { payload, response } = state.pendingPreview;
    previewBox.appendChild(
      el('h4', '', `Preview report (${SOURCE_KIND_LABELS[payload.kind] || payload.kind})`),
    );
    previewBox.appendChild(el('p', 'project-library-locator', payload.root));
    previewBox.appendChild(el('p', '', formatSyncReport(response?.report)));

    const warnings = Array.isArray(response?.report?.warnings) ? response.report.warnings : [];
    if (warnings.length) {
      const wList = el('ul', 'library-source-warnings');
      for (const w of warnings) wList.appendChild(el('li', '', formatScanWarning(w)));
      previewBox.appendChild(wList);
    }

    const row = el('div', 'project-library-actions');
    const acceptBtn = el('button', 'pill-btn sm primary library-source-accept-btn', 'Accept & index');
    acceptBtn.type = 'button';
    const cancelBtn = el('button', 'pill-btn sm library-source-cancel-preview', 'Cancel');
    cancelBtn.type = 'button';
    acceptBtn.addEventListener('click', () => acceptPreview());
    cancelBtn.addEventListener('click', () => {
      state.pendingPreview = null;
      renderPreview();
      setStatus('Preview dismissed; nothing was written to the library.');
    });
    row.appendChild(acceptBtn);
    row.appendChild(cancelBtn);
    previewBox.appendChild(row);
  }

  function renderSources() {
    clearChildren(listEl);
    if (!state.sources.length) {
      listEl.appendChild(el('p', 'project-library-status', 'No user paper sources registered.'));
      return;
    }
    for (const entry of state.sources) {
      const src = entry.source || entry;
      const itemCount = entry.item_count ?? src.item_count ?? 0;
      const card = el('article', 'paper-card library-source-card');
      card.setAttribute('data-source-id', String(src.id || ''));

      const titleText = `${SOURCE_KIND_LABELS[src.kind] || src.kind}: ${src.id}`;
      card.appendChild(el('h4', '', titleText));
      card.appendChild(el('p', 'project-library-locator', String(src.root || '')));
      card.appendChild(
        el(
          'p',
          'project-library-status',
          `State: ${src.state || 'idle'} · ${itemCount} ${itemCount === 1 ? 'item' : 'items'}`,
        ),
      );

      const actions = el('div', 'project-library-actions');
      const syncBtn = el('button', 'pill-btn sm library-source-sync-btn', 'Sync now');
      syncBtn.type = 'button';
      syncBtn.setAttribute('data-sync-source', String(src.id || ''));
      syncBtn.addEventListener('click', () => syncSource(src.id));

      const rmBtn = el('button', 'pill-btn sm library-source-remove-btn', 'Remove');
      rmBtn.type = 'button';
      rmBtn.setAttribute('data-remove-source', String(src.id || ''));
      rmBtn.addEventListener('click', () => removeSource(src.id));

      actions.appendChild(syncBtn);
      actions.appendChild(rmBtn);
      card.appendChild(actions);
      listEl.appendChild(card);
    }
  }

  function renderDuplicateMember(member) {
    if (typeof member === 'string') return member;
    const label = member.title || member.citekey || member.external_id || member.key || '';
    const extra = [member.citekey ? `citekey: ${member.citekey}` : '', member.source_id || ''].filter(Boolean).join(' · ');
    return extra ? `${label} [${extra}]` : label;
  }

  function renderDuplicates() {
    clearChildren(dupesBox);
    if (!state.duplicates) {
      dupesBox.hidden = true;
      return;
    }
    dupesBox.hidden = false;
    dupesBox.appendChild(el('h4', '', 'Duplicate analysis (read-only)'));
    const exact = Array.isArray(state.duplicates.exact_groups)
      ? state.duplicates.exact_groups : [];
    const fuzzy = Array.isArray(state.duplicates.possible_groups)
      ? state.duplicates.possible_groups
      : (Array.isArray(state.duplicates.possible_duplicates) ? state.duplicates.possible_duplicates : []);
    if (!exact.length && !fuzzy.length) {
      dupesBox.appendChild(el('p', 'project-library-status', 'No duplicates detected.'));
      return;
    }
    for (const grp of exact) {
      const card = el('div', 'paper-card library-duplicate-group');
      const signals = Array.isArray(grp.signals) && grp.signals.length
        ? grp.signals.join(', ')
        : (grp.signal || grp.reason || 'identifier match');
      const members = Array.isArray(grp.members) ? grp.members : (Array.isArray(grp.items) ? grp.items : []);
      const heading = grp.canonical_key
        ? `Exact match (${signals}) · ${grp.canonical_key}`
        : `Exact match (${signals})`;
      card.appendChild(el('strong', '', heading));
      for (const m of members) card.appendChild(el('p', 'project-library-locator', renderDuplicateMember(m)));
      dupesBox.appendChild(card);
    }
    for (const grp of fuzzy) {
      const card = el('div', 'paper-card library-possible-duplicate');
      const members = Array.isArray(grp.members) ? grp.members : (Array.isArray(grp.items) ? grp.items : []);
      card.appendChild(el('strong', '', 'Possible duplicate (not auto-merged)'));
      for (const m of members) card.appendChild(el('p', 'project-library-locator', renderDuplicateMember(m)));
      dupesBox.appendChild(card);
    }
  }

  async function loadSources() {
    state.busy = true;
    try {
      const data = await req('GET', SOURCES_API_BASE);
      state.sources = Array.isArray(data?.sources) ? data.sources : [];
      renderSources();
      return state.sources;
    } catch (err) {
      setStatus(`Failed to load sources: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function previewSource(overridePayload = null) {
    const payload = overridePayload || {
      kind: String(kindSelect.value || 'folder'),
      root: String(rootInput.value || '').trim(),
      hash_files: Boolean(hashInput.checked),
    };
    const customId = overridePayload ? overridePayload.id : String(idInput.value || '').trim();
    if (customId) payload.id = customId;
    if (!payload.root) {
      setStatus('Enter a source root path before previewing.');
      return null;
    }
    state.busy = true;
    setStatus('Scanning source (preview, read-only)...');
    try {
      const response = await req('POST', SOURCES_API_BASE, { ...payload, accept: false });
      state.pendingPreview = { payload, response };
      renderPreview();
      setStatus('Preview ready. Accept to index into the library.');
      return response;
    } catch (err) {
      setStatus(`Preview failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function acceptPreview() {
    if (!state.pendingPreview) return null;
    const { payload } = state.pendingPreview;
    state.busy = true;
    setStatus('Indexing accepted source...');
    try {
      const response = await req('POST', SOURCES_API_BASE, { ...payload, accept: true });
      state.pendingPreview = null;
      renderPreview();
      rootInput.value = '';
      idInput.value = '';
      setStatus(`Indexed source: ${formatSyncReport(response?.report)}`);
      await loadSources();
      if (typeof options.onSourcesChanged === 'function') {
        await options.onSourcesChanged(response);
      }
      return response;
    } catch (err) {
      setStatus(`Accept failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function syncSource(sourceId, syncOpts = {}) {
    state.busy = true;
    setStatus(`Syncing ${sourceId}...`);
    try {
      const response = await req('POST', sourceUrl(sourceId, '/sync'), syncOpts);
      setStatus(`Synced ${sourceId}: ${formatSyncReport(response?.report)}`);
      await loadSources();
      if (typeof options.onSourcesChanged === 'function') {
        await options.onSourcesChanged(response);
      }
      return response;
    } catch (err) {
      setStatus(`Sync failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function syncAll(syncOpts = {}) {
    state.busy = true;
    setStatus('Syncing all sources...');
    try {
      const response = await req('POST', `${SOURCES_API_BASE}/sync`, syncOpts);
      const count = response?.count ?? (Array.isArray(response?.reports) ? response.reports.length : 0);
      setStatus(`Synced ${count} ${count === 1 ? 'source' : 'sources'}.`);
      await loadSources();
      if (typeof options.onSourcesChanged === 'function') {
        await options.onSourcesChanged(response);
      }
      return response;
    } catch (err) {
      setStatus(`Sync all failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function loadDuplicates() {
    state.busy = true;
    setStatus('Checking duplicates...');
    try {
      const response = await req('GET', `${SOURCES_API_BASE}/duplicates`);
      state.duplicates = response?.duplicates || { exact_groups: [], possible_groups: [] };
      renderDuplicates();
      setStatus('Duplicate check complete.');
      return state.duplicates;
    } catch (err) {
      setStatus(`Duplicate check failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  async function removeSource(sourceId) {
    state.busy = true;
    setStatus(`Removing ${sourceId}...`);
    try {
      const response = await req('DELETE', sourceUrl(sourceId));
      setStatus(`Removed source ${sourceId} from index (files untouched).`);
      await loadSources();
      if (typeof options.onSourcesChanged === 'function') {
        await options.onSourcesChanged(response);
      }
      return response;
    } catch (err) {
      setStatus(`Remove failed: ${err?.message || String(err)}`);
      throw err;
    } finally {
      state.busy = false;
    }
  }

  form.addEventListener('submit', (ev) => {
    ev?.preventDefault?.();
    previewSource().catch(() => {});
  });
  syncAllBtn.addEventListener('click', () => { syncAll().catch(() => {}); });
  dupesBtn.addEventListener('click', () => { loadDuplicates().catch(() => {}); });

  return {
    el: root,
    loadSources,
    previewSource,
    acceptPreview,
    syncSource,
    syncAll,
    loadDuplicates,
    removeSource,
    getState: () => ({ ...state }),
  };
}

export const mountLibrarySourcesPanel = createLibrarySourcesPanel;
