// Floating Gemini composer pill: auto-resizing textarea, <=5 file / 10 MiB Base64 upload, workspace picker, toggles; the model comes from model_picker.js.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';
import { closeEffortWindow, getReasoningEffort } from '/assets/effort_picker.js';
import { getActiveSubagent } from '/assets/subagents.js';

const MAX_FILES = 5;
const MAX_FILE_BYTES = 10 * 1024 * 1024;

let formEl = null;
let inputEl = null;
let sendEl = null;
let stopEl = null;
let topbarStopEl = null;
let uploadEl = null;
let filesBoxEl = null;
let workspaceBtn = null;
let verifierChip = null;
let strategistChip = null;
let wsDialogEl = null;
let wsInputEl = null;
let callbacks = {};
const drafts = new Map();
let draftSessionId = null;
let activeDraft = createDraft();
let draftRevision = 0;
let filePickerRevision = null;
let composerVisible = true;
let sessionLoading = false;
let turnRunning = false;
let stopRequest = null;
let defaultSendHtml = '';
let currentRoute = null;
let defaultPlaceholder = 'Ask a derivation, verify a theorem, or audit a paper...';
let placeholderOverride = null;

export function initComposer(opts = {}) {
  callbacks = opts;
  formEl = document.getElementById('composer-pill');
  inputEl = document.getElementById('composer-input');
  if (inputEl && inputEl.placeholder) defaultPlaceholder = inputEl.placeholder;
  sendEl = document.getElementById('composer-send');
  defaultSendHtml = sendEl ? sendEl.innerHTML : 'Send';
  stopEl = document.getElementById('composer-stop');
  topbarStopEl = document.getElementById('topbar-stop-btn');
  uploadEl = document.getElementById('composer-upload-input');
  filesBoxEl = document.getElementById('composer-files');
  workspaceBtn = document.getElementById('workspace-attach-btn');
  verifierChip = document.getElementById('verifier-chip');
  strategistChip = document.getElementById('strategist-chip');
  wsDialogEl = document.getElementById('workspace-dialog');
  wsInputEl = document.getElementById('workspace-path-input');

  if (inputEl) {
    inputEl.addEventListener('input', resizeTextarea);
    inputEl.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' && !ev.shiftKey && !ev.isComposing && ev.keyCode !== 229) {
        ev.preventDefault();
        if (formEl) formEl.requestSubmit();
      }
    });
  }

  if (formEl) {
    formEl.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      await handleSubmit();
    });
  }

  if (uploadEl) {
    uploadEl.addEventListener('click', () => { filePickerRevision = draftRevision; });
    uploadEl.addEventListener('cancel', () => { filePickerRevision = null; });
    uploadEl.addEventListener('change', handleFileSelection);
  }

  const triggerStop = async () => {
    if (!turnRunning || stopRequest || typeof callbacks.onStopTurn !== 'function') return;
    const request = {};
    stopRequest = request;
    syncButtons();
    try {
      if (await callbacks.onStopTurn()) return;
    } catch (err) {
      notify(`Could not stop the turn. ${err.message}`);
    }
    if (stopRequest === request) {
      stopRequest = null;
      syncButtons();
    }
  };

  if (stopEl) stopEl.addEventListener('click', triggerStop);
  if (topbarStopEl) topbarStopEl.addEventListener('click', triggerStop);

  document.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Escape' || ev.defaultPrevented || ev.repeat || ev.isComposing || ev.keyCode === 229) return;
    if (wsDialogEl?.open || document.getElementById('settings-dialog')?.open ||
        document.getElementById('source-reader-dialog')?.open ||
        document.querySelector?.('dialog[open]')) return;
    if (ev.target?.closest?.('#investigation-panel :is(input,textarea,select)')) return;
    const effortPop = document.getElementById('effort-popover');
    if (effortPop && !effortPop.hidden) {
      ev.preventDefault();
      closeEffortWindow();
      return;
    }
    if (!turnRunning || document.getElementById('project-overview')?.hidden === false) return;
    const convOpen = document.getElementById('conventions-popover');
    if (convOpen && !convOpen.hidden) return;
    ev.preventDefault();
    triggerStop();
  });

  for (const chip of [verifierChip, strategistChip]) {
    if (chip) {
      chip.addEventListener('click', () => {
        const next = chip.getAttribute('aria-pressed') !== 'true';
        chip.setAttribute('aria-pressed', next ? 'true' : 'false');
      });
    }
  }

  wireWorkspaceModal();
}

function resizeTextarea() {
  if (!inputEl) return;
  inputEl.style.height = 'auto';
  inputEl.style.height = `${Math.min(inputEl.scrollHeight, 180)}px`;
}

export function setComposerPrompt(text, focusInput = true) {
  activeDraft.text = text || '';
  if (!inputEl) return;
  inputEl.value = activeDraft.text;
  resizeTextarea();
  if (focusInput) inputEl.focus();
}

export function reuseComposerPrompt(text) {
  if (!inputEl || !text) return;
  if (inputEl.value.trim() && inputEl.value.trim() !== text.trim()) {
    inputEl.focus();
    notify('Your current draft is still in the composer.');
    return;
  }
  setComposerPrompt(text);
}

export function submitComposerPrompt(text) {
  if (!inputEl || !text) return false;
  setComposerPrompt(text, false);
  if (!turnRunning && formEl) {
    formEl.requestSubmit();
    return true;
  }
  inputEl.focus();
  return false;
}

const CLI_ROUTES = new Set(['claude-cli', 'claude-code-cli', 'antigravity']);
const canSteer = () => !CLI_ROUTES.has(currentRoute);

function createDraft() {
  return { text: '', files: [], submitting: false, notice: '' };
}

function readDraftText(draft) {
  if (draft === activeDraft && inputEl) draft.text = inputEl.value;
  return draft.text;
}

export function setComposerSession(sessionId, { adoptDraft = false } = {}) {
  if (sessionId === draftSessionId) return;
  readDraftText(activeDraft);
  if (adoptDraft && draftSessionId === null && sessionId && !drafts.has(sessionId)) {
    // First Send keeps ownership of the draft while it creates its destination.
    drafts.delete(null);
  } else {
    if (activeDraft.text || activeDraft.files.length || activeDraft.submitting || activeDraft.notice) {
      drafts.set(draftSessionId, activeDraft);
    } else {
      drafts.delete(draftSessionId);
    }
    activeDraft = drafts.get(sessionId) || createDraft();
    draftRevision += 1;
  }
  draftSessionId = sessionId;
  if (uploadEl) uploadEl.value = '';
  renderDraft();
}

export function setComposerVisible(visible) {
  if (visible === composerVisible) return;
  composerVisible = visible;
  draftRevision += 1;
  showDraftNotice();
}

export function discardComposerDraft(sessionId) {
  drafts.delete(sessionId);
  if (draftSessionId !== sessionId) return;
  activeDraft = createDraft();
  draftRevision += 1;
  renderDraft();
}

function renderDraft() {
  setComposerPrompt(activeDraft.text, false);
  renderFileChips();
  syncButtons();
  showDraftNotice();
}

function showDraftNotice() {
  if (!composerVisible || !activeDraft.notice) return;
  notify(activeDraft.notice);
  activeDraft.notice = '';
}

function notifyDraft(draft, message) {
  draft.notice = message;
  if (draft === activeDraft) showDraftNotice();
}

function restoreDraftText(draft, text) {
  if (readDraftText(draft)) return;
  draft.text = text;
  if (draft === activeDraft) setComposerPrompt(text, false);
}

function syncButtons() {
  const { submitting } = activeDraft;
  const steerMode = turnRunning && canSteer();
  if (uploadEl) uploadEl.disabled = submitting;
  if (filesBoxEl) {
    for (const button of filesBoxEl.querySelectorAll('button')) button.disabled = submitting;
  }
  if (sendEl) {
    sendEl.disabled = submitting || sessionLoading || Boolean(stopRequest) || (turnRunning && !canSteer());
    sendEl.hidden = false;
    if (steerMode) {
      sendEl.textContent = 'Steer';
      sendEl.classList.add('is-steer');
      sendEl.title = 'Send live steer to active turn (Enter)';
      sendEl.setAttribute('aria-label', 'Steer active turn');
    } else {
      if (defaultSendHtml && !sendEl.querySelector('svg')) sendEl.innerHTML = defaultSendHtml;
      sendEl.classList.remove('is-steer');
      sendEl.title = turnRunning
        ? 'Live steering is only available on in-core API/ChatGPT sessions'
        : 'Send prompt (Enter)';
      sendEl.setAttribute('aria-label', 'Send prompt');
    }
  }
  if (inputEl) {
    inputEl.placeholder = steerMode ? 'Steer the active turn (injected at next step)...' : (placeholderOverride || defaultPlaceholder);
  }
  if (stopEl) {
    stopEl.hidden = !turnRunning;
    stopEl.disabled = Boolean(stopRequest);
  }
  if (topbarStopEl) {
    topbarStopEl.hidden = !turnRunning;
    topbarStopEl.disabled = Boolean(stopRequest);
  }
}

export function setComposerPlaceholder(text) {
  placeholderOverride = text || null;
  syncButtons();
}

export function setComposerBusy(busy) {
  turnRunning = !!busy;
  if (!busy) stopRequest = null;
  syncButtons();
}

export function setComposerLoading(loading) {
  sessionLoading = loading;
  syncButtons();
}

export function setComposerRoute(route) {
  currentRoute = route || null;
  syncButtons();
}

// Provider and model come from the single model dropdown (model_picker.js).
export function getComposerOptions(choice) {
  const inCore = !CLI_ROUTES.has(choice.provider);
  const verifyOn = inCore && verifierChip?.getAttribute('aria-pressed') === 'true';
  const stratOn = inCore && strategistChip?.getAttribute('aria-pressed') === 'true';
  const interactiveOn = inCore && document.querySelector('[data-rigor-preset="interactive"]')?.getAttribute('aria-pressed') === 'true';
  const pauseExploreOn = inCore && document.getElementById('pause-explore-chip')?.getAttribute('aria-pressed') === 'true';
  const effort = getReasoningEffort();
  const subagent = getActiveSubagent();
  return {
    provider: choice.provider,
    model: choice.model,
    verification_mode: verifyOn,
    calculation_mode: verifyOn,
    use_strategist: stratOn,
    interactive_plan_mode: interactiveOn,
    pause_after_explore: pauseExploreOn,
    reasoning_effort: effort || 'default',
    subagent_id: subagent ? subagent.id : undefined,
  };
}

function handleFileSelection() {
  const { files, submitting } = activeDraft;
  const pickedIn = filePickerRevision;
  filePickerRevision = null;
  if (!uploadEl?.files) return;
  const chosen = Array.from(uploadEl.files);
  uploadEl.value = '';
  if (pickedIn !== null && pickedIn !== draftRevision) {
    if (chosen.length) notify('The chat changed while the file picker was open. Choose files again in the intended chat.');
    return;
  }
  if (submitting || !composerVisible) return;
  if (files.length + chosen.length > MAX_FILES) {
    notify(`Maximum ${MAX_FILES} files per upload batch.`);
    return;
  }
  for (const file of chosen) {
    if (file.size > MAX_FILE_BYTES) {
      notify(`"${file.name}" exceeds the 10 MiB limit.`);
      continue;
    }
    files.push(file);
  }
  renderFileChips();
}

function renderFileChips() {
  if (!filesBoxEl) return;
  const draft = activeDraft;
  filesBoxEl.innerHTML = '';
  if (draft.files.length === 0) {
    filesBoxEl.hidden = true;
    return;
  }
  filesBoxEl.hidden = false;
  draft.files.forEach((file, idx) => {
    const chip = document.createElement('span');
    chip.className = 'file-chip';
    const kb = Math.max(1, Math.round(file.size / 1024));
    chip.innerHTML = `<span>${escapeHtml(file.name)} (${kb} KiB)</span>`;
    const rm = document.createElement('button');
    rm.type = 'button';
    rm.className = 'icon-btn';
    rm.disabled = draft.submitting;
    rm.style.height = '20px';
    rm.style.minWidth = '20px';
    rm.style.padding = '0';
    rm.textContent = '×';
    rm.setAttribute('aria-label', `Remove ${file.name}`);
    rm.addEventListener('click', () => {
      if (draft !== activeDraft || draft.submitting) return;
      draft.files.splice(idx, 1);
      renderFileChips();
    });
    chip.appendChild(rm);
    filesBoxEl.appendChild(chip);
  });
}

export function arrayBufferToBase64(buffer) {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

async function flushStagedUploads(sessionId, draft, isCurrent) {
  const files = draft.files;
  const encoded = [];
  for (const file of files) {
    const buf = await file.arrayBuffer();
    if (!isCurrent()) return null;
    encoded.push({
      name: file.name,
      content_base64: arrayBufferToBase64(buf),
    });
  }
  let result;
  try {
    result = await api.uploadFilesBase64(sessionId, encoded);
  } catch (error) {
    if (Array.isArray(error.payload?.ingested) && Array.isArray(error.payload?.failed)) {
      result = error.payload;
    } else if (error.status >= 400 && error.status < 500) {
      throw error;
    }
  }
  const failures = new Map(
    Array.isArray(result?.failed) ? result.failed.map((item) => [item?.index, item?.error]) : []
  );
  const ingested = files.filter((_, index) => !failures.has(index));
  if (!Array.isArray(result?.ingested) || result.ingested.length !== ingested.length ||
      ingested.length + failures.size !== files.length) {
    // Without a receipt, some files may already exist on disk. Require explicit
    // reattachment so the next Send cannot duplicate an unconfirmed batch.
    draft.files = [];
    if (draft === activeDraft) renderFileChips();
    return { ingested: [], unconfirmed: files, failed: [] };
  }
  draft.files = files.filter((_, index) => failures.has(index));
  if (draft === activeDraft) renderFileChips();
  const failed = Array.from(failures, ([index, error]) => ({ name: files[index].name, error }));
  return { ingested, unconfirmed: [], failed };
}

async function handleSubmit() {
  const draft = activeDraft;
  const revision = draftRevision;
  const isCurrent = () => draft === activeDraft && revision === draftRevision;
  if (!composerVisible || draft.submitting || sessionLoading || stopRequest) return;
  const prompt = readDraftText(draft).trim();
  if (turnRunning) {
    if (!canSteer()) {
      notify('Live steering is only available on in-core API/ChatGPT sessions.');
      return;
    }
    if (!draftSessionId || !prompt || typeof callbacks.onSteerTurn !== 'function') return;
    try {
      draft.submitting = true;
      syncButtons();
      setComposerPrompt('', false);
      await callbacks.onSteerTurn(draftSessionId, prompt);
    } catch (err) {
      restoreDraftText(draft, prompt);
      notifyDraft(draft, `Could not queue steer: ${err.message}`);
    } finally {
      draft.submitting = false;
      if (draft === activeDraft) syncButtons();
    }
    return;
  }
  if (!prompt && draft.files.length === 0) return;
  const currentChoice = () => (typeof callbacks.getModelChoice === 'function' ? callbacks.getModelChoice() : null);
  // Keep the prompt when nothing can answer it: no model connected, or none on the
  // route the session is bound to (the server would refuse the send).
  const refuseNoModel = () => {
    const reason = typeof callbacks.noModelReason === 'function' ? callbacks.noModelReason() : null;
    notify(reason || 'No model connected - open Connect.');
    if (!reason && typeof callbacks.onNeedModel === 'function') callbacks.onNeedModel();
  };
  if (prompt && !currentChoice()) {
    refuseNoModel();
    return;
  }

  try {
    draft.submitting = true;
    syncButtons();
    const sessionId = await callbacks.ensureSession();
    if (!sessionId || sessionId !== draftSessionId || !isCurrent()) return;
    // Selecting or creating the session may have changed which models it can use: check
    // again before anything is uploaded, cleared or shown as sent.
    const choice = currentChoice();
    if (prompt && !choice) {
      refuseNoModel();
      return;
    }
    const options = prompt ? getComposerOptions(choice) : null;

    if (draft.files.length > 0) {
      const result = await flushStagedUploads(sessionId, draft, isCurrent);
      if (!result) return;
      const { ingested, unconfirmed, failed } = result;
      if ((ingested.length || unconfirmed.length) && typeof callbacks.onSourcesUpdated === 'function') {
        callbacks.onSourcesUpdated(sessionId);
      }
      const names = (files) => files.map((file) => `“${file.name}”`).join(', ');
      if (unconfirmed.length) {
        notifyDraft(draft, `Could not confirm upload of ${names(unconfirmed)}. Check Sources and reattach any missing files.`);
        return;
      }
      const added = ingested.length ? `Added ${names(ingested)}.` : '';
      if (failed.length) {
        const details = failed.map(({ name, error }) => `“${name}” (${error})`).join(', ');
        notifyDraft(draft, `${added} Could not add ${details}.`.trim());
        return;
      }
      if (added) notifyDraft(draft, added);
    }

    if (prompt && isCurrent()) {
      if (readDraftText(draft).trim() === prompt) setComposerPrompt('', false);
      if (typeof callbacks.onSendPrompt === 'function') {
        try {
          await callbacks.onSendPrompt(sessionId, prompt, options);
        } catch (err) {
          restoreDraftText(draft, prompt);
          throw err;
        }
      }
    }
  } catch (err) {
    notifyDraft(draft, `Send failed: ${err.message}`);
  } finally {
    draft.submitting = false;
    if (draft === activeDraft) syncButtons();
  }
}

function wireWorkspaceModal() {
  const closeBtn = document.getElementById('workspace-close-btn');
  const cancelBtn = document.getElementById('workspace-cancel-btn');
  const wsForm = document.getElementById('workspace-form');

  const closeModal = () => wsDialogEl?.close();
  const openManualModal = () => {
    if (!wsDialogEl || wsDialogEl.open) return;
    wsDialogEl.showModal();
    wsInputEl?.focus();
  };
  wsDialogEl?.addEventListener('close', () => {
    if (wsDialogEl.open) return;
    const controls = [workspaceBtn, document.querySelector('#research-options > summary'), inputEl];
    for (const node of controls) {
      if (!node) continue;
      node.focus();
      if (document.activeElement === node) break;
    }
  });
  wsDialogEl?.addEventListener('click', (event) => {
    if (event.target === wsDialogEl) closeModal();
  });
  if (workspaceBtn) {
    workspaceBtn.addEventListener('click', async () => {
      workspaceBtn.disabled = true;
      try {
        const picked = await api.pickWorkspaceFolder();
        if (picked && picked.supported) {
          if (!picked.cancelled && picked.path && typeof callbacks.onAttachWorkspace === 'function') {
            await callbacks.onAttachWorkspace(picked.path);
          }
          return;
        }
        openManualModal();
      } catch {
        openManualModal();
      } finally {
        workspaceBtn.disabled = false;
      }
    });
  }
  if (closeBtn) closeBtn.addEventListener('click', closeModal);
  if (cancelBtn) cancelBtn.addEventListener('click', closeModal);
  if (wsForm) {
    wsForm.addEventListener('submit', async (ev) => {
      ev.preventDefault();
      const path = wsInputEl ? wsInputEl.value.trim() : '';
      if (!path) {
        closeModal();
        return;
      }
      closeModal();
      if (typeof callbacks.onAttachWorkspace === 'function') {
        await callbacks.onAttachWorkspace(path);
      }
    });
  }
}

function notify(msg) {
  if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
