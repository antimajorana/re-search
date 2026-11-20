// Loopback REST + CSRF auth client & SSE EventSource plumbing for re:Search desktop server.
let csrfToken = '';

export function getCsrfToken() {
  return csrfToken;
}

export async function initAuth() {
  const meta = document.querySelector('meta[name="re-search-csrf"]');
  const metaToken = meta && typeof meta.content === 'string' ? meta.content.trim() : '';
  const params = new URLSearchParams(window.location.search);
  const queryToken = (params.get('token') || '').trim();

  if (metaToken) {
    csrfToken = metaToken;
  } else if (queryToken) {
    csrfToken = queryToken;
  }

  if (queryToken) {
    params.delete('token');
    const clean = params.toString();
    const nextUrl = window.location.pathname + (clean ? `?${clean}` : '') + window.location.hash;
    window.history.replaceState(null, '', nextUrl);
  }

  if (!csrfToken) {
    try {
      const boot = await rawJson('GET', '/api/bootstrap');
      if (boot && typeof boot.token === 'string' && boot.token.trim()) {
        csrfToken = boot.token.trim();
      }
    } catch {
      // Cookie-only or unbootstrapped health probe fallback
    }
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

export function exportUrl(id, format) {
  return `/api/sessions/${encodeURIComponent(id)}/export?format=${encodeURIComponent(format)}`;
}

export function openSessionEvents(id, handlers) {
  const url = `/api/sessions/${encodeURIComponent(id)}/events`;
  const es = new EventSource(url);
  const eventTypes = [
    'session_event',
    'thinking_delta',
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
