// Explore mode REST API helpers (package X4).
// Uses the CSRF token initialized by api.js (never puts tokens in URLs).
import { getCsrfToken } from './api.js';

const TOKEN_STORAGE_KEY = 're-search-token';

export function resolveIdeaToken() {
  const tok = getCsrfToken();
  if (tok) return tok;
  if (typeof document !== 'undefined' && typeof document.querySelector === 'function') {
    const meta = document.querySelector('meta[name="re-search-csrf"]');
    if (meta && typeof meta.content === 'string' && meta.content.trim()) {
      return meta.content.trim();
    }
  }
  try {
    if (typeof window !== 'undefined' && window?.sessionStorage) {
      const stored = window.sessionStorage.getItem(TOKEN_STORAGE_KEY);
      if (stored && stored.trim()) return stored.trim();
    }
  } catch {
    // sessionStorage unavailable.
  }
  return '';
}

async function ideaJson(method, path, body) {
  const headers = {};
  const token = resolveIdeaToken();
  if (token) headers['X-ReSearch-Token'] = token;
  const init = { method, credentials: 'same-origin', headers };
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const resp = await fetch(path, init);
  const text = await resp.text();
  let data = null;
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { raw: text };
    }
  }
  if (!resp.ok) {
    const msg = (data && (data.error || data.message)) || `HTTP ${resp.status}`;
    const err = new Error(msg);
    err.status = resp.status;
    err.payload = data;
    throw err;
  }
  return data;
}

/** Local-only preview: zero network requests, zero sessions created. */
export function previewIdea(payload) {
  return ideaJson('POST', '/api/idea/preview', payload);
}

/** Create an Explore session (`POST /api/sessions` with `mode: "explore"`). */
export function createExploreSession(payload) {
  return ideaJson('POST', '/api/sessions', { ...payload, mode: 'explore' });
}

/** Fetch an Explore session including its `"idea"` view projection. */
export function getExploreSession(sessionId) {
  return ideaJson('GET', `/api/sessions/${encodeURIComponent(sessionId)}`);
}

/** Invoke an Explore session operation (`POST /api/sessions/{id}/idea/{op}`). */
export function postIdeaOp(sessionId, op, payload = {}) {
  return ideaJson(
    'POST',
    `/api/sessions/${encodeURIComponent(sessionId)}/idea/${encodeURIComponent(op)}`,
    payload,
  );
}

/** Dispatch an approval decision through the shared desktop approval route. */
export function decideIdeaApproval(sessionId, callId, payload) {
  return ideaJson(
    'POST',
    `/api/sessions/${encodeURIComponent(sessionId)}/approvals/${encodeURIComponent(callId)}`,
    payload,
  );
}

/** Attach a user-supplied PDF/TeX file (or uploaded source_id) to an Explore paper key (X9). */
export function attachIdeaPdf(sessionId, attachPayload) {
  return postIdeaOp(sessionId, 'read', { attach_pdf: attachPayload });
}

/** Fetch read-only F7 incoming/outgoing hand-offs and child session statuses (`00-overview.md` 4.1 Rule 5). */
export function getExploreHandoffs(sessionId) {
  return ideaJson('GET', `/api/sessions/${encodeURIComponent(sessionId)}/handoffs`);
}


