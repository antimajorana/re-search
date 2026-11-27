// F16b "What I have learnt" user-approved lessons page & left-rail entry.
// Plain ES module; DOM construction uses createElement and textContent only (zero innerHTML).
import * as api from './api.js';

export const LESSONS_PAGE_TITLE = 'What I have learnt';
export const LESSONS_MAX_CHARS = 300;

export function unicodeCharCount(text) {
  return Array.from(String(text || '')).length;
}

function resolveAuthToken(apiMod) {
  const fromApi = typeof apiMod?.getCsrfToken === 'function' ? String(apiMod.getCsrfToken() || '').trim() : '';
  if (fromApi) return fromApi;
  if (typeof document !== 'undefined' && typeof document.querySelector === 'function') {
    const meta = document.querySelector('meta[name="re-search-csrf"]');
    if (meta && typeof meta.content === 'string' && meta.content.trim()) return meta.content.trim();
  }
  try {
    if (typeof window !== 'undefined' && window?.sessionStorage) {
      return String(window.sessionStorage.getItem('re-search-token') || '').trim();
    }
  } catch { /* sessionStorage unavailable */ }
  return '';
}

export async function lessonsFetchJson(method, path, body, deps = {}) {
  if (typeof deps.fetchJson === 'function') return deps.fetchJson(method, path, body);
  const fetchFn = deps.fetch || (typeof fetch === 'function' ? fetch : null);
  if (!fetchFn) throw new Error('fetch is unavailable');
  const headers = {};
  const token = resolveAuthToken(deps.api || api);
  if (token) headers['X-ReSearch-Token'] = token;
  const init = { method, credentials: 'same-origin', headers };
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const resp = await fetchFn(path, init);
  const text = await resp.text();
  let data = null;
  if (text) { try { data = JSON.parse(text); } catch { data = { raw: text }; } }
  if (!resp.ok) {
    const err = new Error((data && (data.error || data.message)) || `HTTP ${resp.status}`);
    err.status = resp.status;
    err.payload = data;
    throw err;
  }
  return data;
}

function el(doc, tag, cls, text = null, attrs = null) {
  const node = doc.createElement(tag);
  if (cls) node.className = cls;
  if (text !== null && text !== undefined) node.textContent = String(text);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v !== null && v !== undefined) node.setAttribute(k, String(v));
    }
  }
  return node;
}

export function formatScopeLabel(scope) {
  if (!scope || typeof scope !== 'object') return 'Global';
  const kind = String(scope.kind || 'global').toLowerCase();
  if (kind === 'mode_field') return `${scope.mode || 'mode'} + ${scope.field || 'field'}`;
  if (kind === 'mode') return `${scope.mode || 'mode'} only`;
  if (kind === 'field') return `Field: ${scope.field || 'general'}`;
  if (kind === 'paper') return `Paper: ${scope.paper_key || 'paper'}`;
  return 'Global';
}

export function formatOriginLabel(from) {
  if (!from || typeof from !== 'object') return 'origin unknown';
  const parts = [from.mode || 'session', from.session || '', from.id || ''].filter(Boolean);
  if (from.paper?.key) parts.push(`(${from.paper.key})`);
  return parts.join(' / ');
}

function formatCreatedDate(ms) {
  const n = Number(ms);
  if (!Number.isFinite(n) || n <= 0) return '—';
  try { return new Date(n).toISOString().slice(0, 10); } catch { return '—'; }
}

function defaultNavigateOrigin(from) {
  if (typeof window !== 'undefined' && typeof CustomEvent === 'function') {
    window.dispatchEvent(new CustomEvent('re-search:open-node', { detail: from }));
  }
  if (typeof document !== 'undefined' && from?.session && typeof document.querySelector === 'function') {
    const row = document.querySelector(`[data-session-id="${String(from.session).replace(/"/g, '\\"')}"]`);
    if (row && typeof row.click === 'function') row.click();
  }
}

export function createLessonsManager(hostEl, deps = {}) {
  const doc = deps.document || hostEl?.ownerDocument || (typeof document !== 'undefined' ? document : null);
  if (!doc || !hostEl) throw new Error('createLessonsManager requires hostEl and document');

  const fetchJson = (method, path, body) => lessonsFetchJson(method, path, body, deps);
  let lessons = Array.isArray(deps.initialLessons) ? [...deps.initialLessons] : [];
  let editingId = null;
  let confirmDeleteId = null;
  let showRestore = false;
  let exportedBackupText = '';
  let statusText = '';
  let statusTone = 'info';

  const root = el(doc, 'section', 'lessons-page', null, { 'data-testid': 'lessons-page' });
  hostEl.replaceChildren(root);

  function setStatus(msg, tone = 'info') {
    statusText = String(msg || '');
    statusTone = tone;
  }

  async function refresh() {
    try {
      const res = await fetchJson('GET', '/api/lessons/notes');
      lessons = Array.isArray(res?.lessons) ? res.lessons : [];
      setStatus('');
    } catch (err) {
      setStatus(err?.message || 'Failed to load lessons.', 'error');
    }
    render();
    return lessons;
  }

  async function handleExportBackup() {
    try {
      const res = await fetchJson('GET', '/api/lessons/notes/backup?confirm=true');
      const payload = res?.backup || res;
      exportedBackupText = JSON.stringify(payload, null, 2);
      setStatus(`Exported ${Array.isArray(payload?.lessons) ? payload.lessons.length : lessons.length} lessons (SHA-256 verified).`, 'ok');
    } catch (err) {
      setStatus(err?.message || 'Backup export failed.', 'error');
    }
    render();
  }

  async function handleRestoreBackup(rawText, replace) {
    let parsed;
    try {
      parsed = JSON.parse(String(rawText || ''));
    } catch {
      setStatus('Restore JSON is invalid.', 'error');
      render();
      return;
    }
    try {
      const res = await fetchJson('POST', '/api/lessons/notes/restore', { backup: parsed, replace: Boolean(replace) });
      showRestore = false;
      setStatus(`Restored ${res?.restored ?? 0} lessons.`, 'ok');
      await refresh();
    } catch (err) {
      setStatus(err?.message || 'Restore failed.', 'error');
      render();
    }
  }

  function renderLessonRow(lesson) {
    const item = el(doc, 'article', 'lesson-card', null, { 'data-testid': 'lesson-item', 'data-lesson-id': lesson.id });
    if (editingId === lesson.id) {
      const form = el(doc, 'form', 'lesson-edit-form', null, { 'data-testid': 'lesson-edit-form' });
      const textarea = el(doc, 'textarea', 'lesson-edit-input', null, {
        'data-testid': 'lesson-edit-input', rows: '3',
      });
      textarea.value = lesson.text || '';

      const scopeSelect = el(doc, 'select', 'lesson-scope-select', null, { 'data-testid': 'lesson-edit-scope' });
      const kindChoices = [
        ['global', 'Global'],
        ['mode', 'Mode only'],
        ['field', 'Field only'],
        ['mode_field', 'Mode + Field'],
        ['paper', 'Paper'],
      ];
      for (const [val, label] of kindChoices) {
        const opt = el(doc, 'option', null, label, { value: val });
        if ((lesson.scope?.kind || 'global') === val) opt.selected = true;
        scopeSelect.appendChild(opt);
      }

      const modeInput = el(doc, 'input', 'lesson-meta-input', null, {
        type: 'text', placeholder: 'mode (solve/explore/investigate)', 'data-testid': 'lesson-edit-mode',
      });
      modeInput.value = lesson.scope?.mode || lesson.from?.mode || 'solve';
      const fieldInput = el(doc, 'input', 'lesson-meta-input', null, {
        type: 'text', placeholder: 'field (e.g. qft)', 'data-testid': 'lesson-edit-field',
      });
      fieldInput.value = lesson.scope?.field || '';
      const paperInput = el(doc, 'input', 'lesson-meta-input', null, {
        type: 'text', placeholder: 'paper_key (e.g. arxiv:2401.00001)', 'data-testid': 'lesson-edit-paper',
      });
      paperInput.value = lesson.scope?.paper_key || lesson.from?.paper?.key || '';

      const counter = el(doc, 'span', 'lesson-char-count', `${unicodeCharCount(textarea.value)} / ${LESSONS_MAX_CHARS}`, {
        'data-testid': 'lesson-char-count',
      });
      const saveBtn = el(doc, 'button', 'pill-btn sm primary', 'Save changes', {
        type: 'submit', 'data-action': 'save-lesson-edit',
      });
      const cancelBtn = el(doc, 'button', 'pill-btn sm', 'Cancel', {
        type: 'button', 'data-action': 'cancel-lesson-edit',
      });
      cancelBtn.addEventListener('click', () => { editingId = null; render(); });

      textarea.addEventListener('input', () => {
        const len = unicodeCharCount(textarea.value);
        counter.textContent = `${len} / ${LESSONS_MAX_CHARS}`;
        const bad = len === 0 || len > LESSONS_MAX_CHARS;
        counter.className = bad ? 'lesson-char-count lesson-char-count--over' : 'lesson-char-count';
        saveBtn.disabled = bad;
      });

      form.addEventListener('submit', async (ev) => {
        ev.preventDefault();
        const text = String(textarea.value || '').trim();
        const len = unicodeCharCount(text);
        if (len === 0 || len > LESSONS_MAX_CHARS) {
          setStatus(`Lesson must be 1..=${LESSONS_MAX_CHARS} characters (got ${len}).`, 'error');
          render();
          return;
        }
        const kind = scopeSelect.value;
        const scope = { kind };
        if (kind === 'mode' || kind === 'mode_field') scope.mode = String(modeInput.value || 'solve').trim().toLowerCase();
        if (kind === 'field' || kind === 'mode_field') scope.field = String(fieldInput.value || '').trim().toLowerCase();
        if (kind === 'paper') scope.paper_key = String(paperInput.value || '').trim();
        try {
          await fetchJson('PUT', `/api/lessons/notes/${encodeURIComponent(lesson.id)}`, {
            text, scope, model_summary: false,
          });
          editingId = null;
          setStatus('Saved lesson update.', 'ok');
          await refresh();
        } catch (err) {
          setStatus(err?.message || 'Failed to update lesson.', 'error');
          render();
        }
      });

      const metaEdit = el(doc, 'div', 'lesson-edit-meta');
      metaEdit.append(scopeSelect, modeInput, fieldInput, paperInput);
      const actions = el(doc, 'div', 'lesson-edit-actions');
      actions.append(counter, saveBtn, cancelBtn);
      form.append(textarea, metaEdit, actions);
      item.appendChild(form);
      return item;
    }

    const head = el(doc, 'div', 'lesson-card-head');
    head.appendChild(el(doc, 'span', 'lesson-scope-badge', formatScopeLabel(lesson.scope), { 'data-testid': 'lesson-scope' }));
    if (lesson.model_summary) {
      head.appendChild(el(doc, 'span', 'lesson-model-badge', '(model)', { 'data-testid': 'lesson-model-badge' }));
    }
    head.appendChild(el(doc, 'span', 'lesson-uses-badge', `Uses: ${lesson.uses ?? 0}`, { 'data-testid': 'lesson-uses' }));
    head.appendChild(el(doc, 'span', 'lesson-date', formatCreatedDate(lesson.created_ms), { 'data-testid': 'lesson-date' }));

    const body = el(doc, 'p', 'lesson-text', lesson.text || '', { 'data-testid': 'lesson-text' });

    const foot = el(doc, 'div', 'lesson-card-foot');
    const originBtn = el(doc, 'button', 'lesson-origin-link', `From: ${formatOriginLabel(lesson.from)}`, {
      type: 'button', 'data-testid': 'lesson-origin-link',
    });
    originBtn.addEventListener('click', () => {
      if (typeof deps.onNavigateOrigin === 'function') deps.onNavigateOrigin(lesson.from);
      else defaultNavigateOrigin(lesson.from);
    });
    foot.appendChild(originBtn);

    const btns = el(doc, 'div', 'lesson-actions');
    const editBtn = el(doc, 'button', 'pill-btn sm', 'Edit', {
      type: 'button', 'data-action': 'edit-lesson',
    });
    editBtn.addEventListener('click', () => { editingId = lesson.id; confirmDeleteId = null; render(); });
    btns.appendChild(editBtn);

    if (confirmDeleteId === lesson.id) {
      const confirmBtn = el(doc, 'button', 'pill-btn sm danger', 'Confirm delete', {
        type: 'button', 'data-action': 'confirm-delete-lesson',
      });
      confirmBtn.addEventListener('click', async () => {
        try {
          await fetchJson('DELETE', `/api/lessons/notes/${encodeURIComponent(lesson.id)}`);
          confirmDeleteId = null;
          setStatus('Deleted lesson.', 'ok');
          await refresh();
        } catch (err) {
          setStatus(err?.message || 'Delete failed.', 'error');
          render();
        }
      });
      const cancelDelBtn = el(doc, 'button', 'pill-btn sm', 'Cancel', {
        type: 'button', 'data-action': 'cancel-delete-lesson',
      });
      cancelDelBtn.addEventListener('click', () => { confirmDeleteId = null; render(); });
      btns.append(confirmBtn, cancelDelBtn);
    } else {
      const delBtn = el(doc, 'button', 'pill-btn sm', 'Delete', {
        type: 'button', 'data-action': 'delete-lesson',
      });
      delBtn.addEventListener('click', () => { confirmDeleteId = lesson.id; editingId = null; render(); });
      btns.appendChild(delBtn);
    }

    foot.appendChild(btns);
    item.append(head, body, foot);
    return item;
  }

  function render() {
    root.replaceChildren();
    const header = el(doc, 'header', 'lessons-header');
    const titleBox = el(doc, 'div', 'lessons-title-box');
    titleBox.append(
      el(doc, 'h2', 'lessons-title', LESSONS_PAGE_TITLE),
      el(doc, 'p', 'lessons-subtitle', 'Notes you explicitly saved from corrections. Quoted on fresh runs; never proof or verification.'),
    );
    const topBtns = el(doc, 'div', 'lessons-top-actions');
    const refreshBtn = el(doc, 'button', 'pill-btn sm', 'Refresh', { type: 'button', 'data-action': 'refresh-lessons' });
    refreshBtn.addEventListener('click', () => { refresh(); });
    const exportBtn = el(doc, 'button', 'pill-btn sm', 'Export backup (opt-in)', { type: 'button', 'data-action': 'export-lessons' });
    exportBtn.addEventListener('click', () => { handleExportBackup(); });
    const restoreToggleBtn = el(doc, 'button', 'pill-btn sm', 'Restore backup', { type: 'button', 'data-action': 'toggle-restore-lessons' });
    restoreToggleBtn.addEventListener('click', () => { showRestore = !showRestore; render(); });
    topBtns.append(refreshBtn, exportBtn, restoreToggleBtn);
    if (typeof deps.onClose === 'function') {
      const closeBtn = el(doc, 'button', 'pill-btn sm', 'Close', { type: 'button', 'data-action': 'close-lessons' });
      closeBtn.addEventListener('click', () => deps.onClose());
      topBtns.appendChild(closeBtn);
    }
    header.append(titleBox, topBtns);
    root.appendChild(header);

    if (statusText) {
      root.appendChild(el(doc, 'div', `lessons-status lessons-status--${statusTone}`, statusText, {
        'data-testid': 'lessons-status', role: 'status',
      }));
    }

    if (exportedBackupText) {
      const box = el(doc, 'div', 'lessons-backup-box', null, { 'data-testid': 'lessons-backup-box' });
      const area = el(doc, 'textarea', 'lessons-backup-json', null, {
        readonly: 'readonly', rows: '4', 'data-testid': 'lessons-backup-json',
      });
      area.value = exportedBackupText;
      box.append(el(doc, 'div', 'lessons-backup-label', 'Portable checksummed JSON backup (opt-in):'), area);
      root.appendChild(box);
    }

    if (showRestore) {
      const form = el(doc, 'form', 'lessons-restore-form', null, { 'data-testid': 'lessons-restore-form' });
      const area = el(doc, 'textarea', 'lessons-restore-input', null, {
        rows: '4', placeholder: 'Paste checksummed lessons backup JSON', 'data-testid': 'lessons-restore-input',
      });
      const replaceChk = el(doc, 'input', null, null, { type: 'checkbox', 'data-testid': 'lessons-restore-replace' });
      const replaceLbl = el(doc, 'label', 'lessons-restore-opt');
      replaceLbl.append(replaceChk, el(doc, 'span', null, 'Replace existing lessons'));
      const submit = el(doc, 'button', 'pill-btn sm primary', 'Apply restore', { type: 'submit', 'data-action': 'submit-restore-lessons' });
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        handleRestoreBackup(area.value, replaceChk.checked);
      });
      form.append(area, replaceLbl, submit);
      root.appendChild(form);
    }

    const listEl = el(doc, 'div', 'lessons-list', null, { 'data-testid': 'lessons-list' });
    if (!lessons.length) {
      listEl.appendChild(el(doc, 'p', 'lessons-empty', 'No saved lessons yet.', { 'data-testid': 'lessons-empty' }));
    } else {
      for (const lesson of lessons) listEl.appendChild(renderLessonRow(lesson));
    }
    root.appendChild(listEl);
  }

  render();
  return {
    element: root,
    refresh,
    getLessons: () => [...lessons],
    setLessons: (next) => { lessons = Array.isArray(next) ? [...next] : []; render(); },
    destroy: () => { hostEl.replaceChildren(); },
  };
}

export function openLessonsDialog(deps = {}) {
  const doc = deps.document || (typeof document !== 'undefined' ? document : null);
  if (!doc || !doc.body) return null;
  let overlay = doc.getElementById('lessons-dialog-overlay');
  if (!overlay) {
    overlay = el(doc, 'div', 'lessons-modal-overlay', null, {
      id: 'lessons-dialog-overlay',
      role: 'dialog',
      'aria-label': LESSONS_PAGE_TITLE,
      style: 'position:fixed;inset:0;z-index:1200;background:rgba(15,23,42,0.55);display:flex;align-items:center;justify-content:center;padding:24px;',
    });
    const panel = el(doc, 'div', 'lessons-modal-panel', null, {
      style: 'background:var(--surface,#fff);color:var(--ink,#111);max-width:780px;width:100%;max-height:85vh;overflow:auto;border-radius:12px;padding:20px;box-shadow:0 20px 50px rgba(0,0,0,0.28);',
    });
    overlay.appendChild(panel);
    doc.body.appendChild(overlay);
  }
  const panel = overlay.firstElementChild || overlay;
  const close = () => {
    if (overlay && overlay.parentNode) overlay.parentNode.removeChild(overlay);
  };
  const mgr = createLessonsManager(panel, { ...deps, document: doc, onClose: close });
  if (!deps.initialLessons) mgr.refresh();
  return mgr;
}

export function mountLessonsNavButton(footerEl, deps = {}) {
  const doc = deps.document || footerEl?.ownerDocument || (typeof document !== 'undefined' ? document : null);
  if (!footerEl || !doc) return null;
  let btn = typeof footerEl.querySelector === 'function' ? footerEl.querySelector('#open-lessons-btn') : null;
  if (!btn) {
    btn = el(doc, 'button', 'pill-btn sm', LESSONS_PAGE_TITLE, {
      id: 'open-lessons-btn',
      type: 'button',
      'data-testid': 'open-lessons-btn',
      title: 'View, edit, or delete saved correction lessons',
    });
    footerEl.appendChild(btn);
  }
  if (!btn.__lessonsBound) {
    btn.__lessonsBound = true;
    btn.addEventListener('click', () => {
      if (typeof deps.onOpen === 'function') deps.onOpen();
      else openLessonsDialog({ ...deps, document: doc });
    });
  }
  return btn;
}
