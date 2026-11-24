import { searchFormalLibrary } from '/assets/api.js';

let view;
let sessionId = null;
let workspaceRoot = '';
let generation = 0;

export function initFormalLibrary({ onNotify } = {}) {
  const form = document.getElementById('formal-library-form');
  if (!form) return;

  view = {
    query: document.getElementById('formal-library-query'),
    submit: form.querySelector('button[type="submit"]'),
    status: document.getElementById('formal-library-status'),
    results: document.getElementById('formal-library-results'),
    onNotify,
  };
  reset();
  form.addEventListener('submit', submitSearch);
}

export function setFormalLibraryWorkspace(nextSessionId, nextWorkspaceRoot) {
  if (sessionId === nextSessionId && workspaceRoot === nextWorkspaceRoot) return;
  sessionId = nextSessionId;
  workspaceRoot = nextWorkspaceRoot;
  generation += 1;
  reset();
}

function reset() {
  if (!view) return;
  const available = Boolean(sessionId && workspaceRoot);
  view.query.value = '';
  view.query.disabled = !available;
  view.submit.disabled = !available;
  view.results.replaceChildren();
  view.status.textContent = available ? '' : 'Attach a Lean workspace to search Physlib.';
}

async function submitSearch(event) {
  event.preventDefault();
  const query = view.query.value.trim();
  if (!sessionId || !workspaceRoot || !query) return;
  const request = ++generation;
  view.results.replaceChildren();
  view.submit.disabled = true;
  view.status.textContent = 'Searching Physlib…';
  try {
    const response = await searchFormalLibrary(sessionId, { query });
    if (generation !== request) return;

    const results = Array.isArray(response.results) ? response.results : [];
    if (response.status === 'library_not_found') {
      view.status.textContent = 'No Physlib sources found in this workspace.';
      return;
    }
    const messages = [results.length
      ? 'Read from Lean source. Check an intended use with Lean.'
      : 'No matching declarations found.'];
    if (response.scan_truncated) messages.push('Some sources could not be searched.');
    if (response.results_truncated) messages.push('Use a more specific query to narrow the results.');
    view.status.textContent = messages.join(' ');
    view.results.append(...results.map(resultCard));
  } catch (error) {
    if (generation === request) view.status.textContent = error.message;
  } finally {
    if (generation === request) view.submit.disabled = false;
  }
}

function element(type, text, className) {
  const node = document.createElement(type);
  if (text) node.textContent = text;
  if (className) node.className = className;
  return node;
}

function resultCard(result) {
  const card = element('article', '', 'formal-library-result');
  const title = result.qualified_name || result.name;
  const metadata = [`${result.path}:${result.line}`, result.module].filter(Boolean);
  if (result.visibility && result.visibility !== 'public') metadata.push(result.visibility);
  card.append(
    element('strong', title),
    element('div', 'Declaration found', 'formal-library-meta'),
    element('div', metadata.join(' · '), 'formal-library-meta'),
    element('pre', result.signature, 'source-raw-pre'),
  );
  if (result.signature_truncated) {
    card.append(element('div', 'Signature excerpt', 'formal-library-meta'));
  }
  if (result.source_sha256) {
    const digest = element('details', '', 'formal-library-meta');
    digest.append(element('summary', 'Source SHA-256'), element('code', result.source_sha256));
    card.append(digest);
  }
  if (result.qualified_name) {
    const copy = element('button', 'Copy Lean name', 'pill-btn sm');
    copy.type = 'button';
    copy.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(result.qualified_name);
        view.onNotify?.('Copied Lean name');
      } catch {
        view.onNotify?.('Could not copy Lean name');
      }
    });
    card.append(copy);
  }
  return card;
}
