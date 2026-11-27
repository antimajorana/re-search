// F14b local Tesseract OCR settings panel and status banner.
// Free local Tesseract (Apache-2.0) text layer; zero network downloads; zero innerHTML writes.
import { BORDER_SOLID, OCR_TAG_WORD, createBadge } from './ui_badge.js';

export const DEFAULT_OCR_ENABLED = true;
export const OCR_INSTALL_PROMPT = 'Install Tesseract to enable OCR';
const DEFAULT_LANG = 'eng';

function el(tag, className, text = null, attrs = null) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v !== undefined && v !== null) node.setAttribute(k, String(v));
    }
  }
  if (text !== null && text !== undefined) node.textContent = String(text);
  return node;
}

function parseLangList(value) {
  const items = Array.isArray(value)
    ? value
    : (typeof value === 'string' ? value.split(/[\s,;+]+/) : []);
  const out = [];
  const seen = new Set();
  for (const raw of items) {
    const clean = String(raw ?? '').trim();
    if (clean && !seen.has(clean)) {
      seen.add(clean);
      out.push(clean);
    }
  }
  return out;
}

export function normalizeOcrStatus(raw) {
  const src = raw && typeof raw === 'object' ? raw : {};
  const cfg = src.settings && typeof src.settings === 'object' ? src.settings : src;
  const st = src.status && typeof src.status === 'object' ? src.status : src;

  const enabled = typeof cfg.enabled === 'boolean'
    ? cfg.enabled
    : (typeof st.enabled === 'boolean' ? st.enabled : DEFAULT_OCR_ENABLED);
  const available = Boolean(st.available);
  const tesseract_path = String(st.tesseract_path ?? cfg.tesseract_path ?? '').trim();
  const tessdata_dir = String(st.tessdata_dir ?? cfg.tessdata_dir ?? '').trim();
  const langs = parseLangList(st.langs ?? cfg.langs);
  const missing_langs = parseLangList(st.missing_langs);
  const version = String(st.version ?? '').trim();
  const rawMessage = String(st.message ?? cfg.message ?? '').trim();
  const message = rawMessage || (!available ? OCR_INSTALL_PROMPT : '');

  return { enabled, available, tesseract_path, tessdata_dir, langs, missing_langs, version, message };
}

export function createOcrStatusBanner(status) {
  const s = normalizeOcrStatus(status);
  const banner = el('div', `ocr-status-banner ocr-status-banner--${s.available ? 'ready' : 'missing'}`, null, {
    role: 'status',
    'data-ocr-available': s.available ? 'true' : 'false',
  });

  banner.appendChild(createBadge(
    s.available
      ? { text: `${OCR_TAG_WORD} ready`, icon: 'ocr', tone: 'ok', border: BORDER_SOLID }
      : { text: `${OCR_TAG_WORD} unavailable`, icon: 'warn', tone: 'warn', border: BORDER_SOLID },
  ));

  let summaryText;
  if (!s.available) {
    summaryText = OCR_INSTALL_PROMPT;
  } else {
    const ver = s.version
      ? (/^tesseract\b/i.test(s.version) ? s.version : `Tesseract ${s.version}`)
      : 'Local Tesseract';
    const count = s.langs.length;
    summaryText = `${ver} · ${count} ${count === 1 ? 'language' : 'languages'}`;
  }
  banner.appendChild(el('span', 'ocr-status-banner-text', summaryText));

  const details = [];
  if (s.message && (s.available || s.message !== OCR_INSTALL_PROMPT)) details.push(s.message);
  if (s.missing_langs.length > 0) details.push(`Missing languages: ${s.missing_langs.join(', ')}`);
  if (details.length > 0) banner.appendChild(el('span', 'ocr-status-banner-detail', details.join(' · ')));

  return banner;
}

function makeTextField(name, labelText, placeholder) {
  const label = el('label', 'ocr-settings-field');
  const input = el('input', 'ocr-settings-input', null, { 'data-ocr-field': name });
  input.type = 'text';
  input.name = name;
  input.placeholder = placeholder;
  label.append(el('span', 'ocr-settings-field-label', labelText), input);
  return { label, input };
}

function makeSummaryRow(termText, key) {
  const row = el('div', 'ocr-settings-summary-row');
  const dd = el('dd', null, null, { 'data-ocr-detected': key });
  row.append(el('dt', null, termText), dd);
  return { row, dd };
}

export function createOcrSettingsPanel(options = {}) {
  const panel = el('section', 'ocr-settings-panel', null, { 'data-ocr-available': 'false' });
  const heading = el('h3', 'ocr-settings-heading', 'Local OCR (Tesseract)');
  const desc = el(
    'p',
    'ocr-settings-desc',
    'Free local Tesseract (Apache-2.0) OCR text layer for scanned PDFs. Runs locally with zero network downloads.',
  );

  const statusRow = el('div', 'ocr-settings-status-row');
  const bannerSlot = el('div', 'ocr-settings-banner-slot');
  const refreshBtn = el('button', 'ocr-settings-refresh', 'Refresh status', { 'data-ocr-action': 'refresh' });
  refreshBtn.type = 'button';
  statusRow.append(bannerSlot, refreshBtn);

  const toggleLabel = el('label', 'ocr-settings-toggle');
  const enabledInput = el('input', null, null, { 'data-ocr-field': 'enabled' });
  enabledInput.type = 'checkbox';
  enabledInput.name = 'enabled';
  enabledInput.checked = DEFAULT_OCR_ENABLED;
  toggleLabel.append(
    enabledInput,
    el('span', null, 'Enable automatic local OCR for scanned PDFs (free, default on)'),
  );

  const fieldsWrap = el('div', 'ocr-settings-fields');
  const pathField = makeTextField('tesseract_path', 'Tesseract executable path (optional)', 'Auto-detect local tesseract');
  const tessdataField = makeTextField('tessdata_dir', 'Local tessdata folder (optional)', 'Default local tessdata');
  const langsField = makeTextField('langs', 'Languages (comma-separated)', DEFAULT_LANG);
  langsField.input.value = DEFAULT_LANG;
  fieldsWrap.append(pathField.label, tessdataField.label, langsField.label);

  const detectedList = el('dl', 'ocr-settings-detected');
  const detPath = makeSummaryRow('Detected executable', 'tesseract_path');
  const detTessdata = makeSummaryRow('Detected tessdata', 'tessdata_dir');
  const detLangs = makeSummaryRow('Available languages', 'langs');
  const detVersion = makeSummaryRow('Detected version', 'version');
  detectedList.append(detPath.row, detTessdata.row, detLangs.row, detVersion.row);

  const actions = el('div', 'ocr-settings-actions');
  const saveBtn = el('button', 'ocr-settings-save', 'Save OCR settings', { 'data-ocr-action': 'save' });
  saveBtn.type = 'button';
  const feedbackEl = el('p', 'ocr-settings-feedback', '', { role: 'status', 'aria-live': 'polite' });
  actions.append(saveBtn, feedbackEl);

  panel.append(heading, desc, statusRow, toggleLabel, fieldsWrap, detectedList, actions);

  function getValues() {
    const parsed = parseLangList(langsField.input.value);
    return {
      enabled: Boolean(enabledInput.checked),
      tesseract_path: String(pathField.input.value ?? '').trim(),
      tessdata_dir: String(tessdataField.input.value ?? '').trim(),
      langs: parsed.length > 0 ? parsed : [DEFAULT_LANG],
    };
  }

  function update(settingsAndStatus) {
    const s = normalizeOcrStatus(settingsAndStatus);
    panel.setAttribute('data-ocr-available', s.available ? 'true' : 'false');
    bannerSlot.replaceChildren(createOcrStatusBanner(s));
    enabledInput.checked = s.enabled;
    pathField.input.value = s.tesseract_path;
    tessdataField.input.value = s.tessdata_dir;
    langsField.input.value = s.langs.length > 0 ? s.langs.join(', ') : DEFAULT_LANG;
    detPath.dd.textContent = s.tesseract_path || 'Not detected';
    detTessdata.dd.textContent = s.tessdata_dir || 'Default';
    detLangs.dd.textContent = s.langs.length > 0 ? s.langs.join(', ') : 'None';
    detVersion.dd.textContent = s.version || 'Unavailable';
    return s;
  }

  function setBusy(busy) {
    const isBusy = Boolean(busy);
    panel.setAttribute('data-busy', isBusy ? 'true' : 'false');
    for (const ctrl of [saveBtn, refreshBtn, enabledInput, pathField.input, tessdataField.input, langsField.input]) {
      ctrl.disabled = isBusy;
    }
  }

  function setFeedback(text, isError = false) {
    feedbackEl.textContent = String(text ?? '');
    if (isError) feedbackEl.setAttribute('data-error', 'true');
    else feedbackEl.removeAttribute('data-error');
  }

  saveBtn.addEventListener('click', () => {
    if (typeof options.onSave === 'function') options.onSave(getValues());
  });
  refreshBtn.addEventListener('click', () => {
    if (typeof options.onRefresh === 'function') options.onRefresh();
  });

  update(options.initial ?? options.status ?? {});
  return { element: panel, update, getValues, setBusy, setFeedback };
}

async function callOcrSettingsApi(apiFetch, method, payload) {
  if (typeof apiFetch !== 'function') throw new TypeError('apiFetch must be a function');
  const init = { method };
  if (payload !== undefined) {
    init.headers = { 'Content-Type': 'application/json' };
    init.body = JSON.stringify(payload);
  }
  const resp = await apiFetch('/api/settings/ocr', init);
  if (resp && typeof resp === 'object' && typeof resp.json === 'function') {
    let data = null;
    try { data = await resp.json(); } catch { data = null; }
    if (resp.ok === false) {
      throw new Error(String((data && (data.error || data.message)) || `HTTP ${resp.status ?? 'error'}`));
    }
    return normalizeOcrStatus(data);
  }
  return normalizeOcrStatus(resp);
}

export function loadOcrSettings(apiFetch) {
  return callOcrSettingsApi(apiFetch, 'GET');
}

export function saveOcrSettings(apiFetch, payload = {}) {
  const langs = parseLangList(payload.langs);
  return callOcrSettingsApi(apiFetch, 'PUT', {
    enabled: typeof payload.enabled === 'boolean' ? payload.enabled : DEFAULT_OCR_ENABLED,
    tesseract_path: String(payload.tesseract_path ?? '').trim(),
    tessdata_dir: String(payload.tessdata_dir ?? '').trim(),
    langs: langs.length > 0 ? langs : [DEFAULT_LANG],
  });
}
