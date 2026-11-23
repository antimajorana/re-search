// Main entry controller: wires left rail (Active/Archived + Delete), Connect panel, model picker,
// live pipeline bar, center stream, SSE live updates, resizable Canvas tabs, and postMessage bridge.
import * as api from '/assets/api.js';
import {
  appendAssistantMessage, appendUserMessage, appendWarningLine, confirmOptimisticUserMessage,
  finalizeLiveAssistant, finalizeLiveOnError, finalizeLiveThinking, handleRoundReset,
  handleTextDelta, handleThinkingDelta, handleToolTrace, initStream, refreshEmptyHeroIfVisible,
  removeOptimisticUserMessage, renderApprovalCard, renderSessionStream, resetLiveTurnState,
  resolveApprovalCard, setStreamStatus,
} from '/assets/stream.js';
import { initComposer, setComposerBusy, setComposerPrompt, setComposerRoute, submitComposerPrompt } from '/assets/composer.js';
import { applySession, getModelChoice, initModelPicker, loadModels, noModelReason, rememberForSession } from '/assets/model_picker.js';
import { initConnect, openConnect } from '/assets/connect.js';
import { applySessionEffort, initEffortPicker, openEffortWindow, rememberEffortForSession, setReasoningEffort } from '/assets/effort_picker.js';
import { renderGuide } from '/assets/guide.js';
import { initSubagents, loadSubagents } from '/assets/subagents.js';
import { initLatexCanvas, refreshWorkspaceFiles } from '/assets/canvas_latex.js';
import { initDagCanvas, onDagTabShown, refreshDag, selectNodeById } from '/assets/canvas_dag.js';
import { askInExplain, initSourcesAndSidetalk, loadSessionSources, refreshSourcesAndSidetalk } from '/assets/canvas_sources.js';
import { buildRecoveryPrompt } from '/assets/trace_cards.js';
import { appendSteerBadge, renderCheckpointCard, resolveCheckpointCard } from '/assets/hitl.js';
import {
  applySessionTelemetry, finishLivePipeline, getWorkbenchSnapshot, initWorkbench,
  refreshConnectionBadge, refreshConventionsState, setRigorPreset, startLivePipeline,
  updateCanvasTabCounts, updatePipelineFromEvent,
} from '/assets/workbench.js';
import { formatBuiltinToolWarning, initSessionRail, renderSessionGrantsPill, renderSessionRailList, showToast, updateWorkspaceBadges } from '/assets/session_rail.js';

const state = {
  sessions: [],
  activeSessionId: null,
  activeWorkspaceRoot: '',
  sessionQuery: '',
  railFilterMode: 'active',
  tabCounts: { dag: 0, sources: 0, sidetalk: 0 },
  eventSource: null,
  theme: 'light',
};

const el = {};
let sessionSelection = 0;
let sessionLoading = false;

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
    onRecoveryAction: (action, ctx) => {
      const prompt = buildRecoveryPrompt(action, ctx);
      if (action === 'explain') { activateCanvasTab('tab-sidetalk'); askInExplain(prompt); }
      else submitComposerPrompt(prompt);
    },
    onOpenConnect: openConnect,
    onPickWorkspace: () => el.wsBtn?.click(),
    onOpenConventions: () => document.getElementById('conventions-btn')?.click(),
    getSetupInfo: () => ({ ...getWorkbenchSnapshot(), workspaceRoot: state.activeWorkspaceRoot }),
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
      const selection = sessionSelection;
      await refreshSessionList();
      if (open && selection === sessionSelection) {
        state.railFilterMode = 'active';
        await selectSession(newId);
      }
      showToast(`Forked session ${newId}`);
    },
    onExplainStep: (entry) => {
      activateCanvasTab('tab-sidetalk');
      const deps = Array.isArray(entry.depends_on) && entry.depends_on.length ? ` (building on ${entry.depends_on.join(', ')})` : '';
      askInExplain(`Explain proof step ${entry.entry_id}${deps} in detail: give the intuition, fill in all intermediate steps, and justify why it follows from its assumptions.`);
    },
    onNarrativeReady: (md) => { appendAssistantMessage(md); showToast('Proof narrative added to stream.'); },
    onNotify: showToast,
  });
  initSourcesAndSidetalk({
    onNotify: showToast,
    onSelectStepRef: inspectStepInCanvas,
    onConventionsPromoted: (resp) => refreshConventionsState(resp && resp.pinned_conventions),
    onPromoteToComposer: (txt) => setComposerPrompt(txt, true),
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
  await Promise.all([checkHealth(), loadModels(false), refreshSessionList(), loadSubagents()]);

  const firstActive = state.sessions.find((s) => !s.archived);
  if (firstActive) await selectSession(firstActive.id);
  else resetToBlankState();
}

function cacheElements() {
  const ids = {
    shell: 'app-shell', railToggle: 'rail-toggle', canvasToggle: 'canvas-toggle',
    newBtn: 'new-research-btn', sessionSearch: 'session-search', sessionList: 'session-list',
    sessionTitle: 'active-session-title', wsPill: 'active-workspace-pill',
    wsBtn: 'workspace-attach-btn', grantsPill: 'session-grants-pill',
    topbarArchiveBtn: 'topbar-archive-btn', topbarDeleteBtn: 'topbar-delete-btn',
    healthBadge: 'health-badge', themeToggle: 'theme-toggle',
  };
  for (const [k, id] of Object.entries(ids)) el[k] = document.getElementById(id);
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
    el.shell.setAttribute('data-rail-open', el.shell.getAttribute('data-rail-open') !== 'false' ? 'false' : 'true');
  });
  el.canvasToggle?.addEventListener('click', () => {
    const next = el.shell.getAttribute('data-canvas-open') !== 'false' ? 'false' : 'true';
    el.shell.setAttribute('data-canvas-open', next);
    el.canvasToggle.setAttribute('aria-expanded', next);
  });
  el.themeToggle?.addEventListener('click', () => applyTheme(state.theme === 'dark' ? 'light' : 'dark'));
  el.newBtn?.addEventListener('click', () => { state.railFilterMode = 'active'; createAndSelectSession({}); });
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
  for (const tabId of ['tab-latex', 'tab-dag', 'tab-sources', 'tab-sidetalk']) {
    document.getElementById(tabId)?.addEventListener('click', () => activateCanvasTab(tabId));
  }
}

function activateCanvasTab(activeTabId) {
  el.shell?.setAttribute('data-canvas-open', 'true');
  for (const [tId, pId] of [['tab-latex', 'panel-latex'], ['tab-dag', 'panel-dag'], ['tab-sources', 'panel-sources'], ['tab-sidetalk', 'panel-sidetalk']]) {
    const tEl = document.getElementById(tId);
    const pEl = document.getElementById(pId);
    const active = tId === activeTabId;
    if (tEl) {
      tEl.classList.toggle('is-active', active);
      tEl.setAttribute('aria-selected', active ? 'true' : 'false');
    }
    pEl?.classList.toggle('is-active', active);
  }
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
  try {
    const data = await api.listSessions();
    state.sessions = Array.isArray(data) ? data : data.sessions || [];
  } catch {
    state.sessions = [];
  }
  renderSessionList();
}

function renderSessionList() {
  renderSessionRailList(el.sessionList, {
    sessions: state.sessions,
    activeSessionId: state.activeSessionId,
    query: state.sessionQuery,
    filterMode: state.railFilterMode,
    onSelect: (id) => selectSession(id),
    onArchive: handleArchiveSession,
    onDelete: handleDeleteSession,
  });
}

function syncWorkspaceIndicators(wsRoot) {
  state.activeWorkspaceRoot = wsRoot || '';
  updateWorkspaceBadges(el.wsPill, el.wsBtn, state.activeWorkspaceRoot);
}

function resetToBlankState() {
  sessionSelection += 1;
  sessionLoading = false;
  closeSse();
  state.activeSessionId = null;
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
      if (next) await selectSession(next.id);
      else resetToBlankState();
    } else if (state.activeSessionId === sess.id) {
      await selectSession(sess.id);
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
    const wasActive = state.activeSessionId === sess.id;
    await refreshSessionList();
    showToast(`Deleted "${label}"`);
    if (wasActive) {
      const next = state.sessions.find((s) => Boolean(s.archived) === (state.railFilterMode === 'archived')) ||
        state.sessions.find((s) => !s.archived);
      if (next) await selectSession(next.id);
      else resetToBlankState();
    }
  } catch (err) {
    showToast(`Delete failed: ${err.message}`);
  }
}

async function createAndSelectSession(opts = {}) {
  try {
    const created = await api.createSession(opts);
    await refreshSessionList();
    await selectSession(created.id);
    return created.id;
  } catch (err) {
    showToast(`Could not create session: ${err.message}`);
    return null;
  }
}

async function ensureActiveSession() {
  if (sessionLoading) return null;
  if (state.activeSessionId) return state.activeSessionId;
  return createAndSelectSession({});
}

function checkpointOpts(id) {
  return {
    onResolved: () => { if (state.activeSessionId === id) setStreamStatus('Running...', 'accent'); },
    onNotify: showToast,
  };
}

async function selectSession(id) {
  if (!id) return;
  const selection = ++sessionSelection;
  const changed = state.activeSessionId !== id;
  state.activeSessionId = id;
  sessionLoading = true;
  if (changed) {
    resetLiveTurnState();
    if (el.sessionTitle) el.sessionTitle.textContent = state.sessions.find((s) => s.id === id)?.title || id;
    renderSessionStream(id, { messages: [] });
    syncWorkspaceIndicators('');
    renderSessionGrantsPill(el.grantsPill, []);
    finishLivePipeline();
    applySessionTelemetry(null, false);
    setComposerRoute(null);
    setComposerBusy(false);
    setStreamStatus('Loading…', 'accent');
  }
  const canvas = Promise.all([refreshWorkspaceFiles(id), refreshDag(id), refreshSourcesAndSidetalk(id)]);
  renderSessionList();
  connectSessionSse(id);

  try {
    const sess = await api.getSession(id);
    if (selection !== sessionSelection) return;
    sessionLoading = false;
    if (el.sessionTitle) el.sessionTitle.textContent = sess.title || id;
    if (el.topbarArchiveBtn) {
      el.topbarArchiveBtn.hidden = false;
      el.topbarArchiveBtn.title = sess.archived ? 'Restore chat to Active' : 'Archive this chat';
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
  } catch (err) {
    if (selection === sessionSelection) {
      sessionLoading = false;
      setComposerBusy(false);
      setStreamStatus('Error', 'err');
      showToast(`Failed to load session ${id}: ${err.message}`);
    }
  }
}

function connectSessionSse(id) {
  closeSse();
  const selection = sessionSelection;
  const isAct = () => selection === sessionSelection && state.activeSessionId === id;
  state.eventSource = api.openSessionEvents(id, {
    text_delta: (d) => { if (!isAct()) return; setComposerBusy(true); setStreamStatus('Streaming...', 'accent'); updatePipelineFromEvent('solver_stream', d); handleTextDelta(d); },
    thinking_delta: (d) => { if (!isAct()) return; setComposerBusy(true); updatePipelineFromEvent('thinking_delta', d); handleThinkingDelta(d); },
    round_reset: (d) => { if (!isAct()) return; setStreamStatus('Revising...', 'accent'); updatePipelineFromEvent('round_reset', d); handleRoundReset(); },
    tool_trace: (d) => { if (!isAct()) return; updatePipelineFromEvent('tool_trace', d); handleToolTrace(d); },
    approval_requested: (d) => {
      if (!isAct()) return;
      setComposerBusy(true);
      setStreamStatus('Awaiting Approval', 'warn');
      renderApprovalCard(d);
    },
    approval_resolved: async (d) => {
      if (!isAct()) return;
      setStreamStatus('Running...', 'accent');
      if (d) {
        resolveApprovalCard(d.call_id, d.approved, Boolean(d.approve_for_session));
        if (d.approve_for_session && isAct()) {
          try {
            const s = await api.getSession(id);
            if (isAct()) renderSessionGrantsPill(el.grantsPill, s.session_grants || []);
          } catch { /* ignore */ }
        }
      }
    },
    checkpoint_requested: (d) => {
      if (!isAct() || !d) return;
      setComposerBusy(true);
      setStreamStatus('Awaiting Input', 'warn');
      renderCheckpointCard(document.getElementById('message-stream'), id, d, checkpointOpts(id));
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
      updatePipelineFromEvent('session_event', d);
      if (d && d.kind === 'vendor.builtin_tool') appendWarningLine(formatBuiltinToolWarning(d.data || {}));
    },
    turn_complete: async (d) => {
      if (!isAct()) return;
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
      if (!isAct()) return;
      finalizeLiveThinking();
      finalizeLiveOnError();
      resetLiveTurnState();
      setComposerBusy(false);
      const rawErr = String((d && (d.error || d.message)) || 'unknown failure');
      if (/stopped/i.test(rawErr)) {
        setStreamStatus('Stopped', 'warn');
        finishLivePipeline('stopped', null);
        showToast('Turn stopped.');
      } else {
        setStreamStatus('Error', 'err');
        finishLivePipeline('error', null);
        showToast(`Turn error: ${rawErr}`);
      }
      refreshSessionList();
      refreshSessionRoute(id);
    },
  });
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
  const isActive = () => selection === sessionSelection && sessionId === state.activeSessionId;
  if (isActive()) {
    resetLiveTurnState();
    appendUserMessage(prompt, { optimistic: true });
    setComposerRoute(opts.provider || null);
    setComposerBusy(true);
    setStreamStatus('Thinking...', 'accent');
    startLivePipeline(opts);
  }
  try {
    await api.sendMessage(sessionId, { prompt, ...opts });
    if (isActive()) confirmOptimisticUserMessage();
    rememberForSession(sessionId, { provider: opts.provider, model: opts.model });
    rememberEffortForSession(sessionId, opts.reasoning_effort);
    refreshSessionList();
  } catch (err) {
    if (isActive()) {
      removeOptimisticUserMessage();
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
      finishLivePipeline('error', null);
    }
    throw err;
  }
}

async function handleSteerTurn(text) {
  if (!state.activeSessionId) return;
  try {
    await api.postSteer(state.activeSessionId, text);
  } catch (err) {
    showToast(`Could not queue steer: ${err.message}`);
    err._notified = true;
    throw err;
  }
}

async function handleStopTurn() {
  if (!state.activeSessionId) return;
  try {
    setStreamStatus('Stopping...', 'warn');
    await api.stopTurn(state.activeSessionId);
  } catch (err) {
    showToast(`Could not stop turn: ${err.message}`);
  }
}

async function handleAttachWorkspace(workspaceRoot) {
  if (!state.activeSessionId) {
    const createdId = await createAndSelectSession({ workspace_root: workspaceRoot });
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
