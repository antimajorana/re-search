// Investigate V15 `.rsmap` and standalone HTML share / export / import controls.
// Strictly DOM-based (`textContent` / `createElement` only; no raw HTML string injection).

import * as api from './api.js';
import { mountWriteupButton } from './writeup_btn.js';

export const READ_ONLY_BANNER_TEXT = 'quotes not matched on this machine';
let activeWriteupCtrl = null;
let activeBrowserCtx = null;

export function destroyInvestigationWriteup() {
  if (activeWriteupCtrl) {
    activeWriteupCtrl.destroy?.();
    activeWriteupCtrl = null;
  }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function formatDroppedQuotesBanner(droppedCount) {
  const n = Number(droppedCount || 0);
  return `Imported, ${n} quotes could not be matched`;
}

export function latestImportedShareState(sessionData) {
  const events = Array.isArray(sessionData?.events) ? sessionData.events : [];
  for (let i = events.length - 1; i >= 0; i -= 1) {
    if (events[i]?.kind === 'investigation.imported') return events[i].payload || null;
  }
  return null;
}

function triggerBlobDownload(blob, filename) {
  if (typeof URL?.createObjectURL !== 'function' || typeof document?.createElement !== 'function') return;
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body?.appendChild?.(a);
  a.click?.();
  a.remove?.();
  setTimeout?.(() => URL.revokeObjectURL?.(url), 1000);
}

function renderShareStatusBanner(host, info) {
  host.replaceChildren();
  if (!info) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  const text = info.source_match === false || info.read_only
    ? READ_ONLY_BANNER_TEXT
    : (Number(info.items_dropped || 0) > 0
      ? formatDroppedQuotesBanner(info.items_dropped)
      : (info.banner || `Imported (${Number(info.items_matched || 0)} items matched)`));
  const cls = info.source_match === false || Number(info.items_dropped || 0) > 0
    ? 'inv-share-banner inv-share-banner--warn'
    : 'inv-share-banner inv-share-banner--ok';
  const box = el('div', cls, text);
  box.setAttribute('role', 'status');
  host.appendChild(box);
}

function renderReadOnlyPreview(host, importState) {
  host.replaceChildren();
  if (!importState || importState.source_match !== false || !importState.preview) {
    host.hidden = true;
    return;
  }
  host.hidden = false;
  const p = importState.preview;
  const card = el('section', 'inv-share-readonly-preview');
  card.appendChild(el('h3', 'inv-share-ro-title', p.title || p.paper_key || 'Imported paper map'));
  const meta = [p.paper_key, p.version, p.builder ? `builder: ${p.builder}` : ''].filter(Boolean).join(' · ');
  if (meta) card.appendChild(el('p', 'inv-share-ro-meta', meta));
  if (p.question) card.appendChild(el('p', 'inv-share-ro-question', `Question: ${p.question}`));
  const banner = el('div', 'inv-share-banner inv-share-banner--warn', READ_ONLY_BANNER_TEXT);
  banner.setAttribute('role', 'status');
  card.appendChild(banner);
  const gists = Array.isArray(p.gists) ? p.gists : [];
  if (gists.length > 0) {
    const list = el('ul', 'inv-share-ro-list');
    for (const g of gists.slice(0, 12)) {
      const text = [g.tag, g.role ? `(${g.role})` : '', g.gist ? `${g.gist} (model)` : ''].filter(Boolean).join(' ');
      list.appendChild(el('li', 'inv-share-ro-item', text));
    }
    card.appendChild(list);
  }
  host.appendChild(card);
}

async function performImportText(apiMod, sessionId, text, bannerHost, hooks, onRefresh, onDone) {
  const trimmed = String(text || '').trim();
  if (!trimmed) return;
  let body;
  try {
    const parsed = JSON.parse(trimmed);
    body = parsed && parsed.format === 'rsmap' ? parsed : { rsmap: parsed };
  } catch {
    body = { raw_json: trimmed };
  }
  const res = await apiMod.importInvestigationMap(sessionId, body);
  if (bannerHost) renderShareStatusBanner(bannerHost, res);
  onDone?.(res);
  await (onRefresh?.() ?? hooks?.refresh?.(sessionId));
}

export function mountInvestigationShare({
  toolbar, centerCol, apiMod = api, sessionId, sessionData, st, hooks, onRefresh, fetchFn, urlApi, scheduleRevoke,
} = {}) {
  if (!toolbar && !centerCol) return null;
  destroyInvestigationWriteup();
  const oldGroup = toolbar?.querySelector?.('[data-testid="inv-share-controls"]');
  oldGroup?.remove?.();
  const oldWriteup = toolbar?.querySelector?.('[data-testid="inv-writeup-host"]');
  oldWriteup?.remove?.();
  const oldPanel = centerCol?.querySelector?.('[data-testid="inv-share-panel"]');
  oldPanel?.remove?.();

  const group = el('div', 'inv-share-controls');
  group.setAttribute('data-testid', 'inv-share-controls');

  const panel = el('div', 'inv-share-panel');
  panel.setAttribute('data-testid', 'inv-share-panel');

  const stmtLabel = el('label', 'inv-share-opt');
  const stmtCheck = el('input', 'inv-share-checkbox');
  stmtCheck.type = 'checkbox';
  stmtCheck.checked = false;
  stmtCheck.setAttribute('data-opt', 'include_statements');
  stmtLabel.append(stmtCheck, el('span', null, 'Include full statements (copyright opt-in)'));

  const notesLabel = el('label', 'inv-share-opt');
  const notesCheck = el('input', 'inv-share-checkbox');
  notesCheck.type = 'checkbox';
  notesCheck.checked = false;
  notesCheck.setAttribute('data-opt', 'include_notes');
  notesLabel.append(notesCheck, el('span', null, 'Include private notes'));

  const exportRsmapBtn = el('button', 'inv-node-action-btn', 'Export .rsmap');
  exportRsmapBtn.type = 'button';
  exportRsmapBtn.setAttribute('data-action', 'export-rsmap');

  const exportHtmlBtn = el('button', 'inv-node-action-btn', 'Export HTML');
  exportHtmlBtn.type = 'button';
  exportHtmlBtn.setAttribute('data-action', 'export-html');

  const importBtn = el('button', 'inv-node-action-btn', 'Import .rsmap');
  importBtn.type = 'button';
  importBtn.setAttribute('data-action', 'import-rsmap');

  const fileInput = el('input', 'inv-share-file-input');
  fileInput.type = 'file';
  fileInput.accept = '.rsmap,.json';
  fileInput.hidden = true;

  const bannerHost = el('div', 'inv-share-banner-host');
  bannerHost.setAttribute('data-testid', 'inv-share-banner');
  const importInfo = st?.lastShareResult || latestImportedShareState(sessionData);
  renderShareStatusBanner(bannerHost, importInfo);

  const runExport = async (format) => {
    try {
      const res = await apiMod.exportInvestigationFile(sessionId, {
        format,
        includeStatements: Boolean(stmtCheck.checked),
        includeNotes: Boolean(notesCheck.checked),
      });
      if (res?.blob) triggerBlobDownload(res.blob, res.filename || `paper.${format}`);
    } catch (err) {
      hooks?.showStatus?.(err?.message || `Could not export ${format}.`);
    }
  };

  exportRsmapBtn.addEventListener('click', () => { void runExport('rsmap'); });
  exportHtmlBtn.addEventListener('click', () => { void runExport('html'); });
  importBtn.addEventListener('click', () => fileInput.click());
  fileInput.addEventListener('change', async () => {
    const file = fileInput.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      await performImportText(apiMod, sessionId, text, bannerHost, hooks, onRefresh, (res) => {
        if (st) st.lastShareResult = res;
      });
    } catch (err) {
      hooks?.showStatus?.(err?.message || 'Could not import .rsmap file.');
    } finally {
      fileInput.value = '';
    }
  });

  group.append(exportRsmapBtn, exportHtmlBtn, importBtn, fileInput);
  toolbar?.appendChild?.(group);
  let writeupHost = null;
  let writeupCtrl = null;
  if (toolbar) {
    writeupHost = el('div', 'inv-writeup-host');
    writeupHost.setAttribute('data-testid', 'inv-writeup-host');
    toolbar.appendChild?.(writeupHost);
    writeupCtrl = mountWriteupButton(writeupHost, {
      sessionId: sessionId || '',
      getToken: () => (typeof apiMod?.getCsrfToken === 'function' ? apiMod.getCsrfToken() : ''),
      fetchFn,
      urlApi,
      scheduleRevoke,
      onStatus: (res) => hooks?.showStatus?.(`Saved ${res?.filename || 'write-up'}.`),
      onError: (err) => hooks?.showStatus?.(err?.message || 'Could not generate write-up.'),
    });
    activeWriteupCtrl = writeupCtrl || null;
  }
  panel.append(stmtLabel, notesLabel, bannerHost);
  centerCol?.appendChild?.(panel);

  return {
    group, panel, stmtCheck, notesCheck, exportRsmapBtn, exportHtmlBtn, importBtn, fileInput, bannerHost,
    writeupHost, writeupCtrl,
  };
}

export function syncInvestigationEntryShare(entryHost, { sessionId, sessionData, apiMod = api, hooks } = {}) {
  if (!entryHost) return null;
  const card = entryHost.querySelector?.('.inv-entry-card') || entryHost;
  let bar = card.querySelector?.('[data-testid="inv-entry-share"]');
  if (!bar) {
    bar = el('div', 'inv-entry-share');
    bar.setAttribute('data-testid', 'inv-entry-share');
    const importBtn = el('button', 'inv-entry-file-btn', 'Import .rsmap');
    importBtn.type = 'button';
    importBtn.setAttribute('data-action', 'entry-import-rsmap');
    const fileInput = el('input', 'inv-share-file-input');
    fileInput.type = 'file';
    fileInput.accept = '.rsmap,.json';
    fileInput.hidden = true;
    const roHost = el('div', 'inv-share-ro-host');
    roHost.setAttribute('data-testid', 'inv-share-ro-preview');
    importBtn.addEventListener('click', () => fileInput.click());
    fileInput.addEventListener('change', async () => {
      const file = fileInput.files?.[0];
      if (!file || !sessionId) return;
      try {
        const text = await file.text();
        await performImportText(apiMod, sessionId, text, null, hooks, null, (res) => {
          if (res?.source_match === false) {
            renderReadOnlyPreview(roHost, { source_match: false, preview: res.preview });
          }
        });
      } catch (err) {
        hooks?.showStatus?.(err?.message || 'Could not import .rsmap file.');
      } finally {
        fileInput.value = '';
      }
    });
    bar.append(importBtn, fileInput, roHost);
    card.appendChild?.(bar);
  }
  const roHost = bar.querySelector?.('[data-testid="inv-share-ro-preview"]');
  if (roHost) renderReadOnlyPreview(roHost, latestImportedShareState(sessionData));
  return bar;
}

function remountShellControls(shellHost) {
  if (!shellHost || shellHost.hidden || !activeBrowserCtx) return;
  const toolbar = shellHost.querySelector?.('.inv-node-actions');
  const centerCol = shellHost.querySelector?.('.inv-col--center');
  if (!toolbar && !centerCol) return;
  mountInvestigationShare({
    toolbar,
    centerCol,
    apiMod: api,
    sessionId: activeBrowserCtx.sessionId,
    sessionData: activeBrowserCtx.sessionData,
    hooks: activeBrowserCtx.hooks,
    onRefresh: () => activeBrowserCtx?.hooks?.refresh?.(activeBrowserCtx.sessionId),
  });
}

function ensureShellObserver(shellHost) {
  if (!shellHost || shellHost.__shareObserver || typeof MutationObserver !== 'function') return;
  let remounting = false;
  const observer = new MutationObserver(() => {
    if (remounting || !activeBrowserCtx || shellHost.hidden) return;
    const toolbar = shellHost.querySelector?.('.inv-node-actions');
    if (toolbar && !toolbar.querySelector?.('[data-testid="inv-writeup-host"]')) {
      remounting = true;
      try {
        remountShellControls(shellHost);
      } finally {
        remounting = false;
      }
    }
  });
  observer.observe(shellHost, { childList: true });
  shellHost.__shareObserver = observer;
}

function installBrowserHook() {
  if (typeof globalThis === 'undefined' || typeof queueMicrotask !== 'function') return;
  queueMicrotask(() => {
    const ctrl = globalThis.__reSearchInvestigationV3;
    if (!ctrl || typeof ctrl.syncSession !== 'function' || ctrl.__shareHookInstalled) return;
    const origSync = ctrl.syncSession.bind(ctrl);
    ctrl.__shareHookInstalled = true;
    ctrl.syncSession = async function syncWithShare(sessionId, sessionData, hooks = {}) {
      const ok = await origSync(sessionId, sessionData, hooks);
      if (typeof document === 'undefined' || sessionData?.mode !== 'investigation' || !sessionId) {
        activeBrowserCtx = null;
        destroyInvestigationWriteup();
        return ok;
      }
      const shellHost = document.getElementById('investigation-v3-shell');
      if (ok && shellHost && !shellHost.hidden) {
        activeBrowserCtx = { sessionId, sessionData, hooks };
        ensureShellObserver(shellHost);
        remountShellControls(shellHost);
      } else {
        activeBrowserCtx = null;
        destroyInvestigationWriteup();
      }
      const entryHost = document.getElementById('investigation-v4-entry');
      if (entryHost && !entryHost.hidden) {
        syncInvestigationEntryShare(entryHost, { sessionId, sessionData, apiMod: api, hooks });
      }
      return ok;
    };
  });
}

installBrowserHook();
