import * as api from '/assets/api.js';
import { escapeHtml, renderRichInto } from '/assets/math.js';
import { reconcileEdits, showSavedValues } from '/assets/project_edits.js';
import {
  extractArxivId, extractDoi, linksForPaper, renderSourceLinkChips, safeSourceUrl,
} from '/assets/source_links.js';

let project = null;
let ui = null;
let options = {};
let editing = null;
const writes = new Map();
const paperLabels = {
  title: 'Title', reference: 'DOI, arXiv ID, or paper URL', location: 'Location',
  quote: 'Excerpt', notes: 'Notes', bibtex: 'BibTeX',
};

const formHtml = `
  <label class="project-library-field">
    Title <input class="text-input" name="title" required maxlength="256">
  </label>
  <label class="project-library-field">
    DOI, arXiv ID, or paper URL
    <input class="text-input" name="reference" maxlength="2048" placeholder="10.1103/PhysRev.47.777">
  </label>
  <label class="project-library-field">
    Location <input class="text-input" name="location" maxlength="256" placeholder="§ 3, p. 8, Eq. (12)">
  </label>
  <label class="project-library-field">
    Excerpt <textarea class="text-input" name="quote" rows="2" maxlength="3000"></textarea>
  </label>
  <label class="project-library-field">
    Notes <textarea class="text-input" name="notes" rows="2" maxlength="4000"></textarea>
  </label>
  <label class="project-library-field">
    BibTeX <textarea class="text-input project-library-bibtex" name="bibtex" rows="3" maxlength="8192" spellcheck="false"></textarea>
  </label>
  <details class="project-conflicts" hidden><summary>Compare saved values</summary><dl></dl></details>
  <div class="project-library-actions">
    <button type="submit" class="pill-btn sm primary">Save paper</button>
    <button type="button" class="pill-btn sm" data-cancel>Cancel</button>
  </div>
  <p class="project-library-status" role="status" hidden></p>
`;

export function normalizeProjectReference(raw) {
  const text = String(raw || '').trim();
  if (!text) return { url: '', doi: '' };
  let doiInput = text;
  if (/^https?:\/\//i.test(text)) {
    if (/[\s\x00-\x1f\x7f\\<>"']/.test(text)) return null;
    try {
      const url = new URL(text);
      if (url.username || url.password || url.href.length > 2048) return null;
      if (!['doi.org', 'dx.doi.org'].includes(url.hostname)) return { url: url.href, doi: '' };
      doiInput = decodeURIComponent(url.pathname.slice(1));
    } catch {
      return null;
    }
  }
  const doi = extractDoi(doiInput);
  if (doi) {
    const encoded = doi.split('/').map(encodeURIComponent).join('/');
    const doiUrl = safeSourceUrl(`https://doi.org/${encoded}`);
    return doiUrl ? { url: doiUrl, doi } : null;
  }
  const identifier = text.replace(/^arxiv:\s*/i, '');
  const arxiv = extractArxivId(identifier);
  if (arxiv && arxiv === identifier) {
    return { url: `https://arxiv.org/abs/${arxiv}`, doi: '' };
  }
  return null;
}

function paperLinks(source) {
  const url = safeSourceUrl(source.url);
  const parsed = url && new URL(url);
  const arxivId = parsed && ['arxiv.org', 'export.arxiv.org'].includes(parsed.hostname)
    ? extractArxivId(parsed.pathname)
    : null;
  const links = linksForPaper({ url, arxiv_id: arxivId });
  const reference = source.doi ? normalizeProjectReference(source.doi) : null;
  if (reference?.doi && !links.some((link) => link.url === reference.url)) {
    links.push({ label: 'DOI', url: reference.url, kind: 'doi' });
  }
  return links;
}

export function filterProjectSources(sources, query) {
  const words = query.trim().toLowerCase().replace(/\b(?:doi|arxiv):/g, ' ').split(/\s+/).filter(Boolean);
  if (words.length === 0) return sources;
  return sources.filter((source) => {
    const text = [
      source.title, source.url, source.doi, source.bibtex, source.location,
      source.quote, source.notes, source.session_id, source.source_id,
    ].join('\n').toLowerCase();
    return words.every((word) => text.includes(word));
  });
}

export function initProjectLibrary(opts = {}) {
  options = opts;
  const panel = document.getElementById('project-sources-panel');
  if (!panel) return;
  panel.innerHTML = `
    <div class="project-library-toolbar">
      <input type="search" class="text-input" aria-label="Search project papers" placeholder="Search papers, notes, or citations">
      <div class="project-library-actions">
        <button type="button" class="pill-btn sm primary" data-add-paper>Add paper</button>
        <a class="pill-btn sm" data-export-bib download hidden>Export BibTeX</a>
      </div>
    </div>
    <form class="project-library-form" hidden>${formHtml}</form>
    <p class="project-library-status" data-paper-count aria-live="polite"></p>
    <div class="project-library-list"></div>
  `;
  ui = {
    panel,
    search: panel.querySelector('input[type=search]'),
    form: panel.querySelector('form'),
    add: panel.querySelector('[data-add-paper]'),
    export: panel.querySelector('[data-export-bib]'),
    count: panel.querySelector('[data-paper-count]'),
    list: panel.querySelector('.project-library-list'),
  };
  ui.search.addEventListener('input', renderPapers);
  ui.add.addEventListener('click', () => showEditor(null));
  ui.form.querySelector('[data-cancel]').addEventListener('click', () => {
    const sourceId = editing?.source?.id;
    hideEditor();
    focusAfterEdit(sourceId);
  });
  ui.form.addEventListener('submit', saveEditor);
  renderProjectLibrary(project);
}

export function renderProjectLibrary(nextProject) {
  const changed = project?.id !== nextProject?.id;
  project = nextProject;
  if (!ui) return;
  if (changed) {
    ui.search.value = '';
    hideEditor();
  }
  ui.add.disabled = !project;
  renderPapers();
}

function renderPapers() {
  const sources = project?.sources || [];
  const visible = filterProjectSources(sources, ui.search.value);
  ui.list.replaceChildren(...visible.map(renderPaper));
  ui.count.textContent = !sources.length
    ? 'No papers saved yet.'
    : visible.length === sources.length
      ? `${sources.length} ${sources.length === 1 ? 'paper' : 'papers'}`
      : `${visible.length} of ${sources.length} papers`;
  ui.export.hidden = !sources.some((source) => source.bibtex?.trim());
  if (!ui.export.hidden) ui.export.href = `/api/projects/${encodeURIComponent(project.id)}/export?format=bib`;
}

function renderPaper(source) {
  const card = document.createElement('article');
  card.className = 'paper-card project-library-paper';
  card.dataset.sourceId = source.id;
  card.tabIndex = -1;
  card.innerHTML = `
    <h3>${escapeHtml(source.title)}</h3>
    ${source.url && !safeSourceUrl(source.url) ? `<p class="project-library-locator">${escapeHtml(source.url)}</p>` : ''}
    ${source.location ? `<p class="project-library-locator">${escapeHtml(source.location)}</p>` : ''}
    ${source.quote ? `<blockquote>${escapeHtml(source.quote)}</blockquote>` : ''}
    ${source.notes ? '<div class="project-library-notes rich-prose"></div>' : ''}
    <div class="project-library-actions">
      ${renderSourceLinkChips(paperLinks(source))}
      ${source.session_id && options.onOpenSession ? '<button type="button" class="pill-btn sm" data-open-source>Open source session</button>' : ''}
      <button type="button" class="pill-btn sm" aria-label="Edit ${escapeHtml(source.title)}" data-edit-paper>Edit</button>
      <button type="button" class="pill-btn sm" aria-label="Remove ${escapeHtml(source.title)}" data-remove-paper>Remove</button>
    </div>
  `;
  if (source.notes) renderRichInto(card.querySelector('.project-library-notes'), source.notes);
  card.querySelector('[data-open-source]')?.addEventListener('click', () => options.onOpenSession(source.session_id, source.source_id));
  card.querySelector('[data-edit-paper]').addEventListener('click', () => showEditor(source));
  const remove = card.querySelector('[data-remove-paper]');
  remove.addEventListener('click', async () => {
    if (!project || !window.confirm(`Remove “${source.title}” from the project library?`)) return;
    const { id } = project;
    remove.disabled = true;
    try {
      await updateProject(id, () => api.deleteProjectSource(id, source.id, source.revision));
      if (project?.id === id && editing?.source?.id === source.id) hideEditor();
      if (project?.id === id && document.activeElement === document.body) ui.add.focus();
    } catch (err) {
      if (err.status === 409 && project?.id === id) {
        try {
          await updateProject(id, () => api.getProject(id));
          options.onNotify?.('This paper changed. Review the updated paper before removing it.');
          return;
        } catch (refreshError) {
          options.onNotify?.(`Could not refresh the changed paper. ${refreshError.message}`);
          return;
        }
      }
      options.onNotify?.(`Could not remove the paper. ${err.message}`);
    } finally {
      remove.disabled = false;
    }
  });
  return card;
}

export function focusProjectSource(sourceId) {
  if (!ui) return false;
  if (ui.search.value) {
    ui.search.value = '';
    renderPapers();
  }
  const card = Array.from(ui.list.children).find((candidate) => candidate.dataset.sourceId === sourceId);
  if (!card) return false;
  card.focus();
  card.scrollIntoView({ block: 'nearest' });
  return true;
}

function showEditor(source) {
  editing = { source };
  ui.form.reset();
  writeFields(paperFields(source));
  ui.form.querySelector('details').hidden = true;
  ui.form.querySelector('[type=submit]').textContent = 'Save paper';
  setEditorBusy(false);
  ui.form.querySelector('[role=status]').hidden = true;
  ui.form.hidden = false;
  ui.form.elements.namedItem('title').focus();
}

function paperFields(source) {
  const fields = {};
  for (const name of Object.keys(paperLabels)) fields[name] = source?.[name] || '';
  fields.reference = source?.url || source?.doi || '';
  return fields;
}

function writeFields(fields) {
  for (const [name, value] of Object.entries(fields)) ui.form.elements.namedItem(name).value = value;
}

function setEditorBusy(busy) {
  ui.form.querySelector('[type=submit]').disabled = busy;
  for (const name of Object.keys(paperLabels)) ui.form.elements.namedItem(name).readOnly = busy;
}

function focusAfterEdit(sourceId) {
  const card = Array.from(ui.list.children).find((candidate) => candidate.dataset.sourceId === sourceId);
  (card?.querySelector('[data-edit-paper]') || ui.add).focus();
}

function hideEditor() {
  editing = null;
  ui.form.hidden = true;
}

async function saveEditor(event) {
  event.preventDefault();
  if (!project || !editing) return;
  const { id } = project;
  const editor = editing;
  let draft = Object.fromEntries(new FormData(ui.form));
  const status = ui.form.querySelector('[role=status]');
  if (!normalizeProjectReference(draft.reference)) {
    status.textContent = 'Enter a DOI, an arXiv ID, or an HTTP or HTTPS paper address.';
    status.hidden = false;
    return;
  }
  const submit = ui.form.querySelector('[type=submit]');
  if (submit.disabled) return;
  setEditorBusy(true);
  const save = () => {
    const { reference, ...textFields } = draft;
    const ref = normalizeProjectReference(reference);
    if (!ref) throw new Error('The saved paper address is invalid. Update it before saving.');
    const source = editor.source;
    if (source?.url && reference.trim() === source.url && source.doi) ref.doi = source.doi;
    const fields = { ...textFields, ...ref };
    const payload = source
      ? { ...fields, id: source.id, revision: source.revision }
      : { ...Object.fromEntries(Object.entries(fields).filter(([, value]) => value.trim())), create_only: true };
    return updateProject(id, () => api.saveProjectSource(id, payload));
  };
  try {
    try {
      await save();
    } catch (err) {
      if (!editor.source || editor !== editing || ![404, 409].includes(err.status)) throw err;
      const latestProject = await api.getProject(id);
      if (editor !== editing) return;
      if (options.onChanged) options.onChanged(latestProject);
      else renderProjectLibrary(latestProject);
      const latest = latestProject.sources.find((source) => source.id === editor.source.id);
      if (!latest) {
        editor.source = null;
        ui.form.querySelector('details').hidden = true;
        submit.textContent = 'Save as new paper';
        status.textContent = 'This paper was removed. Your draft can be saved as a new paper.';
        status.hidden = false;
        return;
      }
      if (editor.source.revision === latest.revision) throw err;
      const fields = paperFields(latest);
      const { values, conflicts } = reconcileEdits(paperFields(editor.source), draft, fields);
      editor.source = latest;
      draft = values;
      writeFields(values);
      showSavedValues(ui.form.querySelector('details'), conflicts, fields, paperLabels);
      if (conflicts.length) {
        submit.textContent = 'Save my edits';
        status.textContent = 'The saved values changed while you were editing. Compare them with your draft before saving.';
        status.hidden = false;
        return;
      }
      await save();
    }
    if (project?.id === id && editing === editor) {
      hideEditor();
      focusAfterEdit(editor.source?.id);
      options.onNotify?.('Paper saved.');
    }
  } catch (err) {
    if (project?.id !== id || editing !== editor) return;
    status.textContent = err.message;
    status.hidden = false;
  } finally {
    if (editing === editor) setEditorBusy(false);
  }
}

async function updateProject(id, write) {
  const pending = (writes.get(id) || Promise.resolve()).catch(() => {}).then(write);
  writes.set(id, pending);
  try {
    const updated = await pending;
    if (options.onChanged) options.onChanged(updated);
    else if (project?.id === id) renderProjectLibrary(updated);
    return updated;
  } finally {
    if (writes.get(id) === pending) writes.delete(id);
  }
}

export async function addSessionSourceToLibrary(projectId, source) {
  await updateProject(projectId, () => api.saveProjectSource(projectId, source));
  options.onNotify?.('Paper saved to the project library.');
}
