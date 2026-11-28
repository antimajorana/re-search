// Pure graph layout, topology, group lanes, filtering, and folding over GraphView (overview 2.6).
// Extracted from investigation_graph.js so all modes share one deterministic, DOM-free layout core.

// Overview 2.6 & investigate.md section 3 stage J: fold graphs above 150 visible nodes.
export const DEFAULT_FOLD_THRESHOLD = 150;

export const DEFAULT_LAYOUT_METRICS = Object.freeze({
  nodeWidth: 224,
  nodeHeight: 92,
  hGap: 24,
  vGap: 56,
  laneGap: 32,
  lanePadding: 16,
  laneHeaderHeight: 34,
  padding: 24,
});

export function edgeSrc(edge) {
  return edge?.src ?? edge?.from ?? null;
}

export function edgeDst(edge) {
  return edge?.dst ?? edge?.to ?? null;
}

export function computeTopology(nodes = [], edges = [], { includeCyclic = true } = {}) {
  const safeNodes = Array.isArray(nodes) ? nodes : [];
  const safeEdges = Array.isArray(edges) ? edges : [];
  const incoming = new Map(safeNodes.map((n) => [n.id, []]));
  const outgoing = new Map(safeNodes.map((n) => [n.id, []]));
  for (const edge of safeEdges) {
    const src = edgeSrc(edge);
    const dst = edgeDst(edge);
    if (!src || !dst || src === dst) continue;
    if (!incoming.has(dst) || !outgoing.has(src)) continue;
    incoming.get(dst).push(edge);
    outgoing.get(src).push(edge);
  }
  const remaining = new Map([...incoming].map(([id, list]) => [id, list.length]));
  const order = safeNodes.filter((n) => !remaining.get(n.id)).map((n) => n.id);
  for (let i = 0; i < order.length; i++) {
    for (const edge of outgoing.get(order[i])) {
      const dst = edgeDst(edge);
      remaining.set(dst, remaining.get(dst) - 1);
      if (!remaining.get(dst)) order.push(dst);
    }
  }
  const seen = new Set(order);
  const cyclic = safeNodes.filter((n) => !seen.has(n.id)).map((n) => n.id);
  if (includeCyclic && cyclic.length) order.push(...cyclic);
  return { order, incoming, outgoing, cyclic };
}

export function projectVisibleGraph(nodes = [], edges = [], shown = []) {
  const visible = new Set(shown);
  const { order, incoming, outgoing } = computeTopology(nodes, edges, { includeCyclic: false });
  const projected = [];
  const level = new Map(order.map((id) => [id, 0]));
  for (const from of order.filter((id) => visible.has(id))) {
    const paths = new Map([[from, { via: new Set(), inferred: false, explicit: true, kinds: new Set() }]]);
    for (const id of order) {
      const current = paths.get(id);
      if (!current) continue;
      if (id !== from && visible.has(id)) {
        projected.push({
          from,
          to: id,
          via: [...current.via],
          inferred: current.inferred,
          explicit: current.explicit,
          kinds: [...current.kinds],
        });
        level.set(id, Math.max(level.get(id), level.get(from) + 1));
        continue;
      }
      for (const edge of outgoing.get(id)) {
        const to = edgeDst(edge);
        const next = paths.get(to) || { via: new Set(), inferred: false, explicit: false, kinds: new Set() };
        if (id !== from) next.via.add(id);
        for (const hidden of current.via) next.via.add(hidden);
        next.inferred ||= current.inferred || edge.basis === 'inferred' || edge.basis === 'model';
        next.explicit ||= current.explicit && (edge.basis === 'explicit' || edge.basis === 'code' || edge.basis === 'evidence');
        next.kinds.add(edge.kind);
        paths.set(to, next);
      }
    }
  }
  const rows = [];
  const neighbors = new Map();
  for (const id of order.filter((id) => visible.has(id))) {
    (rows[level.get(id)] ||= []).push(id);
    neighbors.set(
      id,
      [...new Set([
        ...incoming.get(id).map((e) => edgeSrc(e)),
        ...outgoing.get(id).map((e) => edgeDst(e)),
      ])].filter((x) => !visible.has(x)),
    );
  }
  return { rows, edges: projected, neighbors };
}

export function collectUpstreamDependencies(nodes = [], edges = [], resultId) {
  const { incoming, order } = computeTopology(nodes, edges, { includeCyclic: false });
  const keep = new Set([resultId]);
  for (const id of [...order].reverse()) {
    if (keep.has(id)) {
      for (const edge of incoming.get(id)) keep.add(edgeSrc(edge));
    }
  }
  return keep;
}

export function filterGraphView(graphView, filter = {}) {
  const nodes = Array.isArray(graphView?.nodes) ? graphView.nodes : [];
  const edges = Array.isArray(graphView?.edges) ? graphView.edges : [];
  const legend = Array.isArray(graphView?.legend) ? graphView.legend : [];
  const kinds = filter.kinds ? new Set(filter.kinds) : null;
  const groups = filter.groups ? new Set(filter.groups) : null;
  const roles = filter.roles ? new Set(filter.roles) : null;
  const bases = filter.bases ? new Set(filter.bases) : null;
  const query = String(filter.search || '').trim().toLowerCase();

  const keptNodes = nodes.filter((node) => {
    if (filter.hideDimmed && node.dim) return false;
    if (filter.showModelNodes === false && node.grade === 'model') return false;
    if (kinds && kinds.size > 0 && !kinds.has(node.kind)) return false;
    if (groups && groups.size > 0 && !groups.has(node.group ?? '')) return false;
    if (query) {
      const hay = [node.id, node.label, node.kind, node.group, ...(node.flags || [])]
        .filter(Boolean)
        .join(' ')
        .toLowerCase();
      if (!hay.includes(query)) return false;
    }
    return true;
  });

  const keptIds = new Set(keptNodes.map((n) => n.id));
  const keptEdges = edges.filter((edge) => {
    const src = edgeSrc(edge);
    const dst = edgeDst(edge);
    if (!keptIds.has(src) || !keptIds.has(dst)) return false;
    if (filter.showStatementUses === false && edge.role === 'statement') return false;
    if (filter.showProofUses === false && edge.role === 'proof') return false;
    if (filter.showModelEdges === false && (edge.basis === 'model' || edge.basis === 'unchecked')) return false;
    if (roles && roles.size > 0 && !roles.has(edge.role)) return false;
    if (bases && bases.size > 0 && !bases.has(edge.basis)) return false;
    return true;
  });

  return { nodes: keptNodes, edges: keptEdges, legend };
}

function resolveFoldTarget(node, byId, autoFold, expanded, collapsed) {
  if (node.parent && byId.has(node.parent)) {
    let cur = node.parent;
    const seen = new Set([node.id]);
    let target = null;
    while (cur && byId.has(cur) && !seen.has(cur)) {
      seen.add(cur);
      const shouldFoldParent = (autoFold && !expanded.has(cur)) || collapsed.has(cur);
      if (shouldFoldParent) target = cur;
      cur = byId.get(cur).parent;
    }
    if (target) return { kind: 'parent', key: target, id: target };
  }
  const groupKey = typeof node.group === 'string' && node.group.trim() ? node.group : null;
  if (groupKey) {
    const shouldFoldGroup = (autoFold && !expanded.has(groupKey)) || collapsed.has(groupKey);
    if (shouldFoldGroup) return { kind: 'group', key: groupKey, id: `group:${groupKey}` };
  } else if (autoFold && !expanded.has('__ungrouped__')) {
    return { kind: 'group', key: '__ungrouped__', id: 'group:__ungrouped__', label: 'Other' };
  }
  return null;
}

export function foldGraphView(graphView, options = {}) {
  const nodes = Array.isArray(graphView?.nodes) ? graphView.nodes : [];
  const edges = Array.isArray(graphView?.edges) ? graphView.edges : [];
  const legend = Array.isArray(graphView?.legend) ? graphView.legend : [];
  const threshold = Number.isFinite(options.threshold) ? options.threshold : DEFAULT_FOLD_THRESHOLD;
  const expanded = new Set(options.expandedGroups || []);
  const collapsed = new Set(options.collapsedGroups || []);
  const autoFold = Boolean(options.forceFold) || nodes.length > threshold;

  if (!autoFold && collapsed.size === 0) {
    return {
      nodes: nodes.map((n) => ({ ...n })),
      edges: edges.map((e) => ({ ...e, src: edgeSrc(e), dst: edgeDst(e) })),
      legend,
      folded: false,
      foldedGroups: [],
      originalNodeCount: nodes.length,
      visibleNodeCount: nodes.length,
    };
  }

  const byId = new Map(nodes.map((n) => [n.id, n]));
  const repOf = new Map();
  const groupMembers = new Map();
  const groupMeta = new Map();
  const visibleNodes = [];
  const visibleIndex = new Map();

  for (const node of nodes) {
    const target = resolveFoldTarget(node, byId, autoFold, expanded, collapsed);
    if (!target) {
      repOf.set(node.id, node.id);
      if (!visibleIndex.has(node.id)) {
        visibleIndex.set(node.id, visibleNodes.length);
        visibleNodes.push({ ...node });
      }
      continue;
    }
    repOf.set(node.id, target.id);
    if (!groupMembers.has(target.id)) {
      groupMembers.set(target.id, []);
      groupMeta.set(target.id, target);
    }
    groupMembers.get(target.id).push(node);
    if (!visibleIndex.has(target.id)) {
      visibleIndex.set(target.id, visibleNodes.length);
      visibleNodes.push(null);
    }
  }

  const foldedGroups = [];
  for (const [repId, members] of groupMembers.entries()) {
    const meta = groupMeta.get(repId);
    const parentNode = meta.kind === 'parent' ? byId.get(repId) : null;
    const allDim = members.every((m) => Boolean(m.dim)) && (!parentNode || Boolean(parentNode.dim));
    const anyStale = members.some((m) => Boolean(m.stale)) || Boolean(parentNode?.stale);
    const allModel = members.every((m) => m.grade === 'model') && (!parentNode || parentNode.grade === 'model');
    const count = members.length + (parentNode ? 1 : 0);
    const label = parentNode ? parentNode.label : (meta.label || meta.key);
    const summary = {
      ...(parentNode || {}),
      id: repId,
      kind: parentNode ? parentNode.kind : 'group_summary',
      label,
      group: parentNode ? (parentNode.group ?? null) : (meta.key === '__ungrouped__' ? null : meta.key),
      parent: parentNode ? (parentNode.parent ?? null) : null,
      grade: allModel ? 'model' : (parentNode?.grade || 'code'),
      badges: [
        ...(parentNode?.badges || []),
        { text: `${count} nodes`, icon: 'group', tone: 'neutral' },
      ],
      flags: [...(parentNode?.flags || []), 'folded'],
      stale: anyStale,
      dim: allDim,
      folded: true,
      foldedKey: meta.key,
      foldedCount: count,
      memberIds: members.map((m) => m.id),
      internalEdgeCount: 0,
    };
    visibleNodes[visibleIndex.get(repId)] = summary;
    foldedGroups.push({ id: repId, key: meta.key, label, count, memberIds: summary.memberIds });
  }

  const finalNodes = visibleNodes.filter(Boolean);
  const summaryById = new Map(finalNodes.map((n) => [n.id, n]));
  const edgeMap = new Map();

  for (const edge of edges) {
    const src = repOf.get(edgeSrc(edge)) ?? edgeSrc(edge);
    const dst = repOf.get(edgeDst(edge)) ?? edgeDst(edge);
    if (!summaryById.has(src) || !summaryById.has(dst)) continue;
    if (src === dst) {
      const box = summaryById.get(src);
      if (box && box.folded) box.internalEdgeCount += 1;
      continue;
    }
    const basis = edge.basis || 'model';
    const role = edge.role || '';
    const abs = Boolean(edge.abstract_only);
    // Keep distinct solid/dashed bases and roles separate so folding never masks model edges as code.
    const key = `${src}\0${dst}\0${role}\0${basis}\0${abs ? 1 : 0}`;
    const existing = edgeMap.get(key);
    if (existing) {
      existing.count += 1;
      if (existing.kind !== edge.kind) existing.kind = 'aggregated';
    } else {
      edgeMap.set(key, {
        ...edge,
        src,
        dst,
        basis,
        role,
        abstract_only: abs,
        count: 1,
      });
    }
  }

  return {
    nodes: finalNodes,
    edges: [...edgeMap.values()],
    legend,
    folded: foldedGroups.length > 0,
    foldedGroups,
    originalNodeCount: nodes.length,
    visibleNodeCount: finalNodes.length,
  };
}

export function assignLayers(nodes = [], edges = []) {
  const safeNodes = Array.isArray(nodes) ? nodes : [];
  const byId = new Map(safeNodes.map((n) => [n.id, n]));
  const indexById = new Map(safeNodes.map((n, i) => [n.id, i]));
  const { order, outgoing, cyclic } = computeTopology(safeNodes, edges, { includeCyclic: true });
  const cyclicSet = new Set(cyclic);
  const layerById = new Map();

  for (const id of order) {
    if (cyclicSet.has(id)) continue;
    const cur = layerById.get(id) ?? 0;
    layerById.set(id, cur);
    for (const edge of outgoing.get(id) || []) {
      const dst = edgeDst(edge);
      if (cyclicSet.has(dst)) continue;
      layerById.set(dst, Math.max(layerById.get(dst) ?? 0, cur + 1));
    }
  }

  const maxDagLayer = layerById.size ? Math.max(...layerById.values()) : -1;
  for (const id of cyclic) {
    layerById.set(id, maxDagLayer + 1);
  }

  const layers = [];
  for (const node of safeNodes) {
    const lv = layerById.get(node.id) ?? 0;
    (layers[lv] ||= []).push(node);
  }
  for (const layer of layers) {
    layer.sort((a, b) => (indexById.get(a.id) ?? 0) - (indexById.get(b.id) ?? 0));
  }

  return { layers: layers.filter(Boolean), layerById, order, cyclic };
}

export function buildLanes(nodes = []) {
  const safeNodes = Array.isArray(nodes) ? nodes : [];
  const laneOrder = [];
  const byKey = new Map();
  const ungrouped = [];

  for (const node of safeNodes) {
    const key = typeof node.group === 'string' && node.group.trim() ? node.group : null;
    if (!key) {
      ungrouped.push(node.id);
      continue;
    }
    if (!byKey.has(key)) {
      const lane = { key, label: key, index: laneOrder.length, nodeIds: [] };
      byKey.set(key, lane);
      laneOrder.push(lane);
    }
    byKey.get(key).nodeIds.push(node.id);
  }

  if (laneOrder.length === 0) {
    return {
      hasLanes: false,
      lanes: [{ key: null, label: '', index: 0, nodeIds: safeNodes.map((n) => n.id) }],
    };
  }

  if (ungrouped.length > 0) {
    laneOrder.push({ key: '__ungrouped__', label: 'Other', index: laneOrder.length, nodeIds: ungrouped });
  }
  return { hasLanes: true, lanes: laneOrder };
}

export function layoutGraphView(graphView, options = {}) {
  const m = { ...DEFAULT_LAYOUT_METRICS, ...(options.metrics || {}) };
  const foldedResult = foldGraphView(graphView, options);
  const { nodes, edges, legend } = foldedResult;
  const { layers, layerById, order, cyclic } = assignLayers(nodes, edges);
  const { hasLanes, lanes } = buildLanes(nodes);

  const measure = typeof options.measureNode === 'function' ? options.measureNode : null;
  const sizes = new Map();
  for (const node of nodes) {
    const measured = measure ? measure(node) : null;
    sizes.set(node.id, {
      w: Math.max(120, Number(measured?.w) || m.nodeWidth),
      h: Math.max(56, Number(measured?.h) || m.nodeHeight),
    });
  }

  const nodeById = new Map();
  const positionedNodes = [];
  const positionedLanes = [];
  let totalWidth = m.padding * 2;
  let totalHeight = m.padding * 2;

  if (!nodes.length) {
    return {
      nodes: [],
      nodeById,
      edges: [],
      layers: [],
      lanes: [],
      hasLanes: false,
      legend,
      folded: foldedResult.folded,
      foldedGroups: foldedResult.foldedGroups,
      originalNodeCount: foldedResult.originalNodeCount,
      visibleNodeCount: 0,
      cyclic: [],
      width: 420,
      height: 180,
    };
  }

  if (hasLanes) {
    const laneIndexByNode = new Map();
    for (const lane of lanes) {
      for (const id of lane.nodeIds) laneIndexByNode.set(id, lane.index);
    }
    const cellNodes = layers.map(() => lanes.map(() => []));
    layers.forEach((layer, li) => {
      for (const node of layer) {
        const laneIdx = laneIndexByNode.get(node.id) ?? 0;
        cellNodes[li][laneIdx].push(node);
      }
    });

    const laneWidths = lanes.map((_, laneIdx) => {
      let widest = m.nodeWidth + m.lanePadding * 2;
      for (let li = 0; li < layers.length; li++) {
        const cell = cellNodes[li][laneIdx];
        if (!cell.length) continue;
        const w = cell.reduce((acc, n) => acc + sizes.get(n.id).w, (cell.length - 1) * m.hGap) + m.lanePadding * 2;
        if (w > widest) widest = w;
      }
      return widest;
    });

    const laneX = [];
    let curX = m.padding;
    for (let i = 0; i < lanes.length; i++) {
      laneX.push(curX);
      curX += laneWidths[i] + m.laneGap;
    }
    totalWidth = Math.max(m.padding * 2, curX - m.laneGap + m.padding);

    let curY = m.padding + m.laneHeaderHeight + m.lanePadding;
    for (let li = 0; li < layers.length; li++) {
      let rowH = m.nodeHeight;
      for (let laneIdx = 0; laneIdx < lanes.length; laneIdx++) {
        const cell = cellNodes[li][laneIdx];
        if (!cell.length) continue;
        const cellW = cell.reduce((acc, n) => acc + sizes.get(n.id).w, (cell.length - 1) * m.hGap);
        let x = laneX[laneIdx] + (laneWidths[laneIdx] - cellW) / 2;
        for (const node of cell) {
          const sz = sizes.get(node.id);
          const placed = {
            ...node,
            layer: li,
            laneIndex: laneIdx,
            laneKey: lanes[laneIdx].key,
            x,
            y: curY,
            w: sz.w,
            h: sz.h,
          };
          positionedNodes.push(placed);
          nodeById.set(node.id, placed);
          if (sz.h > rowH) rowH = sz.h;
          x += sz.w + m.hGap;
        }
      }
      curY += rowH + m.vGap;
    }
    totalHeight = curY - m.vGap + m.lanePadding + m.padding;

    for (let i = 0; i < lanes.length; i++) {
      positionedLanes.push({
        ...lanes[i],
        x: laneX[i],
        y: m.padding,
        w: laneWidths[i],
        h: Math.max(m.laneHeaderHeight + m.nodeHeight, totalHeight - m.padding * 2),
      });
    }
  } else {
    const rowWidths = layers.map((layer) =>
      layer.reduce((acc, n) => acc + sizes.get(n.id).w, Math.max(0, layer.length - 1) * m.hGap),
    );
    const maxRowW = Math.max(...rowWidths, m.nodeWidth);
    totalWidth = maxRowW + m.padding * 2;
    let curY = m.padding;
    layers.forEach((layer, li) => {
      let x = m.padding + (maxRowW - rowWidths[li]) / 2;
      let rowH = m.nodeHeight;
      for (const node of layer) {
        const sz = sizes.get(node.id);
        const placed = {
          ...node,
          layer: li,
          laneIndex: 0,
          laneKey: null,
          x,
          y: curY,
          w: sz.w,
          h: sz.h,
        };
        positionedNodes.push(placed);
        nodeById.set(node.id, placed);
        if (sz.h > rowH) rowH = sz.h;
        x += sz.w + m.hGap;
      }
      curY += rowH + m.vGap;
    });
    totalHeight = curY - m.vGap + m.padding;
  }

  const routedEdges = [];
  for (const edge of edges) {
    const srcNode = nodeById.get(edgeSrc(edge));
    const dstNode = nodeById.get(edgeDst(edge));
    if (!srcNode || !dstNode) continue;
    routedEdges.push({
      ...edge,
      src: srcNode.id,
      dst: dstNode.id,
      srcLayer: layerById.get(srcNode.id) ?? 0,
      dstLayer: layerById.get(dstNode.id) ?? 0,
      x1: srcNode.x + srcNode.w / 2,
      y1: srcNode.y + srcNode.h,
      x2: dstNode.x + dstNode.w / 2,
      y2: dstNode.y,
    });
  }

  return {
    nodes: positionedNodes,
    nodeById,
    edges: routedEdges,
    layers,
    lanes: positionedLanes,
    hasLanes,
    legend,
    folded: foldedResult.folded,
    foldedGroups: foldedResult.foldedGroups,
    originalNodeCount: foldedResult.originalNodeCount,
    visibleNodeCount: positionedNodes.length,
    order,
    cyclic,
    width: Math.ceil(totalWidth),
    height: Math.ceil(totalHeight),
  };
}
