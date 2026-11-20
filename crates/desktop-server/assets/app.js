// Main entry controller: wires left rail, settings modal, center stream, SSE live updates, Canvas tabs, and postMessage bridge.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';
import {
  appendAssistantMessage,
  appendUserMessage,
  finalizeLiveThinking,
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
  setComposerProviderModel,
} from '/assets/composer.js';
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
  wireSettingsModal();
  wirePostMessageBridge();

  initStream({
    onPromptPick: (prompt) => setComposerPrompt(prompt, true),
    onNotify: showToast,
  });

  initComposer({
    ensureSession: ensureActiveSession,
    onSendPrompt: handleSendPrompt,
    onAttachWorkspace: handleAttachWorkspace,
    onSourcesUpdated: (sid) => loadSessionSources(sid),
    onNotify: showToast,
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
  await checkHealthAndSettings();
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
  el.settingsBtn = document.getElementById('settings-btn');
  el.settingsDialog = document.getElementById('settings-dialog');
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

async function checkHealthAndSettings() {
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
  try {
    const settings = await api.getProviderSettings();
    if (settings) {
      setComposerProviderModel(settings.default_provider || settings.provider, settings.default_model || settings.model);
      populateSettingsForm(settings);
    }
  } catch {
    // Default composer provider remains selected
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
      } else {
        el.wsPill.hidden = true;
      }
    }
    renderSessionStream(id, sess);
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
    thinking_delta: (d) => handleThinkingDelta(d),
    tool_trace: (d) => handleToolTrace(d),
    approval_requested: (d) => {
      setStreamStatus('Awaiting Approval', 'warn');
      renderApprovalCard(d);
    },
    approval_resolved: (d) => {
      setStreamStatus('Running', 'accent');
      if (d) resolveApprovalCard(d.call_id, d.approved);
    },
    dag_updated: () => refreshDag(id),
    session_event: (d) => {
      if (d && d.kind === 'model.response' && d.text) {
        appendAssistantMessage(d.text);
      }
    },
    turn_complete: async (d) => {
      finalizeLiveThinking();
      setComposerBusy(false);
      setStreamStatus('Ready', 'ok');
      if (d && d.text) {
        appendAssistantMessage(d.text);
      } else {
        await selectSession(id);
      }
      await refreshWorkspaceFiles(id);
      await refreshDag(id);
    },
    turn_error: (d) => {
      finalizeLiveThinking();
      setComposerBusy(false);
      setStreamStatus('Error', 'err');
      showToast(`Turn error: ${(d && (d.error || d.message)) || 'unknown failure'}`);
    },
  });
}

async function handleSendPrompt(sessionId, prompt, opts) {
  appendUserMessage(prompt);
  setStreamStatus('Thinking...', 'accent');
  await api.sendMessage(sessionId, { prompt, ...opts });
}

async function handleAttachWorkspace(workspaceRoot) {
  if (!state.activeSessionId) {
    await createAndSelectSession({ workspace_root: workspaceRoot });
    return;
  }
  await createAndSelectSession({ title: 'Workspace Research', workspace_root: workspaceRoot });
  showToast(`Attached workspace: ${workspaceRoot}`);
}

function wireSettingsModal() {
  const closeBtn = document.getElementById('settings-close-btn');
  const cancelBtn = document.getElementById('settings-cancel-btn');
  const form = document.getElementById('settings-form');
  const close = () => { if (el.settingsDialog) el.settingsDialog.hidden = true; };

  if (el.settingsBtn) {
    el.settingsBtn.addEventListener('click', async () => {
      if (el.settingsDialog) el.settingsDialog.hidden = false;
      try {
        const s = await api.getProviderSettings();
        if (s) populateSettingsForm(s);
      } catch {
        // Use current field state
      }
    });
  }
  if (closeBtn) closeBtn.addEventListener('click', close);
  if (cancelBtn) cancelBtn.addEventListener('click', close);

  if (form) {
    form.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      const payload = {
        default_provider: document.getElementById('set-provider').value,
        default_model: document.getElementById('set-model').value.trim(),
        pinned_conventions: document.getElementById('set-conventions').value,
        role_presets: {
          step_verification: document.getElementById('set-role-verifier').value.trim() || undefined,
          research_strategist: document.getElementById('set-role-strategist').value.trim() || undefined,
        },
        keys: {},
      };
      const kAnth = document.getElementById('set-key-anthropic').value.trim();
      const kOpen = document.getElementById('set-key-openai').value.trim();
      const kGem = document.getElementById('set-key-gemini').value.trim();
      const kOll = document.getElementById('set-key-ollama').value.trim();
      if (kAnth) payload.keys.anthropic = kAnth;
      if (kOpen) payload.keys.openai = kOpen;
      if (kGem) payload.keys.gemini = kGem;
      if (kOll) {
        if (kOll.startsWith('http://') || kOll.startsWith('https://')) {
          payload.base_urls = { ollama: kOll };
        } else {
          payload.keys.ollama = kOll;
        }
      }

      try {
        await api.updateProviderSettings(payload);
        setComposerProviderModel(payload.default_provider, payload.default_model);
        close();
        showToast('Provider settings saved.');
      } catch (err) {
        showToast(`Failed to save settings: ${err.message}`);
      }
    });
  }
}

function populateSettingsForm(s) {
  const setVal = (id, val) => {
    const node = document.getElementById(id);
    if (node && val !== undefined && val !== null) node.value = val;
  };
  setVal('set-provider', s.default_provider || s.provider || 'anthropic');
  setVal('set-model', s.default_model || s.model || '');
  setVal('set-conventions', s.pinned_conventions || '');
  if (s.role_presets) {
    setVal('set-role-verifier', s.role_presets.step_verification || '');
    setVal('set-role-strategist', s.role_presets.research_strategist || '');
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
