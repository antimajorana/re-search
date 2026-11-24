import * as api from '/assets/api.js';

let ui;
let callbacks;
let project = null;
let experiments = [];
let selectedId = null;
let loadSerial = 0;
let cutoffSerial = 0;

const experiment = () => experiments.find((item) => item.id === selectedId);
const field = (form, name) => form.elements.namedItem(name);
const value = (form, name) => field(form, name).value;

function el(tag, text = '', className = '') {
  const node = document.createElement(tag);
  node.textContent = text;
  if (className) node.className = className;
  return node;
}

function button(label, act) {
  const node = el('button', label, 'pill-btn sm');
  node.type = 'button';
  node.addEventListener('click', act);
  return node;
}

function option(id, text) {
  const node = el('option', text);
  node.value = id;
  return node;
}

function status(text, error = false) {
  ui.status.textContent = text;
  ui.status.hidden = !text;
  ui.status.dataset.error = String(error);
}

export function initProjectExperiments(opts = {}) {
  callbacks = opts;
  const panel = document.getElementById('project-experiments-panel');
  if (!panel) return;
  panel.innerHTML = `
    <div class="experiment-actions">
      <label>Experiment <select class="compact-select" data-select></select></label>
      <button type="button" class="pill-btn sm primary" data-new>New experiment</button>
    </div>
    <p class="project-status" data-status role="status" hidden></p>
    <form class="experiment-form" data-create hidden>
      <label>Question<textarea class="text-input" name="question" rows="3" required maxlength="4000"></textarea></label>
      <div class="experiment-fields">
        <label>Metric <input class="text-input" name="metric" required maxlength="128"></label>
        <label>Metric unit <input class="text-input" name="unit" required maxlength="64" placeholder="Dimensionless if applicable"></label>
        <label>Better outcome <select class="compact-select" name="direction"><option value="minimize">Lower score</option><option value="maximize">Higher score</option></select></label>
        <label>Budget limit <input class="text-input" type="number" name="budget" min="0" step="any" required></label>
        <label>Budget unit <input class="text-input" name="budget_unit" required maxlength="64" placeholder="e.g. function evaluations"></label>
        <label>Baseline session <select class="compact-select" name="baseline" required></select></label>
        <label>Fork from proof step <select class="compact-select" name="checkpoint" disabled></select></label>
        <label>Ledger sequence <input class="text-input" type="number" name="at_seq" min="1" step="1" required></label>
      </div>
      <div class="experiment-actions">
        <button type="submit" class="pill-btn sm primary">Freeze experiment</button>
        <button type="button" class="pill-btn sm" data-cancel>Cancel</button>
      </div>
    </form>
    <div data-detail hidden>
      <div class="experiment-description" data-description></div>
      <form class="experiment-form" data-branch>
        <div class="experiment-fields">
          <label>Branch name <input class="text-input" name="label" required maxlength="128"></label>
          <label>Fork <select class="compact-select" name="session_id"></select></label>
        </div>
        <label data-assumption>Changed assumption <textarea class="text-input" name="assumption" maxlength="4000" rows="2"></textarea></label>
        <div class="experiment-actions"><button type="submit" class="pill-btn sm primary">Add branch</button></div>
      </form>
      <div data-results></div>
      <form class="experiment-form" data-observation hidden>
        <h3 data-observation-heading>Record observation</h3>
        <input type="hidden" name="session_id">
        <div class="experiment-fields">
          <label>Outcome <select class="compact-select" name="status"><option value="completed">Completed</option><option value="failed">Failed</option></select></label>
          <label>Actual budget <span data-budget-unit></span><input class="text-input" type="number" name="actual_budget" min="0" step="any" required></label>
          <label>Observed score <span data-score-unit></span><input class="text-input" type="number" name="score" step="any" required></label>
          <label>Evidence sequence <input class="text-input" type="number" name="at_seq" min="1" step="1" placeholder="Verified ledger head"></label>
        </div>
        <label>Measurement or failure notes <textarea class="text-input" name="note" maxlength="4000" rows="2"></textarea></label>
        <div class="experiment-actions">
          <button type="submit" class="pill-btn sm primary">Save observation</button>
          <button type="button" class="pill-btn sm" data-cancel>Cancel</button>
        </div>
      </form>
    </div>
  `;
  const get = (key) => panel.querySelector(`[data-${key}]`);
  ui = {
    panel, select: get('select'), create: get('create'), detail: get('detail'),
    description: get('description'), branch: get('branch'), results: get('results'),
    observation: get('observation'), status: get('status'), newButton: get('new'),
  };
  ui.newButton.addEventListener('click', openCreate);
  ui.select.addEventListener('change', () => {
    selectedId = ui.select.value || null;
    ui.branch.reset();
    ui.observation.hidden = true;
    renderExperiment();
  });
  ui.create.querySelector('[data-cancel]').addEventListener('click', () => { ui.create.hidden = true; });
  ui.observation.querySelector('[data-cancel]').addEventListener('click', () => { ui.observation.hidden = true; });
  field(ui.create, 'baseline').addEventListener('change', loadCutoffs);
  field(ui.create, 'checkpoint').addEventListener('change', () => {
    field(ui.create, 'at_seq').value = value(ui.create, 'checkpoint');
  });
  field(ui.branch, 'session_id').addEventListener('change', toggleAssumption);
  field(ui.observation, 'status').addEventListener('change', () => {
    field(ui.observation, 'score').required = value(ui.observation, 'status') === 'completed';
  });
  ui.create.addEventListener('submit', saveExperiment);
  ui.branch.addEventListener('submit', addBranch);
  ui.observation.addEventListener('submit', saveObservation);
  renderExperiment();
}

export function setExperimentProject(nextProject) {
  const changed = project?.id !== nextProject?.id;
  project = nextProject;
  if (!ui) return;
  if (changed) {
    ++loadSerial;
    ++cutoffSerial;
    experiments = [];
    selectedId = null;
    ui.create.hidden = true;
    ui.branch.reset();
    ui.observation.hidden = true;
    status('');
    renderExperiment();
    loadProjectExperiments();
  } else {
    refreshExperimentSessions();
  }
}

export async function loadProjectExperiments() {
  if (!project || !ui || ui.panel.hidden) return;
  const id = project.id;
  const serial = ++loadSerial;
  status('Loading experiments…');
  try {
    const result = await api.listExperiments(id);
    if (serial !== loadSerial || project?.id !== id) return;
    experiments = result.experiments || [];
    if (!experiment()) selectedId = experiments[0]?.id || null;
    renderExperiment();
    status(experiments.length ? '' : 'Freeze a question, metric, budget, and fork point to compare approaches.');
  } catch (error) {
    if (serial === loadSerial && project?.id === id) status(error.message, true);
  }
}

function projectSessions() {
  const ids = new Set(project?.session_ids || []);
  return (callbacks.getSessions?.() || []).filter((session) => ids.has(session.id));
}

export function refreshExperimentSessions() {
  if (!ui) return;
  if (!ui.create.hidden) {
    const input = field(ui.create, 'baseline');
    const selected = input.value;
    const sessions = projectSessions().filter((session) => session.event_count > 0);
    input.replaceChildren(option('', sessions.length ? 'Select a session' : 'No sessions with ledger events'));
    for (const session of sessions) input.append(option(session.id, session.title || session.id));
    if (sessions.some((session) => session.id === selected)) input.value = selected;
  }
  const current = experiment();
  if (!current) return;
  const forkPick = field(ui.branch, 'session_id');
  const selectedFork = forkPick.value;
  const registered = new Set(current.branches.map((branch) => branch.session_id));
  const forks = projectSessions().filter((session) =>
    session.parent_session === current.baseline.session_id && !registered.has(session.id));
  forkPick.replaceChildren(option('', 'Create a fork at the frozen point'));
  for (const fork of forks) forkPick.append(option(fork.id, `Use existing fork · ${fork.title || fork.id}`));
  if (forks.some((fork) => fork.id === selectedFork)) forkPick.value = selectedFork;
  toggleAssumption();
}

function openCreate() {
  ui.create.reset();
  ui.create.hidden = false;
  field(ui.create, 'question').value = (project?.question || '').slice(0, 4000);
  refreshExperimentSessions();
  loadCutoffs();
  field(ui.create, 'question').focus();
}

async function loadCutoffs() {
  const id = value(ui.create, 'baseline');
  const serial = ++cutoffSerial;
  const picker = field(ui.create, 'checkpoint');
  const seq = field(ui.create, 'at_seq');
  picker.disabled = true;
  picker.replaceChildren(option('', 'Choose a ledger sequence or proof step'));
  seq.value = '';
  const baseline = projectSessions().find((session) => session.id === id);
  if (!baseline) return;
  picker.append(option(baseline.event_count, `Latest event · ${baseline.event_count}`));
  picker.disabled = false;
  try {
    const dag = await api.getDag(id);
    if (serial !== cutoffSerial) return;
    for (const entry of (dag.entries || [])) {
      picker.append(option(entry.seq, `${entry.entry_id} · seq ${entry.seq} · ${entry.summary || entry.entry_type}`));
    }
  } catch (error) {
    if (serial === cutoffSerial) status(`Could not load proof steps. Enter a ledger sequence. ${error.message}`, true);
  }
}

function toggleAssumption() {
  ui.branch.querySelector('[data-assumption]').hidden = Boolean(value(ui.branch, 'session_id'));
}

function applyExperiment(updated, select = false) {
  ++loadSerial;
  const index = experiments.findIndex((entry) => entry.id === updated.id);
  if (index < 0) experiments.push(updated);
  else if (updated.revision >= experiments[index].revision) experiments[index] = updated;
  if (select) {
    selectedId = updated.id;
    ui.branch.reset();
    ui.observation.hidden = true;
  }
  renderExperiment();
}

async function saveExperiment(event) {
  event.preventDefault();
  if (!project) return;
  const id = project.id;
  const form = ui.create;
  const submit = form.querySelector('[type=submit]');
  if (submit.disabled) return;
  submit.disabled = true;
  try {
    const created = await api.createExperiment(id, {
      question: value(form, 'question'),
      metric: { name: value(form, 'metric'), unit: value(form, 'unit'), direction: value(form, 'direction') },
      budget: { limit: Number(value(form, 'budget')), unit: value(form, 'budget_unit') },
      baseline: { session_id: value(form, 'baseline'), at_seq: Number(value(form, 'at_seq')) },
    });
    if (project?.id !== id) return;
    form.hidden = true;
    applyExperiment(created, true);
    status('Experiment frozen.');
  } catch (error) {
    if (project?.id === id) status(error.message, true);
  } finally {
    submit.disabled = false;
  }
}

async function addBranch(event) {
  event.preventDefault();
  const current = experiment();
  if (!current) return;
  const projectId = project.id;
  const form = ui.branch;
  const label = value(form, 'label');
  let id = value(form, 'session_id');
  let created = false;
  let linked = false;
  let retryExisting = false;
  const submit = form.querySelector('[type=submit]');
  if (submit.disabled) return;
  submit.disabled = true;
  try {
    if (!id) {
      const assumption = value(form, 'assumption').trim();
      const receipt = await api.forkSession(current.baseline.session_id, {
        at_seq: current.baseline.at_seq, ...(assumption ? { modified_assumption: assumption } : {}),
      });
      id = receipt.new_session;
      created = true;
      if (receipt.project_id !== projectId) await api.linkProjectSession(projectId, id);
      linked = true;
    }
    const updated = await api.registerExperimentBranch(projectId, current.id, { session_id: id, label });
    if (project?.id === projectId) {
      if (selectedId === current.id) form.reset();
      applyExperiment(updated);
      if (selectedId === current.id) status('Branch registered.');
    }
  } catch (error) {
    if (project?.id === projectId && selectedId === current.id) {
      retryExisting = created && linked;
      const recovery = linked
        ? 'Use the existing fork to retry.'
        : 'Link the fork from the project Sessions panel, then retry.';
      status(created ? `Fork ${id} was created. ${recovery} ${error.message}` : error.message, true);
    }
  } finally {
    if (created) await callbacks.onForkCreated?.(projectId);
    if (retryExisting && project?.id === projectId && selectedId === current.id) {
      const input = field(form, 'session_id');
      if (Array.from(input.options).some((fork) => fork.value === id)) input.value = id;
      toggleAssumption();
    }
    submit.disabled = false;
  }
}

function openObservation(branch) {
  const current = experiment();
  if (!current) return;
  const form = ui.observation;
  form.reset();
  field(form, 'session_id').value = branch.session_id;
  field(form, 'score').required = true;
  form.querySelector('[data-observation-heading]').textContent = `Record observation for ${branch.label}`;
  form.querySelector('[data-budget-unit]').textContent = `(${current.budget.unit})`;
  form.querySelector('[data-score-unit]').textContent = `(${current.metric.unit})`;
  form.hidden = false;
  field(form, 'status').focus();
}

async function saveObservation(event) {
  event.preventDefault();
  const current = experiment();
  if (!current) return;
  const projectId = project.id;
  const form = ui.observation;
  const submit = form.querySelector('[type=submit]');
  if (submit.disabled) return;
  submit.disabled = true;
  try {
    const score = value(form, 'score');
    const seq = value(form, 'at_seq');
    const updated = await api.recordExperimentObservation(projectId, current.id, value(form, 'session_id'), {
      status: value(form, 'status'), actual_budget: Number(value(form, 'actual_budget')),
      score: score === '' ? null : Number(score), note: value(form, 'note'),
      at_seq: seq === '' ? null : Number(seq),
    });
    if (project?.id !== projectId) return;
    if (selectedId === current.id) form.hidden = true;
    applyExperiment(updated);
    if (selectedId === current.id) status('Observation recorded.');
  } catch (error) {
    if (project?.id === projectId && selectedId === current.id) status(error.message, true);
  } finally {
    submit.disabled = false;
  }
}

function renderExperiment() {
  if (!ui) return;
  const current = experiment();
  ui.newButton.disabled = !project;
  ui.select.replaceChildren();
  if (!current) ui.select.append(option('', 'No experiments'));
  for (const item of experiments) ui.select.append(option(item.id, item.question));
  ui.select.value = selectedId || '';
  ui.select.disabled = !current;
  ui.detail.hidden = !current;
  if (!current) return;
  const baseline = current.baseline;
  const direction = current.metric.direction === 'minimize' ? 'Lower is better' : 'Higher is better';
  const baselineText = baseline.entry_id ? `${baseline.entry_id} at seq ${baseline.at_seq}` : `seq ${baseline.at_seq}`;
  const anchor = button(`Open baseline · ${baselineText}`, () => callbacks.onOpenSession?.(baseline.session_id, baseline.entry_id));
  anchor.title = baseline.event_hash;
  ui.description.replaceChildren(
    el('h3', current.question),
    el('p', `${current.metric.name} · ${current.metric.unit} · ${direction}`, 'experiment-muted'),
    el('p', `Common budget ${current.budget.limit} ${current.budget.unit}`, 'experiment-muted'),
    anchor,
  );
  const comparison = current.comparison;
  ui.results.replaceChildren(
    el('h3', 'Researcher-recorded outcomes'),
    el('p',
      `${comparison.completed_within_budget} within budget · ${comparison.completed_over_budget} over budget · ${comparison.failed} failed · ${comparison.pending} pending`,
      'experiment-muted'),
  );
  for (const branch of current.branches) ui.results.append(branchCard(current, branch));
  refreshExperimentSessions();
}

function branchCard(current, branch) {
  const card = el('article', '', 'experiment-branch-card');
  const observed = branch.observations.at(-1);
  const title = el('h4', branch.label);
  const actions = el('div', '', 'experiment-actions');
  const diff = el('pre', '', 'experiment-diff');
  diff.hidden = true;
  actions.append(
    button('Open branch', () => callbacks.onOpenSession?.(branch.session_id)),
    button('Record observation', () => openObservation(branch)),
    button('Diff with frozen baseline', async () => {
      if (!diff.hidden) { diff.hidden = true; return; }
      diff.hidden = false;
      diff.textContent = 'Loading diff…';
      try {
        const report = await api.diffExperimentBranch(current.project_id, current.id, branch.session_id);
        diff.textContent = report.summary_diff || 'No proof graph changes.';
      } catch (error) {
        diff.textContent = error.message;
      }
    }),
  );
  card.append(title, actions, diff);
  if (current.comparison.best_session_ids.includes(branch.session_id)) {
    card.append(el('p', 'Best observed score within budget', 'experiment-best'));
  }
  if (observed) card.append(observationEntry(current, branch, observed));
  else card.append(el('p', 'Awaiting observation', 'experiment-muted'));
  if (branch.observations.length > 1) {
    const history = el('details');
    history.append(el('summary', 'Earlier observations'));
    for (const earlier of branch.observations.slice(0, -1).reverse()) {
      history.append(observationEntry(current, branch, earlier));
    }
    card.append(history);
  }
  return card;
}

function observationEntry(current, branch, observed) {
  const box = el('div', '', 'experiment-observation');
  const outcome = observed.status === 'failed' ? 'Failed' : observed.within_budget ? 'Completed' : 'Completed over budget';
  const score = observed.score == null ? '' : ` · score ${observed.score} ${current.metric.unit}`;
  box.append(el('p', `${outcome} · used ${observed.actual_budget} ${current.budget.unit}${score}`));
  if (observed.note) box.append(el('p', observed.note, 'experiment-note'));
  const receipt = el('details');
  receipt.append(el('summary', `Ledger at seq ${observed.evidence.seq} · ${observed.evidence.tools.length} tool receipts`));
  receipt.append(el('code', observed.evidence.event_hash));
  if (observed.evidence.entry_id) {
    receipt.append(button('Inspect evidence step', () => callbacks.onOpenSession?.(branch.session_id, observed.evidence.entry_id)));
  }
  for (const tool of observed.evidence.tools) {
    receipt.append(el('div', `${tool.tool} · ${tool.result.replace('_', ' ')} · seq ${tool.request_seq}${tool.result_seq ? ` → ${tool.result_seq}` : ''}`));
  }
  if (observed.evidence.tools_omitted) {
    receipt.append(el('p', `${observed.evidence.tools_omitted} earlier tool receipts in the branch ledger.`));
  }
  box.append(receipt);
  return box;
}
