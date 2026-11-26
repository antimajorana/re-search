// Shared mode-neutral approval card component (package S5a; single owner for Solve S5,
// Explore X7, and Library sources F13b — `00-overview.md` 3.1 row 13, `solve.md` D5).
//
// Renders both existing grant stores honestly:
// 1. `session_capability` (`desktop-server/src/session_lifecycle.rs`, per session & effect)
// 2. `worker_session` (`tools/src/router.rs`, per caller, tool, def_hash, policy & worker epoch)
//
// Clicking "Allow listed" dispatches each listed grant SEPARATELY through the existing approver
// callback — zero auto-approve, zero new approval scope, and a clear note that a worker restart
// asks again. Built exclusively with DOM APIs (`createElement` + `textContent`; zero `innerHTML`).

// Single-place named constants governed by `solve.md` D5 and `00-overview.md` 3.1 row 13.
export const GRANT_KIND_SESSION_CAPABILITY = 'session_capability';
export const GRANT_KIND_WORKER_SESSION = 'worker_session';
export const ALLOW_LISTED_LABEL = 'Allow listed';
export const ALLOW_ONE_LABEL = 'Allow';
export const DENY_ONE_LABEL = 'Deny';
export const WORKER_RESTART_NOTE = 'A worker restart asks again.';
export const NO_AUTO_APPROVE_NOTE =
  'Each grant is recorded separately through the approval flow; nothing is auto-approved.';

const SCOPE_LABELS = Object.freeze({
  [GRANT_KIND_SESSION_CAPABILITY]: 'Session capability',
  [GRANT_KIND_WORKER_SESSION]: 'Worker session',
});

const STATUS_LABELS = Object.freeze({
  granted: 'Granted',
  denied: 'Denied',
  pending: 'Pending approval',
  required: 'Needs approval',
  error: 'Not granted',
});

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

/**
 * Normalize a raw grant descriptor from Solve, Explore, or Library into a strict,
 * fail-closed shape. Unknown kinds default to `worker_session` (narrowest scope).
 */
export function normalizeGrantDescriptor(raw, idx = 0) {
  const src = raw && typeof raw === 'object' ? raw : {};
  const rawKind = String(src.kind || '').trim();
  const kind = rawKind === GRANT_KIND_SESSION_CAPABILITY
    ? GRANT_KIND_SESSION_CAPABILITY
    : GRANT_KIND_WORKER_SESSION;
  const granted = src.granted === true;
  const status = granted
    ? 'granted'
    : (src.status === 'denied' || src.status === 'pending' || src.status === 'error'
      ? src.status
      : (src.call_id ? 'pending' : 'required'));

  return {
    id: String(src.id || `grant-${idx}`),
    kind,
    label: String(src.label || src.tool_id || src.effect || `Grant ${idx + 1}`),
    caller: src.caller ? String(src.caller) : null,
    tool_id: src.tool_id ? String(src.tool_id) : null,
    effect: src.effect ? String(src.effect) : null,
    epoch: src.epoch ?? null,
    call_id: src.call_id ? String(src.call_id) : null,
    digest: src.digest ? String(src.digest) : null,
    optional: src.optional === true,
    granted,
    status,
    restart_asks_again: kind === GRANT_KIND_WORKER_SESSION || src.restart_asks_again === true,
    note: src.note ? String(src.note) : (kind === GRANT_KIND_WORKER_SESSION ? WORKER_RESTART_NOTE : ''),
    error: src.error ? String(src.error) : '',
  };
}

function describeGrantMeta(grant) {
  const parts = [SCOPE_LABELS[grant.kind] || SCOPE_LABELS[GRANT_KIND_WORKER_SESSION]];
  if (grant.kind === GRANT_KIND_WORKER_SESSION) {
    if (grant.caller) parts.push(`caller: ${grant.caller}`);
    if (grant.tool_id) parts.push(`tool: ${grant.tool_id}`);
    if (grant.epoch !== null && grant.epoch !== undefined) parts.push(`epoch: ${grant.epoch}`);
  } else if (grant.effect) {
    parts.push(`effect: ${grant.effect}`);
  }
  if (grant.optional) parts.push('optional');
  return parts.join(' · ');
}

/**
 * Mount a mode-neutral approval checklist card into `host`.
 *
 * `options`:
 * - `title`: card heading (default `'Approvals for this run'`)
 * - `grants`: array of grant descriptors (both `session_capability` and `worker_session`)
 * - `preTurnSupported`: boolean indicating whether pre-turn prompting is active (default `true`)
 * - `onAllowGrant(grant, index)`: async/sync callback dispatching ONE grant through the real
 *   approver (`POST /api/sessions/{id}/approvals/{call_id}` or first-turn approval hook).
 *   Must return `{ approved: true }` or `true` to mark the grant granted.
 * - `onDenyGrant(grant, index)`: optional callback when the user clicks Deny on a row.
 * - `onChange(grants)`: optional listener notified after any grant status update.
 */
export function createApprovalCard(host, options = {}) {
  if (!host) throw new Error('createApprovalCard requires a host element');

  const opts = {
    title: options.title || 'Approvals for this run',
    preTurnSupported: options.preTurnSupported !== false,
    onAllowGrant: typeof options.onAllowGrant === 'function' ? options.onAllowGrant : null,
    onDenyGrant: typeof options.onDenyGrant === 'function' ? options.onDenyGrant : null,
    onChange: typeof options.onChange === 'function' ? options.onChange : null,
  };

  let grants = (Array.isArray(options.grants) ? options.grants : []).map(normalizeGrantDescriptor);
  let busy = false;
  let statusMessage = '';

  const root = el('section', 'approval-card');
  root.setAttribute('data-component', 'approval-card');
  root.setAttribute('aria-label', opts.title);

  function notifyChange() {
    if (opts.onChange) opts.onChange(grants.map((g) => ({ ...g })));
  }

  async function dispatchSingleAllow(idx) {
    const grant = grants[idx];
    if (!grant || grant.granted) return { id: grant?.id, approved: Boolean(grant?.granted) };
    const grantId = grant.id;
    const resolveLive = () => grants.find((g) => g.id === grantId) || grant;
    if (!opts.onAllowGrant) {
      const live = resolveLive();
      live.status = 'pending';
      live.error = 'Awaiting active approver prompt.';
      return { id: grantId, approved: false };
    }
    try {
      // Dispatch strictly one grant descriptor at a time — never a bulk/wildcard scope.
      const res = await opts.onAllowGrant({ ...grant }, idx);
      const live = resolveLive();
      const approved = res === true || (res && typeof res === 'object' && res.approved === true);
      if (approved) {
        live.granted = true;
        live.status = 'granted';
        live.error = '';
      } else {
        live.granted = false;
        live.status = res && typeof res === 'object' && res.pending ? 'pending' : 'denied';
        live.error = res && typeof res === 'object' && res.reason ? String(res.reason) : '';
      }
      return { id: grantId, approved };
    } catch (err) {
      const live = resolveLive();
      live.granted = false;
      live.status = 'error';
      live.error = String(err?.message || err || 'Approval request failed');
      return { id: grantId, approved: false, error: live.error };
    }
  }

  async function dispatchSingleDeny(idx) {
    const grant = grants[idx];
    if (!grant) return;
    grant.granted = false;
    grant.status = 'denied';
    grant.error = '';
    if (opts.onDenyGrant) {
      try {
        await opts.onDenyGrant({ ...grant }, idx);
      } catch (err) {
        grant.error = String(err?.message || err || 'Deny failed');
      }
    }
    notifyChange();
    render();
  }

  async function allowListed() {
    if (busy) return [];
    busy = true;
    statusMessage = '';
    render();
    const results = [];
    // Snapshot IDs before awaiting so a mid-loop update() cannot skip or duplicate grants.
    const targetIds = grants.filter((g) => !g.granted).map((g) => g.id);
    try {
      for (const id of targetIds) {
        const idx = grants.findIndex((g) => g.id === id);
        if (idx < 0 || grants[idx].granted) continue;
        // Sequential per-grant dispatch through the existing approver (solve.md D5).
        const outcome = await dispatchSingleAllow(idx);
        results.push(outcome);
      }
      const grantedCount = results.filter((r) => r.approved).length;
      statusMessage = results.length === 0
        ? 'All listed grants are already active.'
        : `Dispatched ${results.length} separate approval request${results.length === 1 ? '' : 's'} (${grantedCount} approved).`;
    } finally {
      busy = false;
      notifyChange();
      render();
    }
    return results;
  }

  function render() {
    root.replaceChildren();

    const header = el('div', 'approval-card-head');
    header.append(el('h4', 'approval-card-title', opts.title));

    const remaining = grants.filter((g) => !g.granted);
    const allowAllBtn = el('button', 'pill-btn sm primary approval-allow-listed', ALLOW_LISTED_LABEL);
    allowAllBtn.type = 'button';
    allowAllBtn.setAttribute('data-action', 'allow-listed');
    allowAllBtn.disabled = busy || remaining.length === 0;
    allowAllBtn.addEventListener('click', () => { allowListed(); });
    header.append(allowAllBtn);
    root.append(header);

    const hasWorkerGrant = grants.some((g) => g.kind === GRANT_KIND_WORKER_SESSION);
    const noteText = hasWorkerGrant
      ? `${NO_AUTO_APPROVE_NOTE} ${WORKER_RESTART_NOTE}`
      : NO_AUTO_APPROVE_NOTE;
    const noteEl = el('p', 'approval-card-note', noteText);
    noteEl.setAttribute('data-role', 'approval-note');
    root.append(noteEl);

    if (!opts.preTurnSupported) {
      const preNote = el(
        'p',
        'approval-card-note',
        'Approvals are prompted when the first turn starts; nothing is auto-approved.',
      );
      preNote.setAttribute('data-role', 'preturn-fallback-note');
      root.append(preNote);
    }

    const list = el('ul', 'approval-grant-list');
    if (grants.length === 0) {
      list.append(el('li', 'approval-row approval-empty', 'No approvals required for this action.'));
    } else {
      grants.forEach((grant, idx) => {
        const row = el('li', `approval-row approval-status-${grant.status}`);
        row.setAttribute('data-grant-id', grant.id);
        row.setAttribute('data-grant-kind', grant.kind);
        row.setAttribute('data-grant-status', grant.status);

        const main = el('div', 'approval-row-main');
        main.append(el('div', 'approval-row-label', grant.label));
        main.append(el('div', 'approval-row-meta', describeGrantMeta(grant)));
        if (grant.note) {
          main.append(el('div', 'approval-row-subnote', grant.note));
        }
        if (grant.error) {
          main.append(el('div', 'approval-row-error', grant.error));
        }

        const actions = el('div', 'approval-row-actions');
        const pill = el('span', `mini-pill approval-pill-${grant.status}`, STATUS_LABELS[grant.status] || grant.status);
        actions.append(pill);

        if (!grant.granted) {
          const allowBtn = el('button', 'pill-btn sm', ALLOW_ONE_LABEL);
          allowBtn.type = 'button';
          allowBtn.setAttribute('data-action', 'allow-one');
          allowBtn.setAttribute('data-grant-id', grant.id);
          allowBtn.disabled = busy;
          allowBtn.addEventListener('click', async () => {
            if (busy) return;
            busy = true;
            render();
            try {
              await dispatchSingleAllow(idx);
            } finally {
              busy = false;
              notifyChange();
              render();
            }
          });
          actions.append(allowBtn);

          if (opts.onDenyGrant) {
            const denyBtn = el('button', 'pill-btn sm', DENY_ONE_LABEL);
            denyBtn.type = 'button';
            denyBtn.setAttribute('data-action', 'deny-one');
            denyBtn.setAttribute('data-grant-id', grant.id);
            denyBtn.disabled = busy;
            denyBtn.addEventListener('click', () => { dispatchSingleDeny(idx); });
            actions.append(denyBtn);
          }
        }

        row.append(main, actions);
        list.append(row);
      });
    }
    root.append(list);

    if (statusMessage) {
      const msg = el('div', 'approval-card-status', statusMessage);
      msg.setAttribute('data-role', 'approval-status');
      root.append(msg);
    }
  }

  host.replaceChildren(root);
  render();

  return {
    element: root,
    allowListed,
    getGrants: () => grants.map((g) => ({ ...g })),
    update(nextGrants, nextOpts = {}) {
      if (Array.isArray(nextGrants)) {
        grants = nextGrants.map(normalizeGrantDescriptor);
      }
      if (typeof nextOpts.preTurnSupported === 'boolean') {
        opts.preTurnSupported = nextOpts.preTurnSupported;
      }
      if (!busy) statusMessage = '';
      render();
    },
  };
}
