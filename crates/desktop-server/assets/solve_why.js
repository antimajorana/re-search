// Solve "Why?" step provenance & actions popover (package S13b).
// Plain ES module; DOM construction uses createElement and textContent only.

import { SOLVE_TIER_LABELS, createSolveBadgeGroup, normalizeSolveTier } from './solve_badges.js';
import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';
import { createQuoteChip } from './quote_chip.js';

export const SOLVE_CHECK_NAMES = Object.freeze([
  'identity_symbolic', 'identity_numeric', 'series_match', 'vanishes_to_order',
  'counterexample_search', 'dimensions', 'limit_case', 'symmetry',
  'conservation', 'convergence', 'matrix_property',
]);
export const REVIEW_DISABLED_NOTICE =
  'Review with another model is available once verifier review (S14) lands.';
export const RECHECK_CHANGED_WARNING = 'result changed on re-run';

const EVIDENCE_KEYS = Object.freeze([
  'residual', 'points', 'seed', 'digits', 'precision', 'tolerance',
  'order', 'counterexample', 'dimensions', 'reason', 'comparison',
]);
const CHECK_OUTCOMES = Object.freeze({
  pass: Object.freeze({ text: 'Pass', icon: 'check', tone: 'ok' }),
  fail: Object.freeze({ text: 'Fail', icon: 'error', tone: 'err' }),
  inconclusive: Object.freeze({ text: 'Inconclusive', icon: 'warn', tone: 'warn' }),
  not_run: Object.freeze({ text: 'Not run', icon: 'info', tone: 'muted' }),
});

function formatMetricValue(val) {
  if (val == null) return '';
  if (typeof val !== 'object') return String(val);
  try { return JSON.stringify(val); } catch { return String(val); }
}

export function partitionStepChecks(step, allChecks = [], currentMathSha = null) {
  const stepEntryId = step?.entry_id ? String(step.entry_id) : null;
  const stepClaimSha = step?.claim_sha256 ? String(step.claim_sha256) : null;
  const mathSha = currentMathSha ? String(currentMathSha) : null;
  const current = [];
  const stale = [];
  for (const check of Array.isArray(allChecks) ? allChecks : []) {
    if (!check || typeof check !== 'object') continue;
    const isCurrent = Boolean(stepClaimSha)
      && check.claim_sha256 === stepClaimSha
      && (!mathSha || check.math_sha256 === mathSha);
    if (isCurrent) {
      current.push(check);
    } else if ((stepEntryId && check.entry_id === stepEntryId)
      || (stepClaimSha && check.claim_sha256 === stepClaimSha)) {
      stale.push(check);
    }
  }
  return { current, stale };
}

export function formatEvidenceSummary(evidence) {
  if (!evidence || typeof evidence !== 'object' || Array.isArray(evidence)) return [];
  const rows = [];
  for (const key of EVIDENCE_KEYS) {
    if (!Object.prototype.hasOwnProperty.call(evidence, key)) continue;
    const raw = evidence[key];
    if (raw == null || raw === '') continue;
    const value = formatMetricValue(raw).trim();
    if (value) rows.push({ key, value });
  }
  return rows;
}

function classifyCheckReason(evidence) {
  if (!evidence || typeof evidence !== 'object') return null;
  if (evidence.counterexample != null && evidence.counterexample !== '') {
    return { kind: 'counterexample', label: 'Counterexample', text: formatMetricValue(evidence.counterexample) };
  }
  const text = String(evidence.reason ?? evidence.error ?? '').trim();
  if (evidence.timeout === true || /\b(time\s*out|timed\s*out|timeout)\b/i.test(text)) {
    return { kind: 'timeout', label: 'Timeout', text: text || 'check timed out' };
  }
  if (/\b(undecid\w*|cannot\s+decide|indeterminate)\b/i.test(text)) {
    return { kind: 'undecidable', label: 'Undecidable', text };
  }
  return text ? { kind: 'reason', label: 'Reason', text } : null;
}

function formatReceiptEnv(env) {
  if (!env || typeof env !== 'object') return '';
  const keys = ['python', 'sympy', 'numpy', 'mpmath', 'scipy', ...Object.keys(env)];
  const seen = new Set();
  const parts = [];
  for (const k of keys) {
    if (seen.has(k)) continue;
    seen.add(k);
    const v = env[k];
    if (v != null && v !== '' && typeof v !== 'object') parts.push(`${k} ${String(v)}`);
  }
  return parts.join(' · ');
}

function parseSourceIdentifier(rawSource) {
  const s = String(rawSource || '').trim();
  if (/^(arxiv:|doi:|inspire:|s2:|openalex:|file:|sha256:|local:|url:|key:)/i.test(s)) return { library_key: s };
  if (/^(\d{4}\.\d{4,5}(v\d+)?|[a-z-]+(\.[a-z]{2})?\/\d{7}(v\d+)?|https?:\/\/)/i.test(s)) {
    return { arxiv_id: s };
  }
  return { source_id: s };
}

function el(doc, tag, className, text, attrs) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = String(text);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v != null) node.setAttribute(k, String(v));
    }
  }
  return node;
}

function fieldEl(doc, tag, className, props = {}, attrs = null) {
  const node = el(doc, tag, className, null, attrs);
  for (const [k, v] of Object.entries(props)) {
    if (v != null) {
      node[k] = v;
      if (k === 'name' || k === 'type') node.setAttribute(k, String(v));
    }
  }
  return node;
}

function clearNode(node) {
  if (typeof node.replaceChildren === 'function') { node.replaceChildren(); return; }
  while (node.firstChild) node.removeChild(node.firstChild);
}

function formatSafeTierLabel(rawTier) {
  const raw = String(rawTier || '').trim();
  if (!raw) return SOLVE_TIER_LABELS.unchecked;
  const key = raw.toLowerCase();
  if (Object.prototype.hasOwnProperty.call(SOLVE_TIER_LABELS, key)) return SOLVE_TIER_LABELS[key];
  if (/\b(verified|proved|new|novel)\b/i.test(raw)) return SOLVE_TIER_LABELS.unchecked;
  return raw;
}

function renderLoweringSection(doc, step, onSelectStep) {
  const badge = step?.badge || {};
  const own = normalizeSolveTier(badge.own);
  const effective = normalizeSolveTier(badge.effective);
  if (own === effective) return null;

  const box = el(doc, 'div', 'solve-why-lowering', null, { 'data-testid': 'solve-why-lowering' });
  const ownText = own === 'verified' && !badge.stale ? 'own check: verified' : `Own: ${SOLVE_TIER_LABELS[own]}`;
  box.appendChild(el(doc, 'div', 'solve-why-tier-diff', `${ownText} · Effective: ${SOLVE_TIER_LABELS[effective]}`));
  const groups = [
    ['lowered_by', 'Lowered by', badge.lowered_by],
    ['broken_by', 'Broken by', badge.broken_by],
    ['weakest', 'Weakest ancestor', step?.rests_on?.weakest],
    ['assumed_by_model', 'Assumed by model', step?.rests_on?.assumed_by_model],
  ];
  for (const [kind, label, rawItems] of groups) {
    const list = Array.isArray(rawItems) ? rawItems : [];
    if (!list.length) continue;
    const row = el(doc, 'div', 'solve-why-ancestor-row', null, { 'data-ancestor-group': kind });
    row.appendChild(el(doc, 'span', 'solve-why-ancestor-label', `${label}: `));
    for (const item of list) {
      const id = typeof item === 'string' ? item : String(item?.entry_id || '');
      if (!id) continue;
      const suffix = typeof item === 'object' && item?.tier ? ` (${formatSafeTierLabel(item.tier)})` : '';
      const btn = el(doc, 'button', 'solve-step-link', `${id}${suffix}`, {
        type: 'button', 'data-jump-step': id, 'data-ancestor-kind': kind,
      });
      if (typeof onSelectStep === 'function') btn.addEventListener('click', () => onSelectStep(id));
      else btn.disabled = true;
      row.appendChild(btn);
    }
    box.appendChild(row);
  }
  return box;
}

function buildQuoteSpec(step) {
  const sr = step.source_ref;
  const badge = step.badge || {};
  const isOcr = sr.ocr !== undefined
    ? Boolean(sr.ocr)
    : (badge.provenance_tag === 'OCR' || (Array.isArray(badge.flags) && badge.flags.includes('OCR source')));
  return {
    source_id: sr.source_id, path: sr.path, start_line: sr.start_line, end_line: sr.end_line,
    page: sr.page, ocr: isOcr, basis: sr.basis || (isOcr ? 'ocr' : undefined),
    abstract_only: sr.abstract_only, quote: sr.quote ?? sr.excerpt ?? '',
    paper: sr.paper || sr.source_id || sr.path || '',
    evidence_id: sr.evidence_id || (sr.evidence_seq != null ? `xe-${sr.evidence_seq}` : undefined),
    evidence_seq: sr.evidence_seq,
  };
}

export function createSolveWhyPopover(host, props = {}) {
  let currentProps = { ...props };
  const doc = currentProps.document || host?.ownerDocument || document;
  const root = el(doc, 'section', 'solve-why-popover', null, { role: 'dialog', 'data-testid': 'solve-why-popover' });
  let activeForm = null, recheckWarning = null, errorMessage = null, lastEntryId = currentProps.step?.entry_id ?? null;
  const expandedCodes = new Set();
  if (host && typeof host.appendChild === 'function') host.appendChild(root);

  function invokeCallback(fn, onDone) {
    if (typeof fn !== 'function') return;
    errorMessage = null;
    try {
      const res = fn();
      if (res && typeof res.then === 'function') {
        res.then((v) => onDone?.(v), (err) => { errorMessage = String(err?.message || err || 'Action failed'); render(); });
      } else onDone?.(res);
    } catch (err) { errorMessage = String(err?.message || err || 'Action failed'); render(); }
  }

  function handleRecheckResult(res) {
    if (res && res.changed === true) {
      const base = String(res.warning || RECHECK_CHANGED_WARNING).trim();
      const prevSha = res.original_evidence_sha256 || res.results?.find?.((r) => r?.changed)?.original_evidence_sha256;
      const nextSha = res.new_evidence_sha256 || res.results?.find?.((r) => r?.changed)?.new_evidence_sha256;
      recheckWarning = prevSha && nextSha
        ? `${base} (${String(prevSha).slice(0, 8)} → ${String(nextSha).slice(0, 8)})`
        : base;
      render();
    } else if (recheckWarning && res && res.changed === false) {
      recheckWarning = null;
      render();
    }
  }

  function renderCheckCard(check, isStale, idx, receiptMap) {
    const step = currentProps.step;
    const card = el(doc, 'article', `solve-why-check${isStale ? ' solve-why-check--stale' : ''}`, null, {
      'data-check-id': String(check.check_id || idx), 'data-check-name': String(check.name || ''),
      ...(isStale ? { 'data-stale': 'true' } : {}),
    });
    const head = el(doc, 'div', 'solve-why-check-head');
    head.appendChild(el(doc, 'code', 'solve-why-check-name', String(check.name || 'check')));

    const outcomeKey = String(check.outcome || 'not_run').trim().toLowerCase();
    const outcomeBadge = createBadge({ ...(CHECK_OUTCOMES[outcomeKey] || CHECK_OUTCOMES.not_run), border: isStale ? BORDER_DASHED : BORDER_SOLID });
    outcomeBadge.setAttribute('data-check-outcome', outcomeKey);
    head.appendChild(outcomeBadge);
    if (isStale) {
      const staleBadge = createBadge({ text: 'Stale', icon: 'warn', tone: 'muted', border: BORDER_DASHED });
      staleBadge.setAttribute('data-check-stale', 'true');
      head.appendChild(staleBadge);
    }
    if (check.asked_by) head.appendChild(el(doc, 'span', 'solve-why-asked-by', `Asked by: ${String(check.asked_by)}`));
    card.appendChild(head);

    const reason = classifyCheckReason(check.evidence);
    if (reason) {
      card.appendChild(el(doc, 'div', `solve-why-check-reason solve-why-reason--${reason.kind}`,
        `${reason.label}: ${reason.text}`, { 'data-reason-kind': reason.kind }));
    }
    const metrics = formatEvidenceSummary(check.evidence);
    if (metrics.length) {
      const dl = el(doc, 'dl', 'solve-why-evidence-list');
      for (const p of metrics) dl.append(el(doc, 'dt', 'solve-why-evidence-key', p.key), el(doc, 'dd', 'solve-why-evidence-val', p.value));
      card.appendChild(dl);
    }

    const callId = check.receipt?.call_id || check.call_id || null;
    const receipt = callId ? receiptMap.get(String(callId)) || null : null;
    const envLine = formatReceiptEnv(receipt?.env);
    if (envLine) card.appendChild(el(doc, 'div', 'solve-why-check-env', `Engine: ${envLine}`));

    const codeKey = String(check.check_id || `${isStale ? 's' : 'c'}-${idx}`);
    const expanded = expandedCodes.has(codeKey);
    const controls = el(doc, 'div', 'solve-why-check-controls');
    const codeBtn = el(doc, 'button', 'solve-why-btn', expanded ? 'Hide code' : 'Show code', {
      type: 'button', 'data-action': 'toggle-code', 'aria-expanded': expanded ? 'true' : 'false',
    });
    codeBtn.addEventListener('click', () => {
      if (expandedCodes.has(codeKey)) expandedCodes.delete(codeKey); else expandedCodes.add(codeKey);
      render();
    });
    const rerunBtn = el(doc, 'button', 'solve-why-btn', 'Re-run', { type: 'button', 'data-action': 'recheck-one' });
    rerunBtn.addEventListener('click', () => invokeCallback(
      () => currentProps.onRerunChecks?.({ entry_id: step.entry_id, check_id: check.check_id }), handleRecheckResult,
    ));
    controls.append(codeBtn, rerunBtn);
    card.appendChild(controls);

    if (expanded) {
      card.appendChild(el(doc, 'pre', 'solve-why-code', JSON.stringify({
        call_id: receipt?.call_id ?? callId ?? null, code_sha256: receipt?.code_sha256 ?? null,
        wall_ms: receipt?.wall_ms ?? null, process: receipt?.process ?? null,
        env: receipt?.env ?? null, evidence: check.evidence ?? null,
      }, null, 2), { 'data-testid': 'solve-why-code' }));
    }
    return card;
  }

  function renderInlineForm(step) {
    if (activeForm === 'attach_source') {
      const form = el(doc, 'form', 'solve-why-form', null, { 'data-testid': 'solve-attach-source-form' });
      const srcIn = fieldEl(doc, 'input', 'solve-why-input', { name: 'source', placeholder: 'library_key, arxiv_id, or source_id' });
      const pathIn = fieldEl(doc, 'input', 'solve-why-input', { name: 'path', placeholder: 'Path (optional)' });
      const startIn = fieldEl(doc, 'input', 'solve-why-input', { name: 'start_line', type: 'number', min: '1', placeholder: 'Start line' });
      const endIn = fieldEl(doc, 'input', 'solve-why-input', { name: 'end_line', type: 'number', min: '1', placeholder: 'End line' });
      const quoteIn = fieldEl(doc, 'textarea', 'solve-why-textarea', { name: 'quote', placeholder: 'Exact quote from source' });
      form.append(srcIn, pathIn, startIn, endIn, quoteIn, el(doc, 'button', 'solve-why-btn solve-why-btn--primary', 'Attach', { type: 'submit' }));
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        const source = srcIn.value.trim();
        const quote = quoteIn.value.trim();
        const startLine = Number.parseInt(startIn.value, 10);
        const endLine = endIn.value.trim() ? Number.parseInt(endIn.value, 10) : startLine;
        if (!source || !quote || !Number.isInteger(startLine) || startLine < 1) {
          errorMessage = 'Source, positive start_line, and quote are required.'; render(); return;
        }
        const payload = {
          ...parseSourceIdentifier(source), source, start_line: startLine,
          end_line: Number.isInteger(endLine) && endLine >= startLine ? endLine : startLine, quote,
          ...(pathIn.value.trim() ? { path: pathIn.value.trim() } : {}),
        };
        invokeCallback(() => currentProps.onAttachSource?.(step, payload), () => { activeForm = null; render(); });
      });
      return form;
    }

    if (activeForm === 'add_check') {
      const form = el(doc, 'form', 'solve-why-form', null, { 'data-testid': 'solve-add-check-form' });
      const select = fieldEl(doc, 'select', 'solve-why-select', { name: 'name' });
      for (const name of SOLVE_CHECK_NAMES) select.appendChild(fieldEl(doc, 'option', '', { value: name, textContent: name }));
      const idxIn = fieldEl(doc, 'input', 'solve-why-input', { name: 'claim_index', type: 'number', min: '0', value: '0' });
      const paramsIn = fieldEl(doc, 'textarea', 'solve-why-textarea', { name: 'params', placeholder: 'Optional JSON params (e.g. {"digits": 50})' });
      form.append(select, idxIn, paramsIn, el(doc, 'button', 'solve-why-btn solve-why-btn--primary', 'Run check', { type: 'submit' }));
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        let params = {};
        if (paramsIn.value.trim()) {
          try { params = JSON.parse(paramsIn.value.trim()); }
          catch { errorMessage = 'Check params must be valid JSON.'; render(); return; }
          if (!params || typeof params !== 'object' || Array.isArray(params)) {
            errorMessage = 'Check params must be a JSON object.'; render(); return;
          }
        }
        const idx = Number.parseInt(idxIn.value, 10);
        const payload = { entry_id: step.entry_id, check: select.value, claim_index: Number.isInteger(idx) && idx >= 0 ? idx : 0, params };
        invokeCallback(() => currentProps.onAddCheck?.(step, payload), () => { activeForm = null; render(); });
      });
      return form;
    }

    if (activeForm === 'correct_step') {
      const form = el(doc, 'form', 'solve-why-form', null, { 'data-testid': 'solve-correct-form' });
      const input = fieldEl(doc, 'textarea', 'solve-why-textarea', {
        name: 'correction', placeholder: 'Describe what is wrong or how this step should be corrected',
      }, { 'data-testid': 'solve-correct-input' });
      form.append(input, el(doc, 'button', 'solve-why-btn solve-why-btn--primary', 'Submit correction', {
        type: 'submit', 'data-action': 'submit-correction',
      }));
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        const correctionText = input.value.trim();
        if (!correctionText) { errorMessage = 'Please enter a correction before submitting.'; render(); return; }
        invokeCallback(() => currentProps.onStepCorrected?.({
          sessionId: currentProps.sessionId, session: currentProps.sessionId,
          entryId: step.entry_id, entrySha256: step.entry_sha256,
          step, correctionText, text: correctionText, originalSummary: step.summary,
        }), () => { activeForm = null; render(); });
      });
      return form;
    }
    return null;
  }

  function render() {
    clearNode(root);
    const step = currentProps.step || null;
    root.setAttribute('data-entry-id', step?.entry_id ? String(step.entry_id) : '');

    // 1. Header
    const header = el(doc, 'header', 'solve-why-header');
    const titleWrap = el(doc, 'div', 'solve-why-title-wrap');
    titleWrap.appendChild(el(doc, 'code', 'solve-step-id', step?.entry_id ? String(step.entry_id) : '—'));
    if (step?.entry_type) titleWrap.appendChild(el(doc, 'span', 'solve-why-entry-type', String(step.entry_type)));
    const closeBtn = el(doc, 'button', 'solve-why-close', 'Close', { type: 'button', 'data-action': 'close-why' });
    closeBtn.addEventListener('click', () => currentProps.onClose?.());
    header.append(titleWrap, closeBtn);
    root.appendChild(header);

    if (!step) { root.appendChild(el(doc, 'p', 'solve-muted', 'No step selected.')); return; }

    if (recheckWarning) {
      root.appendChild(el(doc, 'div', 'solve-warning-banner', recheckWarning, {
        role: 'alert', 'data-testid': 'solve-recheck-warning',
      }));
    }
    if (errorMessage) root.appendChild(el(doc, 'div', 'solve-error-banner', errorMessage, { role: 'alert' }));

    // 2. Badge summary & lowering ancestors
    const badgeSection = el(doc, 'div', 'solve-why-section');
    badgeSection.appendChild(createSolveBadgeGroup(step.badge || {}, { document: doc, onSelectStep: currentProps.onSelectStep }));
    const loweringEl = renderLoweringSection(doc, step, currentProps.onSelectStep);
    if (loweringEl) badgeSection.appendChild(loweringEl);
    root.appendChild(badgeSection);

    // 3. Source quote chip
    if (step.source_ref) {
      const srcSection = el(doc, 'div', 'solve-why-section');
      srcSection.appendChild(createQuoteChip(buildQuoteSpec(step), {
        onOpen: currentProps.onOpenSource,
        onOpenPageImage: currentProps.onOpenPageImage || currentProps.onOpenSource,
      }));
      root.appendChild(srcSection);
    }

    // 4. Checks list (current & stale)
    const view = currentProps.view || {};
    const { current, stale } = partitionStepChecks(step, view.checks || [], view?.setup?.math_sha256 || null);
    const receiptMap = new Map();
    for (const r of Array.isArray(view.receipts) ? view.receipts : []) {
      if (r && r.call_id) receiptMap.set(String(r.call_id), r);
    }
    const checksSection = el(doc, 'div', 'solve-why-section solve-why-checks');
    checksSection.appendChild(el(doc, 'h4', 'solve-section-title', 'Checks'));
    if (!current.length && !stale.length) {
      checksSection.appendChild(el(doc, 'p', 'solve-muted', 'No checks recorded for this step.'));
    } else {
      if (current.length) {
        const curBox = el(doc, 'div', 'solve-why-checks-group', null, { 'data-checks-group': 'current' });
        current.forEach((chk, i) => curBox.appendChild(renderCheckCard(chk, false, i, receiptMap)));
        checksSection.appendChild(curBox);
      }
      if (stale.length) {
        const staleBox = el(doc, 'div', 'solve-why-checks-group solve-why-checks-group--stale', null, { 'data-checks-group': 'stale' });
        staleBox.appendChild(el(doc, 'h5', 'solve-why-subheading', 'Stale checks'));
        stale.forEach((chk, i) => staleBox.appendChild(renderCheckCard(chk, true, i, receiptMap)));
        checksSection.appendChild(staleBox);
      }
    }
    root.appendChild(checksSection);

    // 5. All 7 actions
    const actionsBar = el(doc, 'div', 'solve-why-actions');
    const addBtn = (action, label, onClick, extraAttrs) => {
      const btn = el(doc, 'button', 'solve-why-btn', label, { type: 'button', 'data-why-action': action, ...extraAttrs });
      if (onClick) btn.addEventListener('click', onClick);
      actionsBar.appendChild(btn);
      return btn;
    };
    const toggleForm = (name) => () => { activeForm = activeForm === name ? null : name; errorMessage = null; render(); };
    const emitAck = (kind) => () => invokeCallback(() => currentProps.onAck?.(step, {
      entry_id: step.entry_id, kind, entry_sha256: step.entry_sha256,
    }));

    addBtn('fork', 'Fork from here', () => invokeCallback(() => currentProps.onFork?.(step)));
    addBtn('attach_source', 'Attach source', toggleForm('attach_source'));
    const originLower = String(step.origin || '').toLowerCase();
    if (step.entry_type === 'assumption'
      && (step.badge?.own === 'assumed_by_model' || !['setup', 'given', 'user'].includes(originLower))) {
      addBtn('ack', 'Acknowledge assumption', emitAck('assumption'), { 'data-ack-kind': 'assumption' });
    }
    const flags = Array.isArray(step.badge?.flags) ? step.badge.flags : null;
    if (step.transcription && (!flags || flags.includes('transcription unconfirmed'))) {
      addBtn('ack', 'Confirm transcription', emitAck('transcription'), { 'data-ack-kind': 'transcription' });
    }
    addBtn('ack', 'Mark checked by you', emitAck('hand_checked'), { 'data-ack-kind': 'hand_checked' });
    addBtn('add_check', 'Add check', toggleForm('add_check'));
    const hasChecks = current.length + stale.length > 0;
    const rerunAllBtn = addBtn('rerun_checks', 'Re-run checks', hasChecks ? () => invokeCallback(
      () => currentProps.onRerunChecks?.({ entry_id: step.entry_id }), handleRecheckResult,
    ) : null);
    if (!hasChecks) {
      rerunAllBtn.disabled = true;
      rerunAllBtn.setAttribute('aria-disabled', 'true');
    }
    addBtn('correct_step', 'Correct this step', toggleForm('correct_step'));

    const canReview = currentProps.reviewEnabled === true && typeof currentProps.onReviewModel === 'function';
    const reviewBtn = addBtn('review_model', 'Review with another model', canReview
      ? () => invokeCallback(() => currentProps.onReviewModel(step)) : null);
    if (!canReview) {
      reviewBtn.disabled = true;
      reviewBtn.setAttribute('aria-disabled', 'true');
      reviewBtn.title = REVIEW_DISABLED_NOTICE;
    }

    root.appendChild(actionsBar);
    const formEl = renderInlineForm(step);
    if (formEl) root.appendChild(formEl);
  }

  render();

  return {
    element: root,
    update(nextProps = {}) {
      currentProps = { ...currentProps, ...nextProps };
      const nextId = currentProps.step?.entry_id ?? null;
      if (nextId !== lastEntryId) {
        lastEntryId = nextId; activeForm = null; recheckWarning = null; errorMessage = null; expandedCodes.clear();
      }
      render();
    },
    destroy() {
      clearNode(root);
      if (root.parentNode) root.parentNode.removeChild(root);
    },
  };
}
