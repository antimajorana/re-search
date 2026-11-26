// Shared run status bar across Investigate, Explore and Solve (overview 2.6, 3.2 row 22, 3.3).
// Displays one unified seven-state vocabulary, elapsed time, model-call budget, and Stop/Continue/Retry.
import { createBadge } from './ui_badge.js';

// Named constants for owner-accepted wording decisions (overview 3.2 row 22 & 3.3).
export const MODEL_CALL_SINGULAR = 'model call';
export const MODEL_CALL_PLURAL = 'model calls';
export const RUN_ACTION_STOP = 'Stop';
export const RUN_ACTION_CONTINUE = 'Continue';
export const RUN_ACTION_RETRY = 'Retry';

// Overview 3.3: the seven canonical run states shown on screen across all modes.
export const RUN_STATES = Object.freeze({
  running: Object.freeze({
    key: 'running',
    label: 'Running',
    icon: 'running',
    tone: 'info',
    canStop: true,
    canContinue: false,
    canRetry: false,
  }),
  waiting: Object.freeze({
    key: 'waiting',
    label: 'Waiting for you',
    icon: 'warn',
    tone: 'warn',
    canStop: true,
    canContinue: false,
    canRetry: false,
  }),
  paused: Object.freeze({
    key: 'paused',
    label: 'Paused',
    icon: 'pause',
    tone: 'warn',
    canStop: false,
    canContinue: true,
    canRetry: false,
  }),
  interrupted: Object.freeze({
    key: 'interrupted',
    label: 'Interrupted',
    icon: 'warn',
    tone: 'warn',
    canStop: false,
    canContinue: false,
    canRetry: true,
  }),
  failed: Object.freeze({
    key: 'failed',
    label: 'Failed',
    icon: 'error',
    tone: 'err',
    canStop: false,
    canContinue: false,
    canRetry: true,
  }),
  done: Object.freeze({
    key: 'done',
    label: 'Done',
    icon: 'check',
    tone: 'ok',
    canStop: false,
    canContinue: false,
    canRetry: false,
  }),
  done_open_items: Object.freeze({
    key: 'done_open_items',
    label: 'Done with open items',
    icon: 'warn',
    tone: 'warn',
    canStop: false,
    canContinue: false,
    canRetry: false,
  }),
});

const STATE_ALIASES = Object.freeze({
  running: 'running',
  requested: 'running',
  leg_running: 'running',
  waiting: 'waiting',
  waiting_for_you: 'waiting',
  approval: 'waiting',
  plan_approval: 'waiting',
  approval_needed: 'waiting',
  paused: 'paused',
  partial: 'paused',
  stopped: 'paused',
  stopped_at_cap: 'paused',
  cancelled: 'paused',
  canceled: 'paused',
  interrupted: 'interrupted',
  transport_error: 'interrupted',
  failed: 'failed',
  error: 'failed',
  done: 'done',
  finished: 'done',
  done_open_items: 'done_open_items',
  finished_open_items: 'done_open_items',
});

export function normalizeRunState(rawState) {
  const clean = String(rawState || '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  const key = STATE_ALIASES[clean] || 'paused';
  return RUN_STATES[key];
}

export function formatElapsed(elapsed, { unit = 'ms' } = {}) {
  if (elapsed == null || !Number.isFinite(Number(elapsed))) return '';
  const totalSeconds = Math.max(0, Math.floor(unit === 's' ? Number(elapsed) : Number(elapsed) / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const mm = String(minutes).padStart(2, '0');
  const ss = String(seconds).padStart(2, '0');
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}

// Overview 2.6 & 3.2 row 22: "7 of at most 24 model calls".
export function formatModelCallBudget(used, max) {
  const u = Math.max(0, Math.floor(Number(used) || 0));
  if (max != null && Number.isFinite(Number(max)) && Number(max) > 0) {
    const m = Math.max(1, Math.floor(Number(max)));
    const noun = m === 1 ? MODEL_CALL_SINGULAR : MODEL_CALL_PLURAL;
    return `${u} of at most ${m} ${noun}`;
  }
  const noun = u === 1 ? MODEL_CALL_SINGULAR : MODEL_CALL_PLURAL;
  return `${u} ${noun}`;
}

export function formatRunStateLabel(stateObj = {}) {
  const def = normalizeRunState(stateObj.state);
  const reason = String(stateObj.reason ?? '').trim();
  if (def.key === 'paused' && reason) return `${def.label}: ${reason}`;
  if (def.key === 'failed' && reason) return `${def.label}: ${reason}`;
  return def.label;
}

export function formatRunSummary(stateObj = {}) {
  const parts = [];
  parts.push(formatRunStateLabel(stateObj));
  const detail = String(stateObj.detail ?? '').trim();
  if (detail) parts.push(detail);
  for (const extra of Array.isArray(stateObj.extras) ? stateObj.extras : []) {
    const clean = String(extra ?? '').trim();
    if (clean) parts.push(clean);
  }
  const elapsedText = stateObj.elapsedSec != null
    ? formatElapsed(stateObj.elapsedSec, { unit: 's' })
    : (stateObj.elapsedMs != null ? formatElapsed(stateObj.elapsedMs, { unit: 'ms' }) : '');
  if (elapsedText) parts.push(elapsedText);
  if (stateObj.modelCallsUsed != null || stateObj.modelCallsMax != null) {
    parts.push(formatModelCallBudget(stateObj.modelCallsUsed, stateObj.modelCallsMax));
  }
  return parts.join(' · ');
}

function makeActionButton(label, actionName, handler, disabled) {
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = `pill-btn sm ui-run-bar-btn ui-run-bar-btn--${actionName}`;
  btn.setAttribute('data-run-action', actionName);
  btn.textContent = label;
  btn.disabled = Boolean(disabled);
  btn.addEventListener('click', (ev) => handler(ev));
  return btn;
}

export function renderRunBar(stateObj = {}, callbacks = {}) {
  const def = normalizeRunState(stateObj.state);
  const bar = document.createElement('div');
  bar.className = `ui-run-bar ui-run-bar--${def.key}`;
  bar.setAttribute('role', 'status');
  bar.setAttribute('aria-live', 'polite');
  bar.setAttribute('data-run-state', def.key);

  const primary = document.createElement('div');
  primary.className = 'ui-run-bar-main';

  const badgeText = formatRunStateLabel(stateObj);
  primary.appendChild(createBadge({
    text: badgeText,
    icon: def.icon,
    tone: def.tone,
  }));

  const detail = String(stateObj.detail ?? '').trim();
  if (detail) {
    const detailEl = document.createElement('span');
    detailEl.className = 'ui-run-bar-detail';
    detailEl.textContent = detail;
    primary.appendChild(detailEl);
  }

  const extras = Array.isArray(stateObj.extras) ? stateObj.extras : [];
  for (const extra of extras) {
    const clean = String(extra ?? '').trim();
    if (!clean) continue;
    const item = document.createElement('span');
    item.className = 'ui-run-bar-extra';
    item.textContent = clean;
    primary.appendChild(item);
  }

  const elapsedText = stateObj.elapsedSec != null
    ? formatElapsed(stateObj.elapsedSec, { unit: 's' })
    : (stateObj.elapsedMs != null ? formatElapsed(stateObj.elapsedMs, { unit: 'ms' }) : '');
  if (elapsedText) {
    const elapsedEl = document.createElement('span');
    elapsedEl.className = 'ui-run-bar-elapsed';
    elapsedEl.setAttribute('data-run-field', 'elapsed');
    elapsedEl.textContent = elapsedText;
    primary.appendChild(elapsedEl);
  }

  if (stateObj.modelCallsUsed != null || stateObj.modelCallsMax != null) {
    const budgetEl = document.createElement('span');
    budgetEl.className = 'ui-run-bar-budget';
    budgetEl.setAttribute('data-run-field', 'budget');
    budgetEl.textContent = formatModelCallBudget(stateObj.modelCallsUsed, stateObj.modelCallsMax);
    primary.appendChild(budgetEl);
  }

  bar.appendChild(primary);

  const actions = document.createElement('div');
  actions.className = 'ui-run-bar-actions';
  if (def.canStop && typeof callbacks.onStop === 'function') {
    actions.appendChild(makeActionButton(RUN_ACTION_STOP, 'stop', callbacks.onStop, stateObj.busy));
  }
  if (def.canContinue && typeof callbacks.onContinue === 'function') {
    actions.appendChild(makeActionButton(RUN_ACTION_CONTINUE, 'continue', callbacks.onContinue, stateObj.busy));
  }
  if (def.canRetry && typeof callbacks.onRetry === 'function') {
    actions.appendChild(makeActionButton(RUN_ACTION_RETRY, 'retry', callbacks.onRetry, stateObj.busy));
  }
  if (actions.children.length > 0) {
    bar.appendChild(actions);
  }

  const openItems = Array.isArray(stateObj.openItems) ? stateObj.openItems : [];
  if (def.key === 'done_open_items' && openItems.length > 0) {
    const list = document.createElement('ul');
    list.className = 'ui-run-bar-open-items';
    list.setAttribute('aria-label', 'Open items');
    for (const itemText of openItems) {
      const li = document.createElement('li');
      li.textContent = String(itemText ?? '');
      list.appendChild(li);
    }
    bar.appendChild(list);
  }

  return bar;
}

export function createRunBar(host, initialState = {}, callbacks = {}) {
  let currentState = { ...initialState };
  let currentCallbacks = { ...callbacks };
  let barEl = renderRunBar(currentState, currentCallbacks);
  if (host) host.replaceChildren(barEl);

  return {
    get element() {
      return barEl;
    },
    getState() {
      return { ...currentState };
    },
    update(nextState = {}, nextCallbacks) {
      currentState = { ...currentState, ...nextState };
      if (nextCallbacks) currentCallbacks = { ...currentCallbacks, ...nextCallbacks };
      barEl = renderRunBar(currentState, currentCallbacks);
      if (host) host.replaceChildren(barEl);
      return barEl;
    },
    destroy() {
      if (host) host.replaceChildren();
    },
  };
}
