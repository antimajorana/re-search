// Main entry controller: wires left rail, Connect panel, model picker, center stream, SSE live updates, Canvas tabs, and postMessage bridge.
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

const state = {
  sessions: [],
  activeSessionId: null,
  eventSource: null,
  theme: 'light',
};

const el = {};

document.addEventListener('DOMContentLoaded', boot);

async function boot() {
  cacheElements();
  restoreTheme();
  wireShellControls();
  wireCanvasTabs();
  wirePostMessageBridge();
  initConnect({
    onNotify: showToast,
    // A key was saved or removed, or an address changed: re-read the model list.
    onChanged: () => loadModels(true),
  });
  initModelPicker({ onNotify: showToast, onOpenConnect: openConnect });

  initStream({
    onPromptPick: (prompt) => setComposerPrompt(prompt, true),
    onNotify: showToast,
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
    onSelectStepRef: (stepId) => {
      activateCanvasTab('tab-dag');
      selectNodeById(stepId);
    },
  });

  const launchToken = await api.initAuth();
  if (!launchToken) {
    // Opened without the launch token (new tab, bookmark, typed URL): every API call would be
    // refused. Say so instead of showing an app that looks empty.
    document.body.innerHTML =
      '<div style="max-width:34rem;margin:18vh auto;padding:0 1.5rem;font:16px/1.5 system-ui,sans-serif">' +
      '<h1 style="font-size:1.4rem">This tab has no launch token</h1>' +
      '<p>re:Search only talks to the tab it opened. Close this tab and start re:Search again ' +
      '(<code>re-search desktop</code> or the Start menu shortcut), or switch back to the window it opened.</p></div>';
    return;
  }
  await checkHealth();
  await loadModels(false);
  await refreshSessionList();

  if (state.sessions.length > 0) {
    await selectSession(state.sessions[0].id);
  } else {
    renderSessionStream(null, { messages: [] });
  }
}

function cacheElements() {
  el.shell = document.getElementById('app-shell');
  el.railToggle = document.getElementById('rail-toggle');
  el.canvasToggle = document.getElementById('canvas-toggle');
  el.newBtn = document.getElementById('new-research-btn');
  el.sessionList = document.getElementById('session-list');
  el.sessionTitle = document.getElementById('active-session-title');
  el.wsPill = document.getElementById('active-workspace-pill');
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
}

function wireCanvasTabs() {
  const tabs = [
    ['tab-latex', 'panel-latex'],
    ['tab-dag', 'panel-dag'],
    ['tab-sources', 'panel-sources'],
    ['tab-sidetalk', 'panel-sidetalk'],
  ];
  for (const [tabId] of tabs) {
    const btn = document.getElementById(tabId);
    if (btn) {
      btn.addEventListener('click', () => activateCanvasTab(tabId));
    }
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
  for (const s of state.sessions) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `session-item${s.id === state.activeSessionId ? ' is-active' : ''}`;
    const forkTag = s.parent_session ? '<span class="badge">Fork</span>' : '';
    btn.innerHTML = `
      <div class="session-title-row">
        <span class="session-item-title">${escapeHtml(s.title || s.id)}</span>
        ${forkTag}
      </div>
      <div class="session-item-meta"><span>${escapeHtml(s.id)}</span></div>
    `;
    btn.addEventListener('click', () => selectSession(s.id));
    el.sessionList.appendChild(btn);
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
    if (el.wsPill) {
      if (sess.workspace_root) {
        el.wsPill.hidden = false;
        el.wsPill.textContent = sess.workspace_root;
        el.wsPill.title = sess.workspace_root;
      } else {
        el.wsPill.hidden = true;
        el.wsPill.title = '';
      }
    }
    renderSessionStream(id, sess);
    applySession(id, sess);
    setComposerBusy(false);
    setStreamStatus(sess.pending_approvals && sess.pending_approvals.length > 0 ? 'Awaiting Approval' : 'Ready', sess.pending_approvals && sess.pending_approvals.length > 0 ? 'warn' : 'ok');
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
      handleTextDelta(d);
    },
    thinking_delta: (d) => handleThinkingDelta(d),
    round_reset: () => {
      setStreamStatus('Revising...', 'accent');
      handleRoundReset();
    },
    tool_trace: (d) => handleToolTrace(d),
    approval_requested: (d) => {
      setStreamStatus('Awaiting Approval', 'warn');
      renderApprovalCard(d);
    },
    approval_resolved: (d) => {
      setStreamStatus('Running', 'accent');
      if (d) resolveApprovalCard(d.call_id, d.approved, Boolean(d.approve_for_session));
    },
    dag_updated: () => refreshDag(id),
    session_event: (d) => {
      if (d && d.kind === 'vendor.builtin_tool') {
        appendWarningLine(builtinToolWarning(d.data || {}));
      }
    },
    turn_complete: async (d) => {
      finalizeLiveThinking();
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
      if (d && d.text) {
        finalizeLiveAssistant(d.text);
        await refreshSessionRoute(id);
      } else {
        finalizeLiveAssistant('');
        await selectSession(id);
      }
      await refreshWorkspaceFiles(id);
      await refreshDag(id);
    },
    turn_error: (d) => {
      finalizeLiveThinking();
      finalizeLiveOnError();
      setComposerBusy(false);
      const rawErr = String((d && (d.error || d.message)) || 'unknown failure');
      if (/stopped/i.test(rawErr)) {
        setStreamStatus('Stopped', 'warn');
        showToast('Turn stopped.');
      } else {
        setStreamStatus('Error', 'err');
        showToast(`Turn error: ${rawErr}`);
      }
      refreshSessionRoute(id);
    },
  });
}

// Same wording as the server's transcript view (turn_runner::builtin_tool_warning).
function builtinToolWarning(data) {
  const name = String(data.tool_name || '?').slice(0, 80);
  const outcome = data.denied_or_failed
    ? 'reported denied or failed'
    : data.state === 'DONE' ? 'reported finished' : 'reported running';
  return `Warning: the Antigravity CLI's built-in tool \`${name}\` ${outcome}. It is outside re:Search's tools, policy and approvals; re:Search only records what the CLI reported (vendor-sandboxed, not confined to re:Search tools).`;
}

// After a turn the session may now be bound to a route (or, after a turn refused before
// it started, still unbound): let the model dropdown follow the server's answer.
async function refreshSessionRoute(id) {
  if (state.activeSessionId !== id) return;
  try {
    const sess = await api.getSession(id);
    if (state.activeSessionId === id) applySession(id, sess);
  } catch {
    // Keep the dropdown as it is; the server still refuses a send on the wrong route.
  }
}

async function handleSendPrompt(sessionId, prompt, opts) {
  appendUserMessage(prompt);
  setComposerBusy(true);
  setStreamStatus('Thinking...', 'accent');
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
    if (el.wsPill) {
      if (sess && sess.workspace_root) {
        el.wsPill.hidden = false;
        el.wsPill.textContent = sess.workspace_root;
        el.wsPill.title = sess.workspace_root;
      } else {
        el.wsPill.hidden = true;
        el.wsPill.title = '';
      }
    }
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
