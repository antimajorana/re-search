// Explore Map UI adapter over F6 graph_view.js, ui_badge.js, and quote_chip.js (package X12).
// Adapter ONLY: zero graph layout code, zero raw innerHTML writes, zero Rule S8 forbidden words.
// Governance: explore.md 3, 4.1, 4.2, 6 (V5, V6, S4, S8), 13 (decisions 6, 13); 00-overview.md 2.6, 2.10, 3.2.
import { createGraphView } from './graph_view.js';
import {
  CONFIRM_SAME_LABEL,
  NOT_IN_CURRENT_MAP_LABEL,
  REJECT_SAME_LABEL,
  SUMMARY_MODEL_LABEL,
  VERDICT_STATUS_CHIPS,
  WHAT_DIFFERS_LABEL,
  buildEvidenceIndex,
  indexNotes,
  openExplorePageImage,
  openExploreSource,
  projectVerdictRow,
  renderVerdictTable,
  resolveVerdictChip,
  toQuoteChipSpec,
} from './idea_verdict_ui.js';
import { createQuoteChip } from './quote_chip.js';
import {
  ABSTRACT_ONLY_WORD,
  BORDER_DASHED,
  BORDER_SOLID,
  MODEL_TEXT_SUFFIX,
  OCR_TAG_WORD,
  USER_TAG_WORD,
  createBadge,
  createNodeCardShell,
} from './ui_badge.js';

export {
  CONFIRM_SAME_LABEL,
  NOT_IN_CURRENT_MAP_LABEL,
  REJECT_SAME_LABEL,
  SUMMARY_MODEL_LABEL,
  VERDICT_STATUS_CHIPS,
  WHAT_DIFFERS_LABEL,
  openExplorePageImage,
  openExploreSource,
  projectVerdictRow,
  renderVerdictTable,
  resolveVerdictChip,
};

export const NODE_KIND_GROUPS = Object.freeze([
  Object.freeze({ kind: 'known_result', prefix: 'k-', label: 'Known results', icon: 'quote', tone: 'ok' }),
  Object.freeze({ kind: 'definition', prefix: 'd-', label: 'Definitions', icon: 'code', tone: 'info' }),
  Object.freeze({ kind: 'concept', prefix: 'c-', label: 'Concepts', icon: 'info', tone: 'neutral' }),
  Object.freeze({ kind: 'open_question', prefix: 'q-', label: 'Open questions', icon: 'warn', tone: 'warn' }),
  Object.freeze({ kind: 'related_work', prefix: 'w-', label: 'Related work', icon: 'link', tone: 'muted' }),
]);

export const NODE_FLAG_ACTIONS = Object.freeze([
  Object.freeze({ action: 'wrong', label: 'Wrong', icon: 'error', tone: 'err' }),
  Object.freeze({ action: 'not_relevant', label: 'Not relevant', icon: 'pause', tone: 'muted' }),
  Object.freeze({ action: 'pin', label: 'Pin', icon: 'pin', tone: 'info' }),
]);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function mapDisplayGradeToF6(node) {
  const dg = String(node?.display_grade || '').toLowerCase();
  if (dg === 'matched_quote_scanned_page' || node?.ocr_tag) return 'quote_ocr';
  if (dg === 'matched_quote_full_text') return 'quote_fulltext';
  if (dg === 'matched_quote_abstract') return 'quote_abstract';
  if (dg === 'user_statement') return 'user';
  return 'model';
}

function gradeBadgeSpec(grade) {
  if (grade === 'quote_ocr') return { text: `Found (${OCR_TAG_WORD})`, icon: 'ocr', tone: 'warn', border: BORDER_SOLID };
  if (grade === 'quote_fulltext') return { text: 'Found', icon: 'quote', tone: 'ok', border: BORDER_SOLID };
  if (grade === 'quote_abstract') return { text: ABSTRACT_ONLY_WORD, icon: 'abstract', tone: 'muted', border: BORDER_DASHED };
  if (grade === 'user') return { text: USER_TAG_WORD, icon: 'user', tone: 'neutral', border: BORDER_SOLID };
  return { text: MODEL_TEXT_SUFFIX, icon: 'model', tone: 'muted', border: BORDER_DASHED };
}

function summarizeNodeFlags(nodeNotes) {
  const flags = [];
  const seen = new Set();
  for (const n of Array.isArray(nodeNotes) ? nodeNotes : []) {
    const a = String(n.action || '').trim();
    if (!a || seen.has(a)) continue;
    seen.add(a);
    flags.push(a === 'not_relevant' ? 'not relevant' : a);
  }
  return flags;
}

/** Adapt Explore's `view.map` into the shared F6 `GraphView` shape (`00-overview.md` 2.6). */
export function adaptExploreGraphView(mapView, notes = [], { stale = false } = {}) {
  const { byNode } = indexNotes(notes);
  const rawNodes = Array.isArray(mapView?.nodes) ? mapView.nodes : [];
  const rawEdges = Array.isArray(mapView?.edges) ? mapView.edges : [];
  const rawLegend = Array.isArray(mapView?.legend) ? mapView.legend : [];
  const nodeGradeById = new Map();

  const nodes = rawNodes.map((n) => {
    const id = String(n.id || '');
    const grade = mapDisplayGradeToF6(n);
    nodeGradeById.set(id, grade);
    const kindMeta = NODE_KIND_GROUPS.find((g) => g.kind === n.kind);
    const isMissing = String(n.status || '') === 'not_in_current_map';
    const badges = [gradeBadgeSpec(grade)];
    if (isMissing) {
      badges.push({ text: NOT_IN_CURRENT_MAP_LABEL, icon: 'warn', tone: 'muted', border: BORDER_DASHED });
    }
    return {
      id,
      kind: String(n.kind || 'concept'),
      label: String(n.label || id),
      group: kindMeta?.label || String(n.kind || 'concept'),
      grade,
      badges,
      flags: summarizeNodeFlags(byNode.get(id)),
      stale: Boolean(stale),
      dim: isMissing,
    };
  });

  const edges = rawEdges.map((e) => {
    const src = String(e.src || e.from || '');
    const dst = String(e.dst || e.to || '');
    const basis = String(e.basis || 'model');
    const endpointAbstract =
      nodeGradeById.get(src) === 'quote_abstract' || nodeGradeById.get(dst) === 'quote_abstract';
    return {
      src,
      dst,
      kind: String(e.kind || ''),
      role: String(e.role || e.kind || ''),
      basis,
      symmetric: Boolean(e.symmetric),
      abstract_only: (e.style === 'dashed' && basis !== 'model') || endpointAbstract,
    };
  });

  const legend = rawLegend.length > 0
    ? rawLegend.map((entry) => {
        const meta = NODE_KIND_GROUPS.find((g) => g.kind === entry.kind);
        const prefix = entry.prefix ? `${entry.prefix} ` : '';
        return {
          label: `${prefix}${entry.label || entry.kind || ''}`,
          icon: meta?.icon || 'info',
          tone: meta?.tone || 'neutral',
          dashed: false,
        };
      })
    : NODE_KIND_GROUPS.map((g) => ({ label: `${g.prefix} ${g.label}`, icon: g.icon, tone: g.tone, dashed: false }));

  return { nodes, edges, legend };
}

function renderNodeDrawer(drawerEl, node, evidenceIndex, nodeNotes, callbacks, stale) {
  drawerEl.replaceChildren();
  if (!node) {
    drawerEl.append(el('p', 'idea-card-meta', 'Select a concept map node to inspect matched quotes and flags.'));
    return;
  }

  const grade = mapDisplayGradeToF6(node);
  const isMissing = String(node.status || '') === 'not_in_current_map';
  const badges = [gradeBadgeSpec(grade)];
  if (isMissing) {
    badges.push({ text: NOT_IN_CURRENT_MAP_LABEL, icon: 'warn', tone: 'muted', border: BORDER_DASHED });
  }

  // Block A (by code): strictly app/code-computed metadata, verbatim idea quote, and matched quote chips.
  const codeItems = [];
  const codeMeta = [String(node.kind || '')];
  if (Array.isArray(node.papers) && node.papers.length > 0) codeMeta.push(`papers: ${node.papers.join(', ')}`);
  if (node.year != null) codeMeta.push(`year: ${node.year}`);
  if (node.stated_year != null) codeMeta.push(`stated year: ${node.stated_year}`);
  if (node.later_work_checked != null) codeMeta.push(`later work checked: ${node.later_work_checked ? 'yes' : 'no'}`);
  codeItems.push(el('p', 'idea-card-meta', codeMeta.filter(Boolean).join(' · ')));

  if (node.idea_quote) {
    codeItems.push(el('blockquote', 'idea-candidate-abstract', `"${node.idea_quote}" (${USER_TAG_WORD})`));
  }
  for (const evId of Array.isArray(node.evidence) ? node.evidence : []) {
    const rec = evidenceIndex.get(String(evId)) || null;
    codeItems.push(createQuoteChip(toQuoteChipSpec(String(evId), rec), {
      onOpen: callbacks.onOpenSource,
      onOpenPageImage: callbacks.onOpenPageImage || callbacks.onOpenSource,
    }));
  }

  const primaryPaper = node.paper_key || (Array.isArray(node.papers) ? node.papers[0] : null);
  if (primaryPaper && typeof callbacks.onInvestigatePaper === 'function') {
    const invBtn = el('button', 'pill-btn sm', 'Investigate this paper');
    invBtn.type = 'button';
    invBtn.setAttribute('data-action', 'investigate-paper');
    invBtn.setAttribute('data-paper-key', String(primaryPaper));
    invBtn.addEventListener('click', () => {
      callbacks.onInvestigatePaper({ paper_key: String(primaryPaper), node_id: String(node.id || '') });
    });
    codeItems.push(invBtn);
  }

  // Block B (model): statement summary and all model-extracted node properties explicitly inside model block.
  const modelItems = [];
  if (node.statement) modelItems.push(el('p', 'ui-node-card-text', String(node.statement)));
  const modelProps = [];
  if (node.relation_to_idea) modelProps.push(`relation: ${node.relation_to_idea}`);
  if (node.symbol) modelProps.push(`symbol: ${node.symbol}`);
  if (node.conventions) modelProps.push(`conventions: ${node.conventions}`);
  if (node.regime) modelProps.push(`regime: ${node.regime}`);
  if (modelProps.length > 0) modelItems.push(el('p', 'idea-card-meta', `${modelProps.join(' · ')} ${MODEL_TEXT_SUFFIX}`));
  if (Array.isArray(node.aliases) && node.aliases.length > 0) {
    modelItems.push(el('p', 'idea-card-meta', `Aliases ${MODEL_TEXT_SUFFIX}: ${node.aliases.join(', ')}`));
  }
  if (Array.isArray(node.hypotheses) && node.hypotheses.length > 0) {
    modelItems.push(el('p', 'idea-card-meta', `Hypotheses ${MODEL_TEXT_SUFFIX}: ${node.hypotheses.join('; ')}`));
  }

  const shell = createNodeCardShell({
    title: String(node.label || node.id || ''),
    tag: String(node.id || ''),
    grade,
    badges,
    flags: summarizeNodeFlags(nodeNotes),
    stale: Boolean(stale),
    dim: isMissing,
    codeBlock: { label: 'Matched quotes (by code)', content: codeItems },
    modelBlock: { label: SUMMARY_MODEL_LABEL, content: modelItems.length > 0 ? modelItems : null },
  });
  shell.setAttribute('data-drawer-node-id', String(node.id || ''));

  const controls = el('div', 'idea-map-flag-controls');
  controls.setAttribute('data-node-flags', String(node.id || ''));
  const btnRow = el('div', 'idea-chip-row');
  for (const spec of NODE_FLAG_ACTIONS) {
    const btn = el('button', 'pill-btn sm', spec.label);
    btn.type = 'button';
    btn.setAttribute('data-flag-action', spec.action);
    btn.setAttribute('data-node-id', String(node.id || ''));
    btn.addEventListener('click', () => {
      callbacks.onNote?.({ target_kind: 'node', target_id: String(node.id || ''), action: spec.action });
    });
    btnRow.append(btn);
  }
  controls.append(btnRow);

  const commentRow = el('div', 'idea-controls-row');
  const commentInput = el('input', 'text-input');
  commentInput.type = 'text';
  commentInput.placeholder = 'Add a note on this node…';
  commentInput.setAttribute('aria-label', 'Note on selected node');
  commentInput.setAttribute('data-node-comment-input', String(node.id || ''));
  const commentBtn = el('button', 'pill-btn sm', 'Add comment');
  commentBtn.type = 'button';
  commentBtn.setAttribute('data-flag-action', 'comment');
  commentBtn.addEventListener('click', () => {
    const txt = String(commentInput.value || '').trim();
    if (!txt) return;
    callbacks.onNote?.({ target_kind: 'node', target_id: String(node.id || ''), action: 'comment', comment: txt });
    commentInput.value = '';
  });
  commentRow.append(commentInput, commentBtn);
  controls.append(commentRow);

  if (Array.isArray(nodeNotes) && nodeNotes.length > 0) {
    const list = el('ul', 'idea-not-searched');
    list.setAttribute('data-node-notes-list', String(node.id || ''));
    for (const n of nodeNotes) {
      const label = n.comment ? `${n.action}: ${n.comment}` : String(n.action || '');
      list.append(el('li', '', `${label} (${USER_TAG_WORD})`));
    }
    controls.append(list);
  }
  shell.append(controls);
  drawerEl.append(shell);
}

/**
 * Render the complete X12 Map & Verdict UI section:
 * - Decision 13 stale banner with "Re-map against revision n"
 * - Verdict table (F12 chips, quote_chip.js, "what differs (model)", Rule V6 confirm/reject)
 * - Synchronized grouped node list + F6 createGraphView + createNodeCardShell drawer with flags.
 */
export function renderIdeaMapSection(view, callbacks = {}) {
  const root = el('div', 'idea-map-section');
  root.setAttribute('data-idea-section', 'map-and-verdicts');

  const currentRev = Number(view?.statement?.rev || 1);
  const mapRev = Number(view?.map?.idea_rev || 0);
  const isStale = Boolean(view?.map_stale) || (mapRev > 0 && currentRev > mapRev);

  if (isStale) {
    const banner = el('div', 'idea-banner idea-banner--warn');
    banner.setAttribute('data-idea-banner', 'map-stale');
    banner.setAttribute('role', 'status');
    const left = el('div', 'idea-candidate-badges');
    const revNote = mapRev > 0
      ? `Map was built for revision ${mapRev}; idea is at revision ${currentRev}. Re-mapping reuses matched quotes at zero reader cost.`
      : `Map is behind idea revision ${currentRev}. Re-mapping reuses matched quotes at zero reader cost.`;
    left.append(
      createBadge({ text: 'Map stale', icon: 'warn', tone: 'warn', border: BORDER_SOLID }),
      el('span', '', revNote),
    );
    banner.append(left);
    const remapBtn = el('button', 'pill-btn sm primary', `Re-map against revision ${currentRev}`);
    remapBtn.type = 'button';
    remapBtn.setAttribute('data-action', 'remap-stale');
    remapBtn.addEventListener('click', () => {
      callbacks.onRemap?.({ idea_rev: currentRev, reuse_evidence: true });
    });
    banner.append(remapBtn);
    root.append(banner);
  }

  root.append(renderVerdictTable(view, callbacks));

  const mapPanel = el('section', 'idea-where-panel idea-map-panel');
  mapPanel.setAttribute('data-idea-section', 'concept-map');
  const mapHead = el('div', 'idea-card-head');
  const rawNodes = Array.isArray(view?.map?.nodes) ? view.map.nodes : [];
  mapHead.append(el('h3', 'idea-card-title', 'Concept map'), el('span', 'idea-card-meta', `${rawNodes.length} node(s)`));
  mapPanel.append(mapHead);

  if (rawNodes.length === 0) {
    mapPanel.append(el(
      'p',
      'idea-card-meta',
      'No concept map yet. Sharpening, reading, and mapping build the map once a model is connected.',
    ));
    root.append(mapPanel);
    return root;
  }

  const evidenceIndex = buildEvidenceIndex(view);
  const { byNode } = indexNotes(view?.notes);
  const nodeById = new Map(rawNodes.map((n) => [String(n.id || ''), n]));
  const adapted = adaptExploreGraphView(view.map, view.notes, { stale: isStale });

  const layout = el('div', 'idea-map-layout');
  const listCol = el('div', 'idea-map-list');
  listCol.setAttribute('data-idea-map-list', 'true');
  const graphCol = el('div', 'idea-map-graph-host');
  graphCol.setAttribute('data-idea-map-graph', 'true');
  const drawerCol = el('aside', 'idea-map-drawer');
  drawerCol.setAttribute('data-idea-map-drawer', 'true');

  const rowById = new Map();
  let selectedId = callbacks.initialSelectedNodeId && nodeById.has(callbacks.initialSelectedNodeId)
    ? callbacks.initialSelectedNodeId
    : String(rawNodes[0].id || '');
  let graphView = null;

  function syncSelection(nodeId, source) {
    if (!nodeId || !nodeById.has(nodeId)) return;
    selectedId = nodeId;
    for (const [id, btn] of rowById.entries()) {
      const active = id === selectedId;
      btn.setAttribute('aria-selected', String(active));
      btn.classList?.toggle('is-selected', active);
    }
    if (source !== 'graph' && graphView) graphView.selectNode(selectedId);
    renderNodeDrawer(drawerCol, nodeById.get(selectedId), evidenceIndex, byNode.get(selectedId) || [], callbacks, isStale);
    callbacks.onSelectNode?.(nodeById.get(selectedId));
  }

  const appendGroup = (title, groupNodes, attrKind) => {
    if (groupNodes.length === 0) return;
    const grp = el('div', 'idea-map-group');
    grp.setAttribute('data-map-group', attrKind);
    grp.append(el('h4', 'idea-field-label', `${title} (${groupNodes.length})`));
    for (const n of groupNodes) {
      const id = String(n.id || '');
      const btn = el('button', 'idea-map-node-row');
      btn.type = 'button';
      btn.setAttribute('data-map-list-node', id);
      btn.setAttribute('data-node-status', String(n.status || 'active'));
      btn.setAttribute('aria-selected', String(id === selectedId));
      if (id === selectedId) btn.classList?.add('is-selected');
      const grade = mapDisplayGradeToF6(n);
      const badgeSpec = n.status === 'not_in_current_map'
        ? { text: NOT_IN_CURRENT_MAP_LABEL, icon: 'warn', tone: 'muted', border: BORDER_DASHED }
        : gradeBadgeSpec(grade);
      btn.append(el('code', 'idea-map-node-id', id), el('span', 'idea-map-node-label', String(n.label || id)), createBadge(badgeSpec));
      for (const fl of summarizeNodeFlags(byNode.get(id))) btn.append(el('span', 'idea-chip', fl));
      btn.addEventListener('click', () => syncSelection(id, 'list'));
      rowById.set(id, btn);
      grp.append(btn);
    }
    listCol.append(grp);
  };

  const activeNodes = rawNodes.filter((n) => String(n.status || '') !== 'not_in_current_map');
  const carriedNodes = rawNodes.filter((n) => String(n.status || '') === 'not_in_current_map');
  for (const g of NODE_KIND_GROUPS) {
    appendGroup(g.label, activeNodes.filter((n) => n.kind === g.kind), g.kind);
  }
  appendGroup('Not in current map (noted)', carriedNodes, 'not_in_current_map');

  graphView = createGraphView(graphCol, adapted, {
    ariaLabel: 'Explore concept map',
    selectedId,
    onSelectNode: (gNode) => syncSelection(String(gNode?.id || ''), 'graph'),
  });
  renderNodeDrawer(drawerCol, nodeById.get(selectedId), evidenceIndex, byNode.get(selectedId) || [], callbacks, isStale);

  layout.append(listCol, graphCol, drawerCol);
  mapPanel.append(layout);
  root.append(mapPanel);
  return root;
}
