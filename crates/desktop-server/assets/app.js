// Main entry controller: wires left rail, Connect panel, model picker, live pipeline bar,
// center stream, SSE live updates, resizable Canvas tabs, and postMessage bridge.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';
import {
  appendAssistantMessage,
  appendUserMessage,
  appendWarningLine,
  finalizeLiveAssistant,
  finalizeLiveOnError,
  finalizeLiveThinking,
  handleRoundReset,
  handleTextDelta,
  handleThinkingDelta,
  handleToolTrace,
  initStream,
  refreshEmptyHeroIfVisible,
  renderApprovalCard,
  renderSessionStream,
  resolveApprovalCard,
  setStreamStatus,
} from '/assets/stream.js';
import {
  initComposer,
  setComposerBusy,
  setComposerPrompt,
} from '/assets/composer.js';
import {
  applySession,
  getModelChoice,
  initModelPicker,
  loadModels,
  noModelReason,
  rememberForSession,
} from '/assets/model_picker.js';
import { initConnect, openConnect } from '/assets/connect.js';
import { initLatexCanvas, refreshWorkspaceFiles } from '/assets/canvas_latex.js';
import { initDagCanvas, refreshDag, selectNodeById } from '/assets/canvas_dag.js';
import {
  initSourcesAndSidetalk,
  loadSessionSources,
  refreshSourcesAndSidetalk,
} from '/assets/canvas_sources.js';
import {
  applySessionTelemetry,
  finishLivePipeline,
  getWorkbenchSnapshot,
  initWorkbench,
  refreshConnectionBadge,
  refreshConventionsState,
  startLivePipeline,
  updateCanvasTabCounts,
  updatePipelineFromEvent,
} from '/assets/workbench.js';

const state = {
  sessions: [],
  activeSessionId: null,
  activeWorkspaceRoot: '',
  sessionQuery: '',
  tabCounts: { dag: 0, sources: 0, sidetalk: 0 },
  eventSource: null,
  theme: 'light',
};

const el = {};

document.addEventListener('DOMContentLoaded', boot);

function basenameOf(p) {
  if (!p) return '';
  const parts = String(p).replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || String(p);
}

async function boot() {
  cacheElements();
  restoreTheme();
  wireShellControls();
  wireCanvasTabs();
  wirePostMessageBridge();

  const inspectStepInCanvas = (stepId) => {
    activateCanvasTab('tab-dag');
    selectNodeById(stepId);
  };

  initConnect({
    onNotify: showToast,
    onChanged: async () => {
      await Promise.all([loadModels(true), refreshConnectionBadge(false), refreshConventionsState()]);
      refreshEmptyHeroIfVisible();
    },
  });
  initModelPicker({ onNotify: showToast, onOpenConnect: openConnect });

  initStream({
    onPromptPick: (prompt) => setComposerPrompt(prompt, true),
    onNotify: showToast,
    onInspectStep: inspectStepInCanvas,
    onOpenConnect: openConnect,
    onPickWorkspace: () => {
      const btn = document.getElementById('workspace-attach-btn');
      if (btn) btn.click();
    },
    onOpenConventions: () => {
      const btn = document.getElementById('conventions-btn');
      if (btn) btn.click();
    },
    getSetupInfo: () => ({
      ...getWorkbenchSnapshot(),
      workspaceRoot: state.activeWorkspaceRoot,
    }),
  });

  initComposer({
    ensureSession: ensureActiveSession,
    onSendPrompt: handleSendPrompt,
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
    onDagCount: (n) => {
      state.tabCounts.dag = n;
      updateCanvasTabCounts(state.tabCounts);
    },
    onForkCreated: async (newId) => {
      await refreshSessionList();
      await selectSession(newId);
      showToast(`Forked session ${newId}`);
    },
    onNarrativeReady: (md) => {
      appendAssistantMessage(md);
      showToast('Proof narrative added to stream.');
    },
    onNotify: showToast,
  });
  initSourcesAndSidetalk({
    onNotify: showToast,
    onSelectStepRef: inspectStepInCanvas,
    onCountsChange: ({ sources, sidetalk }) => {
      state.tabCounts.sources = sources;
      state.tabCounts.sidetalk = sidetalk;
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

  initWorkbench({
    onNotify: showToast,
    onConnectionsUpdated: () => refreshEmptyHeroIfVisible(),
  });

  await Promise.all([checkHealth(), loadModels(false), refreshSessionList()]);

  if (state.sessions.length > 0) {
    await selectSession(state.sessions[0].id);
  } else {
    syncWorkspaceIndicators('');
    renderSessionStream(null, { messages: [] });
  }
}

function cacheElements() {
  el.shell = document.getElementById('app-shell');
  el.railToggle = document.getElementById('rail-toggle');
  el.canvasToggle = document.getElementById('canvas-toggle');
  el.newBtn = document.getElementById('new-research-btn');
  el.sessionSearch = document.getElementById('session-search');
  el.sessionList = document.getElementById('session-list');
  el.sessionTitle = document.getElementById('active-session-title');
  el.wsPill = document.getElementById('active-workspace-pill');
  el.wsBtn = document.getElementById('workspace-attach-btn');
  el.healthBadge = document.getElementById('health-badge');
  el.themeToggle = document.getElementById('theme-toggle');
}

function restoreTheme() {
  const saved = window.localStorage.getItem('re-search-theme') || 'light';
  applyTheme(saved === 'dark' ? 'dark' : 'light');
}

function applyTheme(theme) {
  state.theme = theme;
  document.documentElement.setAttribute('data-theme', theme);
  if (el.themeToggle) {
    el.themeToggle.textContent = theme === 'dark' ? 'Light' : 'Dark';
  }
  window.localStorage.setItem('re-search-theme', theme);
}

function wireShellControls() {
  if (el.railToggle) {
    el.railToggle.addEventListener('click', () => {
      const open = el.shell.getAttribute('data-rail-open') !== 'false';
      el.shell.setAttribute('data-rail-open', open ? 'false' : 'true');
    });
  }
  if (el.canvasToggle) {
    el.canvasToggle.addEventListener('click', () => {
      const open = el.shell.getAttribute('data-canvas-open') !== 'false';
      el.shell.setAttribute('data-canvas-open', open ? 'false' : 'true');
      el.canvasToggle.setAttribute('aria-expanded', open ? 'false' : 'true');
    });
  }
  if (el.themeToggle) {
    el.themeToggle.addEventListener('click', () => {
      applyTheme(state.theme === 'dark' ? 'light' : 'dark');
    });
  }
  if (el.newBtn) {
    el.newBtn.addEventListener('click', () => createAndSelectSession({}));
  }
  if (el.sessionSearch) {
    el.sessionSearch.addEventListener('input', () => {
      state.sessionQuery = el.sessionSearch.value.trim().toLowerCase();
      renderSessionList();
    });
  }
  if (el.wsPill) {
    el.wsPill.style.cursor = 'pointer';
    el.wsPill.addEventListener('click', () => {
      if (el.wsBtn) el.wsBtn.click();
    });
  }
}

function wireCanvasTabs() {
  const tabs = ['tab-latex', 'tab-dag', 'tab-sources', 'tab-sidetalk'];
  for (const tabId of tabs) {
    const btn = document.getElementById(tabId);
    if (btn) btn.addEventListener('click', () => activateCanvasTab(tabId));
  }
}

function activateCanvasTab(activeTabId) {
  if (el.shell) el.shell.setAttribute('data-canvas-open', 'true');
  const map = {
    'tab-latex': 'panel-latex',
    'tab-dag': 'panel-dag',
    'tab-sources': 'panel-sources',
    'tab-sidetalk': 'panel-sidetalk',
  };
  for (const [tId, pId] of Object.entries(map)) {
    const tEl = document.getElementById(tId);
    const pEl = document.getElementById(pId);
    const active = tId === activeTabId;
    if (tEl) {
      tEl.classList.toggle('is-active', active);
      tEl.setAttribute('aria-selected', active ? 'true' : 'false');
    }
    if (pEl) pEl.classList.toggle('is-active', active);
  }
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
  if (!el.sessionList) return;
  el.sessionList.innerHTML = '';
  const q = state.sessionQuery;
  const filtered = q
    ? state.sessions.filter((s) => {
        const hay = `${s.title || ''} ${s.id || ''} ${s.workspace_root || ''} ${(s.stats && s.stats.model) || ''}`.toLowerCase();
        return hay.includes(q);
      })
    : state.sessions;

  if (filtered.length === 0) {
    const empty = document.createElement('div');
    empty.style.cssText = 'padding:10px 12px;font-size:12px;color:var(--ink-muted);';
    empty.textContent = q ? 'No sessions match filter.' : 'No research sessions yet.';
    el.sessionList.appendChild(empty);
    return;
  }

  for (const s of filtered) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `session-item${s.id === state.activeSessionId ? ' is-active' : ''}`;
    const stats = s.stats || {};
    const badges = [];
    if (stats.verified_status === 'verified') {
      badges.push('<span class="mini-pill ok" title="Independently verified">✓</span>');
    } else if (stats.verified_status === 'unverified') {
      badges.push('<span class="mini-pill warn" title="Unverified draft">!</span>');
    }
    if ((Number(stats.proof_entries) || 0) > 0) {
      badges.push(`<span class="mini-pill" title="Proof-DAG steps">${Number(stats.proof_entries)}</span>`);
    }
    if (s.parent_session) {
      badges.push('<span class="mini-pill">Fork</span>');
    }
    const metaParts = [];
    if (stats.model) metaParts.push(stats.model);
    if (s.workspace_root) metaParts.push(basenameOf(s.workspace_root));
    if (metaParts.length === 0) metaParts.push(s.id);

    btn.innerHTML = `
      <div class="session-title-row">
        <span class="session-item-title">${escapeHtml(s.title || s.id)}</span>
        <span class="session-badges">${badges.join('')}</span>
      </div>
      <div class="session-item-meta"><span>${escapeHtml(metaParts.join(' · '))}</span></div>
    `;
    btn.addEventListener('click', () => selectSession(s.id));
    el.sessionList.appendChild(btn);
  }
}

function syncWorkspaceIndicators(wsRoot) {
  state.activeWorkspaceRoot = wsRoot || '';
  if (el.wsPill) {
    if (wsRoot) {
      el.wsPill.hidden = false;
      el.wsPill.textContent = wsRoot;
      el.wsPill.title = `${wsRoot} (click to switch workspace folder)`;
    } else {
      el.wsPill.hidden = true;
      el.wsPill.title = '';
    }
  }
  if (el.wsBtn) {
    if (wsRoot) {
      el.wsBtn.textContent = `Workspace: ${basenameOf(wsRoot)}`;
      el.wsBtn.setAttribute('aria-pressed', 'true');
      el.wsBtn.title = `Attached workspace: ${wsRoot} (click to switch)`;
    } else {
      el.wsBtn.textContent = 'Workspace';
      el.wsBtn.setAttribute('aria-pressed', 'false');
      el.wsBtn.title = 'Attach local workspace directory';
    }
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
  if (state.activeSessionId) return state.activeSessionId;
  return createAndSelectSession({});
}

async function selectSession(id) {
  if (!id) return;
  state.activeSessionId = id;
  renderSessionList();
  connectSessionSse(id);

  try {
    const sess = await api.getSession(id);
    if (el.sessionTitle) el.sessionTitle.textContent = sess.title || id;
    syncWorkspaceIndicators(sess.workspace_root || '');
    renderSessionStream(id, sess);
    applySession(id, sess);
    applySessionTelemetry(sess.stats, Array.isArray(sess.messages) && sess.messages.length > 0);
    setComposerBusy(false);
    const hasPending = sess.pending_approvals && sess.pending_approvals.length > 0;
    setStreamStatus(hasPending ? 'Awaiting Approval' : 'Ready', hasPending ? 'warn' : 'ok');
    await Promise.all([
      refreshWorkspaceFiles(id),
      refreshDag(id),
      refreshSourcesAndSidetalk(id),
    ]);
  } catch (err) {
    showToast(`Failed to load session ${id}: ${err.message}`);
  }
}

function connectSessionSse(id) {
  if (state.eventSource) {
    state.eventSource.close();
    state.eventSource = null;
  }
  state.eventSource = api.openSessionEvents(id, {
    text_delta: (d) => {
      setStreamStatus('Streaming...', 'accent');
      updatePipelineFromEvent('solver_stream', d);
      handleTextDelta(d);
    },
    thinking_delta: (d) => {
      updatePipelineFromEvent('thinking_delta', d);
      handleThinkingDelta(d);
    },
    round_reset: (d) => {
      setStreamStatus('Revising...', 'accent');
      updatePipelineFromEvent('round_reset', d);
      handleRoundReset();
    },
    tool_trace: (d) => {
      updatePipelineFromEvent('tool_trace', d);
      handleToolTrace(d);
    },
    approval_requested: (d) => {
      setStreamStatus('Awaiting Approval', 'warn');
      renderApprovalCard(d);
    },
    approval_resolved: (d) => {
      setStreamStatus('Running', 'accent');
      if (d) resolveApprovalCard(d.call_id, d.approved, Boolean(d.approve_for_session));
    },
    dag_updated: (d) => {
      updatePipelineFromEvent('dag_updated', d);
      refreshDag(id);
    },
    session_event: (d) => {
      updatePipelineFromEvent('session_event', d);
      if (d && d.kind === 'vendor.builtin_tool') {
        appendWarningLine(builtinToolWarning(d.data || {}));
      }
    },
    turn_complete: async (d) => {
      finalizeLiveThinking();
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
      try {
        const sess = await api.getSession(id);
        const msgs = Array.isArray(sess && sess.messages) ? sess.messages : [];
        const lastMsg = msgs.length > 0 ? msgs[msgs.length - 1] : null;
        if (d && d.text) {
          finalizeLiveAssistant(d.text, lastMsg && lastMsg.role === 'assistant' ? lastMsg : null);
          if (state.activeSessionId === id) applySession(id, sess);
        } else if (state.activeSessionId === id) {
          renderSessionStream(id, sess);
          applySession(id, sess);
        }
        finishLivePipeline('ok', sess && sess.stats);
      } catch {
        finalizeLiveAssistant((d && d.text) || '');
        finishLivePipeline('ok', null);
      }
      await Promise.all([
        refreshSessionList(),
        refreshWorkspaceFiles(id),
        refreshDag(id),
        loadSessionSources(id),
      ]);
    },
    turn_error: (d) => {
      finalizeLiveThinking();
      finalizeLiveOnError();
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
      refreshSessionRoute(id);
    },
  });
}

function builtinToolWarning(data) {
  const name = String(data.tool_name || '?').slice(0, 80);
  const outcome = data.denied_or_failed
    ? 'reported denied or failed'
    : data.state === 'DONE' ? 'reported finished' : 'reported running';
  return `Warning: the Antigravity CLI's built-in tool \`${name}\` ${outcome}. It is outside re:Search's tools, policy and approvals; re:Search only records what the CLI reported (vendor-sandboxed, not confined to re:Search tools).`;
}

async function refreshSessionRoute(id) {
  if (state.activeSessionId !== id) return;
  try {
    const sess = await api.getSession(id);
    if (state.activeSessionId === id) applySession(id, sess);
  } catch {
    // Keep dropdown state
  }
}

async function handleSendPrompt(sessionId, prompt, opts) {
  appendUserMessage(prompt);
  setComposerBusy(true);
  setStreamStatus('Thinking...', 'accent');
  startLivePipeline(opts);
  await api.sendMessage(sessionId, { prompt, ...opts });
  rememberForSession(sessionId, { provider: opts.provider, model: opts.model });
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
  try {
    const sess = await api.setSessionWorkspace(state.activeSessionId, workspaceRoot);
    syncWorkspaceIndicators((sess && sess.workspace_root) || workspaceRoot);
    refreshEmptyHeroIfVisible();
    await Promise.all([
      refreshSessionList(),
      refreshWorkspaceFiles(state.activeSessionId),
    ]);
    showToast(`Attached workspace: ${(sess && sess.workspace_root) || workspaceRoot}`);
  } catch (err) {
    showToast(`Could not attach workspace: ${err.message}`);
  }
}

function wirePostMessageBridge() {
  window.addEventListener('message', (ev) => {
    if (ev.origin && ev.origin !== window.location.origin && ev.origin !== 'null') {
      return;
    }
    const msg = ev.data;
    if (!msg || typeof msg !== 'object') return;
    if (msg.type === 're-search:select-tab' && msg.tab) {
      activateCanvasTab(msg.tab);
    } else if (msg.type === 're-search:set-prompt' && typeof msg.prompt === 'string') {
      setComposerPrompt(msg.prompt, false);
    } else if (msg.type === 're-search:set-theme' && (msg.theme === 'light' || msg.theme === 'dark')) {
      applyTheme(msg.theme);
    }
  });
}

function showToast(msg) {
  const stack = document.getElementById('toast-stack');
  if (!stack) return;
  const item = document.createElement('div');
  item.className = 'toast-item';
  item.textContent = String(msg);
  stack.appendChild(item);
  setTimeout(() => item.remove(), 3800);
}
