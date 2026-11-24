// Project notebook, session links, and research notes.
import * as api from '/assets/api.js';
import { wireTabKeys } from '/assets/connect.js';
import { renderMathTextInto, renderRichInto } from '/assets/math.js';
import { reconcileEdits, showSavedValues } from '/assets/project_edits.js';
import { focusProjectSource, initProjectLibrary, renderProjectLibrary } from '/assets/project_library.js';
import { initProjectValidation, loadProjectValidation, setValidationProject } from '/assets/project_validation.js';
import {
  initProjectExperiments, loadProjectExperiments, refreshExperimentSessions, setExperimentProject,
} from '/assets/experiments.js';

const byId = (id) => document.getElementById(`project-${id}`);
const sessionIds = (project) => project?.session_ids || [];
const itemLabels = { finding: 'Finding', failed_approach: 'Failed approach', open_question: 'Open question' };
const settingLabels = { title: 'Project name', question: 'Research question', conventions: 'Pinned conventions' };
const drafts = new Map();
let callbacks = {};
let projects = [];
let currentProject = null;
let loadRequest = 0;
let loading = false;
let linking = false;
let edit = null;
let promotion = null;
let savingPromotion = false;

export function initProjects(opts) {
  callbacks = opts;
  initProjectLibrary({
    onNotify: callbacks.onNotify,
    onChanged: acceptProjectUpdate,
    onOpenSession: callbacks.onOpenSourceSession,
  });
  initProjectValidation({
    getSessions: callbacks.getSessions,
    onNotify: callbacks.onNotify,
    onOpenSession: callbacks.onOpenSession,
  });
  initProjectExperiments({
    getSessions: callbacks.getSessions,
    onOpenSession: callbacks.onOpenSession,
    onForkCreated: async (projectId) => {
      try {
        await callbacks.onSessionsChanged?.();
        if (currentProject?.id === projectId) acceptProjectUpdate(await api.getProject(projectId));
      } catch (error) {
        callbacks.onNotify?.('Could not refresh project sessions. ' + error.message);
      }
    },
  });

  document.getElementById('projects-open-btn').addEventListener('click', () => {
    if (byId('overview').hidden) openProjects(callbacks.getActiveProjectId());
    else closeProjects(true);
  });
  byId('select').addEventListener('change', () => selectProject(byId('select').value));
  byId('create-btn').addEventListener('click', () => editProject(null));
  byId('edit-btn').addEventListener('click', () => editProject(currentProject));
  byId('edit-cancel').addEventListener('click', () => closeEditor(true));
  byId('edit-form').addEventListener('submit', saveProject);
  byId('new-session').addEventListener('click', createProjectSession);
  byId('link-session-form').addEventListener('submit', linkSession);
  byId('restore-btn')?.addEventListener('click', () => byId('restore-file').click());
  byId('restore-file')?.addEventListener('change', restoreBackup);
  byId('promote-cancel').addEventListener('click', () => { promotion = null; renderPromotion(); });
  byId('promote-form').addEventListener('submit', savePromotion);
  byId('promote-source').addEventListener('change', updatePromotionEvidenceInputs);

  for (const id of ['finding', 'open-question']) {
    byId(`${id}-form`).addEventListener('submit', (ev) => saveNote(ev, id));
    byId(`${id}-text`).addEventListener('input', rememberDraft);
  }
  byId('finding-kind').addEventListener('change', rememberDraft);

  const tabs = Array.from(byId('tabs').querySelectorAll('[role="tab"]'));
  for (const tab of tabs) tab.addEventListener('click', () => selectSection(tab.dataset.projectTab));
  wireTabKeys(tabs);
}

export async function openProjects(preferredId = null, section = 'question') {
  const opening = byId('overview').hidden;
  byId('overview').hidden = false;
  document.getElementById('app-shell').setAttribute('data-project-overview', 'true');
  const toggle = document.getElementById('projects-open-btn');
  toggle.textContent = 'Back to research';
  toggle.setAttribute('aria-expanded', 'true');
  if (opening) callbacks.onVisibilityChange?.(true);
  byId('page-title').focus();
  selectSection(section);

  const request = ++loadRequest;
  setLoading(true);
  showStatus('Loading projects…');
  try {
    const data = await api.listProjects();
    if (request !== loadRequest) return;
    // Keep a mutation response newer than a request that was already in flight.
    const loaded = data.projects || [];
    const prior = new Map(projects.map((project) => [project.id, project]));
    for (const project of loaded) prior.set(project.id, newerProject(prior.get(project.id), project));
    projects = [...prior.values()];
    const id = preferredId || currentProject?.id || rememberedProject();
    const selected = projects.find((project) => project.id === id) || projects[0] || null;
    setLoading(false);
    displayProject(selected);
    showStatus(selected ? '' : 'Create a project to collect its question, findings, and sources.');
  } catch (err) {
    if (request !== loadRequest) return;
    setLoading(false);
    displayProject(currentProject);
    showStatus(`Could not load projects: ${err.message}`, true);
  }
}

export function closeProjects(restoreFocus = false) {
  if (byId('overview')?.hidden !== false) return;
  byId('overview').hidden = true;
  document.getElementById('app-shell').removeAttribute('data-project-overview');
  const toggle = document.getElementById('projects-open-btn');
  toggle.textContent = 'Projects';
  toggle.setAttribute('aria-expanded', 'false');
  callbacks.onVisibilityChange?.(false);
  if (restoreFocus) toggle.focus();
}

async function selectProject(id) {
  if (!id) return;
  callbacks.onProjectSelected?.();
  const request = ++loadRequest;
  setLoading(true);
  closeEditor();
  showStatus('Loading project…');
  try {
    const project = await api.getProject(id);
    if (request !== loadRequest) return;
    setLoading(false);
    displayProject(acceptProjectUpdate(project));
    showStatus('');
  } catch (err) {
    if (request !== loadRequest) return;
    setLoading(false);
    displayProject(currentProject);
    showStatus(`Could not open project: ${err.message}`, true);
  }
}

function setLoading(value) {
  loading = value;
  byId('create-btn').disabled = loading;
  if (loading) byId('notebook').hidden = true;
}

function newerProject(previous, response) {
  return previous && previous.revision > response.revision ? previous : response;
}

function acceptProjectUpdate(response) {
  if (!response?.id) return null;
  const index = projects.findIndex((project) => project.id === response.id);
  const project = newerProject(projects[index], response);
  if (index < 0) projects.push(project);
  else projects[index] = project;
  if (currentProject?.id === project.id) displayProject(project);
  else renderPicker();
  return project;
}

function displayProject(project) {
  if (currentProject?.id !== project?.id) {
    rememberDraft();
    closeEditor();
    const draft = drafts.get(project?.id);
    byId('finding-text').value = draft?.finding || '';
    byId('finding-kind').value = draft?.kind || 'finding';
    byId('open-question-text').value = draft?.question || '';
  }
  currentProject = project;
  const backup = byId('backup-link');
  if (backup) {
    backup.hidden = !project;
    if (project) backup.href = `/api/projects/${encodeURIComponent(project.id)}/export?format=backup`;
    else backup.removeAttribute('href');
  }
  if (project) rememberProject(project.id);
  renderPicker();
  byId('notebook').hidden = !project || Boolean(edit) || loading;
  renderProjectLibrary(project);
  setValidationProject(project);
  setExperimentProject(project);
  if (!project) return;
  renderMathTextInto(byId('title'), project.title);
  renderRichInto(byId('question'), project.question || 'No research question recorded.');
  renderRichInto(byId('conventions'), project.conventions || 'No conventions pinned.');
  renderNotes(project, false);
  renderNotes(project, true);
  refreshProjectSessions();
  renderPromotion();
}

async function restoreBackup(event) {
  const file = event.target.files?.[0];
  event.target.value = '';
  if (!file) return;
  const button = byId('restore-btn');
  button.disabled = true;
  try {
    if (file.size > 60 * 1024 * 1024) throw new Error('Project backups can be up to 60 MiB.');
    showStatus('Checking the backup and restoring its project…');
    const response = await fetch('/api/projects/restore', {
      method: 'POST', credentials: 'same-origin',
      headers: { 'X-ReSearch-Token': api.getCsrfToken(), 'Content-Type': 'application/zip' },
      body: file,
    });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || 'Could not restore the project.');
    await callbacks.onSessionsChanged?.();
    await openProjects(result.id);
    showStatus(`Restored ${result.title} (${result.sessions} sessions). Reattach any manuscript folder under Research options.`);
  } catch (err) {
    showStatus(err.message, true);
  } finally {
    button.disabled = false;
  }
}

function selectSection(section) {
  for (const tab of byId('tabs').querySelectorAll('[role="tab"]')) {
    const selected = tab.dataset.projectTab === section;
    tab.setAttribute('aria-selected', String(selected));
    tab.tabIndex = selected ? 0 : -1;
    document.getElementById(tab.getAttribute('aria-controls')).hidden = !selected;
  }
  if (section === 'validation') loadProjectValidation();
  if (section === 'experiments') loadProjectExperiments();
}

function renderPicker() {
  const select = byId('select');
  select.replaceChildren();
  if (projects.length === 0) {
    select.append(option('', 'No projects'));
    select.disabled = true;
    return;
  }
  select.disabled = false;
  for (const project of projects) select.append(option(project.id, project.title));
  if (currentProject) select.value = currentProject.id;
}

function editProject(project) {
  edit = { original: project, id: project?.id };
  byId('edit-form').hidden = false;
  byId('edit-conflicts').hidden = true;
  byId('edit-save').textContent = 'Save project';
  setEditorBusy(false);
  byId('notebook').hidden = true;
  byId('title-input').value = project?.title || '';
  byId('question-input').value = project?.question || '';
  byId('conventions-input').value = project?.conventions || '';
  byId('title-input').focus();
  showStatus('');
}

function setEditorBusy(busy) {
  byId('edit-save').disabled = busy;
  for (const name of Object.keys(settingLabels)) byId(`${name}-input`).readOnly = busy;
}

function closeEditor(restoreFocus = false) {
  const prior = edit;
  edit = null;
  byId('edit-form').hidden = true;
  byId('notebook').hidden = !currentProject || loading;
  if (restoreFocus && prior) byId(prior.id ? 'edit-btn' : 'create-btn').focus();
}

async function saveProject(ev) {
  ev.preventDefault();
  const request = edit;
  const save = byId('edit-save');
  if (!request || save.disabled) return;
  let payload = {
    title: byId('title-input').value.trim(),
    question: byId('question-input').value.trim(),
    conventions: byId('conventions-input').value.trim(),
  };
  if (!payload.title) {
    showStatus('Enter a project name.', true);
    return;
  }
  setEditorBusy(true);
  try {
    let response;
    if (request.id) {
      try {
        response = await api.updateProject(request.id, { ...payload, revision: request.original.revision });
      } catch (err) {
        if (err.status !== 409 || request !== edit) throw err;
        const latest = acceptProjectUpdate(await api.getProject(request.id));
        if (request !== edit) return;
        const { values, conflicts } = reconcileEdits(request.original, payload, latest);
        request.original = latest;
        for (const [name, value] of Object.entries(values)) byId(`${name}-input`).value = value;
        showSavedValues(byId('edit-conflicts'), conflicts, latest, settingLabels);
        if (conflicts.length) {
          showStatus('The saved values changed while you were editing. Compare them with your draft before saving.', true);
          save.textContent = 'Save my edits';
          return;
        }
        payload = values;
        response = await api.updateProject(request.id, { ...payload, revision: latest.revision });
      }
    } else {
      response = await api.createProject(payload);
    }
    const project = acceptProjectUpdate(response);
    if (request === edit) {
      closeEditor();
      displayProject(project);
      const section = promotion ? 'findings' : 'question';
      selectSection(section);
      if (!byId('overview').hidden) byId(`tab-${section}`).focus();
      showStatus('');
    }
    callbacks.onNotify('Project saved.');
  } catch (err) {
    if (request === edit) {
      showStatus(err.status === 409 ? 'Another change arrived during the save. Your draft is still here. Save again to review it.' : err.message, true);
    }
  } finally {
    if (!edit || request === edit) setEditorBusy(false);
  }
}

export function refreshProjectSessions() {
  if (!currentProject) return;
  const bySession = new Map(callbacks.getSessions().map((session) => [session.id, session]));
  const linked = sessionIds(currentProject);
  const list = byId('session-list');
  list.replaceChildren();
  if (linked.length === 0) list.append(emptyItem('No sessions linked.'));
  for (const id of linked) {
    const li = document.createElement('li');
    const session = bySession.get(id);
    if (session) {
      const title = session.title || id;
      const link = action(title, () => callbacks.onOpenSession(id), 'project-session-link');
      renderMathTextInto(link, title);
      link.title = title;
      li.append(link);
    } else {
      li.textContent = 'Session unavailable';
      li.className = 'project-empty';
    }
    list.append(li);
  }

  const pick = byId('link-session-select');
  const selectedId = pick.value;
  const available = [...bySession.values()].filter((session) => !session.project_id && !linked.includes(session.id));
  pick.replaceChildren(option('', available.length ? 'Select a session' : 'No unlinked sessions'));
  for (const session of available) pick.append(option(session.id, session.title || session.id));
  if (available.some((session) => session.id === selectedId)) pick.value = selectedId;
  pick.disabled = available.length === 0;
  byId('link-session-btn').disabled = available.length === 0 || linking;
  refreshExperimentSessions();
}

async function createProjectSession() {
  const projectId = currentProject?.id;
  const button = byId('new-session');
  if (!projectId || button.disabled) return;
  button.disabled = true;
  try {
    await callbacks.onCreateSession(projectId);
  } finally {
    button.disabled = false;
  }
}

async function linkSession(ev) {
  ev.preventDefault();
  const projectId = currentProject?.id;
  const id = byId('link-session-select').value;
  const button = byId('link-session-btn');
  if (!projectId || !id || button.disabled) return;
  linking = true;
  button.disabled = true;
  showStatus('');
  try {
    acceptProjectUpdate(await api.linkProjectSession(projectId, id));
    await callbacks.onSessionsChanged();
    callbacks.onNotify('Session linked.');
  } catch (err) {
    if (currentProject?.id === projectId) showStatus(err.message, true);
  } finally {
    linking = false;
    refreshProjectSessions();
  }
}

function renderNotes(project, questions) {
  const list = byId(questions ? 'open-question-list' : 'finding-list');
  const notes = (project.items || []).filter((item) => (item.kind === 'open_question') === questions);
  list.replaceChildren();
  if (notes.length === 0) {
    list.append(emptyItem(questions ? 'No open questions recorded.' : 'No findings or failed approaches recorded.'));
  }
  for (const note of notes) {
    const li = document.createElement('li');
    li.className = 'project-note';
    if (note.kind === 'failed_approach') {
      const badge = document.createElement('span');
      badge.className = 'badge warn';
      badge.textContent = itemLabels.failed_approach;
      li.append(badge);
    }
    const body = document.createElement('div');
    body.className = 'rich-prose';
    renderRichInto(body, note.text);
    li.append(body);
    if (note.quote) {
      const quote = document.createElement('blockquote');
      renderMathTextInto(quote, note.quote);
      li.append(quote);
    }
    const actions = document.createElement('div');
    actions.className = 'project-actions';
    if (note.session_id) {
      const label = note.entry_id ? `Open proof step ${note.entry_id}` : 'Open session';
      actions.append(action(label, () => callbacks.onOpenSession(note.session_id, note.entry_id)));
    }
    if (note.source_id) {
      actions.append(action(note.location ? `Open paper · ${note.location}` : 'Open paper', () => {
        selectSection('sources');
        if (!focusProjectSource(note.source_id) && note.session_id) {
          callbacks.onOpenSourceSession?.(note.session_id, note.source_id);
        }
      }));
    }
    const remove = action('Remove', () => removeNote(project.id, note), 'project-remove-note');
    remove.setAttribute('aria-label', `Remove ${itemLabels[note.kind]?.toLowerCase() || 'note'}`);
    actions.append(remove);
    li.append(actions);
    list.append(li);
  }
}

function rememberDraft() {
  if (!currentProject) return;
  drafts.set(currentProject.id, {
    finding: byId('finding-text').value,
    kind: byId('finding-kind').value,
    question: byId('open-question-text').value,
  });
}

async function saveNote(ev, id) {
  ev.preventDefault();
  const projectId = currentProject?.id;
  const textInput = byId(`${id}-text`);
  const draftText = textInput.value;
  const text = draftText.trim();
  const button = ev.currentTarget.querySelector('[type="submit"]');
  if (!projectId || !text || button.disabled) return;
  const field = id === 'finding' ? 'finding' : 'question';
  const kind = id === 'finding' ? byId('finding-kind').value : 'open_question';
  rememberDraft();
  button.disabled = true;
  showStatus('');
  try {
    acceptProjectUpdate(await api.addProjectItem(projectId, { kind, text }));
    if (currentProject?.id === projectId && textInput.value === draftText) {
      textInput.value = '';
      rememberDraft();
    } else {
      const savedDraft = drafts.get(projectId);
      if (savedDraft?.[field] === draftText) savedDraft[field] = '';
    }
    callbacks.onNotify('Note added.');
  } catch (err) {
    if (currentProject?.id === projectId) showStatus(err.message, true);
  } finally {
    button.disabled = false;
  }
}

async function removeNote(projectId, note) {
  if (!window.confirm('Remove this project note?')) return;
  showStatus('');
  try {
    acceptProjectUpdate(await api.deleteProjectItem(projectId, note.id));
  } catch (err) {
    if (currentProject?.id === projectId) showStatus(err.message, true);
  }
}

export async function promoteProjectStep(sessionId, projectId, entry) {
  if (!sessionId || !entry?.entry_id) return;
  const failed = entry.status === 'rejected' || entry.status === 'superseded';
  const draft = { sessionId, projectId, entryId: entry.entry_id, kind: failed ? 'failed_approach' : 'finding' };
  promotion = draft;
  byId('promote-text').value = (entry.summary || entry.details || entry.entry_id).slice(0, 4000);
  byId('promote-source').value = '';
  byId('promote-location').value = '';
  byId('promote-quote').value = '';
  byId('promote-evidence').open = false;
  await openProjects(projectId, 'findings');
  if (promotion === draft && currentProject && !byId('overview').hidden && !byId('findings-panel').hidden) {
    byId('promote-text').focus();
  }
}

function renderPromotion() {
  byId('promote-form').hidden = !promotion;
  if (!promotion || !currentProject) return;
  const linked = sessionIds(currentProject).includes(promotion.sessionId);
  const otherProject = promotion.projectId && promotion.projectId !== currentProject.id;
  byId('promote-label').textContent = `Proof step ${promotion.entryId}${promotion.kind === 'failed_approach' ? ' · failed approach' : ''}`;
  const sources = currentProject.sources || [];
  const select = byId('promote-source');
  const previousSource = select.value;
  select.replaceChildren(option('', sources.length ? 'No paper selected' : 'No project papers'));
  for (const source of sources) select.append(option(source.id, source.title));
  const sameSource = sources.some((source) => source.id === previousSource);
  select.value = sameSource ? previousSource : '';
  select.disabled = sources.length === 0;
  updatePromotionEvidenceInputs();
  const button = byId('promote-save');
  button.textContent = otherProject ? 'Choose the step’s project' : linked ? 'Add linked note' : 'Link session and add note';
  button.disabled = Boolean(otherProject) || savingPromotion;
}

function updatePromotionEvidenceInputs() {
  const hasPaper = Boolean(byId('promote-source').value);
  byId('promote-location').disabled = !hasPaper;
  byId('promote-quote').disabled = !hasPaper;
}

async function savePromotion(ev) {
  ev.preventDefault();
  const projectId = currentProject?.id;
  const saved = promotion;
  const text = byId('promote-text').value.trim();
  const button = byId('promote-save');
  if (!projectId || !saved || !text || button.disabled) return;
  const sourceId = byId('promote-source').value;
  const evidence = sourceId ? {
    source_id: sourceId,
    location: byId('promote-location').value.trim() || undefined,
    quote: byId('promote-quote').value.trim() || undefined,
  } : {};
  savingPromotion = true;
  button.disabled = true;
  showStatus('');
  try {
    if (!sessionIds(currentProject).includes(saved.sessionId)) {
      const record = await api.linkProjectSession(projectId, saved.sessionId);
      saved.projectId = projectId;
      acceptProjectUpdate(record);
      await callbacks.onSessionsChanged();
    }
    acceptProjectUpdate(await api.addProjectItem(projectId, {
      kind: saved.kind,
      text,
      session_id: saved.sessionId,
      entry_id: saved.entryId,
      ...evidence,
    }));
    if (promotion === saved) promotion = null;
    callbacks.onEvidenceChanged?.(saved.sessionId);
    callbacks.onNotify('Proof step added to project.');
  } catch (err) {
    if (currentProject?.id === projectId) showStatus(err.message, true);
  } finally {
    savingPromotion = false;
    renderPromotion();
  }
}

function showStatus(text, error = false) {
  const status = byId('page-status');
  status.hidden = !text;
  status.textContent = text;
  status.dataset.error = String(error);
}

function action(label, onClick, extraClass = '') {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = `pill-btn sm ${extraClass}`;
  button.textContent = label;
  button.addEventListener('click', onClick);
  return button;
}

function option(value, text) {
  const node = document.createElement('option');
  node.value = value;
  node.textContent = text;
  return node;
}

function emptyItem(text) {
  const li = document.createElement('li');
  li.className = 'project-empty';
  li.textContent = text;
  return li;
}

function rememberedProject() {
  try { return window.localStorage.getItem('re-search-project'); } catch { return null; }
}

function rememberProject(id) {
  try { window.localStorage.setItem('re-search-project', id); } catch { /* optional */ }
}
