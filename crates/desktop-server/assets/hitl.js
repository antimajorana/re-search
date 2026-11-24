// Stage 15 Interactive Human-in-the-Loop UI: ask_user checkpoints, research plan approval,
// Ultramath parallel branch picker, and live mid-turn steer confirmation badges.
import * as api from '/assets/api.js';
import { escapeHtml, mathTextToHtml, renderMathTextInto, renderRichInto } from '/assets/math.js';
import { scrollToBottom } from '/assets/stream.js';

function getStreamInner(streamEl, clearHero = true) {
  if (!streamEl) return null;
  let inner = streamEl.querySelector('.stream-inner');
  if (!inner) {
    streamEl.innerHTML = '';
    inner = document.createElement('div');
    inner.className = 'stream-inner';
    streamEl.appendChild(inner);
  }
  if (clearHero) {
    const hero = inner.querySelector('.empty-hero');
    if (hero) hero.remove();
  }
  return inner;
}

function findCheckpointCard(streamEl, checkpointId) {
  if (!streamEl || !checkpointId) return null;
  const target = String(checkpointId);
  for (const node of streamEl.querySelectorAll('.checkpoint-card')) {
    if (node.dataset && node.dataset.checkpointId === target) return node;
  }
  return null;
}

function checkpointMeta(kind) {
  if (kind === 'branch') return { title: 'Ultramath Branch Picker', badge: 'branch' };
  if (kind === 'plan') return { title: 'Research Plan Checkpoint', badge: 'plan' };
  return { title: 'Orchestrator Question', badge: kind || 'decision' };
}

function buildBranchGridHtml(branches) {
  return `<div class="branch-grid">${branches.map((b, idx) => {
    const num = Number.isFinite(Number(b?.index)) ? Number(b.index) + 1 : idx + 1;
    const failed = b && b.ok === false;
    const statusCls = failed ? 'err' : 'ok';
    const statusTxt = failed ? 'error' : 'ready';
    const tools = Array.isArray(b?.tools_used) ? b.tools_used : [];
    const toolsHtml = tools.length
      ? `<div class="branch-tools">${tools.map((t) => `<code>${escapeHtml(t)}</code>`).join('')}</div>`
      : '';
    return `
      <div class="branch-card" data-branch-index="${idx}" role="button" tabindex="0">
        <div class="branch-card-head">
          <span>Branch ${num}</span>
          <span class="badge ${statusCls}">${statusTxt}</span>
        </div>
        <div class="branch-approach">${mathTextToHtml(b?.approach || `Approach ${num}`)}</div>
        <div class="branch-summary rich-prose"></div>
        ${toolsHtml}
      </div>
    `;
  }).join('')}</div>`;
}

export function renderCheckpointCard(streamEl, sessionId, prompt, { onResolved, onNotify } = {}) {
  if (!streamEl || !prompt) return;
  const checkpointId = String(prompt.checkpoint_id || '');
  if (checkpointId && findCheckpointCard(streamEl, checkpointId)) return;
  const inner = getStreamInner(streamEl, true);
  if (!inner) return;

  const kind = String(prompt.kind || 'decision');
  const meta = checkpointMeta(kind);
  const options = Array.isArray(prompt.options) ? prompt.options : [];
  const branches = kind === 'branch' && Array.isArray(prompt.branches) ? prompt.branches : [];
  const submitLabel = kind === 'branch' ? 'Submit Selection' : kind === 'plan' ? 'Send Response' : 'Submit';
  const cancelLabel = kind === 'branch' ? 'Dismiss' : 'Cancel';
  const placeholder = kind === 'branch'
    ? 'Optional steering note or instructions for adopting/combining...'
    : kind === 'plan'
      ? 'Optional plan edits, constraints, or approval notes...'
      : 'Type your answer or clarification...';

  const card = document.createElement('div');
  card.className = 'checkpoint-card';
  card.dataset.checkpointId = checkpointId;
  card.dataset.checkpointKind = kind;
  card.innerHTML = `
    <div class="checkpoint-head">
      <div class="checkpoint-title-wrap">
        <strong>${escapeHtml(meta.title)}</strong>
        <span class="badge accent">${escapeHtml(meta.badge)}</span>
      </div>
      <span class="checkpoint-status badge warn">Awaiting input</span>
    </div>
    <div class="checkpoint-body">
      <div class="checkpoint-question rich-prose"></div>
      ${prompt.context ? '<div class="checkpoint-context rich-prose"></div>' : ''}
      ${branches.length ? buildBranchGridHtml(branches) : ''}
      ${options.length ? '<div class="checkpoint-options"></div>' : ''}
      ${kind === 'branch'
        ? `<input type="text" class="text-input checkpoint-note" placeholder="${escapeHtml(placeholder)}">`
        : `<textarea class="text-input checkpoint-note" rows="${kind === 'plan' ? 3 : 2}" placeholder="${escapeHtml(placeholder)}"></textarea>`}
    </div>
    <div class="checkpoint-actions">
      <button type="button" class="pill-btn sm checkpoint-cancel-btn">${cancelLabel}</button>
      <button type="button" class="pill-btn sm primary checkpoint-submit-btn">${submitLabel}</button>
    </div>
  `;

  renderRichInto(card.querySelector('.checkpoint-question'), String(prompt.question || ''));
  if (prompt.context) {
    renderRichInto(card.querySelector('.checkpoint-context'), String(prompt.context));
  }
  if (branches.length) {
    const sumEls = card.querySelectorAll('.branch-summary');
    branches.forEach((b, idx) => {
      if (sumEls[idx]) renderRichInto(sumEls[idx], String(b?.summary || b?.error || ''));
    });
  }

  let selectedBranchOption = null;
  const noteEl = card.querySelector('.checkpoint-note');
  const optsBox = card.querySelector('.checkpoint-options');
  const cancelBtn = card.querySelector('.checkpoint-cancel-btn');
  const submitBtn = card.querySelector('.checkpoint-submit-btn');

  const submitReply = async ({ selectedOption = null, text = '', cancelled = false }) => {
    if (!sessionId || !checkpointId || card.dataset.resolved === 'true') return;
    const controls = Array.from(card.querySelectorAll('button, input, textarea'));
    for (const c of controls) c.disabled = true;
    const payload = { selected_option: selectedOption, text, cancelled };
    try {
      await api.answerCheckpoint(sessionId, checkpointId, payload);
      resolveCheckpointCard(streamEl, checkpointId, payload);
      if (typeof onResolved === 'function') onResolved({ checkpoint_id: checkpointId, ...payload });
      if (typeof onNotify === 'function') {
        onNotify(cancelled ? 'Checkpoint cancelled.' : 'Checkpoint response sent.');
      }
    } catch (err) {
      if (card.dataset.resolved !== 'true') {
        for (const c of controls) c.disabled = false;
      }
      if (typeof onNotify === 'function') onNotify(`Checkpoint error: ${err.message}`);
    }
  };

  if (optsBox) {
    for (const opt of options) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'pill-btn sm';
      renderMathTextInto(btn, opt);
      btn.addEventListener('click', () => {
        const noteText = noteEl ? noteEl.value.trim() : '';
        submitReply({ selectedOption: String(opt), text: noteText, cancelled: false });
      });
      optsBox.appendChild(btn);
    }
  }

  for (const bCard of card.querySelectorAll('.branch-card')) {
    const pickBranch = () => {
      if (card.dataset.resolved === 'true') return;
      const idx = Number(bCard.dataset.branchIndex);
      for (const other of card.querySelectorAll('.branch-card')) {
        other.classList.toggle('is-selected', other === bCard);
      }
      selectedBranchOption = options[idx] || `Branch ${idx + 1}`;
    };
    bCard.addEventListener('click', pickBranch);
    bCard.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') {
        ev.preventDefault();
        pickBranch();
      }
    });
  }

  if (noteEl && noteEl.tagName === 'INPUT') {
    noteEl.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        submitBtn?.click();
      }
    });
  }

  cancelBtn?.addEventListener('click', () => {
    const noteText = noteEl ? noteEl.value.trim() : '';
    submitReply({ selectedOption: null, text: noteText, cancelled: true });
  });

  submitBtn?.addEventListener('click', () => {
    const noteText = noteEl ? noteEl.value.trim() : '';
    if (!selectedBranchOption && !noteText) {
      if (typeof onNotify === 'function') {
        onNotify(kind === 'branch' ? 'Choose a branch or enter steering notes.' : 'Select an option or enter a reply.');
      }
      noteEl?.focus();
      return;
    }
    submitReply({ selectedOption: selectedBranchOption, text: noteText, cancelled: false });
  });

  inner.appendChild(card);
  scrollToBottom();
}

export function resolveCheckpointCard(streamEl, checkpointId, reply = {}) {
  const card = findCheckpointCard(streamEl, checkpointId);
  if (!card) return;
  card.dataset.resolved = 'true';
  card.classList.add('is-resolved');
  for (const ctrl of card.querySelectorAll('button, input, textarea')) {
    ctrl.disabled = true;
  }

  const cancelled = Boolean(reply.cancelled);
  const chosen = reply.selected_option ? String(reply.selected_option) : '';
  const note = reply.text ? String(reply.text).trim() : '';

  const headStatus = card.querySelector('.checkpoint-status');
  if (headStatus) {
    headStatus.className = `checkpoint-status badge ${cancelled ? 'warn' : 'ok'}`;
    headStatus.textContent = cancelled ? 'Cancelled' : 'Resolved';
  }

  const actions = card.querySelector('.checkpoint-actions');
  if (actions) {
    if (cancelled) {
      actions.innerHTML = '<span class="badge warn">Cancelled by researcher</span>';
    } else if (chosen) {
      const noteHtml = note ? `<span class="badge">${mathTextToHtml(note)}</span>` : '';
      actions.innerHTML = `<span class="badge ok">Selected: ${mathTextToHtml(chosen)}</span>${noteHtml}`;
    } else if (note) {
      actions.innerHTML = `<span class="badge ok">Replied: ${mathTextToHtml(note)}</span>`;
    } else {
      actions.innerHTML = '<span class="badge ok">Resolved</span>';
    }
  }
}

export function appendSteerBadge(streamEl, text) {
  const inner = getStreamInner(streamEl, true);
  if (!inner) return;
  const row = document.createElement('div');
  row.className = 'steer-badge-row';
  row.setAttribute('role', 'status');
  row.innerHTML = `<span class="steer-badge-pill">Steer queued -&gt; ${mathTextToHtml(text)}</span>`;
  inner.appendChild(row);
  scrollToBottom();
}
