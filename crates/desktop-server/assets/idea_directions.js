// Explore Directions cards & F7 hand-over buttons (package X13).
// Concrete vs collapsed Loose ideas groups, fixed card layout, Rule D3 ("suggestion (model)" on
// all free prose; only node chips count as citations), "Open in Solve" and "Investigate this paper"
// buttons, read-only child status chips, zero innerHTML, zero Rule S8 forbidden words.
import { getExploreHandoffs } from './api_idea.js';
import {
  BORDER_DASHED,
  BORDER_SOLID,
  MODEL_TEXT_SUFFIX,
  createBadge,
} from './ui_badge.js';

// Named constants for owner-accepted on-screen strings (explore.md 3, 4.1, 4.2, D2, D3; overview 3.2 row 24).
export const SUGGESTION_MODEL_LABEL = `suggestion ${MODEL_TEXT_SUFFIX}`;
export const CONCRETE_GROUP_LABEL = 'Concrete';
export const LOOSE_GROUP_LABEL = 'Loose ideas';
export const OPEN_IN_SOLVE_LABEL = 'Open in Solve';
export const INVESTIGATE_PAPER_LABEL = 'Investigate this paper';
export const CHECK_FIRST_LABEL = 'Check first';

const METHOD_LABELS = Object.freeze({
  by_hand: 'by hand',
  sympy: 'SymPy',
  numeric: 'numeric',
  literature: 'literature',
});

const RISK_KIND_LABELS = Object.freeze({
  already_done: 'already done',
  technical: 'technical',
  no_go: 'no-go',
  low_payoff: 'low payoff',
});

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

/** Index read-only outgoing hand-off child statuses by origin `from.id` (e.g. `r-1` or paper key). */
export function indexHandoffStatuses(handoffs, minDirectionSeq = 0) {
  const byFromId = new Map();
  const list = Array.isArray(handoffs?.outgoing)
    ? handoffs.outgoing
    : (Array.isArray(handoffs) ? handoffs : []);
  const floor = Number(minDirectionSeq) || 0;
  for (const item of list) {
    const sentSeq = Number(item?.sent_seq || 0);
    if (item?.target_mode === 'solve' && floor > 0 && sentSeq > 0 && sentSeq < floor) continue;
    const fromId = String(item?.from?.id || item?.from_id || '').trim();
    if (fromId) byFromId.set(fromId, item);
    if (item?.target_mode === 'investigate') {
      const pkey = String(item?.from?.paper?.key || '').trim();
      if (pkey) byFromId.set(pkey, item);
    }
  }
  return byFromId;
}

function formatMissingBasis(rawBasis) {
  const s = String(rawBasis || '').trim();
  if (s === 'no_evidence_in_scope') return 'no evidence in searched scope';
  const m = s.match(/^open_question\((q-[1-9][0-9]*)\)$/);
  if (m) return `open question (${m[1]})`;
  return s || 'basis unspecified';
}

function makeCitationNodeChip(nodeId, nodesById, onSelectNode) {
  const id = String(nodeId || '');
  const node = nodesById.get(id);
  const label = node?.label ? `${id}: ${node.label}` : id;
  const chip = createBadge({
    text: label,
    icon: 'quote',
    tone: node ? 'ok' : 'info',
    border: BORDER_SOLID,
    onClick: typeof onSelectNode === 'function' ? () => onSelectNode({ id, node }) : null,
  });
  chip.setAttribute('data-citation-chip', 'true');
  chip.setAttribute('data-node-id', id);
  return chip;
}

function makeSuggestionBlock(text) {
  const row = el('div', 'idea-direction-prose');
  row.setAttribute('data-model-suggestion', 'true');
  row.append(
    el('span', 'idea-direction-prose-text', String(text || '')),
    createBadge({ text: SUGGESTION_MODEL_LABEL, icon: 'model', tone: 'muted', border: BORDER_DASHED }),
  );
  return row;
}

function renderChildStatusChip(handoffItem, onOpenChildSession) {
  if (!handoffItem) return null;
  const childId = String(handoffItem.child_session || '');
  const label = String(
    handoffItem.child_status?.label
    || (handoffItem.target_mode === 'investigate'
      ? 'Opened in Investigate: Not run yet'
      : 'Opened in Solve: Not run yet'),
  );
  const state = String(handoffItem.child_status?.state || 'opened');
  const tone = state === 'finished' || state === 'mapped' || state === 'skeleton'
    ? 'ok'
    : (state.startsWith('paused') || state === 'finished_open_items' ? 'warn' : 'info');
  const chip = createBadge({
    text: label,
    icon: 'link',
    tone,
    border: BORDER_SOLID,
    onClick: childId && typeof onOpenChildSession === 'function'
      ? () => onOpenChildSession({ session_id: childId, mode: handoffItem.target_mode })
      : null,
  });
  chip.setAttribute('data-handoff-status', state);
  if (childId) chip.setAttribute('data-child-session', childId);
  return chip;
}

function renderDirectionCard(dir, nodesById, statusByOrigin, callbacks) {
  const id = String(dir?.id || '');
  const isConcrete = String(dir?.concreteness || 'sketch') === 'concrete';
  const card = el('article', `idea-direction-card ${isConcrete ? 'is-concrete' : 'is-sketch'}`);
  card.setAttribute('data-direction-card', id);
  card.setAttribute('data-concreteness', isConcrete ? 'concrete' : 'sketch');

  // Header: ID, title, concreteness, effort, suggestion tag, read-only child status.
  const head = el('div', 'idea-card-head');
  const titleWrap = el('div', 'idea-candidate-badges');
  titleWrap.append(
    createBadge({ text: id || 'r-?', icon: 'info', tone: 'info', border: BORDER_SOLID }),
    el('h4', 'idea-candidate-title', String(dir.title || id)),
    createBadge({ text: SUGGESTION_MODEL_LABEL, icon: 'model', tone: 'muted', border: BORDER_DASHED }),
  );
  const metaBadges = el('div', 'idea-candidate-badges');
  metaBadges.append(
    createBadge({
      text: isConcrete ? CONCRETE_GROUP_LABEL : LOOSE_GROUP_LABEL,
      icon: isConcrete ? 'check' : 'model',
      tone: isConcrete ? 'ok' : 'muted',
      border: isConcrete ? BORDER_SOLID : BORDER_DASHED,
    }),
  );
  const effort = String(dir?.first_calculation?.effort || '').trim();
  if (effort) {
    metaBadges.append(createBadge({ text: `effort: ${effort}`, icon: 'info', tone: 'neutral', border: BORDER_SOLID }));
  }
  const childStatus = statusByOrigin.get(id) || null;
  const statusChip = renderChildStatusChip(childStatus, callbacks.onOpenChildSession);
  if (statusChip) metaBadges.append(statusChip);
  head.append(titleWrap, metaBadges);
  card.append(head);

  // 1. Question (Rule D3: tagged suggestion (model)).
  const qSec = el('div', 'idea-direction-field');
  qSec.setAttribute('data-direction-field', 'question');
  qSec.append(el('span', 'idea-field-label', 'Question'), makeSuggestionBlock(dir.question || ''));
  card.append(qSec);

  // 2. Known (Rule D3: ONLY node chips count as citations; free text tagged suggestion (model)).
  const kSec = el('div', 'idea-direction-field');
  kSec.setAttribute('data-direction-field', 'known');
  kSec.append(el('span', 'idea-field-label', 'Known'));
  const knownItems = Array.isArray(dir.known) ? dir.known : [];
  if (knownItems.length === 0) {
    kSec.append(el('p', 'idea-card-meta', 'No map nodes cited.'));
  } else {
    for (const k of knownItems) {
      const kRow = el('div', 'idea-direction-item');
      const chips = el('div', 'idea-chip-row');
      for (const nid of Array.isArray(k.node_ids) ? k.node_ids : []) {
        chips.append(makeCitationNodeChip(nid, nodesById, callbacks.onSelectNode));
      }
      if (chips.children.length > 0) kRow.append(chips);
      if (k.text) kRow.append(makeSuggestionBlock(k.text));
      kSec.append(kRow);
    }
  }
  card.append(kSec);

  // 3. Missing (basis badge + text tagged suggestion (model)).
  const mSec = el('div', 'idea-direction-field');
  mSec.setAttribute('data-direction-field', 'missing');
  mSec.append(el('span', 'idea-field-label', 'Missing'));
  const missingItems = Array.isArray(dir.missing) ? dir.missing : [];
  if (missingItems.length === 0) {
    mSec.append(el('p', 'idea-card-meta', 'None listed.'));
  } else {
    for (const m of missingItems) {
      const mRow = el('div', 'idea-direction-item');
      mRow.append(
        createBadge({ text: formatMissingBasis(m.basis), icon: 'code', tone: 'info', border: BORDER_SOLID }),
        makeSuggestionBlock(m.text || ''),
      );
      mSec.append(mRow);
    }
  }
  card.append(mSec);

  // 4. First calculation + Check first.
  const fc = dir.first_calculation || {};
  const fcSec = el('div', 'idea-direction-field');
  fcSec.setAttribute('data-direction-field', 'first-calculation');
  const fcHead = el('div', 'idea-chip-row');
  fcHead.append(el('span', 'idea-field-label', 'First calculation'));
  const methodStr = METHOD_LABELS[String(fc.method || '')] || String(fc.method || '');
  if (methodStr) {
    fcHead.append(createBadge({ text: methodStr, icon: 'code', tone: 'info', border: BORDER_SOLID }));
  }
  fcSec.append(fcHead);
  if (Array.isArray(fc.inputs) && fc.inputs.length > 0) {
    const inRow = el('div', 'idea-chip-row');
    inRow.append(el('span', 'idea-card-meta', 'Inputs:'));
    for (const nid of fc.inputs) inRow.append(makeCitationNodeChip(nid, nodesById, callbacks.onSelectNode));
    fcSec.append(inRow);
  }
  if (fc.setup) fcSec.append(makeSuggestionBlock(fc.setup));
  if (fc.expected_form) {
    const ef = el('div', 'idea-direction-item');
    ef.append(el('span', 'idea-card-meta', 'Expected form:'), makeSuggestionBlock(fc.expected_form));
    fcSec.append(ef);
  }
  const checkBox = el('div', 'idea-preview-box');
  checkBox.setAttribute('data-direction-field', 'check-first');
  const checkHead = el('div', 'idea-chip-row');
  checkHead.append(el('strong', 'idea-field-label', CHECK_FIRST_LABEL));
  if (fc.check?.reproduces) {
    checkHead.append(makeCitationNodeChip(fc.check.reproduces, nodesById, callbacks.onSelectNode));
  }
  checkBox.append(checkHead);
  checkBox.append(fc.check?.text ? makeSuggestionBlock(fc.check.text) : el('span', 'idea-card-meta', 'No check specified.'));
  fcSec.append(checkBox);
  card.append(fcSec);

  // 5. Risks.
  const rSec = el('div', 'idea-direction-field');
  rSec.setAttribute('data-direction-field', 'risks');
  rSec.append(el('span', 'idea-field-label', 'Risks'));
  const risks = Array.isArray(dir.risks) ? dir.risks : [];
  if (risks.length === 0) {
    rSec.append(el('p', 'idea-card-meta', 'No risks listed.'));
  } else {
    for (const r of risks) {
      const rRow = el('div', 'idea-direction-item');
      const rHead = el('div', 'idea-chip-row');
      rHead.append(createBadge({
        text: RISK_KIND_LABELS[String(r.kind || '')] || String(r.kind || 'risk'),
        icon: 'warn',
        tone: 'warn',
        border: BORDER_DASHED,
      }));
      for (const nid of Array.isArray(r.node_ids) ? r.node_ids : []) {
        rHead.append(makeCitationNodeChip(nid, nodesById, callbacks.onSelectNode));
      }
      rRow.append(rHead, makeSuggestionBlock(r.text || ''));
      rSec.append(rRow);
    }
  }
  card.append(rSec);

  // 6. Stop if.
  const sSec = el('div', 'idea-direction-field');
  sSec.setAttribute('data-direction-field', 'stop-if');
  sSec.append(
    el('span', 'idea-field-label', 'Stop if'),
    dir.stop_if ? makeSuggestionBlock(dir.stop_if) : el('p', 'idea-card-meta', 'No stop condition listed.'),
  );
  card.append(sSec);

  if (!isConcrete && Array.isArray(dir.concreteness_reasons) && dir.concreteness_reasons.length > 0) {
    const why = el('p', 'idea-card-meta', `Classified as loose idea (by code): ${dir.concreteness_reasons.join('; ')}`);
    why.setAttribute('data-concreteness-reasons', 'true');
    card.append(why);
  }

  // 7. Hand-over buttons (F7: Open in Solve + Investigate this paper).
  const actions = el('div', 'idea-controls-row idea-direction-actions');
  const solveBtn = el('button', 'pill-btn sm primary', OPEN_IN_SOLVE_LABEL);
  solveBtn.type = 'button';
  solveBtn.setAttribute('data-action', 'open-in-solve');
  solveBtn.setAttribute('data-direction-id', id);
  solveBtn.addEventListener('click', () => callbacks.onOpenInSolve?.({ direction_id: id, direction: dir }));
  actions.append(solveBtn);

  const sourceKeys = Array.isArray(dir?.solve_setup?.source_keys) ? dir.solve_setup.source_keys : [];
  for (const pkey of sourceKeys) {
    const invBtn = el('button', 'pill-btn sm', `${INVESTIGATE_PAPER_LABEL} (${pkey})`);
    invBtn.type = 'button';
    invBtn.setAttribute('data-action', 'investigate-paper');
    invBtn.setAttribute('data-paper-key', String(pkey));
    invBtn.addEventListener('click', () => callbacks.onInvestigatePaper?.({ paper_key: String(pkey) }));
    actions.append(invBtn);
    const pStatus = statusByOrigin.get(String(pkey));
    const pChip = renderChildStatusChip(pStatus, callbacks.onOpenChildSession);
    if (pChip) actions.append(pChip);
  }
  card.append(actions);
  return card;
}

/**
 * Render the X13 Directions section: Concrete group (expanded) + Loose ideas group (collapsed
 * `<details>`), fixed card layout, Rule D3 model-suggestion tagging, and F7 hand-over buttons.
 */
export function renderIdeaDirectionsSection(view, callbacks = {}) {
  const section = el('section', 'idea-where-panel idea-directions-section');
  section.setAttribute('data-idea-section', 'directions');

  const directions = Array.isArray(view?.directions) ? view.directions : [];
  const head = el('div', 'idea-card-head');
  head.append(
    el('h3', 'idea-card-title', 'Directions'),
    el('span', 'idea-card-meta', `${directions.length} direction(s)`),
  );
  section.append(head);

  if (directions.length === 0) {
    section.append(el(
      'p',
      'idea-card-meta',
      'No directions recorded yet. Run Directions after mapping to propose concrete first calculations.',
    ));
    return section;
  }

  const nodesById = new Map(
    (Array.isArray(view?.map?.nodes) ? view.map.nodes : []).map((n) => [String(n.id || ''), n]),
  );
  const statusByOrigin = indexHandoffStatuses(
    callbacks.handoffs ?? view?.handoffs,
    Number(view?.map?.map_seq || view?.statement?.seq || 0),
  );

  const concrete = directions.filter((d) => String(d?.concreteness || '') === 'concrete');
  const loose = directions.filter((d) => String(d?.concreteness || '') !== 'concrete');

  const concreteBox = el('div', 'idea-directions-group');
  concreteBox.setAttribute('data-directions-group', 'concrete');
  concreteBox.append(el('h4', 'idea-field-label', `${CONCRETE_GROUP_LABEL} (${concrete.length})`));
  if (concrete.length === 0) {
    concreteBox.append(el('p', 'idea-card-meta', 'No concrete directions yet; inspect Loose ideas below.'));
  } else {
    for (const d of concrete) concreteBox.append(renderDirectionCard(d, nodesById, statusByOrigin, callbacks));
  }
  section.append(concreteBox);

  if (loose.length > 0) {
    const looseDetails = el('details', 'idea-directions-loose');
    looseDetails.setAttribute('data-directions-group', 'loose');
    // Collapsed by default (`explore.md` 3 & Rule D2).
    const summary = el('summary', 'idea-field-label', `${LOOSE_GROUP_LABEL} (${loose.length})`);
    looseDetails.append(summary);
    for (const d of loose) looseDetails.append(renderDirectionCard(d, nodesById, statusByOrigin, callbacks));
    section.append(looseDetails);
  }

  return section;
}

/** Helper used by `idea.js` to fetch read-only F7 child statuses without inflating `idea.js`. */
export async function fetchExploreHandoffsSafe(sessionId, fetchFn = getExploreHandoffs) {
  if (!sessionId || typeof fetchFn !== 'function') return null;
  try {
    return await fetchFn(sessionId);
  } catch {
    return null;
  }
}

/** Default navigation helper for opening a Solve or Investigate hand-off child session. */
export async function openChildSessionDefault(info) {
  const id = info?.session_id || info?.id;
  if (!id) return;
  const rawMode = String(info?.mode || 'solve');
  const mode = rawMode === 'investigate' ? 'investigation' : rawMode;
  const title = String(info?.title || (mode === 'investigation' ? 'Investigate' : 'Solve'));
  const rail = typeof globalThis !== 'undefined' ? globalThis.__reSearchSessionRail : null;
  if (rail && typeof rail.upsertSession === 'function') {
    await rail.upsertSession({ id, title, mode, archived: false }, { select: true });
    return;
  }
  const stages = typeof globalThis !== 'undefined' ? globalThis.__reSearchModeStages : null;
  if (stages && typeof stages.syncFromRail === 'function') {
    await stages.syncFromRail(id, [{ id, mode }]);
  }
}

/** Run an Explore F7 hand-off op, refresh read-only child statuses, and open the new child session. */
export async function executeExploreHandoff({
  sessionId,
  payload,
  invokeOp,
  getExploreHandoffsFn,
  onHandoffsUpdated,
  onOpenChildSession,
}) {
  const res = await invokeOp('handoff', payload);
  if (!res) return null;
  const nextHandoffs = await fetchExploreHandoffsSafe(sessionId, getExploreHandoffsFn);
  if (nextHandoffs && typeof onHandoffsUpdated === 'function') onHandoffsUpdated(nextHandoffs);
  if (res.session_id) {
    const opener = typeof onOpenChildSession === 'function' ? onOpenChildSession : openChildSessionDefault;
    await opener({
      session_id: res.session_id,
      mode: res.mode,
      title: res.prefill?.title || res.receipt?.packet?.title || '',
    });
  }
  return res;
}

