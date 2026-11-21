// The Connect panel: one row per provider from GET /api/connections, with a green
// "Connected" badge or a single action (an API key field + Save, or the how-to for the
// subscription CLI). Keys are sent once with PUT /api/settings/providers and never shown
// again. Base URLs, role presets and pinned conventions sit behind "Advanced".
// Every server string is set as textContent, never as HTML.
import * as api from '/assets/api.js';

let dialogEl = null;
let listEl = null;
let baseUrlsEl = null;
let callbacks = {};
let lastFocus = null;
let connections = [];

function make(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function initConnect(opts = {}) {
  callbacks = opts;
  dialogEl = document.getElementById('settings-dialog');
  listEl = document.getElementById('connect-list');
  baseUrlsEl = document.getElementById('advanced-base-urls');
  const openBtn = document.getElementById('settings-btn');
  const closeBtn = document.getElementById('settings-close-btn');
  const form = document.getElementById('settings-form');
  if (openBtn) openBtn.addEventListener('click', () => openConnect());
  if (closeBtn) closeBtn.addEventListener('click', closeConnect);
  if (dialogEl) {
    dialogEl.addEventListener('keydown', (ev) => {
      if (ev.key === 'Escape') closeConnect();
    });
    dialogEl.addEventListener('click', (ev) => {
      if (ev.target === dialogEl) closeConnect();
    });
  }
  if (form) form.addEventListener('submit', saveAdvanced);
}

export async function openConnect() {
  if (!dialogEl) return;
  lastFocus = document.activeElement;
  dialogEl.hidden = false;
  const closeBtn = document.getElementById('settings-close-btn');
  if (closeBtn) closeBtn.focus();
  await Promise.all([refreshConnections(true), loadAdvanced()]);
}

function closeConnect() {
  if (!dialogEl) return;
  dialogEl.hidden = true;
  if (lastFocus && typeof lastFocus.focus === 'function') lastFocus.focus();
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
    const presets = (s && s.role_presets) || {};
    set('set-role-verifier', presets.step_verification);
    set('set-role-strategist', presets.research_strategist);
  } catch {
    // Keep the fields as they are.
  }
}

async function saveAdvanced(ev) {
  ev.preventDefault();
  const payload = {
    pinned_conventions: document.getElementById('set-conventions').value,
    role_presets: {
      step_verification: document.getElementById('set-role-verifier').value.trim() || undefined,
      research_strategist: document.getElementById('set-role-strategist').value.trim() || undefined,
    },
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
  if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
