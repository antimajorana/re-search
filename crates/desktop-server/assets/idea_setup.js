// Explore home-screen setup card mounted in F5's home screen (`new_menu.js`, package X4).
// Zero innerHTML writes: all untrusted text uses textContent + safe DOM APIs and F6 createBadge.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any rendered label.
import { createExploreSession, previewIdea } from './api_idea.js';
import {
  BADGE_IN_LIBRARY,
  BADGE_OWNED_LIBRARY,
  EXPLORE_SUBMIT_LABEL,
  IDEA_PRESETS,
  MAX_IDEA_CHARS,
  NO_MODEL_NOTICE,
  formatPresetSummary,
  isBarePaperIdentifier,
} from './idea_state.js';
import { createBadge } from './ui_badge.js';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function parseSeedList(raw) {
  return String(raw || '')
    .split(/[\s,;]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function unicodeCharCount(str) {
  return [...String(str || '')].length;
}

/**
 * Render the Explore setup card inside `containerEl`.
 */
export function renderIdeaSetupCard(containerEl, opts = {}) {
  if (!containerEl) return null;
  const previewFn = typeof opts.previewFn === 'function' ? opts.previewFn : previewIdea;
  const createFn = typeof opts.createFn === 'function' ? opts.createFn : createExploreSession;
  const debounceMs = Number.isFinite(Number(opts.debounceMs)) ? Number(opts.debounceMs) : 220;

  let selectedPreset = 'standard';
  let offline = false;
  let busy = false;
  let previewTimer = null;
  let previewSeq = 0;
  let lastPreview = null;
  let lastPreviewKey = null;
  let checkedQueries = new Set();

  const card = el('section', 'idea-setup-card');
  card.setAttribute('data-explore-setup', 'true');
  card.setAttribute('aria-label', 'Explore an idea');

  const head = el('div', 'idea-card-head');
  head.append(
    el('h2', 'idea-card-title', 'Explore an idea'),
    el('span', 'idea-card-meta', 'TeX allowed · local preview uses zero network'),
  );
  card.append(head);

  if (!opts.hasModel) {
    const notice = el('div', 'idea-banner idea-banner--notice');
    notice.setAttribute('data-idea-banner', 'no-model');
    notice.append(
      createBadge({ text: 'No model', icon: 'info', tone: 'info' }),
      el('span', '', NO_MODEL_NOTICE),
    );
    card.append(notice);
  }

  const form = el('form', 'idea-field-group');
  form.setAttribute('novalidate', '');

  const textGroup = el('div', 'idea-field-group');
  const textLabel = el('label', 'idea-field-label', 'Your idea');
  textLabel.setAttribute('for', 'idea-setup-text');
  const textarea = el('textarea', 'idea-textarea');
  textarea.id = 'idea-setup-text';
  textarea.rows = 4;
  textarea.placeholder =
    'Describe your physics or math idea, expected scaling, or conjecture (TeX allowed)...';
  const charRow = el('div', 'idea-char-row');
  const charCountEl = el('span', '', `0 / ${MAX_IDEA_CHARS}`);
  const shortcutHint = el('span', '', 'Ctrl+Enter to submit');
  charRow.append(charCountEl, shortcutHint);
  textGroup.append(textLabel, textarea, charRow);

  const seedGroup = el('div', 'idea-field-group');
  const seedLabel = el('label', 'idea-field-label', 'Papers you already know (optional)');
  seedLabel.setAttribute('for', 'idea-setup-seeds');
  const seedInput = el('input', 'text-input');
  seedInput.id = 'idea-setup-seeds';
  seedInput.type = 'text';
  seedInput.placeholder = 'arXiv IDs or DOIs, e.g. 2405.01234, 10.1103/...';
  seedGroup.append(seedLabel, seedInput);

  const controlsRow = el('div', 'idea-controls-row');
  const presetGroup = el('div', 'idea-preset-group');
  presetGroup.setAttribute('role', 'group');
  presetGroup.setAttribute('aria-label', 'Explore preset');
  const presetButtons = new Map();

  for (const preset of IDEA_PRESETS) {
    const btn = el('button', 'idea-preset-btn');
    btn.type = 'button';
    btn.setAttribute('data-preset', preset.id);
    btn.setAttribute('aria-pressed', preset.id === selectedPreset ? 'true' : 'false');
    btn.title = formatPresetSummary(preset);
    btn.append(
      el('strong', '', preset.label),
      el(
        'span',
        'idea-preset-price',
        `${preset.max_queries}q · ${preset.max_papers}p · at most ${preset.max_model_calls} calls`,
      ),
    );
    btn.addEventListener('click', () => {
      selectedPreset = preset.id;
      for (const [id, b] of presetButtons) {
        b.setAttribute('aria-pressed', id === selectedPreset ? 'true' : 'false');
      }
      updateSubmitState();
    });
    presetButtons.set(preset.id, btn);
    presetGroup.append(btn);
  }

  const offlineLabel = el('label', 'idea-offline-label');
  const offlineCheck = el('input');
  offlineCheck.id = 'idea-setup-offline';
  offlineCheck.type = 'checkbox';
  offlineCheck.checked = offline;
  offlineCheck.addEventListener('change', () => {
    offline = Boolean(offlineCheck.checked);
    renderPreviewArea();
    updateSubmitState();
  });
  offlineLabel.append(offlineCheck, el('span', '', 'Offline: library only'));

  controlsRow.append(presetGroup, offlineLabel);

  const previewBox = el('div', 'idea-preview-box');
  previewBox.setAttribute('data-idea-preview', 'true');
  previewBox.hidden = true;

  const statusEl = el('div', 'idea-banner idea-banner--warn');
  statusEl.setAttribute('role', 'alert');
  statusEl.hidden = true;

  const actionRow = el('div', 'idea-controls-row');
  const submitBtn = el('button', 'pill-btn primary', EXPLORE_SUBMIT_LABEL);
  submitBtn.id = 'idea-setup-submit';
  submitBtn.type = 'submit';
  submitBtn.disabled = true;
  actionRow.append(el('span', 'idea-card-meta', 'First queries run only as shown above.'), submitBtn);

  form.append(textGroup, seedGroup, controlsRow, previewBox, statusEl, actionRow);
  card.append(form);
  containerEl.replaceChildren(card);

  function currentInputSnapshot() {
    const trimmed = String(textarea.value || '').trim();
    const seeds = parseSeedList(seedInput.value);
    return { trimmed, seeds, key: `${trimmed}\0${seeds.join('\0')}` };
  }

  function setStatus(msg) {
    if (!msg) {
      statusEl.hidden = true;
      statusEl.textContent = '';
    } else {
      statusEl.hidden = false;
      statusEl.textContent = String(msg);
    }
  }

  function selectedQueryPayload() {
    const list = Array.isArray(lastPreview?.first_queries) ? lastPreview.first_queries : [];
    return list
      .filter((q) => checkedQueries.has(`${q.provider}::${q.q}`))
      .map((q) => ({ provider: q.provider, q: q.q }));
  }

  function isPreviewCurrent(snap) {
    return Boolean(lastPreview && lastPreviewKey === snap.key);
  }

  function updateSubmitState() {
    const snap = currentInputSnapshot();
    const len = unicodeCharCount(snap.trimmed);
    charCountEl.textContent = `${len} / ${MAX_IDEA_CHARS}`;
    charRow.classList.toggle('is-over', len > MAX_IDEA_CHARS);
    const previewReady = isPreviewCurrent(snap);
    const hasInvalidSeeds = previewReady && Array.isArray(lastPreview?.invalid_seeds) && lastPreview.invalid_seeds.length > 0;
    const hasQueries = offline || selectedQueryPayload().length > 0;
    submitBtn.disabled =
      busy ||
      len === 0 ||
      len > MAX_IDEA_CHARS ||
      isBarePaperIdentifier(snap.trimmed) ||
      hasInvalidSeeds ||
      (!offline && !previewReady) ||
      !hasQueries;
  }

  function renderPreviewArea() {
    previewBox.replaceChildren();
    if (!lastPreview) {
      previewBox.hidden = true;
      return;
    }
    const terms = Array.isArray(lastPreview.terms) ? lastPreview.terms : [];
    const refs = Array.isArray(lastPreview.paper_refs) ? lastPreview.paper_refs : [];
    const matches = Array.isArray(lastPreview.library_matches) ? lastPreview.library_matches : [];
    const queries = Array.isArray(lastPreview.first_queries) ? lastPreview.first_queries : [];

    if (terms.length === 0 && refs.length === 0 && matches.length === 0 && queries.length === 0) {
      previewBox.hidden = true;
      return;
    }
    previewBox.hidden = false;

    if (terms.length > 0) {
      const row = el('div', 'idea-chip-row');
      row.setAttribute('data-preview-section', 'terms');
      for (const t of terms.slice(0, 12)) {
        row.append(el('span', 'idea-chip', typeof t === 'string' ? t : t.text || ''));
      }
      previewBox.append(el('span', 'idea-field-label', 'Extracted terms'), row);
    }

    if (refs.length > 0) {
      const row = el('div', 'idea-chip-row');
      row.setAttribute('data-preview-section', 'paper-refs');
      for (const r of refs) {
        row.append(createBadge({ text: String(r.key || ''), icon: 'link', tone: 'info' }));
      }
      previewBox.append(el('span', 'idea-field-label', 'Detected paper identifiers'), row);
    }

    if (matches.length > 0) {
      const list = el('div', 'idea-query-list');
      list.setAttribute('data-preview-section', 'library-matches');
      for (const m of matches.slice(0, 5)) {
        const row = el('div', 'idea-query-chip');
        row.append(createBadge({ text: BADGE_IN_LIBRARY, icon: 'pin', tone: 'ok' }));
        if (m.owned) {
          row.append(createBadge({ text: BADGE_OWNED_LIBRARY, icon: 'code', tone: 'info' }));
        }
        row.append(el('span', '', m.paper?.title || m.paper?.key || ''));
        list.append(row);
      }
      previewBox.append(
        el('span', 'idea-field-label', `${matches.length} paper(s) in your library`),
        list,
      );
    }

    if (queries.length > 0) {
      const qList = el('div', 'idea-query-list');
      qList.setAttribute('data-preview-section', 'first-queries');
      for (const q of queries) {
        const key = `${q.provider}::${q.q}`;
        const row = el('label', 'idea-query-chip');
        const cb = el('input');
        cb.type = 'checkbox';
        cb.checked = !offline && checkedQueries.has(key);
        cb.disabled = offline;
        cb.addEventListener('change', () => {
          if (cb.checked) checkedQueries.add(key);
          else checkedQueries.delete(key);
          updateSubmitState();
        });
        row.append(
          cb,
          el('span', 'idea-query-provider', q.provider || ''),
          el('span', 'idea-query-text', q.q || ''),
        );
        qList.append(row);
      }
      previewBox.append(
        el(
          'span',
          'idea-field-label',
          offline ? 'First queries (skipped in offline mode)' : `First queries (${queries.length})`,
        ),
        qList,
      );
    }
  }

  function invalidatePreview() {
    previewSeq += 1;
    lastPreview = null;
    lastPreviewKey = null;
    checkedQueries = new Set();
    renderPreviewArea();
  }

  async function runPreviewNow() {
    if (previewTimer) {
      clearTimeout(previewTimer);
      previewTimer = null;
    }
    const snap = currentInputSnapshot();
    if (!snap.trimmed) {
      invalidatePreview();
      setStatus('');
      updateSubmitState();
      return null;
    }
    if (isBarePaperIdentifier(snap.trimmed)) {
      invalidatePreview();
      if (typeof opts.onFocusInvestigate === 'function') {
        opts.onFocusInvestigate(snap.trimmed);
      }
      updateSubmitState();
      return null;
    }
    const seq = ++previewSeq;
    try {
      const data = await previewFn({ text: snap.trimmed, seeds: snap.seeds });
      if (seq !== previewSeq) return null;
      lastPreview = data;
      lastPreviewKey = snap.key;
      checkedQueries = new Set(
        (Array.isArray(data?.first_queries) ? data.first_queries : []).map(
          (q) => `${q.provider}::${q.q}`,
        ),
      );
      if (data?.focus_investigate && typeof opts.onFocusInvestigate === 'function') {
        opts.onFocusInvestigate(snap.trimmed);
      }
      const badSeeds = Array.isArray(data?.invalid_seeds) ? data.invalid_seeds : [];
      setStatus(badSeeds.length > 0 ? `Unrecognized paper identifier(s): ${badSeeds.join(', ')}` : '');
      renderPreviewArea();
      updateSubmitState();
      return data;
    } catch (err) {
      if (seq !== previewSeq) return null;
      invalidatePreview();
      setStatus(err?.message || 'Preview failed');
      updateSubmitState();
      return null;
    }
  }

  function schedulePreview() {
    const snap = currentInputSnapshot();
    if (snap.key !== lastPreviewKey) {
      invalidatePreview();
    }
    if (isBarePaperIdentifier(snap.trimmed) && typeof opts.onFocusInvestigate === 'function') {
      opts.onFocusInvestigate(snap.trimmed);
      updateSubmitState();
      return;
    }
    updateSubmitState();
    if (previewTimer) clearTimeout(previewTimer);
    if (debounceMs <= 0) {
      void runPreviewNow();
      return;
    }
    previewTimer = setTimeout(() => {
      previewTimer = null;
      void runPreviewNow();
    }, debounceMs);
  }

  async function submitExplore(ev) {
    if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
    if (busy) return null;
    if (previewTimer) {
      clearTimeout(previewTimer);
      previewTimer = null;
    }
    const snap = currentInputSnapshot();
    const len = unicodeCharCount(snap.trimmed);
    if (len === 0 || len > MAX_IDEA_CHARS) {
      updateSubmitState();
      return null;
    }
    if (isBarePaperIdentifier(snap.trimmed)) {
      if (typeof opts.onFocusInvestigate === 'function') opts.onFocusInvestigate(snap.trimmed);
      updateSubmitState();
      return null;
    }
    if (!offline && !isPreviewCurrent(snap)) {
      await runPreviewNow();
    }
    // Re-check after awaiting preview (Rule P4: never submit stale or zero online queries).
    const afterSnap = currentInputSnapshot();
    if (!offline && !isPreviewCurrent(afterSnap)) {
      updateSubmitState();
      return null;
    }
    if (Array.isArray(lastPreview?.invalid_seeds) && lastPreview.invalid_seeds.length > 0) {
      setStatus(`Unrecognized paper identifier(s): ${lastPreview.invalid_seeds.join(', ')}`);
      updateSubmitState();
      return null;
    }
    const queries = offline ? [] : selectedQueryPayload();
    if (!offline && queries.length === 0) {
      setStatus('Select at least one preview query or switch on Offline: library only.');
      updateSubmitState();
      return null;
    }

    busy = true;
    setStatus('');
    updateSubmitState();
    try {
      const res = await createFn({
        text: afterSnap.trimmed,
        seeds: afterSnap.seeds,
        preset: selectedPreset,
        offline,
        queries,
      });
      if (typeof opts.onCreated === 'function') opts.onCreated(res);
      return res;
    } catch (err) {
      setStatus(err?.message || 'Could not start Explore session');
      return null;
    } finally {
      busy = false;
      updateSubmitState();
    }
  }

  textarea.addEventListener('input', schedulePreview);
  textarea.addEventListener('paste', (ev) => {
    const pasted = ev?.clipboardData?.getData?.('text') || '';
    if (pasted && isBarePaperIdentifier(pasted.trim()) && typeof opts.onFocusInvestigate === 'function') {
      opts.onFocusInvestigate(pasted.trim());
    }
  });
  seedInput.addEventListener('input', schedulePreview);
  textarea.addEventListener('keydown', (ev) => {
    if (ev && ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey)) {
      void submitExplore(ev);
    }
  });
  form.addEventListener('submit', (ev) => {
    void submitExplore(ev);
  });

  updateSubmitState();
  return {
    element: card,
    runPreviewNow,
    submitExplore,
    getState: () => ({
      text: textarea.value,
      preset: selectedPreset,
      offline,
      busy,
      preview: lastPreview,
      previewCurrent: isPreviewCurrent(currentInputSnapshot()),
      selectedQueries: selectedQueryPayload(),
    }),
  };
}
