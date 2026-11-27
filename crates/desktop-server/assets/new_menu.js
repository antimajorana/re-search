// Left-rail New session mode menu (package F5; overview 5.1, 5.2, 5.3, 8.2 decisions 6 & 7)
// and visibility-flagged Explore setup card mount (package X4).
// Explore and Solve are listed but disabled with "not built yet" until enabled;
// nothing is faked, and "New research" sits under "More" until Solve and Explore ship.
import { renderIdeaSetupCard } from './idea_setup.js';
import { EXPLORE_VISIBILITY_STORAGE_KEY } from './idea_state.js';

// Single-place named constants for owner-accepted wording (overview 5.2, 5.3, 8.2 decision 7).
export const NOT_BUILT_YET_LABEL = 'not built yet';
export const MORE_MENU_LABEL = 'More';

export const MODE_MENU_ENTRIES = Object.freeze([
  Object.freeze({
    id: 'investigate',
    apiMode: 'investigation',
    buttonId: 'new-investigation-btn',
    label: 'Investigate',
    title: 'Read a paper and trace its argument',
    built: true,
    underMore: false,
  }),
  Object.freeze({
    id: 'explore',
    apiMode: 'explore',
    buttonId: 'new-explore-btn',
    label: 'Explore',
    title: `Explore an idea — ${NOT_BUILT_YET_LABEL}`,
    built: false,
    underMore: false,
  }),
  Object.freeze({
    id: 'solve',
    apiMode: 'solve',
    buttonId: 'new-solve-btn',
    label: 'Solve',
    title: `Solve a problem — ${NOT_BUILT_YET_LABEL}`,
    built: false,
    underMore: false,
  }),
  Object.freeze({
    id: 'research',
    apiMode: 'research',
    buttonId: 'new-research-btn',
    label: 'New research',
    title: 'Start a fresh research session',
    built: true,
    underMore: true,
  }),
]);

function makeModeButton(entry, onSelect) {
  const btn = document.createElement('button');
  btn.id = entry.buttonId;
  btn.type = 'button';
  btn.className = entry.built ? 'pill-btn wide' : 'pill-btn wide mode-disabled';
  btn.setAttribute('data-mode', entry.id);
  btn.title = String(entry.title || entry.label);

  if (entry.id === 'research') {
    const plus = document.createElement('span');
    plus.setAttribute('aria-hidden', 'true');
    plus.textContent = '+';
    const labelEl = document.createElement('span');
    labelEl.textContent = String(entry.label);
    btn.append(plus, labelEl);
  } else {
    const labelEl = document.createElement('span');
    labelEl.textContent = String(entry.label);
    btn.append(labelEl);
  }

  if (!entry.built) {
    btn.disabled = true;
    btn.setAttribute('disabled', '');
    btn.setAttribute('aria-disabled', 'true');
    const tag = document.createElement('span');
    tag.className = 'mini-pill mode-soon';
    tag.textContent = NOT_BUILT_YET_LABEL;
    btn.append(tag);
  }

  btn.addEventListener('click', (ev) => {
    if (!entry.built) {
      if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
      if (ev && typeof ev.stopPropagation === 'function') ev.stopPropagation();
      return;
    }
    if (typeof onSelect === 'function') onSelect(entry.apiMode, entry);
  });

  return btn;
}

/**
 * Render the New session mode menu into `containerEl` using safe DOM APIs only
 * (zero innerHTML writes). Unbuilt modes are disabled with "not built yet".
 */
export function renderNewMenu(containerEl, opts = {}) {
  if (!containerEl) return null;
  const entries = Array.isArray(opts.entries) ? opts.entries : MODE_MENU_ENTRIES;
  const onSelect = typeof opts.onSelect === 'function' ? opts.onSelect : null;

  containerEl.replaceChildren();
  const primary = entries.filter((e) => !e.underMore);
  const secondary = entries.filter((e) => e.underMore);

  for (const entry of primary) {
    containerEl.append(makeModeButton(entry, onSelect));
  }

  if (secondary.length > 0) {
    const more = document.createElement('details');
    more.id = 'rail-more-menu';
    more.className = 'rail-more-menu';
    const summary = document.createElement('summary');
    summary.className = 'rail-more-summary';
    summary.textContent = MORE_MENU_LABEL;
    more.append(summary);
    for (const entry of secondary) {
      more.append(makeModeButton(entry, onSelect));
    }
    containerEl.append(more);
  }

  return containerEl;
}

/**
 * Guard any statically rendered unbuilt-mode buttons inside `rootEl` so they remain
 * disabled and inert, and optionally forward enabled mode clicks to `opts.onSelect`.
 */
export function bindNewMenu(rootEl, opts = {}) {
  if (!rootEl || typeof rootEl.querySelector !== 'function') return;
  const entries = Array.isArray(opts.entries) ? opts.entries : MODE_MENU_ENTRIES;
  const onSelect = typeof opts.onSelect === 'function' ? opts.onSelect : null;

  for (const entry of entries) {
    const btn = rootEl.querySelector(`#${entry.buttonId}`);
    if (!btn) continue;
    if (!entry.built) {
      btn.disabled = true;
      btn.setAttribute('disabled', '');
      btn.setAttribute('aria-disabled', 'true');
      btn.addEventListener('click', (ev) => {
        if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
        if (ev && typeof ev.stopPropagation === 'function') ev.stopPropagation();
      });
    } else if (onSelect) {
      btn.addEventListener('click', () => onSelect(entry.apiMode, entry));
    }
  }
}

/**
 * Determine whether the Explore home card is visible (`explore.md` 3, `00-overview.md` 5.1).
 * Hidden behind a visibility/settings flag until Explore is enabled — never shown greyed out.
 */
export function isExploreCardVisible(opts = {}) {
  if (typeof opts.exploreEnabled === 'boolean') return opts.exploreEnabled;
  try {
    if (typeof localStorage !== 'undefined' && localStorage) {
      return localStorage.getItem(EXPLORE_VISIBILITY_STORAGE_KEY) === '1';
    }
  } catch {
    // Storage unavailable; stay hidden by default.
  }
  return false;
}

function ensureIdeaStylesheet() {
  if (typeof document === 'undefined' || !document.head || typeof document.querySelector !== 'function') {
    return;
  }
  if (document.querySelector('link[href="/assets/idea.css"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/assets/idea.css';
  document.head.appendChild(link);
}

function defaultFocusInvestigate() {
  if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return;
  const target =
    document.getElementById('investigation-v4-entry-input') ||
    document.getElementById('new-investigation-btn');
  if (target && typeof target.focus === 'function') target.focus();
}

/**
 * Mount the Explore setup card (`idea_setup.js`) into `containerEl` ONLY when the visibility
 * flag is enabled; otherwise hide and clear `containerEl` so it is never shown greyed out.
 */
export function mountExploreCard(containerEl, opts = {}) {
  if (!containerEl) return null;
  if (!isExploreCardVisible(opts)) {
    containerEl.hidden = true;
    containerEl.replaceChildren();
    return null;
  }
  ensureIdeaStylesheet();
  containerEl.hidden = false;
  const onCreated = typeof opts.onCreated === 'function'
    ? opts.onCreated
    : (res) => globalThis.__reSearchModeStages?.openExploreCreated?.(res);
  return renderIdeaSetupCard(containerEl, {
    onFocusInvestigate: defaultFocusInvestigate,
    ...opts,
    onCreated,
  });
}

// Solve menu enablement hook (package S13b; default stays disabled while S12's
// `DESKTOP_MODE_SOLVE_ENABLED` is `false` on the server so existing F5/S5 tests remain intact).
export const SOLVE_ENABLED_STORAGE_KEY = 're-search.solve.enabled';

export function isSolveMenuEnabled(opts = {}) {
  if (typeof opts.solveEnabled === 'boolean') return opts.solveEnabled;
  try {
    if (typeof localStorage !== 'undefined' && localStorage) {
      return localStorage.getItem(SOLVE_ENABLED_STORAGE_KEY) === '1';
    }
  } catch {
    // Storage unavailable; stay disabled until S12 enables Solve server-side.
  }
  return false;
}

export function buildModeMenuEntries({ solveBuilt = false, exploreBuilt = false } = {}) {
  return MODE_MENU_ENTRIES.map((entry) => {
    if (entry.id === 'solve' && solveBuilt) {
      return Object.freeze({
        ...entry,
        built: true,
        title: 'Solve a problem',
      });
    }
    if (entry.id === 'explore' && exploreBuilt) {
      return Object.freeze({
        ...entry,
        built: true,
        title: 'Explore an idea',
      });
    }
    return entry;
  });
}

export function enableSolveButton(rootEl, onSelect) {
  if (!rootEl || typeof rootEl.querySelector !== 'function') return null;
  const btn = rootEl.querySelector('#new-solve-btn');
  if (!btn) return null;
  btn.disabled = false;
  if (typeof btn.removeAttribute === 'function') {
    btn.removeAttribute('disabled');
    btn.removeAttribute('aria-disabled');
  }
  if (btn.classList && typeof btn.classList.remove === 'function') {
    btn.classList.remove('mode-disabled');
  }
  btn.title = 'Solve a problem';
  btn.replaceChildren();
  const labelEl = document.createElement('span');
  labelEl.textContent = 'Solve';
  btn.append(labelEl);
  const entry = buildModeMenuEntries({ solveBuilt: true }).find((e) => e.id === 'solve');
  btn.addEventListener('click', () => {
    if (typeof onSelect === 'function') {
      onSelect('solve', entry);
    } else if (typeof CustomEvent === 'function' && typeof document !== 'undefined' && typeof document.dispatchEvent === 'function') {
      document.dispatchEvent(new CustomEvent('re-search:new-mode', { detail: { mode: 'solve', entry } }));
    }
  });
  return btn;
}

if (typeof document !== 'undefined' && typeof document.getElementById === 'function') {
  const mount = document.getElementById('rail-new-menu');
  if (mount) {
    if (isSolveMenuEnabled()) {
      bindNewMenu(mount, { entries: buildModeMenuEntries({ solveBuilt: true }) });
      enableSolveButton(mount);
    } else {
      bindNewMenu(mount);
    }
  }
  const exploreHost = document.getElementById('home-explore-card');
  if (exploreHost) mountExploreCard(exploreHost);
}
