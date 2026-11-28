// Centre-stage Solve setup card & first screen (package S5b; `solve.md` 2, 3, D2, D4, D5, D15, D20, D21;
// `00-overview.md` 5.1, 8.3 #9). Zero modal dialogs; works with zero model connected; zero `innerHTML` writes.

import { createApprovalCard } from './approval_card.js';
import { keepFocus } from './dom_focus.js';
import {
  checkboxRow,
  el,
  renderChecklistSection,
  renderDimensionsSection,
  renderModelsSection,
  renderSafeMathPreview,
  renderSymbolsSection,
} from './solve_setup_fields.js';
import {
  ASK_BEFORE_RUNNING_LABEL,
  NO_MODEL_BANNER,
  PROVE_PROSE_NOTICE,
  SAME_MODEL_WARNING,
  TASK_KINDS,
  applyPrefillToState,
  applySolveViewToState,
  buildLocalGivenPreviewSteps,
  buildSetupSubmission,
  createInitialSolveState,
  isSameModelSelection,
  markFieldOwned,
  prefillTaskKindOverride,
  setupCardSig,
  summarizeCollapsedSetup,
  switchTaskKind,
} from './solve_state.js';
import { createBadge } from './ui_badge.js';

export { renderSafeMathPreview };

export function createSolveSetupView(host, options = {}) {
  if (!host) throw new Error('createSolveSetupView requires a host element');

  let state = createInitialSolveState(options.initial || {});
  const onPrefill = typeof options.onPrefill === 'function' ? options.onPrefill : null;
  const onSubmitSetup = typeof options.onSubmitSetup === 'function' ? options.onSubmitSetup : null;
  const onAllowGrant = typeof options.onAllowGrant === 'function' ? options.onAllowGrant : null;
  const onDenyGrant = typeof options.onDenyGrant === 'function' ? options.onDenyGrant : null;
  const onRecheck = typeof options.onRecheck === 'function' ? options.onRecheck : null;
  const katexImpl = options.katex ?? globalThis.window?.katex ?? null;

  let approvalCardApi = null, activeRunBtn = null, prefillTimer = null, prefillSeq = 0, submitting = false;
  const root = el('section', 'solve-stage');
  root.setAttribute('data-component', 'solve-setup-stage');

  function updateSameModelFlags() {
    state.sameModel = isSameModelSelection(state.solver, state.verifier, state.connectedModels);
    state.sameModelWarning = state.sameModel ? SAME_MODEL_WARNING : null;
  }

  async function triggerPrefill() {
    if (!onPrefill) return;
    const seq = ++prefillSeq;
    try {
      const resp = await onPrefill({
        statement: state.statement,
        conventions: state.conventions,
        task_kind: prefillTaskKindOverride(state),
        solver: state.solver,
        verifier: state.verifier,
        allow_numeric_target: state.allowNumericTarget,
      });
      if (seq !== prefillSeq) return;
      if (resp) { state = applyPrefillToState(state, resp); render(); }
    } catch (err) {
      if (seq !== prefillSeq) return;
      state.error = String(err?.message || err || 'Prefill failed');
      render();
    }
  }

  function schedulePrefill(ev) {
    if (!onPrefill || ev?.isComposing) return;
    if (prefillTimer) clearTimeout(prefillTimer);
    prefillTimer = setTimeout(() => { triggerPrefill(); }, options.debounceMs ?? 250);
  }

  async function submitSetup() {
    if (submitting) return null;
    if (prefillTimer) { clearTimeout(prefillTimer); prefillTimer = null; }
    prefillSeq += 1;
    const payload = buildSetupSubmission(state);
    if (!payload.statement) {
      state.error = 'Enter a problem statement before running.';
      render();
      return null;
    }
    state.error = '';
    if (!onSubmitSetup) {
      state = applySolveViewToState(state, {
        setup: { ...payload, revision: (state.setup?.revision || 0) + 1 },
        steps: buildLocalGivenPreviewSteps(payload),
      });
      state.phase = 'collapsed';
      render();
      return payload;
    }
    submitting = true;
    if (activeRunBtn) activeRunBtn.disabled = true;
    try {
      const res = await onSubmitSetup(payload);
      const view = res?.view || {
        setup: res?.setup || { ...payload, revision: (state.setup?.revision || 0) + 1 },
        steps: res?.steps || buildLocalGivenPreviewSteps(payload),
      };
      state.phase = 'collapsed';
      state = applySolveViewToState(state, view);
      return res;
    } catch (err) {
      state.error = String(err?.message || err || 'Setup submission failed');
      return null;
    } finally {
      submitting = false;
      render();
    }
  }

  function renderCollapsed() {
    const bar = el('div', 'solve-collapsed-bar');
    bar.setAttribute('data-testid', 'solve-collapsed-bar');
    bar.append(el('span', 'solve-collapsed-summary', summarizeCollapsedSetup(state.setup)));

    const btns = el('div', 'solve-collapsed-actions');
    const editBtn = el('button', 'pill-btn sm', 'Edit setup');
    editBtn.type = 'button';
    editBtn.setAttribute('data-action', 'edit-setup');
    editBtn.addEventListener('click', () => {
      state.phase = 'revising';
      render();
    });
    btns.append(editBtn);

    const recheckBtn = el('button', 'pill-btn sm', 'Re-run checks');
    recheckBtn.type = 'button';
    recheckBtn.setAttribute('data-action', 'recheck');
    recheckBtn.addEventListener('click', async () => {
      if (onRecheck) {
        await onRecheck(state.setup);
      } else {
        state.recheckNotice = 'Re-running checks works without a model once checks exist (S9).';
        render();
      }
    });
    btns.append(recheckBtn);
    bar.append(btns);
    root.append(bar);

    if (state.recheckNotice) {
      root.append(el('div', 'solve-info-banner', state.recheckNotice));
    }

    const givenWrap = el('div', 'solve-given-list');
    givenWrap.setAttribute('data-testid', 'solve-given-steps');
    const steps = state.steps.filter((s) => !s.superseded_by);
    const allGiven = steps.length === 0 || steps.every((s) => (s.badge?.label || 'Given') === 'Given');
    givenWrap.append(el('h3', 'solve-section-title', allGiven ? 'Given steps' : 'Steps'));
    if (steps.length === 0) {
      givenWrap.append(el('p', 'solve-muted', 'No setup conventions or assumptions recorded.'));
    } else {
      for (const step of steps) {
        const card = el('article', 'solve-given-card');
        card.setAttribute('data-entry-id', String(step.entry_id || ''));
        const head = el('div', 'solve-given-head');
        head.append(el('span', 'solve-step-id', String(step.entry_id || '')));
        head.append(el('strong', 'solve-step-summary', String(step.summary || '')));
        const badgeLabel = step.badge?.label || 'Given';
        head.append(createBadge({
          text: badgeLabel,
          icon: step.badge?.icon || 'user',
          tone: step.badge?.tone || 'neutral',
        }));
        if (step.badge?.provenance_tag) {
          head.append(el('span', 'mini-pill solve-tag-you', String(step.badge.provenance_tag)));
        }
        card.append(head);
        if (step.details) {
          card.append(el('p', 'solve-step-details', String(step.details)));
        }
        givenWrap.append(card);
      }
    }
    root.append(givenWrap);
  }

  function renderSetupCard() {
    const card = el('div', 'solve-setup-card');
    card.setAttribute('data-testid', 'solve-setup-card');

    const head = el('div', 'solve-setup-head');
    head.append(el('h2', 'solve-setup-title', state.phase === 'revising' ? 'Revise Solve setup' : 'Solve a problem'));
    const toolboxLine = el('span', 'solve-toolbox-line', state.toolbox?.line || 'Toolbox: unavailable');
    toolboxLine.setAttribute('data-testid', 'solve-toolbox-line');
    head.append(toolboxLine);
    card.append(head);

    // 3-way kind switch: Calculate / Prove / Check a derivation.
    const kindBar = el('div', 'solve-kind-switch');
    kindBar.setAttribute('role', 'radiogroup');
    kindBar.setAttribute('aria-label', 'Solve task kind');
    for (const kind of TASK_KINDS) {
      const btn = el('button', state.taskKind === kind.id ? 'seg-btn is-active' : 'seg-btn', kind.label);
      btn.type = 'button';
      btn.setAttribute('data-kind', kind.id);
      btn.setAttribute('aria-pressed', state.taskKind === kind.id ? 'true' : 'false');
      btn.addEventListener('click', () => {
        if (switchTaskKind(state, kind.id)) render();
      });
      kindBar.append(btn);
    }
    card.append(kindBar);

    if (state.taskKind === 'prove') {
      const notice = el('div', 'solve-prove-notice', state.proveNotice || PROVE_PROSE_NOTICE);
      notice.setAttribute('data-testid', 'solve-prove-notice');
      card.append(notice);
    }

    // Statement input + live KaTeX preview.
    const stmtField = el('div', 'solve-field');
    stmtField.append(el('label', 'solve-label', 'Problem statement (TeX supported, max 16,000 chars)'));
    const stmtIn = el('textarea', 'text-input solve-statement-input');
    stmtIn.rows = 4;
    stmtIn.value = state.statement;
    stmtIn.placeholder = 'State the calculation, proof, or derivation to check…';
    stmtIn.setAttribute('maxlength', '16000');
    stmtIn.setAttribute('data-testid', 'solve-statement-input');
    const preview = el('div', 'solve-katex-preview');
    preview.setAttribute('data-testid', 'solve-katex-preview');
    renderSafeMathPreview(preview, state.statement, katexImpl);
    stmtIn.addEventListener('input', (ev) => {
      state.statement = ev.target.value;
      renderSafeMathPreview(preview, state.statement, katexImpl);
      schedulePrefill(ev);
    });
    stmtIn.addEventListener('compositionend', (ev) => {
      state.statement = ev.target.value;
      renderSafeMathPreview(preview, state.statement, katexImpl);
      schedulePrefill();
    });
    stmtField.append(stmtIn, preview);
    card.append(stmtField);

    renderSymbolsSection(card, state, render);

    // Conventions textarea (max 2,000 chars).
    const convField = el('div', 'solve-field');
    convField.append(el('label', 'solve-label', 'Conventions (max 2,000 chars)'));
    const convIn = el('textarea', 'text-input solve-conventions-input');
    convIn.rows = 2;
    convIn.value = state.conventions;
    convIn.placeholder = 'Euclidean signature, hbar = c = 1, MS-bar…';
    convIn.setAttribute('maxlength', '2000');
    convIn.setAttribute('data-testid', 'solve-conventions-input');
    convIn.addEventListener('input', (ev) => {
      state.conventions = ev.target.value;
      markFieldOwned(state, 'conventions');
    });
    convField.append(convIn);
    card.append(convField);

    renderDimensionsSection(card, state, render);
    renderChecklistSection(card, state, render);
    renderModelsSection(card, state, updateSameModelFlags, render);

    // Shared approval card (S5a, D5).
    const approvalHost = el('div', 'solve-approval-host');
    approvalHost.setAttribute('data-testid', 'solve-approval-host');
    approvalCardApi = createApprovalCard(approvalHost, {
      title: 'Approvals for this Solve run',
      grants: state.approvalGrants?.grants || [],
      preTurnSupported: state.approvalGrants?.pre_turn_approver_supported !== false,
      onAllowGrant,
      onDenyGrant,
      onChange: (nextGrants) => {
        state.approvalGrants = { ...(state.approvalGrants || {}), grants: nextGrants };
      },
    });
    card.append(approvalHost);

    // D20 opt-in switch + Run button.
    const footer = el('div', 'solve-setup-footer');
    footer.append(checkboxRow(ASK_BEFORE_RUNNING_LABEL, state.askBeforeRunning, 'solve-ask-before-running', (c) => {
      state.askBeforeRunning = c;
    }));

    const actions = el('div', 'solve-setup-actions');
    if (state.phase === 'revising') {
      const cancelBtn = el('button', 'pill-btn', 'Cancel');
      cancelBtn.type = 'button';
      cancelBtn.setAttribute('data-action', 'cancel-revision');
      cancelBtn.addEventListener('click', () => { state.phase = 'collapsed'; render(); });
      actions.append(cancelBtn);
    }
    const runBtn = el('button', 'pill-btn primary solve-run-btn', state.phase === 'revising' ? 'Save revision' : 'Run');
    runBtn.type = 'button';
    runBtn.disabled = submitting;
    runBtn.setAttribute('data-testid', 'solve-run-btn');
    runBtn.addEventListener('click', () => { submitSetup(); });
    activeRunBtn = runBtn;
    actions.append(runBtn);
    footer.append(actions);
    card.append(footer);

    if (state.error) {
      const err = el('div', 'solve-error-banner', state.error);
      err.setAttribute('data-testid', 'solve-error-banner');
      card.append(err);
    }

    root.append(card);
  }

  function render() {
    keepFocus(root, () => {
      activeRunBtn = null;
      root.replaceChildren();
      if (!state.modelConnected) {
        const banner = el('div', 'solve-no-model-banner', NO_MODEL_BANNER);
        banner.setAttribute('data-testid', 'solve-no-model-banner');
        root.append(banner);
      }
      if (state.phase === 'collapsed' && state.setup) {
        renderCollapsed();
      } else {
        renderSetupCard();
      }
    });
  }

  host.replaceChildren(root);
  render();

  return {
    element: root,
    getState: () => ({ ...state }),
    getApprovalCard: () => approvalCardApi,
    applyPrefill(resp) {
      state = applyPrefillToState(state, resp);
      render();
    },
    applyView(view) {
      const prevPhase = state.phase;
      const prevSig = setupCardSig(state);
      state = applySolveViewToState(state, view);
      if (prevPhase !== 'collapsed' && state.phase !== 'collapsed' && setupCardSig(state) === prevSig) {
        return;
      }
      render();
    },
    submitSetup,
    triggerPrefill,
  };
}
