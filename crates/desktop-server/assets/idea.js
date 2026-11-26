// Explore session page shell (package X4).
// Renders verbatim Idea card, 7-stage rail mapped through F9 run states, shared F6 run_bar.js
// cost line, no-model notice, empty/failure states, candidate list, and "Where I looked" skeleton.
// Zero innerHTML writes. Rule S8: zero "verified", "new", "novel", or "proved" strings in DOM.
import { getExploreSession, postIdeaOp } from './api_idea.js';
import { renderCandidateCards, renderCandidateList, renderWhereILooked } from './idea_search.js';
import {
  NO_MODEL_NOTICE,
  createInitialIdeaState,
  ideaReducer,
  selectEmptyState,
  selectFilteredCandidates,
  selectRunBarModel,
  selectStageRail,
} from './idea_state.js';
import { createRunBar } from './run_bar.js';
import { createBadge } from './ui_badge.js';

const STOPWORDS = new Set([
  'that', 'this', 'with', 'from', 'have', 'were', 'been', 'will', 'their', 'about', 'into',
  'when', 'what', 'which', 'where', 'than', 'then', 'them', 'they', 'some', 'such', 'only',
  'also', 'does', 'using', 'used', 'under', 'over', 'between', 'across', 'expect',
]);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function extractHighlightTerms(view) {
  const structured = [];
  if (Array.isArray(view?.brief?.terms)) {
    for (const entry of view.brief.terms) {
      if (entry?.term) structured.push(String(entry.term));
      if (Array.isArray(entry?.aliases)) {
        for (const a of entry.aliases) structured.push(String(a));
      }
    }
  }
  if (Array.isArray(view?.terms)) {
    for (const t of view.terms) {
      structured.push(String(typeof t === 'string' ? t : t?.text || ''));
    }
  }
  if (structured.length > 0) {
    return Array.from(new Set(structured.map((s) => s.trim()).filter((s) => s.length >= 2))).slice(0, 16);
  }
  const text = String(view?.statement?.text || '');
  if (!text) return [];
  const words = text
    .split(/[^A-Za-z0-9_-]+/)
    .map((w) => w.trim())
    .filter((w) => {
      if (w.length < 2) return false;
      const low = w.toLowerCase();
      if (STOPWORDS.has(low)) return false;
      return w.length >= 4 || (w.length >= 2 && w === w.toUpperCase() && /[A-Z]/.test(w));
    });
  return Array.from(new Set(words)).slice(0, 16);
}

/**
 * Render the Explore session view cleanly into `hostEl` using safe DOM APIs only.
 */
export function renderExploreSession(hostEl, state, callbacks = {}) {
  if (!hostEl) return null;
  const view = state?.view || {};
  const shell = el('div', 'idea-session-shell');
  shell.setAttribute('data-idea-shell', 'true');

  // 1. Stage rail: Idea - Sharpen - Search - Triage - Read - Map - Directions (`explore.md` 3).
  const rail = el('nav', 'idea-stage-rail');
  rail.setAttribute('aria-label', 'Explore stages');
  rail.setAttribute('data-idea-section', 'stage-rail');
  for (const stage of selectStageRail(state)) {
    const pill = el('div', 'idea-stage-pill');
    pill.setAttribute('data-stage-id', stage.id);
    pill.setAttribute('data-stage-state', stage.runStateKey || 'idle');
    pill.append(
      el('strong', '', stage.label),
      createBadge({ text: stage.runStateLabel, icon: stage.icon, tone: stage.tone }),
    );
    rail.append(pill);
  }
  shell.append(rail);

  // 2. Shared F6 run_bar.js cost line (`explore.md` 3 & `00-overview.md` 2.6, 3.3).
  const runBarHost = el('div', 'idea-run-bar-host');
  runBarHost.setAttribute('data-idea-section', 'run-bar');
  const rbModel = selectRunBarModel(state);
  createRunBar(runBarHost, rbModel, {
    onStop: callbacks.onStop,
    onContinue: callbacks.onContinue,
    onRetry: callbacks.onRetry,
  });
  shell.append(runBarHost);

  // 3. No-model banner (`explore.md` 3).
  if (!state?.hasModel) {
    const notice = el('div', 'idea-banner idea-banner--notice');
    notice.setAttribute('data-idea-banner', 'no-model');
    notice.setAttribute('role', 'status');
    notice.append(
      createBadge({ text: 'No model', icon: 'info', tone: 'info' }),
      el('span', '', String(view.no_model_notice || NO_MODEL_NOTICE)),
    );
    shell.append(notice);
  }

  // 4. Notice / error / empty-or-failure state banner (`explore.md` 3).
  if (state?.notice) {
    const noteBanner = el('div', 'idea-banner idea-banner--notice');
    noteBanner.setAttribute('data-idea-banner', 'status-notice');
    noteBanner.setAttribute('role', 'status');
    noteBanner.append(
      createBadge({ text: 'Status', icon: 'info', tone: 'info' }),
      el('span', '', String(state.notice)),
    );
    shell.append(noteBanner);
  }
  if (state?.error) {
    const errBanner = el('div', 'idea-banner idea-banner--err');
    errBanner.setAttribute('role', 'alert');
    errBanner.append(
      createBadge({ text: 'Error', icon: 'error', tone: 'err' }),
      el('span', '', String(state.error)),
    );
    shell.append(errBanner);
  }

  const emptyState = selectEmptyState(state);
  if (emptyState) {
    const toneClass =
      emptyState.code === 'all_services_failed' || emptyState.code === 'stage_failed'
        ? 'idea-banner--err'
        : 'idea-banner--warn';
    const banner = el('div', `idea-banner ${toneClass}`);
    banner.setAttribute('data-empty-state', emptyState.code);
    banner.setAttribute('role', 'status');
    const left = el('div', 'idea-candidate-badges');
    left.append(
      createBadge({
        text: emptyState.code === 'offline_library_only' ? 'Offline' : 'Notice',
        icon: emptyState.canRetry ? 'error' : 'warn',
        tone: emptyState.canRetry ? 'err' : 'warn',
      }),
      el('span', '', emptyState.message),
    );
    banner.append(left);
    if (emptyState.canRetry && typeof callbacks.onRetry === 'function') {
      const retryBtn = el('button', 'pill-btn sm', 'Retry');
      retryBtn.type = 'button';
      retryBtn.addEventListener('click', () => callbacks.onRetry());
      banner.append(retryBtn);
    }
    shell.append(banner);
  }

  // 5. Verbatim Idea card (`explore.md` 3 & 4.1).
  const stmt = view.statement;
  const ideaCard = el('section', 'idea-verbatim-card');
  ideaCard.setAttribute('data-idea-section', 'idea-card');
  const ideaHead = el('div', 'idea-card-head');
  ideaHead.append(
    el('h3', 'idea-card-title', `Idea (rev ${Number(stmt?.rev || 1)})`),
    createBadge({ text: 'you', icon: 'user', tone: 'neutral' }),
  );
  ideaCard.append(ideaHead);
  ideaCard.append(el('p', 'idea-verbatim-text', stmt?.text || 'No idea statement recorded.'));
  if (view.map_stale) {
    ideaCard.append(createBadge({ text: 'Map stale after revision', icon: 'warn', tone: 'warn' }));
  }
  shell.append(ideaCard);

  // 6. Candidate list (`idea_search.js`).
  const candidates = selectFilteredCandidates(state);
  const terms = extractHighlightTerms(view);
  shell.append(
    renderCandidateList(candidates, {
      highlightTerms: terms,
      filters: state?.filters,
      onFilterChange: callbacks.onFilterChange,
    }),
  );

  // 7. "Where I looked" skeleton (`idea_search.js`).
  shell.append(renderWhereILooked(view.where_i_looked));

  // 8. Map placeholder (F6 GraphView populated in X11/X12).
  const mapNodes = Array.isArray(view.map?.nodes) ? view.map.nodes : [];
  const mapBox = el('section', 'idea-where-panel');
  mapBox.setAttribute('data-idea-section', 'map-placeholder');
  mapBox.append(
    el('h3', 'idea-card-title', 'Concept map'),
    el(
      'p',
      'idea-card-meta',
      mapNodes.length === 0
        ? 'No concept map yet. Sharpening, reading, and mapping build the map once a model is connected.'
        : `${mapNodes.length} map node(s) recorded.`,
    ),
  );
  shell.append(mapBox);

  hostEl.replaceChildren(shell);
  return shell;
}

/**
 * Stateful Explore session controller coordinating REST ops, focus-safe filters, and SSE updates.
 */
export function createExploreSessionController(hostEl, deps = {}) {
  const fetchSession = deps.getExploreSession || getExploreSession;
  const runOp = deps.postIdeaOp || postIdeaOp;
  let state = createInitialIdeaState({ hasModel: Boolean(deps.hasModel) });
  let loadSeq = 0;

  function patchCandidateCardsOnly() {
    const countEl = hostEl?.querySelector?.('[data-idea-candidate-count="true"]');
    const oldCards = hostEl?.querySelector?.('[data-idea-candidate-cards="true"]');
    const section = hostEl?.querySelector?.('[data-idea-section="candidates"]');
    if (!section || !oldCards) return false;
    const filtered = selectFilteredCandidates(state);
    if (countEl) countEl.textContent = `Candidates (${filtered.length})`;
    const freshCards = renderCandidateCards(filtered, extractHighlightTerms(state.view));
    const idx = Array.isArray(section.children) ? section.children.indexOf(oldCards) : -1;
    if (typeof oldCards.replaceWith === 'function') {
      oldCards.replaceWith(freshCards);
      return true;
    }
    if (idx >= 0 && Array.isArray(section.children)) {
      section.children.splice(idx, 1, freshCards);
      return true;
    }
    return false;
  }

  function dispatch(action, { candidatesOnly = false } = {}) {
    state = ideaReducer(state, action);
    if (candidatesOnly && patchCandidateCardsOnly()) return;
    render();
  }

  function render() {
    renderExploreSession(hostEl, state, {
      onFilterChange: (patch) => dispatch({ type: 'SET_FILTER', patch }, { candidatesOnly: true }),
      onStop: () => void invokeOp('stop'),
      onContinue: () => void handleContinueClick(),
      onRetry: () => void handleRetryClick(),
    });
  }

  async function handleContinueClick() {
    const rb = selectRunBarModel(state);
    const payload = rb.state === 'paused'
      ? { extra_queries: 6, extra_papers: 2, extra_model_calls: 6 }
      : {};
    return invokeOp('continue', payload);
  }

  async function handleRetryClick() {
    return invokeOp('continue', {});
  }

  async function invokeOp(op, payload = {}) {
    const sid = state.sessionId;
    if (!sid) return null;
    const seq = loadSeq;
    dispatch({ type: 'SET_ERROR', error: null });
    dispatch({ type: 'SET_NOTICE', notice: null });
    dispatch({ type: 'SET_BUSY', busy: true });
    try {
      const res = await runOp(sid, op, payload);
      if (seq !== loadSeq || state.sessionId !== sid) return null;
      if (res && res.idea) {
        dispatch({ type: 'SET_IDEA_VIEW', sessionId: sid, view: res.idea });
      }
      return res;
    } catch (err) {
      if (seq !== loadSeq || state.sessionId !== sid) return null;
      if (err?.status === 501) {
        dispatch({ type: 'SET_NOTICE', notice: err.message });
      } else {
        dispatch({ type: 'SET_ERROR', error: err?.message || `Operation ${op} failed` });
      }
      return null;
    } finally {
      if (seq === loadSeq && state.sessionId === sid) {
        dispatch({ type: 'SET_BUSY', busy: false });
      }
    }
  }

  async function loadSession(sessionId, hasModel) {
    const seq = ++loadSeq;
    if (hasModel !== undefined) state = ideaReducer(state, { type: 'SET_HAS_MODEL', hasModel });
    dispatch({ type: 'SET_BUSY', busy: true });
    try {
      const data = await fetchSession(sessionId);
      if (seq !== loadSeq) return null;
      dispatch({ type: 'SET_IDEA_VIEW', sessionId, view: data?.idea || null });
      return data;
    } catch (err) {
      if (seq !== loadSeq) return null;
      dispatch({ type: 'SET_ERROR', error: err?.message || 'Could not load Explore session' });
      return null;
    } finally {
      if (seq === loadSeq) dispatch({ type: 'SET_BUSY', busy: false });
    }
  }

  function setSessionView(sessionId, view, hasModel) {
    loadSeq += 1;
    if (hasModel !== undefined) state = ideaReducer(state, { type: 'SET_HAS_MODEL', hasModel });
    dispatch({ type: 'SET_IDEA_VIEW', sessionId, view });
  }

  function handleSseEvent(kindOrEnvelope, maybePayload) {
    const kind =
      kindOrEnvelope && typeof kindOrEnvelope === 'object'
        ? kindOrEnvelope.kind
        : kindOrEnvelope;
    const payload =
      kindOrEnvelope && typeof kindOrEnvelope === 'object'
        ? (kindOrEnvelope.payload ?? kindOrEnvelope.data ?? {})
        : (maybePayload || {});
    if (!kind) return;
    dispatch({ type: 'APPLY_SSE_EVENT', kind, payload });
  }

  function resetFilters() {
    dispatch({ type: 'RESET_FILTERS' });
  }

  return {
    loadSession,
    setSessionView,
    handleSseEvent,
    invokeOp,
    resetFilters,
    getState: () => state,
  };
}
