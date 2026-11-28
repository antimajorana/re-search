// Pure DOM-free model & projection helpers for Investigate focused and whole-paper graph views (Package V13).
// Adapts Investigate mapview JSON for F6 graph_view.js / graph_layout.js without duplicating layout or folding.

import { DEFAULT_FOLD_THRESHOLD, edgeDst, edgeSrc } from './graph_layout.js';
import { EDGE_DASH_ARRAY, EDGE_WEIGHTS, edgeStrokeWidth, isEdgeDashed } from './graph_view.js';

// Owner-accepted conventions (00-overview.md §2.6, §3.2 rows 20/24/25, §8.2 #4; investigate.md §3, §4.3, §6.14).
export const VIEW_MODES = Object.freeze({ RESULTS: 'results', FOCUSED: 'focused', WHOLE: 'whole' });
export const RESULT_KINDS = Object.freeze(['theorem', 'lemma', 'proposition', 'corollary', 'conjecture', 'claim']);
export const STATEMENT_FALLBACK_KINDS = Object.freeze(['definition', 'assumption', 'equation', 'algorithm']);
export const STRUCTURAL_KINDS = Object.freeze(['section', 'front_matter', 'bibliography', 'proof']);

// Authors' novelty claims are always model-extracted and labelled "(model)" (00-overview.md §3.2 row 24).
export const AUTHORS_CLAIM_OPTIONS = Object.freeze([
  { id: 'new', label: 'New (model)' },
  { id: 'restated', label: 'Restated (model)' },
  { id: 'background', label: 'Background (model)' },
  { id: 'not_stated', label: 'Not stated (model)' },
]);

// User-facing labels obey 00-overview.md §3.2 row 20 & §8.2 #4 (zero forbidden certainty words).
export const WHOLEMAP_LABELS = Object.freeze({
  modeResults: 'Main results',
  modeResultsFallback: 'Results overview',
  modeFocused: 'Focused node',
  modeWhole: 'Whole map (M)',
  statementUses: 'Statement uses',
  proofUses: 'Proof uses',
  usedBy: 'Used by',
  otherUses: 'Other links',
  breadcrumbLabel: 'Argument trail',
  backButton: '← Back',
  mainResultsOnly: 'Main results only',
  authorsLabelHeading: "Authors' label (model)",
  sectionsHeading: 'Sections',
  collapseAll: 'Collapse all',
  expandAll: 'Expand all',
  skipBasisExplicit: 'by explicit cross-references',
  skipBasisModel: 'by cross-references (includes model-inferred links)',
  skipEmptyNote: 'No main results yet — nothing is dimmed by cross-references.',
  heuristicTag: 'heuristic',
  emptyFocus: 'Select a node to inspect its immediate neighbours.',
  emptyNeighbours: 'None',
});

const RESULT_KIND_SET = new Set(RESULT_KINDS);
const FALLBACK_KIND_SET = new Set(STATEMENT_FALLBACK_KINDS);
const STRUCTURAL_KIND_SET = new Set(STRUCTURAL_KINDS);
const FORBIDDEN_WORD_RE = new RegExp('\\b(?:' + 'verif' + 'ied|not\\s+' + 'needed)\\b', 'i');
const UNREF_FLAG_RE = /not referred to by any main result/i;

const isSafeText = (text) => typeof text === 'string' && text.trim().length > 0 && !FORBIDDEN_WORD_RE.test(text);

function normalizeAuthorsId(raw) {
  const clean = String(raw || '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  return (clean === 'new' || clean === 'restated' || clean === 'background' || clean === 'not_stated') ? clean : null;
}

export function isHeuristicEdge(edge) {
  if (!edge || typeof edge !== 'object') return false;
  const rule = String(edge.rule || '').trim().toLowerCase();
  const confidence = String(edge.confidence || '').trim().toLowerCase();
  const basis = String(edge.basis || '').trim().toLowerCase();
  return rule === 'text_mention' || confidence === 'heuristic' || basis === 'heuristic';
}

// Heuristic links force basis='model' so F6 draws them dashed 6 4 and hides them under Model links (00-overview.md §8.2 #4).
export function normalizeEdge(edge) {
  if (!edge || typeof edge !== 'object') {
    return { src: null, dst: null, kind: 'uses', role: '', heuristic: false, basis: 'model' };
  }
  const heuristic = isHeuristicEdge(edge);
  const rawBasis = String(edge.basis || '').trim().toLowerCase();
  return {
    ...edge,
    src: edgeSrc(edge),
    dst: edgeDst(edge),
    kind: String(edge.kind || 'uses').trim().toLowerCase(),
    role: String(edge.role || '').trim().toLowerCase(),
    heuristic,
    basis: heuristic ? 'model' : (rawBasis || 'model'),
  };
}

export function extractAuthorsClaimLabel(node) {
  if (!node || typeof node !== 'object') return null;
  const direct = normalizeAuthorsId(node.authors_claim?.label) || normalizeAuthorsId(node.authors_label);
  if (direct) return direct;
  for (const badge of Array.isArray(node.badges) ? node.badges : []) {
    const text = String(badge?.text ?? badge?.label ?? '').trim();
    const match = /^authors:\s*(new|restated|background|not[\s_-]+stated)\b/i.exec(text);
    const parsed = match ? normalizeAuthorsId(match[1]) : null;
    if (parsed) return parsed;
  }
  return null;
}

// Reader pin overrides model role (investigate.md §4.3).
export function isMainResultNode(node, mainResultSet) {
  if (!node || typeof node !== 'object') return false;
  if (node.pin === 'not_main_result' || node.unpinned === true) return false;
  if (node.pinned === true || node.pin === 'main_result') return true;
  if (mainResultSet && node.id != null && mainResultSet.has(node.id)) return true;
  if (String(node.role || '').trim().toLowerCase() === 'main_result') return true;
  for (const flag of Array.isArray(node.flags) ? node.flags : []) {
    const clean = String(flag || '').trim().toLowerCase();
    if (clean === 'pinned' || clean === 'main_result') return true;
  }
  for (const badge of Array.isArray(node.badges) ? node.badges : []) {
    if (String(badge?.text ?? badge?.label ?? '').trim().toLowerCase().startsWith('main result')) return true;
  }
  return false;
}

function addCandidateIds(set, rawList, nodeById) {
  if (!Array.isArray(rawList)) return;
  for (const item of rawList) {
    const id = typeof item === 'string' ? item : (item?.id ?? item?.tag ?? null);
    if (id != null && nodeById.has(id)) set.add(id);
  }
}

function collectDimmedSectionKeys(skipPlan) {
  const keys = new Set();
  for (const sec of Array.isArray(skipPlan?.sections) ? skipPlan.sections : []) {
    if (!sec || !sec.dim) continue;
    for (const field of [sec.label, sec.display_label, sec.section_tag, sec.tag]) {
      if (typeof field === 'string' && field.trim()) {
        const clean = field.trim();
        keys.add(clean);
      }
    }
    const lbl = typeof sec.label === 'string' ? sec.label.trim() : '';
    if (lbl && !/^(?:section|appendix)\b/i.test(lbl)) keys.add(`Section: ${lbl}`);
  }
  return keys;
}

function nodeMatchesDimmedSection(node, nodeById, dimmedKeys) {
  if (!node || !dimmedKeys || dimmedKeys.size === 0) return false;
  const g = typeof node.group === 'string' ? node.group.trim() : '';
  if (g !== '' && dimmedKeys.has(g)) return true;
  let cur = typeof node.parent === 'string' ? node.parent.trim() : '';
  const seen = new Set([node.id]);
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    if (dimmedKeys.has(cur)) return true;
    const parentNode = nodeById?.get(cur);
    if (!parentNode) break;
    const pg = typeof parentNode.group === 'string' ? parentNode.group.trim() : '';
    if (pg !== '' && dimmedKeys.has(pg)) return true;
    cur = typeof parentNode.parent === 'string' ? parentNode.parent.trim() : '';
  }
  return false;
}

export function inspectMapviewCapabilities(mapview) {
  const allNodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const nodeById = new Map();
  for (const node of allNodes) if (node?.id != null) nodeById.set(node.id, node);

  const candidateIds = new Set();
  addCandidateIds(candidateIds, mapview?.main_results, nodeById);
  addCandidateIds(candidateIds, mapview?.main_result_tags, nodeById);
  addCandidateIds(candidateIds, mapview?.skip_plan?.main_result_tags, nodeById);

  const mainResultIds = new Set();
  const labelSet = new Set();
  for (const node of allNodes) {
    if (node?.id != null && isMainResultNode(node, candidateIds)) mainResultIds.add(node.id);
    const claimLabel = extractAuthorsClaimLabel(node);
    if (claimLabel) labelSet.add(claimLabel);
  }

  const availableAuthorsLabels = AUTHORS_CLAIM_OPTIONS.map((o) => o.id).filter((id) => labelSet.has(id));
  const hasAuthorsClaims = availableAuthorsLabels.length > 0;
  const hasMainResults = mainResultIds.size > 0;

  const skipPlan = mapview?.skip_plan || null;
  const skipPlanEmpty = skipPlan?.empty === true;
  const dimmedSectionKeys = collectDimmedSectionKeys(skipPlan);
  const nonStructural = allNodes.filter((n) => !STRUCTURAL_KIND_SET.has(String(n?.kind || '').trim().toLowerCase()));
  const anyNodeHasDim = nonStructural.some(
    (n) => Boolean(n?.dim) || nodeMatchesDimmedSection(n, nodeById, dimmedSectionKeys),
  );
  const active = hasMainResults && (skipPlan ? !skipPlanEmpty : anyNodeHasDim);

  const rawBasisText = String(skipPlan?.basis_text || '');
  const basis = skipPlan?.basis === 'includes_model_inferred' || rawBasisText.toLowerCase().includes('model-inferred')
    ? 'includes_model_inferred'
    : 'explicit';
  const basisText = basis === 'includes_model_inferred'
    ? (rawBasisText.includes('includes model-inferred links') && isSafeText(rawBasisText)
      ? rawBasisText.trim()
      : WHOLEMAP_LABELS.skipBasisModel)
    : WHOLEMAP_LABELS.skipBasisExplicit;

  const hasGrouped = nonStructural.some((n) => typeof n?.group === 'string' && n.group.trim().length > 0);
  const sectionMap = new Map();
  let dimmedNodeCount = 0;
  for (const node of nonStructural) {
    const rawGroup = typeof node?.group === 'string' ? node.group.trim() : '';
    if (!rawGroup && !hasGrouped) continue;
    const key = rawGroup || '__ungrouped__';
    const label = rawGroup || 'Other';
    if (!sectionMap.has(key)) sectionMap.set(key, { key, label, count: 0, dimmedCount: 0, allDimmed: false });
    const sec = sectionMap.get(key);
    sec.count += 1;
    const nodeDim = active && (Boolean(node?.dim) || nodeMatchesDimmedSection(node, nodeById, dimmedSectionKeys));
    if (nodeDim) {
      sec.dimmedCount += 1;
      dimmedNodeCount += 1;
    }
  }
  const sections = [...sectionMap.values()].map((s) => ({
    ...s,
    allDimmed: active && s.count > 0 && s.dimmedCount === s.count,
  }));
  const dimmedSectionCount = active ? sections.filter((s) => s.allDimmed).length : 0;

  let summaryLine = WHOLEMAP_LABELS.skipEmptyNote;
  if (active) {
    summaryLine = isSafeText(skipPlan?.summary_line)
      ? skipPlan.summary_line.trim()
      : `${dimmedNodeCount} item${dimmedNodeCount === 1 ? '' : 's'} dimmed ${basisText}`;
  }

  return {
    mainResultIds,
    hasMainResults,
    hasAuthorsClaims,
    availableAuthorsLabels,
    sections,
    skipPlanEmpty,
    dimmedSectionKeys,
    nodeById,
    skipPlanSummary: { hasMainResults, active, basis, basisText, dimmedSectionCount, dimmedNodeCount, summaryLine },
  };
}

// Rule 6.14: never compute skip plans in JS; only guarantee zero nodes are dimmed without main results.
export function enforceSkipPlanHonesty(nodes = [], capabilities = null, mapview = null) {
  const list = Array.isArray(nodes) ? nodes : [];
  const caps = capabilities || (mapview ? inspectMapviewCapabilities(mapview) : null);
  const skipEmpty = !caps?.hasMainResults || caps?.skipPlanEmpty === true || mapview?.skip_plan?.empty === true;
  if (skipEmpty) {
    return list.map((node) => {
      const next = { ...node, dim: false };
      if (Array.isArray(node?.flags)) next.flags = node.flags.filter((f) => !UNREF_FLAG_RE.test(String(f || '')));
      return next;
    });
  }
  const dimmedKeys = caps?.dimmedSectionKeys instanceof Set
    ? caps.dimmedSectionKeys
    : collectDimmedSectionKeys(mapview?.skip_plan);
  const nodeById = caps?.nodeById instanceof Map
    ? caps.nodeById
    : new Map((Array.isArray(mapview?.nodes) ? mapview.nodes : list).filter((n) => n?.id != null).map((n) => [n.id, n]));
  return list.map((node) => ({
    ...node,
    dim: Boolean(node?.dim) || nodeMatchesDimmedSection(node, nodeById, dimmedKeys),
  }));
}

export function extractVisibleNodes(mapview) {
  const nodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const nonStructural = nodes.filter((n) => !STRUCTURAL_KIND_SET.has(String(n?.kind || '').trim().toLowerCase()));
  return nonStructural.length > 0 ? nonStructural : nodes.slice();
}

export function extractVisibleEdges(mapview, visibleIdSet) {
  const idSet = visibleIdSet instanceof Set ? visibleIdSet : new Set(visibleIdSet || []);
  const out = [];
  for (const raw of Array.isArray(mapview?.edges) ? mapview.edges : []) {
    const edge = normalizeEdge(raw);
    if (!edge.src || !edge.dst || edge.src === edge.dst) continue;
    if (edge.kind === 'contains' || edge.kind === 'cites') continue;
    if (!idSet.has(edge.src) || !idSet.has(edge.dst)) continue;
    out.push(edge);
  }
  return out;
}

function buildNeighborEntry(node, edge) {
  return {
    id: node.id,
    label: node.label || node.id,
    kind: node.kind || '',
    group: node.group ?? null,
    dim: Boolean(node.dim),
    basis: edge.basis,
    role: edge.role,
    rule: edge.rule ?? null,
    heuristic: edge.heuristic,
    dashed: isEdgeDashed(edge),
    strokeWidth: edgeStrokeWidth(edge),
    edge,
  };
}

function dedupeNeighborList(entries) {
  const byId = new Map();
  for (const item of entries) {
    const prev = byId.get(item.id);
    if (!prev || (prev.dashed && !item.dashed) || (prev.dashed === item.dashed && item.strokeWidth > prev.strokeWidth)) {
      byId.set(item.id, item);
    }
  }
  return [...byId.values()];
}

export function collectFocusedNeighborhood(mapview, focusId, capabilities = null) {
  const caps = capabilities || inspectMapviewCapabilities(mapview);
  const allNodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const rawById = new Map(allNodes.filter((n) => n?.id != null).map((n) => [n.id, n]));
  const visibleNodes = enforceSkipPlanHonesty(extractVisibleNodes(mapview), caps, mapview);
  const visibleById = new Map(visibleNodes.filter((n) => n?.id != null).map((n) => [n.id, n]));

  let resolvedId = typeof focusId === 'object' && focusId !== null ? focusId.id : focusId;
  const rawFocus = resolvedId != null ? rawById.get(resolvedId) : null;
  if (rawFocus && String(rawFocus.kind || '').trim().toLowerCase() === 'proof' && rawFocus.proof_of) {
    resolvedId = rawFocus.proof_of;
  }

  const focusNode = resolvedId != null ? (visibleById.get(resolvedId) || null) : null;
  if (!focusNode) {
    return { focusNode: null, statementUses: [], proofUses: [], usedBy: [], otherIncoming: [], neighborIds: new Set() };
  }

  const edges = extractVisibleEdges(mapview, new Set(visibleById.keys()));
  const rawStmt = [];
  const rawProof = [];
  const rawUsedBy = [];
  const rawOther = [];
  for (const edge of edges) {
    if (edge.kind !== 'uses' && edge.kind !== 'proves') continue;
    if (edge.dst === focusNode.id) {
      const target = visibleById.get(edge.src);
      if (!target) continue;
      const entry = buildNeighborEntry(target, edge);
      if (edge.role === 'statement') rawStmt.push(entry);
      else if (edge.role === 'proof' || edge.role === 'argument' || edge.kind === 'proves') rawProof.push(entry);
      else rawOther.push(entry);
    } else if (edge.src === focusNode.id) {
      const target = visibleById.get(edge.dst);
      if (target) rawUsedBy.push(buildNeighborEntry(target, edge));
    }
  }

  const statementUses = dedupeNeighborList(rawStmt);
  const proofUses = dedupeNeighborList(rawProof);
  const usedBy = dedupeNeighborList(rawUsedBy);
  const otherIncoming = dedupeNeighborList(rawOther);
  const neighborIds = new Set([
    ...statementUses.map((x) => x.id),
    ...proofUses.map((x) => x.id),
    ...usedBy.map((x) => x.id),
    ...otherIncoming.map((x) => x.id),
  ]);
  return { focusNode, statementUses, proofUses, usedBy, otherIncoming, neighborIds };
}

export function applyV7NodeFilters(nodes = [], filterState = {}, capabilities = null) {
  let list = Array.isArray(nodes) ? nodes : [];
  if (filterState?.mainResultsOnly && capabilities?.hasMainResults) {
    list = list.filter((n) => capabilities.mainResultIds.has(n?.id));
  }
  const rawLabels = filterState?.authorsLabels;
  const labelSet = rawLabels instanceof Set ? rawLabels : (Array.isArray(rawLabels) ? new Set(rawLabels) : null);
  if (labelSet && labelSet.size > 0 && capabilities?.hasAuthorsClaims) {
    list = list.filter((n) => labelSet.has(extractAuthorsClaimLabel(n)));
  }
  return list;
}

function buildSafeLegend(mapview, { requireBothBases = false } = {}) {
  const cleaned = [];
  for (const entry of Array.isArray(mapview?.legend) ? mapview.legend : []) {
    if (typeof entry === 'string') {
      if (isSafeText(entry)) cleaned.push(entry);
    } else if (entry && typeof entry === 'object') {
      const label = entry.label ?? entry.text ?? entry.kind ?? '';
      if (!FORBIDDEN_WORD_RE.test(String(label))) cleaned.push({ ...entry });
    }
  }
  const solidDefault = { label: 'By code or matched quote', icon: 'code', tone: 'ok', dashed: false };
  const dashedDefault = { label: 'Model-inferred or heuristic (not checked by code)', icon: 'model', tone: 'muted', dashed: true };
  if (cleaned.length === 0) return [solidDefault, dashedDefault];
  if (requireBothBases) {
    if (!cleaned.some((e) => typeof e === 'object' && e.dashed === false)) cleaned.unshift(solidDefault);
    if (!cleaned.some((e) => typeof e === 'object' && e.dashed === true)) cleaned.push(dashedDefault);
  }
  return cleaned;
}

export function projectPaperScaleResultsGraph(mapview, filterState = {}, capabilities = null) {
  const caps = capabilities || inspectMapviewCapabilities(mapview);
  const visible = enforceSkipPlanHonesty(extractVisibleNodes(mapview), caps, mapview);
  const totalVisibleCount = visible.length;

  let baseNodes;
  if (caps.hasMainResults) {
    baseNodes = visible.filter((n) => caps.mainResultIds.has(n.id));
  } else {
    const candidates = visible.filter((n) => RESULT_KIND_SET.has(String(n?.kind || '').trim().toLowerCase()));
    if (candidates.length > 0) baseNodes = candidates;
    else {
      const fallbacks = visible.filter((n) => FALLBACK_KIND_SET.has(String(n?.kind || '').trim().toLowerCase()));
      baseNodes = fallbacks.length > 0 ? fallbacks : visible;
    }
  }

  const nodes = applyV7NodeFilters(baseNodes, filterState, caps);
  const keptIds = new Set(nodes.map((n) => n.id));
  return {
    nodes,
    edges: extractVisibleEdges(mapview, keptIds),
    legend: buildSafeLegend(mapview),
    meta: { mode: VIEW_MODES.RESULTS, fallbackToCandidates: !caps.hasMainResults, totalVisibleCount },
  };
}

export function projectFocusedGraph(mapview, focusId, filterState = {}, capabilities = null) {
  const caps = capabilities || inspectMapviewCapabilities(mapview);
  const neighborhood = collectFocusedNeighborhood(mapview, focusId, caps);
  const legend = buildSafeLegend(mapview);
  if (!neighborhood.focusNode) {
    return { nodes: [], edges: [], legend, neighborhood, meta: { mode: VIEW_MODES.FOCUSED, focusId: null } };
  }

  const fid = neighborhood.focusNode.id;
  const keptIds = new Set([fid, ...neighborhood.neighborIds]);
  const visible = enforceSkipPlanHonesty(extractVisibleNodes(mapview), caps, mapview);
  const nodes = visible
    .filter((n) => keptIds.has(n.id))
    .map((n) => (n.parent && !keptIds.has(n.parent) ? { ...n, parent: null } : { ...n }));
  const edges = extractVisibleEdges(mapview, keptIds).filter((e) => e.src === fid || e.dst === fid);
  return { nodes, edges, legend, neighborhood, meta: { mode: VIEW_MODES.FOCUSED, focusId: fid } };
}

export function projectWholePaperGraph(mapview, filterState = {}, capabilities = null, options = {}) {
  const caps = capabilities || inspectMapviewCapabilities(mapview);
  const foldThreshold = Number.isFinite(options?.foldThreshold) ? options.foldThreshold : DEFAULT_FOLD_THRESHOLD;
  const visible = enforceSkipPlanHonesty(extractVisibleNodes(mapview), caps, mapview);
  const filtered = applyV7NodeFilters(visible, filterState, caps);

  // Strip parent on grouped nodes so F6 foldGraphView folds strictly one box per section (group before parent).
  const nodes = filtered.map((n) => (
    typeof n?.group === 'string' && n.group.trim().length > 0
      ? { ...n, originalParent: n.parent ?? null, parent: null }
      : { ...n }
  ));
  const keptIds = new Set(nodes.map((n) => n.id));
  return {
    nodes,
    edges: extractVisibleEdges(mapview, keptIds),
    legend: buildSafeLegend(mapview, { requireBothBases: true }),
    meta: {
      mode: VIEW_MODES.WHOLE,
      nodeCount: nodes.length,
      willAutoFold: nodes.length > foldThreshold,
      dashArray: EDGE_DASH_ARRAY,
      edgeWeights: EDGE_WEIGHTS,
    },
  };
}

export function pushBreadcrumb(trail = [], id, maxLen = 24) {
  const base = Array.isArray(trail) ? trail.filter((item) => item != null && String(item).trim() !== '') : [];
  const cleanId = id == null ? '' : String(id).trim();
  if (!cleanId || (base.length > 0 && base[base.length - 1] === cleanId)) return base;
  const next = [...base, cleanId];
  const limit = Number.isFinite(maxLen) && maxLen > 0 ? Math.floor(maxLen) : 24;
  return next.length > limit ? next.slice(next.length - limit) : next;
}

export function truncateBreadcrumb(trail = [], index) {
  if (!Array.isArray(trail) || trail.length === 0) return [];
  const idx = Number(index);
  if (!Number.isFinite(idx)) return trail.slice();
  return trail.slice(0, Math.max(0, Math.min(trail.length, Math.floor(idx) + 1)));
}

export function pickDefaultFocusNode(mapview, capabilities = null) {
  const caps = capabilities || inspectMapviewCapabilities(mapview);
  const visible = enforceSkipPlanHonesty(extractVisibleNodes(mapview), caps, mapview);
  if (visible.length === 0) return null;
  if (caps.hasMainResults) {
    const main = visible.find((n) => caps.mainResultIds.has(n.id));
    if (main) return main;
  }
  return visible.find((n) => RESULT_KIND_SET.has(String(n?.kind || '').trim().toLowerCase())) || visible[0] || null;
}
