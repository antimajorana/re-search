// Solve S13b derivation workspace: topological step cards (R5 handle-only formulas),
// Result sheet (R15), F6 run_bar.js + graph_view.js ("closure of the final answer"),
// Why popover hook, Expressions/Sources/Log tabs, and registerSolveTab extension registry.
// Zero `innerHTML` writes; every untrusted string uses `textContent` only.

import { createGraphView } from './graph_view.js';
import { createQuoteChip } from './quote_chip.js';
import { createRunBar } from './run_bar.js';
import {
  OWN_CHECK_EQUATIONS_VERIFIED_LINE,
  SOLVE_TIER_LABELS, adaptSolveGraphView, createSolveBadgeGroup,
  createSolveLegend, findFinalAnswerEntryId,
} from './solve_badges.js';
import { buildHandleLookup, createSolveExprsView, renderClaimFormula } from './solve_exprs.js';
import {
  DONE_LABEL, DONE_OPEN_ITEMS_LABEL, NOT_DONE_LABEL,
  computeFinalClosureIds, describeCriterion, renderSolveResultSheet, resolveDoneHeading,
} from './solve_result_sheet.js';
import { createSolveWhyPopover } from './solve_why.js';
import { mountWriteupButton } from './writeup_btn.js';

export {
  DONE_LABEL, DONE_OPEN_ITEMS_LABEL, NOT_DONE_LABEL,
  computeFinalClosureIds, describeCriterion, renderSolveResultSheet, resolveDoneHeading,
};
export const CLOSURE_FILTER_LABEL = 'Closure of the final answer';

export const SOLVE_BUILTIN_TABS = Object.freeze([
  Object.freeze({ id: 'graph', label: 'Graph' }),
  Object.freeze({ id: 'expressions', label: 'Expressions' }),
  Object.freeze({ id: 'sources', label: 'Sources' }),
  Object.freeze({ id: 'log', label: 'Log' }),
]);

const sharedExtensionTabs = new Map();

/**
 * Register a Solve canvas tab (`{ id, label, mount }`) for extensions such as S18a Artifacts
 * or S17b Transcribe. Also supports the 2-argument form `registerSolveTab(registry, tab)`.
 */
export function registerSolveTab(tabOrRegistry, maybeTab) {
  if (maybeTab !== undefined) {
    const registry = tabOrRegistry;
    const tab = maybeTab;
    if (!registry || !tab || !tab.id) return tab;
    if (typeof registry === 'function') { registry(tab); return tab; }
    if (typeof registry.registerTab === 'function') { registry.registerTab(tab); return tab; }
    if (typeof Map !== 'undefined' && registry instanceof Map) { registry.set(tab.id, tab); return tab; }
    if (Array.isArray(registry)) {
      const idx = registry.findIndex((item) => item && item.id === tab.id);
      if (idx >= 0) registry[idx] = tab; else registry.push(tab);
      return tab;
    }
    registry[tab.id] = tab;
    return tab;
  }
  const tab = tabOrRegistry;
  if (!tab || !tab.id) return null;
  const entry = {
    id: String(tab.id),
    label: String(tab.label || tab.id),
    mount: typeof tab.mount === 'function' ? tab.mount : null,
  };
  sharedExtensionTabs.set(entry.id, entry);
  return entry;
}

export function unregisterSolveTab(id) {
  return sharedExtensionTabs.delete(String(id || ''));
}

export function getRegisteredSolveTabs() {
  return Array.from(sharedExtensionTabs.values());
}

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function buildStepQuoteChip(step, onOpen) {
  const sr = step.source_ref;
  const badge = step.badge || {};
  const isOcr = sr.ocr !== undefined
    ? Boolean(sr.ocr)
    : (badge.provenance_tag === 'OCR' || (Array.isArray(badge.flags) && badge.flags.includes('OCR source')));
  return createQuoteChip({
    source_id: sr.source_id, path: sr.path, start_line: sr.start_line, end_line: sr.end_line,
    page: sr.page, quote: sr.quote ?? sr.excerpt ?? '', ocr: isOcr,
    basis: sr.basis || (isOcr ? 'ocr' : undefined), abstract_only: sr.abstract_only,
    evidence_id: sr.evidence_id || (sr.evidence_seq != null ? `xe-${sr.evidence_seq}` : undefined),
    evidence_seq: sr.evidence_seq, paper: sr.paper || sr.source_id || sr.path || '',
  }, { onOpen, onOpenPageImage: onOpen });
}

function mapRunStateToBarProps(runState, view = {}) {
  const rs = runState && typeof runState === 'object' ? runState : {};
  const rawState = String(rs.state || 'paused');
  const setupCriteria = Array.isArray(view?.setup?.done) ? view.setup.done : [];
  const doneCriteria = Array.isArray(view?.done?.criteria) ? view.done.criteria : [];
  const openItems = Array.isArray(rs.open_items)
    ? rs.open_items.map((item) => {
        if (typeof item === 'string') return item;
        if (item?.kind === 'unmet_criterion') {
          const match = doneCriteria.find((c) => c && String(c.id || '') === String(item.id || ''))
            || setupCriteria.find((c) => c && String(c.id || '') === String(item.id || ''));
          return `Open criterion: ${match ? describeCriterion(match, setupCriteria) : String(item.id || 'criterion')}`;
        }
        if (item?.kind === 'unacknowledged_assumption') {
          return item.outside_final_closure
            ? `Unacknowledged assumption outside final closure: ${item.entry_id}`
            : `Unacknowledged assumption: ${item.entry_id}`;
        }
        return String(item?.message || item?.id || item?.entry_id || 'Open item');
      })
    : [];
  return {
    state: rawState === 'ready' ? 'paused' : rawState,
    reason: rawState === 'ready' ? 'ready' : (rs.reason || ''),
    detail: rs.detail || '',
    elapsedMs: rs.elapsed_ms,
    elapsedSec: rs.elapsed_sec,
    modelCallsUsed: rs.model_calls_used,
    modelCallsMax: rs.model_calls_max,
    openItems,
  };
}

function renderStepCard(doc, step, handleMap, receiptsById, handlesByCallId, checksByStep, opts) {
  const card = el(doc, 'article', 'solve-step-card');
  if (opts.selectedStepId === step.entry_id) card.classList.add('is-selected');
  if (step.superseded_by) card.classList.add('is-superseded');
  if (opts.outsideFinal) {
    card.classList.add('is-outside-final');
    card.setAttribute('data-outside-final', 'true');
  }
  card.setAttribute('data-testid', 'solve-step-card');
  card.setAttribute('data-entry-id', String(step.entry_id || ''));
  card.setAttribute('data-entry-type', String(step.entry_type || ''));

  const head = el(doc, 'div', 'solve-step-head');
  const idBtn = el(doc, 'button', 'solve-step-id-btn', String(step.entry_id || ''));
  idBtn.type = 'button';
  idBtn.addEventListener('click', () => opts.onSelectStep?.(String(step.entry_id)));
  head.append(idBtn, el(doc, 'span', 'solve-step-type', String(step.entry_type || 'step')));
  if (step.superseded_by) head.append(el(doc, 'span', 'solve-flag-chip', `superseded by ${step.superseded_by}`));
  if (opts.outsideFinal) head.append(el(doc, 'span', 'solve-flag-chip', 'not used by the final answer'));

  const rawBadge = step.badge || {};
  const hasProseDetails = String(step.details || '').trim().length > 0;
  const displayBadge = rawBadge.own_check_line && hasProseDetails
    && rawBadge.own === 'verified' && rawBadge.effective !== 'verified' && !rawBadge.stale
    ? { ...rawBadge, own_check_line: OWN_CHECK_EQUATIONS_VERIFIED_LINE }
    : rawBadge;
  head.append(createSolveBadgeGroup(displayBadge, {
    document: doc, onBadgeClick: () => opts.onOpenWhy?.(step), onSelectStep: opts.onSelectStep,
  }));
  card.append(head);

  const summaryText = String(step.summary || '').trim();
  if (summaryText) card.append(el(doc, 'p', 'solve-step-summary', summaryText));

  // Rule R5: render claims STRICTLY from solve.expr.latex via math_katex.js; ignore step.latex.
  const claims = Array.isArray(step.claims) ? step.claims : [];
  if (claims.length > 0) {
    const eqWrap = el(doc, 'div', 'solve-step-equations');
    eqWrap.setAttribute('data-testid', 'solve-step-equations');
    for (const claim of claims) {
      const eqRow = el(doc, 'div', 'solve-claim-formula');
      renderClaimFormula(eqRow, claim, handleMap, opts.katex, { display: true });
      eqWrap.append(eqRow);
    }
    card.append(eqWrap);
  }

  const deps = [];
  for (const list of [step.depends_on, step.derived_depends_on]) {
    for (const d of Array.isArray(list) ? list : []) {
      const ds = String(d || '');
      if (ds && !deps.includes(ds)) deps.push(ds);
    }
  }
  if (deps.length > 0) {
    const depBar = el(doc, 'div', 'solve-step-deps');
    depBar.append(el(doc, 'span', 'solve-muted', 'Depends on:'));
    for (const depId of deps) {
      const chip = el(doc, 'button', 'solve-dep-chip', depId);
      chip.type = 'button';
      chip.setAttribute('data-dep-id', depId);
      chip.addEventListener('click', () => opts.onSelectStep?.(depId));
      depBar.append(chip);
    }
    card.append(depBar);
  }

  if (step.source_ref) card.append(buildStepQuoteChip(step, opts.onOpenSource));

  const stepChecks = checksByStep.get(String(step.entry_id)) || [];
  const receiptRefs = Array.isArray(step.receipt_refs) ? step.receipt_refs : [];
  if (step.details || receiptRefs.length > 0 || stepChecks.length > 0) {
    const det = el(doc, 'details', 'solve-step-caret');
    det.append(el(doc, 'summary', 'solve-step-caret-summary',
      `Details · ${stepChecks.length} check${stepChecks.length === 1 ? '' : 's'} · ${receiptRefs.length} receipt${receiptRefs.length === 1 ? '' : 's'}`));
    if (step.details) det.append(el(doc, 'p', 'solve-step-details', String(step.details)));
    for (const rr of receiptRefs) {
      const cid = String(rr.call_id || '');
      const rec = receiptsById.get(cid);
      const line = rec
        ? `Receipt ${rec.call_id} (${rec.tool}, ${rec.ok ? 'ok' : 'error'}${rec.wall_ms != null ? `, ${rec.wall_ms} ms` : ''})`
        : `Receipt ${cid} (missing)`;
      const recCode = typeof rec?.code === 'string' && rec.code.trim() ? rec.code : '';
      const callHandles = (handlesByCallId.get(cid) || []).filter((h) => typeof h?.source_expr === 'string' && h.source_expr.trim());
      if (recCode || callHandles.length > 0) {
        const recDet = el(doc, 'details', 'solve-receipt-details');
        recDet.append(el(doc, 'summary', 'solve-receipt-line', line));
        if (recCode) {
          const pre = el(doc, 'pre', 'solve-receipt-code', recCode);
          pre.setAttribute('data-testid', 'solve-receipt-code');
          recDet.append(pre);
        }
        for (const h of callHandles) {
          const srcRow = el(doc, 'div', 'solve-handle-source', `${h.handle || h.name}: ${h.source_expr}`);
          srcRow.setAttribute('data-testid', 'solve-handle-source');
          recDet.append(srcRow);
        }
        det.append(recDet);
      } else {
        det.append(el(doc, 'div', 'solve-receipt-line', line));
      }
    }
    card.append(det);
  }
  return card;
}

export function createSolveWorkspace(host, initialView = {}, opts = {}) {
  if (!host) throw new Error('createSolveWorkspace requires a host element');
  const doc = opts.document || document;
  let view = initialView && typeof initialView === 'object' ? initialView : {};
  let selectedStepId = opts.selectedStepId || null;
  let whyEntryId = null;
  let activeTabId = opts.initialTab || 'graph';
  let renderedTabId = null;
  let closureOnly = Boolean(opts.closureOfFinal);
  let renderedClosureOnly = closureOnly;
  let closureCheckbox = null;
  let showSuperseded = false;
  let runBarApi = null, graphViewApi = null, exprsViewApi = null, whyPopoverApi = null, activeExtApi = null, activeExtDestroy = null, writeupCtrl = null;

  const localTabs = new Map();
  for (const t of Array.isArray(opts.extraTabs) ? opts.extraTabs : []) {
    if (t && t.id) localTabs.set(String(t.id), t);
  }

  const root = el(doc, 'section', 'solve-workspace');
  root.setAttribute('data-testid', 'solve-workspace');
  const runBarHost = el(doc, 'div', 'solve-run-bar-host');
  const writeupHost = el(doc, 'div', 'solve-writeup-host');
  const grid = el(doc, 'div', 'solve-workspace-grid');
  const mainCol = el(doc, 'div', 'solve-derivation-col');
  const sideCol = el(doc, 'aside', 'solve-canvas-col');
  const whyHost = el(doc, 'div', 'solve-why-host');
  whyHost.hidden = true;
  grid.append(mainCol, sideCol);
  root.append(runBarHost, writeupHost, grid, whyHost);
  writeupCtrl = mountWriteupButton(writeupHost, { document: doc, sessionId: opts.sessionId || '', token: opts.token, getToken: opts.getToken, fetchFn: opts.fetch });
  host.replaceChildren(root);

  function allTabs() {
    const merged = [...SOLVE_BUILTIN_TABS];
    const seen = new Set(merged.map((t) => t.id));
    for (const src of [sharedExtensionTabs.values(), localTabs.values()]) {
      for (const t of src) if (t?.id && !seen.has(t.id)) { seen.add(t.id); merged.push(t); }
    }
    return merged;
  }

  function destroySideWidgets() {
    if (graphViewApi) { graphViewApi.destroy(); graphViewApi = null; }
    if (exprsViewApi) { exprsViewApi.destroy(); exprsViewApi = null; }
    if (typeof activeExtDestroy === 'function') { activeExtDestroy(); activeExtDestroy = null; }
    activeExtApi = null;
    renderedTabId = null;
    closureCheckbox = null;
  }

  function closeWhy() {
    whyEntryId = null;
    if (whyPopoverApi) { whyPopoverApi.destroy(); whyPopoverApi = null; }
    whyHost.hidden = true;
    whyHost.replaceChildren();
  }

  function openWhy(stepOrId) {
    const id = typeof stepOrId === 'string' ? stepOrId : stepOrId?.entry_id;
    const step = (view.steps || []).find((s) => String(s?.entry_id) === String(id));
    if (!step) return;
    whyEntryId = String(step.entry_id);
    whyHost.hidden = false;
    const whyProps = {
      document: doc, step, view, sessionId: opts.sessionId || '',
      onClose: closeWhy, onSelectStep: selectStep, onFork: opts.onFork,
      onAttachSource: opts.onAttachSource, onAck: opts.onAck, onAddCheck: opts.onAddCheck,
      onRerunChecks: opts.onRerunChecks, onStepCorrected: opts.onStepCorrected,
      onReviewModel: opts.onReviewModel, reviewEnabled: Boolean(opts.reviewEnabled),
      onOpenSource: opts.onOpenSource, onOpenPageImage: opts.onOpenPageImage,
    };
    if (whyPopoverApi) whyPopoverApi.update(whyProps);
    else whyPopoverApi = createSolveWhyPopover(whyHost, whyProps);
  }

  function selectStep(entryId) {
    selectedStepId = entryId ? String(entryId) : null;
    if (graphViewApi && selectedStepId) graphViewApi.selectNode(selectedStepId);
    renderMain();
    opts.onSelectStep?.(selectedStepId);
  }

  function renderRunBarSection() {
    const props = mapRunStateToBarProps(view.run_state, view);
    const cbs = { onStop: opts.onStop, onContinue: opts.onContinue, onRetry: opts.onRetry };
    if (!runBarApi) runBarApi = createRunBar(runBarHost, props, cbs);
    else runBarApi.update(props, cbs);
  }

  function renderMain() {
    mainCol.replaceChildren();
    const handleMap = buildHandleLookup(view.handles || []);
    const handlesByCallId = new Map();
    for (const h of Array.isArray(view.handles) ? view.handles : []) {
      const cid = String(h?.produced_by?.call_id || '');
      if (!cid) continue;
      if (!handlesByCallId.has(cid)) handlesByCallId.set(cid, []);
      handlesByCallId.get(cid).push(h);
    }
    const receiptsById = new Map();
    for (const r of Array.isArray(view.receipts) ? view.receipts : []) if (r?.call_id) receiptsById.set(String(r.call_id), r);
    const checksByStep = new Map();
    for (const c of Array.isArray(view.checks) ? view.checks : []) {
      const k = String(c?.entry_id || '');
      if (k) { if (!checksByStep.has(k)) checksByStep.set(k, []); checksByStep.get(k).push(c); }
    }

    const resultSheet = renderSolveResultSheet(doc, view, handleMap, {
      katex: opts.katex, onSelectStep: selectStep, onOpenWhy: openWhy, onAck: opts.onAck,
    });
    if (resultSheet) mainCol.append(resultSheet);

    const toolbar = el(doc, 'div', 'solve-steps-toolbar');
    toolbar.append(el(doc, 'h3', 'solve-section-title', 'Derivation'));
    const supLabel = el(doc, 'label', 'solve-check-row');
    const supBox = el(doc, 'input');
    supBox.type = 'checkbox';
    supBox.checked = showSuperseded;
    supBox.setAttribute('data-testid', 'solve-show-superseded');
    supBox.addEventListener('change', (ev) => { showSuperseded = Boolean(ev.target.checked); renderMain(); });
    supLabel.append(supBox, el(doc, 'span', '', 'Show superseded'));
    toolbar.append(supLabel);
    mainCol.append(toolbar, createSolveLegend(view.graph?.legend || [], view.steps || [], { document: doc }));

    const allSteps = Array.isArray(view.steps) ? view.steps : [];
    const finalEntryId = view?.done?.final_entry_id ? String(view.done.final_entry_id) : '';
    const hasFinal = Boolean(finalEntryId) && allSteps.some((s) => s && String(s.entry_id) === finalEntryId);
    const closureIds = hasFinal ? computeFinalClosureIds(allSteps, finalEntryId) : null;

    const visibleSteps = allSteps.filter((s) => s && (showSuperseded || !s.superseded_by));
    const mainSteps = [];
    const outsideActiveSteps = [];
    for (const s of visibleSteps) {
      if (closureIds && !s.superseded_by && !closureIds.has(String(s.entry_id))) {
        outsideActiveSteps.push(s);
      } else {
        mainSteps.push(s);
      }
    }

    const list = el(doc, 'div', 'solve-steps-list');
    list.setAttribute('data-testid', 'solve-steps-list');
    const cardOpts = {
      katex: opts.katex, selectedStepId, onSelectStep: selectStep, onOpenWhy: openWhy, onOpenSource: opts.onOpenSource,
    };
    if (mainSteps.length === 0 && outsideActiveSteps.length === 0) {
      list.append(el(doc, 'p', 'solve-muted', 'No derivation steps recorded yet.'));
    } else {
      for (const step of mainSteps) {
        list.append(renderStepCard(doc, step, handleMap, receiptsById, handlesByCallId, checksByStep, cardOpts));
      }
      if (outsideActiveSteps.length > 0) {
        const group = el(doc, 'details', 'solve-outside-final-group');
        group.setAttribute('data-testid', 'solve-outside-final-group');
        const n = outsideActiveSteps.length;
        group.append(el(doc, 'summary', 'solve-outside-final-summary',
          `Not used by the final answer (${n} ${n === 1 ? 'step' : 'steps'})`));
        for (const step of outsideActiveSteps) {
          group.append(renderStepCard(doc, step, handleMap, receiptsById, handlesByCallId, checksByStep, {
            ...cardOpts, outsideFinal: true,
          }));
        }
        list.append(group);
      }
    }
    mainCol.append(list);
  }

  function renderSideTabs(forceRebuild = false) {
    const tabs = allTabs();
    if (!tabs.some((t) => t.id === activeTabId)) activeTabId = 'graph';

    if (!forceRebuild && renderedTabId === activeTabId) {
      if (activeTabId === 'graph' && graphViewApi && renderedClosureOnly === closureOnly) {
        const finalId = findFinalAnswerEntryId(view);
        if (closureCheckbox) closureCheckbox.disabled = !finalId;
        graphViewApi.setData(adaptSolveGraphView(view, { closureOfFinal: closureOnly, finalEntryId: finalId }));
        if (selectedStepId) graphViewApi.selectNode(selectedStepId);
        return;
      }
      if (activeTabId === 'expressions' && exprsViewApi) { exprsViewApi.update(view.handles || []); return; }
      if (activeExtApi && typeof activeExtApi.update === 'function') { activeExtApi.update(view); return; }
    }

    destroySideWidgets();
    sideCol.replaceChildren();
    renderedTabId = activeTabId;
    renderedClosureOnly = closureOnly;

    const bar = el(doc, 'div', 'solve-tab-bar');
    bar.setAttribute('role', 'tablist');
    bar.setAttribute('data-testid', 'solve-tab-bar');
    for (const tab of tabs) {
      const btn = el(doc, 'button', tab.id === activeTabId ? 'seg-btn is-active' : 'seg-btn', tab.label);
      btn.type = 'button';
      btn.setAttribute('role', 'tab');
      btn.setAttribute('data-tab-id', tab.id);
      btn.setAttribute('aria-selected', String(tab.id === activeTabId));
      btn.addEventListener('click', () => { activeTabId = tab.id; renderSideTabs(true); });
      bar.append(btn);
    }
    const panel = el(doc, 'div', 'solve-tab-panel');
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('data-active-tab', activeTabId);
    sideCol.append(bar, panel);

    if (activeTabId === 'graph') {
      const controls = el(doc, 'div', 'solve-graph-controls');
      const finalId = findFinalAnswerEntryId(view);
      const toggleLabel = el(doc, 'label', 'solve-check-row');
      const box = el(doc, 'input');
      box.type = 'checkbox';
      box.checked = closureOnly;
      box.disabled = !finalId;
      box.setAttribute('data-testid', 'solve-closure-filter');
      box.addEventListener('change', (ev) => { closureOnly = Boolean(ev.target.checked); renderSideTabs(true); });
      closureCheckbox = box;
      toggleLabel.append(box, el(doc, 'span', '', CLOSURE_FILTER_LABEL));
      controls.append(toggleLabel);
      const gvHost = el(doc, 'div', 'solve-graph-host');
      panel.append(controls, gvHost);
      graphViewApi = createGraphView(gvHost, adaptSolveGraphView(view, { closureOfFinal: closureOnly, finalEntryId: finalId }), {
        selectedId: selectedStepId, ariaLabel: 'Solve dependency graph',
        onSelectNode: (node) => { if (node?.id) selectStep(String(node.id)); },
      });
      return;
    }
    if (activeTabId === 'expressions') {
      exprsViewApi = createSolveExprsView(panel, view.handles || [], { document: doc, katex: opts.katex, onSelectStep: selectStep });
      return;
    }
    if (activeTabId === 'sources') {
      const cited = (view.steps || []).filter((s) => s && !s.superseded_by && s.source_ref);
      if (cited.length === 0) { panel.append(el(doc, 'p', 'solve-muted', 'No cited sources attached yet.')); return; }
      for (const s of cited) panel.append(buildStepQuoteChip(s, opts.onOpenSource));
      return;
    }
    if (activeTabId === 'log') {
      const receipts = Array.isArray(view.receipts) ? view.receipts : [];
      if (receipts.length === 0) { panel.append(el(doc, 'p', 'solve-muted', 'No tool receipts recorded yet.')); return; }
      const list = el(doc, 'ul', 'solve-log-list');
      for (const r of receipts) {
        list.append(el(doc, 'li', 'solve-log-item',
          `${r.call_id} · ${r.tool} · ${r.ok ? 'ok' : 'error'}${r.process ? ` (${r.process})` : ''}${r.wall_ms != null ? ` · ${r.wall_ms} ms` : ''}`));
      }
      panel.append(list);
      return;
    }
    const custom = tabs.find((t) => t.id === activeTabId);
    if (custom && typeof custom.mount === 'function') {
      const mounted = custom.mount(panel, {
        document: doc, view, sessionId: opts.sessionId || '', token: opts.token, fetch: opts.fetch,
        katex: opts.katex, onSelectStep: selectStep, onAck: opts.onAck,
        onStepCorrected: opts.onStepCorrected, onTranscribe: opts.onTranscribe,
      });
      activeExtApi = mounted && typeof mounted === 'object' ? mounted : null;
      if (mounted && typeof mounted.destroy === 'function') activeExtDestroy = () => mounted.destroy();
      else if (typeof mounted === 'function') activeExtDestroy = mounted;
    }
  }

  function renderAll() {
    renderRunBarSection();
    writeupCtrl?.update({ sessionId: opts.sessionId || '' });
    renderMain();
    renderSideTabs(false);
    if (whyEntryId) {
      if ((view.steps || []).some((s) => String(s?.entry_id) === whyEntryId)) openWhy(whyEntryId);
      else closeWhy();
    }
  }

  renderAll();

  return {
    element: root,
    update(nextView) { view = nextView && typeof nextView === 'object' ? nextView : {}; renderAll(); },
    selectStep, openWhy, closeWhy,
    selectTab(tabId) { activeTabId = String(tabId || 'graph'); renderSideTabs(true); },
    registerTab(tab) { if (tab?.id) { localTabs.set(String(tab.id), tab); renderSideTabs(true); } },
    getTabs: allTabs, getGraphView: () => graphViewApi, getRunBar: () => runBarApi,
    destroy() { closeWhy(); destroySideWidgets(); if (writeupCtrl) { writeupCtrl.destroy(); writeupCtrl = null; } if (runBarApi) { runBarApi.destroy(); runBarApi = null; } host.replaceChildren(); },
  };
}
