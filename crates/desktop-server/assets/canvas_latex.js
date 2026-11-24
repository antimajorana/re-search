// Tab 1: LaTeX Manuscript Canvas — file selector, live KaTeX preview, patch save, tex_audit, and 1-click export.
import * as api from '/assets/api.js';
import { escapeHtml, renderLatexDocumentPreview } from '/assets/math.js';

let fileSelectEl = null;
let editorEl = null;
let previewEl = null;
let lineCountEl = null;
let saveBtn = null;
let reloadBtn = null;
let auditBtn = null;
let auditResultsEl = null;
let fileNoteEl = null;
let currentSessionId = null;
let currentWorkspace = null;
let displayedFile = null;
let fileHash;
const drafts = new Map();
let workspaceRequest = 0;
let fileRequest = 0;
let originalContent = '';
let editorBaseline = '';
let previewTimer = null;
let notifyFn = () => {};

export function initLatexCanvas(opts = {}) {
  notifyFn = opts.onNotify || (() => {});
  fileSelectEl = document.getElementById('latex-file-select');
  editorEl = document.getElementById('latex-editor');
  previewEl = document.getElementById('latex-preview');
  lineCountEl = document.getElementById('latex-line-count');
  saveBtn = document.getElementById('latex-save-btn');
  reloadBtn = document.getElementById('latex-reload-btn');
  auditBtn = document.getElementById('tex-audit-btn');
  auditResultsEl = document.getElementById('tex-audit-results');
  fileNoteEl = document.getElementById('latex-file-note');

  if (editorEl) {
    editorEl.addEventListener('input', () => {
      updateLineCount();
      clearTimeout(previewTimer);
      previewTimer = setTimeout(() => {
        renderLatexDocumentPreview(previewEl, editorEl.value);
      }, 120);
    });
  }

  if (fileSelectEl) {
    fileSelectEl.addEventListener('change', () => {
      if (!currentSessionId) return;
      rememberDraft();
      if (fileSelectEl.value) {
        loadLatexFile(currentSessionId, fileSelectEl.value);
      } else {
        fileRequest += 1;
        openDraft('', '', false, null);
      }
    });
  }

  if (saveBtn) saveBtn.addEventListener('click', handleSavePatch);
  reloadBtn?.addEventListener('click', async () => {
    if (!window.confirm('Discard your edits and reload this file from disk?')) return;
    const request = fileRequest;
    const path = fileSelectEl.value || 'draft.tex';
    try {
      const data = await api.getWorkspace(currentSessionId, path);
      if (request !== fileRequest) return;
      drafts.delete(displayedFile);
      if (!fileSelectEl.value) {
        const option = document.createElement('option');
        option.value = option.textContent = path;
        fileSelectEl.appendChild(option);
        fileSelectEl.value = path;
      }
      openDraft(path, data.content, data.truncated === true, data.file_sha256);
    } catch (err) {
      if (request === fileRequest) notifyFn(`Could not reload the file. ${err.message}`);
    }
  });
  if (auditBtn) auditBtn.addEventListener('click', handleTexAudit);

  const exportButtons = [
    ['export-tex-btn', 'tex'],
    ['export-html-btn', 'html'],
    ['export-zip-btn', 'zip'],
  ];
  for (const [btnId, fmt] of exportButtons) {
    const btn = document.getElementById(btnId);
    if (btn) {
      btn.addEventListener('click', () => triggerExport(fmt));
    }
  }

  updateLineCount();
  renderLatexDocumentPreview(previewEl, '');
}

function updateLineCount() {
  if (!editorEl || !lineCountEl) return;
  const text = editorEl.value;
  const lines = text ? text.split('\n').length : 0;
  lineCountEl.textContent = `${lines} line${lines === 1 ? '' : 's'}`;
}

function rememberDraft() {
  if (!displayedFile || !editorEl || editorEl.disabled) return;
  if (editorEl.value === editorBaseline) drafts.delete(displayedFile);
  else drafts.set(displayedFile, { text: editorEl.value, originalContent, editorBaseline, fileHash });
}

function openDraft(path, text, truncated, hash) {
  const key = JSON.stringify([currentSessionId, currentWorkspace, path]);
  const draft = drafts.get(key);
  showFile(draft ? draft.text : text, true, truncated && !draft);
  displayedFile = key;
  fileHash = hash;
  if (draft) {
    ({ originalContent, editorBaseline, fileHash } = draft);
  }
}

function showFile(text, ready, truncated = false) {
  displayedFile = null;
  if (reloadBtn) reloadBtn.hidden = true;
  originalContent = text;
  clearTimeout(previewTimer);
  if (editorEl) {
    editorEl.value = text;
    editorEl.disabled = !ready;
    editorEl.readOnly = truncated;
    if (truncated || (ready && !currentWorkspace)) editorEl.setAttribute('aria-describedby', 'latex-file-note');
    else editorEl.removeAttribute('aria-describedby');
    editorBaseline = editorEl.value;
  }
  if (saveBtn) saveBtn.disabled = !ready || truncated || !currentWorkspace;
  if (auditBtn) auditBtn.disabled = !ready || truncated;
  if (fileNoteEl) {
    fileNoteEl.hidden = !ready || (!truncated && Boolean(currentWorkspace));
    fileNoteEl.textContent = truncated
      ? 'This preview is truncated. Open the full file in your workspace to edit or audit it.'
      : 'This scratch draft is kept while the app is open. Attach a folder under Research options → Workspace to save a manuscript.';
  }
  if (auditResultsEl) auditResultsEl.hidden = true;
  updateLineCount();
  renderLatexDocumentPreview(previewEl, text);
}

export async function refreshWorkspaceFiles(sessionId) {
  const request = ++workspaceRequest;
  if (sessionId !== currentSessionId) {
    rememberDraft();
    currentSessionId = sessionId;
    currentWorkspace = null;
    fileRequest += 1;
    if (fileSelectEl) fileSelectEl.replaceChildren();
    showFile('', false);
  }
  if (!sessionId || !fileSelectEl) return;
  try {
    const data = await api.getWorkspace(sessionId);
    if (request !== workspaceRequest) return;
    const workspace = data?.workspace_root ?? null;
    const sameWorkspace = workspace === currentWorkspace;
    if (!sameWorkspace) rememberDraft();
    const scratchKey = JSON.stringify([sessionId, null, '']);
    currentWorkspace = workspace;
    const entries = Array.isArray(data && data.files)
      ? data.files
      : Array.isArray(data && data.entries)
        ? data.entries
        : [];
    const texFiles = entries
      .map((item) => (typeof item === 'string' ? item : item.path || ''))
      .filter((p) => /\.(tex|bib|md|txt)$/i.test(p));

    const prev = sameWorkspace ? fileSelectEl.value : '';
    const preserveDraft = sameWorkspace && editorEl && !editorEl.disabled && editorEl.value !== editorBaseline;
    const newDraft = workspace && !sameWorkspace && !texFiles.includes('draft.tex') && drafts.get(scratchKey);
    if (newDraft) {
      drafts.set(JSON.stringify([sessionId, workspace, '']), newDraft);
      drafts.delete(scratchKey);
    }
    const chosen = newDraft ? '' : preserveDraft || texFiles.includes(prev) ? prev : texFiles[0] || '';
    const options = [...texFiles];
    if (!options.includes(chosen)) options.unshift(chosen);
    fileSelectEl.innerHTML = '';
    for (const p of options) {
      const opt = document.createElement('option');
      opt.value = p;
      opt.textContent = p || (workspace ? 'draft.tex (new)' : 'Scratch draft');
      fileSelectEl.appendChild(opt);
    }
    fileSelectEl.value = chosen;
    if (preserveDraft) return;
    if (chosen) {
      await loadLatexFile(sessionId, chosen);
    } else {
      fileRequest += 1;
      openDraft('', '', false, null);
    }
  } catch (err) {
    if (request === workspaceRequest) notifyFn(`Could not load manuscript files. ${err.message}`);
  }
}

async function loadLatexFile(sessionId, relPath) {
  const request = ++fileRequest;
  showFile('', false);
  try {
    const data = await api.getWorkspace(sessionId, relPath);
    if (request !== fileRequest) return;
    const content = (data && (data.content || data.text)) || '';
    openDraft(relPath, content, data?.truncated === true, data?.file_sha256);
  } catch (err) {
    if (request === fileRequest) notifyFn(`Could not load ${relPath}: ${err.message}`);
  }
}

async function handleSavePatch() {
  if (!currentSessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  if (saveBtn?.disabled) return;
  const sessionId = currentSessionId;
  const request = fileRequest;
  const path = fileSelectEl && fileSelectEl.value ? fileSelectEl.value : 'draft.tex';
  const nextText = editorEl ? editorEl.value : '';
  const replacement = originalContent.includes('\r\n') && !originalContent.replaceAll('\r\n', '').includes('\n')
    ? nextText.replaceAll('\n', '\r\n') : nextText;
  const payload = originalContent
    ? { path, search: originalContent, replace: replacement, content: replacement }
    : { path, content: replacement };
  if (fileHash !== undefined) payload.expected_sha256 = fileHash;
  const key = displayedFile;

  try {
    if (saveBtn) saveBtn.disabled = true;
    const result = await api.patchLatex(sessionId, payload);
    const savedHash = result?.output?.sha256_after;
    const pending = drafts.get(key);
    if (pending?.text === nextText) drafts.delete(key);
    else if (pending) Object.assign(pending, { originalContent: replacement, editorBaseline: nextText, fileHash: savedHash });
    if (request === fileRequest) {
      originalContent = replacement;
      editorBaseline = nextText;
      fileHash = savedHash;
      notifyFn(`Saved patch to ${path}`);
    }
  } catch (err) {
    if (request === fileRequest) {
      if (reloadBtn && /changed on disk/.test(err.message)) reloadBtn.hidden = false;
      notifyFn(`Save failed: ${err.message}`);
    }
  } finally {
    if (request === fileRequest && saveBtn) saveBtn.disabled = false;
  }
}

async function handleTexAudit() {
  if (!currentSessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  if (auditBtn?.disabled) return;
  const sessionId = currentSessionId;
  const request = fileRequest;
  const path = fileSelectEl && fileSelectEl.value ? fileSelectEl.value : undefined;
  const candidate_tex = editorEl && editorEl.value.trim() ? editorEl.value : undefined;
  try {
    if (auditBtn) auditBtn.disabled = true;
    const report = await api.runTexAudit(sessionId, { path, candidate_tex });
    if (request === fileRequest) renderAuditReport(report);
  } catch (err) {
    if (request === fileRequest) notifyFn(`TeX Audit failed: ${err.message}`);
  } finally {
    if (request === fileRequest && auditBtn) auditBtn.disabled = false;
  }
}

function renderAuditReport(report) {
  if (!auditResultsEl || !report) return;
  auditResultsEl.hidden = false;
  const issues = Array.isArray(report.issues) ? report.issues : [];
  const ok = Boolean(report.ok);
  const headerBadge = ok
    ? '<span class="badge ok">TeX Audit Clean</span>'
    : `<span class="badge err">${Number(report.blocker_count) || 0} blockers, ${Number(report.error_count) || 0} errors</span>`;
  const summary = `
    <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:6px;">
      ${headerBadge}
      <span style="color:var(--ink-muted);font-size:11.5px;">${Number(report.files_scanned) || 0} files · ${Number(report.labels_indexed) || 0} labels · ${Number(report.bib_keys_indexed) || 0} bib keys</span>
    </div>
  `;
  if (issues.length === 0) {
    auditResultsEl.innerHTML = `${summary}<div style="color:var(--ink-secondary)">No missing labels, missing citations, unverified quotes, or retracted references detected.</div>`;
    return;
  }
  const itemsHtml = issues
    .map((iss) => {
      const sev = String(iss.severity || 'warning').toLowerCase();
      const tone = sev === 'blocker' || sev === 'error' ? 'err' : 'warn';
      const loc = iss.line ? `${iss.file}:${iss.line}` : iss.file || '';
      return `
        <div style="padding:5px 0;border-top:1px solid var(--border-subtle);">
          <span class="badge ${tone}">${escapeHtml(sev)}</span>
          <strong>${escapeHtml(iss.kind || '')}</strong>
          <code style="font-size:11px;">${escapeHtml(iss.target || '')}</code>
          <span style="color:var(--ink-muted);font-size:11px;">${escapeHtml(loc)}</span>
          <div>${escapeHtml(iss.message || '')}</div>
        </div>
      `;
    })
    .join('');
  auditResultsEl.innerHTML = summary + itemsHtml;
}

function triggerExport(format) {
  if (!currentSessionId) {
    notifyFn('Select or create a session before exporting.');
    return;
  }
  const url = api.exportUrl(currentSessionId, format);
  window.location.assign(url);
}
