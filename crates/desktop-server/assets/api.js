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

export function getSessionFolders() {
  return rawJson('GET', '/api/session-folders');
}

export function changeSessionFolder(payload) {
  return rawJson('POST', '/api/session-folders', payload);
}

export function listProjects() {
  return rawJson('GET', '/api/projects');
}

export function createProject(payload) {
  return rawJson('POST', '/api/projects', payload);
}

export function getProject(id) {
  return rawJson('GET', projectPath(id));
}

export function prepareProjectBackup(id) {
  return rawJson('POST', projectPath(id, '/backup'), {});
}

export function updateProject(id, payload) {
  return rawJson('PUT', projectPath(id), payload);
}

export function linkProjectSession(id, sessionId) {
  return rawJson('PUT', projectPath(id, '/sessions', sessionId), {});
}

export function addProjectItem(id, payload) {
  return rawJson('POST', projectPath(id, '/items'), payload);
}

export function deleteProjectItem(id, itemId) {
  return rawJson('DELETE', projectPath(id, '/items', itemId));
}

export function saveProjectSource(id, payload) {
  return rawJson('POST', projectPath(id, '/sources'), payload);
}

export function deleteProjectSource(id, sourceId, revision) {
  const query = revision == null ? '' : `?revision=${encodeURIComponent(revision)}`;
  return rawJson('DELETE', projectPath(id, '/sources', sourceId) + query);
}

export function listValidationPlans(projectId) {
  return rawJson('GET', projectPath(projectId, '/validation-plans'));
}

export function freezeValidationPlan(projectId, plan) {
  return rawJson('POST', projectPath(projectId, '/validation-plans'), plan);
}

export function listValidationCandidates(projectId, planId) {
  return rawJson('GET', projectPath(projectId, '/validation-plans', planId) + '/candidates');
}

export function evaluateValidationPlan(projectId, planId, candidate) {
  return rawJson('POST', projectPath(projectId, '/validation-plans', planId) + '/evaluate', candidate);
}

function projectPath(id, suffix = '', child = null) {
  return `/api/projects/${encodeURIComponent(id)}${suffix}${child === null ? '' : `/${encodeURIComponent(child)}`}`;
}

export function listExperiments(projectId) {
  return rawJson('GET', projectPath(projectId, '/experiments'));
}

export function createExperiment(projectId, payload) {
  return rawJson('POST', projectPath(projectId, '/experiments'), payload);
}

export function registerExperimentBranch(projectId, experimentId, payload) {
  return rawJson('POST', projectPath(projectId, '/experiments', experimentId) + '/branches', payload);
}

export function recordExperimentObservation(projectId, experimentId, sessionId, payload) {
  const path = projectPath(projectId, '/experiments', experimentId);
  return rawJson('POST', path + '/branches/' + encodeURIComponent(sessionId) + '/observations', payload);
}

export function diffExperimentBranch(projectId, experimentId, sessionId) {
  const path = projectPath(projectId, '/experiments', experimentId);
  return rawJson('GET', path + '/branches/' + encodeURIComponent(sessionId) + '/diff');
}

export function createSession(opts = {}) {
  return rawJson('POST', '/api/sessions', opts);
}

export function correctInvestigation(id, change) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/corrections`, change);
}

export function checkInvestigation(id, request) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/rechecks`, request);
}

export function developInvestigation(id, request) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/branches`, request);
}

export function findInvestigationConventions(id, request) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/conventions`, request);
}

export function openInvestigationResearch(id, request) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/research`, request);
}

export function pinInvestigationSkeleton(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/skeleton`, payload);
}

export function getInvestigationMapview(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/investigation/mapview`);
}

export function getInvestigationLegacy(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/investigation/legacy`);
}

export function getInvestigationNodeText(id, tag, seq) {
  const qs = new URLSearchParams({ tag: String(tag) });
  if (seq != null) qs.set('seq', String(seq));
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/investigation/node-text?${qs}`);
}

export function openInvestigationPaper(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/open`, payload);
}

export function getInvestigationOpenStatus(id, jobId) {
  const qs = new URLSearchParams({ job_id: String(jobId) });
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/investigation/open-status?${qs}`);
}

export function investigationCited(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/cited`, payload);
}

export function startInvestigationTask(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/tasks`, payload);
}

export function saveInvestigationNote(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/investigation/notes`, payload);
}

export function getInvestigateSettings() {
  return rawJson('GET', '/api/settings/investigate');
}

export function putInvestigateSettings(payload) {
  return rawJson('PUT', '/api/settings/investigate', payload);
}

export function listLibraryPapers(params = {}) {
  const qs = new URLSearchParams(params);
  const suffix = qs.toString() ? `?${qs}` : '';
  return rawJson('GET', `/api/library${suffix}`);
}

export function createHandoff(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/handoff`, payload);
}

export function getHandoffs(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/handoffs`);
}

export function getSession(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}`);
}

export function getPendingApprovals(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/approvals`);
}

export function sendMessage(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/messages`, payload);
}

export function stopTurn(id) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/stop`, {});
}

export function archiveSession(id, archived = true) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/archive`, { archived });
}

export function deleteSession(id) {
  return rawJson('DELETE', `/api/sessions/${encodeURIComponent(id)}`);
}

export function revokeSessionGrants(id) {
  return rawJson('DELETE', `/api/sessions/${encodeURIComponent(id)}/grants`);
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

export function postSteer(sessionId, text) {
  const payload = typeof text === 'string' ? { text } : text;
  return rawJson('POST', `/api/sessions/${encodeURIComponent(sessionId)}/steer`, payload);
}

export function answerCheckpoint(sessionId, checkpointId, payload) {
  return rawJson(
    'POST',
    `/api/sessions/${encodeURIComponent(sessionId)}/checkpoints/${encodeURIComponent(checkpointId)}`,
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

export function readSourceFile(id, relPath, anchor) {
  const query = new URLSearchParams({root: 'sources', path: relPath});
  if (Number.isSafeInteger(anchor?.start_line)) {
    query.set('start_line', anchor.start_line);
    query.set('end_line', anchor.end_line ?? anchor.start_line);
  }
  return rawJson(
    'GET',
    `/api/sessions/${encodeURIComponent(id)}/workspace/file?${query}`
  );
}

export function patchLatex(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/latex/patch`, payload);
}

export function getManuscriptDrafts(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/manuscript-drafts`);
}

export function putManuscriptDraft(id, payload) {
  return rawJson('PUT', `/api/sessions/${encodeURIComponent(id)}/manuscript-drafts`, payload);
}

export function runTexAudit(id, payload = {}) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/tex-audit`, payload);
}

export function searchLiterature(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/literature`, payload);
}

export function searchFormalLibrary(id, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/formal-library`, payload);
}

export function searchPaperPassages(id, query) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/paper-passages`, { query });
}

export function getDag(id) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/dag`);
}

export function getProofEvidence(id, entryId) {
  return rawJson('GET', `/api/sessions/${encodeURIComponent(id)}/evidence?entry_id=${encodeURIComponent(entryId)}`);
}

export function verifySession(id) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/verify`, {});
}

export function reviewClaim(id, target) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(id)}/claim-review`, target);
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

export function promoteExplain(sessionId, payload) {
  return rawJson('POST', `/api/sessions/${encodeURIComponent(sessionId)}/explain/promote`, payload);
}

export function setSessionConventions(sessionId, pinnedConventions) {
  return rawJson('PUT', `/api/sessions/${encodeURIComponent(sessionId)}/conventions`, {
    pinned_conventions: pinnedConventions,
  });
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

export function listSubagents() {
  return rawJson('GET', '/api/subagents');
}

export function createSubagent(payload) {
  return rawJson('POST', '/api/subagents', payload);
}

export function deleteSubagent(id) {
  return rawJson('DELETE', `/api/subagents/${encodeURIComponent(id)}`);
}

export function draftSubagent(payload) {
  return rawJson('POST', '/api/subagents/draft', payload);
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
    'checkpoint_requested',
    'checkpoint_resolved',
    'steer_queued',
    'dag_updated',
    'claim_review_started',
    'claim_review_finished',
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
  if (typeof handlers.open === 'function') {
    es.addEventListener('open', handlers.open);
  }
  es.onerror = (err) => {
    if (typeof handlers.error === 'function') {
      handlers.error(err);
    }
  };
  return es;
}

export function waitForSessionEvents(source, timeoutMs = 5000) {
  if (!source || source.readyState === EventSource.CLOSED) return Promise.resolve(false);
  if (source.readyState === EventSource.OPEN) return Promise.resolve(true);
  return new Promise((resolve) => {
    const finish = (ready) => {
      clearTimeout(timer);
      source.removeEventListener('open', onOpen);
      source.removeEventListener('error', onError);
      resolve(ready);
    };
    const onOpen = () => finish(true);
    const onError = () => {
      if (source.readyState === EventSource.CLOSED) finish(false);
    };
    const timer = setTimeout(() => finish(false), timeoutMs);
    source.addEventListener('open', onOpen);
    source.addEventListener('error', onError);
  });
}
