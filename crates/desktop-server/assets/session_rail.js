// Left rail session list: Active vs Archived filter, search query filter,
// verification/step/running badges, and inline Archive/Unarchive + Delete actions.
import { escapeHtml, mathTextToHtml } from '/assets/math.js';

function basenameOf(p) {
  if (!p) return '';
  const parts = String(p).replace(/[\\/]+$/, '').split(/[\\/]/);
  return parts[parts.length - 1] || String(p);
}

const ARCHIVE_ICON = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><rect x="2" y="4" width="20" height="5" rx="1"/><path d="M4 9v9a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9"/><line x1="10" y1="13" x2="14" y2="13"/></svg>`;
const RESTORE_ICON = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>`;
const TRASH_ICON = `<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg>`;

export function initSessionRail(opts = {}) {
  const activeTabBtn = document.getElementById('rail-tab-active');
  const archivedTabBtn = document.getElementById('rail-tab-archived');
  if (activeTabBtn) {
    activeTabBtn.addEventListener('click', () => {
      if (typeof opts.onFilterMode === 'function') opts.onFilterMode('active');
    });
  }
  if (archivedTabBtn) {
    archivedTabBtn.addEventListener('click', () => {
      if (typeof opts.onFilterMode === 'function') opts.onFilterMode('archived');
    });
  }
}

export function renderSessionRailList(containerEl, opts = {}) {
  if (!containerEl) return;
  const sessions = Array.isArray(opts.sessions) ? opts.sessions : [];
  const activeSessionId = opts.activeSessionId || null;
  const query = String(opts.query || '').trim().toLowerCase();
  const showArchived = opts.filterMode === 'archived';

  const archivedTotal = sessions.filter((s) => Boolean(s.archived)).length;
  const activeTabBtn = document.getElementById('rail-tab-active');
  const archivedTabBtn = document.getElementById('rail-tab-archived');
  const archivedCountEl = document.getElementById('rail-archived-count');

  if (activeTabBtn) {
    activeTabBtn.classList.toggle('is-active', !showArchived);
    activeTabBtn.setAttribute('aria-pressed', !showArchived ? 'true' : 'false');
  }
  if (archivedTabBtn) {
    archivedTabBtn.classList.toggle('is-active', showArchived);
    archivedTabBtn.setAttribute('aria-pressed', showArchived ? 'true' : 'false');
  }
  if (archivedCountEl) {
    archivedCountEl.textContent = String(archivedTotal);
    archivedCountEl.hidden = archivedTotal === 0;
  }

  const byArchive = sessions.filter((s) => Boolean(s.archived) === showArchived);
  const filtered = query
    ? byArchive.filter((s) => {
        const hay = `${s.title || ''} ${s.id || ''} ${s.workspace_root || ''} ${(s.stats && s.stats.model) || ''}`.toLowerCase();
        return hay.includes(query);
      })
    : byArchive;

  containerEl.innerHTML = '';
  if (filtered.length === 0) {
    const empty = document.createElement('div');
    empty.style.cssText = 'padding:10px 12px;font-size:12px;color:var(--ink-muted);';
    empty.textContent = query
      ? 'No chats match filter.'
      : (showArchived ? 'No archived chats.' : 'No active chats yet.');
    containerEl.appendChild(empty);
    return;
  }

  for (const s of filtered) {
    const row = document.createElement('div');
    row.className = `session-row${s.id === activeSessionId ? ' is-active' : ''}`;

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `session-item${s.id === activeSessionId ? ' is-active' : ''}`;
    if (s.id === activeSessionId) btn.setAttribute('aria-current', 'true');

    const stats = s.stats || {};
    const badges = [];
    if (s.running) {
      badges.push('<span class="mini-pill accent" title="Turn currently running">● Run</span>');
    }
    if (stats.verified_status === 'verified') {
      badges.push('<span class="mini-pill ok" title="Answer verified by model">✓</span>');
    } else if (stats.verified_status === 'unverified') {
      badges.push('<span class="mini-pill warn" title="Unverified draft">!</span>');
    } else if (stats.verified_status === 'proof_reviewed') {
      badges.push('<span class="mini-pill" title="Proof graph reviewed">Review</span>');
    }
    if ((Number(stats.proof_entries) || 0) > 0) {
      badges.push(`<span class="mini-pill" title="Proof-DAG steps">${Number(stats.proof_entries)}</span>`);
    }
    if (s.parent_session) {
      badges.push('<span class="mini-pill">Fork</span>');
    }

    const metaParts = [];
    if (stats.model) metaParts.push(stats.model);
    if (s.workspace_root) metaParts.push(basenameOf(s.workspace_root));
    if (metaParts.length === 0) metaParts.push(s.id);

    btn.innerHTML = `
      <div class="session-title-row">
        <span class="session-item-title">${mathTextToHtml(s.title || s.id)}</span>
        <span class="session-badges">${badges.join('')}</span>
      </div>
      <div class="session-item-meta"><span>${escapeHtml(metaParts.join(' · '))}</span></div>
    `;
    btn.addEventListener('click', () => {
      if (typeof opts.onSelect === 'function') opts.onSelect(s.id);
    });

    const actions = document.createElement('div');
    actions.className = 'session-row-actions';

    const archBtn = document.createElement('button');
    archBtn.type = 'button';
    archBtn.className = 'row-action-btn session-archive-btn';
    archBtn.title = s.archived ? 'Restore chat to Active' : 'Archive chat';
    archBtn.setAttribute('aria-label', archBtn.title);
    archBtn.innerHTML = s.archived ? RESTORE_ICON : ARCHIVE_ICON;
    archBtn.addEventListener('click', (ev) => {
      ev.stopPropagation();
      if (typeof opts.onArchive === 'function') opts.onArchive(s, !s.archived);
    });

    const delBtn = document.createElement('button');
    delBtn.type = 'button';
    delBtn.className = 'row-action-btn danger session-delete-btn';
    delBtn.title = 'Delete chat';
    delBtn.setAttribute('aria-label', `Delete ${s.title || s.id}`);
    delBtn.innerHTML = TRASH_ICON;
    delBtn.addEventListener('click', (ev) => {
      ev.stopPropagation();
      if (typeof opts.onDelete === 'function') opts.onDelete(s);
    });

    actions.append(archBtn, delBtn);
    row.append(btn, actions);
    containerEl.appendChild(row);
  }
}

export function renderSessionGrantsPill(pillEl, grants) {
  if (!pillEl) return;
  const arr = Array.isArray(grants) ? grants : [];
  if (arr.length === 0) {
    pillEl.hidden = true;
    pillEl.textContent = '';
    return;
  }
  const names = arr.map((g) => (
    g === 'execute-code' ? 'SymPy' : g === 'network-read' ? 'Literature' : g === 'write-workspace' ? 'Edits' : g
  ));
  pillEl.hidden = false;
  pillEl.textContent = `Session allowed: ${names.join(', ')} ×`;
}

export function updateWorkspaceBadges(wsPill, wsBtn, wsRoot) {
  if (wsPill) {
    if (wsRoot) {
      wsPill.hidden = false;
      wsPill.textContent = wsRoot;
      wsPill.title = `${wsRoot} (click to switch workspace folder)`;
    } else {
      wsPill.hidden = true;
      wsPill.title = '';
    }
  }
  if (wsBtn) {
    if (wsRoot) {
      wsBtn.textContent = `Workspace: ${basenameOf(wsRoot)}`;
      wsBtn.setAttribute('aria-pressed', 'true');
      wsBtn.title = `Attached workspace: ${wsRoot} (click to switch)`;
    } else {
      wsBtn.textContent = 'Workspace';
      wsBtn.setAttribute('aria-pressed', 'false');
      wsBtn.title = 'Attach local workspace directory';
    }
  }
}

export function formatBuiltinToolWarning(data = {}) {
  const name = String(data.tool_name || '?').slice(0, 80);
  const outcome = data.denied_or_failed
    ? 'reported denied or failed'
    : data.state === 'DONE' ? 'reported finished' : 'reported running';
  return `Antigravity built-in tool \`${name}\` ${outcome}.`;
}

export function showToast(msg) {
  const stack = document.getElementById('toast-stack');
  if (!stack) return;
  const item = document.createElement('div');
  item.className = 'toast-item';
  item.title = 'Click to dismiss';
  item.textContent = String(msg);
  item.addEventListener('click', () => item.remove());
  stack.appendChild(item);
  setTimeout(() => item.remove(), 3800);
}
