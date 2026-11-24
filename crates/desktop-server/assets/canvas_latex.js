// Tab 1: LaTeX Manuscript Canvas — file selector, live KaTeX preview, patch save, tex_audit, and 1-click export.
import * as api from '/assets/api.js';
import { escapeHtml, renderLatexDocumentPreview } from '/assets/math.js';

let fileSelectEl = null;
let editorEl = null;
let previewEl = null;
let lineCountEl = null;
let saveBtn = null;
let auditBtn = null;
let auditResultsEl = null;
let fileNoteEl = null;
let currentSessionId = null;
let currentWorkspace = null;
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
      if (fileSelectEl.value) {
        loadLatexFile(currentSessionId, fileSelectEl.value);
      } else {
        fileRequest += 1;
        showFile('', true);
      }
    });
  }

  if (saveBtn) saveBtn.addEventListener('click', handleSavePatch);
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

function showFile(text, ready, truncated = false) {
  originalContent = text;
  clearTimeout(previewTimer);
  if (editorEl) {
    editorEl.value = text;
    editorEl.disabled = !ready;
    editorEl.readOnly = truncated;
    if (truncated) editorEl.setAttribute('aria-describedby', 'latex-file-note');
    else editorEl.removeAttribute('aria-describedby');
    editorBaseline = editorEl.value;
  }
  if (saveBtn) saveBtn.disabled = !ready || truncated;
  if (auditBtn) auditBtn.disabled = !ready || truncated;
  if (fileNoteEl) fileNoteEl.hidden = !truncated;
  if (auditResultsEl) auditResultsEl.hidden = true;
  updateLineCount();
  renderLatexDocumentPreview(previewEl, text);
}

export async function refreshWorkspaceFiles(sessionId) {
  const request = ++workspaceRequest;
  if (sessionId !== currentSessionId) {
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
    const chosen = preserveDraft || texFiles.includes(prev) ? prev : texFiles[0] || '';
    const options = [...texFiles];
    if (!options.includes(chosen)) options.unshift(chosen);
    fileSelectEl.innerHTML = '';
    for (const p of options) {
      const opt = document.createElement('option');
      opt.value = p;
      opt.textContent = p || 'draft.tex (scratch)';
      fileSelectEl.appendChild(opt);
    }
    fileSelectEl.value = chosen;
    if (preserveDraft) return;
    if (chosen) {
      await loadLatexFile(sessionId, chosen);
    } else {
      fileRequest += 1;
      showFile('', true);
    }
  } catch {
    // Session may not have an attached workspace yet
  }
}

async function loadLatexFile(sessionId, relPath) {
  const request = ++fileRequest;
  showFile('', false);
  try {
    const data = await api.getWorkspace(sessionId, relPath);
    if (request !== fileRequest) return;
    const content = (data && (data.content || data.text)) || '';
    showFile(content, true, data?.truncated === true);
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
  const payload = originalContent
    ? { path, search: originalContent, replace: nextText, content: nextText }
    : { path, content: nextText };

  try {
    if (saveBtn) saveBtn.disabled = true;
    await api.patchLatex(sessionId, payload);
    if (request === fileRequest) {
      originalContent = nextText;
      editorBaseline = nextText;
      notifyFn(`Saved patch to ${path}`);
    }
  } catch (err) {
    if (request === fileRequest) notifyFn(`Save failed: ${err.message}`);
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
