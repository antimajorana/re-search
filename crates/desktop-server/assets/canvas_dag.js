// Tab 2: Proof Graph & Ledger Canvas — zoomable/pannable SVG DAG, SHA-256 chain verify, fork, diff, and narration.
import * as api from '/assets/api.js';
import { escapeHtml, firstMathSentence, renderMathTextInto } from '/assets/math.js';
import { getModelChoice } from '/assets/model_picker.js';
import { renderAssumptionImpact, renderDagNodeDetail } from '/assets/canvas_dag_detail.js';
import { openSessionSource } from '/assets/canvas_sources.js';

function sideModelPayload() {
  const c = getModelChoice();
  return c ? { provider: c.provider, model: c.model } : {};
}

const SVG_NS = 'http://www.w3.org/2000/svg';
const MIN_SCALE = 0.02;
const MAX_SCALE = 3.0;

let svgEl = null;
let viewportEl = null;
let stageEl = null;
let zoomResetBtn = null;
let detailEl = null;
let verifyBadgeEl = null;
let forkBarEl = null;
let forkSeqEl = null;
let forkInputEl = null;
let diffPanelEl = null;

let currentSessionId = null;
let sessionRevision = 0;
let dagRequest = 0;
let currentDag = null;
let selectedEntry = null;
let forkPoint = null;
let nodeCoords = new Map();
let callbacks = {};
let viewState = { scale: 1, tx: 16, ty: 16, contentW: 420, contentH: 240, hasAutoFit: false, userAdjusted: false };
let suppressNextNodeClick = false;
let readingOrder = [];
let chosenView = null;
let lastWidth = 0;
let resizeFrame;

export function initDagCanvas(opts = {}) {
  callbacks = opts;
  svgEl = document.getElementById('proof-dag-svg');
  viewportEl = document.getElementById('dag-viewport');
  zoomResetBtn = document.getElementById('dag-zoom-reset');
  detailEl = document.getElementById('dag-node-detail');
  if (detailEl) {
    detailEl.tabIndex = -1;
    detailEl.setAttribute('role', 'region');
    detailEl.setAttribute('aria-label', 'Selected proof step');
  }
  verifyBadgeEl = document.getElementById('dag-verify-badge');
  forkBarEl = document.getElementById('dag-fork-bar');
  forkSeqEl = document.getElementById('fork-seq-label');
  forkInputEl = document.getElementById('fork-assumption-input');
  diffPanelEl = document.getElementById('dag-diff-panel');

  document.getElementById('dag-verify-btn')?.addEventListener('click', handleVerifyChain);
  document.getElementById('dag-narrate-btn')?.addEventListener('click', handleNarrateProof);
  document.getElementById('dag-fork-btn')?.addEventListener('click', () => openForkBarForSeq());
  document.getElementById('fork-cancel-btn')?.addEventListener('click', closeForkBar);
  document.getElementById('fork-confirm-btn')?.addEventListener('click', handleConfirmFork);

  document.getElementById('dag-zoom-in')?.addEventListener('click', () => zoomAroundCenter(1.25));
  document.getElementById('dag-zoom-out')?.addEventListener('click', () => zoomAroundCenter(0.8));
  document.getElementById('dag-zoom-fit')?.addEventListener('click', () => { viewState.userAdjusted = true; fitDagToView(); });
  zoomResetBtn?.addEventListener('click', resetZoomOneToOne);

  wirePanAndWheel();
  for (const mode of ['graph', 'list']) {
    document.getElementById('dag-view-' + mode)?.addEventListener('click', () => {
      chosenView = mode;
      if (currentDag) renderDagSvg(currentDag);
    });
  }
  const controls = document.getElementById('dag-views');
  if (controls && typeof ResizeObserver !== 'undefined') {
    new ResizeObserver(() => {
      if (!currentDag || !controls.clientWidth || controls.clientWidth === lastWidth) return;
      cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(() => currentDag && renderDagSvg(currentDag));
    }).observe(controls);
  }
}

export function onDagTabShown() {
  if (!viewState.userAdjusted && currentDag && Array.isArray(currentDag.entries) && currentDag.entries.length > 0) {
    renderDagSvg(currentDag);
  }
}

function clampScale(s) {
  return Math.max(MIN_SCALE, Math.min(MAX_SCALE, s));
}

function viewportSize() {
  const w = (viewportEl && viewportEl.clientWidth) || (svgEl && svgEl.clientWidth) || 440;
  const h = (viewportEl && viewportEl.clientHeight) || (svgEl && svgEl.clientHeight) || 320;
  return { vw: Math.max(200, w), vh: Math.max(180, h) };
}

function applyViewTransform() {
  if (stageEl) {
    stageEl.setAttribute('transform', `translate(${viewState.tx.toFixed(1)}, ${viewState.ty.toFixed(1)}) scale(${viewState.scale.toFixed(4)})`);
  }
  if (zoomResetBtn) {
    zoomResetBtn.textContent = `${Math.round(viewState.scale * 100)}%`;
  }
}

export function fitDagToView() {
  const { vw, vh } = viewportSize();
  const cw = Math.max(1, viewState.contentW);
  const ch = Math.max(1, viewState.contentH);
  const pad = 24;
  const scale = clampScale(Math.min((vw - pad * 2) / cw, (vh - pad * 2) / ch, 1.15));
  viewState.scale = scale;
  viewState.tx = (vw - cw * scale) / 2;
  viewState.ty = Math.max(14, (vh - ch * scale) / 2);
  viewState.hasAutoFit = true;
  applyViewTransform();
}

function resetZoomOneToOne() {
  viewState.userAdjusted = true;
  viewState.scale = 1;
  if (readingOrder.length) centerOnNode(selectedEntry?.entry_id || readingOrder[0].entry_id);
  else applyViewTransform();
}

function zoomAtPoint(factor, cx, cy) {
  const oldScale = viewState.scale;
  const nextScale = clampScale(oldScale * factor);
  if (Math.abs(nextScale - oldScale) < 0.001) return;
  const ratio = nextScale / oldScale;
  viewState.tx = cx - (cx - viewState.tx) * ratio;
  viewState.ty = cy - (cy - viewState.ty) * ratio;
  viewState.scale = nextScale;
  viewState.userAdjusted = true;
  applyViewTransform();
}

function zoomAroundCenter(factor) {
  const { vw, vh } = viewportSize();
  zoomAtPoint(factor, vw / 2, vh / 2);
}

function centerOnNode(entryId) {
  const pos = nodeCoords.get(entryId);
  if (!pos) return;
  const { vw, vh } = viewportSize();
  const cx = pos.x + pos.w / 2;
  const cy = pos.y + pos.h / 2;
  viewState.tx = vw / 2 - cx * viewState.scale;
  viewState.ty = vh / 2 - cy * viewState.scale;
  viewState.userAdjusted = true;
  applyViewTransform();
}

function wirePanAndWheel() {
  if (!viewportEl) return;
  viewportEl.addEventListener('wheel', (ev) => {
    if (!stageEl || !currentDag || !(currentDag.entries || []).length) return;
    if (!ev.ctrlKey && !ev.metaKey) return;
    ev.preventDefault();
    const rect = viewportEl.getBoundingClientRect();
    const mx = ev.clientX - rect.left;
    const my = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? 1.14 : 0.88;
    zoomAtPoint(factor, mx, my);
  }, { passive: false });
  viewportEl.addEventListener('keydown', (ev) => {
    if (ev.target !== viewportEl) return;
    const move = { ArrowLeft: [80, 0], ArrowRight: [-80, 0], ArrowUp: [0, 80], ArrowDown: [0, -80] }[ev.key];
    if (!move) return;
    ev.preventDefault();
    viewState.userAdjusted = true;
    viewState.tx += move[0]; viewState.ty += move[1];
    applyViewTransform();
  });

  let drag = null;
  viewportEl.addEventListener('pointerdown', (ev) => {
    if (ev.button !== 0 || (ev.target && ev.target.closest('.dag-zoom-controls'))) return;
    drag = {
      id: ev.pointerId,
      startX: ev.clientX,
      startY: ev.clientY,
      startTx: viewState.tx,
      startTy: viewState.ty,
      moved: false,
    };
  });
  viewportEl.addEventListener('pointermove', (ev) => {
    if (!drag || ev.pointerId !== drag.id) return;
    const dx = ev.clientX - drag.startX;
    const dy = ev.clientY - drag.startY;
    if (!drag.moved && Math.hypot(dx, dy) > 4) {
      drag.moved = true;
      viewportEl.setPointerCapture?.(ev.pointerId);
      viewportEl.classList.add('is-panning');
    }
    if (drag.moved) {
      viewState.tx = drag.startTx + dx;
      viewState.ty = drag.startTy + dy;
      viewState.userAdjusted = true;
      applyViewTransform();
    }
  });
  const endDrag = (ev) => {
    if (!drag || ev.pointerId !== drag.id) return;
    if (drag.moved) {
      suppressNextNodeClick = true;
      if (viewportEl.hasPointerCapture?.(ev.pointerId)) viewportEl.releasePointerCapture(ev.pointerId);
      setTimeout(() => { suppressNextNodeClick = false; }, 0);
    }
    viewportEl.classList.remove('is-panning');
    drag = null;
  };
  viewportEl.addEventListener('pointerup', endDrag);
  viewportEl.addEventListener('pointercancel', endDrag);
}

function openForkBarForSeq(seqOverride) {
  if (!forkBarEl || !currentSessionId || document.getElementById('fork-confirm-btn')?.disabled) return;
  if (seqOverride === undefined && !forkBarEl.hidden) {
    closeForkBar();
    return;
  }
  const atSeq = Number(seqOverride ?? selectedEntry?.seq ?? latestSeq()) || undefined;
  forkPoint = { atSeq };
  const entries = currentDag?.entries || [];
  renderAssumptionImpact(document.getElementById('fork-assumption-impact'), entries,
    entries.find(entry => entry.seq === atSeq), id => selectNodeById(id));
  forkBarEl.hidden = false;
  if (forkSeqEl) forkSeqEl.textContent = String(atSeq ?? 'latest');
}

function closeForkBar() {
  forkPoint = null;
  if (forkBarEl) forkBarEl.hidden = true;
}

function latestSeq() {
  const entries = (currentDag && currentDag.entries) || [];
  return entries.reduce((acc, e) => Math.max(acc, Number(e.seq) || 0), 0);
}

export async function refreshDag(sessionId) {
  const request = ++dagRequest;
  const isNewSession = sessionId !== currentSessionId;
  if (isNewSession) {
    currentSessionId = sessionId;
    sessionRevision += 1;
    currentDag = null;
    selectedEntry = null;
    viewState.hasAutoFit = false;
    viewState.userAdjusted = false;
    chosenView = null;
    closeForkBar();
    if (forkInputEl) forkInputEl.value = '';
    if (verifyBadgeEl) verifyBadgeEl.hidden = true;
    if (diffPanelEl) diffPanelEl.hidden = true;
    for (const id of ['dag-verify-btn', 'dag-narrate-btn', 'fork-confirm-btn']) {
      const button = document.getElementById(id);
      if (button) button.disabled = false;
    }
    const narrateBtn = document.getElementById('dag-narrate-btn');
    if (narrateBtn) narrateBtn.textContent = 'Narrate Proof';
    renderEmptySvg(sessionId ? 'Loading proof graph…' : 'Select a session to view its proof graph.');
    renderDagNodeDetail(detailEl, null, []);
    callbacks.onDagCount?.(0);
  }
  if (!sessionId || !svgEl) return 0;
  try {
    const dag = await api.getDag(sessionId);
    if (request !== dagRequest) return 0;
    currentDag = dag;
    const entries = Array.isArray(dag && dag.entries) ? dag.entries : [];
    const oldSelection = selectedEntry;
    selectedEntry = oldSelection && entries.find((e) => e.entry_id === oldSelection.entry_id);
    renderDagSvg(dag, isNewSession);
    if (oldSelection) renderDetailForEntry(selectedEntry);
    callbacks.onDagCount?.(entries.length);
    return entries.length;
  } catch {
    if (request !== dagRequest) return 0;
    currentDag = null;
    selectedEntry = null;
    renderEmptySvg('No proof graph recorded yet.');
    renderDagNodeDetail(detailEl, null, []);
    callbacks.onDagCount?.(0);
    return 0;
  }
}

function renderEmptySvg(msg) {
  if (!svgEl) return;
  stageEl = null;
  nodeCoords = new Map();
  readingOrder = [];
  for (const id of ['dag-views','dag-reading-list','dag-direction']) {
    const element = document.getElementById(id);
    if (element) element.hidden = true;
  }
  const map = document.getElementById('dag-map');
  if (map) map.hidden = false;
  svgEl.setAttribute('viewBox', '0 0 420 220');
  svgEl.innerHTML = `<text x="210" y="112" text-anchor="middle" fill="var(--ink-muted)" font-size="13">${escapeHtml(msg)}</text>`;
}

function statusPalette(status) {
  switch (String(status || 'pending').toLowerCase()) {
    case 'accepted':
      return { fill: 'var(--status-ok-bg)', stroke: 'var(--status-ok)', ink: 'var(--status-ok)' };
    case 'rejected':
      return { fill: 'var(--status-err-bg)', stroke: 'var(--status-err)', ink: 'var(--status-err)' };
    case 'superseded':
      return { fill: 'var(--status-muted-bg)', stroke: 'var(--border-strong)', ink: 'var(--status-muted)' };
    default:
      return { fill: 'var(--status-warn-bg)', stroke: 'var(--status-warn)', ink: 'var(--status-warn)' };
  }
}

function dependencyLayers(entries) {
  const byId = new Map(entries.map(entry => [entry.entry_id, entry]));
  const children = new Map();
  const pending = new Map();
  const depth = new Map();
  const queue = [];
  for (const entry of entries) {
    const parents = new Set((entry.depends_on || []).filter(id => byId.has(id)));
    pending.set(entry.entry_id, parents.size);
    if (!parents.size) queue.push(entry.entry_id);
    for (const parent of parents) {
      if (!children.has(parent)) children.set(parent, []);
      children.get(parent).push(entry.entry_id);
    }
  }
  const layers = [];
  for (let n = 0; n < queue.length; n++) {
    const id = queue[n];
    const level = depth.get(id) || 0;
    (layers[level] ||= []).push(byId.get(id));
    for (const child of children.get(id) || []) {
      depth.set(child, Math.max(depth.get(child) || 0, level + 1));
      pending.set(child, pending.get(child) - 1);
      if (!pending.get(child)) queue.push(child);
    }
  }
  const unresolved = entries.filter(entry => pending.get(entry.entry_id));
  if (unresolved.length) layers.push(unresolved);
  for (const layer of layers) layer.sort((a, b) => a.seq - b.seq);
  return layers;
}

function stepSentence(entry) {
  return firstMathSentence(entry.summary || entry.details) || 'No summary recorded.';
}

function stepLabel(entry) {
  return (entry.entry_type || 'Step').replaceAll('_', ' ') + ' · ' + (entry.status || 'pending');
}

function showReadingOrder(entries, visible) {
  const list = document.getElementById('dag-reading-list');
  if (!list) return;
  const previous = list.contains(document.activeElement) && document.activeElement.dataset.entryId;
  list.hidden = !visible;
  list.replaceChildren();
  if (!visible) return;
  for (const entry of entries) {
    const item = document.createElement('li');
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.entryId = entry.entry_id;
    button.setAttribute('aria-controls', 'dag-node-detail');
    button.setAttribute('aria-pressed', String(entry.entry_id === selectedEntry?.entry_id));
    const type = document.createElement('span');
    type.className = 'dag-step-label';
    type.textContent = stepLabel(entry);
    const sentence = document.createElement('span');
    sentence.className = 'dag-list-sentence';
    renderMathTextInto(sentence, stepSentence(entry), { inlineOnly: true });
    button.append(type, sentence);
    item.appendChild(button);
    button.addEventListener('click', () => { selectNode(entry); detailEl?.focus(); });
    button.addEventListener('keydown', key => moveBetweenSteps(key, entry.entry_id, list));
    list.appendChild(item);
    if (previous === entry.entry_id) button.focus({ preventScroll: true });
  }
}

function moveBetweenSteps(event, id, container) {
  const index = readingOrder.findIndex(entry => entry.entry_id === id);
  const destination = { ArrowDown: index + 1, ArrowRight: index + 1,
    ArrowUp: index - 1, ArrowLeft: index - 1, Home: 0, End: readingOrder.length - 1 }[event.key];
  if (destination === undefined) return false;
  event.preventDefault();
  const target = readingOrder[Math.min(readingOrder.length - 1, Math.max(0, destination))];
  const node = Array.from(container.querySelectorAll('[data-entry-id]')).find(el => el.getAttribute('data-entry-id') === target.entry_id);
  node?.focus();
  return true;
}

function placeCards(layers, width) {
  const cardWidth = Math.max(204, Math.min(330, width - 56));
  const measure = document.createElement('div');
  measure.className = 'dag-measure';
  measure.style.width = (cardWidth - 28) + 'px';
  const sizes = new Map();
  for (const entry of readingOrder) {
    const content = document.createElement('div');
    content.className = 'dag-node-summary';
    renderMathTextInto(content, stepSentence(entry), { inlineOnly: true });
    measure.appendChild(content);
    sizes.set(entry.entry_id, { content });
  }
  document.body?.appendChild(measure);
  for (const item of sizes.values()) {
    // Unbreakable math atoms may require a wider node. Never shrink its glyphs to fit.
    item.w = Math.max(cardWidth, (item.content.scrollWidth || 0) + 28);
    item.h = Math.max(65, (item.content.offsetHeight || 32) + 42);
  }
  measure.remove?.();
  const gap = 32;
  const rowWidth = row => row.reduce((n, entry) => n + sizes.get(entry.entry_id).w, (row.length-1)*gap);
  const total = layers.reduce((n, layer) => Math.max(n, rowWidth(layer)), 0);
  let y = 16;
  for (const row of layers) {
    let x = (total - rowWidth(row)) / 2 + 16;
    let tallest = 0;
    for (const entry of row) {
      const card = sizes.get(entry.entry_id);
      Object.assign(card, { x, y });
      tallest = Math.max(tallest, card.h);
      x += card.w + gap;
    }
    y += tallest + 56;
  }
  viewState.contentW = total + 32;
  viewState.contentH = y - 40;
  return sizes;
}

function renderDagSvg(dag, isNewSession = false) {
  const entries = dag.entries || [];
  if (!entries.length) { renderEmptySvg('Proof steps will appear here as they are recorded.'); return; }
  const layers = dependencyLayers(entries);
  readingOrder = layers.flat();
  const controls = document.getElementById('dag-views');
  if (controls) controls.hidden = false;
  lastWidth = controls?.clientWidth || viewportSize().vw;
  const listView = !!controls && (chosenView ? chosenView === 'list' : (lastWidth < 470 || entries.length >= 25));
  const map = document.getElementById('dag-map');
  const note = document.getElementById('dag-direction');
  if (map) map.hidden = listView;
  if (note) note.hidden = listView;
  document.getElementById('dag-view-graph')?.setAttribute('aria-pressed', String(!listView));
  document.getElementById('dag-view-list')?.setAttribute('aria-pressed', String(listView));
  showReadingOrder(readingOrder, listView);
  if (listView) { stageEl = null; svgEl.replaceChildren(); return; }

  const focused = document.activeElement;
  const focusId = focused && stageEl?.contains(focused) ? focused.getAttribute('data-entry-id') : null;
  nodeCoords = placeCards(layers, lastWidth);
  svgEl.removeAttribute('viewBox');
  svgEl.innerHTML = '';
  const svgNode = (name, attrs) => {
    const element = document.createElementNS(SVG_NS, name);
    for (const [key, value] of Object.entries(attrs)) element.setAttribute(key, String(value));
    return element;
  };
  const defs = svgNode('defs', {});
  defs.innerHTML = '<marker id="dag-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path d="M 0 0 L 10 5 L 0 10 z" fill="var(--ink-muted)"/></marker>';
  stageEl = svgNode('g', { class: 'dag-stage' });
  svgEl.append(defs, stageEl);
  for (const entry of readingOrder) {
    const to = nodeCoords.get(entry.entry_id);
    for (const prerequisite of entry.depends_on || []) {
      const from = nodeCoords.get(prerequisite);
      if (!from) continue;
      const x1 = from.x + from.w / 2, x2 = to.x + to.w / 2;
      const y1 = from.y + from.h, y2 = to.y - 3, mid = (y1+y2)/2;
      const edge = svgNode('path', { d: 'M '+x1+' '+y1+' C '+x1+' '+mid+', '+x2+' '+mid+', '+x2+' '+y2,
        class: 'dag-edge', 'data-prerequisite': prerequisite, 'data-consequence': entry.entry_id,
        fill: 'none', stroke: 'var(--ink-muted)', 'stroke-width': 1.6, 'marker-end': 'url(#dag-arrow)' });
      stageEl.appendChild(edge);
    }
  }
  let refocus;
  for (const entry of readingOrder) {
    const card = nodeCoords.get(entry.entry_id);
    const pal = statusPalette(entry.status);
    const selected = selectedEntry?.entry_id === entry.entry_id;
    const node = svgNode('g', { class: selected ? 'dag-node is-selected' : 'dag-node',
      'data-entry-id': entry.entry_id, transform: 'translate('+card.x+', '+card.y+')', role: 'button',
      tabindex: 0, 'aria-label': entry.entry_id+' · '+stepLabel(entry)+' · '+(entry.summary || ''),
      'aria-controls': 'dag-node-detail', 'aria-pressed': selected });
    const rect = svgNode('rect', { width: card.w, height: card.h, rx: 10, fill: pal.fill, stroke: pal.stroke });
    const label = svgNode('text', { x: 14, y: 21, 'font-size': 12, 'font-weight': 600, fill: pal.ink });
    label.textContent = stepLabel(entry);
    const box = svgNode('foreignObject', { x: 14, y: 29, width: card.w-28, height: card.h-33 });
    box.appendChild(card.content);
    node.append(rect, label, box);
    node.addEventListener('click', () => { if (!suppressNextNodeClick) selectNode(entry); });
    node.addEventListener('keydown', ev => {
      if (moveBetweenSteps(ev, entry.entry_id, stageEl)) return;
      if (ev.key !== 'Enter' && ev.key !== ' ') return;
      ev.preventDefault();
      if (!ev.repeat) { selectNode(entry); detailEl?.focus(); }
    });
    node.addEventListener('focus', () => {
      if (viewportEl && node.matches(':focus-visible')) centerOnNode(entry.entry_id);
    });
    stageEl.appendChild(node);
    if (entry.entry_id === focusId) refocus = node;
  }
  if (isNewSession || !viewState.hasAutoFit || !viewState.userAdjusted) {
    const { vw, vh } = viewportSize();
    const fit = Math.min((vw-24)/viewState.contentW, (vh-24)/viewState.contentH);
    if (fit >= .88) fitDagToView();
    else {
      viewState.scale = 1;
      const first = nodeCoords.get(selectedEntry?.entry_id || readingOrder[0].entry_id);
      viewState.tx = (vw-first.w)/2-first.x;
      viewState.ty = 14-first.y;
      viewState.hasAutoFit = true;
      applyViewTransform();
    }
  } else applyViewTransform();
  refocus?.focus({ preventScroll: true });
}

function updateSelectedHighlight() {
  const list = document.getElementById('dag-reading-list');
  for (const node of [...(stageEl?.querySelectorAll('.dag-node') || []), ...(list?.querySelectorAll('button') || [])]) {
    const selected = selectedEntry?.entry_id === node.getAttribute('data-entry-id');
    node.classList.toggle('is-selected', selected);
    node.setAttribute('aria-pressed', String(selected));
  }
}

export function selectNodeById(entryId, centerView = true) {
  if (!currentDag || !Array.isArray(currentDag.entries)) return false;
  const found = currentDag.entries.find((e) => e.entry_id === entryId);
  if (found) {
    selectNode(found);
    if (centerView) {
      if (stageEl) centerOnNode(found.entry_id);
      else document.querySelector('#dag-reading-list [aria-pressed="true"]')?.scrollIntoView({ block: 'nearest' });
    }
    return true;
  }
  return false;
}

function renderDetailForEntry(entry) {
  const entries = (currentDag && currentDag.entries) || [];
  const sid = currentSessionId;
  renderDagNodeDetail(detailEl, entry, entries, {
    loadEvidence: sid && ((id) => api.getProofEvidence(sid, id)),
    onReviewStep: sid && (async (entryId) => {
      await api.reviewClaim(sid, { entry_id: entryId });
      if (sid === currentSessionId) await refreshDag(sid);
    }),
    onNotify: callbacks.onNotify,
    onSelectNode: (id) => {
      if (selectNodeById(id)) detailEl?.focus();
      else callbacks.onNotify?.('This proof step is unavailable.');
    },
    onOpenSource: sid && ((anchor) => openSessionSource(sid, anchor.source_id, anchor.path, anchor)),
    onExplainStep: (ent) => typeof callbacks.onExplainStep === 'function' && callbacks.onExplainStep(ent),
    onForkAtStep: (ent) => openForkBarForSeq(ent.seq),
    onPromoteStep: callbacks.onPromoteStep && ((ent) => {
      if (currentSessionId === sid) callbacks.onPromoteStep(ent);
    }),
  });
}

function selectNode(entry) {
  selectedEntry = entry;
  updateSelectedHighlight();
  renderDetailForEntry(entry);
}

async function handleVerifyChain() {
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  const button = document.getElementById('dag-verify-btn');
  if (!sessionId || button?.disabled) return;
  if (button) button.disabled = true;
  try {
    const rep = await api.verifySession(sessionId);
    if (revision !== sessionRevision) return;
    if (verifyBadgeEl) {
      const head = String(rep.head_hash || rep.chain_head || '').slice(0, 12);
      const evCount = rep.event_count ?? rep.events_verified ?? 0;
      const blobs = rep.blob_refs_verified ?? rep.blobs_verified ?? 0;
      verifyBadgeEl.hidden = false;
      verifyBadgeEl.className = 'badge ok';
      verifyBadgeEl.textContent = `Record intact · ${evCount} events · ${blobs} file references`;
      verifyBadgeEl.title = head ? `SHA-256 chain head ${head}` : '';
    }
    await refreshDag(sessionId);
  } catch (err) {
    if (revision === sessionRevision && verifyBadgeEl) {
      verifyBadgeEl.hidden = false;
      verifyBadgeEl.className = 'badge err';
      verifyBadgeEl.title = '';
      verifyBadgeEl.textContent = `Verify failed: ${err.message}`;
    }
  } finally {
    if (revision === sessionRevision && button) button.disabled = false;
  }
}

async function handleConfirmFork() {
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  const button = document.getElementById('fork-confirm-btn');
  if (!sessionId || !forkPoint || button?.disabled) return;
  const at_seq = forkPoint.atSeq;
  const modified_assumption = forkInputEl && forkInputEl.value.trim() ? forkInputEl.value.trim() : undefined;
  if (button) button.disabled = true;
  try {
    const receipt = await api.forkSession(sessionId, { at_seq, modified_assumption });
    if (revision === sessionRevision) {
      closeForkBar();
      if (forkInputEl) forkInputEl.value = '';
    }
    const newId = receipt.new_session || receipt.id;
    if (newId) {
      let diff;
      if (revision === sessionRevision) {
        try { diff = await api.diffSessions(sessionId, newId); } catch { /* Optional diff summary */ }
      }
      await callbacks.onForkCreated?.(newId, revision === sessionRevision);
      if (currentSessionId === newId) renderBranchDiff(diff);
    }
    if (typeof receipt.warning === 'string' && receipt.warning.trim()
        && (revision === sessionRevision || currentSessionId === newId)) {
      callbacks.onNotify?.(receipt.warning);
    }
  } catch (err) {
    if (revision === sessionRevision) callbacks.onNotify?.(`Fork failed: ${err.message}`);
  } finally {
    if (revision === sessionRevision && button) button.disabled = false;
  }
}

function renderBranchDiff(diff) {
  if (!diffPanelEl || !diff) return;
  diffPanelEl.hidden = false;
  diffPanelEl.innerHTML = `
    <strong>Branch Diff: ${escapeHtml(diff.base_session || '')} -> ${escapeHtml(diff.target_session || '')}</strong>
    <div style="font-size:12px;color:var(--ink-secondary);margin:4px 0;">
      Shared prefix: ${escapeHtml(String(diff.shared_prefix_len ?? 0))} · Divergence seq: ${escapeHtml(String(diff.divergence_seq ?? 'none'))}
    </div>
    <pre class="trace-io">${escapeHtml(diff.summary_diff || '')}</pre>
  `;
}

async function handleNarrateProof() {
  const sessionId = currentSessionId;
  const revision = sessionRevision;
  const narrateBtn = document.getElementById('dag-narrate-btn');
  if (!sessionId || narrateBtn?.disabled) return;
  if (narrateBtn) {
    narrateBtn.disabled = true;
    narrateBtn.textContent = 'Narrating...';
  }
  try {
    const res = await api.narrateSession(sessionId, sideModelPayload());
    if (revision !== sessionRevision) return;
    const text = (res && (res.narrative || res.markdown || res.text)) || JSON.stringify(res);
    if (typeof callbacks.onNarrativeReady === 'function') callbacks.onNarrativeReady(text);
  } catch (err) {
    if (revision === sessionRevision) callbacks.onNotify?.(`Narration failed: ${err.message}`);
  } finally {
    if (revision === sessionRevision && narrateBtn) {
      narrateBtn.disabled = false;
      narrateBtn.textContent = 'Narrate Proof';
    }
  }
}
