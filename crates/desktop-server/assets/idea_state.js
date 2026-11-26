// Pure Explore client state reducer, selectors, paste detection, and text-highlight tokenizer (X4).
// Zero DOM access, zero network calls, zero innerHTML.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any user-facing label constant.
import { formatModelCallBudget, normalizeRunState } from './run_bar.js';

// Named constants for owner-accepted decisions (`explore.md` 3 & 13, `00-overview.md` 3.3, 5.1, 8.2).
export const EXPLORE_VISIBILITY_STORAGE_KEY = 're-search-explore-enabled';
export const MAX_IDEA_CHARS = 4000;
export const MAX_USER_PREVIEW_QUERIES = 6;
export const NO_MODEL_NOTICE =
  'Search and library work without a model. Sharpening, reading and mapping need one.';
export const BADGE_NOT_READ = 'not read';
export const BADGE_IN_LIBRARY = 'in your library';
export const BADGE_OWNED_LIBRARY = 'owned';
export const STAGE_IDLE_LABEL = 'Idle';
export const EXPLORE_SUBMIT_LABEL = 'Explore';

export const EMPTY_STATE_MESSAGES = Object.freeze({
  offline_library_only: 'Offline: showing library matches only; nothing was searched',
  all_services_failed: 'All services failed: no result is shown',
  zero_candidates: 'Zero candidates: try fewer or broader terms',
});

export const IDEA_STAGE_ORDER = Object.freeze([
  'idea', 'sharpen', 'search', 'triage', 'read', 'map', 'directions',
]);

export const IDEA_STAGE_LABELS = Object.freeze({
  idea: 'Idea',
  sharpen: 'Sharpen',
  search: 'Search',
  triage: 'Triage',
  read: 'Read',
  map: 'Map',
  directions: 'Directions',
});

export const IDEA_PRESETS = Object.freeze([
  Object.freeze({
    id: 'quick', label: 'Quick', max_queries: 8, max_screened: 25,
    max_papers: 0, calls_per_paper: 0, typical_model_calls: 3, max_model_calls: 5,
  }),
  Object.freeze({
    id: 'standard', label: 'Standard', max_queries: 18, max_screened: 40,
    max_papers: 6, calls_per_paper: 3, typical_model_calls: 15, max_model_calls: 24,
  }),
  Object.freeze({
    id: 'thorough', label: 'Thorough', max_queries: 36, max_screened: 80,
    max_papers: 12, calls_per_paper: 4, typical_model_calls: 35, max_model_calls: 55,
  }),
]);

// Paste detection (`00-overview.md` 5.1 & `explore.md` 3): matches Rust `parse_paper_input` / `split_arxiv_id`.
const RE_ARXIV_MODERN = /^(\d{2})(\d{2})\.(\d{4,5})(?:v[1-9]\d*)?$/i;
const RE_ARXIV_OLD = /^[a-z-]+(?:\.[a-z-]+)?\/\d{7}(?:v[1-9]\d*)?$/i;
const RE_ARXIV_URL = /^https?:\/\/(?:export\.)?arxiv\.org\/(?:abs|pdf|e-print)\/([^\s?#]+?)(?:\.pdf)?(?:[?#].*)?$/i;
const RE_DOI = /^(?:doi:|https?:\/\/(?:dx\.)?doi\.org\/)?10\.\d{4,9}\/[^\s%]+$/i;
const RE_CANONICAL_KEY = /^(?:inspire:\d+|s2:[0-9a-f]{8,64}|openalex:W\d+|file:[0-9a-f]{16})$/i;

function isValidArxivId(core) {
  const m = core.match(RE_ARXIV_MODERN);
  if (m) {
    const yy = Number(m[1]);
    const mm = Number(m[2]);
    const yymm = yy * 100 + mm;
    if (mm < 1 || mm > 12) return false;
    if (yymm < 704) return false;
    if (m[3].length === 5 && yymm < 1501) return false;
    return true;
  }
  return RE_ARXIV_OLD.test(core);
}

export function isBarePaperIdentifier(rawText) {
  const trimmed = String(rawText || '').trim();
  if (!trimmed || /\s/.test(trimmed)) return false;
  if (RE_CANONICAL_KEY.test(trimmed)) return true;
  const urlMatch = trimmed.match(RE_ARXIV_URL);
  if (urlMatch) return isValidArxivId(urlMatch[1]);
  const strippedArxiv = trimmed.replace(/^arxiv:/i, '');
  if (isValidArxivId(strippedArxiv)) return true;
  return RE_DOI.test(trimmed);
}

function nonNegInt(val, fallback) {
  if (val === null || val === undefined || val === '') return fallback;
  const n = Number(val);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : fallback;
}

export function createInitialIdeaState(overrides = {}) {
  return {
    sessionId: null,
    view: null,
    hasModel: false,
    busy: false,
    error: null,
    notice: null,
    filters: { searchText: '', libraryOnly: false, ownedOnly: false, provider: 'all' },
    ...overrides,
  };
}

function applyStageEventToView(view, payload) {
  const stageId = String(payload.stage || '');
  const rawState = String(payload.state || '');
  const norm = normalizeRunState(rawState);
  const prevStages = Array.isArray(view.stages) ? view.stages : [];
  let found = false;
  const stages = prevStages.map((st) => {
    if (st.id !== stageId) return st;
    found = true;
    return {
      ...st,
      stored_state: rawState,
      run_state: norm.key,
      run_state_label: norm.label,
      action: norm.canContinue ? 'continue' : norm.canRetry ? 'retry' : null,
      reason: payload.reason ?? null,
    };
  });
  if (!found && stageId) {
    stages.push({
      id: stageId,
      label: IDEA_STAGE_LABELS[stageId] || stageId,
      stored_state: rawState,
      run_state: norm.key,
      run_state_label: norm.label,
      action: norm.canContinue ? 'continue' : norm.canRetry ? 'retry' : null,
      reason: payload.reason ?? null,
    });
  }
  const runBar = {
    ...(view.run_bar || {}),
    state: norm.key,
    label: norm.label,
    reason: payload.reason ?? '',
    action: norm.canContinue ? 'continue' : norm.canRetry ? 'retry' : null,
    stage: stageId,
  };
  return { ...view, stages, run_bar: runBar };
}

export function ideaReducer(state, action) {
  if (!action || typeof action !== 'object') return state;
  switch (action.type) {
    case 'SET_IDEA_VIEW':
      return {
        ...state,
        sessionId: action.sessionId ?? state.sessionId,
        view: action.view || null,
        error: null,
        notice: null,
      };
    case 'SET_HAS_MODEL':
      return { ...state, hasModel: Boolean(action.hasModel) };
    case 'SET_BUSY':
      return { ...state, busy: Boolean(action.busy) };
    case 'SET_ERROR':
      return { ...state, error: action.error ? String(action.error) : null };
    case 'SET_NOTICE':
      return { ...state, notice: action.notice ? String(action.notice) : null };
    case 'SET_FILTER':
      return { ...state, filters: { ...state.filters, ...(action.patch || {}) } };
    case 'RESET_FILTERS':
      return { ...state, filters: { searchText: '', libraryOnly: false, ownedOnly: false, provider: 'all' } };
    case 'APPLY_SSE_EVENT': {
      if (!state.view || !action.kind) return state;
      const payload = action.payload || {};
      if (action.kind === 'idea.candidates' && Array.isArray(payload.candidates)) {
        const annotated = payload.candidates.map((c) => ({
          read_status: 'not_read',
          read_badge: BADGE_NOT_READ,
          ...c,
        }));
        return { ...state, view: { ...state.view, candidates: annotated } };
      }
      if (action.kind === 'idea.stated' && payload.text) {
        const hasMap = Array.isArray(state.view.map?.nodes) && state.view.map.nodes.length > 0;
        return {
          ...state,
          view: { ...state.view, statement: payload, map_stale: hasMap || Boolean(state.view.map_stale) },
        };
      }
      if (action.kind === 'idea.stage' && payload.stage) {
        return { ...state, view: applyStageEventToView(state.view, payload) };
      }
      if (action.kind === 'idea.search_finished' && Array.isArray(payload.results)) {
        const wil = { ...(state.view.where_i_looked || {}), queries: payload.results };
        return { ...state, view: { ...state.view, where_i_looked: wil } };
      }
      return state;
    }
    default:
      return state;
  }
}

export function formatPresetSummary(preset) {
  const p = preset || IDEA_PRESETS[1];
  const papersText = p.max_papers === 0 ? 'abstracts only' : `up to ${p.max_papers} papers`;
  return `${p.label} — ${p.max_queries} queries, ${p.max_screened} screened, ${papersText}, at most ${p.max_model_calls} model calls`;
}

export function formatCostLine(budgetLike = {}) {
  const presetLabel = String(budgetLike.preset_label || budgetLike.presetLabel || 'Standard');
  const qUsed = nonNegInt(budgetLike.queries_used ?? budgetLike.queriesUsed, 0);
  const qMax = nonNegInt(budgetLike.queries_max ?? budgetLike.max_queries, 18);
  const pUsed = nonNegInt(budgetLike.papers_read ?? budgetLike.papers_fetched, 0);
  const pMax = nonNegInt(budgetLike.papers_max ?? budgetLike.max_papers, 6);
  const cUsed = nonNegInt(
    budgetLike.modelCallsUsed ?? budgetLike.model_calls_used ?? budgetLike.model_calls?.used,
    0,
  );
  const cMax = nonNegInt(
    budgetLike.modelCallsMax ?? budgetLike.model_calls_max ?? budgetLike.model_calls?.cap,
    24,
  );
  return `${presetLabel} - ${qUsed} of ${qMax} queries - ${pUsed} of ${pMax} papers - ${formatModelCallBudget(cUsed, cMax)}`;
}

/** Build the ordered 7-stage rail descriptor mapped through F9 run states. */
export function selectStageRail(state) {
  const byId = new Map();
  const rawStages = Array.isArray(state?.view?.stages) ? state.view.stages : [];
  for (const s of rawStages) {
    if (s && s.id) byId.set(String(s.id), s);
  }
  return IDEA_STAGE_ORDER.map((id) => {
    const rec = byId.get(id);
    const stored = rec?.stored_state || 'idle';
    if (stored === 'idle' || !rec?.run_state) {
      return {
        id,
        label: IDEA_STAGE_LABELS[id] || id,
        storedState: 'idle',
        runStateKey: null,
        runStateLabel: STAGE_IDLE_LABEL,
        icon: 'info',
        tone: 'muted',
        action: null,
        reason: null,
      };
    }
    const norm = normalizeRunState(rec.run_state || stored);
    return {
      id,
      label: rec.label || IDEA_STAGE_LABELS[id] || id,
      storedState: stored,
      runStateKey: norm.key,
      runStateLabel: norm.label,
      icon: norm.icon,
      tone: norm.tone,
      action: rec.action || (norm.canContinue ? 'continue' : norm.canRetry ? 'retry' : null),
      reason: rec.reason || null,
    };
  });
}

/** Build the F6 `run_bar.js`-compatible state object for an Explore session. */
export function selectRunBarModel(state) {
  const rb = state?.view?.run_bar || {};
  const budget = state?.view?.budget || {};
  const presetLabel = String(rb.preset_label || budget.preset_label || 'Standard');
  const qUsed = nonNegInt(rb.queries_used ?? budget.queries_used, 0);
  const qMax = nonNegInt(rb.queries_max ?? budget.max_queries, 18);
  const pUsed = nonNegInt(rb.papers_read ?? budget.papers_fetched, 0);
  const pMax = nonNegInt(rb.papers_max ?? budget.max_papers, 6);
  const modelCallsUsed = nonNegInt(rb.modelCallsUsed ?? rb.model_calls_used ?? budget.model_calls?.used, 0);
  const modelCallsMax = nonNegInt(rb.modelCallsMax ?? rb.model_calls_max ?? budget.model_calls?.cap, 24);
  const openItems = Array.isArray(rb.openItems)
    ? rb.openItems
    : Array.isArray(rb.open_items) ? rb.open_items : [];
  return {
    state: rb.state || 'done',
    reason: rb.reason || '',
    extras: Array.isArray(rb.extras)
      ? rb.extras
      : [presetLabel, `${qUsed} of ${qMax} queries`, `${pUsed} of ${pMax} papers`],
    modelCallsUsed,
    modelCallsMax,
    openItems,
    busy: Boolean(state?.busy),
    costLine: formatCostLine({
      preset_label: presetLabel,
      queries_used: qUsed,
      queries_max: qMax,
      papers_read: pUsed,
      papers_max: pMax,
      modelCallsUsed,
      modelCallsMax,
    }),
  };
}

/** Filter candidates deterministically by search text, library/owned toggles, and provider. */
export function selectFilteredCandidates(state) {
  const list = Array.isArray(state?.view?.candidates) ? state.view.candidates : [];
  const f = state?.filters || {};
  const q = String(f.searchText || '').trim().toLowerCase();
  const qidToProvider = new Map();
  for (const row of Array.isArray(state?.view?.where_i_looked?.queries) ? state.view.where_i_looked.queries : []) {
    if (row?.qid && row?.provider) qidToProvider.set(String(row.qid), String(row.provider).toLowerCase());
  }
  return list.filter((c) => {
    if (!c || typeof c !== 'object') return false;
    if (f.libraryOnly && !c.in_library) return false;
    if (f.ownedOnly && !c.owned) return false;
    if (f.provider && f.provider !== 'all') {
      const target = String(f.provider).toLowerCase();
      const absProv = String(c.abstract_ref?.provider || '').toLowerCase();
      const foundProvs = Array.isArray(c.found_by)
        ? c.found_by.map((qid) => qidToProvider.get(String(qid)) || String(qid).toLowerCase())
        : [];
      if (absProv !== target && !foundProvs.includes(target)) return false;
    }
    if (q) {
      const hay = [
        c.title || '', c.key || '',
        Array.isArray(c.authors) ? c.authors.join(' ') : '',
        c.abstract_text || c.abstract || '',
      ].join(' ').toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}

/** Return the active empty/failure banner model (`explore.md` 3) or `null`. */
export function selectEmptyState(state) {
  const es = state?.view?.empty_state;
  if (es && typeof es === 'object' && es.message) {
    return {
      code: String(es.code || 'info'),
      stage: es.stage ? String(es.stage) : null,
      message: String(es.message),
      canRetry: es.code === 'all_services_failed' || es.code === 'stage_failed' || es.code === 'interrupted',
    };
  }
  if (state?.view?.offline) {
    return {
      code: 'offline_library_only',
      stage: null,
      message: EMPTY_STATE_MESSAGES.offline_library_only,
      canRetry: false,
    };
  }
  return null;
}

/**
 * Pure case-insensitive highlighter splitting `text` into `{ text, match }` segments.
 * Safe by construction: callers render segments via `textContent` / `<mark>`, never `innerHTML`.
 */
export function highlightTermsSegments(text, terms) {
  const source = String(text ?? '');
  if (!source) return [];
  const cleanTerms = Array.from(
    new Set(
      (Array.isArray(terms) ? terms : [])
        .map((t) => String(typeof t === 'string' ? t : t?.text || '').trim())
        .filter((t) => t.length >= 2),
    ),
  )
    .sort((a, b) => b.length - a.length)
    .slice(0, 32);
  if (cleanTerms.length === 0) return [{ text: source, match: false }];

  const escaped = cleanTerms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const re = new RegExp(`(${escaped.join('|')})`, 'gi');
  const out = [];
  let cursor = 0;
  let m;
  while ((m = re.exec(source)) !== null) {
    if (m.index > cursor) out.push({ text: source.slice(cursor, m.index), match: false });
    out.push({ text: m[0], match: true });
    cursor = m.index + m[0].length;
    if (m[0].length === 0) break;
  }
  if (cursor < source.length) out.push({ text: source.slice(cursor), match: false });
  return out;
}
