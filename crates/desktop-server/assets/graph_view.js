// Shared interactive graph view over GraphView JSON across Investigate, Explore and Solve (overview 2.6).
// Draws top-to-bottom (src prerequisite -> dst dependent), pans, zooms, selects, filters, and folds >150 nodes.
import { DEFAULT_FOLD_THRESHOLD, filterGraphView, layoutGraphView } from './graph_layout.js';
import { BORDER_DASHED, BORDER_SOLID, createBadge, describeEvidenceGrade } from './ui_badge.js';

const SVG_NS = 'http://www.w3.org/2000/svg';
const MIN_SCALE = 0.08;
const MAX_SCALE = 3.0;

// Named constants for owner-accepted conventions (overview 2.6, 3.2 row 25, 8.2 #4).
// Decision 8.2 #4 & 3.2 row 25: dashed always means model-inferred, abstract-only or unchecked;
// statement uses vs proof uses differ by line weight and a toggle, never by dashes.
export const EDGE_DASH_ARRAY = '6 4';
export const EDGE_WEIGHTS = Object.freeze({ statement: 2.8, proof: 1.4, prose: 1.8, default: 1.9 });

export const GRAPH_VIEW_LABELS = Object.freeze({
  direction: 'Top to bottom: prerequisite → dependent',
  solidMeaning: 'Solid: by code or matched full-text quote',
  dashedMeaning: 'Dashed: model-inferred, abstract-only or unchecked',
  weightMeaning: 'Heavy line: statement uses · Light line: proof uses',
  toggleStatement: 'Statement uses',
  toggleProof: 'Proof uses',
  toggleModel: 'Model links',
  toggleHideDimmed: 'Hide dimmed',
  searchPlaceholder: 'Filter nodes…',
  emptyMessage: 'No graph nodes to display.',
  foldAction: 'Fold groups',
});

export function isEdgeDashed(edge) {
  if (!edge || edge.abstract_only) return true;
  const basis = String(edge.basis || '').trim().toLowerCase();
  return basis !== 'code' && basis !== 'evidence';
}

export function edgeStrokeWidth(edge) {
  const role = String(edge?.role || '').trim().toLowerCase();
  return EDGE_WEIGHTS[role] ?? EDGE_WEIGHTS.default;
}

export function isNodeDashed(node) {
  return describeEvidenceGrade(node?.grade).border === BORDER_DASHED;
}

function svgEl(tag, attrs = {}) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

function clampScale(s) {
  return Math.max(MIN_SCALE, Math.min(MAX_SCALE, s));
}

function edgePathD({ x1, y1, x2, y2 }) {
  const dy = y2 - y1;
  if (dy >= 12) {
    const mid = (y1 + y2) / 2;
    return `M ${x1} ${y1} C ${x1} ${mid}, ${x2} ${mid}, ${x2} ${y2}`;
  }
  const bend = Math.max(40, Math.abs(dy) / 2 + 28);
  const side = x2 >= x1 ? 36 : -36;
  return `M ${x1} ${y1} C ${x1 + side} ${y1 + bend}, ${x2 + side} ${y2 - bend}, ${x2} ${y2}`;
}

function nodeToneColors(grade, stale, selected) {
  if (selected) return { fill: 'var(--bg-active, #d3e3fd)', stroke: 'var(--accent, #0b57d0)' };
  if (stale) return { fill: 'var(--status-muted-bg, #eef1f5)', stroke: 'var(--border-strong, #c4c7c5)' };
  if (grade === 'code' || grade === 'quote_fulltext') {
    return { fill: 'var(--bg-elevated, #ffffff)', stroke: 'var(--border-strong, #c4c7c5)' };
  }
  if (grade === 'quote_ocr') {
    return { fill: 'var(--status-warn-bg, #fef7e0)', stroke: 'var(--status-warn-border, #f6d57a)' };
  }
  if (grade === 'user') {
    return { fill: 'var(--bg-surface, #f8fafd)', stroke: 'var(--accent, #0b57d0)' };
  }
  return { fill: 'var(--bg-subtle, #f1f4f9)', stroke: 'var(--ink-muted, #606569)' };
}

function buildNodeCard(node) {
  const body = document.createElement('div');
  body.className = 'gv-node-body';
  const labelEl = document.createElement('div');
  labelEl.className = 'gv-node-label';
  labelEl.textContent = String(node.label ?? node.id ?? '');
  body.appendChild(labelEl);

  const badgesWrap = document.createElement('div');
  badgesWrap.className = 'gv-node-badges';
  const gradeDesc = describeEvidenceGrade(node.grade);
  const rawBadges = Array.isArray(node.badges) && node.badges.length
    ? node.badges
    : [{ text: gradeDesc.defaultText, icon: gradeDesc.icon, tone: gradeDesc.tone }];
  for (const badge of rawBadges) {
    badgesWrap.appendChild(createBadge({ ...badge, tone: node.stale ? 'muted' : badge.tone }));
  }
  if (node.stale) badgesWrap.appendChild(createBadge({ text: 'Stale', icon: 'warn', tone: 'muted' }));
  body.appendChild(badgesWrap);

  if (Array.isArray(node.flags) && node.flags.length) {
    const flagsWrap = document.createElement('div');
    flagsWrap.className = 'gv-node-flags';
    for (const flag of node.flags) {
      const chip = document.createElement('span');
      chip.className = 'gv-node-flag';
      chip.textContent = String(flag);
      flagsWrap.appendChild(chip);
    }
    body.appendChild(flagsWrap);
  }
  return body;
}

function buildDefs() {
  const defs = svgEl('defs');
  const markers = [
    ['gv-arrow-solid', 'var(--ink-secondary, #444746)'],
    ['gv-arrow-dashed', 'var(--ink-muted, #606569)'],
    ['gv-arrow-selected', 'var(--accent, #0b57d0)'],
  ];
  for (const [id, fill] of markers) {
    const marker = svgEl('marker', { id, viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: 'auto' });
    marker.appendChild(svgEl('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill }));
    defs.appendChild(marker);
  }
  return defs;
}

export function createGraphView(host, initialGraphView = { nodes: [], edges: [], legend: [] }, options = {}) {
  let rawGraph = initialGraphView || { nodes: [], edges: [], legend: [] };
  let filter = {
    showStatementUses: true, showProofUses: true, showModelEdges: true,
    showModelNodes: true, hideDimmed: false, search: '',
    ...(options.initialFilter || {}),
  };
  let selectedNodeId = options.selectedId ?? null;
  let selectedEdgeKey = null;
  const expandedGroups = new Set(options.expandedGroups || []);
  const collapsedGroups = new Set(options.collapsedGroups || []);
  const foldThreshold = Number.isFinite(options.foldThreshold) ? options.foldThreshold : DEFAULT_FOLD_THRESHOLD;
  const viewState = { scale: 1, tx: 16, ty: 16, userAdjusted: false };
  let currentLayout = null;
  let stageGroup = null;
  let suppressNextClick = false;
  const cleanups = [];

  const root = document.createElement('section');
  root.className = 'gv-root';
  root.setAttribute('aria-label', String(options.ariaLabel || 'Dependency graph'));

  const toolbar = document.createElement('div');
  toolbar.className = 'gv-toolbar';
  toolbar.setAttribute('role', 'toolbar');
  toolbar.setAttribute('aria-label', 'Graph filters and zoom');

  const filterGroup = document.createElement('div');
  filterGroup.className = 'gv-filter-group';
  const makeToggle = (key, label, onToggle) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'pill-btn sm gv-toggle';
    btn.setAttribute('data-filter', key);
    btn.textContent = label;
    btn.addEventListener('click', () => { onToggle(); render(); });
    return btn;
  };
  const stmtBtn = makeToggle('statement', GRAPH_VIEW_LABELS.toggleStatement, () => { filter.showStatementUses = !filter.showStatementUses; });
  const proofBtn = makeToggle('proof', GRAPH_VIEW_LABELS.toggleProof, () => { filter.showProofUses = !filter.showProofUses; });
  const modelBtn = makeToggle('model', GRAPH_VIEW_LABELS.toggleModel, () => { filter.showModelEdges = !filter.showModelEdges; });
  const dimBtn = makeToggle('hide-dimmed', GRAPH_VIEW_LABELS.toggleHideDimmed, () => { filter.hideDimmed = !filter.hideDimmed; });

  const searchInput = document.createElement('input');
  searchInput.type = 'search';
  searchInput.className = 'text-input gv-search';
  searchInput.placeholder = GRAPH_VIEW_LABELS.searchPlaceholder;
  searchInput.setAttribute('aria-label', GRAPH_VIEW_LABELS.searchPlaceholder);
  searchInput.value = filter.search || '';
  searchInput.addEventListener('input', () => { filter.search = searchInput.value; render(); });
  filterGroup.append(stmtBtn, proofBtn, modelBtn, dimBtn, searchInput);

  const zoomGroup = document.createElement('div');
  zoomGroup.className = 'gv-zoom-controls';
  const makeZoomBtn = (text, label, action, onClick) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'dag-zoom-btn gv-zoom-btn';
    btn.setAttribute('data-zoom-action', action);
    btn.setAttribute('aria-label', label);
    btn.textContent = text;
    btn.addEventListener('click', onClick);
    return btn;
  };
  const zoomOutBtn = makeZoomBtn('−', 'Zoom out', 'out', () => zoomBy(0.8));
  const zoomResetBtn = makeZoomBtn('100%', 'Reset zoom to 100%', 'reset', () => resetZoom());
  const zoomInBtn = makeZoomBtn('+', 'Zoom in', 'in', () => zoomBy(1.25));
  const zoomFitBtn = makeZoomBtn('Fit', 'Fit graph to view', 'fit', () => fitToView());
  zoomGroup.append(zoomOutBtn, zoomResetBtn, zoomInBtn, zoomFitBtn);
  toolbar.append(filterGroup, zoomGroup);

  const foldBanner = document.createElement('div');
  foldBanner.className = 'gv-fold-banner';
  foldBanner.setAttribute('role', 'status');
  foldBanner.hidden = true;

  const viewport = document.createElement('div');
  viewport.className = 'gv-viewport';
  viewport.tabIndex = 0;
  viewport.setAttribute('role', 'region');
  viewport.setAttribute('aria-label', 'Graph canvas');
  const svg = svgEl('svg', { class: 'gv-svg', role: 'img', 'aria-label': 'Dependency graph canvas' });
  viewport.appendChild(svg);

  const legendEl = document.createElement('div');
  legendEl.className = 'gv-legend';
  legendEl.setAttribute('aria-label', 'Graph legend');

  root.append(toolbar, foldBanner, viewport, legendEl);
  if (host) host.replaceChildren(root);

  const viewportSize = () => ({
    vw: Math.max(240, Number(viewport.clientWidth) || 640),
    vh: Math.max(200, Number(viewport.clientHeight) || 400),
  });

  function applyTransform() {
    if (stageGroup) {
      stageGroup.setAttribute('transform', `translate(${viewState.tx.toFixed(1)}, ${viewState.ty.toFixed(1)}) scale(${viewState.scale.toFixed(4)})`);
    }
    zoomResetBtn.textContent = `${Math.round(viewState.scale * 100)}%`;
  }

  function panBy(dx, dy) {
    viewState.tx += Number(dx) || 0;
    viewState.ty += Number(dy) || 0;
    viewState.userAdjusted = true;
    applyTransform();
  }

  function zoomAt(factor, cx, cy) {
    const prev = viewState.scale;
    const next = clampScale(prev * (Number(factor) || 1));
    if (Math.abs(next - prev) < 0.0005) return;
    const ratio = next / prev;
    viewState.tx = cx - (cx - viewState.tx) * ratio;
    viewState.ty = cy - (cy - viewState.ty) * ratio;
    viewState.scale = next;
    viewState.userAdjusted = true;
    applyTransform();
  }

  function zoomBy(factor) {
    const { vw, vh } = viewportSize();
    zoomAt(factor, vw / 2, vh / 2);
  }

  function fitToView() {
    if (!currentLayout) return;
    const { vw, vh } = viewportSize();
    const cw = Math.max(1, currentLayout.width);
    const ch = Math.max(1, currentLayout.height);
    const scale = clampScale(Math.min((vw - 40) / cw, (vh - 40) / ch, 1.1));
    viewState.scale = scale;
    viewState.tx = (vw - cw * scale) / 2;
    viewState.ty = Math.max(12, (vh - ch * scale) / 2);
    applyTransform();
  }

  function resetZoom() {
    viewState.scale = 1;
    viewState.tx = 16;
    viewState.ty = 16;
    viewState.userAdjusted = true;
    applyTransform();
  }

  let drag = null;
  const onPointerDown = (ev) => {
    if (ev.button != null && ev.button !== 0) return;
    drag = { id: ev.pointerId ?? 1, sx: ev.clientX, sy: ev.clientY, tx: viewState.tx, ty: viewState.ty, moved: false };
  };
  const onPointerMove = (ev) => {
    if (!drag || (ev.pointerId != null && ev.pointerId !== drag.id)) return;
    const dx = ev.clientX - drag.sx;
    const dy = ev.clientY - drag.sy;
    if (!drag.moved && Math.hypot(dx, dy) > 4) {
      drag.moved = true;
      viewport.classList?.add('is-panning');
    }
    if (drag.moved) {
      viewState.tx = drag.tx + dx;
      viewState.ty = drag.ty + dy;
      viewState.userAdjusted = true;
      applyTransform();
    }
  };
  const onPointerEnd = (ev) => {
    if (!drag || (ev.pointerId != null && ev.pointerId !== drag.id)) return;
    if (drag.moved) {
      suppressNextClick = true;
      setTimeout(() => { suppressNextClick = false; }, 0);
    }
    viewport.classList?.remove('is-panning');
    drag = null;
  };
  const onWheel = (ev) => {
    if (!ev.ctrlKey && !ev.metaKey) return;
    ev.preventDefault?.();
    const rect = viewport.getBoundingClientRect ? viewport.getBoundingClientRect() : { left: 0, top: 0 };
    zoomAt(ev.deltaY < 0 ? 1.14 : 0.88, ev.clientX - rect.left, ev.clientY - rect.top);
  };
  const onKeyDown = (ev) => {
    if (ev.target && ev.target !== viewport) return;
    const delta = { ArrowLeft: [60, 0], ArrowRight: [-60, 0], ArrowUp: [0, 60], ArrowDown: [0, -60] }[ev.key];
    if (!delta) return;
    ev.preventDefault?.();
    panBy(delta[0], delta[1]);
  };
  for (const [ev, fn, opts] of [
    ['pointerdown', onPointerDown], ['pointermove', onPointerMove], ['pointerup', onPointerEnd],
    ['pointercancel', onPointerEnd], ['wheel', onWheel, { passive: false }], ['keydown', onKeyDown],
  ]) {
    viewport.addEventListener(ev, fn, opts);
    cleanups.push(() => viewport.removeEventListener?.(ev, fn));
  }

  function renderLegend(legendItems) {
    legendEl.replaceChildren();
    const keyLine = document.createElement('p');
    keyLine.className = 'gv-legend-core';
    keyLine.textContent = [
      GRAPH_VIEW_LABELS.direction, GRAPH_VIEW_LABELS.solidMeaning,
      GRAPH_VIEW_LABELS.dashedMeaning, GRAPH_VIEW_LABELS.weightMeaning,
    ].join(' · ');
    legendEl.appendChild(keyLine);
    if (!Array.isArray(legendItems) || !legendItems.length) return;
    const list = document.createElement('div');
    list.className = 'gv-legend-items';
    for (const entry of legendItems) {
      if (typeof entry === 'string') {
        const item = document.createElement('span');
        item.className = 'gv-legend-item';
        item.textContent = entry;
        list.appendChild(item);
      } else if (entry && typeof entry === 'object') {
        list.appendChild(createBadge({
          text: entry.label || entry.text || entry.kind || 'Item',
          icon: entry.icon || 'info',
          tone: entry.tone || 'neutral',
          border: entry.dashed ? BORDER_DASHED : BORDER_SOLID,
        }));
      }
    }
    legendEl.appendChild(list);
  }

  function selectNode(id, { notify = true } = {}) {
    selectedNodeId = id ?? null;
    selectedEdgeKey = null;
    render();
    if (notify && selectedNodeId != null && typeof options.onSelectNode === 'function') {
      const node = currentLayout?.nodeById.get(selectedNodeId) || null;
      if (node) options.onSelectNode(node);
    }
  }

  function bindActivate(el, handler) {
    el.addEventListener('click', handler);
    el.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault?.(); handler(); }
    });
  }

  function render() {
    const hasDimmed = (rawGraph.nodes || []).some((n) => Boolean(n.dim));
    stmtBtn.setAttribute('aria-pressed', String(filter.showStatementUses !== false));
    proofBtn.setAttribute('aria-pressed', String(filter.showProofUses !== false));
    modelBtn.setAttribute('aria-pressed', String(filter.showModelEdges !== false));
    dimBtn.hidden = !hasDimmed;
    dimBtn.setAttribute('aria-pressed', String(Boolean(filter.hideDimmed)));

    const filtered = filterGraphView(rawGraph, filter);
    currentLayout = layoutGraphView(filtered, {
      threshold: foldThreshold, expandedGroups, collapsedGroups,
      metrics: options.metrics, measureNode: options.measureNode,
    });

    if (currentLayout.folded || expandedGroups.size > 0) {
      foldBanner.hidden = false;
      foldBanner.replaceChildren();
      const msg = document.createElement('span');
      msg.textContent = currentLayout.folded
        ? `Showing ${currentLayout.visibleNodeCount} items (${currentLayout.originalNodeCount} nodes folded above ${foldThreshold}). Select a folded box to expand.`
        : `Showing all ${currentLayout.visibleNodeCount} nodes.`;
      foldBanner.appendChild(msg);
      if (expandedGroups.size > 0) {
        const refoldBtn = document.createElement('button');
        refoldBtn.type = 'button';
        refoldBtn.className = 'pill-btn sm';
        refoldBtn.setAttribute('data-fold-action', 'refold');
        refoldBtn.textContent = GRAPH_VIEW_LABELS.foldAction;
        refoldBtn.addEventListener('click', () => { expandedGroups.clear(); render(); });
        foldBanner.appendChild(refoldBtn);
      }
    } else {
      foldBanner.hidden = true;
      foldBanner.replaceChildren();
    }

    svg.replaceChildren();
    svg.setAttribute('viewBox', `0 0 ${currentLayout.width} ${currentLayout.height}`);
    if (!currentLayout.nodes.length) {
      const empty = svgEl('text', {
        x: Math.round(currentLayout.width / 2), y: Math.round(currentLayout.height / 2),
        'text-anchor': 'middle', fill: 'var(--ink-muted, #606569)', 'font-size': 13,
      });
      empty.textContent = GRAPH_VIEW_LABELS.emptyMessage;
      svg.appendChild(empty);
      renderLegend(currentLayout.legend);
      return;
    }

    stageGroup = svgEl('g', { class: 'gv-stage' });
    svg.append(buildDefs(), stageGroup);

    if (currentLayout.hasLanes) {
      const lanesGroup = svgEl('g', { class: 'gv-lanes' });
      for (const lane of currentLayout.lanes) {
        const laneG = svgEl('g', { class: 'gv-lane', 'data-lane': lane.key ?? '' });
        laneG.appendChild(svgEl('rect', {
          x: lane.x, y: lane.y, width: lane.w, height: lane.h, rx: 10,
          fill: 'var(--bg-surface, #f8fafd)', stroke: 'var(--border-subtle, #dfe4ec)', 'stroke-width': 1,
        }));
        const title = svgEl('text', {
          x: lane.x + 12, y: lane.y + 22, class: 'gv-lane-title',
          fill: 'var(--ink-secondary, #444746)', 'font-size': 12, 'font-weight': 600,
        });
        title.textContent = lane.label || '';
        laneG.appendChild(title);
        lanesGroup.appendChild(laneG);
      }
      stageGroup.appendChild(lanesGroup);
    }

    const edgesGroup = svgEl('g', { class: 'gv-edges' });
    for (const edge of currentLayout.edges) {
      const dashed = isEdgeDashed(edge);
      const weight = edgeStrokeWidth(edge);
      const edgeKey = `${edge.src}->${edge.dst}:${edge.role || ''}:${edge.basis || ''}`;
      const selected = selectedEdgeKey === edgeKey;
      const stroke = selected ? 'var(--accent, #0b57d0)' : (dashed ? 'var(--ink-muted, #606569)' : 'var(--ink-secondary, #444746)');
      const markerId = selected ? 'gv-arrow-selected' : (dashed ? 'gv-arrow-dashed' : 'gv-arrow-solid');
      const d = edgePathD(edge);
      const srcLabel = currentLayout.nodeById.get(edge.src)?.label || edge.src;
      const dstLabel = currentLayout.nodeById.get(edge.dst)?.label || edge.dst;
      const desc = `${srcLabel} → ${dstLabel} · ${edge.role || edge.kind || 'link'} · ${dashed ? 'dashed (unchecked/model)' : 'solid (' + (edge.basis || 'code') + ')'}`;

      const g = svgEl('g', {
        class: selected ? 'gv-edge is-selected' : 'gv-edge',
        tabindex: 0, role: 'button', 'aria-label': desc,
        'data-src': edge.src, 'data-dst': edge.dst, 'data-kind': edge.kind || '',
        'data-role': edge.role || '', 'data-basis': edge.basis || '',
        'data-dashed': String(dashed), 'data-stroke-width': String(weight),
      });
      const title = svgEl('title');
      title.textContent = desc;
      const lineAttrs = { d, fill: 'none', stroke, 'stroke-width': weight, 'marker-end': `url(#${markerId})` };
      if (dashed) lineAttrs['stroke-dasharray'] = EDGE_DASH_ARRAY;
      g.append(title, svgEl('path', { d, fill: 'none', stroke: 'transparent', 'stroke-width': 14 }), svgEl('path', lineAttrs));
      bindActivate(g, () => {
        if (suppressNextClick) return;
        selectedEdgeKey = edgeKey;
        selectedNodeId = null;
        render();
        options.onSelectEdge?.(edge);
      });
      edgesGroup.appendChild(g);
    }
    stageGroup.appendChild(edgesGroup);

    const nodesGroup = svgEl('g', { class: 'gv-nodes' });
    for (const node of currentLayout.nodes) {
      const dashed = isNodeDashed(node);
      const grade = describeEvidenceGrade(node.grade).grade;
      const selected = selectedNodeId === node.id;
      const colors = nodeToneColors(grade, Boolean(node.stale), selected);
      const classes = ['gv-node', selected && 'is-selected', node.dim && 'is-dimmed', node.stale && 'is-stale', node.folded && 'is-folded'].filter(Boolean);
      const g = svgEl('g', {
        class: classes.join(' '), transform: `translate(${node.x}, ${node.y})`,
        role: 'button', tabindex: 0, 'aria-pressed': String(selected),
        'aria-label': `${node.label || node.id}${node.kind ? ' · ' + node.kind : ''}`,
        'data-node-id': node.id, 'data-kind': node.kind || '', 'data-group': node.group || '',
        'data-grade': grade, 'data-dashed': String(dashed), 'data-dim': String(Boolean(node.dim)),
        'data-stale': String(Boolean(node.stale)), 'data-folded': String(Boolean(node.folded)),
      });
      if (node.dim) g.setAttribute('opacity', '0.52');
      const rectAttrs = { width: node.w, height: node.h, rx: 10, fill: colors.fill, stroke: colors.stroke, 'stroke-width': selected ? 2.6 : 1.5 };
      if (dashed) rectAttrs['stroke-dasharray'] = EDGE_DASH_ARRAY;
      g.appendChild(svgEl('rect', rectAttrs));
      const fo = svgEl('foreignObject', { x: 8, y: 6, width: node.w - 16, height: node.h - 12 });
      fo.appendChild(buildNodeCard(node));
      g.appendChild(fo);
      bindActivate(g, () => {
        if (suppressNextClick) return;
        if (node.folded && node.foldedKey) {
          expandedGroups.add(node.foldedKey);
          collapsedGroups.delete(node.foldedKey);
          render();
          options.onExpandGroup?.(node.foldedKey, node);
          return;
        }
        selectNode(node.id, { notify: true });
      });
      nodesGroup.appendChild(g);
    }
    stageGroup.appendChild(nodesGroup);

    if (!viewState.userAdjusted) fitToView();
    else applyTransform();
    renderLegend(currentLayout.legend);
  }

  render();

  return {
    get element() { return root; },
    setData(nextGraphView) { rawGraph = nextGraphView || { nodes: [], edges: [], legend: [] }; render(); },
    setFilter(partial = {}) { filter = { ...filter, ...partial }; searchInput.value = filter.search || ''; render(); },
    getFilter() { return { ...filter }; },
    selectNode(id) { selectNode(id, { notify: false }); },
    getSelectedId() { return selectedNodeId; },
    expandGroup(key) { expandedGroups.add(key); collapsedGroups.delete(key); render(); },
    collapseGroup(key) { collapsedGroups.add(key); expandedGroups.delete(key); render(); },
    panBy, zoomBy, fitToView, resetZoom,
    getViewTransform() { return { scale: viewState.scale, tx: viewState.tx, ty: viewState.ty }; },
    getLayout() { return currentLayout; },
    destroy() { for (const fn of cleanups) fn(); if (host) host.replaceChildren(); },
  };
}
