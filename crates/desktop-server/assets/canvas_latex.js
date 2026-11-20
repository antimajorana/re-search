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
let currentSessionId = null;
let originalContent = '';
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
      if (currentSessionId && fileSelectEl.value) {
        loadLatexFile(currentSessionId, fileSelectEl.value);
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

export async function refreshWorkspaceFiles(sessionId) {
  currentSessionId = sessionId;
  if (!sessionId || !fileSelectEl) return;
  try {
    const data = await api.getWorkspace(sessionId);
    const entries = Array.isArray(data && data.files)
      ? data.files
      : Array.isArray(data && data.entries)
        ? data.entries
        : [];
    const texFiles = entries
      .map((item) => (typeof item === 'string' ? item : item.path || ''))
      .filter((p) => /\.(tex|bib|md|txt)$/i.test(p));

    const prev = fileSelectEl.value;
    fileSelectEl.innerHTML = '';
    if (texFiles.length === 0) {
      const opt = document.createElement('option');
      opt.value = '';
      opt.textContent = 'No .tex/.bib files';
      fileSelectEl.appendChild(opt);
      return;
    }
    for (const p of texFiles) {
      const opt = document.createElement('option');
      opt.value = p;
      opt.textContent = p;
      fileSelectEl.appendChild(opt);
    }
    const chosen = texFiles.includes(prev) ? prev : texFiles[0];
    fileSelectEl.value = chosen;
    await loadLatexFile(sessionId, chosen);
  } catch {
    // Session may not have an attached workspace yet
  }
}

async function loadLatexFile(sessionId, relPath) {
  try {
    const data = await api.getWorkspace(sessionId, relPath);
    const content = (data && (data.content || data.text)) || '';
    originalContent = content;
    if (editorEl) {
      editorEl.value = content;
      updateLineCount();
      renderLatexDocumentPreview(previewEl, content);
    }
  } catch (err) {
    notifyFn(`Could not load ${relPath}: ${err.message}`);
  }
}

async function handleSavePatch() {
  if (!currentSessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  const path = fileSelectEl && fileSelectEl.value ? fileSelectEl.value : 'draft.tex';
  const nextText = editorEl ? editorEl.value : '';
  const payload = originalContent
    ? { path, search: originalContent, replace: nextText, content: nextText }
    : { path, content: nextText };

  try {
    if (saveBtn) saveBtn.disabled = true;
    await api.patchLatex(currentSessionId, payload);
    originalContent = nextText;
    notifyFn(`Saved patch to ${path}`);
  } catch (err) {
    notifyFn(`Save failed: ${err.message}`);
  } finally {
    if (saveBtn) saveBtn.disabled = false;
  }
}

async function handleTexAudit() {
  if (!currentSessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  const path = fileSelectEl && fileSelectEl.value ? fileSelectEl.value : undefined;
  const candidate_tex = editorEl && editorEl.value.trim() ? editorEl.value : undefined;
  try {
    if (auditBtn) auditBtn.disabled = true;
    const report = await api.runTexAudit(currentSessionId, { path, candidate_tex });
    renderAuditReport(report);
  } catch (err) {
    notifyFn(`TeX Audit failed: ${err.message}`);
  } finally {
    if (auditBtn) auditBtn.disabled = false;
  }
}

function renderAuditReport(report) {
  if (!auditResultsEl || !report) return;
  auditResultsEl.hidden = false;
  const issues = Array.isArray(report.issues) ? report.issues : [];
  const ok = Boolean(report.ok);
  const headerBadge = ok
    ? '<span class="badge ok">TeX Audit Clean</span>'
    : `<span class="badge err">${report.blocker_count || 0} blockers, ${report.error_count || 0} errors</span>`;
  const summary = `
    <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:6px;">
      ${headerBadge}
      <span style="color:var(--ink-muted);font-size:11.5px;">${report.files_scanned || 0} files · ${report.labels_indexed || 0} labels · ${report.bib_keys_indexed || 0} bib keys</span>
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
