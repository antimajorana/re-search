// Solve expression handle rendering and Expressions table view (package S13c).
// Strict Rule R5: claim formulas read ONLY `solve.expr` handle LaTeX, never `step.latex`.
// Uses DOM createElement / textContent / KaTeX render only (zero raw HTML string writes).

import { katexOptions } from './math_katex.js';
import { createBadge } from './ui_badge.js';

export const RELATION_TEX = Object.freeze({ eq: '=', le: '\\le', ge: '\\ge', approx: '\\approx' });

const UNAVAILABLE_FORMULA_TEXT = 'Formula unavailable: handle missing or non-portable';

const stripNul = (value) => String(value ?? '').replace(/\u0000/g, '');

function resolveDoc(node, opts) {
  return opts?.document || node?.ownerDocument || (typeof document !== 'undefined' ? document : null);
}

function makeEl(doc, tag, className, text) {
  const el = doc.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined && text !== null) el.textContent = String(text);
  return el;
}

function handleIdOf(expr) {
  if (!expr || typeof expr !== 'object') return '';
  if (typeof expr.handle === 'string' && expr.handle.trim()) return stripNul(expr.handle).trim();
  const name = stripNul(expr.name ?? '').trim();
  return name && expr.version !== undefined && expr.version !== null ? `${name}@${expr.version}` : name;
}

function formatFreeSymbols(freeSymbols) {
  if (!Array.isArray(freeSymbols) || freeSymbols.length === 0) return '—';
  const parts = [];
  for (const sym of freeSymbols) {
    if (typeof sym === 'string') {
      const s = stripNul(sym).trim();
      if (s) parts.push(s);
      continue;
    }
    if (!sym || typeof sym !== 'object') continue;
    const name = stripNul(sym.name ?? '').trim();
    if (!name) continue;
    const assumptions = Array.isArray(sym.assumptions)
      ? sym.assumptions.map((a) => stripNul(a).trim()).filter(Boolean)
      : [];
    parts.push(assumptions.length ? `${name} (${assumptions.join(', ')})` : name);
  }
  return parts.length ? parts.join(', ') : '—';
}

function formatSeriesOrderNote(order) {
  if (!order || typeof order !== 'object') return '';
  const variable = stripNul(order.variable ?? '').trim();
  const nNum = Number(order.n);
  if (!variable || !Number.isInteger(nNum)) return '';
  const point = stripNul(order.point ?? '').trim();
  return point && point !== '0'
    ? `to O((${variable} - ${point})^${nNum})`
    : `to O(${variable}^${nNum})`;
}

export function buildHandleLookup(handles = []) {
  const map = new Map();
  if (!Array.isArray(handles)) return map;
  for (const h of handles) {
    if (!h || typeof h !== 'object') continue;
    const name = typeof h.name === 'string' ? stripNul(h.name).trim() : '';
    if (name && h.version !== undefined && h.version !== null) map.set(`${name}@${h.version}`, h);
    if (typeof h.handle === 'string') {
      const key = stripNul(h.handle).trim();
      if (key) map.set(key, h);
    }
  }
  return map;
}

export function renderHandleLatex(container, latex, katexImpl = globalThis.window?.katex, { display = false } = {}) {
  if (!container) return null;
  const doc = resolveDoc(container);
  if (!doc) return null;
  container.replaceChildren();
  const clean = stripNul(latex).trim();
  const span = makeEl(doc, 'span', 'solve-math-expr');
  if (katexImpl && typeof katexImpl.render === 'function' && clean) {
    try {
      katexImpl.render(clean, span, katexOptions(Boolean(display)));
    } catch {
      span.replaceChildren();
      span.textContent = clean;
    }
  } else {
    span.textContent = clean;
  }
  container.appendChild(span);
  return span;
}

export function renderClaimFormula(container, claim, handleMap, katexImpl = globalThis.window?.katex, opts = {}) {
  if (!container) return false;
  const doc = resolveDoc(container, opts);
  if (!doc) return false;
  const lookup = handleMap instanceof Map ? handleMap : buildHandleLookup(Array.isArray(handleMap) ? handleMap : []);
  const lhsKey = typeof claim?.lhs === 'string' ? stripNul(claim.lhs).trim() : '';
  const rhsKey = typeof claim?.rhs === 'string' ? stripNul(claim.rhs).trim() : '';
  const lhsExpr = lhsKey ? lookup.get(lhsKey) : undefined;
  const rhsExpr = rhsKey ? lookup.get(rhsKey) : undefined;
  const lhsLatex = typeof lhsExpr?.latex === 'string' ? stripNul(lhsExpr.latex).trim() : '';
  const rhsLatex = typeof rhsExpr?.latex === 'string' ? stripNul(rhsExpr.latex).trim() : '';

  if (!lhsExpr || !rhsExpr || lhsExpr.portable !== true || rhsExpr.portable !== true || !lhsLatex || !rhsLatex) {
    container.replaceChildren();
    const msg = makeEl(doc, 'span', 'solve-math-unavailable', UNAVAILABLE_FORMULA_TEXT);
    msg.setAttribute('data-handle-unavailable', 'true');
    container.appendChild(msg);
    return false;
  }

  const relKey = stripNul(claim?.relation ?? 'eq').trim().toLowerCase();
  const relTex = Object.prototype.hasOwnProperty.call(RELATION_TEX, relKey) ? RELATION_TEX[relKey] : RELATION_TEX.eq;
  const composed = `${lhsLatex} ${relTex} ${rhsLatex}`;
  renderHandleLatex(container, composed, katexImpl, { display: opts.display !== false });
  const orderNote = formatSeriesOrderNote(claim?.order);
  if (orderNote) {
    container.appendChild(makeEl(doc, 'span', 'solve-series-order', orderNote));
  }
  return true;
}

export function createSolveExprsView(host, handles = [], opts = {}) {
  const doc = resolveDoc(host, opts);
  if (!doc) return null;
  let currentHandles = Array.isArray(handles) ? [...handles] : [];
  let destroyed = false;

  const root = makeEl(doc, 'section', 'solve-exprs-view');
  root.setAttribute('data-testid', 'solve-exprs-view');
  const controls = makeEl(doc, 'div', 'solve-exprs-controls');
  const searchInput = makeEl(doc, 'input', 'solve-exprs-search');
  searchInput.type = 'search';
  searchInput.setAttribute('data-testid', 'solve-exprs-search');
  searchInput.placeholder = 'Filter expressions...';
  searchInput.setAttribute('aria-label', 'Filter expressions');

  const filterLabel = makeEl(doc, 'label', 'solve-check-row');
  const nonPortableCheckbox = makeEl(doc, 'input');
  nonPortableCheckbox.type = 'checkbox';
  nonPortableCheckbox.setAttribute('data-testid', 'solve-exprs-nonportable-only');
  filterLabel.append(nonPortableCheckbox, makeEl(doc, 'span', '', 'Non-portable only'));
  const countEl = makeEl(doc, 'span', 'solve-muted solve-exprs-count');
  controls.append(searchInput, filterLabel, countEl);

  const table = makeEl(doc, 'table', 'solve-exprs-table');
  const headRow = makeEl(doc, 'tr');
  for (const label of ['Handle', 'Expression', 'Preview', 'Free symbols', 'Owner step', 'Status']) {
    const th = makeEl(doc, 'th', '', label);
    th.scope = 'col';
    headRow.appendChild(th);
  }
  const thead = makeEl(doc, 'thead');
  thead.appendChild(headRow);
  const tbody = makeEl(doc, 'tbody');
  table.append(thead, tbody);
  root.append(controls, table);

  function renderRows() {
    if (destroyed) return;
    tbody.replaceChildren();
    const query = stripNul(searchInput.value).trim().toLowerCase();
    const nonPortableOnly = Boolean(nonPortableCheckbox.checked);
    const katexImpl = opts.katex !== undefined ? opts.katex : globalThis.window?.katex;

    const filtered = currentHandles.filter((expr) => {
      if (!expr || typeof expr !== 'object') return false;
      if (nonPortableOnly && expr.portable === true) return false;
      if (!query) return true;
      const haystack = [
        handleIdOf(expr),
        stripNul(expr.str_preview ?? ''),
        stripNul(expr.latex ?? ''),
        stripNul(expr.owner_entry ?? ''),
        formatFreeSymbols(expr.free_symbols),
      ].join(' ').toLowerCase();
      return haystack.includes(query);
    });

    countEl.textContent = `${filtered.length} of ${currentHandles.length}`;
    if (filtered.length === 0) {
      const td = makeEl(doc, 'td', 'solve-muted', 'No matching expressions.');
      td.colSpan = 6;
      const emptyRow = makeEl(doc, 'tr', 'solve-exprs-empty-row');
      emptyRow.appendChild(td);
      tbody.appendChild(emptyRow);
      return;
    }

    for (const expr of filtered) {
      const handleKey = handleIdOf(expr);
      const isPortable = expr.portable === true;
      const tr = makeEl(doc, 'tr', 'solve-exprs-row');
      tr.setAttribute('data-handle', handleKey);
      tr.setAttribute('data-portable', isPortable ? 'true' : 'false');

      const tdHandle = makeEl(doc, 'td');
      tdHandle.appendChild(makeEl(doc, 'code', 'solve-expr-handle', handleKey));

      const tdLatex = makeEl(doc, 'td', 'solve-expr-latex-cell');
      renderHandleLatex(tdLatex, expr.latex ?? '', katexImpl, { display: false });

      const tdPreview = makeEl(doc, 'td');
      tdPreview.appendChild(makeEl(doc, 'code', 'solve-expr-preview', stripNul(expr.str_preview ?? '')));

      const tdSymbols = makeEl(doc, 'td', '', formatFreeSymbols(expr.free_symbols));

      const tdOwner = makeEl(doc, 'td');
      const ownerId = stripNul(expr.owner_entry ?? '').trim();
      if (ownerId) {
        const ownerBtn = makeEl(doc, 'button', 'solve-step-link', ownerId);
        ownerBtn.type = 'button';
        ownerBtn.setAttribute('data-jump-step', ownerId);
        if (typeof opts.onSelectStep === 'function') ownerBtn.addEventListener('click', () => opts.onSelectStep(ownerId));
        else ownerBtn.disabled = true;
        tdOwner.appendChild(ownerBtn);
      } else {
        tdOwner.appendChild(makeEl(doc, 'span', 'solve-muted', '—'));
      }

      const tdPortable = makeEl(doc, 'td');
      tdPortable.appendChild(isPortable
        ? createBadge({ text: 'Portable', icon: 'code', tone: 'ok' })
        : createBadge({ text: 'Non-portable', icon: 'warn', tone: 'warn', title: 'Cannot be named in a claim' }));

      tr.append(tdHandle, tdLatex, tdPreview, tdSymbols, tdOwner, tdPortable);
      tbody.appendChild(tr);
    }
  }

  searchInput.addEventListener('input', renderRows);
  nonPortableCheckbox.addEventListener('change', renderRows);
  renderRows();
  if (host && typeof host.replaceChildren === 'function') host.replaceChildren(root);
  else if (host && typeof host.appendChild === 'function') host.appendChild(root);

  return {
    element: root,
    update(nextHandles) {
      if (destroyed) return;
      currentHandles = Array.isArray(nextHandles) ? [...nextHandles] : [];
      renderRows();
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      searchInput.removeEventListener?.('input', renderRows);
      nonPortableCheckbox.removeEventListener?.('change', renderRows);
      if (typeof root.remove === 'function') root.remove();
      if (host && typeof host.replaceChildren === 'function') host.replaceChildren();
    },
  };
}
