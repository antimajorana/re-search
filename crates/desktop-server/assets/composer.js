// Floating Gemini composer pill: auto-resizing textarea, <=5 file / 10 MiB Base64 upload, workspace picker, toggles.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';

const MAX_FILES = 5;
const MAX_FILE_BYTES = 10 * 1024 * 1024;

let formEl = null;
let inputEl = null;
let sendEl = null;
let uploadEl = null;
let filesBoxEl = null;
let workspaceBtn = null;
let verifierChip = null;
let strategistChip = null;
let providerEl = null;
let modelEl = null;
let wsDialogEl = null;
let wsInputEl = null;
let callbacks = {};
let stagedFiles = [];

export function initComposer(opts = {}) {
  callbacks = opts;
  formEl = document.getElementById('composer-pill');
  inputEl = document.getElementById('composer-input');
  sendEl = document.getElementById('composer-send');
  uploadEl = document.getElementById('composer-upload-input');
  filesBoxEl = document.getElementById('composer-files');
  workspaceBtn = document.getElementById('workspace-attach-btn');
  verifierChip = document.getElementById('verifier-chip');
  strategistChip = document.getElementById('strategist-chip');
  providerEl = document.getElementById('composer-provider');
  modelEl = document.getElementById('composer-model');
  wsDialogEl = document.getElementById('workspace-dialog');
  wsInputEl = document.getElementById('workspace-path-input');

  if (inputEl) {
    inputEl.addEventListener('input', resizeTextarea);
    inputEl.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter' && !ev.shiftKey) {
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
    uploadEl.addEventListener('change', handleFileSelection);
  }

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
  if (!inputEl) return;
  inputEl.value = text || '';
  resizeTextarea();
  if (focusInput) inputEl.focus();
}

export function setComposerBusy(busy) {
  if (sendEl) sendEl.disabled = !!busy;
}

export function setComposerProviderModel(provider, model) {
  if (providerEl && provider) providerEl.value = provider;
  if (modelEl && model !== undefined) modelEl.value = model || '';
}

export function getComposerOptions() {
  const verifyOn = Boolean(verifierChip && verifierChip.getAttribute('aria-pressed') === 'true');
  const stratOn = Boolean(strategistChip && strategistChip.getAttribute('aria-pressed') === 'true');
  return {
    provider: providerEl ? providerEl.value : 'anthropic',
    model: modelEl ? modelEl.value.trim() : '',
    verification_mode: verifyOn,
    calculation_mode: verifyOn,
    use_strategist: stratOn,
  };
}

function handleFileSelection() {
  if (!uploadEl || !uploadEl.files) return;
  const chosen = Array.from(uploadEl.files);
  if (stagedFiles.length + chosen.length > MAX_FILES) {
    notify(`Maximum ${MAX_FILES} files per upload batch.`);
    uploadEl.value = '';
    return;
  }
  for (const file of chosen) {
    if (file.size > MAX_FILE_BYTES) {
      notify(`"${file.name}" exceeds the 10 MiB limit.`);
      continue;
    }
    stagedFiles.push(file);
  }
  uploadEl.value = '';
  renderFileChips();
}

function renderFileChips() {
  if (!filesBoxEl) return;
  filesBoxEl.innerHTML = '';
  if (stagedFiles.length === 0) {
    filesBoxEl.hidden = true;
    return;
  }
  filesBoxEl.hidden = false;
  stagedFiles.forEach((file, idx) => {
    const chip = document.createElement('span');
    chip.className = 'file-chip';
    const kb = Math.max(1, Math.round(file.size / 1024));
    chip.innerHTML = `<span>${escapeHtml(file.name)} (${kb} KiB)</span>`;
    const rm = document.createElement('button');
    rm.type = 'button';
    rm.className = 'icon-btn';
    rm.style.height = '20px';
    rm.style.minWidth = '20px';
    rm.style.padding = '0';
    rm.textContent = '×';
    rm.setAttribute('aria-label', `Remove ${file.name}`);
    rm.addEventListener('click', () => {
      stagedFiles.splice(idx, 1);
      renderFileChips();
    });
    chip.appendChild(rm);
    filesBoxEl.appendChild(chip);
  });
}

function arrayBufferToBase64(buffer) {
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary);
}

async function flushStagedUploads(sessionId) {
  if (stagedFiles.length === 0) return null;
  const encoded = [];
  for (const file of stagedFiles) {
    const buf = await file.arrayBuffer();
    encoded.push({
      name: file.name,
      content_base64: arrayBufferToBase64(buf),
    });
  }
  const resp = await api.uploadFilesBase64(sessionId, encoded);
  stagedFiles = [];
  renderFileChips();
  return resp;
}

async function handleSubmit() {
  const prompt = inputEl ? inputEl.value.trim() : '';
  if (!prompt && stagedFiles.length === 0) return;

  try {
    setComposerBusy(true);
    const sessionId = await callbacks.ensureSession();
    if (!sessionId) return;

    if (stagedFiles.length > 0) {
      await flushStagedUploads(sessionId);
      notify('Uploaded sources ingested into session.');
      if (typeof callbacks.onSourcesUpdated === 'function') {
        callbacks.onSourcesUpdated(sessionId);
      }
    }

    if (prompt) {
      inputEl.value = '';
      resizeTextarea();
      if (typeof callbacks.onSendPrompt === 'function') {
        await callbacks.onSendPrompt(sessionId, prompt, getComposerOptions());
      }
    }
  } catch (err) {
    notify(`Send failed: ${err.message}`);
  } finally {
    setComposerBusy(false);
  }
}

function wireWorkspaceModal() {
  const closeBtn = document.getElementById('workspace-close-btn');
  const cancelBtn = document.getElementById('workspace-cancel-btn');
  const wsForm = document.getElementById('workspace-form');

  const closeModal = () => {
    if (wsDialogEl) wsDialogEl.hidden = true;
  };
  if (workspaceBtn) {
    workspaceBtn.addEventListener('click', () => {
      if (wsDialogEl) {
        wsDialogEl.hidden = false;
        if (wsInputEl) wsInputEl.focus();
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
