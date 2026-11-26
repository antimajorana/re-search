// Deterministic paper outline tree and skeleton quality banner (Investigate V3, investigate.md 3, 4, 9).
// Uses textContent and DOM builders exclusively.

import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';

// Design wording constants (overview 3.2, investigate.md 3 & 4.2, inv-a-v2-v5.md).
export const OUTLINE_HEADING = 'Outline';
export const QUALITY_FALLBACK_LABEL = 'Structure not available from TeX';
export const OUTLINE_NAV_LABEL = 'Paper outline';
export const OUTLINE_EMPTY_NOTE = 'No sections detected in this paper.';

const OUTLINE_KINDS = Object.freeze(['front_matter', 'section', 'bibliography']);
const RESULT_COUNT_KINDS = Object.freeze([
  'theorem', 'lemma', 'proposition', 'corollary', 'conjecture', 'claim',
]);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function formatReadMinutes(minutes) {
  const value = Number(minutes);
  if (!Number.isFinite(value) || value <= 0) return '';
  return value < 1 ? '< 1 min' : `${Math.round(value)} min`;
}

function countResultKinds(counts) {
  if (!counts || typeof counts !== 'object') return 0;
  return RESULT_COUNT_KINDS.reduce((sum, k) => {
    const n = Number(counts[k] || 0);
    return sum + (Number.isFinite(n) && n > 0 ? n : 0);
  }, 0);
}

function deriveChildCounts(nodes) {
  const byParent = new Map();
  for (const node of nodes) {
    const parentId = node?.parent;
    const kind = String(node?.kind || '').toLowerCase();
    if (!parentId || !kind || OUTLINE_KINDS.includes(kind)) continue;
    if (!byParent.has(parentId)) byParent.set(parentId, {});
    const bucket = byParent.get(parentId);
    bucket[kind] = (bucket[kind] || 0) + 1;
  }
  return byParent;
}

function createsCycle(byId, candidateParentId, nodeId) {
  let cursor = candidateParentId;
  const seen = new Set([nodeId]);
  while (cursor && byId.has(cursor)) {
    if (seen.has(cursor)) return true;
    seen.add(cursor);
    cursor = byId.get(cursor).parent;
  }
  return false;
}

// Pure tree builder: orders outline nodes by document order, wires parent links, and attaches stats.
export function buildOutlineTree(mapview) {
  const rawNodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const rawStats = Array.isArray(mapview?.stats) ? mapview.stats : [];
  const statsByTag = new Map();
  for (const stat of rawStats) {
    if (stat?.section_tag) statsByTag.set(String(stat.section_tag), stat);
  }
  const fallbackCounts = deriveChildCounts(rawNodes);

  const outlineEntries = rawNodes
    .map((node, index) => ({ node, index }))
    .filter(({ node }) => OUTLINE_KINDS.includes(String(node?.kind || '').toLowerCase()))
    .sort((a, b) => (a.node.order ?? a.index) - (b.node.order ?? b.index));

  const byId = new Map();
  const ordered = outlineEntries.map(({ node, index }) => {
    const id = String(node.id ?? node.tag ?? `outline-${index}`);
    const stat = statsByTag.get(id) || null;
    const counts = stat?.counts && typeof stat.counts === 'object'
      ? { ...stat.counts }
      : { ...(fallbackCounts.get(id) || {}) };
    const item = {
      id,
      kind: String(node.kind || '').toLowerCase(),
      order: node.order ?? index,
      number: node.number ?? null,
      title: node.title ?? null,
      label: node.label ?? '',
      parent: node.parent ? String(node.parent) : null,
      words: stat ? Number(stat.words || 0) : Number(node.words || 0),
      read_minutes: stat ? Number(stat.read_minutes || 0) : Number(node.read_minutes || 0),
      counts,
      result_count: countResultKinds(counts),
      node,
      children: [],
    };
    byId.set(id, item);
    return item;
  });

  const roots = [];
  for (const item of ordered) {
    if (item.parent && byId.has(item.parent) && !createsCycle(byId, item.parent, item.id)) {
      byId.get(item.parent).children.push(item);
    } else {
      roots.push(item);
    }
  }
  return roots;
}

// Quality banner (overview 3.2 row 25: solid for tex-1/html-1 full or partial, dashed for weak/unknown).
export function createQualityBanner(mapview) {
  const quality = String(mapview?.quality || '').trim().toLowerCase();
  const builder = String(mapview?.builder || '').trim().toLowerCase();
  const qualityText = String(mapview?.quality_line || '').trim() || QUALITY_FALLBACK_LABEL;
  const fallbackNote = String(mapview?.fallback_note || '').trim();

  const icon = quality === 'full' ? 'code' : 'warn';
  const tone = quality === 'full' ? 'ok' : (quality === 'partial' ? 'warn' : 'muted');
  const structuredBuilder = builder === 'tex-1' || builder === 'html-1';
  const border = structuredBuilder && quality !== 'weak' ? BORDER_SOLID : BORDER_DASHED;

  const wrap = el('div', 'inv-quality-line');
  wrap.setAttribute('role', 'status');
  if (quality) wrap.setAttribute('data-quality', quality);
  if (builder) wrap.setAttribute('data-builder', builder);
  wrap.setAttribute('data-border', border);
  wrap.appendChild(createBadge({ text: qualityText, icon, tone, border }));
  if (fallbackNote) wrap.appendChild(el('p', 'inv-quality-fallback', fallbackNote));
  return wrap;
}

function formatOutlineItemText(item) {
  const num = item.number != null ? String(item.number).trim() : '';
  const title = item.title != null ? String(item.title).trim() : '';
  if (num && title) return `${num} ${title}`;
  if (title) return title;
  const label = String(item.label || '').trim();
  if (label) return label;
  if (item.kind === 'front_matter') return 'Front matter';
  if (item.kind === 'bibliography') return 'Bibliography';
  return num ? `Section ${num}` : item.id;
}

function resolveTotalMinutes(mapview, roots) {
  const stats = Array.isArray(mapview?.stats) ? mapview.stats : [];
  const whole = stats.find((s) => s && (s.section_tag == null || s.section_tag === ''));
  if (whole && Number(whole.read_minutes) > 0) return Number(whole.read_minutes);
  if (Number(mapview?.total_read_minutes) > 0) return Number(mapview.total_read_minutes);
  if (Number(mapview?.read_minutes) > 0) return Number(mapview.read_minutes);
  return roots.reduce((acc, r) => acc + (Number(r.read_minutes) > 0 ? Number(r.read_minutes) : 0), 0);
}

export function renderInvestigationOutline(host, mapview, opts = {}) {
  const collapsed = new Set(opts.collapsedTags || []);
  const itemButtons = new Map();
  let selectedTag = opts.selectedTag ?? null;

  const syncSelection = () => {
    for (const [id, btn] of itemButtons.entries()) {
      const active = selectedTag != null && id === selectedTag;
      if (active) {
        btn.classList.add('is-selected');
        btn.setAttribute('aria-current', 'location');
      } else {
        btn.classList.remove('is-selected');
        btn.removeAttribute('aria-current');
      }
    }
  };

  const updateToggle = (toggle, childList, isCollapsed) => {
    toggle.setAttribute('aria-expanded', isCollapsed ? 'false' : 'true');
    toggle.setAttribute('aria-label', isCollapsed ? 'Expand section' : 'Collapse section');
    toggle.textContent = isCollapsed ? '▸' : '▾';
    childList.hidden = isCollapsed;
  };

  const renderList = (items, depth) => {
    const ul = el('ul', 'inv-outline-list');
    ul.setAttribute('data-depth', String(depth));
    for (const item of items) {
      const li = el('li', 'inv-outline-item');
      li.setAttribute('data-tag', item.id);
      li.setAttribute('data-kind', item.kind);
      const row = el('div', 'inv-outline-row');

      let childList = null;
      if (item.children.length > 0) {
        const toggle = el('button', 'inv-outline-toggle');
        toggle.type = 'button';
        childList = renderList(item.children, depth + 1);
        updateToggle(toggle, childList, collapsed.has(item.id));
        toggle.addEventListener('click', (ev) => {
          ev?.stopPropagation?.();
          if (collapsed.has(item.id)) collapsed.delete(item.id);
          else collapsed.add(item.id);
          updateToggle(toggle, childList, collapsed.has(item.id));
        });
        row.appendChild(toggle);
      }

      const btn = el('button', 'inv-outline-btn');
      btn.type = 'button';
      btn.setAttribute('data-tag', item.id);
      btn.appendChild(el('span', 'inv-outline-title', formatOutlineItemText(item)));

      const timeText = formatReadMinutes(item.read_minutes);
      if (timeText) btn.appendChild(el('span', 'inv-outline-time', timeText));
      if (item.result_count > 0) {
        const suffix = item.result_count === 1 ? 'result' : 'results';
        btn.appendChild(el('span', 'inv-outline-count', `${item.result_count} ${suffix}`));
      }

      btn.addEventListener('click', () => {
        selectedTag = item.id;
        syncSelection();
        if (typeof opts.onSelectNode === 'function') opts.onSelectNode(item.id);
      });

      itemButtons.set(item.id, btn);
      row.appendChild(btn);
      li.appendChild(row);
      if (childList) li.appendChild(childList);
      ul.appendChild(li);
    }
    return ul;
  };

  const tree = buildOutlineTree(mapview);
  const nav = el('nav', 'inv-outline');
  nav.setAttribute('aria-label', OUTLINE_NAV_LABEL);

  const header = el('div', 'inv-outline-header');
  header.appendChild(el('h2', 'inv-outline-heading', OUTLINE_HEADING));
  const totalTimeText = formatReadMinutes(resolveTotalMinutes(mapview, tree));
  if (totalTimeText) header.appendChild(el('span', 'inv-outline-total-time', totalTimeText));
  nav.appendChild(header);
  nav.appendChild(tree.length > 0 ? renderList(tree, 0) : el('p', 'inv-outline-empty', OUTLINE_EMPTY_NOTE));

  host.replaceChildren(createQualityBanner(mapview), nav);
  syncSelection();

  return {
    setSelected(tag) { selectedTag = tag ?? null; syncSelection(); },
    getSelected() { return selectedTag; },
    destroy() { itemButtons.clear(); host.replaceChildren(); },
  };
}
