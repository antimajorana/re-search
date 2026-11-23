// Tabs 3 & 4: Literature & Sources Canvas (arXiv/INSPIRE/Crossref/S2) and isolated Explain Canvas.
import * as api from '/assets/api.js';
import { escapeHtml, renderLatexDocumentPreview, renderRichInto } from '/assets/math.js';
import { getModelChoice } from '/assets/model_picker.js';
import { linksForIngestedSource, linksForPaper, renderSourceLinkChips } from '/assets/source_links.js';

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
let sourcesListEl = null;
let sidetalkStreamEl = null;
let sidetalkFormEl = null;
let sidetalkInputEl = null;
let currentSessionId = null;
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
  sourcesListEl = document.getElementById('sources-list');
  sidetalkStreamEl = document.getElementById('sidetalk-stream');
  sidetalkFormEl = document.getElementById('sidetalk-form');
  sidetalkInputEl = document.getElementById('sidetalk-input');

  if (litFormEl) {
    litFormEl.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      await handleLiteratureSearch();
    });
  }
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
  currentSessionId = sessionId;
  if (!sessionId) return;
  await Promise.all([loadSessionSources(sessionId), loadSidetalk(sessionId)]);
}

async function handleLiteratureSearch() {
  if (!currentSessionId) {
    notifyFn('Select or create a session first.');
    return;
  }
  const query = litQueryEl ? litQueryEl.value.trim() : '';
  if (!query) return;

  const payload = {
    source: litSourceEl ? litSourceEl.value : 'arxiv',
    query,
    field: litFieldEl ? litFieldEl.value : 'all',
    include_source: litIncludeEl ? Boolean(litIncludeEl.checked) : false,
    section_filter: litFilterEl && litFilterEl.value.trim() ? litFilterEl.value.trim() : undefined,
  };

  try {
    const resp = await api.searchLiterature(currentSessionId, payload);
    const items = Array.isArray(resp) ? resp : resp.results || resp.papers || resp.hits || [];
    renderLiteratureHits(items);
    if (payload.include_source) {
      await loadSessionSources(currentSessionId);
    }
  } catch (err) {
    notifyFn(`Literature search failed: ${err.message}`);
  }
}

function renderLiteratureHits(items) {
  if (!litResultsEl) return;
  litResultsEl.innerHTML = '';
  if (!items || items.length === 0) {
    litResultsEl.innerHTML = '<div class="paper-card" style="color:var(--ink-muted)">No matching papers returned.</div>';
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
    const activeSrc = item.source && typeof item.source === 'string' ? item.source : (litSourceEl ? litSourceEl.value : '');
    const links = linksForPaper({ ...item, source: activeSrc });
    const titleHtml = links[0]
      ? `<a class="paper-title-link" href="${escapeHtml(links[0].url)}" target="_blank" rel="noopener noreferrer">${escapeHtml(item.title || 'Untitled')} ↗</a>`
      : escapeHtml(item.title || 'Untitled');

    card.innerHTML = `
      ${withdrawn ? '<div class="retraction-banner">Warning: Withdrawn or Retracted Publication — blocked from unflagged citation</div>' : ''}
      <div style="font-weight:600;margin-bottom:3px;">${titleHtml}</div>
      <div style="font-size:12px;color:var(--ink-secondary);margin-bottom:4px;">${escapeHtml(authors)}</div>
      <div style="font-size:11.5px;color:var(--ink-muted);margin-bottom:8px;">${escapeHtml(meta)}</div>
      ${item.abstract_text ? `<div style="font-size:12px;color:var(--ink-secondary);margin-bottom:8px;">${escapeHtml(item.abstract_text.slice(0, 320))}${item.abstract_text.length > 320 ? '…' : ''}</div>` : ''}
      ${sections.length ? `<div style="font-size:11.5px;margin-bottom:8px;"><strong>TeX sections:</strong> ${sections.map((s) => `<code>${escapeHtml(s)}</code>`).join(' ')}</div>` : ''}
      <div style="display:flex;gap:6px;flex-wrap:wrap;align-items:center;">
        ${renderSourceLinkChips(links)}
        ${bibtex ? '<button type="button" class="pill-btn sm copy-bib-btn">Copy BibTeX</button>' : ''}
      </div>
    `;
    const copyBtn = card.querySelector('.copy-bib-btn');
    if (copyBtn) {
      copyBtn.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(bibtex);
          notifyFn('BibTeX copied to clipboard.');
        } catch {
          notifyFn('Clipboard unavailable; BibTeX displayed in console.');
        }
      });
    }
    litResultsEl.appendChild(card);
  }
}

export async function loadSessionSources(sessionId) {
  if (!sourcesListEl || !sessionId) return;
  try {
    const sess = await api.getSession(sessionId);
    const sources = Array.isArray(sess && sess.sources) ? sess.sources : [];
    lastSourcesCount = sources.length;
    onCountsChange({ sources: lastSourcesCount, sidetalk: lastSidetalkCount });
    sourcesListEl.innerHTML = '';
    if (sources.length === 0) {
      sourcesListEl.innerHTML = '<div class="source-card" style="color:var(--ink-muted)">Upload .tex/.bib/.pdf/.tar.gz files via the composer or ingest arXiv e-prints above.</div>';
      return;
    }
    for (const src of sources) {
      const card = document.createElement('div');
      card.className = 'source-card';
      const rawFiles = Array.isArray(src.files) ? src.files : [];
      const readableFiles = Array.from(new Set([src.assembled_path, src.primary_path, ...rawFiles].filter(Boolean)));
      const files = rawFiles.join(', ');
      const extLinks = linksForIngestedSource(src);
      card.innerHTML = `
        <div style="display:flex;justify-content:space-between;gap:8px;align-items:center;">
          <strong>${escapeHtml(src.primary_path || src.origin || src.source_id || 'source')}</strong>
          <span class="badge">${escapeHtml(src.format || src.status || 'ingested')}</span>
        </div>
        ${src.origin && src.origin !== src.primary_path ? `<div style="font-size:11.5px;color:var(--ink-secondary);margin-top:3px;">Origin: <code>${escapeHtml(src.origin)}</code></div>` : ''}
        ${files ? `<div style="font-size:11.5px;color:var(--ink-muted);margin-top:4px;">Files: ${escapeHtml(files)}</div>` : ''}
        <div style="display:flex;gap:6px;flex-wrap:wrap;align-items:center;margin-top:8px;">
          ${readableFiles.length ? '<button type="button" class="pill-btn sm primary read-source-btn">Read Source</button>' : ''}
          ${renderSourceLinkChips(extLinks)}
        </div>
      `;
      const readBtn = card.querySelector('.read-source-btn');
      if (readBtn) {
        readBtn.addEventListener('click', () => openSourceReader(sessionId, src, readableFiles, extLinks));
      }
      sourcesListEl.appendChild(card);
    }
  } catch {
    // Session might not have sources list
  }
}

async function loadSidetalk(sessionId) {
  if (!sidetalkStreamEl || !sessionId) return;
  try {
    const data = await api.getSidetalk(sessionId);
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

async function handleSidetalkSubmit() {
  if (!currentSessionId || !sidetalkInputEl) return;
  const question = sidetalkInputEl.value.trim();
  if (!question) return;
  sidetalkInputEl.value = '';
  appendSidetalkPair(question, 'Thinking...');
  const pendingNode = sidetalkStreamEl ? sidetalkStreamEl.lastElementChild : null;

  try {
    const resp = await api.postSidetalk(currentSessionId, { question, ...sideModelPayload() });
    const answer = (resp && (resp.answer || resp.text || resp.a)) || '';
    if (pendingNode) pendingNode.remove();
    appendSidetalkPair(question, answer);
    lastSidetalkCount += 1;
    onCountsChange({ sources: lastSourcesCount, sidetalk: lastSidetalkCount });
  } catch (err) {
    if (pendingNode) pendingNode.remove();
    appendSidetalkPair(question, `Error: ${err.message}`);
  }
}

function appendSidetalkPair(question, answer) {
  if (!sidetalkStreamEl) return;
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
    badge.addEventListener('click', () => onSelectStepRef(stepId));
    refsBox.appendChild(badge);
  }
  const ansText = String(answer ?? '').trim();
  if (ansText && ansText !== 'Thinking...' && !ansText.startsWith('Error:')) {
    const pinBtn = document.createElement('button');
    pinBtn.type = 'button';
    pinBtn.className = 'pill-btn sm';
    pinBtn.textContent = 'Pin to Conventions';
    pinBtn.addEventListener('click', async () => {
      if (!currentSessionId) return;
      pinBtn.disabled = true;
      try {
        const resp = await api.promoteExplain(currentSessionId, { target: 'conventions', text: answer });
        if (typeof callbacks.onConventionsPromoted === 'function') callbacks.onConventionsPromoted(resp);
        notifyFn('Pinned Explain note to conventions.');
      } catch (err) {
        notifyFn(`Could not pin to conventions: ${err.message}`);
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
      if (!currentSessionId) return;
      sendBtn.disabled = true;
      try {
        const resp = await api.promoteExplain(currentSessionId, { target: 'steer', text: answer });
        if (resp && resp.mode === 'steer') {
          notifyFn('Sent to active Orchestrator as live steer.');
        } else {
          if (typeof callbacks.onPromoteToComposer === 'function') {
            callbacks.onPromoteToComposer((resp && resp.text) || answer);
          }
          notifyFn('Loaded Explain note into composer.');
        }
      } catch (err) {
        notifyFn(`Could not send to Orchestrator: ${err.message}`);
      } finally {
        sendBtn.disabled = false;
      }
    });
    refsBox.appendChild(sendBtn);
  }
  sidetalkStreamEl.appendChild(card);
  sidetalkStreamEl.scrollTop = sidetalkStreamEl.scrollHeight;
}

let readerState = { sessionId: null, files: [], currentPath: '', text: '', mode: 'rendered', links: [] };

function wireSourceReaderModal() {
  const dlg = document.getElementById('source-reader-dialog');
  const closeBtn = document.getElementById('source-reader-close');
  const fileSel = document.getElementById('source-reader-file');
  const modeRender = document.getElementById('source-reader-mode-rendered');
  const modeRaw = document.getElementById('source-reader-mode-raw');
  if (closeBtn && dlg) closeBtn.addEventListener('click', () => { dlg.hidden = true; });
  if (dlg) dlg.addEventListener('click', (ev) => { if (ev.target === dlg) dlg.hidden = true; });
  if (fileSel) {
    fileSel.addEventListener('change', () => loadSourceReaderFile(fileSel.value));
  }
  const setMode = (m) => {
    readerState.mode = m;
    if (modeRender) modeRender.classList.toggle('is-active', m === 'rendered');
    if (modeRaw) modeRaw.classList.toggle('is-active', m === 'raw');
    renderReaderContent();
  };
  if (modeRender) modeRender.addEventListener('click', () => setMode('rendered'));
  if (modeRaw) modeRaw.addEventListener('click', () => setMode('raw'));
}

async function openSourceReader(sessionId, src, files, extLinks) {
  const dlg = document.getElementById('source-reader-dialog');
  const titleEl = document.getElementById('source-reader-title');
  const fileSel = document.getElementById('source-reader-file');
  const linksEl = document.getElementById('source-reader-ext-links');
  if (!dlg) return;
  readerState = { sessionId, files, currentPath: files[0] || '', text: '', mode: 'rendered', links: extLinks || [] };
  if (titleEl) titleEl.textContent = src.origin || src.primary_path || src.source_id || 'Source Reader';
  if (linksEl) linksEl.innerHTML = renderSourceLinkChips(extLinks);
  if (fileSel) {
    fileSel.innerHTML = '';
    for (const f of files) fileSel.append(new Option(f, f));
    fileSel.value = readerState.currentPath;
  }
  dlg.hidden = false;
  await loadSourceReaderFile(readerState.currentPath);
}

async function loadSourceReaderFile(relPath) {
  const bodyEl = document.getElementById('source-reader-body');
  if (!relPath || !readerState.sessionId || !bodyEl) return;
  readerState.currentPath = relPath;
  bodyEl.innerHTML = '<div style="color:var(--ink-muted);font-size:13px;">Loading source...</div>';
  try {
    const data = await api.readSourceFile(readerState.sessionId, relPath);
    readerState.text = (data && (data.content || data.text)) || '';
    renderReaderContent();
  } catch (err) {
    bodyEl.innerHTML = `<div class="badge err">Could not read file: ${escapeHtml(err.message)}</div>`;
  }
}

function renderReaderContent() {
  const bodyEl = document.getElementById('source-reader-body');
  if (!bodyEl) return;
  const text = readerState.text || '';
  if (readerState.mode === 'raw' || /\.(bib|csv|tsv|json|py|log|yml|yaml|sty|cls)$/i.test(readerState.currentPath)) {
    bodyEl.innerHTML = `<pre class="source-raw-pre">${escapeHtml(text)}</pre>`;
    return;
  }
  bodyEl.innerHTML = '<div class="rich-prose"></div>';
  const target = bodyEl.firstElementChild;
  if (/\.(tex|ltx)$/i.test(readerState.currentPath)) renderLatexDocumentPreview(target, text);
  else renderRichInto(target, text);
}
