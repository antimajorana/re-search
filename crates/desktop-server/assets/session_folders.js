import { changeSessionFolder, getSessionFolders } from '/assets/api.js';

let catalog = { revision: -1, folders: [], members: {} };
let filter = '*';
let movingSession = null;
let busy = false;
let rerender = () => {};
const el = id => document.getElementById(id);

function setOptions(select, first, id) {
  select.replaceChildren(...first.map(([value, label]) => new Option(label, value)),
    ...catalog.folders.map(f => new Option(f.name, f.id)));
  if ([...select.options].some(o => o.value === id)) select.value = id;
}

function accept(data) {
  if (data.revision < catalog.revision) return;
  catalog = data;
  if (filter !== '*' && filter !== '' && !catalog.folders.some(f => f.id === filter)) filter = '*';
  setOptions(el('session-folder-filter'), [['*', 'All folders'], ['', 'Unfiled']], filter);
  rerender();
}

async function refresh() { accept(await getSessionFolders()); }

export function sessionsInFolder(sessions) {
  return filter === '*' ? sessions : sessions.filter(s => (catalog.members[s.id] || '') === filter);
}

export function sessionFolderName(session) {
  return catalog.folders.find(f => f.id === catalog.members[session])?.name;
}

async function apply(fields) {
  const response = await changeSessionFolder({ ...fields, revision: catalog.revision });
  accept(response.catalog);
  return response;
}

export async function fileNewSession(id) {
  if (filter === '*' || !filter) return;
  try { await apply({ op: 'move', session: id, folder: filter }); }
  catch (error) {
    filter = '*';
    await refresh();
    throw error;
  }
}

function showName(id) {
  const old = catalog.folders.find(f => f.id === id);
  el('session-folder-name').value = old?.name || '';
  el('remove-session-folder').hidden = !old;
  el('save-session-folder').textContent = old ? 'Rename folder' : 'Create folder';
}

function populate(editing) {
  setOptions(el('session-folder-choice'), [['', 'New folder']], editing);
  showName(el('session-folder-choice').value);
  setOptions(el('file-session-destination'), [['', 'Unfiled']], catalog.members[movingSession?.id] || '');
}

export async function openFolderDialog(session = null) {
  movingSession = session;
  const dlg = el('session-folder-dialog');
  el('folder-dialog-heading').textContent = session ? `Move “${session.title || session.id}”` : 'Session folders';
  el('edit-session-folder').hidden = Boolean(session);
  el('file-session-form').hidden = !session;
  el('folder-dialog-status').textContent = '';
  populate(filter);
  if (!dlg.open) dlg.showModal();
  const target = session ? 'file-session-destination' : 'session-folder-name';
  el(target).focus();
  try {
    const revision = catalog.revision;
    await refresh();
    if (dlg.open && revision !== catalog.revision) populate(filter);
  } catch (error) { el('folder-dialog-status').textContent = error.message; }
}

export function initFolders(onChange, onError) {
  rerender = onChange;
  const dlg = el('session-folder-dialog');
  el('session-folder-filter').addEventListener('change', e => { filter = e.target.value; rerender(); });
  el('manage-session-folders').addEventListener('click', () => openFolderDialog());
  el('session-folder-choice').addEventListener('change', e => showName(e.target.value));
  async function submit(fields) {
    if (busy) return;
    busy = true;
    try {
      const result = await apply(fields);
      if (result.created) {
        filter = result.created;
        accept(result.catalog);
      }
      dlg.close();
      el('session-folder-filter').focus();
    } catch (error) {
      if (error.status === 409) {
        try { await refresh(); populate(el('session-folder-choice').value); } catch { /* keep the original error */ }
      }
      el('folder-dialog-status').textContent = error.message;
    } finally { busy = false; }
  }
  el('edit-session-folder').addEventListener('submit', e => {
    e.preventDefault();
    const id = el('session-folder-choice').value;
    submit({ op: id ? 'rename' : 'create', id, name: el('session-folder-name').value });
  });
  el('remove-session-folder').addEventListener('click', () => {
    const id = el('session-folder-choice').value;
    if (id && window.confirm('Remove this folder? Its sessions will appear in Unfiled.')) submit({ op: 'remove', id });
  });
  el('file-session-form').addEventListener('submit', e => {
    e.preventDefault();
    if (movingSession) submit({ op: 'move', session: movingSession.id, folder: el('file-session-destination').value || null });
  });
  refresh().catch(e => onError(`Could not load session folders. ${e.message}`));
}
