// Center conversational stream: interactive quickstart + starter cards, Markdown+KaTeX turns,
// structured scientific tool traces, Independent Verifier verdict cards, and live approval cards.
import * as api from '/assets/api.js';
import { escapeHtml, highlightPython, highlightUnifiedDiff, renderMathInContainer, renderRichInto } from '/assets/math.js';
import { buildStructuredTraceCard, buildVerificationVerdictCard } from '/assets/trace_cards.js';

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
  {
    title: 'Asymptotic Expansion & Sign Audit',
    prompt: 'Expand $\\int_0^\\infty \\frac{k^2\\,dk}{\\sqrt{k^2 + m^2}\\,(e^{\\beta\\sqrt{k^2+m^2}} + 1)}$ at $\\beta m \\ll 1$, check leading coefficients numerically in Python, and record each step.',
  },
];

let streamEl = null;
let statusPillEl = null;
let currentSessionId = null;
let onPromptPick = null;
let onNotify = null;
let onInspectStep = null;
let onOpenConnect = null;
let onPickWorkspace = null;
let onOpenConventions = null;
let getSetupInfo = null;
let liveStreamText = '';
let liveRenderPending = false;

export function initStream(opts = {}) {
  streamEl = document.getElementById('message-stream');
  statusPillEl = document.getElementById('stream-status-pill');
  onPromptPick = opts.onPromptPick || null;
  onNotify = opts.onNotify || null;
  onInspectStep = opts.onInspectStep || null;
  onOpenConnect = opts.onOpenConnect || null;
  onPickWorkspace = opts.onPickWorkspace || null;
  onOpenConventions = opts.onOpenConventions || null;
  getSetupInfo = opts.getSetupInfo || null;
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
  liveStreamText = '';
  liveRenderPending = false;
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
      appendAssistantMessage(
        msg.text || msg.content || '',
        msg.thinking || '',
        msg.tool_traces || [],
        msg
      );
    }
  }

  for (const req of pending) {
    renderApprovalCard(req);
  }
  scrollToBottom();
}

export function refreshEmptyHeroIfVisible() {
  const inner = getInnerContainer();
  if (!inner || !inner.querySelector('.empty-hero')) return;
  inner.innerHTML = '';
  renderEmptyHero(inner);
}

function basenameOfPath(p) {
  if (!p) return '';
  const parts = String(p).replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || String(p);
}

function renderEmptyHero(inner) {
  const info = typeof getSetupInfo === 'function' ? (getSetupInfo() || {}) : {};
  const connCount = Number(info.connectedCount) || 0;
  const wsPath = String(info.workspaceRoot || '');
  const hasConv = Boolean(info.hasConventions);

  const hero = document.createElement('div');
  hero.className = 'empty-hero';
  hero.innerHTML = `
    <h1>What are we deriving today?</h1>
    <p>Agents propose, computations and independent verifiers decide — with sandboxed SymPy, arXiv TeX auditing, and a hash-chained Proof DAG.</p>
    <div class="quickstart-strip">
      <div class="quickstart-card">
        <div class="quickstart-head">
          <span>1. Model Provider</span>
          <span class="badge ${connCount > 0 ? 'ok' : 'warn'}">${connCount > 0 ? `${connCount} ready` : 'Setup'}</span>
        </div>
        <div class="quickstart-desc">${connCount > 0 ? 'Provider connected. Choose your model next to Send or configure separate Verifier models.' : 'Connect ChatGPT, Claude Code, Anthropic/OpenAI/Gemini API, or local Ollama.'}</div>
        <div><button type="button" class="pill-btn sm hero-connect-btn ${connCount > 0 ? '' : 'primary'}">${connCount > 0 ? 'Manage Providers' : 'Connect a Model'}</button></div>
      </div>
      <div class="quickstart-card">
        <div class="quickstart-head">
          <span>2. Paper Workspace</span>
          <span class="badge ${wsPath ? 'ok' : ''}">${wsPath ? escapeHtml(basenameOfPath(wsPath)) : 'Optional'}</span>
        </div>
        <div class="quickstart-desc">${wsPath ? `Attached to ${escapeHtml(wsPath)}. Browse .tex/.bib files in the right Canvas.` : 'Attach a local folder to edit .tex manuscripts, audit citations, and save patches.'}</div>
        <div><button type="button" class="pill-btn sm hero-ws-btn">${wsPath ? 'Switch Folder' : 'Attach Folder'}</button></div>
      </div>
      <div class="quickstart-card">
        <div class="quickstart-head">
          <span>3. Conventions &amp; Rigor</span>
          <span class="badge ${hasConv ? 'ok' : 'accent'}">${hasConv ? 'Pinned' : 'Verified Loop'}</span>
        </div>
        <div class="quickstart-desc">${hasConv ? 'Custom metric/unit conventions active across Solver & Stateless Verifier.' : 'Pin metric signature, natural units, or Fourier conventions so signs stay exact.'}</div>
        <div><button type="button" class="pill-btn sm hero-conv-btn">${hasConv ? 'Edit Conventions' : 'Pin Conventions'}</button></div>
      </div>
    </div>
    <div class="starter-grid"></div>
  `;

  const connBtn = hero.querySelector('.hero-connect-btn');
  const wsBtn = hero.querySelector('.hero-ws-btn');
  const convBtn = hero.querySelector('.hero-conv-btn');
  if (connBtn && typeof onOpenConnect === 'function') connBtn.addEventListener('click', onOpenConnect);
  if (wsBtn && typeof onPickWorkspace === 'function') wsBtn.addEventListener('click', onPickWorkspace);
  if (convBtn && typeof onOpenConventions === 'function') convBtn.addEventListener('click', onOpenConventions);

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

export function appendAssistantMessage(text, thinkingText = '', toolTraces = [], msgMeta = null) {
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

  const verdictCard = buildVerificationVerdictCard(msgMeta, onInspectStep);
  if (verdictCard) {
    row.appendChild(verdictCard);
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

function summarizeTraceCounts(toolTraces) {
  const list = toolTraces || [];
  if (list.length === 0) return 'Reasoning & agent log';
  const sympy = list.filter((t) => (t.tool || t.tool_id) === 'sympy_eval').length;
  const dag = list.filter((t) => /^ledger_/.test(t.tool || t.tool_id || '')).length;
  const parts = [`${list.length} tool call${list.length === 1 ? '' : 's'}`];
  if (sympy > 0) parts.push(`${sympy} SymPy`);
  if (dag > 0) parts.push(`${dag} Proof step${dag === 1 ? '' : 's'}`);
  return `Activity & reasoning (${parts.join(' · ')})`;
}

function buildThinkingAccordion(thinkingText, toolTraces) {
  const det = document.createElement('details');
  det.className = 'thinking-accordion';
  const label = summarizeTraceCounts(toolTraces);
  det.innerHTML = `
    <summary>
      <span>${escapeHtml(label)}</span>
      <span class="badge">Activity</span>
    </summary>
    <div class="thinking-body">
      <div class="thinking-notes">${escapeHtml(thinkingText || '')}</div>
      <div class="trace-list" style="display:flex;flex-direction:column;gap:8px;"></div>
    </div>
  `;
  const list = det.querySelector('.trace-list');
  for (const tr of toolTraces || []) {
    list.appendChild(buildStructuredTraceCard(tr, { onInspectStep }));
  }
  return det;
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

function ensureLiveAssistantBubble() {
  const inner = clearHeroIfPresent();
  if (!inner) return null;
  let row = inner.querySelector('.msg-row.assistant[data-live="true"]');
  if (!row) {
    row = document.createElement('div');
    row.className = 'msg-row assistant';
    row.dataset.live = 'true';
    row.innerHTML = `
      <div class="msg-assistant">
        <div class="assistant-avatar" aria-hidden="true">∑</div>
        <div class="msg-assistant-body rich-prose"></div>
      </div>
    `;
    inner.appendChild(row);
  }
  return row;
}

export function handleTextDelta(data) {
  const delta = (data && (data.delta || data.text || '')) || '';
  if (!delta) return;
  liveStreamText += delta;
  const row = ensureLiveAssistantBubble();
  if (!row || liveRenderPending) return;
  liveRenderPending = true;
  const renderNow = () => {
    liveRenderPending = false;
    if (!streamEl) return;
    const liveRow = streamEl.querySelector('.msg-row.assistant[data-live="true"]');
    if (!liveRow) return;
    const body = liveRow.querySelector('.msg-assistant-body');
    if (body) renderRichInto(body, liveStreamText);
    scrollToBottom();
  };
  if (typeof window !== 'undefined' && typeof window.requestAnimationFrame === 'function') {
    window.requestAnimationFrame(renderNow);
  } else {
    renderNow();
  }
}

export function handleRoundReset() {
  liveStreamText = '';
  liveRenderPending = false;
  if (!streamEl) return;
  for (const row of streamEl.querySelectorAll('.msg-row.assistant[data-live="true"]')) {
    row.remove();
  }
}

export function finalizeLiveAssistant(finalText, msgMeta = null) {
  const text = (finalText && String(finalText).trim()) ? String(finalText) : liveStreamText;
  liveStreamText = '';
  liveRenderPending = false;
  if (!streamEl) return;
  const liveRow = streamEl.querySelector('.msg-row.assistant[data-live="true"]');
  if (liveRow) {
    delete liveRow.dataset.live;
    if (text && text.trim()) {
      const body = liveRow.querySelector('.msg-assistant-body');
      if (body) renderRichInto(body, text);
      const verdictCard = buildVerificationVerdictCard(msgMeta, onInspectStep);
      if (verdictCard) liveRow.appendChild(verdictCard);
      scrollToBottom();
    } else {
      liveRow.remove();
    }
    return;
  }
  if (text && text.trim()) {
    appendAssistantMessage(text, '', [], msgMeta);
  }
}

export function finalizeLiveOnError() {
  const partial = liveStreamText;
  liveStreamText = '';
  liveRenderPending = false;
  if (!streamEl) return;
  const liveRow = streamEl.querySelector('.msg-row.assistant[data-live="true"]');
  if (liveRow) {
    delete liveRow.dataset.live;
    if (!partial || !partial.trim()) {
      liveRow.remove();
    } else {
      const body = liveRow.querySelector('.msg-assistant-body');
      if (body) renderRichInto(body, partial);
    }
  }
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
  const item = buildStructuredTraceCard(merged, { onInspectStep });
  if (existing) {
    existing.replaceWith(item);
  } else {
    list.appendChild(item);
  }
  const summarySpan = acc.querySelector('summary > span');
  if (summarySpan) {
    summarySpan.textContent = `Activity & reasoning (${list.children.length} tool call${list.children.length === 1 ? '' : 's'})`;
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
  const effect = String(req.effect || '');
  const allowSessionApprove = Boolean(req.allow_session_approve);
  const containmentBadge =
    req.contained || scope === 'worker-session'
      ? '<span class="badge ok">CONTAINED AppContainer</span>'
      : effect === 'network-read'
        ? '<span class="badge accent">Literature Read</span>'
        : effect === 'write-workspace'
          ? '<span class="badge accent">Workspace Edit</span>'
          : '<span class="badge warn">ONCE Approval</span>';

  const sessionHint = allowSessionApprove
    ? (effect === 'execute-code'
        ? 'Tip: Click "Approve for Session" once to run all sandboxed SymPy steps in this chat without asking every time.'
        : effect === 'write-workspace'
          ? 'Tip: Click "Approve for Session" once to allow workspace edits in this chat without asking on every patch.'
          : 'Tip: Click "Approve for Session" once to allow literature lookups in this chat without asking every time.')
    : '';

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
    ${sessionHint ? `<div class="approval-session-hint">${escapeHtml(sessionHint)}</div>` : ''}
    <div class="approval-actions">
      <button type="button" class="pill-btn sm approval-decline-btn">Decline</button>
      ${
        allowSessionApprove
          ? '<button type="button" class="pill-btn sm approval-once-btn" title="Approve this single tool call only">Approve Once</button>' +
            '<button type="button" class="pill-btn sm primary approval-session-btn" title="Approve all calls of this capability in this chat session">Approve for Session</button>'
          : '<button type="button" class="pill-btn sm primary approval-approve-btn">Approve &amp; Run</button>'
      }
    </div>
  `;

  const declineBtn = card.querySelector('.approval-decline-btn');
  const onceBtn = card.querySelector('.approval-once-btn');
  const sessBtn = card.querySelector('.approval-session-btn');
  const approveBtn = card.querySelector('.approval-approve-btn');
  const allBtns = [declineBtn, onceBtn, sessBtn, approveBtn].filter(Boolean);

  const submitDecision = async (approved, approveForSession = false) => {
    if (!currentSessionId) return;
    for (const b of allBtns) b.disabled = true;
    try {
      await api.respondApproval(currentSessionId, callId, {
        digest,
        approved,
        approve_for_session: approveForSession,
      });
      resolveApprovalCard(callId, approved, approveForSession);
    } catch (err) {
      for (const b of allBtns) b.disabled = false;
      if (typeof onNotify === 'function') onNotify(`Approval error: ${err.message}`);
    }
  };
  declineBtn.addEventListener('click', () => submitDecision(false, false));
  if (onceBtn) onceBtn.addEventListener('click', () => submitDecision(true, false));
  if (sessBtn) sessBtn.addEventListener('click', () => submitDecision(true, true));
  if (approveBtn) approveBtn.addEventListener('click', () => submitDecision(true, false));

  inner.appendChild(card);
  scrollToBottom();
}

export function resolveApprovalCard(callId, approved, approveForSession = false) {
  if (!streamEl) return;
  const card = streamEl.querySelector(`.approval-card[data-call-id="${CSS.escape(String(callId || ''))}"]`);
  if (!card) return;
  const actions = card.querySelector('.approval-actions');
  if (actions) {
    actions.innerHTML = approved
      ? approveForSession
        ? '<span class="badge ok">Approved for session</span>'
        : '<span class="badge ok">Approved</span>'
      : '<span class="badge err">Declined</span>';
  }
}
