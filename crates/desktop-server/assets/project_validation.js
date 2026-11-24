import * as api from '/assets/api.js';
import { renderRichInto } from '/assets/math.js';

const axes = { resolution: 'Resolution', basis: 'Basis size', precision: 'Arithmetic precision' };
const planStatus = { open: 'Awaiting candidate', accepted: 'Within frozen criteria', rejected: 'Outside frozen criteria' };
const labels = {
  candidate_method: 'Candidate method', reference_method: 'Reference method',
  axis: 'Refinement axis', atol: 'Absolute tolerance', rtol: 'Relative tolerance',
};
let callbacks = {};
let ui;
let project;
let plans = [];
let activePlan;
let candidates = [];
let projectGeneration = 0;
let planRequest = 0;
let candidateRequest = 0;
const evaluating = new Set();

export function initProjectValidation(opts) {
  const panel = document.getElementById('project-validation-panel');
  if (!panel) return;
  callbacks = opts;
  panel.innerHTML = `
    <div class="validation-toolbar">
      <select class="compact-select" aria-label="Validation plan" data-plan></select>
      <button type="button" class="pill-btn sm primary" data-new>New plan</button>
    </div>
    <p class="validation-status" role="status" data-status hidden></p>
    <form class="validation-card form-grid" data-freeze autocomplete="off" hidden>
      <div class="form-row"><label for="validation-session">Session</label><select id="validation-session" name="session_id" class="compact-select" required></select></div>
      <div class="form-row"><label for="validation-claim">Claim to test</label><textarea id="validation-claim" name="claim" class="text-input" rows="3" maxlength="2000" required></textarea></div>
      <div class="validation-fields">
        <div class="form-row"><label for="validation-candidate-method">Candidate method</label><input id="validation-candidate-method" name="candidate_method" class="text-input" maxlength="400" required></div>
        <div class="form-row"><label for="validation-reference-method">Reference method</label><input id="validation-reference-method" name="reference_method" class="text-input" maxlength="400" required></div>
        <div class="form-row"><label for="validation-axis">Refinement axis</label><select id="validation-axis" name="axis" class="compact-select" required><option value="" selected>Select an axis</option><option value="resolution">Resolution</option><option value="basis">Basis size</option><option value="precision">Arithmetic precision</option></select></div>
      </div>
      <div class="validation-fields">
        <div class="form-row"><label for="validation-atol">Absolute tolerance</label><input id="validation-atol" name="atol" class="text-input" placeholder="e.g. 1e-12" maxlength="384" spellcheck="false" required></div>
        <div class="form-row"><label for="validation-rtol">Relative tolerance</label><input id="validation-rtol" name="rtol" class="text-input" placeholder="e.g. 0" maxlength="384" spellcheck="false" required></div>
      </div>
      <fieldset class="validation-private">
        <legend>Private reference and controls</legend>
        <p>Enter a real scalar reference, a control within tolerance, and a planted control outside tolerance.</p>
        <div class="validation-fields">
          <div class="form-row"><label for="validation-reference">Reference value</label><input id="validation-reference" name="reference" class="text-input" maxlength="384" spellcheck="false" required></div>
          <div class="form-row"><label for="validation-known-pass">Known pass</label><input id="validation-known-pass" name="known_pass" class="text-input" maxlength="384" spellcheck="false" required></div>
          <div class="form-row"><label for="validation-planted-fail">Planted failure</label><input id="validation-planted-fail" name="planted_fail" class="text-input" maxlength="384" spellcheck="false" required></div>
        </div>
      </fieldset>
      <div class="project-actions validation-end">
        <button type="button" class="pill-btn sm" data-cancel>Cancel</button>
        <button type="submit" class="pill-btn sm primary">Freeze plan</button>
      </div>
    </form>
    <section class="validation-card" data-detail hidden></section>
  `;
  ui = {
    panel,
    picker: panel.querySelector('[data-plan]'),
    create: panel.querySelector('[data-new]'),
    status: panel.querySelector('[data-status]'),
    freeze: panel.querySelector('[data-freeze]'),
    detail: panel.querySelector('[data-detail]'),
  };
  ui.cancel = ui.freeze.querySelector('[data-cancel]');
  ui.submit = ui.freeze.querySelector('[type="submit"]');
  ui.picker.addEventListener('change', () => openPlan(ui.picker.value));
  ui.create.addEventListener('click', showFreezeForm);
  ui.cancel.addEventListener('click', () => {
    ui.freeze.reset();
    ui.freeze.hidden = true;
    ui.create.focus();
    renderCurrentPlan();
    status('');
    if (activePlan?.status === 'open') loadCandidates();
  });
  ui.freeze.addEventListener('submit', freezePlan);
  renderPicker();
}

export function setValidationProject(nextProject) {
  const changed = project?.id !== nextProject?.id;
  project = nextProject;
  if (!ui) return;
  updateSessions();
  if (!changed) return;
  projectGeneration += 1;
  planRequest += 1;
  candidateRequest += 1;
  plans = [];
  activePlan = null;
  candidates = [];
  ui.freeze.reset();
  ui.freeze.hidden = true;
  setFreezing(false);
  renderPicker();
  renderCurrentPlan();
  status('');
  if (project && !ui.panel.hidden) loadProjectValidation();
}

function updateSessions() {
  const select = ui.freeze.elements.namedItem('session_id');
  const prev = select.value;
  const sessionIds = project?.session_ids || [];
  const titles = new Map((callbacks.getSessions() || []).map(({ id, title }) => [id, title]));
  select.replaceChildren(option('', 'Select a linked session'));
  for (const id of sessionIds) select.append(option(id, titles.get(id) || id));
  if (sessionIds.includes(prev)) select.value = prev;
  else select.value = sessionIds.length === 1 ? sessionIds[0] : '';
  ui.create.disabled = ui.submit.disabled || sessionIds.length === 0;
}

export async function loadProjectValidation() {
  if (!ui || !project) return;
  const id = project.id;
  const request = ++planRequest;
  updateSessions();
  if (ui.freeze.hidden) status('Loading validation plans…');
  try {
    const data = await api.listValidationPlans(id);
    if (request !== planRequest) return;
    plans = data.plans || [];
    const next = plans.find((plan) => plan.id === activePlan?.id) || plans.at(-1) || null;
    if (next?.id !== activePlan?.id) {
      candidateRequest += 1;
      candidates = [];
    }
    activePlan = next;
    renderPicker();
    renderCurrentPlan();
    if (ui.freeze.hidden) {
      status(plans.length > 0
        ? ''
        : project.session_ids?.length
          ? 'Freeze the criteria before collecting a candidate result.'
          : 'Link or create a session to freeze a validation plan.');
      if (activePlan?.status === 'open') await loadCandidates();
    }
  } catch (err) {
    if (request === planRequest && ui.freeze.hidden) status(`Could not load validation plans. ${err.message}`, true);
  }
}

function renderPicker() {
  ui.picker.replaceChildren();
  if (plans.length === 0) ui.picker.append(option('', 'No validation plans'));
  for (const plan of plans) {
    const claim = plan.claim.replace(/\s+/g, ' ').trim();
    ui.picker.append(option(plan.id, claim.length > 72 ? `${claim.slice(0, 71)}…` : claim));
  }
  ui.picker.disabled = ui.submit.disabled || plans.length === 0;
  if (activePlan) ui.picker.value = activePlan.id;
}

async function openPlan(id) {
  candidateRequest += 1;
  candidates = [];
  activePlan = plans.find((plan) => plan.id === id) || null;
  ui.freeze.reset();
  ui.freeze.hidden = true;
  status('');
  renderCurrentPlan();
  if (activePlan?.status === 'open') await loadCandidates();
}

function showFreezeForm() {
  candidateRequest += 1;
  ui.freeze.reset();
  updateSessions();
  ui.freeze.hidden = false;
  ui.detail.hidden = true;
  status('');
  ui.freeze.elements.namedItem('session_id').focus();
}

function setFreezing(freezing) {
  ui.submit.disabled = freezing;
  ui.cancel.disabled = freezing;
  ui.picker.disabled = freezing || plans.length === 0;
  ui.create.disabled = freezing || !project?.session_ids?.length;
}

async function freezePlan(ev) {
  ev.preventDefault();
  if (!project || ui.submit.disabled) return;
  const projectId = project.id;
  const generation = projectGeneration;
  const payload = Object.fromEntries([...new FormData(ui.freeze)].map(([key, value]) => [key, value.trim()]));
  setFreezing(true);
  status('Freezing validation plan…');
  try {
    const plan = await api.freezeValidationPlan(projectId, payload);
    if (projectId !== project?.id) return;
    if (generation !== projectGeneration) {
      if (!ui.panel.hidden) await loadProjectValidation();
      return;
    }
    planRequest += 1;
    ui.freeze.reset();
    ui.freeze.hidden = true;
    plans = [...plans.filter((item) => item.id !== plan.id), plan];
    activePlan = plan;
    candidates = [];
    renderPicker();
    renderCurrentPlan();
    callbacks.onNotify?.('Validation plan frozen.');
  } catch (err) {
    if (generation === projectGeneration) status(`Could not freeze the plan. ${err.message}`, true);
    return;
  } finally {
    if (generation === projectGeneration) setFreezing(false);
  }
  await loadCandidates();
}

async function loadCandidates() {
  if (!project || !ui.freeze.hidden || activePlan?.status !== 'open') return;
  const request = ++candidateRequest;
  status('Loading candidates…');
  try {
    const data = await api.listValidationCandidates(project.id, activePlan.id);
    if (request !== candidateRequest) return;
    candidates = data.candidates || [];
    renderCurrentPlan();
    status('');
  } catch (err) {
    if (request === candidateRequest) status(`Could not load candidates. ${err.message}`, true);
  }
}

function renderCurrentPlan() {
  ui.detail.replaceChildren();
  ui.detail.hidden = !activePlan || !ui.freeze.hidden;
  if (ui.detail.hidden) return;
  const plan = activePlan;

  const badge = element('span', planStatus[plan.status] || plan.status,
    `badge ${plan.status === 'accepted' ? 'ok' : plan.status === 'rejected' ? 'err' : 'warn'}`);
  const title = element('div', '', 'validation-card-header');
  title.append(badge, button('Open session', () => callbacks.onOpenSession(plan.session_id)));
  ui.detail.append(title);

  const claim = element('div', '', 'rich-prose validation-claim');
  renderRichInto(claim, plan.claim);
  ui.detail.append(claim);
  const metadata = element('dl', '', 'validation-details');
  for (const key of ['candidate_method', 'reference_method', 'axis', 'atol', 'rtol']) {
    addField(metadata, labels[key], key === 'axis' ? axes[plan.axis] : plan[key]);
  }
  ui.detail.append(metadata);

  if (plan.evaluation) renderEvaluation(plan);
  else if (plan.status === 'open') renderCandidatePicker(plan);

  const records = document.createElement('details');
  records.className = 'validation-ledger';
  records.append(element('summary', 'Ledger records'));
  const audit = element('dl', '', 'validation-details');
  addField(audit, 'Frozen plan event', plan.commit_seq);
  addField(audit, 'Frozen plan hash', plan.commit_hash);
  addField(audit, 'Reference commitment', plan.private_commitment);
  if (plan.evaluation) {
    addField(audit, 'Evaluation event', plan.evaluation.evaluation_seq);
    addField(audit, 'Evaluation hash', plan.evaluation.evaluation_hash);
    addField(audit, 'Candidate result hash', plan.evaluation.result_hash);
  }
  records.append(audit);
  ui.detail.append(records);
}

function renderCandidatePicker(plan) {
  const form = element('form', '', 'validation-candidates');
  const field = element('div', '', 'form-row');
  const label = element('label', 'Recorded candidate');
  label.htmlFor = 'validation-candidate';
  const select = document.createElement('select');
  select.id = label.htmlFor;
  select.className = 'compact-select';
  select.required = true;
  select.disabled = candidates.length === 0;
  select.append(option('', candidates.length > 0 ? 'Select a proof step and result' : 'No candidates recorded'));
  for (const [index, candidate] of candidates.entries()) {
    const summary = (candidate.entry_summary || '').replace(/\s+/g, ' ').trim();
    const clipped = summary.length > 80 ? `${summary.slice(0, 79)}…` : summary;
    select.append(option(String(index + 1), `${candidate.entry_id} · result ${candidate.result_seq}${clipped ? ` · ${clipped}` : ''}`));
  }
  field.append(label, select);
  form.append(field);
  if (candidates.length === 0) {
    form.append(element('p', 'In the linked session, run check_convergence with the frozen axis and tolerances as decimal strings. Link the result to a numerical proof step. Then refresh candidates.', 'validation-hint'));
  }
  const actions = element('div', '', 'project-actions');
  const refresh = button('Refresh candidates', loadCandidates);
  const evaluate = element('button', 'Evaluate candidate', 'pill-btn sm primary');
  evaluate.type = 'submit';
  evaluate.disabled = true;
  select.addEventListener('change', () => { evaluate.disabled = !select.value || evaluating.has(plan.id); });
  actions.append(refresh, evaluate);
  form.append(actions);
  const recorded = candidates;
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const candidate = recorded[Number(select.value) - 1];
    if (!candidate || evaluate.disabled) return;
    await evaluateCandidate(plan, candidate, evaluate);
  });
  ui.detail.append(form);
}

async function evaluateCandidate(plan, candidate, evaluate) {
  if (!project || activePlan?.id !== plan.id || evaluating.has(plan.id)) return;
  const projectId = project.id;
  const current = () => project?.id === projectId && activePlan?.id === plan.id;
  evaluating.add(plan.id);
  evaluate.disabled = true;
  status('Evaluating the recorded result…');
  try {
    const evaluated = await api.evaluateValidationPlan(projectId, plan.id, {
      entry_id: candidate.entry_id,
      result_seq: candidate.result_seq,
    });
    if (project?.id !== projectId) return;
    planRequest += 1;
    plans = plans.map((item) => item.id === evaluated.id ? evaluated : item);
    if (current()) {
      candidateRequest += 1;
      activePlan = evaluated;
      renderPicker();
      renderCurrentPlan();
      if (ui.freeze.hidden) status('');
    }
    callbacks.onNotify?.('Validation result recorded.');
  } catch (err) {
    if (current() && ui.freeze.hidden) status(`Could not evaluate the candidate. ${err.message}`, true);
  } finally {
    evaluating.delete(plan.id);
    evaluate.disabled = false;
  }
}

function renderEvaluation(plan) {
  const evaluation = plan.evaluation;
  const section = element('section', '', 'validation-result');
  section.append(element('h4', 'Recorded outcome'));
  const results = element('dl', '', 'validation-details');
  addField(results, 'Candidate value', evaluation.value);
  addField(results, 'Result event', evaluation.result_seq);
  addField(results, 'Known-pass control', evaluation.known_pass_passed ? 'Passed' : 'Failed');
  addField(results, 'Planted-failure control', evaluation.planted_fail_rejected ? 'Rejected' : 'Passed');
  section.append(results);
  section.append(button(`Open proof step ${evaluation.entry_id}`, () =>
    callbacks.onOpenSession(plan.session_id, evaluation.entry_id)));
  ui.detail.append(section);
}

function status(message, error = false) {
  ui.status.textContent = message;
  ui.status.hidden = !message;
  ui.status.dataset.error = String(error);
}

function element(tag, text = '', className = '') {
  const node = document.createElement(tag);
  node.textContent = text;
  if (className) node.className = className;
  return node;
}

function button(label, callback) {
  const node = element('button', label, 'pill-btn sm');
  node.type = 'button';
  node.addEventListener('click', callback);
  return node;
}

function option(value, text) {
  const node = element('option', text);
  node.value = value;
  return node;
}

function addField(list, label, value) {
  if (value === null || value === undefined || value === '') return;
  list.append(element('dt', label), element('dd', String(value)));
}
