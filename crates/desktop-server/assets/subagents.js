// Settings -> Agents & Models tab: Built-in Agents + per-agent models, AI Subagent Builder,
// and user Custom Subagents manager + composer active-subagent selector.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';

const CLI_PROVIDERS = new Set(['claude-cli', 'antigravity']);
const WORKER_ROLE_KEYS = new Set(['compute_worker', 'literature_worker', 'lean_worker', 'explorer_worker']);
const STORE_KEY = 're-search-active-subagent';

let builtinList = [];
let customList = [];
let providerGroups = [];
let agentModels = {};
let activeSubagentId = null;
let callbacks = {};

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
  const [subRes, setRes, modRes] = await Promise.allSettled([
    api.listSubagents(),
    api.getProviderSettings(),
    api.getModels(false),
  ]);
  if (subRes.status === 'fulfilled' && subRes.value) {
    const data = subRes.value;
    builtinList = Array.isArray(data.builtin) ? data.builtin : [];
    customList = Array.isArray(data.custom) ? data.custom : [];
    if (activeSubagentId && !customList.some((s) => s.id === activeSubagentId)) {
      activeSubagentId = null;
      persistActive(null);
    }
  }
  if (setRes.status === 'fulfilled' && setRes.value) {
    agentModels = (setRes.value.agent_models && typeof setRes.value.agent_models === 'object')
      ? setRes.value.agent_models
      : {};
  }
  if (modRes.status === 'fulfilled' && modRes.value) {
    providerGroups = Array.isArray(modRes.value.providers) ? modRes.value.providers : [];
  }
  renderSubagentsPanel();
  syncComposerBadge();
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
  btn.textContent = active ? `Agent: ${active.name}` : 'Agent: Orchestrator';
  btn.setAttribute('aria-pressed', active ? 'true' : 'false');
  btn.title = active
    ? `Active custom subagent "${active.name}" — click to switch or clear in Settings -> Agents & Models`
    : 'Using built-in Orchestrator — click to choose or create a custom specialist subagent';
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

function encodeModelValue(provider, model) {
  return provider && model ? `${provider}::${model}` : '';
}

function decodeModelValue(val) {
  if (!val) return null;
  const idx = String(val).indexOf('::');
  if (idx <= 0) return null;
  const provider = String(val).slice(0, idx);
  const model = String(val).slice(idx + 2);
  return provider && model ? { provider, model } : null;
}

function isCliGroup(g) {
  return !g || CLI_PROVIDERS.has(g.id) || g.route === 'claude-cli' || g.route === 'antigravity';
}

function populateModelSelect(selectEl, { allowCli = false, defaultLabel = 'Default (Follow Orchestrator)', selected = null } = {}) {
  if (!selectEl) return;
  selectEl.replaceChildren();
  selectEl.append(new Option(defaultLabel, ''));
  const targetVal = selected && selected.provider && selected.model
    ? encodeModelValue(selected.provider, selected.model)
    : '';
  let matched = false;
  for (const g of providerGroups) {
    if (!allowCli && isCliGroup(g)) continue;
    const models = Array.isArray(g.models) ? g.models : [];
    if (models.length === 0) continue;
    const og = document.createElement('optgroup');
    og.label = String(g.label || g.id);
    for (const m of models) {
      const val = encodeModelValue(g.id, m.id);
      const opt = new Option(String(m.label || m.id), val);
      opt.title = String(m.id);
      if (targetVal && val === targetVal) {
        opt.selected = true;
        matched = true;
      }
      og.append(opt);
    }
    selectEl.append(og);
  }
  if (targetVal && !matched) {
    const fallback = new Option(`${selected.provider}: ${selected.model} (saved)`, targetVal);
    fallback.selected = true;
    selectEl.append(fallback);
  }
  if (!targetVal) selectEl.value = '';
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
  const subModelSel = document.getElementById('subagent-model-select');
  if (subModelSel) {
    const sel = d && d.provider && d.model ? { provider: String(d.provider), model: String(d.model) } : null;
    populateModelSelect(subModelSel, {
      allowCli: false,
      defaultLabel: 'Default (Follow Orchestrator)',
      selected: sel,
    });
  }
}

function resetBuilderForm() {
  fillForm({
    name: '',
    description: '',
    tools_profile: 'full_research',
    reasoning_effort: '',
    rigor: '',
    system_prompt: '',
    provider: '',
    model: '',
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
  const chosenModel = decodeModelValue(document.getElementById('subagent-model-select')?.value || '');

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
      provider: chosenModel ? chosenModel.provider : undefined,
      model: chosenModel ? chosenModel.model : undefined,
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
  const subModelSel = document.getElementById('subagent-model-select');
  if (subModelSel) {
    populateModelSelect(subModelSel, {
      allowCli: false,
      defaultLabel: 'Default (Follow Orchestrator)',
      selected: decodeModelValue(subModelSel.value),
    });
  }

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
            ${sa.provider && sa.model ? `<span class="badge">${escapeHtml(sa.provider)}: ${escapeHtml(sa.model)}</span>` : ''}
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
    builtinBox.replaceChildren();
    for (const b of builtinList) {
      const card = document.createElement('div');
      card.className = 'builtin-card';
      const categoryBadge = b.category === 'worker'
        ? 'Worker'
        : (b.category === 'builder' ? 'Builder' : 'Built-in');
      card.innerHTML = `
        <div style="display:flex;justify-content:space-between;gap:6px;align-items:center;">
          <strong style="font-size:12.5px;">${escapeHtml(b.name)}</strong>
          <span class="badge">${escapeHtml(categoryBadge)}</span>
        </div>
        <div class="builtin-card-role">${escapeHtml(b.role)}</div>
        <div class="builtin-card-desc">${escapeHtml(b.summary)}</div>
        <div style="font-size:11px;color:var(--ink-muted);margin-top:2px;">${escapeHtml(b.when)}</div>
      `;
      if (b.role_key) {
        const allowCli = b.role_key === 'orchestrator';
        const defaultLabel = allowCli
          ? 'Default (Composer Choice)'
          : (WORKER_ROLE_KEYS.has(b.role_key) ? 'Default (Auto Smaller Model)' : 'Default (Follow Orchestrator)');
        let lastSaved = agentModels[b.role_key] && agentModels[b.role_key].provider && agentModels[b.role_key].model
          ? agentModels[b.role_key]
          : null;
        const row = document.createElement('div');
        row.className = 'form-row';
        row.style.marginTop = '6px';
        const selId = `agent-model-${String(b.role_key).replace(/[^a-z0-9_-]/gi, '')}`;
        const label = document.createElement('label');
        label.htmlFor = selId;
        label.style.cssText = 'font-size:11px;color:var(--ink-muted);';
        label.textContent = 'Assigned Model';
        const select = document.createElement('select');
        select.id = selId;
        select.className = 'compact-select agent-model-select';
        select.dataset.roleKey = b.role_key;
        populateModelSelect(select, { allowCli, defaultLabel, selected: lastSaved });
        select.addEventListener('change', async () => {
          const chosen = decodeModelValue(select.value);
          select.disabled = true;
          try {
            const val = chosen ? { provider: chosen.provider, model: chosen.model } : null;
            await api.updateProviderSettings({ agent_models: { [b.role_key]: val } });
            agentModels[b.role_key] = val;
            lastSaved = val;
            notify(val ? `${b.name} assigned to ${val.provider}: ${val.model}.` : `${b.name} reset to default model.`);
            if (b.role_key === 'orchestrator' && typeof callbacks.onOrchestratorModelChanged === 'function') {
              callbacks.onOrchestratorModelChanged(val);
            }
          } catch (err) {
            populateModelSelect(select, { allowCli, defaultLabel, selected: lastSaved });
            notify(`Could not update ${b.name} model: ${err.message}`);
          } finally {
            select.disabled = false;
          }
        });
        row.append(label, select);
        card.append(row);
      }
      builtinBox.append(card);
    }
  }
}

function notify(msg) {
  if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
