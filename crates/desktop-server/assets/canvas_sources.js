// Tabs 3 & 4: Literature & Sources Canvas (arXiv/INSPIRE/Crossref/S2) and isolated Side-Talk Canvas.
import * as api from '/assets/api.js';
import { escapeHtml, renderRichInto } from '/assets/math.js';

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

export function initSourcesAndSidetalk(opts = {}) {
  notifyFn = opts.onNotify || (() => {});
  onSelectStepRef = opts.onSelectStepRef || (() => {});
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

    card.innerHTML = `
      ${withdrawn ? '<div class="retraction-banner">Warning: Withdrawn or Retracted Publication — blocked from unflagged citation</div>' : ''}
      <div style="font-weight:600;margin-bottom:3px;">${escapeHtml(item.title || 'Untitled')}</div>
      <div style="font-size:12px;color:var(--ink-secondary);margin-bottom:4px;">${escapeHtml(authors)}</div>
      <div style="font-size:11.5px;color:var(--ink-muted);margin-bottom:8px;">${escapeHtml(meta)}</div>
      ${item.abstract_text ? `<div style="font-size:12px;color:var(--ink-secondary);margin-bottom:8px;">${escapeHtml(item.abstract_text.slice(0, 320))}${item.abstract_text.length > 320 ? '…' : ''}</div>` : ''}
      ${sections.length ? `<div style="font-size:11.5px;margin-bottom:8px;"><strong>TeX sections:</strong> ${sections.map((s) => `<code>${escapeHtml(s)}</code>`).join(' ')}</div>` : ''}
      <div style="display:flex;gap:6px;flex-wrap:wrap;">
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
    sourcesListEl.innerHTML = '';
    if (sources.length === 0) {
      sourcesListEl.innerHTML = '<div class="source-card" style="color:var(--ink-muted)">Upload .tex/.bib/.pdf/.tar.gz files via the composer or ingest arXiv e-prints above.</div>';
      return;
    }
    for (const src of sources) {
      const card = document.createElement('div');
      card.className = 'source-card';
      const files = Array.isArray(src.files) ? src.files.join(', ') : '';
      card.innerHTML = `
        <div style="display:flex;justify-content:space-between;gap:8px;align-items:center;">
          <strong>${escapeHtml(src.primary_path || src.origin || src.source_id || 'source')}</strong>
          <span class="badge">${escapeHtml(src.format || src.status || 'ingested')}</span>
        </div>
        ${files ? `<div style="font-size:11.5px;color:var(--ink-muted);margin-top:4px;">Files: ${escapeHtml(files)}</div>` : ''}
        ${src.raw_blob ? `<div style="font-family:var(--font-mono);font-size:11px;color:var(--ink-muted);margin-top:3px;">blob ${escapeHtml(src.raw_blob.slice(0, 14))}…</div>` : ''}
      `;
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
    sidetalkStreamEl.innerHTML = '';
    if (turns.length === 0) {
      sidetalkStreamEl.innerHTML = '<div style="color:var(--ink-muted);font-size:13px;">Ask clarifying questions here without altering the verified proof chain.</div>';
      return;
    }
    for (const t of turns) {
      appendSidetalkPair(t.question || t.q || '', t.answer || t.a || '');
    }
  } catch {
    // Empty sidetalk stream
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
    const resp = await api.postSidetalk(currentSessionId, { question });
    const answer = (resp && (resp.answer || resp.text || resp.a)) || '';
    if (pendingNode) pendingNode.remove();
    appendSidetalkPair(question, answer);
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
    <div style="font-weight:600;margin-bottom:6px;">${escapeHtml(question)}</div>
    <div class="sidetalk-ans rich-prose" style="font-size:13px;"></div>
    <div class="sidetalk-refs" style="display:flex;gap:6px;flex-wrap:wrap;margin-top:6px;"></div>
  `;
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
  sidetalkStreamEl.appendChild(card);
  sidetalkStreamEl.scrollTop = sidetalkStreamEl.scrollHeight;
}
