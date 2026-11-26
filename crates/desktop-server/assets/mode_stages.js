// Mode-specific center-stage mounter for Solve (`solve_setup.js`, S4/S5) and Explore (`idea.js`, X3/X4).
// Keeps `app.js` untouched and keeps left-rail Solve/Explore buttons disabled ("not built yet", F5)
// until S12 and X7-X12 land. Zero `innerHTML` writes; guarded by a monotonic load sequence token.
import * as api from './api.js';
import { createExploreSessionController } from './idea.js';
import { EXPLORE_VISIBILITY_STORAGE_KEY } from './idea_state.js';
import { createSolveSetupView } from './solve_setup.js';

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

async function defaultFetchJson(apiMod, method, path, body) {
  const headers = {};
  const token = resolveAuthToken(apiMod);
  if (token) headers['X-ReSearch-Token'] = token;
  const init = { method, credentials: 'same-origin', headers };
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const resp = await fetch(path, init);
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

function ensureIdeaStylesheet() {
  if (typeof document === 'undefined' || !document.head || typeof document.querySelector !== 'function') return;
  if (document.querySelector('link[href="/assets/idea.css"]')) return;
  const link = Object.assign(document.createElement('link'), { rel: 'stylesheet', href: '/assets/idea.css' });
  document.head.appendChild(link);
}

function normalizeMode(raw) {
  const norm = String(raw || '').trim().toLowerCase();
  if (norm === 'solve' || norm === 'explore') return norm;
  return (norm === 'investigation' || norm === 'investigate') ? 'investigate' : 'research';
}

function detectModelConnected(deps) {
  if (typeof deps.isModelConnected === 'function') return Boolean(deps.isModelConnected());
  const sel = typeof document !== 'undefined' && typeof document.getElementById === 'function' ? document.getElementById('composer-model') : null;
  return Boolean(sel?.options && Array.from(sel.options).some((opt) => opt && !opt.disabled && opt.value && !String(opt.value).startsWith('__')));
}

export function createModeStagesController(deps = {}) {
  const apiMod = deps.api || api;
  const customFetch = typeof deps.fetchJson === 'function';
  const fetchJson = customFetch
    ? deps.fetchJson
    : (method, path, body) => defaultFetchJson(apiMod, method, path, body);
  const modeBySession = new Map();
  let syncSeq = 0;
  let mountedSessionId = null;
  let mountedMode = null;
  let activeSolveView = null;
  let activeExploreCtrl = null;

  const getEl = (id) => (typeof deps.getElementById === 'function'
    ? deps.getElementById(id)
    : (typeof document !== 'undefined' && typeof document.getElementById === 'function' ? document.getElementById(id) : null));

  function setShellStageAttr(stage) {
    const shell = getEl('app-shell');
    if (!shell) return;
    if (stage) shell.setAttribute('data-mode-stage', stage);
    else if (typeof shell.removeAttribute === 'function') shell.removeAttribute('data-mode-stage');
  }

  function clearStage(el) {
    if (!el) return;
    el.hidden = true;
    if (typeof el.replaceChildren === 'function') el.replaceChildren();
  }

  function maybeRestoreHomeExplore(activeId) {
    const homeCard = getEl('home-explore-card');
    if (!homeCard) return;
    if (activeId) { homeCard.hidden = true; return; }
    let enabled = false;
    try { enabled = typeof localStorage !== 'undefined' && localStorage?.getItem(EXPLORE_VISIBILITY_STORAGE_KEY) === '1'; } catch { enabled = false; }
    homeCard.hidden = !(enabled && (homeCard.children?.length || 0) > 0);
  }

  async function resolveGrant(sessionId, grant, approved) {
    const encSid = encodeURIComponent(sessionId);
    let callId = grant?.call_id || '';
    let digest = grant?.digest || '';
    if (!callId || !digest) {
      const pendingResp = (!customFetch && typeof apiMod.getPendingApprovals === 'function')
        ? await apiMod.getPendingApprovals(sessionId)
        : await fetchJson('GET', `/api/sessions/${encSid}/approvals`);
      const list = Array.isArray(pendingResp?.pending) ? pendingResp.pending : [];
      const match = list.find((p) => (
        (grant?.call_id && p.call_id === grant.call_id)
        || (grant?.caller && p.caller === grant.caller && (!grant.tool_id || p.tool === grant.tool_id))
        || (grant?.effect && p.effect === grant.effect)
      ));
      if (match) { callId = match.call_id; digest = match.digest; }
    }
    if (!callId || !digest) {
      return { approved: false, pending: true, reason: 'Approvals are prompted when the first turn starts.' };
    }
    const payload = { digest, approved: Boolean(approved), approve_for_session: Boolean(approved && grant?.kind === 'session_capability') };
    return (!customFetch && typeof apiMod.respondApproval === 'function')
      ? apiMod.respondApproval(sessionId, callId, payload)
      : fetchJson('POST', `/api/sessions/${encSid}/approvals/${encodeURIComponent(callId)}`, payload);
  }

  function resetStages(nextSessionId = null, nextMode = null) {
    mountedSessionId = nextSessionId;
    mountedMode = nextMode;
    activeSolveView = null;
    activeExploreCtrl = null;
    setShellStageAttr(null);
    clearStage(getEl('solve-session-stage'));
    clearStage(getEl('explore-session-stage'));
    maybeRestoreHomeExplore(nextSessionId);
  }

  async function mountSolveStage(sessionId, seq) {
    const solveHost = getEl('solve-session-stage');
    if (!solveHost) return null;
    clearStage(getEl('explore-session-stage'));
    const homeCard = getEl('home-explore-card');
    if (homeCard) homeCard.hidden = true;
    activeExploreCtrl = null;
    setShellStageAttr('solve');
    solveHost.hidden = false;

    const encSid = encodeURIComponent(sessionId);
    const viewData = await fetchJson('GET', `/api/sessions/${encSid}/solve/view`);
    if (seq !== syncSeq) return null;
    const view = createSolveSetupView(solveHost, {
      initial: { modelConnected: detectModelConnected(deps) },
      onPrefill: (body) => (seq === syncSeq ? fetchJson('POST', `/api/sessions/${encSid}/solve/prefill`, body) : Promise.resolve(null)),
      onSubmitSetup: (body) => fetchJson('POST', `/api/sessions/${encSid}/solve/setup`, body),
      onAllowGrant: (grant) => resolveGrant(sessionId, grant, true),
      onDenyGrant: (grant) => resolveGrant(sessionId, grant, false),
      ...(deps.solveOpts || {}),
    });
    if (viewData) view.applyView(viewData);
    activeSolveView = view;
    mountedSessionId = sessionId;
    mountedMode = 'solve';
    return view;
  }

  async function mountExploreStage(sessionId, seq, preloadedIdea = null) {
    const exploreHost = getEl('explore-session-stage');
    if (!exploreHost) return null;
    ensureIdeaStylesheet();
    clearStage(getEl('solve-session-stage'));
    const homeCard = getEl('home-explore-card');
    if (homeCard) homeCard.hidden = true;
    activeSolveView = null;
    setShellStageAttr('explore');
    exploreHost.hidden = false;

    const ctrl = createExploreSessionController(exploreHost, { hasModel: detectModelConnected(deps), ...(deps.exploreDeps || {}) });
    activeExploreCtrl = ctrl;
    mountedSessionId = sessionId;
    mountedMode = 'explore';
    if (preloadedIdea) {
      ctrl.setSessionView(sessionId, preloadedIdea, detectModelConnected(deps));
      return ctrl;
    }
    await ctrl.loadSession(sessionId, detectModelConnected(deps));
    return seq === syncSeq ? ctrl : null;
  }

  async function syncFromRail(activeSessionId, sessions = []) {
    if (Array.isArray(sessions)) {
      for (const s of sessions) if (s?.id && s?.mode) modeBySession.set(s.id, normalizeMode(s.mode));
    }
    if (!activeSessionId) { syncSeq += 1; resetStages(null, null); return null; }
    const seq = ++syncSeq;
    let mode = modeBySession.get(activeSessionId) || null;
    if (!mode) {
      try {
        const sess = (!customFetch && typeof apiMod.getSession === 'function')
          ? await apiMod.getSession(activeSessionId)
          : await fetchJson('GET', `/api/sessions/${encodeURIComponent(activeSessionId)}`);
        if (seq !== syncSeq) return null;
        mode = normalizeMode(sess?.mode);
        modeBySession.set(activeSessionId, mode);
      } catch {
        if (seq !== syncSeq) return null;
        mode = 'research';
      }
    }
    if (mode === mountedMode && activeSessionId === mountedSessionId && (mode === 'solve' || mode === 'explore')) {
      return mode === 'solve' ? activeSolveView : activeExploreCtrl;
    }
    if (mode === 'solve') { try { return await mountSolveStage(activeSessionId, seq); } catch { return null; } }
    if (mode === 'explore') { try { return await mountExploreStage(activeSessionId, seq); } catch { return null; } }
    resetStages(activeSessionId, mode);
    return null;
  }

  async function openExploreCreated(res) {
    const id = res?.id || res?.session?.id || res?.session_id;
    if (!id) return null;
    modeBySession.set(id, 'explore');
    const titleEl = getEl('active-session-title');
    if (titleEl && (res?.title || res?.session?.title)) titleEl.textContent = String(res.title || res.session.title);
    return mountExploreStage(id, ++syncSeq, res?.idea || null);
  }

  return {
    syncFromRail, openExploreCreated,
    getActiveMode: () => mountedMode, getActiveSessionId: () => mountedSessionId,
    getSolveView: () => activeSolveView, getExploreController: () => activeExploreCtrl,
  };
}

if (typeof globalThis !== 'undefined') {
  globalThis.__reSearchModeStages = createModeStagesController();
}
