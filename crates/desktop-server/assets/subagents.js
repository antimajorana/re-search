// Settings -> Subagents tab: Built-in Agents overview, In-App AI Subagent Builder Agent,
// and user Custom Subagents manager + composer active-subagent selector.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';

let builtinList = [];
let customList = [];
let activeSubagentId = null;
let callbacks = {};

const STORE_KEY = 're-search-active-subagent';

export function initSubagents(opts = {}) {
  callbacks = opts;
  try {
    activeSubagentId = window.localStorage.getItem(STORE_KEY) || null;
  } catch {
    activeSubagentId = null;
  }

  const draftForm = document.getElementById('subagent-draft-form');
  const saveForm = document.getElementById('subagent-save-form');
  const clearBtn = document.getElementById('subagent-form-reset');
  const chipBtn = document.getElementById('composer-subagent-btn');

  if (draftForm) draftForm.addEventListener('submit', handleDraftWithAgent);
  if (saveForm) saveForm.addEventListener('submit', handleSaveSubagent);
  if (clearBtn) clearBtn.addEventListener('click', resetBuilderForm);
  if (chipBtn) {
    chipBtn.addEventListener('click', () => {
      if (typeof callbacks.onOpenSubagentsTab === 'function') callbacks.onOpenSubagentsTab();
    });
  }
  syncComposerBadge();
}

export async function loadSubagents() {
  try {
    const data = await api.listSubagents();
    builtinList = Array.isArray(data && data.builtin) ? data.builtin : [];
    customList = Array.isArray(data && data.custom) ? data.custom : [];
    if (activeSubagentId && !customList.some((s) => s.id === activeSubagentId)) {
      activeSubagentId = null;
      persistActive(null);
    }
    renderSubagentsPanel();
    syncComposerBadge();
  } catch {
    // Panel renders empty state cleanly
  }
}

export function getActiveSubagent() {
  if (!activeSubagentId) return null;
  return customList.find((s) => s.id === activeSubagentId) || null;
}

export function setActiveSubagent(id) {
  const found = id ? customList.find((s) => s.id === id) : null;
  activeSubagentId = found ? found.id : null;
  persistActive(activeSubagentId);
  syncComposerBadge();
  renderSubagentsPanel();
  if (found && typeof callbacks.onActivated === 'function') {
    callbacks.onActivated(found);
  }
}

function persistActive(id) {
  try {
    if (id) window.localStorage.setItem(STORE_KEY, id);
    else window.localStorage.removeItem(STORE_KEY);
  } catch {
    // Ignore storage unavailability
  }
}

function syncComposerBadge() {
  const btn = document.getElementById('composer-subagent-btn');
  if (!btn) return;
  const active = getActiveSubagent();
  btn.textContent = active ? `Agent: ${active.name}` : 'Agent: Solver';
  btn.setAttribute('aria-pressed', active ? 'true' : 'false');
  btn.title = active
    ? `Active custom subagent "${active.name}" — click to switch or clear in Settings -> Subagents`
    : 'Using built-in Solver — click to choose or create a custom specialist subagent';
}

async function handleDraftWithAgent(ev) {
  ev.preventDefault();
  const goalEl = document.getElementById('subagent-goal-input');
  const btn = document.getElementById('subagent-draft-btn');
  const goal = goalEl ? goalEl.value.trim() : '';
  if (!goal) {
    if (goalEl) goalEl.focus();
    return;
  }
  if (btn) {
    btn.disabled = true;
    btn.textContent = 'Designing subagent...';
  }
  try {
    const modelChoice = typeof callbacks.getModelChoice === 'function' ? callbacks.getModelChoice() : null;
    const payload = { goal };
    if (modelChoice && modelChoice.provider && !['claude-cli', 'antigravity'].includes(modelChoice.provider)) {
      payload.provider = modelChoice.provider;
      payload.model = modelChoice.model;
    }
    const resp = await api.draftSubagent(payload);
    const d = (resp && resp.draft) || {};
    fillForm(d);
    notify(`Subagent Builder (${resp.model || 'AI'}) drafted "${d.name || 'subagent'}". Review and click Save Subagent.`);
  } catch (err) {
    notify(`Could not generate draft: ${err.message}`);
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = 'Generate with AI Agent';
    }
  }
}

function fillForm(d) {
  const set = (id, v) => {
    const el = document.getElementById(id);
    if (el && v !== undefined && v !== null) el.value = String(v);
  };
  set('subagent-name-input', d.name || '');
  set('subagent-desc-input', d.description || '');
  set('subagent-tools-select', d.tools_profile || 'full_research');
  set('subagent-effort-select', d.reasoning_effort || '');
  set('subagent-rigor-select', d.rigor || '');
  set('subagent-prompt-input', d.system_prompt || '');
}

function resetBuilderForm() {
  fillForm({
    name: '',
    description: '',
    tools_profile: 'full_research',
    reasoning_effort: '',
    rigor: '',
    system_prompt: '',
  });
}

async function handleSaveSubagent(ev) {
  ev.preventDefault();
  const name = (document.getElementById('subagent-name-input')?.value || '').trim();
  const description = (document.getElementById('subagent-desc-input')?.value || '').trim();
  const tools_profile = document.getElementById('subagent-tools-select')?.value || 'full_research';
  const reasoning_effort = document.getElementById('subagent-effort-select')?.value || '';
  const rigor = document.getElementById('subagent-rigor-select')?.value || '';
  const system_prompt = (document.getElementById('subagent-prompt-input')?.value || '').trim();

  if (!name || !system_prompt) {
    notify('Subagent name and specialist instructions are required.');
    return;
  }
  const saveBtn = document.getElementById('subagent-save-btn');
  if (saveBtn) saveBtn.disabled = true;
  try {
    const resp = await api.createSubagent({
      name,
      description,
      tools_profile,
      reasoning_effort: reasoning_effort || undefined,
      rigor: rigor || undefined,
      system_prompt,
    });
    customList = Array.isArray(resp && resp.custom) ? resp.custom : customList;
    if (resp && resp.subagent) setActiveSubagent(resp.subagent.id);
    resetBuilderForm();
    const goalEl = document.getElementById('subagent-goal-input');
    if (goalEl) goalEl.value = '';
    renderSubagentsPanel();
    notify(`Saved and activated subagent "${resp?.subagent?.name || name}".`);
  } catch (err) {
    notify(`Could not save subagent: ${err.message}`);
  } finally {
    if (saveBtn) saveBtn.disabled = false;
  }
}

async function handleDeleteSubagent(sa) {
  if (!window.confirm(`Delete custom subagent "${sa.name}"?`)) return;
  try {
    const resp = await api.deleteSubagent(sa.id);
    customList = Array.isArray(resp && resp.custom) ? resp.custom : customList.filter((x) => x.id !== sa.id);
    if (activeSubagentId === sa.id) setActiveSubagent(null);
    else renderSubagentsPanel();
    notify(`Deleted subagent "${sa.name}".`);
  } catch (err) {
    notify(`Could not delete subagent: ${err.message}`);
  }
}

function renderSubagentsPanel() {
  const customBox = document.getElementById('custom-subagents-list');
  const builtinBox = document.getElementById('builtin-agents-list');

  if (customBox) {
    customBox.innerHTML = '';
    if (customList.length === 0) {
      customBox.innerHTML = '<div class="subagent-card" style="color:var(--ink-muted);font-size:12.5px;">No custom subagents yet. Describe a specialist above and click Generate with AI Agent, or fill the form directly.</div>';
    } else {
      for (const sa of customList) {
        const selected = sa.id === activeSubagentId;
        const card = document.createElement('div');
        card.className = `subagent-card${selected ? ' is-selected' : ''}`;
        card.innerHTML = `
          <div class="subagent-card-head">
            <strong style="font-size:13px;">${escapeHtml(sa.name)}</strong>
            <div style="display:flex;gap:6px;align-items:center;">
              <button type="button" class="pill-btn sm ${selected ? '' : 'primary'} sa-use-btn">${selected ? 'Active (Clear)' : 'Use in Chat'}</button>
              <button type="button" class="pill-btn sm sa-del-btn" title="Delete subagent">Delete</button>
            </div>
          </div>
          ${sa.description ? `<div style="font-size:12px;color:var(--ink-secondary);">${escapeHtml(sa.description)}</div>` : ''}
          <div class="subagent-meta-row">
            <span class="badge">${escapeHtml(sa.tools_profile || 'full_research')}</span>
            ${sa.reasoning_effort ? `<span class="badge">effort: ${escapeHtml(sa.reasoning_effort)}</span>` : ''}
            ${sa.rigor ? `<span class="badge">rigor: ${escapeHtml(sa.rigor)}</span>` : ''}
          </div>
        `;
        card.querySelector('.sa-use-btn')?.addEventListener('click', () => {
          setActiveSubagent(selected ? null : sa.id);
        });
        card.querySelector('.sa-del-btn')?.addEventListener('click', () => handleDeleteSubagent(sa));
        customBox.appendChild(card);
      }
    }
  }

  if (builtinBox) {
    builtinBox.innerHTML = builtinList
      .map(
        (b) => `
      <div class="builtin-card">
        <div style="display:flex;justify-content:space-between;gap:6px;align-items:center;">
          <strong style="font-size:12.5px;">${escapeHtml(b.name)}</strong>
          <span class="badge">${escapeHtml(b.category === 'builder' ? 'Builder' : 'Built-in')}</span>
        </div>
        <div class="builtin-card-role">${escapeHtml(b.role)}</div>
        <div class="builtin-card-desc">${escapeHtml(b.summary)}</div>
        <div style="font-size:11px;color:var(--ink-muted);margin-top:2px;">${escapeHtml(b.when)}</div>
      </div>`
      )
      .join('');
  }
}

function notify(msg) {
  if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
