// Solve S13b derivation workspace: topological step cards (R5 handle-only formulas),
// Result sheet (R15), F6 run_bar.js + graph_view.js ("closure of the final answer"),
// Why popover hook, Expressions/Sources/Log tabs, and registerSolveTab extension registry.
// Zero `innerHTML` writes; every untrusted string uses `textContent` only.

import { createGraphView } from './graph_view.js';
import { createQuoteChip } from './quote_chip.js';
import { createRunBar } from './run_bar.js';
import {
  SOLVE_TIER_LABELS, adaptSolveGraphView, createSolveBadgeGroup,
  createSolveLegend, findFinalAnswerEntryId, normalizeSolveTier,
} from './solve_badges.js';
import { buildHandleLookup, createSolveExprsView, renderClaimFormula } from './solve_exprs.js';
import { createSolveWhyPopover } from './solve_why.js';
import { BORDER_SOLID, createBadge, formatModelText } from './ui_badge.js';

// Owner decisions (`00-overview.md` 3.3, `solve.md` 3 & 5.5, R15).
export const DONE_LABEL = 'Done';
export const DONE_OPEN_ITEMS_LABEL = 'Done with open items';
export const NOT_DONE_LABEL = 'Not done';
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

function formatSafeWeakestTier(rawTier) {
  const raw = String(rawTier || '').trim();
  if (!raw) return SOLVE_TIER_LABELS.unchecked;
  const key = raw.toLowerCase();
  if (Object.prototype.hasOwnProperty.call(SOLVE_TIER_LABELS, key)) return SOLVE_TIER_LABELS[key];
  if (/\b(verified|proved|new|novel)\b/i.test(raw)) return SOLVE_TIER_LABELS.unchecked;
  return raw;
}

/**
 * Rule R15: "Done" without "with open items" is reachable ONLY when `done.status === 'finished'`
 * and every recorded criterion has `met === true`.
 */
export function resolveDoneHeading(done) {
  if (!done || typeof done !== 'object') return NOT_DONE_LABEL;
  const criteria = Array.isArray(done.criteria) ? done.criteria : [];
  const allMet = criteria.every((c) => Boolean(c && c.met === true));
  return done.status === 'finished' && allMet ? DONE_LABEL : DONE_OPEN_ITEMS_LABEL;
}

function mapRunStateToBarProps(runState) {
  const rs = runState && typeof runState === 'object' ? runState : {};
  const rawState = String(rs.state || 'paused');
  const openItems = Array.isArray(rs.open_items)
    ? rs.open_items.map((item) => {
        if (typeof item === 'string') return item;
        if (item?.kind === 'unmet_criterion') return `Unmet criterion: ${item.id}`;
        if (item?.kind === 'unacknowledged_assumption') return `Unacknowledged assumption: ${item.entry_id}`;
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

function countActiveTiers(steps) {
  const counts = new Map();
  for (const s of Array.isArray(steps) ? steps : []) {
    if (!s || s.superseded_by) continue;
    const tier = s.badge?.stale ? 'stale' : normalizeSolveTier(s.badge?.own || s.badge?.effective);
    counts.set(tier, (counts.get(tier) || 0) + 1);
  }
  return counts;
}

export function renderSolveResultSheet(doc, view, handleMap, opts = {}) {
  const done = view?.done;
  if (!done || typeof done !== 'object') return null;

  const sheet = el(doc, 'section', 'solve-result-sheet');
  sheet.setAttribute('data-testid', 'solve-result-sheet');
  sheet.setAttribute('data-done-status', String(done.status || ''));

  const head = el(doc, 'div', 'solve-result-head');
  const title = el(doc, 'h3', 'solve-result-title', resolveDoneHeading(done));
  title.setAttribute('data-testid', 'solve-done-heading');
  head.append(title);

  const steps = Array.isArray(view.steps) ? view.steps : [];
  const finalStep = steps.find((s) => String(s?.entry_id) === String(done.final_entry_id)) || null;
  if (finalStep?.badge) {
    head.append(createSolveBadgeGroup(finalStep.badge, {
      document: doc, onBadgeClick: () => opts.onOpenWhy?.(finalStep), onSelectStep: opts.onSelectStep,
    }));
  }
  sheet.append(head);

  // R5: render final answer formulas strictly from solve.expr handles, never step.latex.
  if (finalStep && Array.isArray(finalStep.claims) && finalStep.claims.length > 0) {
    const ansBox = el(doc, 'div', 'solve-result-answer');
    ansBox.setAttribute('data-testid', 'solve-result-answer');
    for (const claim of finalStep.claims) {
      const row = el(doc, 'div', 'solve-claim-formula');
      renderClaimFormula(row, claim, handleMap, opts.katex, { display: true });
      ansBox.append(row);
    }
    sheet.append(ansBox);
  }

  const criteria = Array.isArray(done.criteria) ? done.criteria : [];
  if (criteria.length > 0) {
    const list = el(doc, 'ul', 'solve-result-criteria');
    list.setAttribute('data-testid', 'solve-result-criteria');
    for (const crit of criteria) {
      const li = el(doc, 'li', 'solve-result-criterion');
      li.setAttribute('data-criterion-id', String(crit?.id || ''));
      li.setAttribute('data-met', String(Boolean(crit?.met)));
      li.append(
        createBadge({ text: crit?.met ? 'Met' : 'Open', icon: crit?.met ? 'check' : 'warn', tone: crit?.met ? 'ok' : 'warn', border: BORDER_SOLID }),
        el(doc, 'span', 'solve-criterion-name', String(crit?.id || 'criterion')),
      );
      list.append(li);
    }
    sheet.append(list);
  }

  const rests = done.rests_on || {};
  const assumed = Array.isArray(rests.assumed_by_model) ? rests.assumed_by_model : [];
  const weakest = Array.isArray(rests.weakest) ? rests.weakest : [];
  if (assumed.length > 0 || weakest.length > 0) {
    const restsSec = el(doc, 'div', 'solve-result-rests');
    restsSec.setAttribute('data-testid', 'solve-result-rests');
    restsSec.append(el(doc, 'strong', 'solve-subhead', 'Rests on'));
    const errBox = el(doc, 'div', 'solve-error-banner');
    errBox.hidden = true;
    errBox.setAttribute('role', 'alert');
    restsSec.append(errBox);
    for (const eid of assumed) {
      const row = el(doc, 'div', 'solve-rests-item');
      const jump = el(doc, 'button', 'solve-step-link', `${eid} (assumed by model)`);
      jump.type = 'button';
      jump.addEventListener('click', () => opts.onSelectStep?.(String(eid)));
      row.append(jump);
      const targetStep = steps.find((s) => String(s?.entry_id) === String(eid));
      if (targetStep && typeof opts.onAck === 'function') {
        const ackBtn = el(doc, 'button', 'pill-btn sm', `Acknowledge ${eid}`);
        ackBtn.type = 'button';
        ackBtn.setAttribute('data-ack-assumption', String(eid));
        ackBtn.addEventListener('click', () => {
          errBox.hidden = true;
          try {
            const p = opts.onAck(targetStep, {
              entry_id: targetStep.entry_id, kind: 'assumption', entry_sha256: targetStep.entry_sha256,
            });
            if (p && typeof p.then === 'function') {
              p.catch((err) => { errBox.textContent = String(err?.message || err || 'Ack failed'); errBox.hidden = false; });
            }
          } catch (err) { errBox.textContent = String(err?.message || err || 'Ack failed'); errBox.hidden = false; }
        });
        row.append(ackBtn);
      }
      if (targetStep && typeof opts.onOpenWhy === 'function') {
        const srcBtn = el(doc, 'button', 'pill-btn sm', 'Attach source');
        srcBtn.type = 'button';
        srcBtn.setAttribute('data-attach-assumption', String(eid));
        srcBtn.addEventListener('click', () => opts.onOpenWhy(targetStep));
        row.append(srcBtn);
      }
      restsSec.append(row);
    }
    for (const w of weakest) {
      if (!w?.entry_id) continue;
      const wStep = steps.find((s) => String(s?.entry_id) === String(w.entry_id));
      const tierText = wStep?.badge?.stale ? SOLVE_TIER_LABELS.stale : formatSafeWeakestTier(w.tier);
      const btn = el(doc, 'button', 'solve-step-link', `${w.entry_id} (${tierText})`);
      btn.type = 'button';
      btn.addEventListener('click', () => opts.onSelectStep?.(String(w.entry_id)));
      restsSec.append(btn);
    }
    sheet.append(restsSec);
  }

  // R22: only list tiers with count > 0 so "Verified" never appears when count === 0.
  const countParts = [];
  for (const [tier, count] of countActiveTiers(steps).entries()) {
    if (count > 0 && SOLVE_TIER_LABELS[tier]) countParts.push(`${count} ${SOLVE_TIER_LABELS[tier]}`);
  }
  if (countParts.length > 0) sheet.append(el(doc, 'p', 'solve-result-counts', `Own checks: ${countParts.join(', ')}.`));

  const rawSummary = String(done.summary_markdown || '').trim();
  if (rawSummary) {
    const sumEl = el(doc, 'p', 'solve-result-summary', formatModelText(rawSummary, { isVerdict: false }));
    sumEl.setAttribute('data-testid', 'solve-result-summary');
    sheet.append(sumEl);
  }
  return sheet;
}

function renderStepCard(doc, step, handleMap, receiptsById, checksByStep, opts) {
  const card = el(doc, 'article', 'solve-step-card');
  if (opts.selectedStepId === step.entry_id) card.classList.add('is-selected');
  if (step.superseded_by) card.classList.add('is-superseded');
  card.setAttribute('data-testid', 'solve-step-card');
  card.setAttribute('data-entry-id', String(step.entry_id || ''));
  card.setAttribute('data-entry-type', String(step.entry_type || ''));

  const head = el(doc, 'div', 'solve-step-head');
  const idBtn = el(doc, 'button', 'solve-step-id-btn', String(step.entry_id || ''));
  idBtn.type = 'button';
  idBtn.addEventListener('click', () => opts.onSelectStep?.(String(step.entry_id)));
  head.append(idBtn, el(doc, 'span', 'solve-step-type', String(step.entry_type || 'step')));
  if (step.superseded_by) head.append(el(doc, 'span', 'solve-flag-chip', `superseded by ${step.superseded_by}`));
  head.append(createSolveBadgeGroup(step.badge || {}, {
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
      const rec = receiptsById.get(String(rr.call_id));
      const line = rec
        ? `Receipt ${rec.call_id} (${rec.tool}, ${rec.ok ? 'ok' : 'error'}${rec.wall_ms != null ? `, ${rec.wall_ms} ms` : ''})`
        : `Receipt ${rr.call_id} (missing)`;
      det.append(el(doc, 'div', 'solve-receipt-line', line));
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
  let runBarApi = null, graphViewApi = null, exprsViewApi = null, whyPopoverApi = null, activeExtApi = null, activeExtDestroy = null;

  const localTabs = new Map();
  for (const t of Array.isArray(opts.extraTabs) ? opts.extraTabs : []) {
    if (t && t.id) localTabs.set(String(t.id), t);
  }

  const root = el(doc, 'section', 'solve-workspace');
  root.setAttribute('data-testid', 'solve-workspace');
  const runBarHost = el(doc, 'div', 'solve-run-bar-host');
  const grid = el(doc, 'div', 'solve-workspace-grid');
  const mainCol = el(doc, 'div', 'solve-derivation-col');
  const sideCol = el(doc, 'aside', 'solve-canvas-col');
  const whyHost = el(doc, 'div', 'solve-why-host');
  whyHost.hidden = true;
  grid.append(mainCol, sideCol);
  root.append(runBarHost, grid, whyHost);
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
    const props = mapRunStateToBarProps(view.run_state);
    const cbs = { onStop: opts.onStop, onContinue: opts.onContinue, onRetry: opts.onRetry };
    if (!runBarApi) runBarApi = createRunBar(runBarHost, props, cbs);
    else runBarApi.update(props, cbs);
  }

  function renderMain() {
    mainCol.replaceChildren();
    const handleMap = buildHandleLookup(view.handles || []);
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

    const steps = (Array.isArray(view.steps) ? view.steps : []).filter((s) => s && (showSuperseded || !s.superseded_by));
    const list = el(doc, 'div', 'solve-steps-list');
    list.setAttribute('data-testid', 'solve-steps-list');
    if (steps.length === 0) {
      list.append(el(doc, 'p', 'solve-muted', 'No derivation steps recorded yet.'));
    } else {
      for (const step of steps) {
        list.append(renderStepCard(doc, step, handleMap, receiptsById, checksByStep, {
          katex: opts.katex, selectedStepId, onSelectStep: selectStep, onOpenWhy: openWhy, onOpenSource: opts.onOpenSource,
        }));
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
    destroy() { closeWhy(); destroySideWidgets(); if (runBarApi) { runBarApi.destroy(); runBarApi = null; } host.replaceChildren(); },
  };
}
