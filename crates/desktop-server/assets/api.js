// Loopback REST + CSRF auth client & SSE EventSource plumbing for re:Search desktop server.
let csrfToken = '';
const TOKEN_STORAGE_KEY = 're-search-token';

export function getCsrfToken() {
  return csrfToken;
}

export async function initAuth() {
  const meta = document.querySelector('meta[name="re-search-csrf"]');
  const metaToken = meta && typeof meta.content === 'string' ? meta.content.trim() : '';
  const params = new URLSearchParams(window.location.search);

  // Only the server-validated value from the <meta> tag is trusted; query parameters are
  // never used as credentials (launch/token are only stripped from the address bar below).
  csrfToken = metaToken;

  if (params.has('launch') || params.has('token')) {
    params.delete('launch');
    params.delete('token');
    const clean = params.toString();
    const nextUrl = window.location.pathname + (clean ? `?${clean}` : '') + window.location.hash;
    window.history.replaceState(null, '', nextUrl);
  }

  // The server writes the token into the page only for header-authenticated loads or the
  // single-use launch nonce. Keep it in sessionStorage (scoped to this origin, port
  // included, and to this tab) so a reload works.
  try {
    if (csrfToken) {
      window.sessionStorage.setItem(TOKEN_STORAGE_KEY, csrfToken);
    } else {
      csrfToken = (window.sessionStorage.getItem(TOKEN_STORAGE_KEY) || '').trim();
    }
  } catch {
    // sessionStorage unavailable: the token lives only for this page load.
  }
  return csrfToken;
}

async function rawJson(method, path, body) {
  const headers = {};
  if (csrfToken) {
    headers['X-ReSearch-Token'] = csrfToken;
  }
  const init = {
    method,
    credentials: 'same-origin',
    headers,
  };
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

export function getHealth() {
  return rawJson('GET', '/api/health');
}

export function listSessions() {
  return rawJson('GET', '/api/sessions');
}

export function createSession(opts = {}) {
  return rawJson('POST', '/api/sessions', opts);
}

export function getSession(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}`);
}

export function sendMessage(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/messages`, payload);
}

export function stopTurn(id) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/stop`, {});
}

export function pickWorkspaceFolder() {
  return rawJson('POST', '/api/workspace/pick', {});
}

export function setSessionWorkspace(id, workspaceRoot) {
  return rawJson('PUT', `/api/sessions/${encodeURIComponent(id)}/workspace`, {
    workspace_root: workspaceRoot,
  });
}

export function respondApproval(id, callId, payload) {
  return rawJson(
    'POST',
    `/api/sessions/${encodeURIComponent(id)}/approvals/${encodeURIComponent(callId)}`,
    payload
  );
}

export function uploadFilesBase64(id, files) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/upload`, { files });
}

export function getWorkspace(id, relPath) {
  const qs = relPath ? `?path=${encodeURIComponent(relPath)}` : '';
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/workspace${qs}`);
}

export function patchLatex(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/latex/patch`, payload);
}

export function runTexAudit(id, payload = {}) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/tex-audit`, payload);
}

export function searchLiterature(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/literature`, payload);
}

export function getDag(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/dag`);
}

export function verifySession(id) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/verify`, {});
}

export function forkSession(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/fork`, payload);
}

export function diffSessions(id, otherId) {
  return rawJson(
    'GET',
    `/api/sessions/${encodeURIComponent(id)}/diff/${encodeURIComponent(otherId)}`
  );
}

export function narrateSession(id, payload = {}) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/narrate`, payload);
}

export function getSidetalk(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/sidetalk`);
}

export function postSidetalk(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/sidetalk`, payload);
}

export function getProviderSettings() {
  return rawJson('GET', '/api/settings/providers');
}

export function updateProviderSettings(payload) {
  return rawJson('PUT', '/api/settings/providers', payload);
}

// Which providers are connected (never returns a key). `refresh` re-asks Ollama.
export function getConnections(refresh = false) {
  return rawJson('GET', `/api/connections${refresh ? '?refresh=1' : ''}`);
}

// A provider's sign-in action (`login`, `open`, `logout`) or its state (GET `login`), for
// rows of GET /api/connections that carry an `action` (Sign in with ChatGPT). The server
// opens only its own pending sign-in URL; nothing here sends a URL or a token.
export function connectionAction(id, action, method = 'POST') {
  const path = `/api/connections/${encodeURIComponent(id)}/${encodeURIComponent(action)}`;
  return rawJson(method, path, method === 'POST' ? {} : undefined);
}

// Models the user can pick now, grouped by connected provider (server-cached 10 minutes).
export function getModels(refresh = false) {
  return rawJson('GET', `/api/models${refresh ? '?refresh=1' : ''}`);
}

export function exportUrl(id, format) {
  return `/api/sessions/${encodeURIComponent(id)}/export?format=${encodeURIComponent(format)}`;
}

export function openSessionEvents(id, handlers) {
  const url = `/api/sessions/${encodeURIComponent(id)}/events`;
  const es = new EventSource(url);
  const eventTypes = [
    'session_event',
    'text_delta',
    'thinking_delta',
    'round_reset',
    'tool_trace',
    'approval_requested',
    'approval_resolved',
    'dag_updated',
    'turn_complete',
    'turn_error',
  ];
  for (const name of eventTypes) {
    es.addEventListener(name, (ev) => {
      let data = null;
      if (ev.data) {
        try {
          data = JSON.parse(ev.data);
        } catch {
          data = { raw: ev.data };
        }
      }
      if (typeof handlers[name] === 'function') {
        handlers[name](data, ev);
      }
    });
  }
  es.onerror = (err) => {
    if (typeof handlers.error === 'function') {
      handlers.error(err);
    }
  };
  return es;
}
