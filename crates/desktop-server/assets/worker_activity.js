// On-demand worker activity inspector & reader-control panel (F9b Core, overview §3.4).
// Zero innerHTML writes: every untrusted string (model output, errors, roles, item IDs) goes
// through textContent. Distinguishes "unknown" token usage from 0 and flags missing artifacts.
import { createBadge } from './ui_badge.js';

export const WORKER_SUMMARY_UNVERIFIED_LABEL = '(worker model summary — unverified)';
export const UNKNOWN_TOKENS_LABEL = 'unknown';

export const RUN_CONTROL_ACTIONS = Object.freeze([
  'focus_note',
  'pin_worker_item',
  'dismiss_worker_item',
  'pause_after_stage',
  'next_run_roles',
  'budget_delta',
]);

export function formatTokenCount(val) {
  if (val === null || val === undefined) return UNKNOWN_TOKENS_LABEL;
  const num = Number(val);
  if (!Number.isFinite(num) || num < 0) return UNKNOWN_TOKENS_LABEL;
  return String(Math.floor(num));
}

export function formatUsageSummary(usage) {
  if (!usage || typeof usage !== 'object') return UNKNOWN_TOKENS_LABEL;
  const calls = Math.max(0, Math.floor(Number(usage.calls ?? 1) || 0));
  const unknownCalls = Math.max(0, Math.floor(Number(usage.unknown_usage_calls ?? 0) || 0));
  const inp = formatTokenCount(usage.input_tokens);
  const out = formatTokenCount(usage.output_tokens);

  if (inp === UNKNOWN_TOKENS_LABEL && out === UNKNOWN_TOKENS_LABEL) {
    if (unknownCalls > 0) {
      return `${UNKNOWN_TOKENS_LABEL} (${unknownCalls} call${unknownCalls === 1 ? '' : 's'} without usage)`;
    }
    return UNKNOWN_TOKENS_LABEL;
  }
  const base = `${inp} in / ${out} out`;
  if (unknownCalls > 0) {
    return `${base} + ${unknownCalls} call${unknownCalls === 1 ? '' : 's'} without usage`;
  }
  return base;
}

function dispositionTone(disposition) {
  switch (String(disposition || '').toLowerCase()) {
    case 'accepted': return { tone: 'ok', icon: 'check', text: 'Accepted' };
    case 'rejected': return { tone: 'err', icon: 'error', text: 'Rejected' };
    case 'partial': return { tone: 'warn', icon: 'warn', text: 'Partial' };
    default: return { tone: 'info', icon: 'running', text: 'Pending' };
  }
}

function roleStatusTone(status) {
  switch (String(status || '').toLowerCase()) {
    case 'pinned': return { tone: 'ok', icon: 'check', text: 'Pinned' };
    case 'unavailable': return { tone: 'err', icon: 'warn', text: 'Unavailable' };
    default: return { tone: 'muted', icon: 'pause', text: 'Disabled' };
  }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function button(label, className, onClick, attrs = {}) {
  const btn = el('button', className, label);
  btn.type = 'button';
  for (const [k, v] of Object.entries(attrs)) {
    btn.setAttribute(k, String(v));
  }
  if (typeof onClick === 'function') btn.addEventListener('click', onClick);
  return btn;
}

function renderRolesStrip(rolesMap) {
  const wrap = el('section', 'wa-roles-strip');
  wrap.setAttribute('aria-label', 'Model roles');
  const entries = rolesMap && typeof rolesMap === 'object' ? Object.entries(rolesMap) : [];
  if (entries.length === 0) {
    wrap.appendChild(el('p', 'wa-empty', 'No pinned role assignments yet.'));
    return wrap;
  }
  for (const [roleName, info] of entries) {
    const chip = el('div', 'wa-role-chip');
    chip.setAttribute('data-role', roleName);
    const meta = roleStatusTone(info?.status);
    const detail = info?.status === 'pinned'
      ? `${info.provider || '?'}/${info.model || '?'} (${info.routing_reason || 'pinned'})`
      : (info?.error || info?.reason || meta.text);
    chip.appendChild(createBadge({
      text: `${roleName}: ${meta.text}`,
      secondaryText: detail,
      icon: meta.icon,
      tone: meta.tone,
    }));
    wrap.appendChild(chip);
  }
  return wrap;
}

function renderUsageRollup(rollup) {
  const wrap = el('section', 'wa-usage-rollup');
  wrap.setAttribute('aria-label', 'Usage by role and model');
  wrap.appendChild(el('h4', 'wa-section-title', 'Usage by role & model'));
  const rows = Array.isArray(rollup?.roles) ? rollup.roles : [];
  if (rows.length === 0) {
    wrap.appendChild(el('p', 'wa-empty', 'No model calls recorded yet.'));
    return wrap;
  }
  const list = el('ul', 'wa-usage-list');
  for (const row of rows) {
    const li = el('li', 'wa-usage-row');
    li.appendChild(el('strong', 'wa-usage-role', `${row.role || 'unknown'} · ${row.provider || 'unknown'}/${row.used_model || 'unknown'}`));
    li.appendChild(el('span', 'wa-usage-calls', ` · ${row.calls ?? 0} call(s) · `));
    li.appendChild(el('span', 'wa-usage-tokens', formatUsageSummary(row)));
    list.appendChild(li);
  }
  if (rollup?.totals) {
    const tot = el('p', 'wa-usage-totals', `Totals (${rollup.totals.calls ?? 0} calls): ${formatUsageSummary(rollup.totals)}`);
    wrap.append(list, tot);
  } else {
    wrap.appendChild(list);
  }
  return wrap;
}

function renderJobDetail(detailData, handlers) {
  const box = el('div', 'wa-job-detail');
  box.setAttribute('data-job-detail', detailData?.job?.job_id || '');

  if (detailData?.artifact_missing) {
    box.appendChild(createBadge({
      text: 'Artifact missing',
      secondaryText: 'One or more stored blobs could not be read from the ledger store.',
      icon: 'warn',
      tone: 'err',
    }));
  }

  // Worker summary block — strictly separated and labeled unverified.
  const summarySection = el('div', 'wa-detail-summary');
  summarySection.appendChild(createBadge({
    text: WORKER_SUMMARY_UNVERIFIED_LABEL,
    icon: 'model',
    tone: 'muted',
    border: 'dashed',
  }));
  const summaryText = detailData?.worker_summary?.text;
  if (detailData?.worker_summary?.missing) {
    summarySection.appendChild(el('p', 'wa-missing-note', 'Missing artifact: worker summary blob absent.'));
  } else {
    summarySection.appendChild(el('p', 'wa-summary-text', summaryText || 'No summary text recorded.'));
  }
  box.appendChild(summarySection);

  // Input context ranges (F4 context.supplied).
  const ctxSection = el('div', 'wa-detail-context');
  ctxSection.appendChild(el('h5', 'wa-subtitle', 'Input ranges & context'));
  const ctxRows = Array.isArray(detailData?.context_supplied) ? detailData.context_supplied : [];
  if (ctxRows.length === 0) {
    ctxSection.appendChild(el('p', 'wa-empty', 'No input line ranges recorded.'));
  } else {
    const ul = el('ul', 'wa-context-list');
    for (const r of ctxRows) {
      ul.appendChild(el('li', 'wa-context-row', `${r.source_id || '?'} · ${r.path || '?'} lines ${r.start_line}–${r.end_line}`));
    }
    ctxSection.appendChild(ul);
  }
  box.appendChild(ctxSection);

  // Deterministic tool receipts.
  const recSection = el('div', 'wa-detail-receipts');
  recSection.appendChild(el('h5', 'wa-subtitle', 'Tool & check receipts (by code)'));
  const receipts = Array.isArray(detailData?.tool_receipts) ? detailData.tool_receipts : [];
  if (receipts.length === 0) {
    recSection.appendChild(el('p', 'wa-empty', 'No tool receipts recorded.'));
  } else {
    const ul = el('ul', 'wa-receipt-list');
    for (const r of receipts) {
      const li = el('li', 'wa-receipt-row');
      const status = r.ok ? 'ok' : 'error';
      const miss = r.artifact_missing ? ' · MISSING ARTIFACT' : '';
      li.textContent = `${r.tool || 'tool'} [${r.call_id || '?'}] (${status})${miss}`;
      ul.appendChild(li);
    }
    recSection.appendChild(ul);
  }
  box.appendChild(recSection);

  // Returned items (ev-*, xe-*, name@v, e-*) with pin / dismiss actions.
  const itemsSection = el('div', 'wa-detail-items');
  itemsSection.appendChild(el('h5', 'wa-subtitle', 'Returned items'));
  const items = Array.isArray(detailData?.returned_items) ? detailData.returned_items : [];
  if (items.length === 0) {
    itemsSection.appendChild(el('p', 'wa-empty', 'No returned evidence or expression items.'));
  } else {
    const ul = el('ul', 'wa-items-list');
    for (const it of items) {
      const li = el('li', 'wa-item-row');
      const id = String(it.id || '?');
      li.setAttribute('data-item-id', id);
      li.appendChild(el('code', 'wa-item-id', id));
      if (it.quote) li.appendChild(el('span', 'wa-item-quote', ` "${it.quote}"`));
      const pinBtn = button(it.pinned ? 'Pinned' : 'Pin', 'pill-btn xs wa-pin-btn', () => {
        handlers.onControl({ action: 'pin_worker_item', item_id: id, job_id: detailData?.job?.job_id });
      }, { 'data-action': 'pin', 'data-item-id': id });
      const disBtn = button(it.dismissed ? 'Dismissed' : 'Dismiss', 'pill-btn xs wa-dismiss-btn', () => {
        handlers.onControl({ action: 'dismiss_worker_item', item_id: id, job_id: detailData?.job?.job_id });
      }, { 'data-action': 'dismiss', 'data-item-id': id });
      li.append(pinBtn, disBtn);
      ul.appendChild(li);
    }
    itemsSection.appendChild(ul);
  }
  box.appendChild(itemsSection);

  // Blobs list with explicit missing markers.
  const blobs = [
    ...(Array.isArray(detailData?.request_blobs) ? detailData.request_blobs.map((b) => ({ kind: 'request', ...b })) : []),
    ...(Array.isArray(detailData?.response_blobs) ? detailData.response_blobs.map((b) => ({ kind: 'response', ...b })) : []),
  ];
  if (blobs.length > 0) {
    const blobSection = el('div', 'wa-detail-blobs');
    blobSection.appendChild(el('h5', 'wa-subtitle', 'Recorded blobs'));
    const ul = el('ul', 'wa-blob-list');
    for (const b of blobs) {
      const li = el('li', 'wa-blob-row');
      li.textContent = b.missing
        ? `${b.kind} blob ${b.blob || '?'}: missing artifact`
        : `${b.kind} blob ${String(b.blob || '').slice(0, 12)}: ${b.preview || '(recorded)'}`;
      if (b.missing) li.setAttribute('data-missing', 'true');
      ul.appendChild(li);
    }
    blobSection.appendChild(ul);
    box.appendChild(blobSection);
  }

  return box;
}

export function createWorkerActivityInspector(host, opts = {}) {
  const sessionId = String(opts.sessionId || '');
  const fetchJson = typeof opts.fetchJson === 'function' ? opts.fetchJson : defaultFetchJson;
  const detailCache = new Map();
  const expanded = new Set();
  let stateData = opts.initialData || null;
  let lastError = null;

  async function defaultFetchJson(method, path, body) {
    const headers = {};
    if (opts.csrfToken) headers['X-ReSearch-Token'] = String(opts.csrfToken);
    const init = { method, credentials: 'same-origin', headers };
    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    const resp = await fetch(path, init);
    const data = await resp.json().catch(() => ({}));
    if (!resp.ok) throw new Error(data?.error || `HTTP ${resp.status}`);
    return data;
  }

  async function sendControl(payload) {
    if (!sessionId) return null;
    const res = await fetchJson('POST', `/api/sessions/${encodeURIComponent(sessionId)}/run-control`, payload);
    if (stateData && res?.controls) {
      stateData = { ...stateData, controls: res.controls };
      if (payload.action === 'pin_worker_item' || payload.action === 'dismiss_worker_item') {
        detailCache.clear();
      }
      render();
    }
    return res;
  }

  async function loadList() {
    if (!sessionId) return null;
    lastError = null;
    try {
      stateData = await fetchJson('GET', `/api/sessions/${encodeURIComponent(sessionId)}/workers`);
    } catch (err) {
      lastError = err?.message || 'Failed to load worker activity.';
    }
    render();
    return stateData;
  }

  async function toggleJobDetail(jobId) {
    if (expanded.has(jobId)) {
      expanded.delete(jobId);
      render();
      return null;
    }
    expanded.add(jobId);
    if (!detailCache.has(jobId) && sessionId) {
      try {
        const detail = await fetchJson(
          'GET',
          `/api/sessions/${encodeURIComponent(sessionId)}/workers/${encodeURIComponent(jobId)}`,
        );
        detailCache.set(jobId, detail);
      } catch (err) {
        detailCache.set(jobId, { error: err?.message || 'Failed to load job details.' });
      }
    }
    render();
    return detailCache.get(jobId) || null;
  }

  function renderControlsBar(controls = {}) {
    const bar = el('section', 'wa-controls');
    bar.setAttribute('aria-label', 'Reader controls');

    const focusWrap = el('div', 'wa-focus-control');
    const focusInput = el('input', 'wa-focus-input');
    focusInput.type = 'text';
    focusInput.placeholder = 'Adjust focus note for next stage...';
    focusInput.setAttribute('aria-label', 'Focus note');
    const focusBtn = button('Save focus note', 'pill-btn sm wa-focus-btn', () => {
      const note = String(focusInput.value || '').trim();
      if (!note) return;
      focusInput.value = '';
      sendControl({ action: 'focus_note', note });
    }, { 'data-control': 'focus_note' });
    focusWrap.append(focusInput, focusBtn);

    const pausedNext = Boolean(controls.pause_after_stage);
    const pauseBtn = button(
      pausedNext ? 'Pause after stage: On' : 'Pause after stage: Off',
      'pill-btn sm wa-pause-btn',
      () => sendControl({ action: 'pause_after_stage', enabled: !pausedNext }),
      { 'data-control': 'pause_after_stage', 'aria-pressed': String(pausedNext) },
    );

    const bumpBtn = button('+4 calls next', 'pill-btn sm wa-budget-btn', () => {
      sendControl({ action: 'budget_delta', delta: 4 });
    }, { 'data-control': 'budget_delta' });

    bar.append(focusWrap, pauseBtn, bumpBtn);
    if (Array.isArray(controls.focus_notes) && controls.focus_notes.length > 0) {
      const latest = controls.focus_notes[controls.focus_notes.length - 1];
      bar.appendChild(el('p', 'wa-latest-focus', `Focus: ${latest?.note || ''}`));
    }
    return bar;
  }

  function render() {
    if (!host) return null;
    const root = el('div', 'wa-inspector');
    root.setAttribute('data-session-id', sessionId);

    const hdr = el('header', 'wa-header');
    hdr.appendChild(el('h3', 'wa-title', 'Worker activity & controls'));
    hdr.appendChild(button('Refresh', 'pill-btn sm wa-refresh-btn', () => loadList()));
    root.appendChild(hdr);

    if (lastError) {
      root.appendChild(createBadge({ text: 'Error', secondaryText: lastError, icon: 'error', tone: 'err' }));
    }

    root.appendChild(renderRolesStrip(stateData?.roles));
    root.appendChild(renderUsageRollup(stateData?.usage_rollup));
    root.appendChild(renderControlsBar(stateData?.controls));

    const jobsSection = el('section', 'wa-jobs');
    jobsSection.appendChild(el('h4', 'wa-section-title', 'Worker & verifier jobs'));
    const jobs = Array.isArray(stateData?.jobs) ? stateData.jobs : [];
    if (jobs.length === 0) {
      jobsSection.appendChild(el('p', 'wa-empty', 'No worker jobs recorded for this session.'));
    } else {
      for (const job of jobs) {
        const card = el('article', 'wa-job-card');
        card.setAttribute('data-job-id', job.job_id || '');
        const top = el('div', 'wa-job-top');
        const disp = dispositionTone(job.disposition);
        top.appendChild(createBadge({
          text: `${job.role || 'worker'} · ${disp.text}`,
          secondaryText: `${job.provider || '?'}/${job.used_model || job.requested_model || '?'} · ${job.calls_used ?? 0} call(s) · ${formatUsageSummary(job.usage)}`,
          icon: disp.icon,
          tone: disp.tone,
        }));
        const hasMissingRef = Array.isArray(job.artifact_refs) && job.artifact_refs.some((r) => r?.missing);
        if (hasMissingRef) {
          top.appendChild(createBadge({ text: 'Missing artifact', icon: 'warn', tone: 'err' }));
        }
        const isExpanded = expanded.has(job.job_id);
        top.appendChild(button(isExpanded ? 'Hide details' : 'Inspect', 'pill-btn sm wa-inspect-btn', () => {
          toggleJobDetail(job.job_id);
        }, { 'data-inspect-job': job.job_id }));
        card.appendChild(top);

        if (job.disposition_reason) {
          card.appendChild(el('p', 'wa-job-reason', job.disposition_reason));
        }
        if (isExpanded) {
          const cached = detailCache.get(job.job_id);
          if (!cached) {
            card.appendChild(el('p', 'wa-loading', 'Loading job details...'));
          } else if (cached.error) {
            card.appendChild(el('p', 'wa-error', cached.error));
          } else {
            card.appendChild(renderJobDetail(cached, { onControl: sendControl }));
          }
        }
        jobsSection.appendChild(card);
      }
    }
    root.appendChild(jobsSection);

    host.replaceChildren(root);
    return root;
  }

  render();

  return {
    loadList,
    toggleJobDetail,
    sendControl,
    render,
    getData: () => stateData,
  };
}

export function mountWorkerActivityButton(host, opts = {}) {
  if (!host) return null;
  let open = false;
  let inspector = null;
  const wrap = el('div', 'wa-mount');
  const drawer = el('div', 'wa-drawer');
  drawer.hidden = true;
  const toggleBtn = button('Workers', 'pill-btn sm wa-open-btn', async () => {
    open = !open;
    drawer.hidden = !open;
    toggleBtn.setAttribute('aria-expanded', String(open));
    if (open && !inspector) {
      inspector = createWorkerActivityInspector(drawer, opts);
      await inspector.loadList();
    }
  }, { 'aria-expanded': 'false', 'data-worker-inspector-toggle': '1' });
  wrap.append(toggleBtn, drawer);
  host.appendChild(wrap);
  return {
    button: toggleBtn,
    drawer,
    getInspector: () => inspector,
  };
}
