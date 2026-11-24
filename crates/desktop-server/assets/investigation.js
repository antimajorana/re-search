import * as api from '/assets/api.js';
import { arrayBufferToBase64 } from '/assets/composer.js';
import { renderMathTextInto } from '/assets/math.js';
import { createPaperMap, DEFAULT_PAPER_REQUEST } from '/assets/investigation_map.js';
import { openOriginalPdf } from '/assets/pdf_preview.js';
import { createConventionCatalog } from '/assets/investigation_conventions.js';
import { citedPaperLinks } from '/assets/source_links.js';

let callbacks, paperMap, conventionCatalog, sessionId, session, view, readRevision = 0, submitting = false;
let summaryTimer = null;
const byId = id => document.getElementById(id);
const setupDrafts = new Map();

export function initInvestigation(options) {
  callbacks = options;
  paperMap = createPaperMap(byId('investigation-map'), {
    citedPaperLinks,
    openSource: anchor => callbacks.openSource(sessionId, anchor),
    openPdf: (anchor, probe) => {
      const source = session?.sources?.find(s => s.source_id === anchor.source_id && s.format === 'pdf');
      if (probe) return Boolean(source);
      if (source) openOriginalPdf({ sessionId, sourceId: source.source_id, path: anchor.path, startLine: anchor.start_line,
        label: anchor.label || source.title || source.origin });
    },
    act: handleClaimAction,
  });
  conventionCatalog = createConventionCatalog(byId('investigation-conventions'), paperMap.renderAnchors, async mapSeq => {
    const id = sessionId;
    if (!id || session?.investigation?.map_seq !== mapSeq) throw new Error('The active paper changed. Open its conventions again.');
    await api.findInvestigationConventions(id, { ...callbacks.requestOptions(), map_seq: mapSeq });
    if (id === sessionId) await refreshInvestigation(id);
  });
  byId('new-investigation-btn').addEventListener('click', async event => {
    const button = event.currentTarget;
    if (button.disabled) return;
    button.disabled = true;
    try { await callbacks.create(); } finally { button.disabled = false; }
  });
  byId('show-paper-map').addEventListener('click', () => selectView('map'));
  byId('show-investigation-chat').addEventListener('click', () => selectView('chat'));
  byId('investigation-file').addEventListener('change', () => {
    if (byId('investigation-file').files.length) byId('investigation-existing').value = '';
    syncPdfButton();
  });
  byId('investigation-existing').addEventListener('change', () => {
    if (byId('investigation-existing').value) byId('investigation-file').value = '';
    syncPdfButton();
  });
  byId('investigation-original').addEventListener('click', () => {
    const source = primaryPdf();
    if (source) openOriginalPdf({ sessionId, sourceId: source.source_id, label: source.title || source.origin });
  });
  byId('investigation-setup').addEventListener('submit', buildMap);
}

function primaryPdf() {
  const id = session?.investigation?.source_id || byId('investigation-existing').value;
  return session?.sources?.find(s => s.source_id === id && s.format === 'pdf');
}

function syncPdfButton() {
  const pdf = primaryPdf();
  const file = byId('investigation-file').files[0];
  byId('investigation-original').hidden = !pdf || Boolean(file);
  const pages = byId('investigation-pages');
  pages.hidden = !(file ? /\.pdf$/i.test(file.name) : pdf);
  pages.disabled = pages.hidden;
}

function selectView(next) {
  view = next;
  const investigation = session?.mode === 'investigation';
  const map = investigation && view === 'map';
  byId('app-shell').setAttribute('data-investigation-view', map ? 'map' : 'chat');
  byId('investigation-panel').hidden = !map;
  byId('investigation-nav').hidden = !investigation;
  for (const [name, active] of [['show-paper-map', map], ['show-investigation-chat', !map]]) {
    const button = byId(name);
    button.setAttribute('aria-pressed', String(active));
    button.classList.toggle('is-active', active);
  }
}

function showStatus(text) {
  const el = byId('investigation-status');
  if (el.textContent !== (text || '')) el.textContent = text || '';
  el.hidden = !text;
}

function renderOrigin(origin) {
  const el = byId('investigation-origin');
  const signature = origin?.parent_session
    ? JSON.stringify([sessionId, origin.parent_session, origin.map_seq, origin.node_id, origin.evidence_through_seq])
    : '';
  if (el.dataset.parent === signature) return;
  el.dataset.parent = signature;
  el.replaceChildren();
  el.hidden = !origin?.parent_session;
  if (el.hidden) return;
  const openPassage = anchor => async () => {
    await callbacks.open(origin.parent_session);
    if (sessionId === origin.parent_session) await callbacks.openSource(sessionId, anchor);
  };
  const details = document.createElement('details');
  const title = document.createElement('summary');
  title.textContent = 'From a paper investigation';
  details.append(title);
  const claim = document.createElement('p');
  renderMathTextInto(claim, origin.claim || '');
  details.append(claim);
  if (origin.question) {
    const question = document.createElement('p');
    renderMathTextInto(question, origin.question);
    const use = document.createElement('button');
    use.type = 'button'; use.className = 'pill-btn sm'; use.textContent = 'Use research question';
    use.addEventListener('click', () => callbacks.useQuestion(origin.question));
    details.append(question, use);
  }
  if (origin.assumptions?.length) {
    const heading = document.createElement('strong');
    heading.textContent = 'Carried assumptions';
    const list = document.createElement('ul');
    for (const assumption of origin.assumptions) {
      const li = document.createElement('li');
      renderMathTextInto(li, assumption);
      list.append(li);
    }
    details.append(heading, list);
  }
  if (origin.paper_conventions?.length) {
    const limit = 8;
    const conventions = document.createElement('details');
    const heading = document.createElement('summary');
    heading.textContent = 'Paper conventions (' + origin.paper_conventions.length + ')';
    conventions.append(heading);
    const list = document.createElement('ul');
    for (const item of origin.paper_conventions.slice(0, limit)) {
      const li = document.createElement('li');
      const term = String(item?.label || '');
      const bareTex = item?.category === 'notation' && /^\\[A-Za-z]+/.test(term) && !term.includes('$') &&
        !term.startsWith('\\(') && !term.startsWith('\\[');
      const label = document.createElement('strong');
      renderMathTextInto(label, bareTex ? '$' + term + '$' : term);
      const meaning = document.createElement('span');
      renderMathTextInto(meaning, item?.meaning || '');
      li.append(label, document.createTextNode(' — '), meaning);
      if (item?.scope) {
        const scope = document.createElement('small');
        renderMathTextInto(scope, 'Scope · ' + item.scope);
        li.append(document.createTextNode(' '), scope);
      }
      for (const anchor of item?.anchors || []) {
        if (!anchor?.source_id || !(anchor.label || anchor.path)) continue;
        const location = anchor.start_line == null ? ''
          : anchor.end_line == null || anchor.start_line === anchor.end_line
            ? ' · line ' + anchor.start_line
            : ' · lines ' + anchor.start_line + '–' + anchor.end_line;
        const link = document.createElement('button');
        link.type = 'button'; link.className = 'pill-btn sm';
        link.textContent = (anchor.label || anchor.path) + location;
        link.addEventListener('click', openPassage(anchor));
        li.append(document.createTextNode(' '), link);
      }
      list.append(li);
    }
    conventions.append(list);
    if (origin.paper_conventions.length > limit) {
      const more = document.createElement('p');
      more.className = 'investigation-meta';
      more.textContent = 'Showing ' + limit + ' of ' + origin.paper_conventions.length + ' conventions.';
      const full = document.createElement('button');
      full.type = 'button'; full.className = 'pill-btn sm';
      full.textContent = 'Open full catalog in Investigation';
      full.addEventListener('click', async () => {
        await callbacks.open(origin.parent_session);
        if (sessionId !== origin.parent_session) return;
        selectView('map');
        const catalog = byId('investigation-conventions');
        if (catalog.hidden) {
          showStatus('The current paper reading has no conventions catalog.');
          return;
        }
        catalog.open = true;
        catalog.querySelector('summary')?.focus();
      });
      conventions.append(more, full);
    }
    details.append(conventions);
  }
  if (origin.sources?.length) {
    const evidence = document.createElement('details');
    const heading = document.createElement('summary');
    heading.textContent = 'Source passages (' + origin.sources.length + ')';
    evidence.append(heading);
    for (const anchor of origin.sources) {
      const passage = document.createElement('blockquote');
      renderMathTextInto(passage, anchor.excerpt || '');
      const link = document.createElement('button');
      link.type = 'button'; link.className = 'pill-btn sm';
      link.textContent = (anchor.label || anchor.path) + ' · lines ' + anchor.start_line + '–' + anchor.end_line;
      link.addEventListener('click', openPassage(anchor));
      evidence.append(passage, link);
    }
    details.append(evidence);
  }
  const open = document.createElement('button');
  open.type = 'button';
  open.className = 'pill-btn sm';
  open.textContent = origin.node_id ? 'Open original claim' : 'Open original investigation';
  open.addEventListener('click', async () => {
    await callbacks.open(origin.parent_session);
    if (sessionId !== origin.parent_session || !origin.node_id) return;
    selectView('map');
    if (!paperMap.openNode(origin.map_seq, origin.node_id)) {
      showStatus('The linked claim belongs to an earlier map revision. The original claim and sources are preserved in the linked Research session.');
    }
  });
  details.append(open);
  el.append(details);
}

export function showInvestigationSession(id, data) {
  if (!paperMap) return;
  clearTimeout(summaryTimer);
  summaryTimer = null;
  readRevision++;
  if (sessionId !== id) {
    if (sessionId && session?.mode === 'investigation') {
      setupDrafts.set(sessionId, { question: byId('investigation-question').value, source: byId('investigation-existing').value,
        start: byId('investigation-page-start').value, end: byId('investigation-page-end').value });
    }
    sessionId = id;
    view = 'map';
    submitting = false;
    paperMap.reset();
    conventionCatalog.reset();
    byId('investigation-setup').reset();
    byId('investigation-existing').replaceChildren();
    byId('investigation-question').value = setupDrafts.get(id)?.question || '';
    byId('investigation-page-start').value = setupDrafts.get(id)?.start || '';
    byId('investigation-page-end').value = setupDrafts.get(id)?.end || '';
    byId('investigation-origin').dataset.parent = '';
    byId('investigation-origin').hidden = true;
    showStatus('');
  }
  session = data;
  selectView(view);
  renderOrigin(data?.investigation?.origin);
  if (data?.mode !== 'investigation') return;
  const inv = data.investigation || {};
  paperMap.updateSummary(inv.summary, Boolean(data.running), Boolean(inv.source_id || inv.map), inv.summary_is_current);
  conventionCatalog.update(inv, Boolean(data.running));
  scheduleSummaryRefresh();
  const select = byId('investigation-existing');
  const previous = select.value || setupDrafts.get(id)?.source || inv.source_id;
  select.replaceChildren(new Option('Select a paper…', ''));
  for (const source of data.sources || []) {
    if (source.source_id) select.add(new Option(source.title || source.origin || source.source_id, source.source_id));
  }
  if (previous && [...select.options].some(o => o.value === previous)) select.value = previous;
  byId('investigation-existing-wrap').hidden = select.options.length < 2;
  syncPdfButton();
  if (inv.source_id) {
    byId('investigation-page-start').value = inv.page_start || '';
    byId('investigation-page-end').value = inv.page_end || '';
  }
  const pinned = Boolean(inv.source_id) || submitting;
  for (const id of ['investigation-page-start','investigation-page-end','investigation-existing','investigation-file']) byId(id).disabled = pinned;
  byId('investigation-question').disabled = submitting;
  const setup = byId('investigation-setup');
  setup.hidden = Boolean(inv.map) || Boolean(data.running);
  setup.querySelector('button').disabled = submitting;
  if (data.running) showStatus(inv.summary && !inv.map ? 'The paper overview is ready. The model is identifying the main results.' :
    'The selected model is working. Open Conversation to follow its progress or approve a tool.');
  else if (!submitting) {
    if (inv.mapping_error) showStatus((inv.map ? 'Showing the saved main results. The latest reading failed. ' : '') + inv.mapping_error);
    else showStatus(!inv.map && inv.source_id ? 'The main results are unavailable. Try again or read the conversation.' : '');
  }
  if (!byId('investigation-question').value && inv.question) byId('investigation-question').value = inv.question;
  paperMap.update(inv, Boolean(data.running), data.events || []);
}

function scheduleSummaryRefresh() {
  if (summaryTimer || session?.mode !== 'investigation' || !session.running) return;
  const inv = session.investigation || {};
  const summaryPending = !inv.summary || inv.summary_is_current === false;
  const conventionsPending = inv.conventions_requested && (!inv.conventions?.done || inv.conventions_is_current === false);
  if (!summaryPending && !conventionsPending) return;
  const id = sessionId;
  summaryTimer = setTimeout(async () => {
    summaryTimer = null;
    await refreshInvestigation(id);
    if (sessionId === id) scheduleSummaryRefresh();
  }, 4000);
}

export async function refreshInvestigation(id) {
  if (sessionId !== id || session?.mode !== 'investigation') return;
  const revision = ++readRevision;
  try {
    const data = await api.getSession(id);
    if (revision === readRevision && id === sessionId) showInvestigationSession(id, data);
  } catch (error) {
    if (revision === readRevision && id === sessionId) showStatus(error.message);
  }
}

async function buildMap(event) {
  event.preventDefault();
  if (submitting || session?.running) return;
  const id = sessionId;
  const file = byId('investigation-file').files[0];
  let sourceId = byId('investigation-existing').value || session?.investigation?.source_id;
  if (!file && !sourceId) { showStatus('Choose a primary paper.'); return; }
  if (file && file.size > 10 * 1024 * 1024) { showStatus('This paper exceeds 10 MiB. Choose a smaller file.'); return; }
  const pageRange = {};
  if (!byId('investigation-pages').hidden) {
    const start = byId('investigation-page-start').valueAsNumber, end = byId('investigation-page-end').valueAsNumber;
    if (!Number.isNaN(start) || !Number.isNaN(end)) {
      if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 1 || end < start) {
        showStatus('Enter both page numbers, with the last page at or after the first.'); return;
      }
      pageRange.investigation_page_start = start;
      pageRange.investigation_page_end = end;
    }
  }
  const readingQuestion = byId('investigation-question').value.trim();
  const button = event.currentTarget.querySelector('button');
  const controls = [...event.currentTarget.querySelectorAll('input, textarea, select')];
  submitting = true;
  button.disabled = true;
  for (const control of controls) control.disabled = true;
  try {
    if (file) {
      showStatus('Adding paper to this session…');
      const content = arrayBufferToBase64(await file.arrayBuffer());
      if (sessionId !== id) return;
      const upload = await api.uploadFilesBase64(id, [{ name: file.name, content_base64: content }]);
      if (sessionId !== id) return;
      const source = upload.ingested?.[0];
      if (!source?.source_id) throw new Error('Could not confirm ingestion. Check Sources before uploading again.');
      sourceId = source.source_id;
      if (!session.sources?.some(item => item.source_id === sourceId)) session.sources = [...(session.sources || []), source];
      const select = byId('investigation-existing');
      if (![...select.options].some(o => o.value === sourceId)) select.add(new Option(file.name, sourceId));
      select.value = sourceId;
      byId('investigation-existing-wrap').hidden = false;
      byId('investigation-file').value = '';
      syncPdfButton();
    }
    if (session.sources?.some(item => item.source_id === sourceId && item.status === 'empty_text_layer')) {
      throw new Error('This PDF has no readable text layer. Open the original PDF, or choose a searchable PDF or TeX source to build a map.');
    }
    const options = callbacks.requestOptions();
    await callbacks.send(id, readingQuestion || DEFAULT_PAPER_REQUEST, {
      ...options, ...pageRange, investigation_source_id: sourceId,
    });
    if (sessionId === id) await refreshInvestigation(id);
  } catch (error) {
    if (sessionId === id) showStatus(error.message);
  } finally {
    if (sessionId === id) {
      submitting = false;
      button.disabled = false;
      for (const control of controls) control.disabled = control.id !== 'investigation-question' && Boolean(session?.investigation?.source_id);
    }
  }
}

async function handleClaimAction(action, target, text, edgeFrom) {
  const id = sessionId;
  if (!id || session?.investigation?.map_seq !== target.map_seq) throw new Error('The paper map changed. Select the claim again.');
  if (action === 'research') {
    const result = await api.openInvestigationResearch(id, { ...target, question: text });
    if (id === sessionId) await callbacks.open(result.session_id);
    return;
  }
  if (action === 'correction') {
    await api.correctInvestigation(id, { ...target, text, ...(edgeFrom ? { edge_from: edgeFrom } : {}) });
  } else if (action === 'branch') {
    await api.developInvestigation(id, { ...callbacks.requestOptions(), ...target, question: text });
  } else {
    await api.checkInvestigation(id, { ...callbacks.requestOptions(), ...target, action,
      ...(edgeFrom ? { edge_from: edgeFrom } : {}), ...(text ? { prompt: text } : {}) });
  }
  if (id === sessionId) await refreshInvestigation(id);
}
