// Deterministic results list with filters, dependency counts, and safe TeX statement preview
// (Investigate V3, investigate.md 3, 9, 12). Uses textContent and renderMathTextInto exclusively.

import { createBadge } from './ui_badge.js';
import { renderMathTextInto } from './math.js';

// Design wording constants (overview 3.2 row 20, investigate.md 3 & 12, inv-a-v2-v5.md).
export const RESULTS_HEADING = 'Results in this paper';
export const RESULT_KINDS = Object.freeze([
  'theorem', 'lemma', 'proposition', 'corollary', 'conjecture', 'claim',
]);
export const DEFINITION_KINDS = Object.freeze(['definition', 'assumption']);
export const PHYSICS_EMPTY_NOTE =
  'No theorem environments found in this paper. Browse sections and labelled equations below.';
export const FILTER_EMPTY_NOTE = 'No items match this filter.';

const STATEMENT_KINDS = Object.freeze([...RESULT_KINDS, ...DEFINITION_KINDS, 'algorithm']);
const EXCLUDED_ALL_KINDS = Object.freeze(['proof', 'front_matter', 'bibliography']);
const PHYSICS_FALLBACK_KINDS = Object.freeze(['section', 'equation']);
const FILTER_PILLS = Object.freeze([
  { id: 'results', label: 'Results' },
  { id: 'definitions', label: 'Definitions' },
  { id: 'equations', label: 'Equations' },
  { id: 'all', label: 'All' },
]);
const KIND_LABELS = Object.freeze({
  theorem: 'Theorem', lemma: 'Lemma', proposition: 'Proposition', corollary: 'Corollary',
  conjecture: 'Conjecture', claim: 'Claim', definition: 'Definition', assumption: 'Assumption',
  algorithm: 'Algorithm', equation: 'Equation', section: 'Section', example: 'Example',
  remark: 'Remark', figure: 'Figure', table: 'Table',
});

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function formatMinutes(minutes) {
  const value = Number(minutes);
  if (!Number.isFinite(value) || value <= 0) return '';
  return value < 1 ? '< 1 min' : `${Math.round(value)} min`;
}

function sortByDocumentOrder(nodes) {
  return nodes
    .map((node, index) => ({ node, index }))
    .sort((a, b) => (a.node.order ?? a.index) - (b.node.order ?? b.index))
    .map(({ node }) => node);
}

export function filterResultNodes(nodes, filter = 'results') {
  const list = Array.isArray(nodes) ? nodes : [];
  const mode = String(filter || 'results').trim().toLowerCase();
  const filtered = list.filter((node) => {
    const kind = String(node?.kind || '').toLowerCase();
    if (!kind) return false;
    if (mode === 'definitions') return DEFINITION_KINDS.includes(kind);
    if (mode === 'statements') return STATEMENT_KINDS.includes(kind);
    if (mode === 'equations') return kind === 'equation';
    if (mode === 'all') return !EXCLUDED_ALL_KINDS.includes(kind);
    return RESULT_KINDS.includes(kind);
  });
  return sortByDocumentOrder(filtered);
}

function buildEdgeCounts(edges) {
  const stmtUses = new Map();
  const proofUses = new Map();
  const usedBy = new Map();
  const add = (map, key, val) => {
    if (!map.has(key)) map.set(key, new Set());
    map.get(key).add(val);
  };
  for (const edge of Array.isArray(edges) ? edges : []) {
    if (!edge || String(edge.kind || 'uses').toLowerCase() !== 'uses') continue;
    const src = edge.src ?? edge.from;
    const dst = edge.dst ?? edge.to;
    if (!src || !dst) continue;
    const role = String(edge.role || '').toLowerCase();
    if (role === 'statement') add(stmtUses, dst, src);
    else if (role === 'proof') add(proofUses, dst, src);
    add(usedBy, src, dst);
  }
  return (id) => ({
    statementUses: stmtUses.get(id)?.size || 0,
    proofUses: proofUses.get(id)?.size || 0,
    usedBy: usedBy.get(id)?.size || 0,
  });
}

function formatNodeHeading(node) {
  const kind = String(node?.kind || '').toLowerCase();
  const kindLabel = KIND_LABELS[kind] || (kind ? kind[0].toUpperCase() + kind.slice(1) : 'Item');
  const num = node?.number != null ? String(node.number).trim() : '';
  const title = node?.title != null ? String(node.title).trim() : '';
  if (num || title) {
    const base = num ? `${kindLabel} ${num}` : kindLabel;
    return title ? `${base}: ${title}` : base;
  }
  return String(node?.label || '').trim() || kindLabel;
}

function extractNodeText(entry) {
  if (entry == null) return '';
  if (typeof entry === 'string') return entry;
  return typeof entry === 'object' && entry.text != null ? String(entry.text) : '';
}

export function renderInvestigationResults(host, mapview, opts = {}) {
  const allNodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const statsByTag = new Map();
  for (const stat of Array.isArray(mapview?.stats) ? mapview.stats : []) {
    if (stat?.section_tag) statsByTag.set(String(stat.section_tag), stat);
  }
  const depsFor = buildEdgeCounts(mapview?.edges);
  const textsByTag = new Map();
  if (opts.nodeTexts && typeof opts.nodeTexts === 'object') {
    for (const [tag, entry] of Object.entries(opts.nodeTexts)) {
      const text = extractNodeText(entry);
      if (text) textsByTag.set(tag, text);
    }
  }

  let currentFilter = String(opts.filter || 'results').trim().toLowerCase();
  let selectedTag = opts.selectedTag ?? null;
  const cardEls = new Map();
  const previewEls = new Map();
  const filterButtons = new Map();

  const root = el('section', 'inv-results');
  root.setAttribute('aria-label', RESULTS_HEADING);

  const header = el('div', 'inv-results-header');
  const countEl = el('span', 'inv-results-count');
  header.append(el('h2', 'inv-results-heading', RESULTS_HEADING), countEl);

  const filterBar = el('div', 'inv-results-filters');
  filterBar.setAttribute('role', 'group');
  filterBar.setAttribute('aria-label', 'Filter paper items');
  for (const pill of FILTER_PILLS) {
    const btn = el('button', 'inv-results-filter-pill', pill.label);
    btn.type = 'button';
    btn.setAttribute('data-filter', pill.id);
    btn.addEventListener('click', () => {
      if (currentFilter === pill.id) return;
      currentFilter = pill.id;
      renderList();
      if (typeof opts.onFilterChange === 'function') opts.onFilterChange(currentFilter);
    });
    filterButtons.set(pill.id, btn);
    filterBar.appendChild(btn);
  }

  const listWrap = el('div', 'inv-results-list');
  root.append(header, filterBar, listWrap);

  const syncSelection = () => {
    for (const [tag, card] of cardEls.entries()) {
      const active = selectedTag != null && tag === selectedTag;
      if (active) {
        card.classList.add('is-selected');
        card.setAttribute('data-selected', 'true');
        card.setAttribute('aria-current', 'true');
      } else {
        card.classList.remove('is-selected');
        card.removeAttribute('data-selected');
        card.removeAttribute('aria-current');
      }
    }
  };

  const syncFilterPills = () => {
    for (const [id, btn] of filterButtons.entries()) {
      const active = id === currentFilter;
      if (active) btn.classList.add('is-active');
      else btn.classList.remove('is-active');
      btn.setAttribute('aria-pressed', active ? 'true' : 'false');
    }
  };

  const nodeByTag = new Map(allNodes.map((n) => [String(n.id ?? n.tag ?? ''), n]));

  const mountPreview = (card, tag, text) => {
    const trimmed = String(text ?? '').trim();
    let previewEl = previewEls.get(tag);
    if (!trimmed) {
      if (previewEl) previewEl.textContent = '';
      return;
    }
    if (!previewEl) {
      previewEl = el('div', 'inv-result-preview');
      previewEls.set(tag, previewEl);
      card.appendChild(previewEl);
    }
    if (nodeByTag.get(tag)?.render_math === false) previewEl.textContent = trimmed;
    else renderMathTextInto(previewEl, trimmed, { katex: opts.katex });
  };

  const buildCard = (node) => {
    const tag = String(node.id ?? node.tag ?? '');
    const card = el('article', 'inv-result-item');
    card.setAttribute('data-tag', tag);
    card.setAttribute('data-kind', String(node.kind || '').toLowerCase());
    card.setAttribute('role', 'button');
    card.tabIndex = 0;
    if (node.stale) card.setAttribute('data-stale', 'true');
    if (node.dim) card.setAttribute('data-dim', 'true');

    const head = el('div', 'inv-result-head');
    head.appendChild(el('h3', 'inv-result-title', formatNodeHeading(node)));
    if (tag) head.appendChild(el('code', 'inv-result-tag', tag));
    card.appendChild(head);

    const badges = Array.isArray(node.badges) ? node.badges : [];
    const flags = Array.isArray(node.flags) ? node.flags : [];
    if (badges.length || flags.length) {
      const badgeRow = el('div', 'inv-result-badges');
      for (const b of badges) badgeRow.appendChild(createBadge({ ...b, tone: node.stale ? 'muted' : b.tone }));
      for (const flag of flags) badgeRow.appendChild(el('span', 'inv-result-flag', flag));
      card.appendChild(badgeRow);
    }

    const deps = depsFor(tag);
    const timeText = formatMinutes(node.read_minutes ?? statsByTag.get(tag)?.read_minutes);
    const metaChips = [
      timeText,
      deps.statementUses > 0 ? `Statement uses: ${deps.statementUses}` : '',
      deps.proofUses > 0 ? `Proof uses: ${deps.proofUses}` : '',
      deps.usedBy > 0 ? `Used by: ${deps.usedBy}` : '',
    ].filter(Boolean);
    if (metaChips.length) {
      const metaRow = el('div', 'inv-result-meta');
      for (const chipText of metaChips) metaRow.appendChild(el('span', 'inv-result-meta-chip', chipText));
      card.appendChild(metaRow);
    }

    if (textsByTag.has(tag)) mountPreview(card, tag, textsByTag.get(tag));

    const activate = () => {
      selectedTag = tag;
      syncSelection();
      if (typeof opts.onSelectNode === 'function') opts.onSelectNode(tag);
    };
    card.addEventListener('click', activate);
    card.addEventListener('keydown', (ev) => {
      if (ev?.key === 'Enter' || ev?.key === ' ') { ev?.preventDefault?.(); activate(); }
    });
    cardEls.set(tag, card);
    return card;
  };

  const renderList = () => {
    cardEls.clear();
    previewEls.clear();
    syncFilterPills();

    let items = filterResultNodes(allNodes, currentFilter);
    const children = [];
    // Investigate.md §12: theorem-less (e.g. RevTeX physics) papers show sections & labelled equations immediately.
    if (currentFilter === 'results' && items.length === 0) {
      const note = el('p', 'inv-results-physics-note', PHYSICS_EMPTY_NOTE);
      note.setAttribute('role', 'status');
      children.push(note);
      items = sortByDocumentOrder(
        allNodes.filter((n) => PHYSICS_FALLBACK_KINDS.includes(String(n?.kind || '').toLowerCase())),
      );
    }

    countEl.textContent = String(items.length);
    countEl.setAttribute('data-count', String(items.length));
    if (items.length === 0) children.push(el('p', 'inv-results-empty', FILTER_EMPTY_NOTE));
    else for (const node of items) children.push(buildCard(node));
    listWrap.replaceChildren(...children);
    syncSelection();
  };

  host.replaceChildren(root);
  renderList();

  return {
    setSelected(tag) { selectedTag = tag ?? null; syncSelection(); },
    setFilter(filter) { currentFilter = String(filter || 'results').trim().toLowerCase(); renderList(); },
    setNodeText(tag, text) {
      const key = String(tag ?? '');
      if (!key) return;
      const raw = extractNodeText(text);
      if (raw) textsByTag.set(key, raw);
      else textsByTag.delete(key);
      const card = cardEls.get(key);
      if (card) mountPreview(card, key, raw);
    },
    destroy() { cardEls.clear(); previewEls.clear(); filterButtons.clear(); host.replaceChildren(); },
  };
}
