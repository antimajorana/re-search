// Solve Result Sheet rendering, final-answer closure helpers, and parameter-based criterion descriptions.
// Extracted from `solve_steps.js` (keeping every source file <= 600 lines).
// Zero `innerHTML` writes; every untrusted string uses `textContent` only.

import { SOLVE_TIER_LABELS, createSolveBadgeGroup, normalizeSolveTier } from './solve_badges.js';
import { renderClaimFormula } from './solve_exprs.js';
import { BORDER_SOLID, createBadge, formatModelText } from './ui_badge.js';

// Owner decisions (`00-overview.md` 3.3, `solve.md` 3 & 5.5, R15).
export const DONE_LABEL = 'Done';
export const DONE_OPEN_ITEMS_LABEL = 'Done with open items';
export const NOT_DONE_LABEL = 'Not done';

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function formatSafeWeakestTier(rawTier) {
  const raw = String(rawTier || '').trim();
  if (!raw) return SOLVE_TIER_LABELS.unchecked;
  const key = raw.toLowerCase();
  if (Object.prototype.hasOwnProperty.call(SOLVE_TIER_LABELS, key)) return SOLVE_TIER_LABELS[key];
  if (/\b(verified|proved|new|novel)\b/i.test(raw)) return SOLVE_TIER_LABELS.unchecked;
  return raw;
}

function formatSafeRequiredTier(rawTier) {
  const raw = String(rawTier || '').trim();
  if (!raw) return SOLVE_TIER_LABELS.unchecked;
  const key = raw.toLowerCase();
  if (Object.prototype.hasOwnProperty.call(SOLVE_TIER_LABELS, key)) return SOLVE_TIER_LABELS[key];
  if (/\b(verified|proved|new|novel)\b/i.test(raw)) return SOLVE_TIER_LABELS.unchecked;
  return raw;
}

/**
 * Compute all `entry_id`s reachable backwards from `finalEntryId` over `depends_on`
 * and `derived_depends_on` (including `finalEntryId` itself if present in `steps`).
 */
export function computeFinalClosureIds(steps = [], finalEntryId = null) {
  if (!finalEntryId) return new Set();
  const byId = new Map();
  for (const s of Array.isArray(steps) ? steps : []) {
    if (s && s.entry_id) byId.set(String(s.entry_id), s);
  }
  const visited = new Set();
  const stack = [String(finalEntryId)];
  while (stack.length > 0) {
    const id = stack.pop();
    if (!id || visited.has(id)) continue;
    const step = byId.get(id);
    if (!step) continue;
    visited.add(id);
    const deps = [
      ...(Array.isArray(step.depends_on) ? step.depends_on : []),
      ...(Array.isArray(step.derived_depends_on) ? step.derived_depends_on : []),
    ];
    for (const dep of deps) {
      const depId = String(dep || '');
      if (depId && !visited.has(depId)) stack.push(depId);
    }
  }
  return visited;
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

/**
 * Format a criterion strictly from its `kind`, `params`, and `evidence` (never raw IDs).
 * Shows "Different model" ONLY when `require_different_model === true`.
 */
export function describeCriterion(crit, setupCriteria = []) {
  if (!crit || typeof crit !== 'object') return 'Criterion';
  const setupMatch = Array.isArray(setupCriteria)
    ? setupCriteria.find((sc) => sc && String(sc.id || '') === String(crit.id || ''))
    : null;
  const ev = crit.evidence && typeof crit.evidence === 'object' ? crit.evidence : {};
  const params = crit.params && typeof crit.params === 'object'
    ? crit.params
    : (setupMatch?.params && typeof setupMatch.params === 'object' ? setupMatch.params : {});
  const kind = String(crit.kind || ev.kind || setupMatch?.kind || '').trim();
  const reqDiff = (ev.require_different_model !== undefined
    ? ev.require_different_model
    : params.require_different_model) === true;
  const rawTier = ev.required ?? params.tier ?? '';
  const tierLabel = rawTier ? formatSafeRequiredTier(rawTier) : '';
  const reRooted = Array.isArray(ev.re_rooted_past_active_final) && ev.re_rooted_past_active_final.length > 0
    ? ` (re-rooted past active final: ${ev.re_rooted_past_active_final.join(', ')})`
    : '';

  switch (kind) {
    case 'final_tier_at_least': {
      const targetDesc = reqDiff
        ? `Different model review (${tierLabel || SOLVE_TIER_LABELS.model_agreed})`
        : `at least ${tierLabel || SOLVE_TIER_LABELS.unchecked}`;
      return `Final answer: ${targetDesc}${reRooted}`;
    }
    case 'closure_tier_at_least': {
      const targetDesc = reqDiff
        ? `Different model review (${tierLabel || SOLVE_TIER_LABELS.model_agreed})`
        : `at least ${tierLabel || SOLVE_TIER_LABELS.unchecked}`;
      return `All closure steps: ${targetDesc}${reRooted}`;
    }
    case 'named_check_passed': {
      const checkName = String(ev.check || params.check || 'check').trim();
      const onScope = String(ev.on || params.on || 'final').trim();
      const scopeText = onScope === 'all_results' ? 'on all result steps' : 'on final answer';
      return `Check "${checkName}" passed (${scopeText})${reRooted}`;
    }
    case 'no_open_flags':
      return `No unacknowledged assumptions or open flags${reRooted}`;
    case 'user_confirms': {
      const text = String(ev.text || params.text || '').trim();
      return `${text || 'User confirmation required'}${reRooted}`;
    }
    default:
      return `Check criterion${reRooted}`;
  }
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

function collectOutsideFinalSummary(steps, closureIds, finalEntryId, doneHistory) {
  const activeOutside = (Array.isArray(steps) ? steps : []).filter(
    (s) => s && !s.superseded_by && !closureIds.has(String(s.entry_id)),
  );
  if (activeOutside.length === 0) return null;

  const historyFinals = new Set(
    (Array.isArray(doneHistory) ? doneHistory : [])
      .map((d) => String(d?.final_entry_id || ''))
      .filter((id) => id && id !== String(finalEntryId || '')),
  );
  const assumedOutside = [];
  const priorFinalsOutside = [];
  for (const s of activeOutside) {
    const id = String(s.entry_id);
    const ownTier = normalizeSolveTier(s.badge?.own);
    if (ownTier === 'assumed_by_model') assumedOutside.push(id);
    if (s.entry_type === 'final_answer' || historyFinals.has(id)) priorFinalsOutside.push(id);
  }
  return { activeOutside, assumedOutside, priorFinalsOutside };
}

export function renderSolveResultSheet(doc, view, handleMap, opts = {}) {
  const done = view?.done;
  if (!done || typeof done !== 'object') return null;

  const sheet = el(doc, 'section', 'solve-result-sheet');
  sheet.setAttribute('data-testid', 'solve-result-sheet');
  sheet.setAttribute('data-done-status', String(done.status || ''));

  const head = el(doc, 'div', 'solve-result-head');
  const liveStatusLine = String(view?.result_sheet?.status_line || '').trim();
  const headingText = liveStatusLine || resolveDoneHeading(done);
  const title = el(doc, 'h3', 'solve-result-title', headingText);
  title.setAttribute('data-testid', 'solve-done-heading');
  head.append(title);

  const steps = Array.isArray(view.steps) ? view.steps : [];
  const finalEntryId = done.final_entry_id ? String(done.final_entry_id) : '';
  const finalStep = steps.find((s) => String(s?.entry_id) === finalEntryId) || null;
  if (finalStep?.badge) {
    head.append(createSolveBadgeGroup(finalStep.badge, {
      document: doc, onBadgeClick: () => opts.onOpenWhy?.(finalStep), onSelectStep: opts.onSelectStep,
    }));
  }
  sheet.append(head);

  // R5: render final answer formulas strictly from solve.expr handles, never step.latex.
  // Fall back to finalStep.summary when the final step has no claims.
  if (finalStep && Array.isArray(finalStep.claims) && finalStep.claims.length > 0) {
    const ansBox = el(doc, 'div', 'solve-result-answer');
    ansBox.setAttribute('data-testid', 'solve-result-answer');
    for (const claim of finalStep.claims) {
      const row = el(doc, 'div', 'solve-claim-formula');
      renderClaimFormula(row, claim, handleMap, opts.katex, { display: true });
      ansBox.append(row);
    }
    sheet.append(ansBox);
  } else if (finalStep && String(finalStep.summary || '').trim()) {
    const ansBox = el(doc, 'div', 'solve-result-answer');
    ansBox.setAttribute('data-testid', 'solve-result-answer');
    ansBox.append(el(doc, 'p', 'solve-result-answer-summary', String(finalStep.summary).trim()));
    sheet.append(ansBox);
  }

  const criteria = Array.isArray(done.criteria) ? done.criteria : [];
  const setupCriteria = Array.isArray(view?.setup?.done) ? view.setup.done : [];
  if (criteria.length > 0) {
    const list = el(doc, 'ul', 'solve-result-criteria');
    list.setAttribute('data-testid', 'solve-result-criteria');
    for (const crit of criteria) {
      const li = el(doc, 'li', 'solve-result-criterion');
      li.setAttribute('data-criterion-id', String(crit?.id || ''));
      li.setAttribute('data-met', String(Boolean(crit?.met)));
      li.append(
        createBadge({ text: crit?.met ? 'Met' : 'Open', icon: crit?.met ? 'check' : 'warn', tone: crit?.met ? 'ok' : 'warn', border: BORDER_SOLID }),
        el(doc, 'span', 'solve-criterion-name', describeCriterion(crit, setupCriteria)),
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

  // Restrict "Own checks" counts strictly to active steps inside the final closure when available.
  const hasFinalInSteps = Boolean(finalEntryId) && steps.some((s) => String(s?.entry_id) === finalEntryId);
  const closureIds = hasFinalInSteps ? computeFinalClosureIds(steps, finalEntryId) : new Set();
  const countedSteps = hasFinalInSteps
    ? steps.filter((s) => s && closureIds.has(String(s.entry_id)))
    : steps;

  // R22: only list tiers with count > 0 so "Verified" never appears when count === 0.
  const countParts = [];
  for (const [tier, count] of countActiveTiers(countedSteps).entries()) {
    if (count > 0 && SOLVE_TIER_LABELS[tier]) countParts.push(`${count} ${SOLVE_TIER_LABELS[tier]}`);
  }
  if (countParts.length > 0) sheet.append(el(doc, 'p', 'solve-result-counts', `Own checks: ${countParts.join(', ')}.`));

  if (hasFinalInSteps) {
    const outside = collectOutsideFinalSummary(steps, closureIds, finalEntryId, view?.done_history);
    if (outside) {
      const outsideIds = outside.activeOutside.map((s) => String(s.entry_id)).join(', ');
      const parts = [`${outside.activeOutside.length} active step${outside.activeOutside.length === 1 ? '' : 's'} not used by the final answer (${outsideIds})`];
      if (outside.assumedOutside.length > 0) {
        parts.push(`unacknowledged model assumption${outside.assumedOutside.length === 1 ? '' : 's'} outside closure: ${outside.assumedOutside.join(', ')}`);
      }
      if (outside.priorFinalsOutside.length > 0) {
        parts.push(`earlier final answer${outside.priorFinalsOutside.length === 1 ? '' : 's'} outside closure: ${outside.priorFinalsOutside.join(', ')}`);
      }
      const note = el(doc, 'p', 'solve-outside-final-note', parts.join('; '));
      note.setAttribute('data-testid', 'solve-outside-final-note');
      sheet.append(note);
    }
  }

  const rawSummary = String(done.summary_markdown || '').trim();
  const fallbackSummary = !rawSummary && finalStep && (!Array.isArray(finalStep.claims) || finalStep.claims.length === 0)
    ? String(finalStep.summary || '').trim()
    : '';
  const summaryText = rawSummary || fallbackSummary;
  if (summaryText) {
    const sumWrap = el(doc, 'div', 'solve-result-summary-wrap');
    sumWrap.append(el(doc, 'strong', 'solve-subhead', 'Summary (written by model)'));
    const sumEl = el(doc, 'p', 'solve-result-summary', formatModelText(summaryText, { isVerdict: false }));
    sumEl.setAttribute('data-testid', 'solve-result-summary');
    sumWrap.append(sumEl);
    sheet.append(sumWrap);
  }

  const openItems = Array.isArray(done.open_items) ? done.open_items.filter((x) => String(x || '').trim()) : [];
  if (openItems.length > 0) {
    const openSec = el(doc, 'div', 'solve-result-open-items');
    openSec.setAttribute('data-testid', 'solve-result-open-items');
    openSec.append(el(doc, 'strong', 'solve-subhead', 'Open items (written by model)'));
    const ul = el(doc, 'ul', 'solve-result-open-items-list');
    for (const item of openItems) {
      ul.append(el(doc, 'li', 'solve-result-open-item', String(item).trim()));
    }
    openSec.append(ul);
    sheet.append(openSec);
  }
  return sheet;
}
