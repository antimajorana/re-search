// Main entry controller: wires left rail (Active/Archived + Delete), Connect panel, model picker,
// live pipeline bar, center stream, SSE live updates, resizable Canvas tabs, and postMessage bridge.
import * as api from '/assets/api.js';
import { renderMathTextInto } from '/assets/math.js';
import { attachAnswerTrace } from '/assets/claim_trace.js';
import {
  appendAssistantMessage, appendUserMessage, appendWarningLine, confirmOptimisticUserMessage,
  finalizeLiveAssistant, finalizeLiveOnError, finalizeLiveThinking, handleRoundReset,
  handleTextDelta, handleThinkingDelta, handleToolTrace, initStream, refreshEmptyHeroIfVisible,
  removeOptimisticUserMessage, renderApprovalCard, renderSessionStream, resetLiveTurnState,
  resolveApprovalCard, setStreamStatus,
} from '/assets/stream.js';
import {
  discardComposerDraft, initComposer, reuseComposerPrompt, setComposerBusy, setComposerLoading, setComposerPrompt,
  setComposerRoute, setComposerSession, setComposerVisible, submitComposerPrompt,
} from '/assets/composer.js';
import { applySession, getModelChoice, initModelPicker, loadModels, noModelReason, rememberForSession } from '/assets/model_picker.js';
import { initConnect, openConnect, wireTabKeys } from '/assets/connect.js';
import { applySessionEffort, closeEffortWindow, initEffortPicker, openEffortWindow, rememberEffortForSession, setReasoningEffort } from '/assets/effort_picker.js';
import { renderGuide } from '/assets/guide.js';
import { initSubagents, loadSubagents } from '/assets/subagents.js';
import { initLatexCanvas, refreshWorkspaceFiles } from '/assets/canvas_latex.js';
import { initDagCanvas, onDagTabShown, refreshDag, selectNodeById } from '/assets/canvas_dag.js';
import { askInExplain, initSourcesAndSidetalk, loadSessionSources, openSessionSource, refreshSourcesAndSidetalk, setProjectLibraryTarget } from '/assets/canvas_sources.js';
import { initFormalLibrary, setFormalLibraryWorkspace } from '/assets/formal_library.js';
import { closeProjects, initProjects, openProjects, promoteProjectStep, refreshProjectSessions } from '/assets/projects.js';
import { addSessionSourceToLibrary } from '/assets/project_library.js';
import { buildRecoveryPrompt } from '/assets/trace_cards.js';
import { appendSteerBadge, renderCheckpointCard, resolveCheckpointCard } from '/assets/hitl.js';
import {
  applySessionTelemetry, finishLivePipeline, getWorkbenchSnapshot, initWorkbench,
  refreshConnectionBadge, refreshConventionsState, setDrawerOpen, setRigorPreset, startLivePipeline,
  updateCanvasTabCounts, updatePipelineFromEvent,
} from '/assets/workbench.js';
import { formatBuiltinToolWarning, initSessionRail, renderSessionGrantsPill, renderSessionRailList, showToast, updateWorkspaceBadges } from '/assets/session_rail.js';

const state = {
  sessions: [],
  activeSessionId: null,
  activeProjectId: null,
  activeWorkspaceRoot: '',
  sessionQuery: '',
  railFilterMode: 'active',
  tabCounts: { dag: 0, sources: 0, sidetalk: 0 },
  eventSource: null,
  theme: 'light',
};

const el = {};
let sessionSelection = 0;
let turnRevision = 0;
let submittedTurn = null;
let sessionListRequest = 0;
let sessionTitleRevision = 0;
let navigationRevision = 0;
let canvasManuallySet = false;

document.addEventListener('DOMContentLoaded', boot);

async function boot() {
  cacheElements();
  restoreTheme();
  wireShellControls();
  wireCanvasTabs();
  wirePostMessageBridge();
  window.addEventListener('pagehide', closeSse);
  window.addEventListener('beforeunload', closeSse);

  initSessionRail({ onFilterMode: (mode) => { state.railFilterMode = mode; renderSessionList(); } });

  const inspectStepInCanvas = (stepId) => {
    activateCanvasTab('tab-dag');
    selectNodeById(stepId);
  };

  initConnect({
    onNotify: showToast,
    onRefreshModels: () => loadModels(true),
    onTabOpened: (tab) => {
      if (tab === 'subagents') loadSubagents();
      else if (tab === 'guide') renderGuide();
    },
    onChanged: async () => {
      await Promise.all([loadModels(true), refreshConnectionBadge(false), refreshConventionsState()]);
      refreshEmptyHeroIfVisible();
    },
  });
  initEffortPicker({ getModelChoice });
  initSubagents({
    onNotify: showToast,
    getModelChoice,
    onOpenSubagentsTab: () => openConnect('subagents'),
    onOrchestratorModelChanged: () => loadModels(false),
    onActivated: (sa) => {
      if (sa.rigor) setRigorPreset(sa.rigor);
      if (sa.reasoning_effort) setReasoningEffort(sa.reasoning_effort);
      showToast(`Active subagent: ${sa.name}`);
    },
  });
  renderGuide();
  initModelPicker({
    onNotify: showToast,
    onOpenConnect: () => openConnect('providers'),
    onModelSelected: (c) => openEffortWindow(c),
  });

  initStream({
    onPromptPick: (prompt) => setComposerPrompt(prompt, true),
    onNotify: showToast,
    onInspectStep: inspectStepInCanvas,
    attachClaimTrace: (row, text, meta, id) => attachAnswerTrace(row, text, meta.seq, id, {
      inspectStep: inspectStepInCanvas,
      openSource: (reader) => openSessionSource(id, reader.source_id, reader.path, reader),
    }),
    onRecoveryAction: (action, ctx) => {
      const prompt = buildRecoveryPrompt(action, ctx);
      if (action === 'explain') { activateCanvasTab('tab-sidetalk'); askInExplain(prompt); }
      else submitComposerPrompt(prompt);
    },
    onOpenConnect: openConnect,
    onOpenProjects: () => openProjects(state.activeProjectId),
    getSetupInfo: getWorkbenchSnapshot,
  });

  initComposer({
    ensureSession: ensureActiveSession,
    onSendPrompt: handleSendPrompt,
    onSteerTurn: handleSteerTurn,
    onStopTurn: handleStopTurn,
    onAttachWorkspace: handleAttachWorkspace,
    onSourcesUpdated: (sid) => loadSessionSources(sid),
    onNotify: showToast,
    getModelChoice,
    noModelReason,
    onNeedModel: openConnect,
  });

  initLatexCanvas({ onNotify: showToast });
  initDagCanvas({
    onDagCount: (n) => { state.tabCounts.dag = n; updateCanvasTabCounts(state.tabCounts); },
    onForkCreated: async (newId, open) => {
      const navigation = navigationRevision;
      await refreshSessionList();
      if (open && navigation === navigationRevision && !projectsVisible()) {
        state.railFilterMode = 'active';
        await selectSession(newId, { focus: true });
      }
      showToast(`Forked session ${newId}`);
    },
    onExplainStep: (entry) => {
      activateCanvasTab('tab-sidetalk');
      const deps = Array.isArray(entry.depends_on) && entry.depends_on.length ? ` (building on ${entry.depends_on.join(', ')})` : '';
      askInExplain(`Explain proof step ${entry.entry_id}${deps} in detail: give the intuition, fill in all intermediate steps, and justify why it follows from its assumptions.`);
    },
    onPromoteStep: (entry) => promoteProjectStep(state.activeSessionId, state.activeProjectId, entry),
    onNarrativeReady: (md) => { appendAssistantMessage(md); showToast('Proof narrative added to stream.'); },
    onNotify: showToast,
  });
  initSourcesAndSidetalk({
    onNotify: showToast,
    onSelectStepRef: inspectStepInCanvas,
    onConventionsPromoted: (resp) => refreshConventionsState(resp && resp.pinned_conventions),
    onPromoteToComposer: (txt) => setComposerPrompt(txt, true),
    onAddToProjectLibrary: addSessionSourceToLibrary,
    onCountsChange: ({ sources, sidetalk }) => {
      Object.assign(state.tabCounts, { sources, sidetalk });
      updateCanvasTabCounts(state.tabCounts);
    },
  });

  const launchToken = await api.initAuth();
  if (!launchToken) {
    document.body.innerHTML =
      '<div style="max-width:34rem;margin:18vh auto;padding:0 1.5rem;font:16px/1.5 system-ui,sans-serif">' +
      '<h1 style="font-size:1.4rem">This tab has no launch token</h1>' +
      '<p>re:Search only talks to the tab it opened. Close this tab and start re:Search again ' +
      '(<code>re-search desktop</code> or the Start menu shortcut), or switch back to the window it opened.</p></div>';
    return;
  }

  initWorkbench({ onNotify: showToast, onConnectionsUpdated: () => refreshEmptyHeroIfVisible(), getActiveSessionId: () => state.activeSessionId });
  initFormalLibrary({ onNotify: showToast });
  initProjects({
    getActiveProjectId: () => state.activeProjectId,
    getSessions: () => state.sessions,
    onNotify: showToast,
    onSessionsChanged: refreshSessionList,
    onVisibilityChange: (visible) => {
      navigationRevision += 1;
      setComposerVisible(!visible);
      if (visible && window.innerWidth <= 780) setDrawerOpen('rail', false);
    },
    onProjectSelected: () => { navigationRevision += 1; },
    onEvidenceChanged: (id) => { if (id === state.activeSessionId) refreshDag(id); },
    onCreateSession: (id) => createAndSelectSession({ project_id: id }),
    onOpenSession: async (id, entryId) => {
      const opening = selectSession(id, { focus: true });
      const navigation = navigationRevision;
      if (await opening && entryId && navigation === navigationRevision) {
        inspectStepInCanvas(entryId);
        document.getElementById('tab-dag')?.focus();
      }
    },
    onOpenSourceSession: async (id, sourceId) => {
      const opening = selectSession(id, { focus: true });
      const navigation = navigationRevision;
      if (await opening && sourceId && navigation === navigationRevision) {
        activateCanvasTab('tab-sources');
        document.getElementById('tab-sources')?.focus();
        await openSessionSource(id, sourceId);
      }
    },
  });
  const navigation = navigationRevision;
  const background = Promise.all([checkHealth(), loadModels(false), loadSubagents()]);
  await refreshSessionList();

  if (navigation === navigationRevision) {
    const firstActive = state.sessions.find((s) => !s.archived);
    if (!canvasManuallySet) setDrawerOpen('canvas', Boolean(firstActive) && window.innerWidth > 1120);
    if (firstActive) await selectSession(firstActive.id);
    else resetToBlankState();
  }
  await background;
}

function cacheElements() {
  const ids = {
    shell: 'app-shell', railToggle: 'rail-toggle', canvasToggle: 'canvas-toggle',
    railClose: 'rail-close-btn', canvasClose: 'canvas-close-btn',
    newBtn: 'new-research-btn', sessionSearch: 'session-search', sessionList: 'session-list',
    sessionTitle: 'active-session-title', wsPill: 'active-workspace-pill',
    wsBtn: 'workspace-attach-btn', grantsPill: 'session-grants-pill',
    topbarArchiveBtn: 'topbar-archive-btn', topbarDeleteBtn: 'topbar-delete-btn',
    healthBadge: 'health-badge', themeToggle: 'theme-toggle',
  };
  for (const [k, id] of Object.entries(ids)) el[k] = document.getElementById(id);
  el.sessionTitle?.setAttribute('tabindex', '-1');
}

function closeSse() {
  if (state.eventSource) { state.eventSource.close(); state.eventSource = null; }
}

function restoreTheme() {
  applyTheme((window.localStorage.getItem('re-search-theme') || 'light') === 'dark' ? 'dark' : 'light');
}

function applyTheme(theme) {
  state.theme = theme;
  document.documentElement.classList.add('theme-switching');
  document.documentElement.setAttribute('data-theme', theme);
  void document.documentElement.offsetHeight;
  document.documentElement.classList.remove('theme-switching');
  if (el.themeToggle) el.themeToggle.textContent = theme === 'dark' ? 'Light' : 'Dark';
  window.localStorage.setItem('re-search-theme', theme);
}

function wireShellControls() {
  el.railToggle?.addEventListener('click', () => {
    const open = el.shell.getAttribute('data-rail-open') === 'false';
    setDrawerOpen('rail', open);
    if (open && window.innerWidth <= 780) el.newBtn?.focus();
  });
  el.railClose?.addEventListener('click', () => {
    setDrawerOpen('rail', false);
    el.railToggle?.focus();
  });
  el.canvasToggle?.addEventListener('click', () => {
    canvasManuallySet = true;
    const open = el.shell.getAttribute('data-canvas-open') === 'false';
    setDrawerOpen('canvas', open);
    if (open && window.innerWidth <= 1120) {
      document.querySelector('.canvas-tabs [aria-selected="true"]')?.focus();
    }
  });
  el.canvasClose?.addEventListener('click', () => {
    canvasManuallySet = true;
    setDrawerOpen('canvas', false);
    el.canvasToggle?.focus();
  });
  el.themeToggle?.addEventListener('click', () => applyTheme(state.theme === 'dark' ? 'light' : 'dark'));
  el.newBtn?.addEventListener('click', async () => {
    if (el.newBtn.disabled) return;
    el.newBtn.disabled = true;
    state.railFilterMode = 'active';
    try {
      await createAndSelectSession();
    } finally {
      el.newBtn.disabled = false;
    }
  });
  el.sessionSearch?.addEventListener('input', () => { state.sessionQuery = el.sessionSearch.value.trim().toLowerCase(); renderSessionList(); });
  if (el.wsPill) { el.wsPill.style.cursor = 'pointer'; el.wsPill.addEventListener('click', () => el.wsBtn?.click()); }
  el.topbarArchiveBtn?.addEventListener('click', () => {
    const cur = state.sessions.find((s) => s.id === state.activeSessionId);
    if (cur) handleArchiveSession(cur, !cur.archived);
  });
  el.topbarDeleteBtn?.addEventListener('click', () => {
    const cur = state.sessions.find((s) => s.id === state.activeSessionId) ||
      (state.activeSessionId ? { id: state.activeSessionId, title: el.sessionTitle?.textContent } : null);
    if (cur) handleDeleteSession(cur);
  });
  el.grantsPill?.addEventListener('click', async () => {
    const id = state.activeSessionId;
    const selection = sessionSelection;
    if (!id) return;
    try {
      await api.revokeSessionGrants(id);
      if (selection === sessionSelection) renderSessionGrantsPill(el.grantsPill, []);
      showToast('Session tool approvals reset — next tool call will ask again.');
    } catch (err) {
      showToast(`Could not reset approvals: ${err.message}`);
    }
  });
}

function wireCanvasTabs() {
  const tabs = Array.from(document.querySelectorAll('.canvas-tabs [role="tab"]'));
  for (const tab of tabs) {
    tab.addEventListener('click', () => activateCanvasTab(tab.id));
  }
  wireTabKeys(tabs);
}

function activateCanvasTab(activeTabId) {
  const openingOverlay = window.innerWidth <= 1120 && el.shell.getAttribute('data-canvas-open') === 'false';
  setDrawerOpen('canvas', true);
  for (const [tId, pId] of [['tab-latex', 'panel-latex'], ['tab-dag', 'panel-dag'], ['tab-sources', 'panel-sources'], ['tab-sidetalk', 'panel-sidetalk']]) {
    const tEl = document.getElementById(tId);
    const pEl = document.getElementById(pId);
    const active = tId === activeTabId;
    if (tEl) {
      tEl.classList.toggle('is-active', active);
      tEl.setAttribute('aria-selected', active ? 'true' : 'false');
      tEl.tabIndex = active ? 0 : -1;
    }
    pEl?.classList.toggle('is-active', active);
  }
  if (openingOverlay) document.getElementById(activeTabId)?.focus();
  if (activeTabId === 'tab-dag') onDagTabShown();
}

async function checkHealth() {
  try {
    const h = await api.getHealth();
    if (el.healthBadge && h && h.ok) {
      el.healthBadge.textContent = `v${h.version || '0.0.1'}`;
      el.healthBadge.className = 'badge ok';
    }
  } catch {
    if (el.healthBadge) {
      el.healthBadge.textContent = 'Offline';
      el.healthBadge.className = 'badge warn';
    }
  }
}

async function refreshSessionList() {
  const request = ++sessionListRequest;
  try {
    const data = await api.listSessions();
    if (request !== sessionListRequest) return;
    state.sessions = Array.isArray(data) ? data : data.sessions || [];
    const active = state.sessions.find((session) => session.id === state.activeSessionId);
    if (active) {
      syncProjectMembership(active.project_id);
      renderMathTextInto(el.sessionTitle, active.title || active.id);
    }
  } catch {
    if (request !== sessionListRequest) return;
  }
  renderSessionList();
  refreshProjectSessions();
}

function syncProjectMembership(projectId) {
  state.activeProjectId = projectId || null;
  setProjectLibraryTarget(state.activeProjectId);
}

function renderSessionList() {
  renderSessionRailList(el.sessionList, {
    sessions: state.sessions,
    activeSessionId: state.activeSessionId,
    query: state.sessionQuery,
    filterMode: state.railFilterMode,
    onSelect: (id) => selectSession(id, { focus: true }),
    onArchive: handleArchiveSession,
    onDelete: handleDeleteSession,
  });
}

function syncWorkspaceIndicators(wsRoot) {
  state.activeWorkspaceRoot = wsRoot || '';
  updateWorkspaceBadges(el.wsPill, el.wsBtn, state.activeWorkspaceRoot);
  setFormalLibraryWorkspace(state.activeSessionId, state.activeWorkspaceRoot);
}

function resetToBlankState() {
  navigationRevision += 1;
  sessionSelection += 1;
  setComposerLoading(false);
  closeSse();
  state.activeSessionId = null;
  setComposerSession(null);
  syncProjectMembership(null);
  resetLiveTurnState();
  refreshWorkspaceFiles(null);
  refreshDag(null);
  refreshSourcesAndSidetalk(null);
  if (el.sessionTitle) el.sessionTitle.textContent = 'New research';
  if (el.topbarArchiveBtn) el.topbarArchiveBtn.hidden = true;
  if (el.topbarDeleteBtn) el.topbarDeleteBtn.hidden = true;
  renderSessionGrantsPill(el.grantsPill, []);
  syncWorkspaceIndicators('');
  finishLivePipeline();
  applySessionTelemetry(null, false);
  refreshConventionsState();
  closeSessionPopovers();
  applySession(null, null);
  applySessionEffort(null, null);
  setComposerRoute(null);
  setComposerBusy(false);
  setStreamStatus('Ready', 'ok');
  state.tabCounts = { dag: 0, sources: 0, sidetalk: 0 };
  updateCanvasTabCounts(state.tabCounts);
  renderSessionList();
  renderSessionStream(null, { messages: [] });
}

async function handleArchiveSession(sess, nextArchived) {
  try {
    await api.archiveSession(sess.id, nextArchived);
    await refreshSessionList();
    showToast(nextArchived ? `Archived "${sess.title || sess.id}"` : `Restored "${sess.title || sess.id}"`);
    if (nextArchived && state.activeSessionId === sess.id && state.railFilterMode === 'active') {
      const next = state.sessions.find((s) => !s.archived);
      if (next && !projectsVisible()) await selectSession(next.id);
      else resetToBlankState();
    } else if (state.activeSessionId === sess.id) {
      if (!projectsVisible()) await selectSession(sess.id);
    }
  } catch (err) {
    showToast(`Archive failed: ${err.message}`);
  }
}

async function handleDeleteSession(sess) {
  const label = sess.title || sess.id;
  if (!window.confirm(`Delete chat "${label}"?\n\nYour attached local workspace files are never touched.`)) return;
  try {
    await api.deleteSession(sess.id);
    discardComposerDraft(sess.id);
    await refreshSessionList();
    showToast(`Deleted "${label}"`);
    if (state.activeSessionId === sess.id) {
      const next = state.sessions.find((s) => Boolean(s.archived) === (state.railFilterMode === 'archived')) ||
        state.sessions.find((s) => !s.archived);
      if (next && !projectsVisible()) await selectSession(next.id);
      else resetToBlankState();
    }
  } catch (err) {
    showToast(`Delete failed: ${err.message}`);
  }
}

async function createAndSelectSession(opts = {}, { focus = true, adoptDraft = false } = {}) {
  const navigation = ++navigationRevision;
  try {
    const created = await api.createSession(opts);
    await refreshSessionList();
    if (navigation === navigationRevision) await selectSession(created.id, { focus, adoptDraft });
    return created.id;
  } catch (err) {
    showToast(`Could not create session: ${err.message}`);
    return null;
  }
}

async function ensureActiveSession() {
  if (state.activeSessionId) return state.activeSessionId;
  const id = await createAndSelectSession({}, { focus: false, adoptDraft: true });
  return id === state.activeSessionId && !projectsVisible() ? id : null;
}

function projectsVisible() {
  return document.getElementById('project-overview')?.hidden === false;
}

function closeSessionPopovers() {
  closeEffortWindow(false);
  const conventions = document.getElementById('conventions-popover');
  if (conventions) conventions.hidden = true;
}

function checkpointOpts(id) {
  return {
    onResolved: () => { if (state.activeSessionId === id) setStreamStatus('Running...', 'accent'); },
    onNotify: showToast,
  };
}

async function selectSession(id, { focus = false, adoptDraft = false } = {}) {
  if (!id) return;
  navigationRevision += 1;
  const selection = ++sessionSelection;
  const changed = state.activeSessionId !== id;
  state.activeSessionId = id;
  setComposerSession(id, { adoptDraft });
  closeProjects();
  setComposerLoading(true);
  if (changed) {
    closeSessionPopovers();
    resetLiveTurnState();
    const selected = state.sessions.find((s) => s.id === id);
    renderMathTextInto(el.sessionTitle, selected?.title || id);
    syncProjectMembership(selected?.project_id);
    renderSessionStream(id, { messages: [] });
    syncWorkspaceIndicators('');
    renderSessionGrantsPill(el.grantsPill, []);
    finishLivePipeline();
    applySessionTelemetry(null, false);
    setComposerRoute(null);
    setComposerBusy(false);
    setStreamStatus('Loading…', 'accent');
  }
  if (focus) {
    if (window.innerWidth <= 780) setDrawerOpen('rail', false);
    const selected = state.sessions.find((s) => s.id === id);
    if (selected) state.railFilterMode = selected.archived ? 'archived' : 'active';
    el.sessionTitle?.focus();
  }
  const canvas = Promise.all([refreshWorkspaceFiles(id), refreshDag(id), refreshSourcesAndSidetalk(id)]);
  renderSessionList();
  const titleRevision = sessionTitleRevision;
  connectSessionSse(id);

  try {
    const sess = await api.getSession(id);
    if (selection !== sessionSelection) return;
    setComposerLoading(false);
    syncProjectMembership(sess.project_id);
    if (titleRevision === sessionTitleRevision) renderMathTextInto(el.sessionTitle, sess.title || id);
    if (el.topbarArchiveBtn) {
      el.topbarArchiveBtn.hidden = false;
      el.topbarArchiveBtn.title = sess.archived ? 'Restore chat to Active' : 'Archive this chat';
      el.topbarArchiveBtn.setAttribute('aria-label', el.topbarArchiveBtn.title);
    }
    if (el.topbarDeleteBtn) el.topbarDeleteBtn.hidden = false;
    syncWorkspaceIndicators(sess.workspace_root || '');
    renderSessionGrantsPill(el.grantsPill, sess.session_grants || []);
    renderSessionStream(id, sess);
    const pendingCps = Array.isArray(sess.pending_checkpoints) ? sess.pending_checkpoints : [];
    const messageStreamEl = document.getElementById('message-stream');
    pendingCps.forEach((cp) => renderCheckpointCard(messageStreamEl, id, cp, checkpointOpts(id)));
    applySession(id, sess);
    setComposerRoute(sess.route || null);
    applySessionEffort(id, sess);
    refreshConventionsState(typeof sess?.meta?.pinned_conventions === 'string' ? sess.meta.pinned_conventions : null);
    applySessionTelemetry(sess.stats, Array.isArray(sess.messages) && sess.messages.length > 0);
    const isRunning = Boolean(sess.running);
    const hasApprovals = Boolean(sess.pending_approvals && sess.pending_approvals.length > 0);
    const hasPending = hasApprovals || pendingCps.length > 0;
    setComposerBusy(isRunning || hasPending);
    setStreamStatus(
      hasApprovals ? 'Awaiting Approval' : pendingCps.length > 0 ? 'Awaiting Input' : isRunning ? 'Running...' : 'Ready',
      hasPending ? 'warn' : isRunning ? 'accent' : 'ok'
    );
    await canvas;
    return selection === sessionSelection;
  } catch (err) {
    if (selection === sessionSelection) {
      setComposerLoading(false);
      setComposerBusy(false);
      setStreamStatus('Error', 'err');
      showToast(`Failed to load session ${id}: ${err.message}`);
    }
  }
}

function connectSessionSse(id) {
  closeSse();
  const selection = sessionSelection;
  let interrupted = false;
  let recoveredRevision = -1;
  let latestPrompt = '';
  const isAct = () => selection === sessionSelection && state.activeSessionId === id && state.eventSource === source;
  const source = api.openSessionEvents(id, {
    open: async () => {
      if (!isAct() || !interrupted) return;
      interrupted = false;
      const revision = turnRevision;
      const titleRevision = sessionTitleRevision;
      try {
        const sess = await api.getSession(id);
        if (!isAct() || interrupted || revision !== turnRevision) return;
        if (titleRevision === sessionTitleRevision) renderMathTextInto(el.sessionTitle, sess.title || id);
        if (sess.running) {
          const approvals = sess.pending_approvals || [];
          const checkpoints = sess.pending_checkpoints || [];
          approvals.forEach(renderApprovalCard);
          const messageStream = document.getElementById('message-stream');
          checkpoints.forEach((cp) => renderCheckpointCard(messageStream, id, cp, checkpointOpts(id)));
          setComposerBusy(true);
          setStreamStatus(
            approvals.length ? 'Awaiting Approval' : checkpoints.length ? 'Awaiting Input' : 'Running...',
            approvals.length || checkpoints.length ? 'warn' : 'accent'
          );
          return;
        }
        recoveredRevision = revision;
        resetLiveTurnState();
        renderSessionStream(id, sess);
        setComposerBusy(false);
        applySession(id, sess);
        setComposerRoute(sess.route || null);
        renderSessionGrantsPill(el.grantsPill, sess.session_grants || []);
        const last = (sess.events || []).findLast((e) => [
          'session.start', 'session.resume', 'session.abort', 'session.end',
        ].includes(e.kind));
        if (last?.kind === 'session.abort') {
          presentTurnError(last.payload?.reason, sess.messages?.findLast((m) => m.role === 'user')?.text);
        } else {
          setStreamStatus('Ready', 'ok');
          finishLivePipeline('ok', sess.stats);
        }
        if (submittedTurn?.sessionId === id) submittedTurn = null;
        await Promise.all([refreshSessionList(), refreshWorkspaceFiles(id), refreshDag(id), loadSessionSources(id)]);
      } catch (err) {
        if (isAct() && revision === turnRevision) showToast(`Could not refresh the chat. ${err.message}`);
      }
    },
    error: () => {
      if (!isAct()) return;
      interrupted = true;
      setStreamStatus(source.readyState === EventSource.CLOSED ? 'Disconnected' : 'Reconnecting...', 'warn');
    },
    text_delta: (d) => { if (!isAct()) return; setComposerBusy(true); setStreamStatus('Streaming...', 'accent'); updatePipelineFromEvent('solver_stream', d); handleTextDelta(d); },
    thinking_delta: (d) => { if (!isAct()) return; setComposerBusy(true); handleThinkingDelta(d); },
    round_reset: (d) => { if (!isAct()) return; setStreamStatus('Revising...', 'accent'); updatePipelineFromEvent('round_reset', d); handleRoundReset(); },
    tool_trace: (d) => { if (!isAct()) return; updatePipelineFromEvent('tool_trace', d); handleToolTrace(d); },
    approval_requested: (d) => {
      if (!isAct()) return;
      setComposerBusy(true);
      setStreamStatus('Awaiting Approval', 'warn');
      const card = renderApprovalCard(d);
      if (card && d.caller === 'user:canvas') {
        if (window.innerWidth <= 1120) setDrawerOpen('canvas', false);
        card.querySelector('button')?.focus({ preventScroll: true });
        card.scrollIntoView({ block: 'nearest' });
      }
    },
    approval_resolved: async (d) => {
      if (!isAct() || !d || d.auto_approved) return;
      resolveApprovalCard(d.call_id, d.approved, Boolean(d.approve_for_session));
      const waiting = d.pending_approvals ? 'Awaiting Approval' : d.pending_checkpoints ? 'Awaiting Input' : '';
      const running = d.running !== false;
      setComposerBusy(Boolean(waiting) || running);
      setStreamStatus(waiting || (running ? 'Running...' : 'Ready'), waiting ? 'warn' : running ? 'accent' : 'ok');
      if (d.approve_for_session) {
        try {
          const s = await api.getSession(id);
          if (isAct()) renderSessionGrantsPill(el.grantsPill, s.session_grants || []);
        } catch { /* ignore */ }
      }
    },
    checkpoint_requested: (d) => {
      if (!isAct() || !d) return;
      setComposerBusy(true);
      setStreamStatus('Awaiting Input', 'warn');
      renderCheckpointCard(document.getElementById('message-stream'), id, d, checkpointOpts(id));
    },
    claim_review_started: () => {
      if (!isAct()) return;
      setComposerBusy(true);
      setStreamStatus('Reviewing claim…', 'accent');
    },
    claim_review_finished: () => {
      if (!isAct()) return;
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
    },
    checkpoint_resolved: (d) => {
      if (!isAct() || !d) return;
      setStreamStatus('Running...', 'accent');
      resolveCheckpointCard(document.getElementById('message-stream'), d.checkpoint_id, d);
    },
    steer_queued: (d) => {
      if (isAct() && d) appendSteerBadge(document.getElementById('message-stream'), d.text);
    },
    dag_updated: (d) => { if (!isAct()) return; updatePipelineFromEvent('dag_updated', d); refreshDag(id); },
    session_event: (d) => {
      if (!isAct()) return;
      if (d?.kind === 'session.title' && typeof d.data?.title === 'string') {
        sessionTitleRevision += 1;
        const session = state.sessions.find((candidate) => candidate.id === id);
        if (session) session.title = d.data.title;
        renderMathTextInto(el.sessionTitle, d.data.title);
        renderSessionList();
        refreshSessionList();
      }
      if (d?.kind === 'session.start' || d?.kind === 'session.resume') {
        recoveredRevision = -1;
        latestPrompt = d.data?.prompt || '';
      }
      if (d?.kind === 'literature.fetched' || d?.kind === 'source.ingested') loadSessionSources(id);
      updatePipelineFromEvent('session_event', d);
      if (d && d.kind === 'vendor.builtin_tool') appendWarningLine(formatBuiltinToolWarning(d.data || {}));
    },
    turn_complete: async (d) => {
      if (!isAct() || recoveredRevision === turnRevision) return;
      turnRevision += 1;
      if (submittedTurn?.sessionId === id) submittedTurn = null;
      latestPrompt = '';
      finalizeLiveThinking();
      try {
        const sess = await api.getSession(id);
        if (!isAct()) return;
        const msgs = Array.isArray(sess?.messages) ? sess.messages : [];
        const lastMsg = msgs.length > 0 ? msgs[msgs.length - 1] : null;
        renderSessionGrantsPill(el.grantsPill, sess.session_grants || []);
        if (d && d.text) {
          finalizeLiveAssistant(d.text, lastMsg?.role === 'assistant' ? lastMsg : null);
          resetLiveTurnState();
        } else if (lastMsg?.role === 'assistant') {
          resetLiveTurnState();
          renderSessionStream(id, sess);
        } else {
          finalizeLiveAssistant('');
          resetLiveTurnState();
        }
        applySession(id, sess);
        setComposerRoute(sess.route || null);
        finishLivePipeline('ok', sess && sess.stats);
      } catch {
        if (!isAct()) return;
        finalizeLiveAssistant((d && d.text) || '');
        resetLiveTurnState();
        finishLivePipeline('ok', null);
      }
      if (isAct()) {
        setComposerBusy(false);
        setStreamStatus('Ready', 'ok');
        await Promise.all([refreshSessionList(), refreshWorkspaceFiles(id), refreshDag(id), loadSessionSources(id)]);
      } else await refreshSessionList();
    },
    turn_error: (d) => {
      if (!isAct() || recoveredRevision === turnRevision) return;
      turnRevision += 1;
      finalizeLiveThinking();
      finalizeLiveOnError();
      resetLiveTurnState();
      setComposerBusy(false);
      const prompt = submittedTurn?.sessionId === id ? submittedTurn.prompt : latestPrompt;
      if (submittedTurn?.sessionId === id) submittedTurn = null;
      presentTurnError(d?.error || d?.message, prompt);
      latestPrompt = '';
      refreshSessionList();
      refreshSessionRoute(id);
    },
  });
  state.eventSource = source;
}

function presentTurnError(reason, prompt) {
  const detail = String(reason || 'Unknown failure');
  const stopped = /stopped by (?:the )?user/i.test(detail);
  setStreamStatus(stopped ? 'Stopped' : 'Error', stopped ? 'warn' : 'err');
  finishLivePipeline(stopped ? 'stopped' : 'error', null);
  appendWarningLine(stopped ? 'Turn stopped.' : `Turn failed. ${detail}`, prompt
    ? { label: 'Reuse prompt', onClick: () => reuseComposerPrompt(prompt) }
    : null);
}

async function refreshSessionRoute(id) {
  if (state.activeSessionId !== id) return;
  const selection = sessionSelection;
  try {
    const sess = await api.getSession(id);
    if (selection === sessionSelection) { applySession(id, sess); setComposerRoute(sess.route || null); }
  } catch { /* ignore */ }
}

async function handleSendPrompt(sessionId, prompt, opts) {
  const selection = sessionSelection;
  const navigation = navigationRevision;
  const isActive = () => selection === sessionSelection && sessionId === state.activeSessionId;
  if (!isActive()) throw new Error('The selected chat changed. Send again in the intended chat.');
  const source = state.eventSource;
  setStreamStatus('Connecting...', 'accent');
  const ready = await api.waitForSessionEvents(source);
  if (!isActive() || state.eventSource !== source || navigation !== navigationRevision) {
    if (isActive() && state.eventSource === source) setStreamStatus('Ready', 'ok');
    throw new Error('The research view changed before Send. Try again in the intended chat.');
  }
  if (!ready) {
    setStreamStatus('Disconnected', 'warn');
    throw new Error('Live updates are unavailable. Try Send again once the connection returns.');
  }
  turnRevision += 1;
  const submitted = { sessionId, prompt };
  submittedTurn = submitted;
  resetLiveTurnState();
  appendUserMessage(prompt, { optimistic: true });
  setComposerRoute(opts.provider || null);
  setComposerBusy(true);
  setStreamStatus('Thinking...', 'accent');
  startLivePipeline(opts);
  try {
    await api.sendMessage(sessionId, { prompt, ...opts });
    if (isActive()) confirmOptimisticUserMessage();
    rememberForSession(sessionId, { provider: opts.provider, model: opts.model });
    rememberEffortForSession(sessionId, opts.reasoning_effort);
    refreshSessionList();
  } catch (err) {
    if (submittedTurn === submitted) submittedTurn = null;
    if (isActive()) {
      removeOptimisticUserMessage();
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
      finishLivePipeline('error', null);
    }
    throw err;
  }
}

async function handleSteerTurn(sessionId, text) {
  if (state.activeSessionId !== sessionId) throw new Error('The selected chat changed.');
  await api.postSteer(sessionId, text);
}

async function handleStopTurn() {
  const id = state.activeSessionId;
  if (!id) return false;
  const selection = sessionSelection;
  const revision = turnRevision;
  try {
    setStreamStatus('Stopping...', 'warn');
    await api.stopTurn(id);
    return true;
  } catch (err) {
    if (selection === sessionSelection && revision === turnRevision) {
      setStreamStatus('Running...', 'accent');
      showToast(`Could not stop the turn. ${err.message}`);
    }
    return false;
  }
}

async function handleAttachWorkspace(workspaceRoot) {
  if (!state.activeSessionId) {
    const createdId = await createAndSelectSession({ workspace_root: workspaceRoot }, { adoptDraft: true });
    if (createdId) showToast(`Attached workspace: ${workspaceRoot}`);
    return;
  }
  const id = state.activeSessionId;
  const selection = sessionSelection;
  try {
    const sess = await api.setSessionWorkspace(id, workspaceRoot);
    if (selection !== sessionSelection) {
      await refreshSessionList();
      return;
    }
    syncWorkspaceIndicators((sess && sess.workspace_root) || workspaceRoot);
    refreshEmptyHeroIfVisible();
    await Promise.all([refreshSessionList(), refreshWorkspaceFiles(id)]);
    showToast(`Attached workspace: ${(sess && sess.workspace_root) || workspaceRoot}`);
  } catch (err) {
    showToast(`Could not attach workspace: ${err.message}`);
  }
}

function wirePostMessageBridge() {
  window.addEventListener('message', (ev) => {
    if (ev.origin && ev.origin !== window.location.origin && ev.origin !== 'null') return;
    const msg = ev.data;
    if (!msg || typeof msg !== 'object') return;
    if (msg.type === 're-search:select-tab' && msg.tab) activateCanvasTab(msg.tab);
    else if (msg.type === 're-search:set-prompt' && typeof msg.prompt === 'string') setComposerPrompt(msg.prompt, false);
    else if (msg.type === 're-search:set-theme' && (msg.theme === 'light' || msg.theme === 'dark')) applyTheme(msg.theme);
  });
}
