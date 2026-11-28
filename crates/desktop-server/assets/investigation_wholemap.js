// Thin UI adapter for Investigate focused & whole-paper dependency maps (Package V13).
// Delegates layout, pan/zoom, SVG rendering, and >150-node folding to F6 graph_view.js.

import { DEFAULT_FOLD_THRESHOLD } from './graph_layout.js';
import { createGraphView } from './graph_view.js';
import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';
import {
  AUTHORS_CLAIM_OPTIONS, VIEW_MODES, WHOLEMAP_LABELS,
  collectFocusedNeighborhood, extractVisibleNodes, inspectMapviewCapabilities,
  pickDefaultFocusNode, projectFocusedGraph, projectPaperScaleResultsGraph,
  projectWholePaperGraph, pushBreadcrumb, truncateBreadcrumb,
} from './investigation_wholemap_model.js';

export { VIEW_MODES, AUTHORS_CLAIM_OPTIONS, WHOLEMAP_LABELS };

const VALID_MODES = new Set(Object.values(VIEW_MODES));
const F6_FILTER_KEYS = ['showStatementUses', 'showProofUses', 'showModelEdges', 'showModelNodes', 'hideDimmed', 'search'];

function isEditableTarget(target) {
  if (!target) return false;
  const tag = String(target.tagName || '').toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable) return true;
  const ce = typeof target.getAttribute === 'function' ? target.getAttribute('contenteditable') : null;
  return ce === '' || ce === 'true' || ce === 'plaintext-only';
}

function isHiddenInTree(el) {
  let cur = el;
  const seen = new Set();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    if (cur.hidden) return true;
    if (typeof cur.getAttribute === 'function' && cur.getAttribute('aria-hidden') === 'true') return true;
    cur = cur.parentNode || cur.parentElement || null;
  }
  return false;
}

function makeEl(tag, className = '', attrs = {}, text = null) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  for (const [k, v] of Object.entries(attrs)) {
    if (v !== undefined && v !== null) el.setAttribute(k, String(v));
  }
  if (text !== null) el.textContent = text;
  return el;
}

const normalizeLabelSet = (raw) => (raw instanceof Set || Array.isArray(raw) ? new Set(raw) : new Set());

export function createInvestigateWholeMap(host, mapviewOrOptions, maybeOptions) {
  const isOptsFirst = mapviewOrOptions && typeof mapviewOrOptions === 'object'
    && !Array.isArray(mapviewOrOptions.nodes) && maybeOptions === undefined;
  const opts = isOptsFirst ? mapviewOrOptions : (maybeOptions || {});
  let mapview = (isOptsFirst ? opts.mapview : mapviewOrOptions) || { nodes: [], edges: [], legend: [] };
  let capabilities = inspectMapviewCapabilities(mapview);
  const foldThreshold = Number.isFinite(opts.foldThreshold) ? opts.foldThreshold : DEFAULT_FOLD_THRESHOLD;

  let mode = VALID_MODES.has(opts.initialMode) ? opts.initialMode : VIEW_MODES.RESULTS;
  let previousMode = mode === VIEW_MODES.WHOLE ? VIEW_MODES.RESULTS : mode;
  let selectedId = opts.selectedId ?? null;
  let focusId = mode === VIEW_MODES.FOCUSED
    ? (selectedId ?? pickDefaultFocusNode(mapview, capabilities)?.id ?? null)
    : selectedId;
  let trail = mode === VIEW_MODES.FOCUSED && focusId ? pushBreadcrumb([], focusId) : [];

  let graphFilter = {
    showStatementUses: true, showProofUses: true, showModelEdges: true,
    showModelNodes: true, hideDimmed: false, search: '',
  };
  for (const key of F6_FILTER_KEYS) {
    if (opts.initialFilter && Object.prototype.hasOwnProperty.call(opts.initialFilter, key)) {
      graphFilter[key] = opts.initialFilter[key];
    }
  }
  const v7Filter = {
    mainResultsOnly: Boolean(opts.initialFilter?.mainResultsOnly),
    authorsLabels: normalizeLabelSet(opts.initialFilter?.authorsLabels),
  };

  const wholeCollapsedGroups = new Set(opts.collapsedGroups || []);
  const wholeExpandedGroups = new Set(opts.expandedGroups || []);
  const hookedControls = new WeakSet();
  let graphView = null;
  let lastProjected = { nodes: [], edges: [], legend: [], meta: {} };

  const root = makeEl('section', 'inv-wm-root', { 'aria-label': 'Paper dependency map' });
  const header = makeEl('header', 'inv-wm-header');
  const modeTabs = makeEl('div', 'inv-wm-mode-tabs', { role: 'group', 'aria-label': 'Map view modes' });
  const resultsBtn = makeEl('button', 'inv-wm-mode-btn', { type: 'button', 'data-wm-mode': VIEW_MODES.RESULTS });
  const focusedBtn = makeEl('button', 'inv-wm-mode-btn', { type: 'button', 'data-wm-mode': VIEW_MODES.FOCUSED }, WHOLEMAP_LABELS.modeFocused);
  const wholeBtn = makeEl('button', 'inv-wm-mode-btn', { type: 'button', 'data-wm-mode': VIEW_MODES.WHOLE }, WHOLEMAP_LABELS.modeWhole);
  modeTabs.append(resultsBtn, focusedBtn, wholeBtn);

  const statsEl = makeEl('span', 'inv-wm-stats');
  header.append(modeTabs, statsEl);

  const skipBar = makeEl('div', 'inv-wm-skip-bar', { role: 'status' });
  const v7Bar = makeEl('div', 'inv-wm-v7-bar', { role: 'toolbar', 'aria-label': 'Result and author filters' });
  const sectionsBar = makeEl('div', 'inv-wm-sections-bar', { role: 'group', 'aria-label': WHOLEMAP_LABELS.sectionsHeading });
  const focusPanel = makeEl('div', 'inv-wm-focus-panel');
  const graphHost = makeEl('div', 'inv-wm-graph-host');
  root.append(header, skipBar, v7Bar, sectionsBar, focusPanel, graphHost);
  if (host) host.replaceChildren(root);

  const lookupVisibleNode = (id) => (id == null ? null : extractVisibleNodes(mapview).find((n) => n?.id === id) || null);
  const resolveTargetId = (tag) => {
    const rawId = typeof tag === 'object' && tag !== null ? tag.id : (tag ?? null);
    if (rawId == null) return null;
    const raw = capabilities.nodeById?.get(rawId);
    if (raw && String(raw.kind || '').trim().toLowerCase() === 'proof' && raw.proof_of) return raw.proof_of;
    return rawId;
  };

  function syncGraphToolbarHooks() {
    const refoldBtn = graphHost.querySelector('[data-fold-action="refold"]');
    if (refoldBtn && !hookedControls.has(refoldBtn)) {
      hookedControls.add(refoldBtn);
      refoldBtn.addEventListener('click', () => {
        wholeExpandedGroups.clear();
        renderSectionsBar();
        renderHeader();
      });
    }
    const syncFromF6 = () => {
      if (graphView) graphFilter = graphView.getFilter();
      renderHeader();
      renderSectionsBar();
      syncGraphToolbarHooks();
    };
    for (const btn of graphHost.querySelectorAll('.gv-toggle')) {
      if (!hookedControls.has(btn)) {
        hookedControls.add(btn);
        btn.addEventListener('click', syncFromF6);
      }
    }
    const searchEl = graphHost.querySelector('.gv-search');
    if (searchEl && !hookedControls.has(searchEl)) {
      hookedControls.add(searchEl);
      searchEl.addEventListener('input', syncFromF6);
    }
  }

  function renderHeader() {
    root.setAttribute('data-mode', mode);
    root.setAttribute('data-has-main-results', String(Boolean(capabilities.hasMainResults)));
    root.setAttribute('data-has-authors-claims', String(Boolean(capabilities.hasAuthorsClaims)));
    resultsBtn.textContent = capabilities.hasMainResults ? WHOLEMAP_LABELS.modeResults : WHOLEMAP_LABELS.modeResultsFallback;
    resultsBtn.setAttribute('aria-pressed', String(mode === VIEW_MODES.RESULTS));
    focusedBtn.setAttribute('aria-pressed', String(mode === VIEW_MODES.FOCUSED));
    wholeBtn.setAttribute('aria-pressed', String(mode === VIEW_MODES.WHOLE));

    const visibleCount = graphView?.getLayout?.()?.visibleNodeCount ?? lastProjected.nodes.length;
    const sectionCount = capabilities.sections.length;
    statsEl.textContent = `${visibleCount} ${visibleCount === 1 ? 'node' : 'nodes'} · ${sectionCount} ${sectionCount === 1 ? 'section' : 'sections'}`;
  }

  function renderSkipBar() {
    const summary = capabilities.skipPlanSummary;
    skipBar.setAttribute('data-basis', summary.basis);
    skipBar.setAttribute('data-active', String(Boolean(summary.active)));
    skipBar.replaceChildren();
    if (summary.active) {
      skipBar.hidden = false;
      const isModel = summary.basis === 'includes_model_inferred';
      const badge = createBadge({
        text: summary.basisText, icon: isModel ? 'model' : 'code',
        tone: isModel ? 'warn' : 'ok', border: isModel ? BORDER_DASHED : BORDER_SOLID,
      });
      badge.classList.add('inv-wm-skip-basis');
      skipBar.append(badge, makeEl('span', 'inv-wm-skip-summary', {}, summary.summaryLine));
      return;
    }
    if (mode === VIEW_MODES.WHOLE) {
      skipBar.hidden = false;
      const badge = createBadge({ text: 'No dimming', icon: 'info', tone: 'neutral', border: BORDER_SOLID });
      badge.classList.add('inv-wm-skip-basis');
      skipBar.append(badge, makeEl('span', 'inv-wm-skip-summary', {}, WHOLEMAP_LABELS.skipEmptyNote));
      return;
    }
    skipBar.hidden = true;
  }

  function renderV7Bar() {
    v7Bar.replaceChildren();
    if (!capabilities.hasMainResults && !capabilities.hasAuthorsClaims) {
      v7Bar.hidden = true;
      return;
    }
    v7Bar.hidden = false;
    if (capabilities.hasMainResults) {
      const pressed = mode === VIEW_MODES.RESULTS || Boolean(v7Filter.mainResultsOnly);
      const mainBtn = makeEl('button', 'inv-wm-chip', {
        type: 'button', 'data-wm-filter': 'main-results-only', 'aria-pressed': String(pressed),
      }, WHOLEMAP_LABELS.mainResultsOnly);
      mainBtn.addEventListener('click', () => {
        if (mode === VIEW_MODES.RESULTS) {
          v7Filter.mainResultsOnly = false;
          setMode(VIEW_MODES.WHOLE);
        } else if (mode === VIEW_MODES.FOCUSED) {
          v7Filter.mainResultsOnly = true;
          setMode(VIEW_MODES.RESULTS);
        } else {
          v7Filter.mainResultsOnly = !v7Filter.mainResultsOnly;
          renderAll({ recreateGraph: true });
        }
      });
      v7Bar.appendChild(mainBtn);
    }
    if (capabilities.hasAuthorsClaims) {
      const group = makeEl('div', 'inv-wm-filter-group', { role: 'group', 'aria-label': WHOLEMAP_LABELS.authorsLabelHeading });
      group.appendChild(makeEl('span', 'inv-wm-filter-heading', {}, WHOLEMAP_LABELS.authorsLabelHeading));
      for (const opt of AUTHORS_CLAIM_OPTIONS) {
        const chip = makeEl('button', 'inv-wm-chip', {
          type: 'button', 'data-wm-authors-label': opt.id, 'data-model': 'true',
          'aria-pressed': String(v7Filter.authorsLabels.has(opt.id)),
        }, opt.label);
        chip.addEventListener('click', () => {
          if (v7Filter.authorsLabels.has(opt.id)) v7Filter.authorsLabels.delete(opt.id);
          else v7Filter.authorsLabels.add(opt.id);
          if (v7Filter.authorsLabels.size > 0 && mode === VIEW_MODES.FOCUSED) setMode(VIEW_MODES.WHOLE);
          else renderAll({ recreateGraph: true });
        });
        group.appendChild(chip);
      }
      v7Bar.appendChild(group);
    }
  }

  function renderSectionsBar() {
    sectionsBar.replaceChildren();
    if (mode !== VIEW_MODES.WHOLE || capabilities.sections.length === 0) {
      sectionsBar.hidden = true;
      return;
    }
    const foldedKeys = new Set((graphView?.getLayout?.()?.foldedGroups || []).map((g) => g.key));
    const overThreshold = lastProjected.nodes.length > foldThreshold;
    const collapsibleSections = capabilities.sections.filter(
      (sec) => sec.key !== '__ungrouped__' || overThreshold || foldedKeys.has(sec.key),
    );
    if (collapsibleSections.length === 0) {
      sectionsBar.hidden = true;
      return;
    }
    sectionsBar.hidden = false;
    sectionsBar.appendChild(makeEl('span', 'inv-wm-filter-heading', {}, WHOLEMAP_LABELS.sectionsHeading));
    for (const sec of collapsibleSections) {
      const isCollapsed = foldedKeys.has(sec.key);
      const pill = makeEl('button', 'inv-wm-section-pill', {
        type: 'button', 'data-wm-section': sec.key, 'data-collapsed': String(isCollapsed),
        'aria-pressed': String(isCollapsed), ...(sec.allDimmed ? { 'data-dim': 'true' } : {}),
      }, `${sec.label} (${sec.count})`);
      pill.addEventListener('click', () => (isCollapsed ? expandSection(sec.key) : collapseSection(sec.key)));
      sectionsBar.appendChild(pill);
    }
    const collapseAllBtn = makeEl('button', 'inv-wm-chip', { type: 'button', 'data-wm-sections-action': 'collapse-all' }, WHOLEMAP_LABELS.collapseAll);
    const expandAllBtn = makeEl('button', 'inv-wm-chip', { type: 'button', 'data-wm-sections-action': 'expand-all' }, WHOLEMAP_LABELS.expandAll);
    collapseAllBtn.addEventListener('click', collapseAllSections);
    expandAllBtn.addEventListener('click', expandAllSections);
    sectionsBar.append(collapseAllBtn, expandAllBtn);
  }

  function buildRelationColumn(relAttr, headingText, entries) {
    const col = makeEl('section', 'inv-wm-rel-col', { 'data-wm-rel': relAttr });
    const list = makeEl('ul', 'inv-wm-rel-list');
    if (!entries.length) {
      list.appendChild(makeEl('li', 'inv-wm-rel-empty', {}, WHOLEMAP_LABELS.emptyNeighbours));
    } else {
      for (const item of entries) {
        const classes = ['inv-wm-rel-btn'];
        if (item.role === 'statement') classes.push('inv-wm-rel-btn--statement');
        else if (item.role === 'proof' || item.role === 'argument') classes.push('inv-wm-rel-btn--proof');
        if (item.dashed) classes.push('inv-wm-rel-btn--dashed');
        const suffix = item.heuristic && !/\(heuristic\)\s*$/i.test(item.label)
          ? ` (${WHOLEMAP_LABELS.heuristicTag})`
          : (item.dashed && !item.heuristic && !/\(model\)\s*$/i.test(item.label) ? ' (model)' : '');
        const btn = makeEl('button', classes.join(' '), {
          type: 'button', 'data-node-id': item.id, 'data-role': item.role || '',
          'data-basis': item.basis || '', 'data-dashed': String(Boolean(item.dashed)),
          'data-stroke-width': String(item.strokeWidth), ...(item.heuristic ? { 'data-heuristic': 'true' } : {}),
        }, `${item.label}${suffix}`);
        btn.addEventListener('click', () => focusNode(item.id, { notify: true, recordTrail: true }));
        const li = document.createElement('li');
        li.appendChild(btn);
        list.appendChild(li);
      }
    }
    col.append(makeEl('h3', 'inv-wm-rel-heading', {}, headingText), list);
    return col;
  }

  function renderFocusPanel(neighborhood) {
    focusPanel.replaceChildren();
    if (mode !== VIEW_MODES.FOCUSED) {
      focusPanel.hidden = true;
      return;
    }
    focusPanel.hidden = false;
    const nav = makeEl('nav', 'inv-wm-breadcrumb', { 'aria-label': WHOLEMAP_LABELS.breadcrumbLabel });
    if (trail.length > 1) {
      const backBtn = makeEl('button', 'inv-wm-crumb-btn', { type: 'button', 'data-wm-action': 'back' }, WHOLEMAP_LABELS.backButton);
      backBtn.addEventListener('click', stepBack);
      nav.appendChild(backBtn);
    }
    trail.forEach((id, idx) => {
      if (idx > 0) nav.appendChild(makeEl('span', 'inv-wm-crumb-sep', { 'aria-hidden': 'true' }, '›'));
      const crumb = makeEl('button', 'inv-wm-crumb-btn', {
        type: 'button', 'data-wm-crumb-index': String(idx), 'data-node-id': id,
        ...(idx === trail.length - 1 ? { 'aria-current': 'step' } : {}),
      }, lookupVisibleNode(id)?.label || id);
      crumb.addEventListener('click', () => goToBreadcrumb(idx));
      nav.appendChild(crumb);
    });
    const wholeActionBtn = makeEl('button', 'inv-wm-crumb-btn', { type: 'button', 'data-wm-action': 'whole-map' }, WHOLEMAP_LABELS.modeWhole);
    wholeActionBtn.addEventListener('click', () => setMode(VIEW_MODES.WHOLE));
    nav.appendChild(wholeActionBtn);
    focusPanel.appendChild(nav);
    if (!neighborhood?.focusNode) {
      focusPanel.appendChild(makeEl('p', 'inv-wm-empty-focus', {}, WHOLEMAP_LABELS.emptyFocus));
    }
    const otherIncoming = neighborhood?.otherIncoming || [];
    const grid = makeEl('div', 'inv-wm-focus-grid', otherIncoming.length > 0 ? { 'data-cols': '4' } : {});
    grid.append(
      buildRelationColumn('statement-uses', WHOLEMAP_LABELS.statementUses, neighborhood?.statementUses || []),
      buildRelationColumn('proof-uses', WHOLEMAP_LABELS.proofUses, neighborhood?.proofUses || []),
      buildRelationColumn('used-by', WHOLEMAP_LABELS.usedBy, neighborhood?.usedBy || []),
    );
    if (otherIncoming.length > 0) {
      grid.appendChild(buildRelationColumn('other-uses', WHOLEMAP_LABELS.otherUses, otherIncoming));
    }
    focusPanel.appendChild(grid);
  }

  function handleGraphNodeSelect(node) {
    if (!node || node.id == null) return;
    if (mode === VIEW_MODES.RESULTS || mode === VIEW_MODES.FOCUSED) {
      focusNode(node.id, { notify: true, recordTrail: true });
      return;
    }
    selectedId = node.id;
    opts.onSelectNode?.(node.id, node);
  }

  function mountGraphView(projected) {
    if (graphView) {
      graphFilter = graphView.getFilter();
      graphView.destroy();
      graphView = null;
    }
    const isAutoFold = mode === VIEW_MODES.WHOLE && projected.nodes.length > foldThreshold;
    if (!isAutoFold && wholeExpandedGroups.size > 0) wholeExpandedGroups.clear();
    const gvOptions = {
      selectedId: mode === VIEW_MODES.FOCUSED ? focusId : selectedId,
      foldThreshold,
      initialFilter: { ...graphFilter },
      onSelectNode: handleGraphNodeSelect,
      onSelectEdge: (edge) => opts.onSelectEdge?.(edge),
      onExpandGroup: (key) => {
        wholeCollapsedGroups.delete(key);
        if (lastProjected.nodes.length > foldThreshold) {
          wholeExpandedGroups.add(key);
          renderSectionsBar();
          renderHeader();
          syncGraphToolbarHooks();
        } else {
          wholeExpandedGroups.delete(key);
          renderAll({ recreateGraph: true });
        }
      },
      ...(mode === VIEW_MODES.WHOLE
        ? { expandedGroups: isAutoFold ? [...wholeExpandedGroups] : [], collapsedGroups: [...wholeCollapsedGroups] }
        : {}),
    };
    graphView = createGraphView(graphHost, projected, gvOptions);
    syncGraphToolbarHooks();
  }

  function renderAll({ recreateGraph = true } = {}) {
    let projected;
    if (mode === VIEW_MODES.FOCUSED) {
      if (!focusId) {
        const def = pickDefaultFocusNode(mapview, capabilities);
        if (def) { focusId = def.id; selectedId = def.id; trail = pushBreadcrumb(trail, focusId); }
      }
      projected = projectFocusedGraph(mapview, focusId, v7Filter, capabilities);
    } else if (mode === VIEW_MODES.WHOLE) {
      projected = projectWholePaperGraph(mapview, v7Filter, capabilities, { foldThreshold });
    } else {
      projected = projectPaperScaleResultsGraph(mapview, v7Filter, capabilities);
    }
    lastProjected = projected;
    if (recreateGraph || !graphView) mountGraphView(projected);
    else { graphView.setData(projected); syncGraphToolbarHooks(); }
    renderHeader();
    renderSkipBar();
    renderV7Bar();
    renderSectionsBar();
    renderFocusPanel(mode === VIEW_MODES.FOCUSED ? projected.neighborhood : null);
  }

  function setMode(nextMode) {
    if (!VALID_MODES.has(nextMode) || nextMode === mode) return;
    const prevMode = mode;
    if (nextMode === VIEW_MODES.WHOLE) previousMode = prevMode;
    mode = nextMode;
    if (mode === VIEW_MODES.FOCUSED && !focusId) {
      const def = pickDefaultFocusNode(mapview, capabilities);
      if (def) { focusId = def.id; selectedId = def.id; }
    }
    if (mode === VIEW_MODES.FOCUSED && focusId && trail.length === 0) trail = pushBreadcrumb([], focusId);
    renderAll({ recreateGraph: true });
    opts.onModeChange?.(mode, prevMode);
  }

  function toggleWholeMap() {
    setMode(mode === VIEW_MODES.WHOLE ? (previousMode || VIEW_MODES.RESULTS) : VIEW_MODES.WHOLE);
  }

  function focusNode(tag, { notify = false, recordTrail = true } = {}) {
    const resolved = collectFocusedNeighborhood(mapview, tag, capabilities).focusNode;
    const nextId = resolved ? resolved.id : resolveTargetId(tag);
    if (nextId == null) return;
    const prevMode = mode;
    focusId = nextId;
    selectedId = nextId;
    if (recordTrail) trail = pushBreadcrumb(trail, nextId);
    mode = VIEW_MODES.FOCUSED;
    renderAll({ recreateGraph: true });
    if (prevMode !== VIEW_MODES.FOCUSED) opts.onModeChange?.(mode, prevMode);
    if (notify) opts.onSelectNode?.(nextId, resolved || lookupVisibleNode(nextId));
  }

  function selectNode(tag, { notify = false } = {}) {
    const nextId = resolveTargetId(tag);
    if (mode === VIEW_MODES.FOCUSED && nextId != null) { focusNode(nextId, { notify, recordTrail: true }); return; }
    selectedId = nextId;
    graphView?.selectNode(selectedId);
    if (notify && selectedId != null) opts.onSelectNode?.(selectedId, lookupVisibleNode(selectedId));
  }

  function goToBreadcrumb(index) {
    if (!trail.length) return;
    trail = truncateBreadcrumb(trail, index);
    const targetId = trail[trail.length - 1] ?? null;
    if (targetId == null) return;
    const prevMode = mode;
    mode = VIEW_MODES.FOCUSED;
    focusId = targetId;
    selectedId = targetId;
    renderAll({ recreateGraph: true });
    if (prevMode !== VIEW_MODES.FOCUSED) opts.onModeChange?.(mode, prevMode);
    opts.onSelectNode?.(targetId, lookupVisibleNode(targetId));
  }

  const stepBack = () => { if (trail.length > 1) goToBreadcrumb(trail.length - 2); };

  function collapseSection(groupKey) {
    if (!groupKey) return;
    wholeCollapsedGroups.add(groupKey);
    wholeExpandedGroups.delete(groupKey);
    renderAll({ recreateGraph: true });
  }

  function expandSection(groupKey) {
    if (!groupKey) return;
    wholeCollapsedGroups.delete(groupKey);
    const wholeCount = projectWholePaperGraph(mapview, v7Filter, capabilities, { foldThreshold }).nodes.length;
    if (wholeCount > foldThreshold) wholeExpandedGroups.add(groupKey);
    else wholeExpandedGroups.delete(groupKey);
    renderAll({ recreateGraph: true });
  }

  function collapseAllSections() {
    wholeExpandedGroups.clear();
    const wholeCount = projectWholePaperGraph(mapview, v7Filter, capabilities, { foldThreshold }).nodes.length;
    for (const sec of capabilities.sections) {
      if (sec.key !== '__ungrouped__' || wholeCount > foldThreshold) wholeCollapsedGroups.add(sec.key);
    }
    renderAll({ recreateGraph: true });
  }

  function expandAllSections() {
    wholeCollapsedGroups.clear();
    wholeExpandedGroups.clear();
    const wholeCount = projectWholePaperGraph(mapview, v7Filter, capabilities, { foldThreshold }).nodes.length;
    if (wholeCount > foldThreshold) {
      for (const sec of capabilities.sections) wholeExpandedGroups.add(sec.key);
      wholeExpandedGroups.add('__ungrouped__');
    }
    renderAll({ recreateGraph: true });
  }

  function setFilter(partial = {}) {
    const f6Patch = {};
    let hasF6 = false;
    let hasV7 = false;
    for (const key of F6_FILTER_KEYS) {
      if (Object.prototype.hasOwnProperty.call(partial, key)) {
        graphFilter[key] = partial[key]; f6Patch[key] = partial[key]; hasF6 = true;
      }
    }
    if (Object.prototype.hasOwnProperty.call(partial, 'mainResultsOnly')) { v7Filter.mainResultsOnly = Boolean(partial.mainResultsOnly); hasV7 = true; }
    if (Object.prototype.hasOwnProperty.call(partial, 'authorsLabels')) { v7Filter.authorsLabels = normalizeLabelSet(partial.authorsLabels); hasV7 = true; }
    if (hasV7) { renderAll({ recreateGraph: true }); return; }
    if (hasF6 && graphView) { graphView.setFilter(f6Patch); graphFilter = graphView.getFilter(); renderHeader(); renderSectionsBar(); }
  }

  function getFilter() {
    if (graphView) graphFilter = graphView.getFilter();
    return { ...graphFilter, mainResultsOnly: v7Filter.mainResultsOnly, authorsLabels: [...v7Filter.authorsLabels] };
  }

  function setMapview(nextMapview, { preserveViewState = false } = {}) {
    mapview = nextMapview || { nodes: [], edges: [], legend: [] };
    capabilities = inspectMapviewCapabilities(mapview);
    if (!preserveViewState) {
      wholeCollapsedGroups.clear();
      wholeExpandedGroups.clear();
      v7Filter.mainResultsOnly = false;
      v7Filter.authorsLabels.clear();
    }
    trail = trail.filter((id) => lookupVisibleNode(id) != null);
    if (focusId && !lookupVisibleNode(focusId)) {
      focusId = mode === VIEW_MODES.FOCUSED ? (pickDefaultFocusNode(mapview, capabilities)?.id ?? null) : null;
      trail = focusId ? [focusId] : [];
    }
    if (selectedId && !lookupVisibleNode(selectedId)) selectedId = focusId;
    renderAll({ recreateGraph: true });
  }

  resultsBtn.addEventListener('click', () => setMode(VIEW_MODES.RESULTS));
  focusedBtn.addEventListener('click', () => setMode(VIEW_MODES.FOCUSED));
  wholeBtn.addEventListener('click', () => setMode(VIEW_MODES.WHOLE));

  const keyboardTarget = opts.keyboardTarget !== undefined ? opts.keyboardTarget : (typeof window !== 'undefined' ? window : null);
  const onKeyDown = (ev) => {
    if (!ev || ev.defaultPrevented || ev.repeat || ev.ctrlKey || ev.metaKey || ev.altKey
      || isHiddenInTree(root) || isEditableTarget(ev.target)) return;
    if (ev.key === 'm' || ev.key === 'M') { ev.preventDefault?.(); toggleWholeMap(); }
  };
  keyboardTarget?.addEventListener?.('keydown', onKeyDown);

  renderAll({ recreateGraph: true });

  return {
    get element() { return root; },
    getMode() { return mode; },
    setMode, toggleWholeMap, focusNode, selectNode, stepBack, goToBreadcrumb,
    getBreadcrumb() { return [...trail]; },
    setFilter, getFilter,
    collapseSection, expandSection, collapseAllSections, expandAllSections,
    setMapview, setData: setMapview,
    getGraphView() { return graphView; },
    getState() {
      const folded = new Set((graphView?.getLayout?.()?.foldedGroups || []).map((g) => g.key));
      return { mode, selectedId, focusId, trail: [...trail], capabilities, collapsedSections: [...folded], filter: getFilter() };
    },
    destroy() {
      keyboardTarget?.removeEventListener?.('keydown', onKeyDown);
      if (graphView) { graphView.destroy(); graphView = null; }
      if (host) host.replaceChildren();
    },
  };
}

export const createInvestigationWholeMap = createInvestigateWholeMap;
