// Explore candidate list with term highlights and "Where I looked" skeleton panel (package X4).
// Zero innerHTML writes: all titles, authors, abstracts, and errors use textContent / createTextNode.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any rendered label.
import {
  BADGE_IN_LIBRARY,
  BADGE_NOT_READ,
  BADGE_OWNED_LIBRARY,
  highlightTermsSegments,
} from './idea_state.js';
import { createBadge } from './ui_badge.js';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function appendHighlighted(targetEl, rawText, terms) {
  const segments = highlightTermsSegments(rawText, terms);
  for (const seg of segments) {
    if (seg.match) {
      const mark = document.createElement('mark');
      mark.className = 'idea-term-mark';
      mark.textContent = seg.text;
      targetEl.appendChild(mark);
    } else {
      const span = document.createElement('span');
      span.textContent = seg.text;
      targetEl.appendChild(span);
    }
  }
}

function providerStateTone(state) {
  switch (String(state || '').toLowerCase()) {
    case 'ok':
      return { icon: 'check', tone: 'ok', text: 'ok' };
    case 'partial':
      return { icon: 'warn', tone: 'warn', text: 'partial' };
    case 'rate_limited':
      return { icon: 'warn', tone: 'err', text: 'rate limited' };
    case 'failed':
      return { icon: 'error', tone: 'err', text: 'failed' };
    case 'offline':
      return { icon: 'pause', tone: 'muted', text: 'offline' };
    case 'not_configured':
      return { icon: 'info', tone: 'muted', text: 'not configured' };
    case 'skipped_budget':
      return { icon: 'pause', tone: 'warn', text: 'skipped (budget)' };
    default:
      return { icon: 'info', tone: 'muted', text: 'not attempted' };
  }
}

function formatQueryStatus(raw) {
  const s = String(raw || '').toLowerCase();
  if (s === 'rate_limited') return 'rate limited';
  if (s === 'skipped_budget') return 'skipped (budget)';
  if (s === 'not_configured') return 'not configured';
  return s || 'planned';
}

/**
 * Render ONLY the candidate cards container (so filter controls can update cards in-place
 * without destroying focus on the search input).
 */
export function renderCandidateCards(candidates, terms = []) {
  const list = Array.isArray(candidates) ? candidates : [];
  const wrap = el('div', 'idea-candidate-cards');
  wrap.setAttribute('data-idea-candidate-cards', 'true');

  if (list.length === 0) {
    wrap.append(el('p', 'idea-card-meta', 'No candidates match the current filter.'));
    return wrap;
  }

  for (const cand of list) {
    const card = el('article', 'idea-candidate-card');
    card.setAttribute('data-candidate-key', String(cand.key || ''));

    const rowHead = el('div', 'idea-candidate-head');
    const titleEl = el('h4', 'idea-candidate-title');
    appendHighlighted(titleEl, cand.title || cand.key || 'Untitled paper', terms);

    const badges = el('div', 'idea-candidate-badges');
    const readText = String(cand.read_badge || BADGE_NOT_READ);
    badges.append(
      createBadge({
        text: readText,
        icon: readText === BADGE_NOT_READ ? 'abstract' : 'quote',
        tone: readText === BADGE_NOT_READ ? 'muted' : 'ok',
        border: readText === BADGE_NOT_READ ? 'dashed' : 'solid',
      }),
    );
    if (cand.in_library) {
      badges.append(createBadge({ text: BADGE_IN_LIBRARY, icon: 'pin', tone: 'ok' }));
    }
    if (cand.owned) {
      badges.append(createBadge({ text: BADGE_OWNED_LIBRARY, icon: 'code', tone: 'info' }));
    }
    if (cand.user_seed) {
      badges.append(createBadge({ text: 'seed', icon: 'user', tone: 'neutral' }));
    }
    rowHead.append(titleEl, badges);

    const authors = Array.isArray(cand.authors) ? cand.authors.slice(0, 8).join(', ') : '';
    const metaParts = [];
    if (authors) metaParts.push(authors);
    if (cand.year) metaParts.push(String(cand.year));
    if (cand.key) metaParts.push(String(cand.key));
    if (cand.citation_count != null) metaParts.push(`${cand.citation_count} citations`);
    const metaEl = el('div', 'idea-candidate-meta', metaParts.join(' · '));

    card.append(rowHead, metaEl);

    const abstractStr = String(cand.abstract_text || cand.abstract || '').trim();
    if (abstractStr) {
      const absEl = el('p', 'idea-candidate-abstract');
      appendHighlighted(absEl, abstractStr, terms);
      card.append(absEl);
    }

    if (Array.isArray(cand.found_by) && cand.found_by.length > 0) {
      const qRow = el('div', 'idea-chip-row');
      for (const qid of cand.found_by) {
        qRow.append(el('span', 'idea-chip', String(qid)));
      }
      card.append(qRow);
    }
    wrap.append(card);
  }
  return wrap;
}

/**
 * Render the Explore candidate list with filter toolbar, term highlights, and honest badges.
 */
export function renderCandidateList(candidates, opts = {}) {
  const list = Array.isArray(candidates) ? candidates : [];
  const terms = Array.isArray(opts.highlightTerms) ? opts.highlightTerms : [];
  const filters = opts.filters || {};
  const onFilterChange = typeof opts.onFilterChange === 'function' ? opts.onFilterChange : null;

  const section = el('section', 'idea-candidate-list');
  section.setAttribute('data-idea-section', 'candidates');
  section.setAttribute('aria-label', 'Candidate papers');

  const head = el('div', 'idea-card-head');
  const countEl = el('h3', 'idea-card-title', `Candidates (${list.length})`);
  countEl.setAttribute('data-idea-candidate-count', 'true');
  head.append(countEl, el('span', 'idea-card-meta', 'Ranked by cross-query fusion; abstracts only until read'));
  section.append(head);

  if (onFilterChange) {
    const bar = el('div', 'idea-controls-row');
    const searchInput = el('input', 'text-input');
    searchInput.type = 'search';
    searchInput.placeholder = 'Filter candidates...';
    searchInput.value = String(filters.searchText || '');
    searchInput.setAttribute('aria-label', 'Filter candidates');
    searchInput.setAttribute('data-idea-filter-input', 'true');
    searchInput.addEventListener('input', () => {
      onFilterChange({ searchText: searchInput.value });
    });

    const libLabel = el('label', 'idea-offline-label');
    const libCb = el('input');
    libCb.type = 'checkbox';
    libCb.checked = Boolean(filters.libraryOnly);
    libCb.addEventListener('change', () => onFilterChange({ libraryOnly: Boolean(libCb.checked) }));
    libLabel.append(libCb, el('span', '', 'In your library'));

    const ownedLabel = el('label', 'idea-offline-label');
    const ownedCb = el('input');
    ownedCb.type = 'checkbox';
    ownedCb.checked = Boolean(filters.ownedOnly);
    ownedCb.addEventListener('change', () => onFilterChange({ ownedOnly: Boolean(ownedCb.checked) }));
    ownedLabel.append(ownedCb, el('span', '', 'Owned locally'));

    bar.append(searchInput, libLabel, ownedLabel);
    section.append(bar);
  }

  section.append(renderCandidateCards(list, terms));
  return section;
}

/**
 * Render the `"Where I looked"` skeleton panel (`explore.md` 3 & 4.1 Coverage).
 */
export function renderWhereILooked(whereILooked) {
  const data = whereILooked && typeof whereILooked === 'object' ? whereILooked : {};
  const panel = el('section', 'idea-where-panel');
  panel.setAttribute('data-idea-section', 'where-i-looked');
  panel.setAttribute('aria-label', 'Where I looked');

  panel.append(el('h3', 'idea-card-title', 'Where I looked'));

  const providers = Array.isArray(data.providers) ? data.providers : [];
  if (providers.length > 0) {
    const grid = el('div', 'idea-provider-grid');
    for (const p of providers) {
      const cell = el('div', 'idea-provider-cell');
      cell.setAttribute('data-provider', String(p.name || ''));
      const badgeSpec = providerStateTone(p.state);
      cell.append(
        el('strong', '', p.label || p.name || ''),
        createBadge(badgeSpec),
        el(
          'span',
          'idea-card-meta',
          `${Number(p.queries_ok || 0)} ok · ${Number(p.queries_failed || 0)} failed · ${Number(p.hits || 0)} hits`,
        ),
      );
      grid.append(cell);
    }
    panel.append(grid);
  }

  const lib = data.library || { hits: 0, owned_hits: 0 };
  panel.append(
    el(
      'div',
      'idea-card-meta',
      `Your library: ${Number(lib.hits || 0)} matched (${Number(lib.owned_hits || 0)} owned locally) — searched locally, not counted as a literature service`,
    ),
  );

  const queries = Array.isArray(data.queries) ? data.queries : [];
  if (queries.length > 0) {
    const qList = el('div', 'idea-query-list');
    for (const q of queries) {
      const row = el('div', 'idea-query-chip');
      row.append(
        el('span', 'idea-query-provider', `${q.qid || ''} ${q.provider || ''}`.trim()),
        el('span', 'idea-query-text', q.q || ''),
        el('span', 'idea-card-meta', `${formatQueryStatus(q.status)} (${Number(q.count || 0)})`),
      );
      qList.append(row);
    }
    panel.append(qList);
  }

  const notSearched = Array.isArray(data.not_searched) ? data.not_searched : [];
  if (notSearched.length > 0) {
    panel.append(el('span', 'idea-field-label', 'Not searched'));
    const ul = el('ul', 'idea-not-searched');
    for (const item of notSearched) {
      ul.append(el('li', '', String(item)));
    }
    panel.append(ul);
  }

  return panel;
}
