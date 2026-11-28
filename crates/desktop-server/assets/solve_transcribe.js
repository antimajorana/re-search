// Solve S17b Check-a-derivation UI: TeX-only drop/paste intake (PDF rejected),
// side-by-side "your TeX" vs "as checked" cards (R5 handle-only), hash-bound
// "transcription (model), unconfirmed" flag, and [Confirm] / [Confirm all] (D13).
// Plain ES module; zero `innerHTML` writes (DOM via createElement + textContent only).

import {
  SOLVE_TIER_LABELS, createSolveBadgeGroup, normalizeSolveTier,
} from './solve_badges.js';
import {
  buildHandleLookup, renderClaimFormula, renderHandleLatex,
} from './solve_exprs.js';
import { registerSolveTab } from './solve_steps.js';
import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';

export const TRANSCRIPTION_UNCONFIRMED_FLAG = 'transcription unconfirmed';
export const TRANSCRIPTION_UNCONFIRMED_TEXT = 'transcription (model), unconfirmed';
export const PDF_REJECT_MESSAGE =
  'PDF derivations are not supported yet; provide a .tex file or paste LaTeX directly.';

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

/** Fail-closed client-side PDF rejection matching S17a (`00-overview.md` 2.12). */
export function isPdfInput(fileName = '', content = '', mimeType = '') {
  if (/^application\/pdf\b/i.test(String(mimeType || '').trim())) return true;
  if (/\.pdf$/i.test(String(fileName || '').trim())) return true;
  const raw = String(content || '').replace(/^\uFEFF/, '').trimStart();
  return raw.startsWith('%PDF-');
}

/**
 * Determine whether a transcribed step is confirmed for its CURRENT `entry_sha256`.
 * A stale or mismatched `entry_sha256` NEVER clears the unconfirmed flag.
 */
export function isTranscriptionConfirmed(step, acks = []) {
  if (!step || !step.transcription) return true;
  const sha = typeof step.entry_sha256 === 'string' ? step.entry_sha256.trim() : '';
  if (!sha) return false;
  const entryId = String(step.entry_id || '');
  const list = Array.isArray(acks) ? acks : [];
  const stepAcks = list.filter((a) => a && a.kind === 'transcription' && String(a.entry_id) === entryId);
  if (stepAcks.length > 0) {
    return stepAcks.some((a) => String(a.entry_sha256 || '').trim() === sha);
  }
  const flags = step.badge?.flags;
  if (Array.isArray(flags)) return !flags.includes(TRANSCRIPTION_UNCONFIRMED_FLAG);
  return false;
}

export function collectTranscribedSteps(view) {
  const steps = Array.isArray(view?.steps) ? view.steps : [];
  return steps.filter((s) => s && !s.superseded_by && Boolean(s.transcription));
}

export function deriveConsecutivePairs(steps) {
  const list = Array.isArray(steps) ? steps : [];
  const byId = new Map(list.map((s) => [String(s.entry_id), s]));
  const pairs = [];
  for (let i = 0; i < list.length; i += 1) {
    const cur = list[i];
    const allDeps = [
      ...(Array.isArray(cur.depends_on) ? cur.depends_on : []),
      ...(Array.isArray(cur.derived_depends_on) ? cur.derived_depends_on : []),
    ].map(String);
    const prevDep = allDeps.find((d) => byId.has(d));
    if (prevDep) pairs.push({ index: pairs.length + 1, from: byId.get(prevDep), to: cur });
  }
  return pairs;
}

function safePairLabel(step) {
  const badge = step?.badge || {};
  if (badge.stale) return SOLVE_TIER_LABELS.stale;
  const eff = normalizeSolveTier(badge.effective || badge.own);
  const label = String(badge.label || SOLVE_TIER_LABELS[eff] || SOLVE_TIER_LABELS.unchecked).trim();
  if (eff !== 'verified' && /\b(verified|proved|new|novel)\b/i.test(label)) {
    return SOLVE_TIER_LABELS[eff] || SOLVE_TIER_LABELS.unchecked;
  }
  return label;
}

function resolveCsrfToken(doc, explicitToken) {
  if (explicitToken) return String(explicitToken);
  const meta = typeof doc?.querySelector === 'function' ? doc.querySelector('meta[name="re-search-csrf"]') : null;
  if (meta?.getAttribute?.('content')) return meta.getAttribute('content');
  try {
    if (typeof sessionStorage !== 'undefined') return sessionStorage.getItem('re-search-token') || '';
  } catch { /* ignore */ }
  return '';
}

function prepareLeftPreviewTex(rawTex) {
  const s = String(rawTex || '').trim();
  if (!s) return '';
  if (/(^|[^\\])&/.test(s) && !/\\begin\{/.test(s)) return `\\begin{aligned}${s}\\end{aligned}`;
  return s;
}

async function defaultJsonPost(fetchFn, url, payload, token) {
  const headers = { 'Content-Type': 'application/json' };
  if (token) headers['X-ReSearch-Token'] = token;
  const res = await fetchFn(url, {
    method: 'POST', credentials: 'same-origin', headers, body: JSON.stringify(payload),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(data?.error || data?.message || `HTTP ${res.status}`);
  return data;
}

/**
 * Mount the S17b Check-a-derivation two-column view inside `host`.
 */
export function createSolveTranscribeView(host, initialView = {}, opts = {}) {
  if (!host) throw new Error('createSolveTranscribeView requires a host element');
  const doc = opts.document || host.ownerDocument || document;
  let view = initialView && typeof initialView === 'object' ? initialView : {};
  let localAcks = Array.isArray(opts.acks) ? [...opts.acks] : [];
  let errorMsg = '';
  let statusMsg = '';
  let busy = false;
  let destroyed = false;
  let draftFileName = 'derivation.tex';
  let draftTex = '';
  let replaceExisting = false;

  const root = el(doc, 'section', 'solve-transcribe-view', null, { 'data-testid': 'solve-transcribe-view' });
  host.replaceChildren(root);

  function mergedAcks() {
    const fromView = Array.isArray(view.acks) ? view.acks : [];
    return [...fromView, ...localAcks];
  }

  function applyAckResponse(step, res) {
    if (!res || typeof res !== 'object' || res.ok === false) {
      throw new Error(res?.error || 'Confirmation was not accepted by the server.');
    }
    if (res.view && typeof res.view === 'object') {
      view = res.view;
      localAcks = [];
      return;
    }
    const ack = res.ack;
    const badgeStillUnconfirmed = Array.isArray(res.badge?.flags)
      && res.badge.flags.includes(TRANSCRIPTION_UNCONFIRMED_FLAG);
    if (!ack || badgeStillUnconfirmed
      || String(ack.entry_id) !== String(step.entry_id)
      || ack.kind !== 'transcription'
      || String(ack.entry_sha256 || '').trim() !== String(step.entry_sha256 || '').trim()) {
      throw new Error(`Confirmation response did not match entry_sha256 for ${step.entry_id}.`);
    }
    localAcks.push({
      entry_id: String(ack.entry_id),
      kind: 'transcription',
      entry_sha256: String(ack.entry_sha256),
    });
  }

  async function sendAck(step) {
    const payload = {
      entry_id: String(step.entry_id),
      kind: 'transcription',
      entry_sha256: String(step.entry_sha256 || ''),
    };
    let res;
    if (typeof opts.onAck === 'function') {
      res = await opts.onAck(step, payload);
    } else if (opts.sessionId && (opts.fetch || typeof fetch === 'function')) {
      const url = `/api/sessions/${encodeURIComponent(opts.sessionId)}/solve/ack`;
      res = await defaultJsonPost(opts.fetch || fetch, url, payload, resolveCsrfToken(doc, opts.token));
    } else {
      throw new Error('No acknowledgement handler configured.');
    }
    applyAckResponse(step, res);
    return res;
  }

  async function handleConfirmOne(step) {
    if (busy || destroyed) return;
    busy = true; errorMsg = ''; statusMsg = ''; render();
    try {
      await sendAck(step);
      statusMsg = `Confirmed ${step.entry_id}.`;
    } catch (err) {
      errorMsg = String(err?.message || err || 'Confirmation failed.');
    } finally {
      busy = false;
      if (!destroyed) render();
    }
  }

  async function handleConfirmAll(unconfirmedSteps) {
    if (busy || destroyed || unconfirmedSteps.length === 0) return;
    busy = true; errorMsg = ''; statusMsg = ''; render();
    let count = 0;
    try {
      for (const step of unconfirmedSteps) {
        if (destroyed) return;
        await sendAck(step);
        count += 1;
      }
      statusMsg = `Confirmed ${count} transcribed step${count === 1 ? '' : 's'}.`;
    } catch (err) {
      const detail = String(err?.message || err || 'Confirm all failed.');
      errorMsg = count > 0 ? `Confirmed ${count}, then failed: ${detail}` : detail;
    } finally {
      busy = false;
      if (!destroyed) render();
    }
  }

  async function handlePickedFile(file, fileInput) {
    if (!file || destroyed) return;
    const candidateName = String(file.name || 'derivation.tex');
    const candidateMime = String(file.type || '');
    if (isPdfInput(candidateName, '', candidateMime)) {
      if (fileInput) fileInput.value = '';
      errorMsg = PDF_REJECT_MESSAGE; statusMsg = ''; render(); return;
    }
    if (typeof file.text === 'function') {
      const text = await file.text();
      if (destroyed) return;
      if (isPdfInput(candidateName, text, candidateMime)) {
        if (fileInput) fileInput.value = '';
        errorMsg = PDF_REJECT_MESSAGE; statusMsg = ''; render(); return;
      }
      draftFileName = candidateName;
      draftTex = text;
      errorMsg = '';
      render();
    }
  }

  async function handleSubmitTex(fileName, tex, replace) {
    if (busy || destroyed) return;
    errorMsg = ''; statusMsg = '';
    if (isPdfInput(fileName, tex)) {
      errorMsg = PDF_REJECT_MESSAGE; render(); return;
    }
    if (!String(tex || '').trim()) {
      errorMsg = 'Provide a .tex file or paste LaTeX equations to split.'; render(); return;
    }
    busy = true; render();
    const payload = { file_name: fileName || 'derivation.tex', tex, replace: Boolean(replace) };
    try {
      let res;
      if (typeof opts.onTranscribe === 'function') {
        res = await opts.onTranscribe(payload);
      } else if (opts.sessionId && (opts.fetch || typeof fetch === 'function')) {
        const url = `/api/sessions/${encodeURIComponent(opts.sessionId)}/solve/transcribe`;
        res = await defaultJsonPost(opts.fetch || fetch, url, payload, resolveCsrfToken(doc, opts.token));
      } else {
        throw new Error('No transcribe handler configured.');
      }
      if (destroyed) return;
      if (res && typeof res === 'object' && res.view) {
        view = res.view;
        localAcks = [];
      }
      const n = res?.equation_count ?? collectTranscribedSteps(view).length;
      statusMsg = `Split ${n} equation${n === 1 ? '' : 's'} deterministically (0 model calls).`;
    } catch (err) {
      errorMsg = String(err?.message || err || 'Failed to split TeX derivation.');
    } finally {
      busy = false;
      if (!destroyed) render();
    }
  }

  function renderIntakeForm(hasExisting) {
    const form = el(doc, 'form', 'solve-transcribe-form', null, { 'data-testid': 'solve-transcribe-form' });
    const row = el(doc, 'div', 'solve-transcribe-controls');

    const fileInput = el(doc, 'input', 'solve-transcribe-file', null, {
      type: 'file', accept: '.tex,.ltx,text/x-tex', 'data-testid': 'solve-transcribe-file',
    });
    fileInput.addEventListener('change', () => handlePickedFile(fileInput.files?.[0], fileInput));

    const nameInput = el(doc, 'input', 'solve-why-input', null, {
      name: 'file_name', placeholder: 'derivation.tex', 'data-testid': 'solve-transcribe-filename',
    });
    nameInput.value = draftFileName;
    nameInput.addEventListener('input', () => { draftFileName = nameInput.value; });

    row.append(fileInput, nameInput);
    if (hasExisting) {
      const repLbl = el(doc, 'label', 'solve-check-row');
      const repBox = el(doc, 'input', null, null, { type: 'checkbox', 'data-testid': 'solve-transcribe-replace' });
      repBox.checked = replaceExisting;
      repBox.addEventListener('change', () => { replaceExisting = Boolean(repBox.checked); });
      repLbl.append(repBox, el(doc, 'span', '', 'Replace existing'));
      row.append(repLbl);
    }

    const submitBtn = el(doc, 'button', 'pill-btn sm primary', busy ? 'Splitting…' : 'Split TeX derivation', {
      type: 'submit', 'data-testid': 'solve-transcribe-submit',
    });
    submitBtn.disabled = busy;
    row.append(submitBtn);

    const texArea = el(doc, 'textarea', 'solve-transcribe-input', null, {
      name: 'tex', rows: '4', placeholder: 'Drop or paste LaTeX derivation (.tex only; PDF is not supported yet)',
      'data-testid': 'solve-transcribe-tex',
    });
    texArea.value = draftTex;
    texArea.addEventListener('input', () => { draftTex = texArea.value; });

    form.append(row, texArea);
    form.addEventListener('dragover', (ev) => { ev.preventDefault(); });
    form.addEventListener('drop', (ev) => {
      ev.preventDefault();
      const dropped = ev.dataTransfer?.files?.[0];
      if (dropped) handlePickedFile(dropped, fileInput);
    });
    form.addEventListener('submit', (ev) => {
      ev.preventDefault();
      const candidateName = nameInput.value.trim() || 'derivation.tex';
      const candidateTex = texArea.value;
      if (isPdfInput(candidateName, candidateTex)) {
        errorMsg = PDF_REJECT_MESSAGE; statusMsg = ''; render(); return;
      }
      draftFileName = candidateName;
      draftTex = candidateTex;
      handleSubmitTex(draftFileName, draftTex, replaceExisting);
    });
    return form;
  }

  function renderStepCard(step, handleMap, acks) {
    const confirmed = isTranscriptionConfirmed(step, acks);
    const eff = normalizeSolveTier(step.badge?.effective || step.badge?.own);
    const isFailed = eff === 'failed' || normalizeSolveTier(step.badge?.own) === 'failed';
    const cls = `solve-transcribe-card${isFailed ? ' solve-transcribe-card--failed' : ''}${confirmed ? ' is-confirmed' : ''}`;
    const card = el(doc, 'article', cls, null, {
      'data-testid': 'solve-transcribe-card',
      'data-entry-id': String(step.entry_id || ''),
      'data-confirmed': confirmed ? 'true' : 'false',
    });

    const head = el(doc, 'div', 'solve-transcribe-card-head');
    const idBtn = el(doc, 'button', 'solve-step-id-btn', String(step.entry_id || ''), { type: 'button' });
    idBtn.addEventListener('click', () => opts.onSelectStep?.(String(step.entry_id)));
    head.append(idBtn, el(doc, 'span', 'solve-transcribe-summary', String(step.summary || '')));

    const rawBadge = step.badge || {};
    const displayBadge = confirmed && Array.isArray(rawBadge.flags)
      ? { ...rawBadge, flags: rawBadge.flags.filter((f) => f !== TRANSCRIPTION_UNCONFIRMED_FLAG) }
      : rawBadge;
    head.append(createSolveBadgeGroup(displayBadge, { document: doc, onSelectStep: opts.onSelectStep }));

    const statusSlot = el(doc, 'div', 'solve-transcribe-confirm-slot');
    if (confirmed) {
      const okChip = createBadge({ text: 'Confirmed', icon: 'user', tone: 'neutral', border: BORDER_SOLID });
      okChip.setAttribute('data-testid', 'solve-transcription-confirmed');
      statusSlot.append(okChip);
    } else {
      const warnChip = createBadge({
        text: TRANSCRIPTION_UNCONFIRMED_TEXT, icon: 'warn', tone: 'warn', border: BORDER_DASHED,
      });
      warnChip.setAttribute('data-testid', 'solve-transcription-unconfirmed');
      const confirmBtn = el(doc, 'button', 'pill-btn sm', 'Confirm', {
        type: 'button', 'data-action': 'confirm-transcription', 'data-entry-id': String(step.entry_id || ''),
      });
      confirmBtn.disabled = busy;
      confirmBtn.addEventListener('click', () => handleConfirmOne(step));
      statusSlot.append(warnChip, confirmBtn);
    }
    head.append(statusSlot);
    card.append(head);

    // Side-by-side two-column comparison: left "your TeX", right "as checked" (R5 handle-only).
    const cols = el(doc, 'div', 'solve-transcribe-cols');

    const leftCol = el(doc, 'div', 'solve-transcribe-col solve-transcribe-col--yours', null, { 'data-col': 'your-tex' });
    leftCol.append(el(doc, 'div', 'solve-transcribe-col-title', 'your TeX'));
    const rawTex = String(step.details || '').trim();
    leftCol.append(el(doc, 'pre', 'solve-transcribe-raw', rawTex || '(empty)'));
    if (rawTex) {
      const preview = el(doc, 'div', 'solve-transcribe-preview');
      renderHandleLatex(preview, prepareLeftPreviewTex(rawTex), opts.katex, { display: true });
      leftCol.append(preview);
    }

    const rightCol = el(doc, 'div', 'solve-transcribe-col solve-transcribe-col--checked', null, { 'data-col': 'as-checked' });
    rightCol.append(el(doc, 'div', 'solve-transcribe-col-title', 'as checked'));
    const claims = Array.isArray(step.claims) ? step.claims : [];
    if (claims.length === 0) {
      rightCol.append(el(doc, 'p', 'solve-muted', 'Pending handle restatement (not checked yet).'));
    } else {
      for (const claim of claims) {
        const row = el(doc, 'div', 'solve-claim-formula');
        renderClaimFormula(row, claim, handleMap, opts.katex, { display: true });
        rightCol.append(row);
      }
    }

    cols.append(leftCol, rightCol);
    card.append(cols);
    return card;
  }

  function render() {
    if (destroyed) return;
    root.replaceChildren();
    const transcribed = collectTranscribedSteps(view);
    const acks = mergedAcks();
    const unconfirmed = transcribed.filter((s) => !isTranscriptionConfirmed(s, acks));
    const handleMap = buildHandleLookup(view.handles || []);

    const header = el(doc, 'header', 'solve-transcribe-header');
    header.append(el(doc, 'h3', 'solve-section-title', 'Check a derivation (TeX)'));
    const actions = el(doc, 'div', 'solve-transcribe-header-actions');
    const summaryText = transcribed.length === 0
      ? 'No TeX derivation split yet'
      : unconfirmed.length === 0
        ? `All ${transcribed.length} confirmed`
        : `${unconfirmed.length} of ${transcribed.length} unconfirmed`;
    actions.append(el(doc, 'span', 'solve-muted', summaryText, { 'data-testid': 'solve-transcribe-count' }));

    const confirmAllBtn = el(doc, 'button', 'pill-btn sm primary', 'Confirm all', {
      type: 'button', 'data-action': 'confirm-all-transcriptions', 'data-testid': 'solve-confirm-all',
    });
    confirmAllBtn.disabled = busy || unconfirmed.length === 0;
    confirmAllBtn.addEventListener('click', () => handleConfirmAll(unconfirmed));
    actions.append(confirmAllBtn);
    header.append(actions);
    root.append(header);

    if (errorMsg) {
      root.append(el(doc, 'div', 'solve-error-banner', errorMsg, { role: 'alert', 'data-testid': 'solve-transcribe-error' }));
    }
    if (statusMsg) {
      root.append(el(doc, 'div', 'solve-status-note', statusMsg, { role: 'status', 'data-testid': 'solve-transcribe-status' }));
    }

    root.append(renderIntakeForm(transcribed.length > 0));

    const pairs = deriveConsecutivePairs(transcribed);
    if (pairs.length > 0) {
      const strip = el(doc, 'div', 'solve-transcribe-pairs', null, { 'data-testid': 'solve-transcribe-pairs' });
      strip.append(el(doc, 'strong', 'solve-subhead', 'Consecutive step checks'));
      const row = el(doc, 'div', 'solve-badge-row');
      for (const p of pairs) {
        const toEff = normalizeSolveTier(p.to.badge?.effective || p.to.badge?.own);
        const failed = toEff === 'failed';
        const pill = el(
          doc, 'button',
          `solve-transcribe-pair${failed ? ' solve-transcribe-pair--failed' : ''}`,
          `${p.from.entry_id} → ${p.to.entry_id}: ${safePairLabel(p.to)}`,
          { type: 'button', 'data-pair-to': String(p.to.entry_id), ...(failed ? { 'data-failed': 'true' } : {}) },
        );
        pill.addEventListener('click', () => opts.onSelectStep?.(String(p.to.entry_id)));
        row.append(pill);
      }
      strip.append(row);
      root.append(strip);
    }

    const list = el(doc, 'div', 'solve-transcribe-list', null, { 'data-testid': 'solve-transcribe-list' });
    if (transcribed.length === 0) {
      list.append(el(doc, 'p', 'solve-muted', 'Drop or paste a .tex derivation to split equations into numbered steps.'));
    } else {
      for (const step of transcribed) list.append(renderStepCard(step, handleMap, acks));
    }
    root.append(list);
  }

  render();

  return {
    element: root,
    update(nextView, nextAcks) {
      view = nextView && typeof nextView === 'object' ? nextView : {};
      localAcks = Array.isArray(nextAcks) ? [...nextAcks] : [];
      render();
    },
    destroy() {
      destroyed = true;
      root.replaceChildren();
      if (root.parentNode) root.parentNode.removeChild(root);
    },
  };
}

export const SOLVE_TRANSCRIBE_TAB = Object.freeze({
  id: 'transcribe',
  label: 'Check derivation',
  mount(host, ctx = {}) {
    return createSolveTranscribeView(host, ctx.view || {}, {
      document: ctx.document, sessionId: ctx.sessionId, token: ctx.token,
      katex: ctx.katex, fetch: ctx.fetch, onAck: ctx.onAck,
      onTranscribe: ctx.onTranscribe, onSelectStep: ctx.onSelectStep,
    });
  },
});

export function registerSolveTranscribeTab(registry) {
  return registry !== undefined
    ? registerSolveTab(registry, SOLVE_TRANSCRIBE_TAB)
    : registerSolveTab(SOLVE_TRANSCRIBE_TAB);
}
