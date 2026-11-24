// Tab 2: Proof Graph & Ledger Canvas — zoomable/pannable SVG DAG, SHA-256 chain verify, fork, diff, and narration.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';
import { getModelChoice } from '/assets/model_picker.js';
import { renderDagNodeDetail } from '/assets/canvas_dag_detail.js';
import { openSessionSource } from '/assets/canvas_sources.js';

function sideModelPayload() {
  const c = getModelChoice();
  return c ? { provider: c.provider, model: c.model } : {};
}

const SVG_NS = 'http://www.w3.org/2000/svg';
const NODE_W = 176;
const NODE_H = 58;
const MIN_SCALE = 0.28;
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
  document.getElementById('dag-zoom-fit')?.addEventListener('click', () => { viewState.userAdjusted = false; fitDagToView(); });
  zoomResetBtn?.addEventListener('click', resetZoomOneToOne);

  wirePanAndWheel();
}

export function onDagTabShown() {
  if (!viewState.userAdjusted && currentDag && Array.isArray(currentDag.entries) && currentDag.entries.length > 0) {
    fitDagToView();
  }
}

function formatSvgSummary(raw) {
  let s = String(raw || '')
    .replace(/\\alpha/g, 'α').replace(/\\beta/g, 'β').replace(/\\gamma/g, 'γ')
    .replace(/\\delta/g, 'δ').replace(/\\Delta/g, 'Δ').replace(/\\epsilon/g, 'ε')
    .replace(/\\theta/g, 'θ').replace(/\\lambda/g, 'λ').replace(/\\mu/g, 'μ')
    .replace(/\\pi/g, 'π').replace(/\\sigma/g, 'σ').replace(/\\phi/g, 'φ')
    .replace(/\\psi/g, 'ψ').replace(/\\omega/g, 'ω').replace(/\\Omega/g, 'Ω')
    .replace(/\\hbar/g, 'ħ').replace(/[$\\`{}]/g, '')
    .replace(/\s+/g, ' ').trim();
  return s.length > 24 ? `${s.slice(0, 23)}…` : s;
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
  const { vw, vh } = viewportSize();
  viewState.userAdjusted = true;
  viewState.scale = 1;
  viewState.tx = (vw - viewState.contentW) / 2;
  viewState.ty = Math.max(16, (vh - viewState.contentH) / 2);
  applyViewTransform();
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
  const cx = pos.x + NODE_W / 2;
  const cy = pos.y + NODE_H / 2;
  viewState.tx = vw / 2 - cx * viewState.scale;
  viewState.ty = vh / 2 - cy * viewState.scale;
  viewState.userAdjusted = true;
  applyViewTransform();
}

function wirePanAndWheel() {
  if (!viewportEl) return;
  viewportEl.addEventListener('wheel', (ev) => {
    if (!stageEl || !currentDag || !(currentDag.entries || []).length) return;
    ev.preventDefault();
    const rect = viewportEl.getBoundingClientRect();
    const mx = ev.clientX - rect.left;
    const my = ev.clientY - rect.top;
    const factor = ev.deltaY < 0 ? 1.14 : 0.88;
    zoomAtPoint(factor, mx, my);
  }, { passive: false });

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

function renderDagSvg(dag, isNewSession = false) {
  const entries = Array.isArray(dag && dag.entries) ? dag.entries : [];
  if (entries.length === 0) {
    renderEmptySvg('Proof steps recorded by ledger_append appear here.');
    return;
  }

  const byId = new Map(entries.map((e) => [e.entry_id, e]));
  const depthMap = new Map();
  const calcDepth = (id, seen = new Set()) => {
    if (depthMap.has(id)) return depthMap.get(id);
    if (seen.has(id)) return 0;
    seen.add(id);
    const node = byId.get(id);
    const deps = (node && Array.isArray(node.depends_on) ? node.depends_on : []).filter((d) => byId.has(d));
    const d = deps.length === 0 ? 0 : 1 + Math.max(...deps.map((p) => calcDepth(p, seen)));
    depthMap.set(id, d);
    return d;
  };
  for (const e of entries) calcDepth(e.entry_id);

  const layers = new Map();
  for (const e of entries) {
    const layer = depthMap.get(e.entry_id) || 0;
    if (!layers.has(layer)) layers.set(layer, []);
    layers.get(layer).push(e);
  }

  const gapX = 28;
  const gapY = 46;
  const maxCols = Math.max(1, ...Array.from(layers.values()).map((arr) => arr.length));
  const width = Math.max(380, maxCols * (NODE_W + gapX));
  const height = Math.max(200, layers.size * (NODE_H + gapY));
  viewState.contentW = width;
  viewState.contentH = height;

  const focused = document.activeElement;
  const focusedEntryId = focused && stageEl?.contains(focused) ? focused.getAttribute('data-entry-id') : null;
  svgEl.removeAttribute('viewBox');
  svgEl.innerHTML = '';

  const defs = document.createElementNS(SVG_NS, 'defs');
  defs.innerHTML = `
    <marker id="dag-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1 L 10 5 L 0 9 z" fill="var(--border-strong)"/>
    </marker>
  `;
  svgEl.appendChild(defs);

  stageEl = document.createElementNS(SVG_NS, 'g');
  stageEl.setAttribute('class', 'dag-stage');
  svgEl.appendChild(stageEl);

  nodeCoords = new Map();
  const sortedLayers = Array.from(layers.keys()).sort((a, b) => a - b);
  for (const lvl of sortedLayers) {
    const row = layers.get(lvl);
    const rowW = row.length * NODE_W + (row.length - 1) * gapX;
    const startX = (width - rowW) / 2;
    const y = 16 + lvl * (NODE_H + gapY);
    row.forEach((entry, idx) => {
      nodeCoords.set(entry.entry_id, { x: startX + idx * (NODE_W + gapX), y });
    });
  }

  const edges = Array.isArray(dag.edges) ? dag.edges : [];
  const edgePairs = edges.length > 0
    ? edges
    : entries.flatMap((e) => (e.depends_on || []).map((dep) => [dep, e.entry_id]));

  for (const pair of edgePairs) {
    const fromId = Array.isArray(pair) ? pair[0] : pair.from;
    const toId = Array.isArray(pair) ? pair[1] : pair.to;
    const a = nodeCoords.get(fromId);
    const b = nodeCoords.get(toId);
    if (!a || !b) continue;
    const path = document.createElementNS(SVG_NS, 'path');
    const x1 = a.x + NODE_W / 2;
    const y1 = a.y + NODE_H;
    const x2 = b.x + NODE_W / 2;
    const y2 = b.y;
    const midY = (y1 + y2) / 2;
    path.setAttribute('d', `M ${x1} ${y1} C ${x1} ${midY}, ${x2} ${midY}, ${x2} ${y2}`);
    path.setAttribute('fill', 'none');
    path.setAttribute('stroke', 'var(--border-strong)');
    path.setAttribute('stroke-width', '1.6');
    path.setAttribute('marker-end', 'url(#dag-arrow)');
    stageEl.appendChild(path);
  }

  let nodeToRefocus;
  for (const entry of entries) {
    const pos = nodeCoords.get(entry.entry_id);
    if (!pos) continue;
    const pal = statusPalette(entry.status);
    const g = document.createElementNS(SVG_NS, 'g');
    const selected = selectedEntry?.entry_id === entry.entry_id;
    g.setAttribute('class', selected ? 'dag-node is-selected' : 'dag-node');
    g.setAttribute('data-entry-id', entry.entry_id);
    g.setAttribute('transform', `translate(${pos.x}, ${pos.y})`);
    g.setAttribute('role', 'button');
    g.setAttribute('tabindex', '0');
    g.setAttribute('aria-label', `${entry.entry_id} · ${entry.entry_type || 'step'} · ${entry.status || 'pending'}${entry.summary ? ` · ${entry.summary}` : ''}`);
    g.setAttribute('aria-controls', 'dag-node-detail');
    g.setAttribute('aria-pressed', String(selected));

    const rect = document.createElementNS(SVG_NS, 'rect');
    rect.setAttribute('width', String(NODE_W));
    rect.setAttribute('height', String(NODE_H));
    rect.setAttribute('rx', '10');
    rect.setAttribute('fill', pal.fill);
    rect.setAttribute('stroke', pal.stroke);

    const t1 = document.createElementNS(SVG_NS, 'text');
    t1.setAttribute('x', '10');
    t1.setAttribute('y', '20');
    t1.setAttribute('font-size', '11');
    t1.setAttribute('font-weight', '600');
    t1.setAttribute('fill', pal.ink);
    t1.textContent = `${entry.entry_id} · ${entry.entry_type || 'step'}`;

    const t2 = document.createElementNS(SVG_NS, 'text');
    t2.setAttribute('x', '10');
    t2.setAttribute('y', '39');
    t2.setAttribute('font-size', '11.5');
    t2.setAttribute('fill', 'var(--ink-primary)');
    t2.textContent = formatSvgSummary(entry.summary);

    g.appendChild(rect);
    g.appendChild(t1);
    g.appendChild(t2);
    g.addEventListener('click', () => {
      if (suppressNextNodeClick) return;
      selectNode(entry);
    });
    g.addEventListener('keydown', (ev) => {
      if (ev.key !== 'Enter' && ev.key !== ' ') return;
      ev.preventDefault();
      if (!ev.repeat) {
        selectNode(entry);
        detailEl?.focus();
      }
    });
    g.addEventListener('focus', () => {
      if (!viewportEl || !g.matches(':focus-visible')) return;
      const view = viewportEl.getBoundingClientRect();
      const node = g.getBoundingClientRect();
      if (node.left < view.left || node.right > view.right || node.top < view.top || node.bottom > view.bottom) {
        centerOnNode(entry.entry_id);
      }
    });
    stageEl.appendChild(g);
    if (entry.entry_id === focusedEntryId) nodeToRefocus = g;
  }

  if (isNewSession || !viewState.hasAutoFit || !viewState.userAdjusted) {
    fitDagToView();
  } else {
    applyViewTransform();
  }
  nodeToRefocus?.focus({ preventScroll: true });
}

function updateSelectedHighlight() {
  if (!stageEl) return;
  for (const node of stageEl.querySelectorAll('.dag-node')) {
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
    if (centerView) centerOnNode(found.entry_id);
    return true;
  }
  return false;
}

function renderDetailForEntry(entry) {
  const entries = (currentDag && currentDag.entries) || [];
  const sid = currentSessionId;
  renderDagNodeDetail(detailEl, entry, entries, {
    loadEvidence: sid && ((id) => api.getProofEvidence(sid, id)),
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
      verifyBadgeEl.textContent = `Verified ${head || 'SHA-256'} (${evCount} ev, ${blobs} blobs)`;
    }
    await refreshDag(sessionId);
  } catch (err) {
    if (revision === sessionRevision && verifyBadgeEl) {
      verifyBadgeEl.hidden = false;
      verifyBadgeEl.className = 'badge err';
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
