// F16b "Remember this?" card (user's own words default, (mode, field) default scope,
// opt-in "(model)" draft only on explicit click) and "used N of your notes" disclosure.
// Plain ES module; DOM construction uses createElement and textContent only (zero innerHTML).
import { LESSONS_MAX_CHARS, formatOriginLabel, lessonsFetchJson, unicodeCharCount } from './lessons.js';

export const REMEMBER_CARD_TITLE = 'Remember this?';
export const DEFAULT_SCOPE_LABEL = 'this mode and this field';
export const DRAFT_MODEL_BTN_LABEL = 'Draft summary (model)';

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

export function normalizeInitialUserWords(raw) {
  const cleaned = String(raw || '').replace(/[\r\n\t]+/g, ' ').replace(/[\x00-\x1f\x7f]/g, '').replace(/\s+/g, ' ').trim();
  const chars = Array.from(cleaned);
  return {
    text: chars.slice(0, LESSONS_MAX_CHARS).join('').trim(),
    clipped: chars.length > LESSONS_MAX_CHARS,
  };
}

export function buildScopeChoices(ctx = {}) {
  const mode = String(ctx.mode || ctx.from?.mode || 'solve').trim().toLowerCase() || 'solve';
  const field = String(ctx.field || '').trim().toLowerCase();
  const paperKey = String(ctx.paperKey || ctx.from?.paper?.key || '').trim();
  const choices = [];
  if (field) {
    choices.push({ id: 'mode_field', label: DEFAULT_SCOPE_LABEL, kind: 'mode_field', mode, field });
  }
  choices.push({ id: 'mode', label: 'this mode', kind: 'mode', mode });
  if (field) {
    choices.push({ id: 'field', label: 'this field', kind: 'field', field });
  }
  if (paperKey) {
    choices.push({ id: 'paper', label: 'this paper', kind: 'paper', paper_key: paperKey });
  }
  choices.push({ id: 'global', label: 'global', kind: 'global' });
  return choices;
}

/**
 * Render the `"Remember this?"` card prefilled strictly with the user's own words
 * (zero automatic model calls, zero automatic DB writes).
 */
export function createRememberThisCard(doc, context = {}, deps = {}) {
  if (!doc) throw new Error('createRememberThisCard requires document');
  const fetchJson = (method, path, body) => lessonsFetchJson(method, path, body, deps);

  const initial = normalizeInitialUserWords(context.userWords || context.correctionText || '');
  let draftText = initial.text;
  let currentField = String(context.field || '').trim().toLowerCase();
  const mode = String(context.mode || context.from?.mode || 'solve').trim().toLowerCase() || 'solve';
  const paperKey = String(context.paperKey || context.from?.paper?.key || '').trim();
  let selectedScopeId = currentField ? 'mode_field' : 'mode';
  let modelSummary = false;
  let busy = false;
  let errorMsg = '';

  const root = el(doc, 'aside', 'remember-lesson-card', null, { 'data-testid': 'remember-lesson-card' });

  function currentScopePayload() {
    if (selectedScopeId === 'mode_field') {
      if (!currentField) return { kind: 'mode', mode };
      return { kind: 'mode_field', mode, field: currentField };
    }
    if (selectedScopeId === 'mode') return { kind: 'mode', mode };
    if (selectedScopeId === 'field') return currentField ? { kind: 'field', field: currentField } : { kind: 'mode', mode };
    if (selectedScopeId === 'paper' && paperKey) return { kind: 'paper', paper_key: paperKey };
    return { kind: 'global' };
  }

  function render() {
    root.replaceChildren();
    const head = el(doc, 'div', 'remember-head');
    head.appendChild(el(doc, 'strong', 'remember-title', REMEMBER_CARD_TITLE));
    if (modelSummary) {
      head.appendChild(el(doc, 'span', 'remember-model-tag', '(model)', { 'data-testid': 'remember-model-badge' }));
    }
    head.appendChild(el(doc, 'span', 'remember-origin', `From ${formatOriginLabel(context.from)}`, { 'data-testid': 'remember-origin' }));
    root.appendChild(head);

    if (initial.clipped) {
      root.appendChild(el(doc, 'div', 'remember-hint', `Clipped to ${LESSONS_MAX_CHARS} characters for saving.`));
    }
    if (errorMsg) {
      root.appendChild(el(doc, 'div', 'remember-error', errorMsg, { 'data-testid': 'remember-error', role: 'alert' }));
    }

    const textarea = el(doc, 'textarea', 'remember-textarea', null, {
      rows: '3',
      'data-testid': 'remember-text-input',
      placeholder: 'Your note to future runs (<= 300 characters)',
    });
    textarea.value = draftText;

    const countSpan = el(doc, 'span', 'remember-char-count', `${unicodeCharCount(draftText)} / ${LESSONS_MAX_CHARS}`, {
      'data-testid': 'remember-char-count',
    });

    const scopeRow = el(doc, 'div', 'remember-scope-row');
    const choices = buildScopeChoices({ mode, field: currentField, paperKey });
    if (!choices.some((c) => c.id === selectedScopeId)) selectedScopeId = choices[0].id;
    const scopeSelect = el(doc, 'select', 'remember-scope-select', null, { 'data-testid': 'remember-scope-select' });
    for (const c of choices) {
      const opt = el(doc, 'option', null, c.label, { value: c.id });
      if (c.id === selectedScopeId) opt.selected = true;
      scopeSelect.appendChild(opt);
    }
    scopeSelect.addEventListener('change', () => {
      selectedScopeId = scopeSelect.value;
    });

    const fieldInput = el(doc, 'input', 'remember-field-input', null, {
      type: 'text',
      placeholder: 'field (optional, e.g. qft)',
      'data-testid': 'remember-field-input',
    });
    fieldInput.value = currentField;
    fieldInput.addEventListener('change', () => {
      currentField = String(fieldInput.value || '').trim().toLowerCase();
      if (currentField && selectedScopeId === 'mode') selectedScopeId = 'mode_field';
      render();
    });

    scopeRow.append(el(doc, 'label', 'remember-scope-label', 'Scope:'), scopeSelect, fieldInput);

    const actions = el(doc, 'div', 'remember-actions');
    const saveBtn = el(doc, 'button', 'pill-btn sm primary', 'Save note', {
      type: 'button',
      'data-action': 'remember-save',
      'data-testid': 'remember-save-btn',
    });
    const draftModelBtn = el(doc, 'button', 'pill-btn sm', DRAFT_MODEL_BTN_LABEL, {
      type: 'button',
      'data-action': 'remember-draft-model',
      'data-testid': 'remember-draft-model-btn',
    });
    const dismissBtn = el(doc, 'button', 'pill-btn sm', 'Dismiss', {
      type: 'button',
      'data-action': 'remember-dismiss',
      'data-testid': 'remember-dismiss-btn',
    });

    const updateCounterState = () => {
      draftText = textarea.value;
      const len = unicodeCharCount(draftText.trim());
      countSpan.textContent = `${len} / ${LESSONS_MAX_CHARS}`;
      const bad = len === 0 || len > LESSONS_MAX_CHARS || busy;
      countSpan.className = (len === 0 || len > LESSONS_MAX_CHARS) ? 'remember-char-count over' : 'remember-char-count';
      saveBtn.disabled = bad;
    };
    textarea.addEventListener('input', () => {
      modelSummary = false;
      errorMsg = '';
      updateCounterState();
    });

    saveBtn.addEventListener('click', async () => {
      if (busy) return;
      const trimmed = String(textarea.value || '').trim();
      const len = unicodeCharCount(trimmed);
      if (len === 0 || len > LESSONS_MAX_CHARS) {
        errorMsg = `Note must be 1..=${LESSONS_MAX_CHARS} characters (got ${len}).`;
        render();
        return;
      }
      busy = true;
      errorMsg = '';
      const payload = {
        text: trimmed,
        scope: currentScopePayload(),
        from: context.from || { session: 'session', mode, id: 'step' },
        source_kind: context.sourceKind || 'step_correction',
        model_summary: modelSummary,
      };
      try {
        const res = typeof deps.onSave === 'function'
          ? await deps.onSave(payload)
          : await fetchJson('POST', '/api/lessons/notes', payload);
        if (root.parentNode) root.parentNode.removeChild(root);
        deps.onSaved?.(res);
      } catch (err) {
        busy = false;
        errorMsg = err?.message || 'Failed to save note.';
        render();
      }
    });

    draftModelBtn.addEventListener('click', async () => {
      if (busy) return;
      const words = String(textarea.value || context.userWords || '').trim();
      if (!words) {
        errorMsg = 'Enter correction words first.';
        render();
        return;
      }
      busy = true;
      errorMsg = '';
      render();
      const req = {
        user_words: words,
        mode,
        field: currentField || null,
        target_summary: context.targetSummary || null,
      };
      try {
        const res = typeof deps.onDraftSummary === 'function'
          ? await deps.onDraftSummary(req)
          : await fetchJson('POST', '/api/lessons/notes/draft-summary', req);
        draftText = String(res?.text || '').trim() || draftText;
        modelSummary = Boolean(res?.model_summary ?? true);
      } catch (err) {
        errorMsg = err?.message || 'Could not draft summary with model.';
      } finally {
        busy = false;
        render();
      }
    });

    dismissBtn.addEventListener('click', () => {
      if (root.parentNode) root.parentNode.removeChild(root);
      deps.onDismiss?.();
    });

    if (busy) {
      saveBtn.disabled = true;
      draftModelBtn.disabled = true;
    }
    actions.append(countSpan, saveBtn, draftModelBtn, dismissBtn);
    root.append(textarea, scopeRow, actions);
  }

  render();
  return {
    element: root,
    destroy: () => { if (root.parentNode) root.parentNode.removeChild(root); },
  };
}

/**
 * Compact `"used N of your notes"` disclosure chip & expandable quoted-notes drawer.
 * Strictly displays the ledger-frozen `supplied` payload (never reads live `lessons.db`).
 */
export function createUsedLessonsDisclosure(doc, suppliedData, deps = {}) {
  if (!doc) throw new Error('createUsedLessonsDisclosure requires document');
  const root = el(doc, 'div', 'used-lessons-disclosure', null, { 'data-testid': 'used-lessons-disclosure' });
  let current = suppliedData && typeof suppliedData === 'object' ? suppliedData : null;
  let expanded = false;

  function render() {
    root.replaceChildren();
    const count = Number(current?.count || 0);
    if (!Number.isFinite(count) || count <= 0) {
      root.hidden = true;
      return;
    }
    root.hidden = false;
    const label = String(current?.disclosure || `used ${count} of your notes`);
    const pill = el(doc, 'button', 'pill-btn sm used-lessons-pill', label, {
      type: 'button',
      'aria-expanded': expanded ? 'true' : 'false',
      'data-testid': 'used-lessons-pill',
    });
    pill.addEventListener('click', () => {
      expanded = !expanded;
      render();
    });
    root.appendChild(pill);

    if (expanded) {
      const panel = el(doc, 'div', 'used-lessons-panel', null, { 'data-testid': 'used-lessons-panel' });
      const records = Array.isArray(current?.records) ? current.records : [];
      const list = el(doc, 'ul', 'used-lessons-list');
      let renderedNotes = 0;
      for (const rec of records) {
        const notes = Array.isArray(rec?.notes) ? rec.notes : [];
        for (const note of notes) {
          renderedNotes += 1;
          const li = el(doc, 'li', 'used-lesson-item', null, { 'data-testid': 'used-lesson-item' });
          li.appendChild(el(doc, 'span', 'used-lesson-text', note.text || ''));
          if (note.scope_label) {
            li.appendChild(el(doc, 'span', 'used-lesson-scope', ` (${note.scope_label})`));
          }
          if (note.from) {
            const originBtn = el(doc, 'button', 'used-lesson-origin', formatOriginLabel(note.from), {
              type: 'button', 'data-testid': 'used-lesson-origin',
            });
            originBtn.addEventListener('click', () => deps.onNavigateOrigin?.(note.from));
            li.appendChild(originBtn);
          }
          list.appendChild(li);
        }
        if (notes.length === 0 && rec?.blob) {
          renderedNotes += 1;
          list.appendChild(el(doc, 'li', 'used-lesson-item', rec.blob, { 'data-testid': 'used-lesson-item' }));
        }
      }
      if (renderedNotes === 0) {
        list.appendChild(el(doc, 'li', 'used-lesson-item', label));
      }
      panel.appendChild(list);
      root.appendChild(panel);
    }
  }

  render();
  return {
    element: root,
    update: (nextData) => { current = nextData; render(); },
    destroy: () => { if (root.parentNode) root.parentNode.removeChild(root); },
  };
}

export function mountUsedLessonsDisclosure(hostEl, suppliedData, deps = {}) {
  const doc = deps.document || hostEl?.ownerDocument || (typeof document !== 'undefined' ? document : null);
  if (!hostEl || !doc) return null;
  const widget = createUsedLessonsDisclosure(doc, suppliedData, deps);
  hostEl.prepend(widget.element);
  return widget;
}
