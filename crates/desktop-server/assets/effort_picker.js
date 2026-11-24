// Secondary smaller floating window for selecting reasoning effort after picking a model
// (and anytime via the compact "Effort: ..." pill next to the model selector).
import { escapeHtml } from '/assets/math.js';

const EFFORT_OPTIONS = [
  { id: '', label: 'Default', badge: 'Default', desc: 'Use provider & model default behavior' },
  { id: 'low', label: 'Low', badge: 'Low', desc: 'Fast replies & quick sanity checks' },
  { id: 'medium', label: 'Medium', badge: 'Medium', desc: 'Balanced derivation depth & speed' },
  { id: 'high', label: 'High', badge: 'High', desc: 'Deep multi-step reasoning & proofs' },
  { id: 'xhigh', label: 'Extra High', badge: 'X-High', desc: 'Extended thinking budget (Claude / ChatGPT)' },
  { id: 'max', label: 'Max', badge: 'Max', desc: 'Maximum reasoning depth supported' },
];

const GLOBAL_EFFORT_KEY = 're-search-effort-choice';
const SESSION_EFFORT_KEY = 're-search-effort-by-session';
const MAX_SESSIONS = 200;

let popoverEl = null;
let triggerBtn = null;
let returnFocus = null;
let gridEl = null;
let modelLabelEl = null;
let currentEffort = '';
let activeSessionId = null;
let callbacks = {};

function readJsonStore(key, fallback) {
  try {
    const raw = window.localStorage.getItem(key);
    return raw ? JSON.parse(raw) : fallback;
  } catch {
    return fallback;
  }
}

function writeJsonStore(key, val) {
  try {
    window.localStorage.setItem(key, JSON.stringify(val));
  } catch {
    // Ignore storage unavailability
  }
}

export function normalizeEffort(raw) {
  const v = String(raw || '').trim().toLowerCase();
  if (v === 'low' || v === 'medium' || v === 'high' || v === 'xhigh' || v === 'max') return v;
  return '';
}

export function initEffortPicker(opts = {}) {
  callbacks = opts;
  popoverEl = document.getElementById('effort-popover');
  triggerBtn = document.getElementById('composer-effort-btn');
  gridEl = document.getElementById('effort-options-grid');
  modelLabelEl = document.getElementById('effort-model-label');
  const closeBtn = document.getElementById('effort-close-btn');

  currentEffort = normalizeEffort(readJsonStore(GLOBAL_EFFORT_KEY, ''));

  if (triggerBtn) {
    triggerBtn.addEventListener('click', (ev) => {
      ev.stopPropagation();
      if (popoverEl && !popoverEl.hidden) closeEffortWindow();
      else openEffortWindow(null);
    });
  }
  if (closeBtn) closeBtn.addEventListener('click', () => closeEffortWindow());

  document.addEventListener('click', (ev) => {
    if (!popoverEl || popoverEl.hidden) return;
    if (popoverEl.contains(ev.target) || (triggerBtn && triggerBtn.contains(ev.target))) return;
    if (ev.target && ev.target.id === 'composer-model') return;
    closeEffortWindow(false);
  });

  renderGrid();
  syncTrigger();
}

export function openEffortWindow(choice = null) {
  if (!popoverEl) return;
  if (popoverEl.hidden) {
    returnFocus = choice ? document.getElementById('composer-model') : document.querySelector('#research-options summary');
  }
  const effective = choice && choice.model
    ? choice
    : (typeof callbacks.getModelChoice === 'function' ? callbacks.getModelChoice() : null);
  if (modelLabelEl) {
    const label = effective && effective.model
      ? `${effective.provider} · ${effective.label || effective.model}`
      : 'No model selected';
    modelLabelEl.textContent = label;
  }
  renderGrid();
  popoverEl.hidden = false;
  const active = popoverEl.querySelector('.effort-option.is-active') || popoverEl.querySelector('.effort-option');
  if (active) active.focus();
}

export function closeEffortWindow(restoreFocus = true) {
  if (!popoverEl || popoverEl.hidden) return;
  popoverEl.hidden = true;
  if (restoreFocus) returnFocus?.focus();
  returnFocus = null;
}

export function getReasoningEffort() {
  return currentEffort || undefined;
}

export function setReasoningEffort(rawEffort) {
  currentEffort = normalizeEffort(rawEffort);
  writeJsonStore(GLOBAL_EFFORT_KEY, currentEffort);
  if (activeSessionId) rememberEffortForSession(activeSessionId, currentEffort);
  renderGrid();
  syncTrigger();
}

export function applySessionEffort(sessionId, sess) {
  activeSessionId = sessionId || null;
  const metaEffort = sess && sess.meta ? normalizeEffort(sess.meta.reasoning_effort) : '';
  const storedMap = readJsonStore(SESSION_EFFORT_KEY, {});
  const remembered = activeSessionId && storedMap[activeSessionId] !== undefined
    ? normalizeEffort(storedMap[activeSessionId])
    : null;
  const globalVal = normalizeEffort(readJsonStore(GLOBAL_EFFORT_KEY, ''));
  currentEffort = metaEffort || (remembered !== null ? remembered : globalVal);
  renderGrid();
  syncTrigger();
}

export function rememberEffortForSession(sessionId, effort) {
  if (!sessionId) return;
  const norm = normalizeEffort(effort);
  const map = readJsonStore(SESSION_EFFORT_KEY, {});
  delete map[sessionId];
  map[sessionId] = norm;
  const keys = Object.keys(map);
  for (const k of keys.slice(0, Math.max(0, keys.length - MAX_SESSIONS))) delete map[k];
  writeJsonStore(SESSION_EFFORT_KEY, map);
  writeJsonStore(GLOBAL_EFFORT_KEY, norm);
}

function renderGrid() {
  if (!gridEl) return;
  gridEl.innerHTML = '';
  for (const opt of EFFORT_OPTIONS) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `effort-option${currentEffort === opt.id ? ' is-active' : ''}`;
    btn.setAttribute('data-effort', opt.id || 'default');
    btn.setAttribute('aria-pressed', currentEffort === opt.id ? 'true' : 'false');
    btn.innerHTML = `
      <span class="effort-opt-name">${escapeHtml(opt.label)}</span>
      <span class="effort-opt-desc">${escapeHtml(opt.desc)}</span>
    `;
    btn.addEventListener('click', () => {
      setReasoningEffort(opt.id);
      closeEffortWindow();
    });
    gridEl.appendChild(btn);
  }
}

function syncTrigger() {
  if (!triggerBtn) return;
  const meta = EFFORT_OPTIONS.find((o) => o.id === currentEffort) || EFFORT_OPTIONS[0];
  triggerBtn.textContent = `Effort: ${meta.badge}`;
  triggerBtn.setAttribute('data-active', currentEffort ? 'true' : 'false');
}
