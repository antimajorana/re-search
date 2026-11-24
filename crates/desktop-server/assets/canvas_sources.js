// Tabs 3 & 4: Literature & Sources Canvas (arXiv/INSPIRE/Crossref/S2) and isolated Explain Canvas.
import * as api from '/assets/api.js';
import { escapeHtml, mathTextToHtml, renderLatexDocumentPreview, renderMathTextInto, renderRichInto } from '/assets/math.js';
import { getModelChoice } from '/assets/model_picker.js';
import {
  extractDoi, linksForIngestedSource, linksForPaper, renderSourceLinkChips,
  resolveOriginInfo, resolveSourceTitle, sessionPaperReferences, sessionSourcesUsed,
} from '/assets/source_links.js';

// The model picker's current {provider, model}. On the Claude subscription route the
// server picks the first connected API provider (narration and side questions need one).
function sideModelPayload() {
  const c = getModelChoice();
  return c ? { provider: c.provider, model: c.model } : {};
}

let litFormEl = null;
let litSourceEl = null;
let litQueryEl = null;
let litFieldEl = null;
let litIncludeEl = null;
let litFilterEl = null;
let litResultsEl = null;
let sessionLiteratureEl = null;
let sessionLiteratureListEl = null;
let usedSourcesListEl = null;
let sourcesListEl = null;
let passageFormEl = null;
let passageQueryEl = null;
let passageStatusEl = null;
let passageResultsEl = null;
let sidetalkStreamEl = null;
let sidetalkFormEl = null;
let sidetalkInputEl = null;
let currentSessionId = null;
let projectLibraryTarget = null;
let sessionRevision = 0;
let sourcesRequest = 0;
let passageRequest = 0;
let literatureRequest = 0;
let sidetalkRequest = 0;
let sidetalkHistory = Promise.resolve();
let notifyFn = () => {};
let onSelectStepRef = () => {};
let onCountsChange = () => {};
let callbacks = {};
let lastSourcesCount = 0;
let lastSidetalkCount = 0;

export function initSourcesAndSidetalk(opts = {}) {
  callbacks = opts;
  notifyFn = opts.onNotify || (() => {});
  onSelectStepRef = opts.onSelectStepRef || (() => {});
  onCountsChange = opts.onCountsChange || (() => {});
  litFormEl = document.getElementById('literature-search-form');
  litSourceEl = document.getElementById('lit-source-select');
  litQueryEl = document.getElementById('lit-query-input');
  litFieldEl = document.getElementById('lit-field-select');
  litIncludeEl = document.getElementById('lit-include-source');
  litFilterEl = document.getElementById('lit-section-filter');
  litResultsEl = document.getElementById('literature-results');
  sessionLiteratureEl = document.getElementById('session-literature');
  sessionLiteratureListEl = document.getElementById('session-literature-list');
  usedSourcesListEl = document.getElementById('used-sources-list');
  sourcesListEl = document.getElementById('sources-list');
  passageFormEl = document.getElementById('paper-passage-form');
  passageQueryEl = document.getElementById('paper-passage-query');
  passageStatusEl = document.getElementById('paper-passage-status');
  passageResultsEl = document.getElementById('paper-passage-results');
  sidetalkStreamEl = document.getElementById('sidetalk-stream');
  sidetalkFormEl = document.getElementById('sidetalk-form');
  sidetalkInputEl = document.getElementById('sidetalk-input');

  if (litFormEl) {
    litFormEl.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      await handleLiteratureSearch();
    });
  }
  litSourceEl?.addEventListener('change', syncLiteratureFields);
  syncLiteratureFields();
  passageFormEl?.addEventListener('submit', async (event) => {
    event.preventDefault();
    await searchPassages();
  });
  if (sidetalkFormEl) {
    sidetalkFormEl.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      await handleSidetalkSubmit();
    });
  }
  const exportMdBtn = document.getElementById('explain-export-md-btn');
  const exportHtmlBtn = document.getElementById('explain-export-html-btn');
  const triggerExplainExport = (format) => {
    if (!currentSessionId) {
      notifyFn('Select or create a session before exporting Explain.');
      return;
    }
    window.location.assign(api.exportUrl(currentSessionId, format));
  };
  if (exportMdBtn) exportMdBtn.addEventListener('click', () => triggerExplainExport('explain_md'));
  if (exportHtmlBtn) exportHtmlBtn.addEventListener('click', () => triggerExplainExport('explain_html'));
  wireSourceReaderModal();
}

export async function refreshSourcesAndSidetalk(sessionId) {
  if (sessionId !== currentSessionId) {
    currentSessionId = sessionId;
    sessionRevision += 1;
    for (const container of [sourcesListEl, usedSourcesListEl, sidetalkStreamEl, litResultsEl, sessionLiteratureListEl, passageStatusEl, passageResultsEl]) {
      if (container) container.replaceChildren();
    }
    if (sessionLiteratureEl) {
      sessionLiteratureEl.hidden = true;
      sessionLiteratureEl.open = false;
    }
    if (passageQueryEl) passageQueryEl.value = '';
    if (sidetalkInputEl) sidetalkInputEl.value = '';
    lastSourcesCount = 0;
    lastSidetalkCount = 0;
    onCountsChange({ sources: 0, sidetalk: 0 });
    readerOpenRequest += 1;
    document.getElementById('source-reader-dialog')?.close();
    readerState.sessionId = null;
  }
  if (!sessionId) return;
  sidetalkHistory = loadSidetalk(sessionId);
  await Promise.all([loadSessionSources(sessionId), sidetalkHistory]);
}

const passageOmissionLabels = {
  byte_limit: 'over scan budget',
  empty_text: 'without readable text',
  file_limit: 'over file count limit',
  file_size_limit: 'over file size limit',
  generated_view: 'generated views',
  hash_mismatch: 'with changed content',
  invalid_text: 'invalid text',
  path_size_limit: 'paths over size limit',
  source_not_ingested: 'unavailable sources',
  time_limit: 'outside search time',
  unavailable_file: 'unavailable files',
  unsupported_file: 'unsupported file types',
  unverified_file: 'without a recorded file hash',
  unverified_receipt: 'without a verified file list',
};

async function searchPassages() {
  if (!passageStatusEl || !passageResultsEl) return;
  const sessionId = currentSessionId;
  const query = passageQueryEl?.value.trim();
  if (!sessionId) {
    passageStatusEl.textContent = 'Select a session to search its ingested papers.';
    return;
  }
  if (!query) return;
  const revision = sessionRevision;
  const request = ++passageRequest;
  passageResultsEl.replaceChildren();
  passageStatusEl.textContent = 'Searching ingested files…';
  try {
    const result = await api.searchPaperPassages(sessionId, query);
    if (revision !== sessionRevision || request !== passageRequest) return;
    const passages = Array.isArray(result.passages) ? result.passages : [];
    const coverage = result.coverage || {};
    const files = Number(coverage.files_searched) || 0;
    const found = Number(coverage.matching_passages) || 0;
    const searched = `${files} searched ${files === 1 ? 'file' : 'files'}`;
    const outcome = passages.length
      ? `Showing ${passages.length} of ${found} matching passages across ${searched}.`
      : files ? `No matching passages in ${searched}.` : 'No eligible files were searched.';
    const omitted = Object.entries(coverage.omissions || {}).map(([reason, count]) =>
      `${count} ${passageOmissionLabels[reason] || 'other omitted items'}`
    );
    passageStatusEl.textContent = omitted.length ? `${outcome} Omitted ${omitted.join(', ')}.` : outcome;
    passageResultsEl.replaceChildren(...passages.map((passage) => renderPassage(passage, sessionId)));
  } catch (err) {
    if (revision !== sessionRevision || request !== passageRequest) return;
    passageStatusEl.textContent = `Passage search failed. ${err.message}`;
  }
}

function renderPassage(passage, sessionId) {
  const result = document.createElement('article');
  result.className = 'paper-passage-result';
  const title = document.createElement('strong');
  renderMathTextInto(title, passage.title || passage.source_id);
  const location = document.createElement('div');
  location.className = 'paper-passage-meta';
  const lines = passage.start_line === passage.end_line
    ? `L${passage.start_line}`
    : `L${passage.start_line}–L${passage.end_line}`;
  const locator = `${passage.path}:${lines}`;
  location.textContent = locator;
  const excerpt = document.createElement('pre');
  excerpt.className = 'source-raw-pre';
  renderMathTextInto(excerpt, passage.excerpt);

  const provenance = document.createElement('details');
  provenance.className = 'paper-passage-meta';
  const summary = document.createElement('summary');
  summary.textContent = 'Provenance';
  const fileHash = document.createElement('div');
  fileHash.textContent = `File SHA-256 ${passage.file_sha256}`;
  const sourceHash = document.createElement('div');
  sourceHash.textContent = `Original SHA-256 ${passage.source_sha256}`;
  const receipt = document.createElement('div');
  receipt.textContent = `Source ${passage.source_id} · ingestion event #${passage.source_seq} · bytes ${passage.start_byte}–${passage.end_byte} (end exclusive)`;
  provenance.append(summary, fileHash, sourceHash, receipt);

  const actions = document.createElement('div');
  actions.className = 'paper-passage-controls';
  const open = document.createElement('button');
  open.type = 'button';
  open.className = 'pill-btn sm';
  open.textContent = 'Open cited lines';
  open.addEventListener('click', () => openSessionSource(sessionId, passage.source_id, passage.path, passage));
  const copy = document.createElement('button');
  copy.type = 'button';
  copy.className = 'pill-btn sm';
  copy.textContent = 'Copy citation';
  copy.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(
        `${title.textContent}\n${locator} · bytes ${passage.start_byte}–${passage.end_byte} (end exclusive)\n`
        + `File SHA-256 ${passage.file_sha256}\n\n${passage.excerpt}`
      );
      notifyFn('Copied the passage and its citation.');
    } catch {
      notifyFn('Clipboard unavailable.');
    }
  });
  actions.append(open, copy);
  result.append(title, location, excerpt, provenance, actions);
  return result;
}

export function setProjectLibraryTarget(projectId) {
  projectLibraryTarget = projectId;
  for (const list of [litResultsEl, sourcesListEl]) {
    for (const button of list?.querySelectorAll('.save-project-source') || []) {
      button.hidden = !projectLibraryTarget;
    }
  }
}

function wireProjectLibraryButton(card, sessionId, sourceId, paper) {
  const button = card.querySelector('.save-project-source');
  if (!button) return;
  button.hidden = !projectLibraryTarget;
  button.addEventListener('click', async () => {
    const projectId = projectLibraryTarget;
    if (!projectId || sessionId !== currentSessionId || !callbacks.onAddToProjectLibrary) return;
    button.disabled = true;
    try {
      await callbacks.onAddToProjectLibrary(projectId, {
        ...paper,
        title: Array.from(paper.title).slice(0, 256).join(''),
        session_id: sessionId,
        source_id: sourceId,
      });
    } catch (err) {
      notifyFn(`Could not save the paper. ${err.message}`);
    } finally {
      button.disabled = false;
    }
  });
}

function syncLiteratureFields() {
  const isArxiv = !litSourceEl || litSourceEl.value === 'arxiv';
  for (const field of [litFieldEl, litIncludeEl, litFilterEl]) {
    if (field) field.disabled = !isArxiv;
  }
}

async function handleLiteratureSearch() {
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  if (!sessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  const query = litQueryEl ? litQueryEl.value.trim() : '';
  if (!query) return;
  const request = ++literatureRequest;
  const source = litSourceEl?.value || 'arxiv';
  const isArxiv = source === 'arxiv';

  const payload = {
    source,
    query,
    field: isArxiv ? (litFieldEl?.value || 'all') : undefined,
    include_source: isArxiv && Boolean(litIncludeEl?.checked),
    section_filter: isArxiv ? (litFilterEl?.value.trim() || undefined) : undefined,
  };

  try {
    if (litResultsEl) litResultsEl.textContent = 'Searching literature…';
    const resp = await api.searchLiterature(sessionId, payload);
    if (revision !== sessionRevision || request !== literatureRequest) return;
    const items = Array.isArray(resp) ? resp : resp.results || resp.papers || resp.hits || [];
    renderLiteratureHits(items, sessionId, payload);
  } catch (err) {
    if (revision === sessionRevision && request === literatureRequest && litResultsEl) {
      litResultsEl.textContent = `Literature search failed. ${err.message}`;
    }
  } finally {
    if (revision === sessionRevision) await loadSessionSources(sessionId);
  }
}

function renderLiteratureHits(items, sessionId, payload, container = litResultsEl) {
  if (!container) return;
  container.replaceChildren();
  if (!items || items.length === 0) {
    container.innerHTML = '<div class="paper-card" style="color:var(--ink-muted)">No matching papers returned.</div>';
    return;
  }
  for (const item of items) {
    const card = document.createElement('article');
    card.className = 'paper-card';
    const withdrawn = Boolean(item.withdrawn || item.retracted);
    const authors = Array.isArray(item.authors) ? item.authors.join(', ') : '';
    const meta = [item.id || item.arxiv_id, item.doi ? `DOI ${item.doi}` : '', item.year].filter(Boolean).join(' · ');
    const bibtex = item.bibtex || '';
    const sections = item.source && Array.isArray(item.source.sections) ? item.source.sections : [];
    const ingestedId = !withdrawn && item.source?.source_id;
    const readable = ingestedId && item.source.source_available && sourceFiles(item.source).length > 0;
    const activeSrc = typeof item.source === 'string' ? item.source : payload.source;
    const links = linksForPaper({ ...item, source: activeSrc });
    const titleHtml = links[0]
      ? `<a class="paper-title-link" href="${escapeHtml(links[0].url)}" target="_blank" rel="noopener noreferrer">${mathTextToHtml(item.title || 'Untitled')} ↗</a>`
      : mathTextToHtml(item.title || 'Untitled');

    card.innerHTML = `
      ${withdrawn ? '<div class="retraction-banner">Warning: Withdrawn or Retracted Publication — blocked from unflagged citation</div>' : ''}
      <div style="font-weight:600;margin-bottom:3px;">${titleHtml}</div>
      <div style="font-size:12px;color:var(--ink-secondary);margin-bottom:4px;">${escapeHtml(authors)}</div>
      <div style="font-size:11.5px;color:var(--ink-muted);margin-bottom:8px;">${escapeHtml(meta)}</div>
      ${item.abstract_text ? `<div style="font-size:12px;color:var(--ink-secondary);margin-bottom:8px;">${mathTextToHtml(item.abstract_text)}</div>` : ''}
      ${sections.length ? `<div style="font-size:11.5px;margin-bottom:8px;"><strong>TeX sections:</strong> ${sections.map((s) => `<code>${escapeHtml(s)}</code>`).join(' ')}</div>` : ''}
      ${payload.include_source && !withdrawn && !readable ? '<p class="paper-passage-meta">No readable TeX source was ingested for this result.</p>' : ''}
      <div style="display:flex;gap:6px;flex-wrap:wrap;align-items:center;">
        ${renderSourceLinkChips(links)}
        ${readable ? '<button type="button" class="pill-btn sm read-paper-btn">Read source</button>' : ''}
        ${bibtex ? '<button type="button" class="pill-btn sm copy-bib-btn">Copy BibTeX</button>' : ''}
        ${ingestedId ? '<button type="button" class="pill-btn sm save-project-source" hidden>Add to Project Library</button>' : ''}
      </div>
    `;
    card.querySelector('.read-paper-btn')?.addEventListener('click', () => openSessionSource(sessionId, ingestedId));
    const copyBtn = card.querySelector('.copy-bib-btn');
    if (copyBtn) {
      copyBtn.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(bibtex);
          notifyFn('BibTeX copied to clipboard.');
        } catch {
          notifyFn('Could not copy BibTeX.');
        }
      });
    }
    if (ingestedId) {
      wireProjectLibraryButton(card, sessionId, ingestedId, {
        title: item.title || 'Untitled paper',
        url: links[0]?.url,
        doi: extractDoi(item.doi) || undefined,
        bibtex: bibtex || undefined,
      });
    }
    container.appendChild(card);
  }
}

export async function loadSessionSources(sessionId) {
  if (!sourcesListEl || !sessionId || sessionId !== currentSessionId) return;
  const revision = sessionRevision;
  const request = ++sourcesRequest;
  try {
    const sess = await api.getSession(sessionId);
    if (revision !== sessionRevision || request !== sourcesRequest) return;
    const sources = Array.isArray(sess && sess.sources) ? sess.sources : [];
    const papers = sessionPaperReferences(sess?.events);
    const used = sessionSourcesUsed(sess);
    renderUsedSources(used, sessionId);
    if (sessionLiteratureEl) {
      sessionLiteratureEl.hidden = papers.length === 0;
      sessionLiteratureEl.querySelector('summary').textContent = `Search History (${papers.length})`;
      if (papers.length) renderLiteratureHits(papers, sessionId, {}, sessionLiteratureListEl);
      else sessionLiteratureListEl?.replaceChildren();
    }
    lastSourcesCount = used.length;
    onCountsChange({ sources: lastSourcesCount, sidetalk: lastSidetalkCount });
    sourcesListEl.innerHTML = '';
    if (sources.length === 0) {
      sourcesListEl.innerHTML = '<div class="source-card" style="color:var(--ink-muted)">Choose Attach beside your message to add a file, or select Save TeX source when searching arXiv.</div>';
      return;
    }
    for (const src of sources) {
      const card = document.createElement('div');
      card.className = 'source-card';
      const rawFiles = Array.isArray(src.files) ? src.files : [];
      const readableFiles = sourceFiles(src);
      const files = rawFiles.join(', ');
      const titleText = resolveSourceTitle(src);
      const originInfo = resolveOriginInfo(src);
      const extLinks = linksForIngestedSource(src);
      const extraLinks = originInfo.url
        ? extLinks.filter((lk) => lk.url !== originInfo.url)
        : extLinks;
      const originValueHtml = originInfo.url
        ? `<a class="source-origin-link" href="${escapeHtml(originInfo.url)}" target="_blank" rel="noopener noreferrer">${escapeHtml(originInfo.text)} ↗</a>`
        : `<code>${escapeHtml(originInfo.text)}</code>`;
      card.innerHTML = `
        <div style="display:flex;justify-content:space-between;gap:8px;align-items:flex-start;">
          <strong class="source-card-title">${mathTextToHtml(titleText)}</strong>
          <span class="badge">${escapeHtml(src.format || src.status || 'ingested')}</span>
        </div>
        ${originInfo.text ? `<div class="source-origin-row"><span>Origin:</span>${originValueHtml}</div>` : ''}
        ${files ? `<div style="font-size:11.5px;color:var(--ink-muted);margin-top:4px;">Files: ${escapeHtml(files)}</div>` : ''}
        <div style="display:flex;gap:6px;flex-wrap:wrap;align-items:center;margin-top:8px;">
          ${readableFiles.length ? '<button type="button" class="pill-btn sm primary read-source-btn">Read Source</button>' : ''}
          ${renderSourceLinkChips(extraLinks)}
          ${src.source_id ? '<button type="button" class="pill-btn sm save-project-source" hidden>Add to Project Library</button>' : ''}
        </div>
      `;
      const readBtn = card.querySelector('.read-source-btn');
      if (readBtn) {
        readBtn.addEventListener('click', () => {
          readerOpenRequest += 1;
          openSourceReader(sessionId, src, readableFiles, extLinks);
        });
      }
      if (src.source_id) {
        wireProjectLibraryButton(card, sessionId, src.source_id, {
          title: titleText,
          url: originInfo.url || extLinks[0]?.url,
          doi: extractDoi(src.origin) || undefined,
          bibtex: src.bibtex || undefined,
        });
      }
      sourcesListEl.appendChild(card);
    }
  } catch (error) {
    if (revision === sessionRevision && request === sourcesRequest) {
      sourcesListEl.textContent = `Could not load session sources. ${error.message}`;
      if (usedSourcesListEl) usedSourcesListEl.textContent = 'Could not load sources used in this session.';
      lastSourcesCount = 0;
      onCountsChange({ sources: 0, sidetalk: lastSidetalkCount });
    }
  }
}

function renderUsedSources(records, sessionId) {
  if (!usedSourcesListEl) return;
  usedSourcesListEl.replaceChildren();
  if (!records.length) {
    usedSourcesListEl.textContent = 'Citations and proof-linked sources will appear here.';
    return;
  }
  for (const { paper, source, readers, steps, cited } of records) {
    const card = document.createElement('article');
    card.className = 'paper-card';
    const links = linksForPaper(paper);
    const title = mathTextToHtml(paper.title);
    const titleHtml = links[0]
      ? `<a class="paper-title-link" href="${escapeHtml(links[0].url)}" target="_blank" rel="noopener noreferrer">${title} ↗</a>`
      : title;
    card.innerHTML = `
      ${paper.withdrawn || paper.retracted ? '<div class="retraction-banner">Withdrawn or retracted publication</div>' : ''}
      <strong>${titleHtml}</strong>
      <div class="used-source-actions" style="display:flex;gap:6px;flex-wrap:wrap;align-items:center;margin-top:8px;">
        ${cited ? '<span class="badge">Cited in answer</span>' : ''}
        ${renderSourceLinkChips(links)}
      </div>
    `;
    const actions = card.querySelector('.used-source-actions');
    const addButton = (label, onClick) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'pill-btn sm';
      button.textContent = label;
      button.addEventListener('click', onClick);
      actions.appendChild(button);
    };
    for (const reader of readers.values()) {
      let label = reader.start_line ? 'Read cited lines' : 'Read source';
      if (readers.size > 1) {
        const file = reader.path?.split('/').pop() || reader.source_id;
        label = reader.start_line ? `${file} · L${reader.start_line}–${reader.end_line ?? reader.start_line}` : file;
      }
      addButton(label, () => openSessionSource(sessionId, reader.source_id, reader.path, reader));
    }
    if (!readers.size && source?.source_id && sourceFiles(source).length) {
      addButton('Read source', () => openSessionSource(sessionId, source.source_id));
    }
    for (const step of steps) {
      if (step) addButton(`Proof ${step}`, () => onSelectStepRef(step));
    }
    usedSourcesListEl.appendChild(card);
  }
}

function sourceFiles(source) {
  const files = Array.isArray(source.files) ? source.files : [];
  return Array.from(new Set([source.assembled_path, source.primary_path, ...files].filter(Boolean)));
}

export async function openSessionSource(sessionId, sourceId, preferredPath, anchor) {
  if (!sourceId || sessionId !== currentSessionId) return false;
  const revision = sessionRevision;
  const request = ++readerOpenRequest;
  const opener = document.activeElement;
  try {
    const data = await api.getSession(sessionId);
    if (revision !== sessionRevision || request !== readerOpenRequest) return false;
    const source = data?.sources?.findLast((item) => item.source_id === sourceId
      && (!preferredPath || sourceFiles(item).includes(preferredPath)));
    const files = source ? sourceFiles(source) : [];
    if (files.length === 0) {
      notifyFn('This source file is unavailable.');
      return false;
    }
    if (preferredPath) {
      files.splice(files.indexOf(preferredPath), 1);
      files.unshift(preferredPath);
    }
    return await openSourceReader(sessionId, source, files, linksForIngestedSource(source), anchor, opener);
  } catch (err) {
    if (revision === sessionRevision && request === readerOpenRequest) {
      notifyFn(`Could not open the source. ${err.message}`);
    }
    return false;
  }
}

async function loadSidetalk(sessionId) {
  if (!sidetalkStreamEl || !sessionId) return;
  const revision = sessionRevision;
  const request = ++sidetalkRequest;
  try {
    const data = await api.getSidetalk(sessionId);
    if (revision !== sessionRevision || request !== sidetalkRequest) return;
    const turns = Array.isArray(data) ? data : data.turns || data.messages || [];
    lastSidetalkCount = turns.length;
    onCountsChange({ sources: lastSourcesCount, sidetalk: lastSidetalkCount });
    sidetalkStreamEl.innerHTML = '';
    if (turns.length === 0) {
      sidetalkStreamEl.innerHTML = '<div style="color:var(--ink-muted);font-size:13px;">Ask Explain clarifying questions here without altering the verified proof chain.</div>';
      return;
    }
    for (const t of turns) {
      appendSidetalkPair(t.question || t.q || '', t.answer || t.a || '');
    }
  } catch {
    // Empty Explain stream
  }
}

export async function askInExplain(question) {
  if (!currentSessionId || !question || !sidetalkInputEl) return;
  sidetalkInputEl.value = question;
  await handleSidetalkSubmit();
}

async function handleSidetalkSubmit() {
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  if (!sessionId || !sidetalkInputEl) return;
  const question = sidetalkInputEl.value.trim();
  if (!question) return;
  sidetalkInputEl.value = '';
  await sidetalkHistory;
  if (revision !== sessionRevision) return;
  appendSidetalkPair(question, 'Thinking...');
  const pendingNode = sidetalkStreamEl ? sidetalkStreamEl.lastElementChild : null;

  try {
    const resp = await api.postSidetalk(sessionId, { question, ...sideModelPayload() });
    if (revision !== sessionRevision) return;
    const answer = (resp && (resp.answer || resp.text || resp.a)) || '';
    if (pendingNode) pendingNode.remove();
    appendSidetalkPair(question, answer);
    lastSidetalkCount += 1;
    onCountsChange({ sources: lastSourcesCount, sidetalk: lastSidetalkCount });
  } catch (err) {
    if (revision !== sessionRevision) return;
    if (pendingNode) pendingNode.remove();
    appendSidetalkPair(question, `Error: ${err.message}`);
  }
}

function appendSidetalkPair(question, answer) {
  if (!sidetalkStreamEl) return;
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  if (sidetalkStreamEl.children.length === 1 && !sidetalkStreamEl.firstElementChild.classList.contains('source-card')) {
    sidetalkStreamEl.innerHTML = '';
  }
  const card = document.createElement('div');
  card.className = 'source-card';
  card.innerHTML = `
    <div class="sidetalk-q rich-prose" style="font-weight:600;margin-bottom:6px;"></div>
    <div class="sidetalk-ans rich-prose" style="font-size:13px;"></div>
    <div class="sidetalk-refs" style="display:flex;gap:6px;flex-wrap:wrap;margin-top:6px;"></div>
  `;
  renderRichInto(card.querySelector('.sidetalk-q'), question);
  renderRichInto(card.querySelector('.sidetalk-ans'), answer);
  const refsBox = card.querySelector('.sidetalk-refs');
  const stepMatches = Array.from(new Set((String(answer).match(/\be-\d+\b/g) || [])));
  for (const stepId of stepMatches) {
    const badge = document.createElement('button');
    badge.type = 'button';
    badge.className = 'pill-btn sm';
    badge.textContent = `Inspect ${stepId}`;
    badge.addEventListener('click', () => {
      if (revision === sessionRevision) onSelectStepRef(stepId);
    });
    refsBox.appendChild(badge);
  }
  const ansText = String(answer ?? '').trim();
  if (ansText && ansText !== 'Thinking...' && !ansText.startsWith('Error:')) {
    const pinBtn = document.createElement('button');
    pinBtn.type = 'button';
    pinBtn.className = 'pill-btn sm';
    pinBtn.textContent = 'Pin to Conventions';
    pinBtn.addEventListener('click', async () => {
      if (revision !== sessionRevision || !sessionId) return;
      pinBtn.disabled = true;
      try {
        const clean = ansText.length > 1500 ? `${ansText.slice(0, 1500).trimEnd()}…` : ansText;
        const resp = await api.promoteExplain(sessionId, { target: 'conventions', text: clean });
        if (revision !== sessionRevision) return;
        if (typeof callbacks.onConventionsPromoted === 'function') callbacks.onConventionsPromoted(resp);
        notifyFn(ansText.length > 1500 ? 'Pinned trimmed Explain excerpt to session conventions.' : 'Pinned Explain note to conventions.');
      } catch (err) {
        if (revision !== sessionRevision) return;
        if (err && err.status === 400 && /2000|exceeds/i.test(err.message || '')) {
          notifyFn('Conventions limit (2000 chars) reached — trim existing conventions in the Conventions popover.');
          const pop = document.getElementById('conventions-popover');
          if (pop && pop.hidden) document.getElementById('conventions-btn')?.click();
        } else {
          notifyFn(`Could not pin to conventions: ${err.message}`);
        }
      } finally {
        pinBtn.disabled = false;
      }
    });
    refsBox.appendChild(pinBtn);

    const sendBtn = document.createElement('button');
    sendBtn.type = 'button';
    sendBtn.className = 'pill-btn sm';
    sendBtn.textContent = 'Send to Orchestrator';
    sendBtn.addEventListener('click', async () => {
      if (revision !== sessionRevision || !sessionId) return;
      sendBtn.disabled = true;
      try {
        const sendText = ansText.length > 3900 ? `${ansText.slice(0, 3900).trimEnd()}…` : ansText;
        const resp = await api.promoteExplain(sessionId, { target: 'steer', text: sendText });
        if (revision !== sessionRevision) return;
        if (resp && resp.mode === 'steer') {
          notifyFn('Sent to active Orchestrator as live steer.');
        } else {
          if (typeof callbacks.onPromoteToComposer === 'function') {
            callbacks.onPromoteToComposer((resp && resp.text) || answer);
          }
          notifyFn('Loaded Explain note into composer.');
        }
      } catch (err) {
        if (revision === sessionRevision) notifyFn(`Could not send to Orchestrator: ${err.message}`);
      } finally {
        sendBtn.disabled = false;
      }
    });
    refsBox.appendChild(sendBtn);
  }
  sidetalkStreamEl.appendChild(card);
  sidetalkStreamEl.scrollTop = sidetalkStreamEl.scrollHeight;
}

let readerState = { sessionId: null };
let readerOpenRequest = 0;
let readerRequest = 0;

function wireSourceReaderModal() {
  const dialog = document.getElementById('source-reader-dialog');
  const closeBtn = document.getElementById('source-reader-close');
  const fileSel = document.getElementById('source-reader-file');
  closeBtn?.addEventListener('click', () => dialog?.close());
  dialog?.addEventListener('click', (event) => { if (event.target === dialog) dialog.close(); });
  dialog?.addEventListener('close', () => {
    if (dialog.open) return;
    const sameSession = readerState.sessionId === currentSessionId;
    const opener = readerState.opener;
    readerState = { sessionId: null };
    readerOpenRequest += 1;
    readerRequest += 1;
    if (!sameSession) return;
    if (opener?.isConnected) opener.focus({ preventScroll: true });
    else document.querySelector('.canvas-tab[aria-selected="true"]')?.focus({ preventScroll: true });
  });
  fileSel?.addEventListener('change', () => loadSourceReaderFile(fileSel.value));
  for (const mode of ['rendered', 'raw']) {
    document.getElementById('source-reader-mode-' + mode)?.addEventListener('click', () => {
      readerState.mode = mode;
      applyReaderMode();
      renderReaderContent();
    });
  }
}

function applyReaderMode() {
  for (const mode of ['rendered', 'raw']) {
    const button = document.getElementById('source-reader-mode-' + mode);
    const selected = readerState.mode === mode;
    button?.classList.toggle('is-active', selected);
    button?.setAttribute('aria-pressed', String(selected));
  }
}

async function openSourceReader(sessionId, source, files, links, anchor, opener = document.activeElement) {
  const dialog = document.getElementById('source-reader-dialog');
  if (!dialog || !files.length || sessionId !== currentSessionId) return false;
  const path = files[0];
  readerState = {
    sessionId,
    anchor: anchor ? { ...anchor, path } : null,
    mode: anchor?.start_line ? 'raw' : 'rendered',
    opener,
  };
  const title = document.getElementById('source-reader-title');
  const fileSelect = document.getElementById('source-reader-file');
  const linkContainer = document.getElementById('source-reader-ext-links');
  renderMathTextInto(title, resolveSourceTitle(source));
  if (linkContainer) linkContainer.innerHTML = renderSourceLinkChips(links);
  if (fileSelect) {
    fileSelect.replaceChildren(...files.map((file) => new Option(file, file)));
    fileSelect.value = path;
  }
  applyReaderMode();
  if (!dialog.open) dialog.showModal();
  document.getElementById('source-reader-close')?.focus({ preventScroll: true });
  return await loadSourceReaderFile(path);
}

async function loadSourceReaderFile(path) {
  const body = document.getElementById('source-reader-body');
  const status = document.getElementById('source-reader-status');
  const sessionId = readerState.sessionId;
  if (!path || !sessionId || !body || !status) return false;
  const revision = sessionRevision;
  const request = ++readerRequest;
  const anchor = readerState.anchor?.path === path ? readerState.anchor : null;
  readerState.currentPath = path;
  readerState.fullText = null;
  readerState.citedRange = null;
  body.replaceChildren();
  body.scrollTop = 0;
  status.textContent = 'Loading source…';
  try {
    const data = await api.readSourceFile(sessionId, path);
    if (revision !== sessionRevision || request !== readerRequest) return false;
    if (anchor?.file_sha256 && data?.file_sha256 !== anchor.file_sha256) {
      status.textContent = 'This file does not match the SHA-256 in the evidence record.';
      return false;
    }
    const text = data?.content ?? data?.text ?? '';
    const range = anchor && citedLineRange(text, anchor.start_line, anchor.end_line, data?.truncated);
    readerState.fullText = text;
    readerState.citedRange = range;
    if (anchor?.start_line && !range) {
      status.textContent = 'The cited lines are outside this preview. The recorded excerpt is in the evidence.';
    } else {
      const line = range && (anchor.start_line === anchor.end_line ? `Line ${anchor.start_line}`
        : `Lines ${anchor.start_line}–${anchor.end_line}`);
      status.textContent = [line, data?.truncated && 'Partial file preview'].filter(Boolean).join(' · ');
    }
    renderReaderContent();
    return true;
  } catch (error) {
    if (revision === sessionRevision && request === readerRequest) {
      status.textContent = `Could not read file. ${error.message}`;
    }
    return false;
  }
}

function citedLineRange(text, first, last, truncated) {
  if (!Number.isSafeInteger(first) || !Number.isSafeInteger(last) || first < 1 || last < first) return null;
  let line = 1;
  let start = 0;
  while (line < first) {
    const newline = text.indexOf('\n', start);
    if (newline < 0) return null;
    start = newline + 1;
    line += 1;
  }
  let end = start;
  while (line <= last) {
    const newline = text.indexOf('\n', end);
    if (newline < 0) return line === last && end < text.length && !truncated ? [start, text.length] : null;
    end = newline + 1;
    line += 1;
  }
  return [start, end];
}

function renderReaderContent() {
  const body = document.getElementById('source-reader-body');
  const text = readerState.fullText;
  if (!body || typeof text !== 'string') return;
  body.replaceChildren();
  body.scrollTop = 0;
  if (readerState.mode === 'raw' || /\.(bib|csv|tsv|json|py|log|yml|yaml|sty|cls)$/i.test(readerState.currentPath)) {
    const pre = document.createElement('pre');
    pre.className = 'source-raw-pre';
    body.append(pre);
    const range = readerState.citedRange;
    if (range) {
      const [start, end] = range;
      const mark = document.createElement('mark');
      mark.textContent = text.slice(start, end);
      pre.append(text.slice(0, start), mark, text.slice(end));
      body.scrollTop = mark.getBoundingClientRect().top - body.getBoundingClientRect().top - body.clientHeight / 4;
    } else {
      pre.textContent = text;
    }
    return;
  }
  const target = document.createElement('div');
  target.className = 'rich-prose';
  body.append(target);
  if (/\.(tex|ltx)$/i.test(readerState.currentPath)) renderLatexDocumentPreview(target, text);
  else renderRichInto(target, text);
}
