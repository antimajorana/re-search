// Deterministic reading-plan strip and nearest-definition symbol lookup (Investigate V5, investigate.md 3 & 6.14).
// Uses textContent and DOM builders exclusively; never uses forbidden wording.

import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';

// Design wording constants (00-overview.md 3.2 row 20, investigate.md 3 Stage F, 6.14 & 12).
export const SKIP_EMPTY_NOTE = 'No main results selected yet — nothing is dimmed by cross-references.';
export const SKIP_BASIS_EXPLICIT = 'by explicit cross-references';
export const SKIP_BASIS_MODEL = 'by cross-references (includes model-inferred links)';
export const SYMBOL_FOUND_LABEL = 'found by pattern';
export const SYMBOL_NOT_FOUND_LABEL = 'No definition found above this line';

const DELIM_PAIRS = Object.freeze([['$$', '$$'], ['$', '$'], ['\\(', '\\)'], ['\\[', '\\]']]);
const LAYOUT_CMD_RE = /\\(?:left|right|qquad|quad)(?![A-Za-z])|\\[,;!]/g;
const STYLING_MACROS = new Set([
  '\\mathcal', '\\mathbb', '\\mathbf', '\\mathrm', '\\mathfrak', '\\mathscr', '\\mathsf', '\\mathtt',
  '\\boldsymbol', '\\bm', '\\text', '\\textrm', '\\textit', '\\textbf', '\\operatorname',
  '\\hat', '\\widehat', '\\tilde', '\\widetilde', '\\bar', '\\overline', '\\underline', '\\vec', '\\dot', '\\ddot',
]);
const SECTION_KINDS = new Set(['section', 'front_matter', 'bibliography']);
const NEED_TITLES = Object.freeze({
  statement: 'Referenced by main result statements',
  proof: 'Referenced by main result proofs',
  unreferenced: 'Not referenced by any main result (by cross-references)',
});

function el(tag, className, text, attrs) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  if (attrs) for (const [k, v] of Object.entries(attrs)) if (v != null) node.setAttribute(k, String(v));
  return node;
}

function makeButton(className, text, attrs, onClick) {
  const button = el('button', className, text, attrs);
  button.type = 'button';
  if (onClick) button.addEventListener('click', onClick);
  return button;
}

function hasWrappingBraces(str) {
  if (str.length < 2 || str[0] !== '{' || str[str.length - 1] !== '}') return false;
  let depth = 0;
  for (let i = 0; i < str.length; i += 1) {
    if (str[i] === '\\') { i += 1; continue; }
    if (str[i] === '{') depth += 1;
    else if (str[i] === '}' && (--depth < 0 || (depth === 0 && i < str.length - 1))) return false;
  }
  return depth === 0;
}

function stripOuterDelimiters(str) {
  let s = str.trim();
  for (let changed = true; changed;) {
    changed = false;
    for (const [open, close] of DELIM_PAIRS) {
      if (s.startsWith(open) && s.endsWith(close) && s.length >= open.length + close.length) {
        s = s.slice(open.length, s.length - close.length).trim();
        changed = true;
        break;
      }
    }
    if (!changed && hasWrappingBraces(s)) { s = s.slice(1, -1).trim(); changed = true; }
  }
  return s;
}

export function normalizeSymbol(raw) {
  if (raw == null) return '';
  const stripped = stripOuterDelimiters(String(raw)).replace(LAYOUT_CMD_RE, (m, offset, full) => {
    const prev = offset > 0 ? full[offset - 1] : '';
    const next = offset + m.length < full.length ? full[offset + m.length] : '';
    return /[A-Za-z]/.test(prev) && /[A-Za-z]/.test(next) ? ' ' : '';
  });
  return stripOuterDelimiters(stripped).replace(/\s+/g, ' ').trim();
}

export function symbolLookupKeys(raw) {
  const norm = normalizeSymbol(raw);
  if (!norm) return [];
  const keys = [norm];
  const m = /^(\\[A-Za-z]+)(?:\s*[\[{(_^]|$)/.exec(norm);
  if (m && m[1] !== norm && !STYLING_MACROS.has(m[1])) keys.push(m[1]);
  return keys;
}

function findAncestor(nodeByTag, node, pred) {
  let cur = node?.parent ? String(node.parent) : null;
  const seen = new Set();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const parent = nodeByTag.get(cur);
    if (pred(cur, parent)) return parent || cur;
    cur = parent?.parent ? String(parent.parent) : null;
  }
  return null;
}

const isAncestorNode = (nodeByTag, ancestorTag, node) => Boolean(findAncestor(nodeByTag, node, (tag) => tag === ancestorTag));
const toFinite = (v) => (v != null && Number.isFinite(Number(v)) ? Number(v) : null);

function formatNodeLabel(node) {
  const kind = String(node?.kind || '').trim().toLowerCase();
  const num = node?.number != null ? String(node.number).trim() : '';
  const title = node?.title != null ? String(node.title).trim() : '';
  if (kind === 'front_matter') return title || 'Front matter';
  if (kind === 'bibliography') return title || 'Bibliography';
  if (kind === 'section') return num && title ? `Section ${num}: ${title}` : (num ? `Section ${num}` : (title || String(node?.label || node?.tag || node?.id || 'Section')));
  const word = kind ? kind[0].toUpperCase() + kind.slice(1).replace(/_/g, ' ') : 'Node';
  return num ? `${word} ${num}` : (title || String(node?.label || node?.tag || node?.id || word));
}

function formatSymbolSectionLabel(sym, nodeByTag) {
  if (!sym) return null;
  if (!sym.in_node) return 'Preamble';
  const tag = String(sym.in_node);
  const node = nodeByTag.get(tag);
  if (!node) return tag;
  const selfLabel = formatNodeLabel(node);
  if (SECTION_KINDS.has(String(node.kind || '').toLowerCase())) return selfLabel;
  const sec = findAncestor(nodeByTag, node, (_, p) => p && SECTION_KINDS.has(String(p.kind || '').toLowerCase()));
  return sec ? `${formatNodeLabel(sec)} — ${selfLabel}` : selfLabel;
}

function compareCandidateRank(a, b, nodeByTag) {
  if (a.tier !== b.tier) return a.tier - b.tier;
  const nested = a.path && b.path && a.path === b.path && a.tag && b.tag && a.tag !== b.tag && (
    isAncestorNode(nodeByTag, a.tag, nodeByTag.get(b.tag)) || isAncestorNode(nodeByTag, b.tag, nodeByTag.get(a.tag))
  );
  if (nested && a.line !== b.line) return a.line - b.line;
  if (a.order !== b.order) return a.order - b.order;
  return a.line !== b.line ? a.line - b.line : a.index - b.index;
}

export function lookupNearestSymbol(symbols, nodes, queryTex, atNodeTag = null, atLine = null) {
  const query = String(queryTex ?? '');
  const norm = normalizeSymbol(query);
  const keys = symbolLookupKeys(query);
  if (keys.length === 0 || !Array.isArray(symbols) || symbols.length === 0) {
    return { query, norm, found: null, sectionLabel: null, basisLabel: SYMBOL_NOT_FOUND_LABEL };
  }

  const keySet = new Set(keys);
  const nodeByTag = new Map();
  for (const node of (Array.isArray(nodes) ? nodes : [])) {
    if (!node || typeof node !== 'object') continue;
    if (node.tag != null) nodeByTag.set(String(node.tag), node);
    if (node.id != null) nodeByTag.set(String(node.id), node);
  }

  const atTag = atNodeTag != null ? String(atNodeTag) : null;
  const atNode = atTag != null ? (nodeByTag.get(atTag) || null) : null;
  const atOrder = toFinite(atNode?.order);
  const atLineNum = toFinite(atLine);
  const atSpan = Array.isArray(atNode?.spans) && atNode.spans.length > 0 ? atNode.spans[0] : null;
  const refLine = atLineNum != null ? atLineNum : toFinite(atSpan?.start_line);

  let best = null;
  for (let i = 0; i < symbols.length; i += 1) {
    const sym = symbols[i];
    if (!sym || typeof sym !== 'object') continue;
    const symNorm = typeof sym.norm === 'string' && sym.norm ? sym.norm : normalizeSymbol(sym.symbol_tex);
    if (!keySet.has(symNorm)) continue;

    const isPreamble = sym.how === 'newcommand' && !sym.in_node;
    const symTag = sym.in_node ? String(sym.in_node) : null;
    const symNode = symTag ? (nodeByTag.get(symTag) || null) : null;
    const symOrder = toFinite(symNode?.order) ?? -1;
    const symPath = sym.span?.path != null ? String(sym.span.path) : '';
    const symLine = toFinite(sym.span?.start_line) ?? -1;

    let before = true;
    if (isPreamble) {
      if (atTag == null && atLineNum != null && symLine >= 0) before = symLine < atLineNum;
    } else if (atTag != null) {
      if (symTag === atTag) before = atLineNum != null && symLine >= 0 && symLine < atLineNum;
      else if (symTag && atNode && isAncestorNode(nodeByTag, symTag, atNode) && atSpan && symPath === atSpan.path) {
        before = symLine >= 0 && refLine != null && symLine < refLine;
      } else if (atOrder != null && symNode != null) before = symOrder < atOrder;
      else if (!symTag && atSpan && symPath === atSpan.path && symLine >= 0) before = refLine != null && symLine < refLine;
      else if (atLineNum != null && symLine >= 0) before = symLine < atLineNum;
    } else if (atLineNum != null && symLine >= 0) before = symLine < atLineNum;

    if (!before) continue;
    const cand = { sym, tag: symTag, tier: isPreamble ? 0 : 1, order: symOrder, line: symLine, path: symPath, index: i };
    if (!best || compareCandidateRank(cand, best, nodeByTag) > 0) best = cand;
  }

  const found = best ? best.sym : null;
  return { query, norm, found, sectionLabel: found ? formatSymbolSectionLabel(found, nodeByTag) : null, basisLabel: found ? SYMBOL_FOUND_LABEL : SYMBOL_NOT_FOUND_LABEL };
}

function normalizeNeed(raw) {
  const s = String(raw || '').trim().toLowerCase();
  return (s === 'statement' || s === 'statements' || s === 'main_statement') ? 'statement'
    : (s === 'proof' || s === 'proofs' || s === 'full_proof') ? 'proof' : 'unreferenced';
}

function formatPlanMinutes(minutes) {
  const n = toFinite(minutes);
  return n == null || n <= 0 ? '0 min' : (n < 1 ? '< 1 min' : `${Math.round(n)} min`);
}

function formatSectionPillLabel(sec) {
  if (sec?.display_label) return String(sec.display_label).trim();
  const num = sec?.number != null ? String(sec.number).trim() : '';
  const title = sec?.title != null ? String(sec.title).trim() : '';
  if (num && title) return `${num} ${title}`;
  return title || (num ? `Section ${num}` : (sec?.label ? String(sec.label).trim() : String(sec?.section_tag ?? sec?.tag ?? '')));
}

export function renderReadingPlanStrip(host, skipPlan, opts = {}) {
  if (!skipPlan || skipPlan.empty) {
    const strip = el('section', 'inv-skip-strip inv-skip-strip--empty', undefined, {
      'aria-label': 'Reading plan', 'data-empty': 'true', 'data-dim-enabled': 'false',
    });
    strip.append(
      createBadge({ text: 'Reading plan', icon: 'info', tone: 'neutral', border: BORDER_SOLID }),
      el('p', 'inv-skip-empty', SKIP_EMPTY_NOTE),
    );
    host.replaceChildren(strip);
    return { setDimEnabled() {}, destroy() { host.replaceChildren(); } };
  }

  let dimEnabled = opts.dimUnreferenced !== false;
  const sections = Array.isArray(skipPlan.sections) ? skipPlan.sections : [];
  const sumNeed = (pred) => sections.reduce((acc, s) => {
    const m = Number(s?.read_minutes || 0);
    return pred(normalizeNeed(s?.need ?? s?.status)) && Number.isFinite(m) && m > 0 ? acc + m : acc;
  }, 0);

  const stmtMin = skipPlan.statement_minutes ?? skipPlan.statements_minutes ?? sumNeed((k) => k === 'statement');
  const proofMin = skipPlan.proof_minutes ?? skipPlan.proofs_minutes ?? sumNeed((k) => k === 'statement' || k === 'proof');
  const unrefMin = skipPlan.unreferenced_minutes ?? sumNeed((k) => k === 'unreferenced');

  const strip = el('section', 'inv-skip-strip', undefined, {
    'aria-label': 'Reading plan', 'data-empty': 'false', 'data-basis': String(skipPlan.basis || 'explicit'),
  });
  const header = el('div', 'inv-skip-header');
  const isModelBasis = skipPlan.basis === 'includes_model_inferred';
  header.appendChild(createBadge(isModelBasis
    ? { text: skipPlan.basis_text || SKIP_BASIS_MODEL, icon: 'model', tone: 'warn', border: BORDER_DASHED }
    : { text: skipPlan.basis_text || SKIP_BASIS_EXPLICIT, icon: 'code', tone: 'ok', border: BORDER_SOLID }));

  const times = el('div', 'inv-skip-times');
  times.append(
    el('span', 'inv-skip-time', `Main statements: ${formatPlanMinutes(stmtMin)}`, { 'data-time-kind': 'statement' }),
    el('span', 'inv-skip-time', `Full proofs: ${formatPlanMinutes(proofMin)}`, { 'data-time-kind': 'proof' }),
  );
  if (Number.isFinite(Number(unrefMin)) && Number(unrefMin) > 0) {
    times.appendChild(el('span', 'inv-skip-time', `Unreferenced: ${formatPlanMinutes(unrefMin)}`, { 'data-time-kind': 'unreferenced' }));
  }

  const sectionButtons = [];
  const syncDim = () => {
    strip.setAttribute('data-dim-enabled', dimEnabled ? 'true' : 'false');
    toggleBtn.setAttribute('aria-pressed', dimEnabled ? 'true' : 'false');
    for (const item of sectionButtons) {
      const dimmed = dimEnabled && item.need === 'unreferenced';
      item.btn.setAttribute('data-dim', dimmed ? 'true' : 'false');
      item.btn.classList.toggle('is-dimmed', dimmed);
    }
  };
  const toggleBtn = makeButton('inv-skip-dim-toggle', 'Dim unreferenced sections', null, () => {
    dimEnabled = !dimEnabled;
    syncDim();
    opts.onToggleDim?.(dimEnabled);
  });
  header.append(times, toggleBtn);
  strip.appendChild(header);
  if (skipPlan.summary_line) strip.appendChild(el('p', 'inv-skip-summary', String(skipPlan.summary_line)));

  const sectionList = el('div', 'inv-skip-sections');
  for (const sec of sections) {
    const need = normalizeNeed(sec?.need ?? sec?.status);
    const secTag = sec?.section_tag ?? sec?.tag ?? '';
    const btn = makeButton('inv-skip-section', undefined, { 'data-section-tag': String(secTag), 'data-need': need },
      () => { opts.onSelectSection?.(sec?.section_tag ?? secTag); });
    btn.title = NEED_TITLES[need];
    btn.appendChild(el('span', 'inv-skip-section-label', formatSectionPillLabel(sec)));
    if (Number.isFinite(Number(sec?.read_minutes)) && Number(sec.read_minutes) > 0) {
      btn.appendChild(el('span', 'inv-skip-section-time', formatPlanMinutes(sec.read_minutes)));
    }
    sectionButtons.push({ btn, need });
    sectionList.appendChild(btn);
  }
  strip.appendChild(sectionList);
  syncDim();
  host.replaceChildren(strip);
  return {
    setDimEnabled(v) { dimEnabled = Boolean(v); syncDim(); },
    destroy() { sectionButtons.length = 0; host.replaceChildren(); },
  };
}

export function renderSymbolPopover(host, result, opts = {}) {
  const found = result?.found || null;
  let closed = false;
  const close = () => { if (closed) return; closed = true; host.replaceChildren(); opts.onClose?.(); };

  const popover = el('div', 'inv-symbol-popover', undefined, {
    role: 'dialog', 'aria-label': 'Symbol definition', 'data-found': found ? 'true' : 'false',
    ...(found?.how ? { 'data-how': String(found.how) } : {}),
  });
  const header = el('div', 'inv-symbol-popover-header');
  header.append(
    el('code', 'inv-symbol-query', String(result?.query ?? '')),
    createBadge(found
      ? { text: SYMBOL_FOUND_LABEL, icon: 'info', tone: 'info', border: BORDER_DASHED }
      : { text: 'Not found', icon: 'warn', tone: 'muted', border: BORDER_DASHED }),
    makeButton('inv-symbol-close', 'Close', { 'aria-label': 'Close' }, close),
  );

  const body = el('div', 'inv-symbol-popover-body');
  if (found) {
    body.appendChild(el('p', 'inv-symbol-how', found.how === 'newcommand' ? 'Macro definition' : 'Nearest definition above'));
    const span = found.span;
    const locPart = span?.path
      ? (span.start_line != null ? `${span.path}:${span.start_line}` : String(span.path))
      : (span?.start_line != null ? `line ${span.start_line}` : '');
    const locText = [String(result?.sectionLabel || '').trim(), locPart].filter(Boolean).join(' · ');
    if (locText) body.appendChild(el('p', 'inv-symbol-location', locText));
    if (opts.excerpt != null && String(opts.excerpt).trim() !== '') {
      body.appendChild(el('pre', 'inv-symbol-excerpt', String(opts.excerpt)));
    }
    body.appendChild(makeButton('inv-symbol-jump', 'Jump to definition', null, () => { opts.onJumpToDefinition?.(found); }));
  } else {
    body.appendChild(el('p', 'inv-symbol-empty', SYMBOL_NOT_FOUND_LABEL));
  }

  popover.append(header, body);
  host.replaceChildren(popover);
  return { close };
}
