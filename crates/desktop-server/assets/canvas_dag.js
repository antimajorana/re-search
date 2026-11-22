// Tab 2: Proof Graph & Ledger Canvas — layered SVG DAG, SHA-256 chain verify, counterfactual fork, diff, and proof narration.
import * as api from '/assets/api.js';
import { escapeHtml, renderRichInto } from '/assets/math.js';
import { getModelChoice } from '/assets/model_picker.js';

// The model picker's current {provider, model}. On the Claude subscription route the
// server picks the first connected API provider (narration and side questions need one).
function sideModelPayload() {
  const c = getModelChoice();
  return c ? { provider: c.provider, model: c.model } : {};
}

const SVG_NS = 'http://www.w3.org/2000/svg';

let svgEl = null;
let detailEl = null;
let verifyBadgeEl = null;
let forkBarEl = null;
let forkSeqEl = null;
let forkInputEl = null;
let diffPanelEl = null;
let currentSessionId = null;
let currentDag = null;
let selectedEntry = null;
let callbacks = {};

export function initDagCanvas(opts = {}) {
  callbacks = opts;
  svgEl = document.getElementById('proof-dag-svg');
  detailEl = document.getElementById('dag-node-detail');
  verifyBadgeEl = document.getElementById('dag-verify-badge');
  forkBarEl = document.getElementById('dag-fork-bar');
  forkSeqEl = document.getElementById('fork-seq-label');
  forkInputEl = document.getElementById('fork-assumption-input');
  diffPanelEl = document.getElementById('dag-diff-panel');

  const verifyBtn = document.getElementById('dag-verify-btn');
  const forkBtn = document.getElementById('dag-fork-btn');
  const forkCancel = document.getElementById('fork-cancel-btn');
  const forkConfirm = document.getElementById('fork-confirm-btn');
  const narrateBtn = document.getElementById('dag-narrate-btn');

  if (verifyBtn) verifyBtn.addEventListener('click', handleVerifyChain);
  if (narrateBtn) narrateBtn.addEventListener('click', handleNarrateProof);
  if (forkBtn) {
    forkBtn.addEventListener('click', () => {
      if (!forkBarEl) return;
      const open = forkBarEl.hidden;
      forkBarEl.hidden = !open;
      if (open && forkSeqEl) {
        const seq = selectedEntry ? selectedEntry.seq : latestSeq();
        forkSeqEl.textContent = String(seq || 1);
      }
    });
  }
  if (forkCancel) forkCancel.addEventListener('click', () => { if (forkBarEl) forkBarEl.hidden = true; });
  if (forkConfirm) forkConfirm.addEventListener('click', handleConfirmFork);
}

function latestSeq() {
  const entries = (currentDag && currentDag.entries) || [];
  return entries.reduce((acc, e) => Math.max(acc, Number(e.seq) || 0), 0);
}

export async function refreshDag(sessionId) {
  currentSessionId = sessionId;
  if (!sessionId || !svgEl) return 0;
  try {
    const dag = await api.getDag(sessionId);
    currentDag = dag;
    renderDagSvg(dag);
    const count = Array.isArray(dag && dag.entries) ? dag.entries.length : 0;
    if (typeof callbacks.onDagCount === 'function') callbacks.onDagCount(count);
    return count;
  } catch {
    renderEmptySvg('No proof graph recorded yet.');
    if (typeof callbacks.onDagCount === 'function') callbacks.onDagCount(0);
    return 0;
  }
}

function renderEmptySvg(msg) {
  if (!svgEl) return;
  svgEl.setAttribute('viewBox', '0 0 420 200');
  svgEl.innerHTML = `<text x="210" y="104" text-anchor="middle" fill="var(--ink-muted)" font-size="13">${escapeHtml(msg)}</text>`;
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

function renderDagSvg(dag) {
  const entries = Array.isArray(dag && dag.entries) ? dag.entries : [];
  if (entries.length === 0) {
    renderEmptySvg('Proof entries appended via ledger_append appear here.');
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

  const nodeW = 164;
  const nodeH = 54;
  const gapX = 28;
  const gapY = 44;
  const maxCols = Math.max(1, ...Array.from(layers.values()).map((arr) => arr.length));
  const width = Math.max(420, maxCols * (nodeW + gapX) + 40);
  const height = Math.max(240, layers.size * (nodeH + gapY) + 40);
  svgEl.setAttribute('viewBox', `0 0 ${width} ${height}`);
  svgEl.innerHTML = '';

  const defs = document.createElementNS(SVG_NS, 'defs');
  defs.innerHTML = `
    <marker id="dag-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1 L 10 5 L 0 9 z" fill="var(--border-strong)"/>
    </marker>
  `;
  svgEl.appendChild(defs);

  const coords = new Map();
  const sortedLayers = Array.from(layers.keys()).sort((a, b) => a - b);
  for (const lvl of sortedLayers) {
    const row = layers.get(lvl);
    const rowW = row.length * nodeW + (row.length - 1) * gapX;
    const startX = (width - rowW) / 2;
    const y = 20 + lvl * (nodeH + gapY);
    row.forEach((entry, idx) => {
      coords.set(entry.entry_id, { x: startX + idx * (nodeW + gapX), y });
    });
  }

  const edges = Array.isArray(dag.edges) ? dag.edges : [];
  const edgePairs = edges.length > 0
    ? edges
    : entries.flatMap((e) => (e.depends_on || []).map((dep) => [dep, e.entry_id]));

  for (const pair of edgePairs) {
    const fromId = Array.isArray(pair) ? pair[0] : pair.from;
    const toId = Array.isArray(pair) ? pair[1] : pair.to;
    const a = coords.get(fromId);
    const b = coords.get(toId);
    if (!a || !b) continue;
    const path = document.createElementNS(SVG_NS, 'path');
    const x1 = a.x + nodeW / 2;
    const y1 = a.y + nodeH;
    const x2 = b.x + nodeW / 2;
    const y2 = b.y;
    const midY = (y1 + y2) / 2;
    path.setAttribute('d', `M ${x1} ${y1} C ${x1} ${midY}, ${x2} ${midY}, ${x2} ${y2}`);
    path.setAttribute('fill', 'none');
    path.setAttribute('stroke', 'var(--border-strong)');
    path.setAttribute('stroke-width', '1.5');
    path.setAttribute('marker-end', 'url(#dag-arrow)');
    svgEl.appendChild(path);
  }

  for (const entry of entries) {
    const pos = coords.get(entry.entry_id);
    if (!pos) continue;
    const pal = statusPalette(entry.status);
    const g = document.createElementNS(SVG_NS, 'g');
    g.setAttribute('class', `dag-node${selectedEntry && selectedEntry.entry_id === entry.entry_id ? ' is-selected' : ''}`);
    g.setAttribute('transform', `translate(${pos.x}, ${pos.y})`);

    const rect = document.createElementNS(SVG_NS, 'rect');
    rect.setAttribute('width', String(nodeW));
    rect.setAttribute('height', String(nodeH));
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
    const sum = String(entry.summary || '');
    t2.textContent = sum.length > 22 ? `${sum.slice(0, 21)}…` : sum;

    g.appendChild(rect);
    g.appendChild(t1);
    g.appendChild(t2);
    g.addEventListener('click', () => selectNode(entry));
    svgEl.appendChild(g);
  }
}

export function selectNodeById(entryId) {
  if (!currentDag || !Array.isArray(currentDag.entries)) return;
  const found = currentDag.entries.find((e) => e.entry_id === entryId);
  if (found) selectNode(found);
}

function selectNode(entry) {
  selectedEntry = entry;
  if (currentDag) renderDagSvg(currentDag);
  if (!detailEl) return;
  const pal = statusPalette(entry.status);
  detailEl.innerHTML = `
    <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:6px;">
      <strong>${escapeHtml(entry.entry_id)} (${escapeHtml(entry.entry_type || '')})</strong>
      <span class="badge" style="color:${pal.ink};border-color:${pal.stroke};background:${pal.fill};">${escapeHtml(entry.status || 'pending')}</span>
    </div>
    <div style="font-weight:500;margin-bottom:6px;">${escapeHtml(entry.summary || '')}</div>
    <div class="node-latex-body rich-prose" style="font-size:12.5px;margin-bottom:8px;"></div>
    <div style="display:flex;align-items:center;justify-content:space-between;gap:8px;font-size:11.5px;color:var(--ink-muted);">
      <span>seq ${escapeHtml(String(entry.seq || 0))} · deps: ${escapeHtml((Array.isArray(entry.depends_on) ? entry.depends_on : []).join(', ') || 'none')}</span>
      <button id="node-verify-step-btn" type="button" class="pill-btn sm">Verify Step</button>
    </div>
  `;
  const rawLatex = String(entry.latex || '').trim();
  const wrappedLatex = rawLatex
    ? (/^(\$\$|\\\[|\\begin\{)/.test(rawLatex) ? rawLatex : `$$\n${rawLatex}\n$$`)
    : '';
  const latexOrDetails = wrappedLatex ? `${wrappedLatex}\n\n${entry.details || ''}` : (entry.details || '');
  renderRichInto(detailEl.querySelector('.node-latex-body'), latexOrDetails);
  const stepBtn = detailEl.querySelector('#node-verify-step-btn');
  if (stepBtn) {
    stepBtn.addEventListener('click', () => handleVerifyChain());
  }
}

async function handleVerifyChain() {
  if (!currentSessionId) return;
  try {
    const rep = await api.verifySession(currentSessionId);
    if (verifyBadgeEl) {
      const head = String(rep.head_hash || rep.chain_head || '').slice(0, 12);
      const evCount = rep.event_count ?? rep.events_verified ?? 0;
      const blobs = rep.blob_refs_verified ?? rep.blobs_verified ?? 0;
      verifyBadgeEl.hidden = false;
      verifyBadgeEl.className = 'badge ok';
      verifyBadgeEl.textContent = `Verified ${head || 'SHA-256'} (${evCount} ev, ${blobs} blobs)`;
    }
    await refreshDag(currentSessionId);
  } catch (err) {
    if (verifyBadgeEl) {
      verifyBadgeEl.hidden = false;
      verifyBadgeEl.className = 'badge err';
      verifyBadgeEl.textContent = `Verify failed: ${err.message}`;
    }
  }
}

async function handleConfirmFork() {
  if (!currentSessionId) return;
  const at_seq = Number(selectedEntry && selectedEntry.seq ? selectedEntry.seq : latestSeq()) || 1;
  const modified_assumption = forkInputEl && forkInputEl.value.trim() ? forkInputEl.value.trim() : undefined;
  try {
    const receipt = await api.forkSession(currentSessionId, { at_seq, modified_assumption });
    if (forkBarEl) forkBarEl.hidden = true;
    if (forkInputEl) forkInputEl.value = '';
    const newId = receipt.new_session || receipt.id;
    if (newId) {
      try {
        const diff = await api.diffSessions(currentSessionId, newId);
        renderBranchDiff(diff);
      } catch {
        // Optional diff summary
      }
      if (typeof callbacks.onForkCreated === 'function') {
        callbacks.onForkCreated(newId);
      }
    }
  } catch (err) {
    if (typeof callbacks.onNotify === 'function') callbacks.onNotify(`Fork failed: ${err.message}`);
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
  if (!currentSessionId) return;
  try {
    const res = await api.narrateSession(currentSessionId, sideModelPayload());
    const text = (res && (res.narrative || res.markdown || res.text)) || JSON.stringify(res);
    if (typeof callbacks.onNarrativeReady === 'function') {
      callbacks.onNarrativeReady(text);
    }
  } catch (err) {
    if (typeof callbacks.onNotify === 'function') callbacks.onNotify(`Narration failed: ${err.message}`);
  }
}
