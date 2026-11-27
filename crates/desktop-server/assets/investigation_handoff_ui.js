// Investigate V16 Cross-Mode Hand-Off UI ("Derive in Solve", "Has this been extended?",
// "Search wider in Explore", and read-only child status chips).
// Zero innerHTML writes; zero parent-ledger mutations on status read; F12 wording enforced.
import * as api from './api.js';

export const HANDOFF_ACTION_BUTTONS = Object.freeze([
  Object.freeze({
    action: 'derive_solve',
    label: 'Derive in Solve',
    title: 'Send statement, closure, symbols, and notes to Solve without model calls.',
  }),
  Object.freeze({
    action: 'extend_explore',
    label: 'Has this been extended?',
    title: 'Ask Explore whether this result has been extended, with origin exclusion and bibliography seeds.',
  }),
  Object.freeze({
    action: 'prior_work_explore',
    label: 'Search wider in Explore',
    title: 'Search earlier literature in Explore, with origin exclusion and bibliography seeds.',
  }),
]);

// F12 invariant (`00-overview.md` §2.9, `investigate.md` §3): never display these words on badges/chips.
export const FORBIDDEN_HANDOFF_WORDS_RE = /\b(?:new|novel|novelty|verified|proved|unexplored)\b/i;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

/**
 * Guard every displayed badge or status chip label against forbidden F12 terms.
 */
export function sanitizeHandoffBadgeText(raw, fallback = 'Opened in Explore: status recorded') {
  const text = String(raw || '').trim();
  if (!text || FORBIDDEN_HANDOFF_WORDS_RE.test(text)) {
    return fallback;
  }
  return text;
}

function normalizeStatusItem(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const childSession = String(raw.child_session || '').trim();
  if (!childSession) return null;
  const targetMode = String(raw.target_mode || raw.child_status?.mode || '').trim().toLowerCase();
  const parentNodeId = String(raw.parent_node_id || raw.from?.id || '').trim();
  const actionKind = String(
    raw.action_kind || (targetMode === 'solve' ? 'derive_solve' : 'extend_explore'),
  ).trim();
  const rawLabel = raw.label || raw.child_status?.label || `Opened in ${targetMode || 'child'}`;
  return {
    child_session: childSession,
    parent_node_id: parentNodeId,
    target_mode: targetMode,
    action_kind: actionKind,
    title: String(raw.title || '').trim(),
    state: String(raw.state || raw.child_status?.state || 'opened').trim(),
    label: sanitizeHandoffBadgeText(
      rawLabel,
      targetMode === 'solve' ? 'Opened in Solve: status recorded' : 'Opened in Explore: status recorded',
    ),
    sent_seq: Number(raw.sent_seq || 0),
  };
}

export function extractNodeHandoffStatuses(sourceData, nodeId) {
  const target = String(nodeId || '').trim();
  if (!target || !sourceData) return [];
  const rawList = Array.isArray(sourceData.child_statuses)
    ? sourceData.child_statuses
    : Array.isArray(sourceData.handoff_statuses)
      ? sourceData.handoff_statuses
      : Array.isArray(sourceData.outgoing)
        ? sourceData.outgoing
        : [];
  const dedup = new Map();
  for (const item of rawList) {
    const norm = normalizeStatusItem(item);
    if (!norm || norm.parent_node_id !== target) continue;
    dedup.set(norm.child_session, norm);
  }
  return Array.from(dedup.values()).sort((a, b) => a.sent_seq - b.sent_seq);
}

/**
 * Enable the right half-badge slot (`[data-v16-slot="prior-work-explore"]`) on an Investigate node
 * card so clicking it starts `"Search wider in Explore"` and reflects any linked child status
 * without violating F12 wording rules.
 */
export function wirePriorWorkHalfBadge(badgeRoot, onTriggerPriorWork, nodeStatuses = []) {
  if (!badgeRoot || typeof badgeRoot.querySelector !== 'function') return null;
  const rightHalf = badgeRoot.querySelector('[data-v16-slot="prior-work-explore"]')
    || badgeRoot.querySelector('[data-half="right"]');
  if (!rightHalf) return null;

  rightHalf.setAttribute('data-v16-slot', 'prior-work-explore');
  if (typeof rightHalf.removeAttribute === 'function') {
    rightHalf.removeAttribute('aria-disabled');
  } else {
    rightHalf.setAttribute('aria-disabled', 'false');
  }
  rightHalf.disabled = false;
  rightHalf.classList?.remove('inv-v7-right-placeholder');
  rightHalf.classList?.add('inv-v16-right-active');
  rightHalf.title = 'Search earlier literature in Explore (origin paper excluded).';

  const priorWork = Array.isArray(nodeStatuses)
    ? [...nodeStatuses].reverse().find((s) => s.action_kind === 'prior_work_explore')
    : null;
  if (priorWork) {
    rightHalf.textContent = sanitizeHandoffBadgeText(
      priorWork.label,
      'Earlier work: search wider in Explore',
    );
  }

  if (typeof onTriggerPriorWork === 'function') {
    rightHalf.addEventListener('click', (ev) => {
      ev?.stopPropagation?.();
      void onTriggerPriorWork();
    });
  }
  return rightHalf;
}

/**
 * Mount V16 outgoing hand-off buttons, right half-badge activation, and read-only child status
 * chips into a node card's `.inv-node-handoff-slot` and `.inv-v7-badges` host.
 */
export function mountInvestigationNodeHandoff(slotEl, mv, node, opts = {}) {
  if (!slotEl || !node?.id) return null;
  const apiMod = opts.api || api;
  const sessionId = opts.sessionId || mv?.session_id || '';
  const initialStatuses = extractNodeHandoffStatuses(
    { handoff_statuses: opts.statuses || mv?.handoff_statuses || [] },
    node.id,
  );
  let currentStatuses = initialStatuses.slice();
  let busy = false;
  let disposed = false;

  const wrap = el('div', 'inv-v16-handoff');
  const actionsRow = el('div', 'inv-v16-actions');
  const statusEl = el('div', 'inv-v16-feedback');
  statusEl.setAttribute('role', 'status');
  statusEl.hidden = true;
  const chipsRow = el('div', 'inv-v16-status-chips');

  const buttons = new Map();

  function setBusy(nextBusy, message = '') {
    busy = nextBusy;
    for (const btn of buttons.values()) {
      btn.disabled = nextBusy;
    }
    if (message) {
      statusEl.hidden = false;
      statusEl.textContent = String(message);
    } else {
      statusEl.hidden = true;
      statusEl.textContent = '';
    }
  }

  function renderChips() {
    chipsRow.replaceChildren();
    chipsRow.hidden = currentStatuses.length === 0;
    for (const st of currentStatuses) {
      const chip = el('button', 'inv-v16-status-chip', sanitizeHandoffBadgeText(st.label));
      chip.type = 'button';
      chip.setAttribute('data-child-session', st.child_session);
      chip.setAttribute('data-handoff-action', st.action_kind);
      chip.title = st.title ? `${st.title} — click to open child session` : 'Click to open child session';
      chip.addEventListener('click', () => {
        if (typeof opts.openSession === 'function') {
          void opts.openSession(st.child_session);
        } else {
          opts.showStatus?.(`Child session ready: ${st.child_session}`);
        }
      });
      chipsRow.appendChild(chip);
    }
  }

  async function trigger(actionKind) {
    if (busy) return null;
    if (!sessionId) {
      opts.showStatus?.('Open a saved Investigation session before sending a hand-off.');
      return null;
    }
    const buildFn = opts.buildPacket || apiMod.buildInvestigationHandoffPacket;
    const createFn = opts.createHandoff || apiMod.createHandoff;
    if (typeof buildFn !== 'function' || typeof createFn !== 'function') {
      opts.showStatus?.('Hand-off client API is unavailable.');
      return null;
    }
    setBusy(true, 'Preparing hand-off packet…');
    try {
      const req = await buildFn(sessionId, { action: actionKind, tag: node.id });
      const created = await createFn(sessionId, req);
      const childId = created?.session_id || created?.receipt?.child_session || '';
      const modeName = String(created?.mode || (actionKind === 'derive_solve' ? 'solve' : 'explore'));
      const defaultLabel = modeName === 'solve'
        ? 'Opened in Solve: Not run yet'
        : 'Opened in Explore: not finished';
      if (childId) {
        const added = normalizeStatusItem({
          child_session: childId,
          parent_node_id: node.id,
          target_mode: modeName,
          action_kind: actionKind,
          title: created?.receipt?.packet?.title || req?.packet?.title || '',
          state: 'opened',
          label: defaultLabel,
          sent_seq: currentStatuses.length + 1,
        });
        if (added) {
          currentStatuses = currentStatuses.filter((s) => s.child_session !== added.child_session);
          currentStatuses.push(added);
          renderChips();
          if (opts.badgeRoot) {
            wirePriorWorkHalfBadge(opts.badgeRoot, null, currentStatuses);
          }
        }
      }
      setBusy(false, '');
      const modeTitle = modeName === 'solve' ? 'Solve' : 'Explore';
      opts.showStatus?.(childId ? `Opened ${modeTitle} child session (${childId}).` : `Hand-off sent to ${modeTitle}.`);
      if (childId && typeof opts.openSession === 'function') {
        await opts.openSession(childId);
      } else if (typeof opts.refresh === 'function') {
        await opts.refresh(sessionId);
      }
      return created;
    } catch (err) {
      setBusy(false, err?.message || 'Could not create hand-off session.');
      opts.showStatus?.(err?.message || 'Could not create hand-off session.');
      return null;
    }
  }

  for (const spec of HANDOFF_ACTION_BUTTONS) {
    const btn = el('button', 'pill-btn sm inv-v16-action-btn', spec.label);
    btn.type = 'button';
    btn.title = spec.title;
    btn.setAttribute('data-handoff-action', spec.action);
    btn.addEventListener('click', () => { void trigger(spec.action); });
    buttons.set(spec.action, btn);
    actionsRow.appendChild(btn);
  }

  renderChips();
  wrap.append(actionsRow, chipsRow, statusEl);
  slotEl.replaceChildren(wrap);

  if (opts.badgeRoot) {
    wirePriorWorkHalfBadge(opts.badgeRoot, () => trigger('prior_work_explore'), currentStatuses);
  }

  // Poll-free single live read-only status refresh when mounted with an active sessionId.
  const getHandoffsFn = opts.getHandoffs || (sessionId ? apiMod.getHandoffs : null);
  if (sessionId && typeof getHandoffsFn === 'function' && !opts.skipLiveFetch) {
    Promise.resolve(getHandoffsFn(sessionId)).then((view) => {
      if (disposed || !view) return;
      const fresh = extractNodeHandoffStatuses(view, node.id);
      if (fresh.length > 0) {
        currentStatuses = fresh;
        renderChips();
        if (opts.badgeRoot) {
          wirePriorWorkHalfBadge(opts.badgeRoot, null, currentStatuses);
        }
      }
    }).catch(() => { /* Keep cached mapview statuses on transient read error. */ });
  }

  return {
    element: wrap,
    trigger,
    getStatuses: () => currentStatuses.slice(),
    dispose: () => { disposed = true; },
  };
}
