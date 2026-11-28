// Explore session page shell (package X4).
// Renders verbatim Idea card, 7-stage rail mapped through F9 run states, shared F6 run_bar.js
// cost line, no-model notice, empty/failure states, candidate list, and "Where I looked" skeleton.
// Zero innerHTML writes. Rule S8: zero "verified", "new", "novel", or "proved" strings in DOM.
import { decideIdeaApproval, getExploreSession, postIdeaOp } from './api_idea.js';
import { executeExploreHandoff, fetchExploreHandoffsSafe, openChildSessionDefault, renderIdeaDirectionsSection } from './idea_directions.js';
import { openExplorePageImage, openExploreSource, renderIdeaMapSection } from './idea_map.js';
import {
  renderCandidateCards,
  renderCandidateList,
  renderSearchPlanApproval,
  renderWhereILooked,
} from './idea_search.js';
import {
  NO_MODEL_NOTICE,
  SHARPENED_IDEA_TITLE,
  createInitialIdeaState,
  highlightSubclaimQuotes,
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

  // 5. Verbatim Idea card with highlighted subclaim `idea_quote` spans (`explore.md` 3 & 4.1).
  const stmt = view.statement;
  const sharpened = view.sharpened_idea || view.brief || null;
  const ideaCard = el('section', 'idea-verbatim-card');
  ideaCard.setAttribute('data-idea-section', 'idea-card');
  const ideaHead = el('div', 'idea-card-head');
  ideaHead.append(
    el('h3', 'idea-card-title', `Idea (rev ${Number(stmt?.rev || 1)})`),
    createBadge({ text: 'you', icon: 'user', tone: 'neutral' }),
  );
  ideaCard.append(ideaHead);
  const ideaTextEl = el('p', 'idea-verbatim-text');
  const rawIdeaText = stmt?.text || 'No idea statement recorded.';
  const quoteSegments = highlightSubclaimQuotes(rawIdeaText, sharpened?.subclaims);
  for (const seg of quoteSegments) {
    if (seg.subclaimId) {
      const mark = document.createElement('mark');
      mark.className = 'idea-quote-mark';
      mark.setAttribute('data-subclaim-id', seg.subclaimId);
      mark.title = seg.subclaimId;
      mark.textContent = seg.text;
      ideaTextEl.appendChild(mark);
    } else {
      const span = document.createElement('span');
      span.textContent = seg.text;
      ideaTextEl.appendChild(span);
    }
  }
  ideaCard.append(ideaTextEl);
  if (view.map_stale) {
    ideaCard.append(createBadge({ text: 'Map stale after revision', icon: 'warn', tone: 'warn' }));
  }
  shell.append(ideaCard);

  // 5b. "Sharpened idea" card (NEVER the word "brief" on screen — `00-overview.md` 3.2 row 18).
  if (sharpened) {
    const sharpenedCard = renderSharpenedIdeaCard(sharpened, {
      answers: view.question_answers,
      onSave: callbacks.onSaveAnswer,
      onPlan: callbacks.onPlan,
    });
    if (sharpenedCard) shell.append(sharpenedCard);
  }

  // 5c. Search plan approval card via shared read-only `approval_card.js` (package X7).
  const liveApprovals = Array.isArray(state?.pendingApprovals) ? state.pendingApprovals : [];
  const effectiveApproval = buildEffectivePlanApproval(view.plan_approval, liveApprovals);
  if (effectiveApproval) {
    const planCard = renderSearchPlanApproval(effectiveApproval, {
      onAllowGrant: callbacks.onAllowGrant,
      onDenyGrant: callbacks.onDenyGrant,
    });
    if (planCard) shell.append(planCard);
  }

  // 6. Candidate list (`idea_search.js`: flat before Triage, grouped with reading list after Triage).
  const candidates = selectFilteredCandidates(state);
  const terms = extractHighlightTerms(view);
  const onSaveReadingList = callbacks.onSaveReadingList
    || (callbacks.onSaveAnswer ? (reading_list) => callbacks.onSaveAnswer({ reading_list }) : null);
  const onTogglePauseBeforeReading = callbacks.onTogglePauseBeforeReading
    || (callbacks.onSaveAnswer ? (pause_before_reading) => callbacks.onSaveAnswer({ pause_before_reading }) : null);
  shell.append(
    renderCandidateList(candidates, {
      highlightTerms: terms, filters: state?.filters, onFilterChange: callbacks.onFilterChange,
      triage: view.triage, papers: view.papers, maxPapers: view.budget?.max_papers,
      pauseBeforeReading: Boolean(view.pause_before_reading),
      onSaveReadingList, onTogglePauseBeforeReading, onAttachPdf: callbacks.onAttachPdf,
    }),
  );

  // 7. Live "Where I looked" panel (`idea_search.js`).
  shell.append(renderWhereILooked(view.where_i_looked, { onRetryService: callbacks.onRetryService }));

  // 8. Map & verdict UI via F6 GraphView, quote_chip.js, and ui_badge.js (package X12).
  shell.append(renderIdeaMapSection(view, callbacks));

  // 9. Directions cards & F7 hand-over buttons (package X13).
  shell.append(renderIdeaDirectionsSection(view, callbacks));

  hostEl.replaceChildren(shell);
  return shell;
}

function buildEffectivePlanApproval(planApproval, pendingApprovals) {
  const live = pendingApprovals.filter(
    (p) => p && p.call_id && (p.tool_id === 'idea_search' || String(p.caller || '').startsWith('host:idea:')),
  );
  if (live.length === 0) return planApproval || null;
  const base = planApproval && typeof planApproval === 'object' ? planApproval : { round: 1, queries: [], services: [] };
  const grants = live.map((p) => ({
    id: String(p.call_id),
    kind: p.allow_session_approve ? 'session_capability' : 'worker_session',
    label: `Allow ${p.tool_id || 'idea_search'} (${p.effect || 'network-read'})`,
    caller: p.caller || null,
    tool_id: p.tool_id || 'idea_search',
    effect: p.effect || 'network-read',
    call_id: String(p.call_id),
    digest: String(p.digest || ''),
    granted: false,
    status: 'pending',
  }));
  return { ...base, grants };
}

/** Stateful Explore session controller coordinating REST ops, focus-safe filters, and SSE updates. */
export function createExploreSessionController(hostEl, deps = {}) {
  const fetchSession = deps.getExploreSession || getExploreSession;
  const runOp = deps.postIdeaOp || postIdeaOp;
  const sendApproval = deps.decideIdeaApproval || decideIdeaApproval;
  let state = createInitialIdeaState({ hasModel: Boolean(deps.hasModel) });
  let loadSeq = 0;
  let selectedMapNodeId = null;
  let handoffsView = null;

  function patchCandidateCardsOnly() {
    const countEl = hostEl?.querySelector?.('[data-idea-candidate-count="true"]');
    const oldCards = hostEl?.querySelector?.('[data-idea-candidate-cards="true"]');
    const section = hostEl?.querySelector?.('[data-idea-section="candidates"]');
    if (!section || !oldCards) return false;
    const filtered = selectFilteredCandidates(state);
    if (countEl) countEl.textContent = `Candidates (${filtered.length})`;
    const freshCards = renderCandidateCards(filtered, extractHighlightTerms(state.view), { triage: state.view?.triage, papers: state.view?.papers, maxPapers: state.view?.budget?.max_papers, onSaveReadingList: (reading_list) => void invokeOp('answer', { reading_list }), onAttachPdf: deps.onAttachPdf });
    const idx = Array.isArray(section.children) ? section.children.indexOf(oldCards) : -1;
    if (typeof oldCards.replaceWith === 'function') { oldCards.replaceWith(freshCards); return true; }
    if (idx >= 0 && Array.isArray(section.children)) { section.children.splice(idx, 1, freshCards); return true; }
    return false;
  }

  function dispatch(action, { candidatesOnly = false } = {}) {
    if (action?.type === 'SET_IDEA_VIEW' && action.sessionId && action.sessionId !== state.sessionId) {
      selectedMapNodeId = null;
      handoffsView = null;
    }
    state = ideaReducer(state, action);
    if (candidatesOnly && patchCandidateCardsOnly()) return;
    render();
  }

  function runHandoff(payload) {
    const sid = state.sessionId;
    if (!sid) return Promise.resolve(null);
    return executeExploreHandoff({
      sessionId: sid,
      payload,
      invokeOp,
      getExploreHandoffsFn: deps.getExploreHandoffs,
      onHandoffsUpdated: (next) => { if (state.sessionId === sid) { handoffsView = next; render(); } },
      onOpenChildSession: deps.onOpenChildSession || openChildSessionDefault,
    });
  }

  function render() {
    renderExploreSession(hostEl, state, {
      onFilterChange: (patch) => dispatch({ type: 'SET_FILTER', patch }, { candidatesOnly: true }),
      onStop: () => void invokeOp('stop'),
      onContinue: () => void handleContinueClick(),
      onRetry: () => void handleRetryClick(),
      onSaveAnswer: (payload) => void invokeOp('answer', payload),
      onPlan: () => void invokeOp('plan', {}),
      onRetryService: (provider) => void invokeOp('search', { provider }),
      onNote: (payload) => void invokeOp('note', payload),
      onRemap: (payload) => void invokeOp('map', payload || {}),
      initialSelectedNodeId: selectedMapNodeId,
      onSelectNode: (node) => { selectedMapNodeId = node?.id || null; deps.onSelectNode?.(node); },
      onOpenSource: deps.onOpenSource || ((target) => void openExploreSource(state.sessionId, target)),
      onOpenPageImage: deps.onOpenPageImage || ((target) => void openExplorePageImage(hostEl, state.sessionId, target)),
      handoffs: handoffsView,
      onOpenChildSession: deps.onOpenChildSession || openChildSessionDefault,
      onOpenInSolve: deps.onOpenInSolve || (({ direction_id }) => void runHandoff({ action: 'open_in_solve', direction_id })),
      onInvestigatePaper: deps.onInvestigatePaper || (({ paper_key, node_id }) => void runHandoff({ action: 'investigate_paper', paper_key, node_id })),
      onAllowGrant: async (grant) => {
        if (grant?.call_id && state.sessionId) {
          const res = await sendApproval(state.sessionId, grant.call_id, {
            approved: true, digest: grant.digest || '',
            approve_for_session: grant.kind === 'session_capability',
          });
          return { approved: Boolean(res?.ok !== false && res?.approved !== false) };
        }
        void invokeOp('search', {});
        return { approved: false };
      },
      onDenyGrant: async (grant) => {
        if (grant?.call_id && state.sessionId) {
          await sendApproval(state.sessionId, grant.call_id, { approved: false, digest: grant.digest || '' });
          return;
        }
        await invokeOp('stop', {});
      },
    });
  }

  async function handleContinueClick() {
    const rb = selectRunBarModel(state);
    const atCap = rb.state === 'paused' && (state.view?.budget?.state === 'stopped_at_cap'
      || (state.view?.stages || []).some((s) => s.stored_state === 'stopped_at_cap'));
    return invokeOp('continue', atCap ? { extra_queries: 6, extra_papers: 2, extra_model_calls: 6 } : {});
  }

  async function handleRetryClick() { return invokeOp('continue', {}); }

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
      if (res && res.idea) dispatch({ type: 'SET_IDEA_VIEW', sessionId: sid, view: res.idea });
      return res;
    } catch (err) {
      if (seq !== loadSeq || state.sessionId !== sid) return null;
      if (err?.status === 501) dispatch({ type: 'SET_NOTICE', notice: err.message });
      else dispatch({ type: 'SET_ERROR', error: err?.message || `Operation ${op} failed` });
      return null;
    } finally {
      if (seq === loadSeq && state.sessionId === sid) dispatch({ type: 'SET_BUSY', busy: false });
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
      void fetchExploreHandoffsSafe(sessionId, deps.getExploreHandoffs).then((next) => {
        if (next && seq === loadSeq && state.sessionId === sessionId) { handoffsView = next; render(); }
      });
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
    const kind = kindOrEnvelope && typeof kindOrEnvelope === 'object' ? kindOrEnvelope.kind : kindOrEnvelope;
    const payload = kindOrEnvelope && typeof kindOrEnvelope === 'object'
      ? (kindOrEnvelope.payload ?? kindOrEnvelope.data ?? {})
      : (maybePayload || {});
    if (!kind) return;
    dispatch({ type: 'APPLY_SSE_EVENT', kind, payload });
  }

  function resetFilters() { dispatch({ type: 'RESET_FILTERS' }); }

  return { loadSession, setSessionView, handleSseEvent, invokeOp, resetFilters, getState: () => state };
}

/**
 * Render the `"Sharpened idea"` card (`00-overview.md` 3.2 row 18 — never show `"brief"` on
 * screen): restatement, assumptions, checkable parts, terms/aliases, editable perspective queries,
 * and clarifying questions with defaults. Zero `innerHTML`.
 */
export function renderSharpenedIdeaCard(sharpened, opts = {}) {
  if (!sharpened || typeof sharpened !== 'object') return null;
  const answersMap = { ...(sharpened.answers || {}), ...(opts.answers || {}) };
  const onSave = typeof opts.onSave === 'function' ? opts.onSave : null;
  const onPlan = typeof opts.onPlan === 'function' ? opts.onPlan : null;

  const section = el('section', 'idea-setup-card');
  section.setAttribute('data-idea-section', 'sharpened-idea');
  section.setAttribute('aria-label', SHARPENED_IDEA_TITLE);

  const head = el('div', 'idea-card-head');
  head.append(
    el('h3', 'idea-card-title', SHARPENED_IDEA_TITLE),
    createBadge({ text: '(model)', icon: 'info', tone: 'info', border: 'dashed' }),
  );
  section.append(head);

  const restateInput = el('textarea', 'idea-textarea');
  restateInput.rows = 2;
  restateInput.value = String(sharpened.restatement || '');
  restateInput.setAttribute('aria-label', 'Restatement');
  restateInput.setAttribute('data-sharpen-field', 'restatement');
  section.append(el('span', 'idea-field-label', 'Restatement (model)'), restateInput);

  const assumptions = Array.isArray(sharpened.assumptions) ? sharpened.assumptions : [];
  if (assumptions.length > 0) {
    const ul = el('ul', 'idea-not-searched');
    for (const a of assumptions) ul.append(el('li', '', String(a)));
    section.append(el('span', 'idea-field-label', 'Assumptions'), ul);
  }

  const subclaims = Array.isArray(sharpened.subclaims) ? sharpened.subclaims : [];
  if (subclaims.length > 0) {
    const list = el('div', 'idea-query-list');
    list.setAttribute('data-sharpen-section', 'subclaims');
    for (const sc of subclaims) {
      const row = el('div', 'idea-query-chip');
      row.setAttribute('data-subclaim-id', String(sc.id || ''));
      row.append(
        createBadge({ text: String(sc.id || 'sc'), icon: 'quote', tone: 'info' }),
        el('span', '', String(sc.text || '')),
        el('span', 'idea-card-meta', sc.idea_quote ? `"${sc.idea_quote}"` : ''),
      );
      list.append(row);
    }
    section.append(el('span', 'idea-field-label', 'Checkable parts'), list);
  }

  const terms = Array.isArray(sharpened.terms) ? sharpened.terms : [];
  if (terms.length > 0) {
    const row = el('div', 'idea-chip-row');
    for (const tg of terms) {
      const aliases = Array.isArray(tg.aliases) && tg.aliases.length > 0
        ? ` (${tg.aliases.join(', ')})`
        : '';
      row.append(el('span', 'idea-chip', `${tg.term || ''}${aliases}`));
    }
    section.append(el('span', 'idea-field-label', 'Terms and aliases'), row);
  }

  const perspectives = Array.isArray(sharpened.perspectives) ? sharpened.perspectives : [];
  const editedPerspectives = perspectives.map((p) => ({
    id: String(p.id || ''),
    kind: String(p.kind || 'exact'),
    question: String(p.question || ''),
    queries: (Array.isArray(p.queries) ? p.queries : []).map((q) => ({
      provider: String(q.provider || 'arxiv'),
      q: String(q.q || ''),
    })),
  }));
  if (editedPerspectives.length > 0) {
    const pWrap = el('div', 'idea-query-list');
    pWrap.setAttribute('data-sharpen-section', 'perspectives');
    for (const p of editedPerspectives) {
      const card = el('div', 'idea-preview-box');
      card.setAttribute('data-perspective-id', p.id);
      card.append(el('strong', '', `${p.id} · ${p.kind}: ${p.question}`));
      for (const q of p.queries) {
        const qRow = el('div', 'idea-query-chip');
        const qInput = el('input', 'text-input');
        qInput.type = 'text';
        qInput.value = q.q;
        qInput.setAttribute('aria-label', `${p.id} ${q.provider} query`);
        qInput.addEventListener('input', () => { q.q = qInput.value; });
        qRow.append(el('span', 'idea-query-provider', q.provider), qInput);
        card.append(qRow);
      }
      pWrap.append(card);
    }
    section.append(el('span', 'idea-field-label', 'Search perspectives and editable queries'), pWrap);
  }

  const questions = Array.isArray(sharpened.questions) ? sharpened.questions : [];
  const currentAnswers = { ...answersMap };
  if (questions.length > 0) {
    const qWrap = el('div', 'idea-query-list');
    qWrap.setAttribute('data-sharpen-section', 'questions');
    for (const q of questions) {
      const qid = String(q.id || '');
      const selected = currentAnswers[qid] ?? String(q.default || '');
      currentAnswers[qid] = selected;
      const qRow = el('div', 'idea-query-chip');
      const sel = el('select', 'text-input');
      sel.setAttribute('aria-label', String(q.question || qid));
      sel.setAttribute('data-question-id', qid);
      for (const opt of Array.isArray(q.options) ? q.options : []) {
        const o = el('option', '', String(opt));
        o.value = String(opt);
        if (String(opt) === selected) o.selected = true;
        sel.append(o);
      }
      sel.value = selected;
      sel.addEventListener('change', () => { currentAnswers[qid] = sel.value; });
      qRow.append(el('span', '', String(q.question || '')), sel);
      qWrap.append(qRow);
    }
    section.append(el('span', 'idea-field-label', 'Clarifying questions (defaults applied)'), qWrap);
  }

  if (onSave || onPlan) {
    const actions = el('div', 'idea-controls-row');
    if (onSave) {
      const saveBtn = el('button', 'pill-btn sm', 'Save edits');
      saveBtn.type = 'button';
      saveBtn.setAttribute('data-action', 'save-sharpen-edits');
      saveBtn.addEventListener('click', () => {
        const payload = {};
        const trimmedRestate = String(restateInput.value || '').trim();
        if (trimmedRestate) payload.restatement = trimmedRestate;
        if (editedPerspectives.length > 0) payload.perspectives = editedPerspectives;
        if (Object.keys(currentAnswers).length > 0) payload.answers = currentAnswers;
        onSave(payload);
      });
      actions.append(saveBtn);
    }
    if (onPlan) {
      const planBtn = el('button', 'pill-btn sm primary', 'Prepare search plan');
      planBtn.type = 'button';
      planBtn.setAttribute('data-action', 'prepare-plan');
      planBtn.addEventListener('click', () => onPlan());
      actions.append(planBtn);
    }
    section.append(actions);
  }

  return section;
}

