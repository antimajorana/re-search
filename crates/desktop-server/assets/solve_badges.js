// Solve S2 StepBadge / StepTier adapter for F6 ui_badge.js and graph_view.js (package S13a).
// Aligns wording 1:1 with `crates/research-tools/src/solve_badge.rs` (`wording::*`),
// `crates/desktop-server/src/solve_view.rs`, `00-overview.md` 2.10 (2026-11-26), and `solve.md` 3 & 4.4.
// Zero `innerHTML` writes; every untrusted string uses `textContent` only.

import {
  BORDER_DASHED,
  BORDER_SOLID,
  OCR_TAG_WORD,
  SOLVE_EXACT_WORD,
  SOLVE_NUMERIC_WORD,
  USER_TAG_WORD,
  createBadge,
} from './ui_badge.js';

// Owner decisions (`00-overview.md` 2.10, 8.1, 8.3 #11; `solve.md` D1, D2, D10, D11, D13, R22).
export const SOLVE_TIER_LABELS = Object.freeze({
  verified: SOLVE_EXACT_WORD,
  checked_numerically: SOLVE_NUMERIC_WORD,
  model_agreed: 'Model agreed',
  model_agreed_same_model: 'Model agreed (same model)',
  checked_by_you: 'Checked by you',
  unchecked: 'Unchecked',
  failed: 'Failed',
  given: 'Given',
  cited: 'Found',
  assumed_by_model: 'Assumed by model',
  stale: 'Stale',
});

// `solve.md` 3 & R22: shown strictly when `own === 'verified'` and `effective !== 'verified'`.
export const OWN_CHECK_VERIFIED_LINE = 'own check: verified';

// `solve.md` 3 & `solve_view.rs`: exact Verified legend explanation.
export const LEGEND_VERIFIED_EXPLANATION =
  'the two sides are equal as exact expressions under the stated assumptions, as decided by SymPy; ' +
  'this does not show that the expression means what the text says';

// `solve.md` 4.4 & D2/D13: flags shown on the badge and never hidden.
export const FLAG_TRANSCRIPTION_UNCONFIRMED = 'transcription unconfirmed';
export const FLAG_SAME_MODEL = 'same model';
export const FLAG_OCR_SOURCE = 'OCR source';
export const PROVENANCE_TAG_YOU = USER_TAG_WORD;
export const PROVENANCE_TAG_OCR = OCR_TAG_WORD;

const DISPLAY_GRADE_MAP = Object.freeze({
  by_code: 'code',
  code: 'code',
  matched_quote_full_text: 'quote_fulltext',
  quote_fulltext: 'quote_fulltext',
  matched_quote_scanned_page: 'quote_ocr',
  quote_ocr: 'quote_ocr',
  matched_quote_abstract: 'quote_abstract',
  quote_abstract: 'quote_abstract',
  model_statement: 'model',
  model: 'model',
  user_statement: 'user',
  user: 'user',
});

const TIER_BORDER_MAP = Object.freeze({
  verified: 'solid',
  checked_numerically: 'solid',
  cited: 'solid',
  model_agreed: 'dashed',
  model_agreed_same_model: 'dashed',
  assumed_by_model: 'dashed',
  unchecked: 'dashed',
  given: 'plain',
  checked_by_you: 'plain',
  failed: 'none',
  stale: 'none',
});

export function normalizeSolveTier(rawTier) {
  const clean = String(rawTier || '').trim().toLowerCase();
  return Object.prototype.hasOwnProperty.call(SOLVE_TIER_LABELS, clean) ? clean : 'unchecked';
}

export function badgeIconAndTone(effectiveTier, stale = false) {
  const tier = normalizeSolveTier(effectiveTier);
  if (stale || tier === 'stale') {
    return { icon: 'warn', tone: 'muted' };
  }
  switch (tier) {
    case 'verified':
      return { icon: 'check', tone: 'ok' };
    case 'checked_numerically':
      return { icon: 'numeric', tone: 'ok' };
    case 'cited':
      return { icon: 'quote', tone: 'ok' };
    case 'given':
    case 'checked_by_you':
      return { icon: 'user', tone: 'neutral' };
    case 'model_agreed':
      return { icon: 'model', tone: 'info' };
    case 'model_agreed_same_model':
      return { icon: 'model', tone: 'warn' };
    case 'assumed_by_model':
      return { icon: 'warn', tone: 'warn' };
    case 'failed':
      return { icon: 'error', tone: 'err' };
    case 'unchecked':
    default:
      return { icon: 'info', tone: 'muted' };
  }
}

export function mapDisplayGradeKey(rawGrade, effectiveTier = 'unchecked', flags = []) {
  const key = String(rawGrade || '').trim().toLowerCase();
  if (Object.prototype.hasOwnProperty.call(DISPLAY_GRADE_MAP, key)) {
    return DISPLAY_GRADE_MAP[key];
  }
  const tier = normalizeSolveTier(effectiveTier);
  if (tier === 'verified' || tier === 'checked_numerically') return 'code';
  if (tier === 'cited') {
    const hasOcr = Array.isArray(flags) && flags.some((f) => String(f) === FLAG_OCR_SOURCE);
    return hasOcr ? 'quote_ocr' : 'quote_fulltext';
  }
  if (tier === 'given' || tier === 'checked_by_you') return 'user';
  return 'model';
}

export function resolveBorderStyle(stepBadge) {
  const raw = String(stepBadge?.border || '').trim().toLowerCase();
  if (raw === 'solid' || raw === 'dashed' || raw === 'plain' || raw === 'none') {
    return raw;
  }
  const tier = normalizeSolveTier(stepBadge?.effective);
  return TIER_BORDER_MAP[tier] || 'dashed';
}

export function resolveSafeBadgeLabel(stepBadge) {
  const effective = normalizeSolveTier(stepBadge?.effective);
  const stale = Boolean(stepBadge?.stale) || effective === 'stale';
  const canonical = stale ? SOLVE_TIER_LABELS.stale : SOLVE_TIER_LABELS[effective];
  const rawLabel = String(stepBadge?.label || '').trim();
  if (!rawLabel) return canonical;
  // Rule R22 fail-closed guard: never show "verified" when stale or effective != verified,
  // and never allow "proved", "new", or "novel" on Solve step badges.
  if ((stale || effective !== 'verified') && /\bverified\b/i.test(rawLabel)) {
    return canonical;
  }
  if (/\b(proved|new|novel)\b/i.test(rawLabel)) {
    return canonical;
  }
  return rawLabel;
}

export function resolveSafeOwnCheckLine(stepBadge) {
  const own = normalizeSolveTier(stepBadge?.own);
  const effective = normalizeSolveTier(stepBadge?.effective);
  const stale = Boolean(stepBadge?.stale) || effective === 'stale';
  if (!stale && own === 'verified' && effective !== 'verified') {
    return OWN_CHECK_VERIFIED_LINE;
  }
  return null;
}

export function mapBadgeToUiSpec(stepBadge = {}, opts = {}) {
  const effective = normalizeSolveTier(stepBadge?.effective);
  const stale = Boolean(stepBadge?.stale) || effective === 'stale';
  const flags = Array.isArray(stepBadge?.flags) ? stepBadge.flags : [];
  const { icon, tone } = badgeIconAndTone(effective, stale);
  const borderStyle = resolveBorderStyle(stepBadge);
  const uiBorder = borderStyle === 'dashed' ? BORDER_DASHED : BORDER_SOLID;
  const grade = mapDisplayGradeKey(stepBadge?.grade, effective, flags);
  const text = resolveSafeBadgeLabel(stepBadge);
  const titleParts = [text];
  const ownLine = resolveSafeOwnCheckLine(stepBadge);
  if (ownLine) titleParts.push(ownLine);
  if (stepBadge?.rests_on_line) titleParts.push(String(stepBadge.rests_on_line));
  return {
    text,
    secondaryText: opts.inlineRestsOn && stepBadge?.rests_on_line ? String(stepBadge.rests_on_line) : '',
    icon,
    tone,
    border: uiBorder,
    solveBorder: borderStyle,
    grade,
    title: opts.title || titleParts.join(' · '),
    onClick: typeof opts.onClick === 'function' ? opts.onClick : undefined,
  };
}

function makeEl(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function createSolveBadgeGroup(stepBadge = {}, opts = {}) {
  const doc = opts.document || document;
  const effective = normalizeSolveTier(stepBadge?.effective);
  const own = normalizeSolveTier(stepBadge?.own);
  const stale = Boolean(stepBadge?.stale) || effective === 'stale';
  const borderStyle = resolveBorderStyle(stepBadge);
  const flags = Array.isArray(stepBadge?.flags) ? stepBadge.flags : [];
  const sublabels = Array.isArray(stepBadge?.sublabels) ? stepBadge.sublabels : [];
  const grade = mapDisplayGradeKey(stepBadge?.grade, effective, flags);

  const wrap = makeEl(doc, 'div', `solve-badge-group solve-border--${borderStyle}`);
  wrap.setAttribute('data-effective-tier', effective);
  wrap.setAttribute('data-own-tier', own);
  wrap.setAttribute('data-grade', grade);
  wrap.setAttribute('data-border', borderStyle);
  if (stale) wrap.setAttribute('data-stale', 'true');

  const topRow = makeEl(doc, 'div', 'solve-badge-row');
  const primarySpec = mapBadgeToUiSpec(stepBadge, {
    onClick: opts.onBadgeClick || opts.onClick,
    title: opts.title,
  });
  const primaryBadge = createBadge(primarySpec);
  primaryBadge.setAttribute('data-badge-role', 'primary');
  topRow.appendChild(primaryBadge);

  const ownLine = resolveSafeOwnCheckLine(stepBadge);
  if (ownLine) {
    const ownBadge = createBadge({
      text: ownLine,
      icon: 'check',
      tone: 'muted',
      border: BORDER_SOLID,
    });
    ownBadge.setAttribute('data-badge-role', 'own-check');
    topRow.appendChild(ownBadge);
  }

  const provTag = String(stepBadge?.provenance_tag || '').trim();
  if (provTag) {
    const provBadge = createBadge({
      text: provTag,
      icon: provTag === PROVENANCE_TAG_OCR ? 'ocr' : 'user',
      tone: provTag === PROVENANCE_TAG_OCR ? 'warn' : 'neutral',
      border: BORDER_SOLID,
    });
    provBadge.setAttribute('data-badge-role', 'provenance');
    topRow.appendChild(provBadge);
  }

  for (const sub of sublabels) {
    const text = String(sub ?? '').trim();
    if (!text) continue;
    const chip = makeEl(doc, 'span', 'solve-sublabel-chip', text);
    chip.setAttribute('data-badge-role', 'sublabel');
    topRow.appendChild(chip);
  }
  wrap.appendChild(topRow);

  const restsOnText = String(stepBadge?.rests_on_line || '').trim();
  const depIds = Array.isArray(stepBadge?.lowered_by) && stepBadge.lowered_by.length
    ? stepBadge.lowered_by
    : (Array.isArray(stepBadge?.broken_by) ? stepBadge.broken_by : []);
  if (restsOnText || (typeof opts.onSelectStep === 'function' && depIds.length > 0)) {
    const restsEl = makeEl(doc, 'div', 'solve-rests-on-line');
    restsEl.setAttribute('data-badge-role', 'rests-on');
    if (restsOnText) {
      restsEl.appendChild(makeEl(doc, 'span', 'solve-rests-on-text', restsOnText));
    }
    if (typeof opts.onSelectStep === 'function' && depIds.length) {
      const links = makeEl(doc, 'span', 'solve-rests-on-links');
      for (const depId of depIds) {
        const btn = makeEl(doc, 'button', 'solve-step-link', String(depId));
        btn.type = 'button';
        btn.setAttribute('data-jump-step', String(depId));
        btn.addEventListener('click', () => opts.onSelectStep(String(depId)));
        links.appendChild(btn);
      }
      restsEl.appendChild(links);
    }
    wrap.appendChild(restsEl);
  }

  // Flags are shown on the badge and NEVER hidden (`solve.md` 4.4).
  if (flags.length > 0) {
    const flagRow = makeEl(doc, 'div', 'solve-badge-flags');
    flagRow.setAttribute('data-badge-role', 'flags');
    for (const flag of flags) {
      const text = String(flag ?? '').trim();
      if (!text) continue;
      const chip = makeEl(doc, 'span', 'solve-flag-chip', text);
      chip.setAttribute('data-flag', text);
      flagRow.appendChild(chip);
    }
    if (flagRow.children.length > 0) {
      wrap.appendChild(flagRow);
    }
  }

  return wrap;
}

export function hasAnyVerifiedStep(steps = []) {
  return (Array.isArray(steps) ? steps : []).some((step) => {
    if (!step || step.superseded_by) return false;
    const badge = step.badge;
    if (!badge || badge.stale) return false;
    return normalizeSolveTier(badge.own) === 'verified' || normalizeSolveTier(badge.effective) === 'verified';
  });
}

export function createSolveLegend(graphLegend = [], steps = [], opts = {}) {
  const doc = opts.document || document;
  const hasSteps = Array.isArray(steps) && steps.length > 0;
  const includeVerified = hasSteps
    ? hasAnyVerifiedStep(steps)
    : (Array.isArray(graphLegend) && graphLegend.some((e) => String(e?.tier || '') === 'verified'));

  const items = Array.isArray(graphLegend) && graphLegend.length > 0
    ? graphLegend.filter((e) => includeVerified || String(e?.tier || '') !== 'verified')
    : [
        ...(includeVerified ? [{
          tier: 'verified',
          label: SOLVE_TIER_LABELS.verified,
          explanation: LEGEND_VERIFIED_EXPLANATION,
          icon: 'check',
          tone: 'ok',
          dashed: false,
        }] : []),
        { tier: 'checked_numerically', label: SOLVE_TIER_LABELS.checked_numerically, icon: 'numeric', tone: 'ok', dashed: false },
        { tier: 'cited', label: SOLVE_TIER_LABELS.cited, icon: 'quote', tone: 'ok', dashed: false },
        { tier: 'model_agreed', label: `${SOLVE_TIER_LABELS.model_agreed} / ${SOLVE_TIER_LABELS.unchecked}`, icon: 'model', tone: 'muted', dashed: true },
        { tier: 'given', label: `${SOLVE_TIER_LABELS.given} / ${SOLVE_TIER_LABELS.checked_by_you} (${PROVENANCE_TAG_YOU})`, icon: 'user', tone: 'neutral', dashed: false },
      ];

  const box = makeEl(doc, 'div', 'solve-legend');
  box.setAttribute('data-testid', 'solve-legend');
  const row = makeEl(doc, 'div', 'solve-legend-row');
  let verifiedNoteText = '';
  for (const entry of items) {
    const isVer = String(entry?.tier || '') === 'verified';
    if (isVer && !includeVerified) continue;
    row.appendChild(createBadge({
      text: String(entry?.label || entry?.text || ''),
      icon: entry?.icon || 'info',
      tone: entry?.tone || 'neutral',
      border: entry?.dashed ? BORDER_DASHED : BORDER_SOLID,
    }));
    if (isVer) {
      verifiedNoteText = String(entry?.explanation || LEGEND_VERIFIED_EXPLANATION);
    }
  }
  box.appendChild(row);
  if (verifiedNoteText) {
    const note = makeEl(doc, 'p', 'solve-legend-verified-note', `${SOLVE_TIER_LABELS.verified}: ${verifiedNoteText}`);
    note.setAttribute('data-testid', 'solve-legend-verified-note');
    box.appendChild(note);
  }
  return box;
}

export function findFinalAnswerEntryId(solveView) {
  if (solveView?.done?.final_entry_id) return String(solveView.done.final_entry_id);
  const steps = Array.isArray(solveView?.steps) ? solveView.steps : [];
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    const s = steps[i];
    if (s && !s.superseded_by && s.entry_type === 'final_answer') return String(s.entry_id);
  }
  return null;
}

export function computeFinalClosureIds(steps = [], targetEntryId = null) {
  if (!targetEntryId) return new Set();
  const byId = new Map();
  for (const s of Array.isArray(steps) ? steps : []) {
    if (s && s.entry_id) byId.set(String(s.entry_id), s);
  }
  const visited = new Set();
  const stack = [String(targetEntryId)];
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

export function adaptSolveGraphView(solveView = {}, opts = {}) {
  const base = solveView?.graph && Array.isArray(solveView.graph.nodes)
    ? solveView.graph
    : { nodes: [], edges: [], legend: [] };
  const steps = Array.isArray(solveView?.steps) ? solveView.steps : [];
  const finalId = opts.finalEntryId ?? findFinalAnswerEntryId(solveView);
  const closureOnly = Boolean(opts.closureOfFinal) && Boolean(finalId);
  const keepIds = closureOnly ? computeFinalClosureIds(steps, finalId) : null;

  const nodes = (base.nodes || [])
    .filter((n) => !keepIds || keepIds.has(String(n.id)))
    .map((n) => ({
      ...n,
      grade: mapDisplayGradeKey(n.grade, n.effective, n.flags),
    }));
  const nodeIds = new Set(nodes.map((n) => String(n.id)));
  const edges = (base.edges || []).filter(
    (e) => nodeIds.has(String(e.src)) && nodeIds.has(String(e.dst)),
  );
  const filteredSteps = keepIds ? steps.filter((s) => keepIds.has(String(s.entry_id))) : steps;
  const keepVerifiedLegend = hasAnyVerifiedStep(filteredSteps);
  const legend = (Array.isArray(base.legend) ? base.legend : []).filter(
    (item) => keepVerifiedLegend || String(item?.tier || '') !== 'verified',
  );
  return { nodes, edges, legend };
}
