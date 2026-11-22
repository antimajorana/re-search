// Center conversational stream: empty hero starters, Markdown+KaTeX turns, thinking traces, and live approval cards.
import * as api from '/assets/api.js';
import { escapeHtml, highlightPython, highlightUnifiedDiff, renderMathInContainer, renderRichInto } from '/assets/math.js';

const STARTER_PROMPTS = [
  {
    title: 'Exact SymPy & Perturbation Check',
    prompt: 'Compute the ground-state energy gap of the 1D Kitaev chain at $\\mu = 0$ and verify the Majorana edge-mode commutator algebra in SymPy.',
  },
  {
    title: 'arXiv TeX Source Audit',
    prompt: 'Fetch the arXiv LaTeX source, cross-check all \\ref{} and \\cite{} keys, and summarize the main theorem with exact notation.',
  },
  {
    title: 'Proof-DAG Step Verification',
    prompt: 'Derive the dispersion relation $\\omega(k) = \\sqrt{\\Delta^2 + (2t \\cos k)^2}$, append structured proof steps, and run deterministic verification.',
  },
];

let streamEl = null;
let statusPillEl = null;
let currentSessionId = null;
let onPromptPick = null;
let onNotify = null;
let traceSeq = 0;

export function initStream(opts) {
  streamEl = document.getElementById('message-stream');
  statusPillEl = document.getElementById('stream-status-pill');
  onPromptPick = opts && opts.onPromptPick ? opts.onPromptPick : null;
  onNotify = opts && opts.onNotify ? opts.onNotify : null;
}

export function setStreamStatus(text, tone = '') {
  if (!statusPillEl) return;
  statusPillEl.textContent = text;
  statusPillEl.className = `badge${tone ? ' ' + tone : ''}`;
}

function getInnerContainer() {
  if (!streamEl) return null;
  let inner = streamEl.querySelector('.stream-inner');
  if (!inner) {
    streamEl.innerHTML = '';
    inner = document.createElement('div');
    inner.className = 'stream-inner';
    streamEl.appendChild(inner);
  }
  return inner;
}

function scrollToBottom() {
  if (streamEl) {
    streamEl.scrollTop = streamEl.scrollHeight;
  }
}

export function renderSessionStream(sessionId, sessionPayload) {
  currentSessionId = sessionId;
  const inner = getInnerContainer();
  if (!inner) return;
  inner.innerHTML = '';

  const messages = Array.isArray(sessionPayload && sessionPayload.messages)
    ? sessionPayload.messages
    : [];
  const traces = Array.isArray(sessionPayload && sessionPayload.traces)
    ? sessionPayload.traces
    : [];
  const pending = Array.isArray(sessionPayload && sessionPayload.pending_approvals)
    ? sessionPayload.pending_approvals
    : [];

  if (messages.length === 0 && traces.length === 0 && pending.length === 0) {
    renderEmptyHero(inner);
    return;
  }

  for (const msg of messages) {
    if ((msg.role || 'assistant') === 'user') {
      appendUserMessage(msg.text || msg.content || '');
    } else if (msg.role === 'warning') {
      appendWarningLine(msg.text || '');
    } else {
      appendAssistantMessage(msg.text || msg.content || '', msg.thinking || '', msg.tool_traces || []);
    }
  }

  for (const req of pending) {
    renderApprovalCard(req);
  }
  scrollToBottom();
}

function renderEmptyHero(inner) {
  const hero = document.createElement('div');
  hero.className = 'empty-hero';
  hero.innerHTML = `
    <h1>What are we deriving today?</h1>
    <p>Verified symbolic algebra, arXiv LaTeX auditing, and hash-chained proof graphs — running locally with zero unverified shortcuts.</p>
    <div class="starter-grid"></div>
  `;
  const grid = hero.querySelector('.starter-grid');
  for (const item of STARTER_PROMPTS) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'starter-chip';
    btn.innerHTML = `<strong>${escapeHtml(item.title)}</strong><span>${escapeHtml(item.prompt)}</span>`;
    btn.addEventListener('click', () => {
      if (typeof onPromptPick === 'function') {
        onPromptPick(item.prompt);
      }
    });
    grid.appendChild(btn);
  }
  inner.appendChild(hero);
  renderMathInContainer(hero);
}

function clearHeroIfPresent() {
  const inner = getInnerContainer();
  if (!inner) return null;
  const hero = inner.querySelector('.empty-hero');
  if (hero) hero.remove();
  return inner;
}

export function appendUserMessage(text) {
  const inner = clearHeroIfPresent();
  if (!inner) return;
  const row = document.createElement('div');
  row.className = 'msg-row user';
  const bubble = document.createElement('div');
  bubble.className = 'msg-bubble-user rich-prose';
  renderRichInto(bubble, text);
  row.appendChild(bubble);
  inner.appendChild(row);
  scrollToBottom();
}

// A plain-text warning line in the transcript (for example a vendor CLI's built-in tool
// running outside re:Search's tools). Text only, never HTML.
export function appendWarningLine(text) {
  const inner = clearHeroIfPresent();
  if (!inner) return;
  const row = document.createElement('div');
  row.className = 'msg-row warning';
  row.setAttribute('role', 'status');
  const line = document.createElement('div');
  line.className = 'msg-warning';
  line.textContent = String(text);
  row.appendChild(line);
  inner.appendChild(row);
  scrollToBottom();
}

export function appendAssistantMessage(text, thinkingText = '', toolTraces = []) {
  const inner = clearHeroIfPresent();
  if (!inner) return;
  const row = document.createElement('div');
  row.className = 'msg-row assistant';

  if (thinkingText || (toolTraces && toolTraces.length > 0)) {
    row.appendChild(buildThinkingAccordion(thinkingText, toolTraces));
  }

  if (text && String(text).trim()) {
    const wrap = document.createElement('div');
    wrap.className = 'msg-assistant';
    wrap.innerHTML = `
      <div class="assistant-avatar" aria-hidden="true">∑</div>
      <div class="msg-assistant-body rich-prose"></div>
    `;
    renderRichInto(wrap.querySelector('.msg-assistant-body'), text);
    row.appendChild(wrap);
  }
  inner.appendChild(row);
  scrollToBottom();
}

function ensureLiveThinkingAccordion() {
  const inner = clearHeroIfPresent();
  if (!inner) return null;
  let acc = inner.querySelector('.thinking-accordion[data-live="true"]');
  if (!acc) {
    acc = buildThinkingAccordion('', []);
    acc.dataset.live = 'true';
    acc.open = true;
    inner.appendChild(acc);
  }
  return acc;
}

function buildThinkingAccordion(thinkingText, toolTraces) {
  const det = document.createElement('details');
  det.className = 'thinking-accordion';
  const count = (toolTraces || []).length;
  const label = count > 0 ? `Show thinking & tool trace (${count})` : 'Show thinking & tool trace';
  det.innerHTML = `
    <summary>
      <span>${escapeHtml(label)}</span>
      <span class="badge">Trace</span>
    </summary>
    <div class="thinking-body">
      <div class="thinking-notes">${escapeHtml(thinkingText || '')}</div>
      <div class="trace-list" style="display:flex;flex-direction:column;gap:8px;"></div>
    </div>
  `;
  const list = det.querySelector('.trace-list');
  for (const tr of toolTraces || []) {
    list.appendChild(buildTraceItem(tr));
  }
  return det;
}

function buildTraceItem(tr) {
  traceSeq += 1;
  const box = document.createElement('div');
  box.className = 'trace-item';
  const toolName = tr.tool || tr.tool_id || 'tool';
  const status = tr.status || (tr.ok === false ? 'error' : 'ok');
  const tone = status === 'error' || status === 'denied' ? 'err' : status === 'running' ? 'warn' : 'ok';
  const inputStr = typeof tr.args === 'string' ? tr.args : JSON.stringify(tr.args || tr.input || {}, null, 2);
  const outputStr = tr.output === undefined ? '' : typeof tr.output === 'string' ? tr.output : JSON.stringify(tr.output, null, 2);
  box.innerHTML = `
    <div class="trace-head">
      <strong>${escapeHtml(toolName)}</strong>
      <span class="badge ${tone}">${escapeHtml(status)}</span>
    </div>
    ${inputStr && inputStr !== '{}' && inputStr !== 'null' ? `<pre class="trace-io">${escapeHtml(inputStr)}</pre>` : ''}
    ${outputStr ? `<pre class="trace-io">${escapeHtml(outputStr.slice(0, 1200))}</pre>` : ''}
  `;
  box.dataset.traceKey = tr.call_id || `t-${traceSeq}`;
  box.dataset.toolName = toolName;
  box.dataset.argsStr = inputStr;
  return box;
}

export function handleThinkingDelta(data) {
  const acc = ensureLiveThinkingAccordion();
  if (!acc) return;
  const notes = acc.querySelector('.thinking-notes');
  if (notes) {
    notes.textContent += (data && (data.delta || data.text || '')) || '';
  }
  scrollToBottom();
}

export function handleToolTrace(data) {
  const acc = ensureLiveThinkingAccordion();
  if (!acc || !data) return;
  const list = acc.querySelector('.trace-list');
  const key = data.call_id || '';
  const existing = key ? list.querySelector(`[data-trace-key="${CSS.escape(key)}"]`) : null;
  const merged = { ...data };
  if (existing) {
    if ((!merged.tool || merged.tool === 'tool') && existing.dataset.toolName) {
      merged.tool = existing.dataset.toolName;
    }
    if (merged.args === undefined && existing.dataset.argsStr) {
      merged.args = existing.dataset.argsStr;
    }
  }
  const item = buildTraceItem(merged);
  if (existing) {
    existing.replaceWith(item);
  } else {
    list.appendChild(item);
  }
  const summarySpan = acc.querySelector('summary > span');
  if (summarySpan) {
    summarySpan.textContent = `Show thinking & tool trace (${list.children.length})`;
  }
  scrollToBottom();
}

export function finalizeLiveThinking() {
  if (!streamEl) return;
  for (const acc of streamEl.querySelectorAll('.thinking-accordion[data-live="true"]')) {
    delete acc.dataset.live;
    acc.open = false;
  }
}

export function renderApprovalCard(req) {
  const inner = clearHeroIfPresent();
  if (!inner || !req) return;
  const callId = String(req.call_id || '');
  const existing = callId ? inner.querySelector(`.approval-card[data-call-id="${CSS.escape(callId)}"]`) : null;
  if (existing) return;

  const toolId = String(req.tool_id || req.tool || 'tool');
  const digest = String(req.digest || '');
  const scope = String(req.scope || 'once');
  const containmentBadge =
    req.contained || scope === 'worker-session'
      ? '<span class="badge ok">CONTAINED AppContainer</span>'
      : '<span class="badge warn">ONCE Approval</span>';

  const editPreview = req.edit_preview || null;
  let previewHtml = '';
  if (editPreview && editPreview.unified_diff) {
    previewHtml = highlightUnifiedDiff(editPreview.unified_diff);
  } else {
    let argsObj = req.args;
    if (!argsObj && typeof req.canonical_args === 'string') {
      try { argsObj = JSON.parse(req.canonical_args); } catch { argsObj = null; }
    }
    if (argsObj && typeof argsObj.code === 'string') {
      previewHtml = highlightPython(argsObj.code);
    } else {
      previewHtml = `<span class="diff-line">${escapeHtml(req.canonical_args || JSON.stringify(argsObj || {}, null, 2))}</span>`;
    }
  }

  const card = document.createElement('div');
  card.className = 'approval-card';
  card.dataset.callId = callId;
  card.dataset.digest = digest;
  card.innerHTML = `
    <div class="approval-head">
      <div class="approval-meta">
        <strong>Approval Required: <code>${escapeHtml(toolId)}</code></strong>
        ${containmentBadge}
        ${editPreview && editPreview.rel_path ? `<span class="badge">${escapeHtml(editPreview.rel_path)}</span>` : ''}
      </div>
      <span class="digest-pill" title="${escapeHtml(digest)}">SHA-256 ${escapeHtml(digest.slice(0, 14))}…</span>
    </div>
    <div class="approval-preview">${previewHtml}</div>
    <div class="approval-actions">
      <button type="button" class="pill-btn sm approval-decline-btn">Decline</button>
      <button type="button" class="pill-btn sm primary approval-approve-btn">Approve &amp; Run</button>
    </div>
  `;

  const approveBtn = card.querySelector('.approval-approve-btn');
  const declineBtn = card.querySelector('.approval-decline-btn');
  const submitDecision = async (approved) => {
    if (!currentSessionId) return;
    approveBtn.disabled = true;
    declineBtn.disabled = true;
    try {
      await api.respondApproval(currentSessionId, callId, { digest, approved });
      resolveApprovalCard(callId, approved);
    } catch (err) {
      approveBtn.disabled = false;
      declineBtn.disabled = false;
      if (typeof onNotify === 'function') onNotify(`Approval error: ${err.message}`);
    }
  };
  approveBtn.addEventListener('click', () => submitDecision(true));
  declineBtn.addEventListener('click', () => submitDecision(false));

  inner.appendChild(card);
  scrollToBottom();
}

export function resolveApprovalCard(callId, approved) {
  if (!streamEl) return;
  const card = streamEl.querySelector(`.approval-card[data-call-id="${CSS.escape(String(callId || ''))}"]`);
  if (!card) return;
  const actions = card.querySelector('.approval-actions');
  if (actions) {
    actions.innerHTML = approved
      ? '<span class="badge ok">Approved</span>'
      : '<span class="badge err">Declined</span>';
  }
}
