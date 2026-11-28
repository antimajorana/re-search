// Focus, caret, and scroll preservation across full-tree DOM rebuilds (F1).
// Uses attribute equality scanning instead of interpolated CSS selectors to avoid selector injection.

const FOCUS_KEYS = Object.freeze(['data-testid', 'aria-label', 'data-action', 'data-kind']);

const SELECTABLE_INPUT_TYPES = new Set(['', 'text', 'search', 'url', 'tel', 'password']);

function isTextSelectable(node) {
  if (!node || typeof node.setSelectionRange !== 'function' || typeof node.value !== 'string') {
    return false;
  }
  const tag = String(node.tagName || '').toUpperCase();
  if (tag === 'TEXTAREA') return true;
  if (tag === 'INPUT') {
    return SELECTABLE_INPUT_TYPES.has(String(node.type || '').toLowerCase());
  }
  return false;
}

function captureFocusKey(node) {
  if (!node || typeof node.getAttribute !== 'function') return null;
  for (const attr of FOCUS_KEYS) {
    const val = node.getAttribute(attr);
    if (val !== null && val !== '') return { attr, val };
  }
  return null;
}

function findByFocusKey(root, key) {
  if (!root || !key) return null;
  if (typeof root.getAttribute === 'function' && root.getAttribute(key.attr) === key.val) {
    return root;
  }
  if (typeof root.querySelectorAll !== 'function') return null;
  return [...root.querySelectorAll(`[${key.attr}]`)]
    .find((x) => typeof x?.getAttribute === 'function' && x.getAttribute(key.attr) === key.val) || null;
}

/**
 * Run `rebuild()` while preserving keyboard focus, text selection, and scrollTop
 * for whichever control inside `root` was active beforehand.
 */
export function keepFocus(root, rebuild) {
  const doc = root?.ownerDocument || (typeof document !== 'undefined' ? document : null);
  const active = doc?.activeElement || null;
  const inside = Boolean(
    active && root && (active === root || (typeof root.contains === 'function' && root.contains(active))),
  );
  if (!inside) return rebuild();

  const key = captureFocusKey(active);
  const prevVal = typeof active.value === 'string' ? active.value : null;
  const selStart = typeof active.selectionStart === 'number' ? active.selectionStart : null;
  const selEnd = typeof active.selectionEnd === 'number' ? active.selectionEnd : null;
  const selDir = typeof active.selectionDirection === 'string' ? active.selectionDirection : 'none';
  const scrollTop = typeof active.scrollTop === 'number' ? active.scrollTop : null;

  const result = rebuild();
  if (!key) return result;

  const next = findByFocusKey(root, key);
  if (!next || typeof next.focus !== 'function') return result;

  try {
    next.focus({ preventScroll: true });
  } catch {
    next.focus();
  }

  const valueMatches = prevVal !== null && typeof next.value === 'string' && next.value === prevVal;
  if (isTextSelectable(next)) {
    try {
      if (valueMatches && selStart !== null && selEnd !== null) {
        next.setSelectionRange(selStart, selEnd, selDir);
      } else {
        const end = next.value.length;
        next.setSelectionRange(end, end);
      }
    } catch { /* element type does not support selection range */ }
  }
  if (valueMatches && scrollTop !== null && typeof next.scrollTop === 'number') {
    next.scrollTop = scrollTop;
  }
  return result;
}
