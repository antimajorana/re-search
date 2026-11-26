// Investigate V10 reader notes composer and list mounted inside the node card
// (00-overview.md §2.10; investigate.md §4.3-4.4, §5.2, rule 6.12).
//
// Plain "you" style from F6 EVIDENCE_GRADES.user; zero innerHTML writes; notes are additive
// and survive re-runs intact, exposed cleanly for F15 reading notes and V16 hand-offs.

import { BORDER_SOLID, USER_TAG_WORD, createBadge } from './ui_badge.js';

// Named constants aligned with investigation_notes.rs & F7 MAX_USER_NOTE_CHARS.
export const MAX_NOTE_TEXT_CHARS = 4000;
export const NOTES_SECTION_TITLE = 'Your notes';
export const NOTE_PLACEHOLDER = 'Add a note on this statement (kept across re-runs)...';
export const NOTE_SAVE_LABEL = 'Save note';
export const NOTE_SAVING_LABEL = 'Saving...';
export const NOTE_EMPTY_ERROR = 'Write a note before saving.';
export const NOTE_STALE_TEXT = 'Saved on an earlier fingerprint of this node';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function extractNodeNotes(node) {
  if (!node || typeof node !== 'object') return [];
  const raw = Array.isArray(node.notes)
    ? node.notes
    : (Array.isArray(node.reading?.notes) ? node.reading.notes : []);
  return raw
    .filter((n) => n && typeof n === 'object' && String(n.text || '').trim().length > 0)
    .map((n) => ({
      seq: n.seq ?? null,
      tag: String(n.tag || node.id || ''),
      node_fp: String(n.node_fp || ''),
      stale: Boolean(n.stale),
      text: String(n.text).trim(),
    }));
}

/**
 * Renders the V10 reader notes section (chronological saved notes + bounded note composer)
 * using F6 `EVIDENCE_GRADES.user` ("you") styling.
 */
export function renderInvestigationNotes(host, node, opts = {}) {
  const section = el('section', 'inv-v10-notes');
  section.setAttribute('aria-label', NOTES_SECTION_TITLE);

  if (!node || typeof node !== 'object' || !node.id) {
    if (host) host.replaceChildren(section);
    return { element: section, focusInput: () => {}, setNotes: () => {} };
  }

  let currentNotes = extractNodeNotes(node);
  let saving = false;
  let errorMsg = '';

  const header = el('div', 'inv-v10-notes-header');
  header.appendChild(createBadge({
    grade: 'user',
    text: NOTES_SECTION_TITLE,
    border: BORDER_SOLID,
  }));
  const countEl = el('span', 'inv-v10-notes-count');
  header.appendChild(countEl);

  const listEl = el('ul', 'inv-v10-notes-list');
  const errorEl = el('div', 'inv-v10-notes-error');
  errorEl.setAttribute('role', 'alert');
  errorEl.hidden = true;

  const form = el('form', 'inv-v10-notes-form');
  const textarea = el('textarea', 'inv-v10-notes-input');
  textarea.placeholder = NOTE_PLACEHOLDER;
  textarea.maxLength = MAX_NOTE_TEXT_CHARS;
  textarea.setAttribute('aria-label', 'Reader note');
  textarea.setAttribute('rows', '2');

  const controls = el('div', 'inv-v10-notes-controls');
  const counterEl = el('span', 'inv-v10-notes-counter', `0 / ${MAX_NOTE_TEXT_CHARS}`);
  const saveBtn = el('button', 'inv-v10-notes-save', NOTE_SAVE_LABEL);
  saveBtn.type = 'submit';
  controls.append(counterEl, saveBtn);
  form.append(textarea, controls);

  function renderList() {
    countEl.textContent = currentNotes.length
      ? `${currentNotes.length} ${currentNotes.length === 1 ? 'note' : 'notes'}`
      : '';
    if (!currentNotes.length) {
      const empty = el('li', 'inv-v10-notes-empty', 'No notes yet.');
      listEl.replaceChildren(empty);
      return;
    }
    const items = currentNotes.map((note) => {
      const li = el('li', 'inv-v10-note-item');
      li.setAttribute('data-grade', 'user');
      li.setAttribute('data-border', BORDER_SOLID);
      if (note.seq != null) li.setAttribute('data-note-seq', String(note.seq));
      if (note.stale) li.setAttribute('data-stale', 'true');

      const meta = el('div', 'inv-v10-note-meta');
      meta.appendChild(createBadge({
        grade: 'user',
        text: USER_TAG_WORD,
        border: BORDER_SOLID,
      }));
      if (note.stale) {
        meta.appendChild(el('span', 'inv-v10-note-stale', NOTE_STALE_TEXT));
      }
      const body = el('p', 'inv-v10-note-text', note.text);
      li.append(meta, body);
      return li;
    });
    listEl.replaceChildren(...items);
  }

  function syncCounter() {
    const len = [...String(textarea.value || '')].length;
    counterEl.textContent = `${len} / ${MAX_NOTE_TEXT_CHARS}`;
  }

  function setError(msg) {
    errorMsg = String(msg || '').trim();
    errorEl.textContent = errorMsg;
    errorEl.hidden = !errorMsg;
  }

  function setBusy(nextBusy) {
    saving = Boolean(nextBusy);
    textarea.disabled = saving;
    saveBtn.disabled = saving;
    saveBtn.textContent = saving ? NOTE_SAVING_LABEL : NOTE_SAVE_LABEL;
  }

  textarea.addEventListener('input', () => {
    syncCounter();
    if (errorMsg) setError('');
  });

  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    if (saving) return;
    const raw = String(textarea.value || '').trim();
    if (!raw) {
      setError(NOTE_EMPTY_ERROR);
      return;
    }
    if ([...raw].length > MAX_NOTE_TEXT_CHARS) {
      setError(`Note must be at most ${MAX_NOTE_TEXT_CHARS} characters.`);
      return;
    }
    if (typeof opts.onSaveNote !== 'function') return;

    setError('');
    setBusy(true);
    try {
      const saved = await opts.onSaveNote({
        tag: node.id,
        node_fp: node.fp || undefined,
        text: raw,
      });
      if (saved && typeof saved === 'object') {
        currentNotes = [
          ...currentNotes,
          {
            seq: saved.seq ?? null,
            tag: String(saved.tag || node.id),
            node_fp: String(saved.node_fp || node.fp || ''),
            stale: false,
            text: String(saved.text || raw),
          },
        ];
      } else {
        currentNotes = [
          ...currentNotes,
          { seq: null, tag: node.id, node_fp: String(node.fp || ''), stale: false, text: raw },
        ];
      }
      textarea.value = '';
      syncCounter();
      renderList();
    } catch (err) {
      setError(err?.message || 'Could not save note.');
    } finally {
      setBusy(false);
    }
  });

  renderList();
  section.append(header, listEl, errorEl, form);
  if (host) host.replaceChildren(section);

  return {
    element: section,
    focusInput() {
      textarea.focus?.();
    },
    setNotes(nextNotes) {
      currentNotes = Array.isArray(nextNotes) ? nextNotes : [];
      renderList();
    },
  };
}
