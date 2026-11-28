// The Connect panel: one row per provider from GET /api/connections, with a green
// "Connected" badge or a single action (an API key field + Save, browser sign-in, or CLI setup).
// Base URLs, auto-approval toggle and pinned conventions sit behind "Advanced".
// Every server string is set as textContent, never as HTML.
import * as api from '/assets/api.js';

let dialogEl = null;
let statusEl = null;
let listEl = null;
let baseUrlsEl = null;
let callbacks = {};
let lastFocus = null;
let connections = [];
let signinPoll = null;
let activeTab = 'providers';

function make(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function initConnect(opts = {}) {
  callbacks = opts;
  dialogEl = document.getElementById('settings-dialog');
  statusEl = document.getElementById('settings-status');
  listEl = document.getElementById('connect-list');
  baseUrlsEl = document.getElementById('advanced-base-urls');
  const openBtn = document.getElementById('settings-btn');
  const closeBtn = document.getElementById('settings-close-btn');
  const form = document.getElementById('settings-form');
  const refreshModelsBtn = document.getElementById('refresh-models-btn');
  if (openBtn) openBtn.addEventListener('click', () => openConnect('providers'));
  if (closeBtn) closeBtn.addEventListener('click', closeConnect);
  const tabs = Array.from(document.querySelectorAll('[data-settings-tab]'));
  for (const btn of tabs) {
    btn.addEventListener('click', () => switchSettingsTab(btn.getAttribute('data-settings-tab')));
  }
  wireTabKeys(tabs);
  if (refreshModelsBtn) {
    refreshModelsBtn.addEventListener('click', async () => {
      refreshModelsBtn.disabled = true;
      try {
        if (typeof callbacks.onRefreshModels === 'function') await callbacks.onRefreshModels();
        notify('Model lists refreshed from connected providers.');
      } finally {
        refreshModelsBtn.disabled = false;
      }
    });
  }
  if (dialogEl) {
    dialogEl.addEventListener('close', () => {
      if (dialogEl.open) return;
      const summary = lastFocus?.closest('#research-options')?.querySelector('summary');
      for (const node of [lastFocus, summary, openBtn, document.getElementById('composer-input')]) {
        if (!node) continue;
        node.focus();
        if (document.activeElement === node) break;
      }
      lastFocus = null;
    });
    dialogEl.addEventListener('click', (ev) => {
      if (ev.target === dialogEl) closeConnect();
    });
  }
  if (form) form.addEventListener('submit', saveAdvanced);
  const autoEl = document.getElementById('set-auto-approve-tools');
  if (autoEl) {
    autoEl.addEventListener('change', async () => {
      const checked = autoEl.checked;
      autoEl.disabled = true;
      try {
        await api.updateProviderSettings({ auto_approve_tools: checked });
        notify(
          checked
            ? 'Auto-approve enabled for SymPy, workspace writes & literature lookups.'
            : 'Manual approval required for SymPy, workspace writes & literature lookups.'
        );
      } catch (err) {
        autoEl.checked = !checked;
        notify(`Could not update tool approval setting: ${err.message}`);
      } finally {
        autoEl.disabled = false;
      }
    });
  }
}

export function switchSettingsTab(tab = 'providers') {
  activeTab = tab;
  for (const btn of document.querySelectorAll('[data-settings-tab]')) {
    const match = btn.getAttribute('data-settings-tab') === tab;
    btn.classList.toggle('is-active', match);
    btn.setAttribute('aria-selected', match ? 'true' : 'false');
    btn.tabIndex = match ? 0 : -1;
  }
  for (const panel of document.querySelectorAll('[data-settings-panel]')) {
    panel.classList.toggle('is-active', panel.getAttribute('data-settings-panel') === tab);
  }
  if (typeof callbacks.onTabOpened === 'function') callbacks.onTabOpened(tab);
  globalThis.__reSearchUsageTracker?.onSettingsTab?.(tab);
}

export function wireTabKeys(tabs) {
  tabs.forEach((tab, index) => {
    tab.addEventListener('keydown', (ev) => {
      let next;
      switch (ev.key) {
        case 'ArrowLeft': next = (index + tabs.length - 1) % tabs.length; break;
        case 'ArrowRight': next = (index + 1) % tabs.length; break;
        case 'Home': next = 0; break;
        case 'End': next = tabs.length - 1; break;
        default: return;
      }
      ev.preventDefault();
      tabs[next].click();
      tabs[next].focus();
    });
  });
}

export async function openConnect(tab = 'providers') {
  if (!dialogEl) return;
  if (!dialogEl.open) {
    lastFocus = document.activeElement;
    if (statusEl) statusEl.textContent = '';
    dialogEl.showModal();
  }
  switchSettingsTab(tab || activeTab || 'providers');
  const closeBtn = document.getElementById('settings-close-btn');
  if (closeBtn) closeBtn.focus();
  await Promise.all([refreshConnections(true), loadAdvanced()]);
}

function closeConnect() {
  dialogEl?.close();
}

async function refreshConnections(refresh) {
  if (!listEl) return;
  if (connections.length === 0) listEl.replaceChildren(make('div', 'connect-status', 'Checking...'));
  try {
    const data = await api.getConnections(refresh);
    connections = Array.isArray(data && data.connections) ? data.connections : [];
  } catch (err) {
    listEl.replaceChildren(make('div', 'connect-status', `Could not check connections: ${err.message}`));
    return;
  }
  listEl.replaceChildren(...connections.map(renderRow));
  renderBaseUrls();
}

function renderRow(c) {
  const row = make('section', 'connect-row');
  row.setAttribute('aria-label', String(c.label || c.id));
  const head = make('div', 'connect-row-head');
  head.append(make('span', 'connect-label', c.label || c.id));
  head.append(make('span', c.connected ? 'badge ok' : 'badge', c.connected ? 'Connected' : 'Not connected'));
  row.append(head);
  if (c.status) row.append(make('div', 'connect-status', c.status));
  if (!c.connected && c.how_to) row.append(make('div', 'connect-howto', c.how_to));

  if (c.warning) renderConsent(c, row);
  if (c.action) return renderActionRow(c, row);
  const needsKey = c.accepts_key && c.key_required !== false;
  if (needsKey && !c.connected) row.append(keyForm(c));
  const actions = make('div', 'connect-actions');
  if (!needsKey && !c.connected) {
    // Nothing to type: the person starts the program (or installs it) and checks again.
    const again = make('button', 'pill-btn sm', 'Check again');
    again.type = 'button';
    again.addEventListener('click', async () => {
      await refreshConnections(true);
      changed();
    });
    actions.append(again);
  }
  if (c.has_saved_key) {
    const remove = make('button', 'pill-btn sm', 'Remove saved key');
    remove.type = 'button';
    remove.addEventListener('click', () => removeKey(c));
    actions.append(remove);
  }
  if (actions.childElementCount > 0) row.append(actions);
  return row;
}

function renderConsent(c, row) {
  const box = make('div', 'connect-warning');
  box.setAttribute('role', 'note');
  box.append(make('strong', '', c.acknowledged ? 'Acknowledged notice' : 'Read before enabling'));
  box.append(make('p', '', c.warning));
  const action = c.acknowledged ? c.withdraw_action : c.acknowledge_action;
  if (action) {
    const label = c.acknowledged ? 'Withdraw acknowledgement' : 'I understand the risk';
    const button = make('button', 'pill-btn sm', label);
    button.type = 'button';
    button.addEventListener('click', async () => {
      if (!c.acknowledged && !window.confirm(`${c.warning}\n\nEnable ${c.label || c.id}?`)) return;
      button.disabled = true;
      try {
        await api.connectionAction(c.id, action);
      } catch (err) {
        notify(`Could not update acknowledgement: ${err.message}`);
        button.disabled = false;
        return;
      }
      notify(c.acknowledged ? 'Antigravity access withdrawn.' : 'Antigravity acknowledgement saved.');
      await refreshConnections(false);
      changed();
    });
    const actions = make('div', 'connect-actions');
    actions.append(button);
    box.append(actions);
  }
  row.append(box);
}

// A provider connected by signing in (data-driven: `action` is "signin" or "signout").
function renderActionRow(c, row) {
  if ((c.connected || c.can_signout) && c.account) row.append(make('div', 'connect-status', `Account: ${c.account}`));
  if (c.connected && c.usage_url) row.append(make('div', 'connect-howto', `Manage usage: ${c.usage_url}`));
  const box = make('div', 'connect-signin');
  const actions = make('div', 'connect-actions');
  let signinBtn = null;
  if (c.action === 'signin' && c.action_supported !== false) {
    signinBtn = make('button', 'pill-btn sm primary', c.action_label || 'Sign in');
    signinBtn.type = 'button';
    signinBtn.addEventListener('click', () => startSignin(c, box, signinBtn));
    actions.append(signinBtn);
  }
  if (c.action === 'signout' || (c.action === 'signin' && c.can_signout)) {
    const outBtn = make('button', 'pill-btn sm', c.action === 'signout' ? (c.action_label || 'Sign out') : 'Sign out');
    outBtn.type = 'button';
    outBtn.addEventListener('click', () => signOut(c));
    actions.append(outBtn);
  }
  if (actions.childElementCount > 0) row.append(actions);
  row.append(box);
  if (c.login && c.login.state === 'pending' && c.pending_authorize_url) {
    if (signinBtn) signinBtn.disabled = true;
    renderPendingSignin(c, box, String(c.pending_authorize_url), c.login.message || 'Waiting for the browser sign-in.', 300);
  }
  return row;
}

function renderPendingSignin(c, box, url, message, seconds) {
  const status = make('div', 'connect-status', message || 'Waiting for the browser sign-in.');
  const label = make('label', 'connect-howto', 'Sign-in link (open it in your browser):');
  const field = make('input', 'text-input');
  field.type = 'text';
  field.readOnly = true;
  field.spellcheck = false;
  field.value = url;
  field.id = `signin-url-${String(c.id).replace(/[^a-z0-9_-]/gi, '')}`;
  label.htmlFor = field.id;
  const open = make('button', 'pill-btn sm primary', 'Open in browser');
  open.type = 'button';
  open.addEventListener('click', async () => {
    try {
      await api.connectionAction(c.id, 'open');
    } catch (err) {
      notify(`Could not open the browser: ${err.message}. Use Copy link.`);
    }
  });
  const copy = make('button', 'pill-btn sm', 'Copy link');
  copy.type = 'button';
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(url);
      notify('Sign-in link copied.');
    } catch {
      field.focus();
      field.select();
      notify('Select the link and copy it.');
    }
  });
  const buttons = make('div', 'connect-actions');
  buttons.append(open, copy);
  box.replaceChildren(label, field, buttons, status);
  pollSignin(c, status, Number(seconds) || 300);
}

async function startSignin(c, box, btn) {
  btn.disabled = true;
  let start;
  try {
    start = await api.connectionAction(c.id, 'login');
  } catch (err) {
    btn.disabled = false;
    notify(`Could not start the sign-in: ${err.message}`);
    return;
  }
  renderPendingSignin(c, box, String((start && start.authorize_url) || ''), start && start.message, start && start.expires_in_s);
}

function pollSignin(c, statusEl, seconds) {
  if (signinPoll) clearInterval(signinPoll);
  const deadline = Date.now() + (seconds + 5) * 1000;
  signinPoll = setInterval(async () => {
    let s = null;
    try {
      s = await api.connectionAction(c.id, 'login', 'GET');
    } catch {
      return;
    }
    if (s && s.message) statusEl.textContent = String(s.message);
    if ((s && s.state !== 'pending') || Date.now() > deadline) {
      clearInterval(signinPoll);
      signinPoll = null;
      if (s && s.state === 'succeeded' && s.connected) {
        notify("You're using your ChatGPT plan. Eligible requests in re:Search use your ChatGPT plan; manage usage in ChatGPT settings.");
      }
      await refreshConnections(false);
      changed();
    }
  }, 2000);
}

async function signOut(c) {
  if (!window.confirm(`Sign out of ${c.label || c.id} on this computer?`)) return;
  try {
    const out = await api.connectionAction(c.id, 'logout');
    notify((out && out.message) || 'Signed out.');
  } catch (err) {
    notify(`Could not sign out: ${err.message}`);
  }
  await refreshConnections(false);
  changed();
}

function keyForm(c) {
  const form = make('form', 'connect-key-form');
  form.autocomplete = 'off';
  const inputId = `connect-key-${String(c.id).replace(/[^a-z0-9_-]/gi, '')}`;
  const label = make('label', 'sr-only', `${c.label || c.id} API key`);
  label.htmlFor = inputId;
  const input = make('input', 'text-input');
  input.id = inputId;
  input.type = 'password';
  input.autocomplete = 'off';
  input.spellcheck = false;
  input.placeholder = 'Paste API key';
  const save = make('button', 'pill-btn sm primary', 'Save');
  save.type = 'submit';
  form.append(label, input, save);
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const key = input.value.trim();
    if (!key) {
      input.focus();
      return;
    }
    save.disabled = true;
    try {
      await api.updateProviderSettings({ keys: { [c.id]: key } });
      input.value = '';
      notify(`${c.label || c.id}: key saved.`);
      await refreshConnections(false);
      changed();
    } catch (err) {
      notify(`Could not save the key: ${err.message}`);
    } finally {
      input.value = '';
      save.disabled = false;
    }
  });
  return form;
}

async function removeKey(c) {
  if (!window.confirm(`Remove the saved ${c.label || c.id} key from this computer?`)) return;
  try {
    await api.updateProviderSettings({ keys: { [c.id]: null } });
    notify(`${c.label || c.id}: saved key removed.`);
    await refreshConnections(false);
    changed();
  } catch (err) {
    notify(`Could not remove the key: ${err.message}`);
  }
}

// Advanced: one base-URL field per API provider (empty = default), set at launch = read-only.
function renderBaseUrls() {
  if (!baseUrlsEl) return;
  const rows = connections
    .filter((c) => c.kind === 'api-key')
    .map((c) => {
      const row = make('div', 'form-row');
      const id = `base-url-${String(c.id).replace(/[^a-z0-9_-]/gi, '')}`;
      const label = make('label', '', `${c.label || c.id} address`);
      label.htmlFor = id;
      const input = make('input', 'text-input');
      input.id = id;
      input.type = 'text';
      input.spellcheck = false;
      input.dataset.provider = String(c.id);
      input.placeholder = String(c.default_base_url || '');
      const fromLaunch = c.base_url_source === 'launch';
      input.value = c.base_url_source === 'saved' || fromLaunch ? String(c.base_url || '') : '';
      input.dataset.initial = input.value;
      input.disabled = fromLaunch;
      if (fromLaunch) input.title = 'Set when re:Search was started (--api-base)';
      row.append(label, input);
      return row;
    });
  baseUrlsEl.replaceChildren(...rows);
}

async function loadAdvanced() {
  try {
    const s = await api.getProviderSettings();
    const set = (id, val) => {
      const node = document.getElementById(id);
      if (node) node.value = val === undefined || val === null ? '' : String(val);
    };
    set('set-conventions', s && s.pinned_conventions);
    const autoEl = document.getElementById('set-auto-approve-tools');
    if (autoEl) autoEl.checked = s ? s.auto_approve_tools !== false : true;
  } catch {
    // Keep the fields as they are.
  }
}

async function saveAdvanced(ev) {
  ev.preventDefault();
  const autoEl = document.getElementById('set-auto-approve-tools');
  const payload = {
    pinned_conventions: document.getElementById('set-conventions').value,
    auto_approve_tools: autoEl ? autoEl.checked : true,
  };
  const bases = {};
  for (const input of baseUrlsEl ? baseUrlsEl.querySelectorAll('input[data-provider]') : []) {
    if (input.disabled || input.value.trim() === input.dataset.initial) continue;
    bases[input.dataset.provider] = input.value.trim() || null;
  }
  if (Object.keys(bases).length > 0) payload.base_urls = bases;
  try {
    await api.updateProviderSettings(payload);
    notify('Advanced settings saved.');
    await refreshConnections(true);
    changed();
  } catch (err) {
    notify(`Could not save: ${err.message}`);
  }
}

function changed() {
  if (typeof callbacks.onChanged === 'function') callbacks.onChanged();
}

function notify(msg) {
  if (dialogEl?.open && statusEl) statusEl.textContent = msg;
  else if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
