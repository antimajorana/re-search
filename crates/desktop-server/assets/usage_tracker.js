// Token and call usage tracker for Settings (#settings-panel-usage) and the
// compact per-session usage line (#session-usage-line). Zero innerHTML writes;
// zero cost estimates; handles subscription CLI routes ("tokens not reported").
import * as api from './api.js';

export const TOKENS_NOT_REPORTED = 'tokens not reported';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function safeNonNeg(value) {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

function isAllUnreported(bucket, calls, unreported) {
  if (bucket?.tokens_reported === false) return true;
  if (bucket?.tokens_reported === true) return false;
  return unreported >= calls && safeNonNeg(bucket?.input_tokens) === 0 && safeNonNeg(bucket?.output_tokens) === 0;
}

export function formatCompactTokens(value) {
  const n = safeNonNeg(value);
  if (n >= 999_950) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function formatCallCount(value) {
  const n = safeNonNeg(value);
  return n === 1 ? '1 call' : `${n} calls`;
}

export function formatTokenSummary(bucket) {
  const calls = safeNonNeg(bucket?.calls);
  if (calls === 0) return 'No calls';
  const unreported = safeNonNeg(bucket?.unreported_calls);
  if (isAllUnreported(bucket, calls, unreported)) return TOKENS_NOT_REPORTED;
  const base = `${formatCompactTokens(bucket?.input_tokens)} in / ${formatCompactTokens(bucket?.output_tokens)} out`;
  return unreported > 0 ? `${base} (+${unreported} unreported)` : base;
}

export function formatSessionUsageLine(summary) {
  const calls = safeNonNeg(summary?.calls);
  if (calls === 0) return '';
  const unreported = safeNonNeg(summary?.unreported_calls);
  if (isAllUnreported(summary, calls, unreported)) {
    return `${formatCallCount(calls)} · ${TOKENS_NOT_REPORTED}`;
  }
  const base = `${formatCallCount(calls)} · ${formatCompactTokens(summary?.input_tokens)} in / ${formatCompactTokens(summary?.output_tokens)} out`;
  return unreported > 0 ? `${base} (${unreported} unreported)` : base;
}

function buildSummaryCard(label, bucket) {
  const card = el('div', 'usage-summary-card');
  card.setAttribute('data-usage-window', label.toLowerCase().replace(/\s+/g, '_'));
  card.append(el('div', 'usage-card-label', label));
  const calls = safeNonNeg(bucket?.calls);
  const unreported = safeNonNeg(bucket?.unreported_calls);
  const allUnrep = calls > 0 && isAllUnreported(bucket, calls, unreported);
  card.append(el('div', 'usage-card-calls', formatCallCount(calls)));
  const tokensText = calls === 0
    ? 'No calls'
    : (allUnrep
      ? TOKENS_NOT_REPORTED
      : `${formatCompactTokens(bucket?.input_tokens)} in / ${formatCompactTokens(bucket?.output_tokens)} out`);
  card.append(el('div', 'usage-card-tokens', tokensText));
  if (unreported > 0 && !allUnrep) {
    const unit = unreported === 1 ? 'call' : 'calls';
    card.append(el('div', 'usage-card-note', `${unreported} ${unit} without full token counts`));
  }
  return card;
}

function buildTableSection(title, sectionKey, headers, rows, emptyText) {
  const section = el('section', 'usage-section');
  section.setAttribute('data-usage-section', sectionKey);
  section.append(el('h3', 'usage-section-title', title));
  if (!Array.isArray(rows) || rows.length === 0) {
    section.append(el('p', 'usage-empty', emptyText));
    return section;
  }
  const wrap = el('div', 'usage-table-wrap');
  const table = el('table', 'usage-table');
  const thead = el('thead');
  const headRow = el('tr');
  for (const h of headers) {
    const th = el('th', h.num ? 'usage-num' : '', h.label);
    th.setAttribute('scope', 'col');
    headRow.append(th);
  }
  thead.append(headRow);
  const tbody = el('tbody');
  for (const cells of rows) {
    const tr = el('tr');
    cells.forEach((cell, idx) => {
      const cls = headers[idx]?.num ? 'usage-num' : (headers[idx]?.mono ? 'usage-mono' : '');
      tr.append(el('td', cls, cell));
    });
    tbody.append(tr);
  }
  table.append(thead, tbody);
  wrap.append(table);
  section.append(wrap);
  return section;
}

export function renderUsagePanel(containerEl, data, opts = {}) {
  if (!containerEl) return;
  const root = el('div', 'usage-panel-body');

  const head = el('div', 'usage-header-row');
  head.append(el('p', 'usage-intro', 'Derived directly from recorded model responses in the session ledger.'));
  const refreshBtn = el('button', 'pill-btn sm', 'Refresh');
  refreshBtn.type = 'button';
  refreshBtn.id = 'usage-refresh-btn';
  if (typeof opts.onRefresh === 'function') {
    refreshBtn.addEventListener('click', () => { opts.onRefresh(); });
  }
  head.append(refreshBtn);
  root.append(head);

  const totals = data?.totals || {};
  const grid = el('div', 'usage-summary-grid');
  grid.append(
    buildSummaryCard('Today', totals.today),
    buildSummaryCard('Last 7 days', totals.last_7_days),
    buildSummaryCard('All time', totals.all_time),
  );
  root.append(grid);

  const pmRows = (Array.isArray(data?.by_provider_model) ? data.by_provider_model : []).map((row) => [
    String(row?.provider || 'unknown'),
    String(row?.model || 'unknown'),
    String(safeNonNeg(row?.calls)),
    formatTokenSummary(row),
  ]);
  root.append(buildTableSection(
    'By provider & model',
    'by_provider_model',
    [
      { label: 'Provider' },
      { label: 'Model', mono: true },
      { label: 'Calls', num: true },
      { label: 'Tokens', num: true },
    ],
    pmRows,
    'No model calls recorded yet.',
  ));

  const dayRows = (Array.isArray(data?.by_day) ? data.by_day : []).map((row) => [
    String(row?.date || ''),
    String(safeNonNeg(row?.calls)),
    formatTokenSummary(row),
  ]);
  root.append(buildTableSection(
    'By day',
    'by_day',
    [
      { label: 'Date (UTC)', mono: true },
      { label: 'Calls', num: true },
      { label: 'Tokens', num: true },
    ],
    dayRows,
    'No daily usage recorded yet.',
  ));

  const sessRows = (Array.isArray(data?.by_session) ? data.by_session : []).map((row) => [
    String(row?.title || row?.session_id || ''),
    String(row?.mode || 'research'),
    String(safeNonNeg(row?.calls)),
    formatTokenSummary(row),
  ]);
  root.append(buildTableSection(
    'By session',
    'by_session',
    [
      { label: 'Session' },
      { label: 'Mode' },
      { label: 'Calls', num: true },
      { label: 'Tokens', num: true },
    ],
    sessRows,
    'No sessions with model calls yet.',
  ));

  containerEl.replaceChildren(root);
}

export function renderSessionUsageLine(lineEl, summary) {
  if (!lineEl) return;
  const text = formatSessionUsageLine(summary);
  lineEl.textContent = text;
  lineEl.hidden = !text;
}

export function createUsageTrackerController(deps = {}) {
  let usageSeq = 0;
  let sessionSeq = 0;
  let lastSessionId = '';
  let lastSessionSig = '';
  const getPanelEl = () => deps.panelEl || (typeof document !== 'undefined' ? document.getElementById('settings-usage-content') : null);
  const getLineEl = () => deps.sessionLineEl || (typeof document !== 'undefined' ? document.getElementById('session-usage-line') : null);
  const fetchUsage = typeof deps.fetchUsage === 'function'
    ? deps.fetchUsage
    : (sessionId) => api.getUsage(sessionId);

  async function loadUsage() {
    const panel = getPanelEl();
    if (!panel) return null;
    const seq = ++usageSeq;
    const existingBtn = typeof panel.querySelector === 'function' ? panel.querySelector('#usage-refresh-btn') : null;
    if (existingBtn) existingBtn.disabled = true;
    else panel.replaceChildren(el('p', 'usage-empty', 'Loading usage...'));
    try {
      const data = await fetchUsage(null);
      if (seq !== usageSeq) return null;
      renderUsagePanel(panel, data, { onRefresh: () => loadUsage() });
      return data;
    } catch (err) {
      if (seq !== usageSeq) return null;
      const msg = el('p', 'usage-empty', `Could not load usage: ${err?.message || 'request failed'}`);
      panel.replaceChildren(msg);
      return null;
    }
  }

  async function syncSession(sessionId, sessions = null) {
    const line = getLineEl();
    if (!line) return null;
    const sid = String(sessionId || '').trim();
    if (!sid) {
      sessionSeq += 1;
      lastSessionId = '';
      lastSessionSig = '';
      renderSessionUsageLine(line, null);
      return null;
    }
    const meta = Array.isArray(sessions) ? sessions.find((s) => s?.id === sid) : null;
    const sig = meta ? `${sid}:${meta.event_count ?? ''}:${meta.updated_ms ?? ''}:${Boolean(meta.running)}` : '';
    if (sig && sig === lastSessionSig) return null;
    if (sid !== lastSessionId) {
      renderSessionUsageLine(line, null);
      lastSessionSig = '';
    }
    lastSessionId = sid;
    const seq = ++sessionSeq;
    try {
      const data = await fetchUsage(sid);
      if (seq !== sessionSeq) return null;
      lastSessionSig = sig;
      const summary = data?.session || (Array.isArray(data?.by_session) ? data.by_session[0] : null);
      renderSessionUsageLine(line, summary);
      return summary;
    } catch {
      if (seq !== sessionSeq) return null;
      lastSessionSig = '';
      renderSessionUsageLine(line, null);
      return null;
    }
  }

  function onSettingsTab(tab) {
    if (tab === 'usage') return loadUsage();
    return null;
  }

  return { loadUsage, syncSession, onSettingsTab };
}

if (typeof globalThis !== 'undefined' && typeof document !== 'undefined') {
  globalThis.__reSearchUsageTracker = createUsageTrackerController();
}
