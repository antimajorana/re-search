// Left-rail New session mode menu (package F5; overview 5.1, 5.2, 5.3, 8.2 decisions 6 & 7).
// Explore and Solve are listed but disabled with "not built yet" until their profiles exist;
// nothing is faked, and "New research" sits under "More" until Solve and Explore ship.

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

if (typeof document !== 'undefined' && typeof document.getElementById === 'function') {
  const mount = document.getElementById('rail-new-menu');
  if (mount) bindNewMenu(mount);
}
