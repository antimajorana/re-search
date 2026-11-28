// Explore candidate list with term highlights and "Where I looked" skeleton panel (package X4).
// Zero innerHTML writes: all titles, authors, abstracts, and errors use textContent / createTextNode.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any rendered label.
import {
  BADGE_IN_LIBRARY,
  BADGE_NOT_READ,
  BADGE_OWNED_LIBRARY,
  TRIAGE_BASIS_LABEL,
  highlightTermsSegments,
} from './idea_state.js';
import { createApprovalCard } from './approval_card.js';
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
    case 'disabled':
      return { icon: 'pause', tone: 'muted', text: 'disabled' };
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
  if (s === 'disabled') return 'disabled';
  return s || 'planned';
}

const TRIAGE_GROUPS = [
  { id: 'core', label: 'Core' },
  { id: 'related', label: 'Related' },
  { id: 'background', label: 'Background' },
  { id: 'unrelated', label: 'Unrelated', collapsed: true },
];

function renderSingleCandidateCard(cand, terms, triageCtx) {
  const card = el('article', 'idea-candidate-card');
  const key = String(cand.key || '');
  card.setAttribute('data-candidate-key', key);

  const tItem = triageCtx?.byKey?.get(key) || null;
  if (tItem) card.setAttribute('data-triage-relevance', String(tItem.relevance || ''));

  const rowHead = el('div', 'idea-candidate-head');
  const titleEl = el('h4', 'idea-candidate-title');
  appendHighlighted(titleEl, cand.title || key || 'Untitled paper', terms);

  const badges = el('div', 'idea-candidate-badges');
  if (tItem) {
    const rel = String(tItem.relevance || 'background');
    badges.append(
      createBadge({
        text: `${rel} · ${TRIAGE_BASIS_LABEL}`,
        icon: 'abstract',
        tone: rel === 'core' ? 'ok' : rel === 'related' ? 'info' : 'muted',
        border: 'dashed',
      }),
    );
  }
  const readText = String(cand.read_badge || BADGE_NOT_READ);
  badges.append(
    createBadge({
      text: readText,
      icon: readText === BADGE_NOT_READ ? 'abstract' : 'quote',
      tone: readText === BADGE_NOT_READ ? 'muted' : 'ok',
      border: readText === BADGE_NOT_READ ? 'dashed' : 'solid',
    }),
  );
  if (cand.in_library) badges.append(createBadge({ text: BADGE_IN_LIBRARY, icon: 'pin', tone: 'ok' }));
  if (cand.owned) badges.append(createBadge({ text: BADGE_OWNED_LIBRARY, icon: 'code', tone: 'info' }));
  if (cand.user_seed) badges.append(createBadge({ text: 'seed', icon: 'user', tone: 'neutral' }));
  rowHead.append(titleEl, badges);

  const authors = Array.isArray(cand.authors) ? cand.authors.slice(0, 8).join(', ') : '';
  const metaParts = [];
  if (authors) metaParts.push(authors);
  if (cand.year) metaParts.push(String(cand.year));
  if (key) metaParts.push(key);
  if (cand.citation_count != null) metaParts.push(`${cand.citation_count} citations`);
  card.append(rowHead, el('div', 'idea-candidate-meta', metaParts.join(' · ')));

  if (tItem) {
    if (tItem.why) {
      card.append(el('p', 'idea-card-meta', `Model judges (from abstract): ${tItem.why}`));
    }
    if (tItem.evidence_id && tItem.abstract_quote) {
      const qBox = el('div', 'idea-preview-box');
      qBox.setAttribute('data-evidence-id', String(tItem.evidence_id));
      qBox.append(
        createBadge({
          text: `matched · abstract only · ${tItem.evidence_id}`,
          icon: 'abstract',
          tone: 'info',
          border: 'dashed',
        }),
        el('blockquote', 'idea-candidate-abstract', `"${tItem.abstract_quote}"`),
      );
      card.append(qBox);
    }
    if (triageCtx?.onToggleReading) {
      const rlLabel = el('label', 'idea-offline-label');
      const cb = el('input');
      cb.type = 'checkbox';
      cb.setAttribute('data-action', 'toggle-reading-list');
      cb.setAttribute('data-candidate-key', key);
      const quickOnly = Boolean(triageCtx.quickAbstractOnly);
      const atCap =
        !triageCtx.readingSet.has(key) &&
        triageCtx.maxPapers > 0 &&
        triageCtx.readingSet.size >= triageCtx.maxPapers;
      cb.checked = !quickOnly && triageCtx.readingSet.has(key);
      cb.disabled = quickOnly || atCap;
      triageCtx.checkboxes.push({ cb, key });
      cb.addEventListener('change', () => triageCtx.onToggleReading(key, Boolean(cb.checked)));
      rlLabel.append(
        cb,
        el('span', '', quickOnly ? 'Quick preset — abstracts only (0 papers read)' : 'Read full paper'),
      );
      card.append(rlLabel);
    }
  }

  const abstractStr = String(cand.abstract_text || cand.abstract || '').trim();
  if (abstractStr) {
    const absEl = el('p', 'idea-candidate-abstract');
    appendHighlighted(absEl, abstractStr, terms);
    card.append(absEl);
  }

  if (Array.isArray(cand.found_by) && cand.found_by.length > 0) {
    const qRow = el('div', 'idea-chip-row');
    for (const qid of cand.found_by) qRow.append(el('span', 'idea-chip', String(qid)));
    card.append(qRow);
  }
  return card;
}

/**
 * Render ONLY the candidate cards container (flat before Triage, grouped by
 * `core` / `related` / `background` / collapsed `unrelated` after Triage).
 */
export function renderCandidateCards(candidates, terms = [], opts = {}) {
  const list = Array.isArray(candidates) ? candidates : [];
  const wrap = el('div', 'idea-candidate-cards');
  wrap.setAttribute('data-idea-candidate-cards', 'true');

  if (list.length === 0) {
    wrap.append(el('p', 'idea-card-meta', 'No candidates match the current filter.'));
    return wrap;
  }

  const triage = opts?.triage && typeof opts.triage === 'object' ? opts.triage : null;
  const triageItems = Array.isArray(triage?.items) ? triage.items : [];
  if (triageItems.length === 0) {
    for (const cand of list) wrap.append(renderSingleCandidateCard(cand, terms, null));
    return wrap;
  }

  const byKey = new Map();
  for (const item of triageItems) {
    if (item?.key) byKey.set(String(item.key), item);
  }
  const maxPapers = Math.max(0, Number(opts?.maxPapers ?? 6));
  const quickAbstractOnly = Boolean(triage?.quick_abstract_only) || maxPapers === 0;
  const readingSet = new Set(
    quickAbstractOnly
      ? []
      : (Array.isArray(triage?.reading_list) ? triage.reading_list : []).map(String),
  );
  const onSaveReadingList =
    typeof opts?.onSaveReadingList === 'function' ? opts.onSaveReadingList : null;
  const checkboxes = [];
  const syncCheckboxes = () => {
    for (const entry of checkboxes) {
      const checked = !quickAbstractOnly && readingSet.has(entry.key);
      entry.cb.checked = checked;
      entry.cb.disabled =
        quickAbstractOnly || (!checked && maxPapers > 0 && readingSet.size >= maxPapers);
    }
  };
  const triageCtx = {
    byKey,
    readingSet,
    maxPapers,
    quickAbstractOnly,
    checkboxes,
    onToggleReading: onSaveReadingList
      ? (key, checked) => {
          if (quickAbstractOnly) return;
          if (checked) {
            if (maxPapers > 0 && readingSet.size >= maxPapers) {
              syncCheckboxes();
              return;
            }
            readingSet.add(key);
          } else {
            readingSet.delete(key);
          }
          syncCheckboxes();
          onSaveReadingList([...readingSet]);
        }
      : null,
  };

  for (const grp of TRIAGE_GROUPS) {
    const members = list.filter((c) => byKey.get(String(c.key || ''))?.relevance === grp.id);
    if (members.length === 0) continue;
    if (grp.collapsed) {
      const details = el('details', 'idea-triage-group');
      details.open = false;
      details.setAttribute('data-triage-group', grp.id);
      details.append(el('summary', 'idea-field-label', `${grp.label} (${members.length}) — from abstract`));
      for (const cand of members) details.append(renderSingleCandidateCard(cand, terms, triageCtx));
      wrap.append(details);
    } else {
      const box = el('div', 'idea-triage-group');
      box.setAttribute('data-triage-group', grp.id);
      box.append(el('h4', 'idea-field-label', `${grp.label} (${members.length}) — from abstract`));
      for (const cand of members) box.append(renderSingleCandidateCard(cand, terms, triageCtx));
      wrap.append(box);
    }
  }

  const untriaged = list.filter((c) => !byKey.has(String(c.key || '')));
  if (untriaged.length > 0) {
    const box = el('div', 'idea-triage-group');
    box.setAttribute('data-triage-group', 'untriaged');
    box.append(el('h4', 'idea-field-label', `Not yet triaged (${untriaged.length})`));
    for (const cand of untriaged) box.append(renderSingleCandidateCard(cand, terms, triageCtx));
    wrap.append(box);
  }
  return wrap;
}

/**
 * Render the Explore candidate list with filter toolbar, triage grouping, reading-list ticks,
 * and optional `"Pause before reading"` toggle (`explore.md` 3 & decision 2).
 */
export function renderCandidateList(candidates, opts = {}) {
  const list = Array.isArray(candidates) ? candidates : [];
  const terms = Array.isArray(opts.highlightTerms) ? opts.highlightTerms : [];
  const filters = opts.filters || {};
  const onFilterChange = typeof opts.onFilterChange === 'function' ? opts.onFilterChange : null;
  const onTogglePause =
    typeof opts.onTogglePauseBeforeReading === 'function' ? opts.onTogglePauseBeforeReading : null;
  const triage = opts.triage || null;
  const hasTriage = Array.isArray(triage?.items) && triage.items.length > 0;

  const section = el('section', 'idea-candidate-list');
  section.setAttribute('data-idea-section', 'candidates');
  section.setAttribute('aria-label', 'Candidate papers');

  const head = el('div', 'idea-card-head');
  const countEl = el('h3', 'idea-card-title', `Candidates (${list.length})`);
  countEl.setAttribute('data-idea-candidate-count', 'true');
  const subtitle = hasTriage
    ? 'Grouped from abstract; reading list is editable'
    : 'Ranked by cross-query fusion; abstracts only until read';
  head.append(countEl, el('span', 'idea-card-meta', subtitle));
  section.append(head);

  if (onFilterChange || onTogglePause) {
    const bar = el('div', 'idea-controls-row');
    if (onFilterChange) {
      const searchInput = el('input', 'text-input');
      searchInput.type = 'search';
      searchInput.placeholder = 'Filter candidates...';
      searchInput.value = String(filters.searchText || '');
      searchInput.setAttribute('aria-label', 'Filter candidates');
      searchInput.setAttribute('data-idea-filter-input', 'true');
      searchInput.addEventListener('input', () => onFilterChange({ searchText: searchInput.value }));

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
    }
    if (onTogglePause) {
      const pauseLabel = el('label', 'idea-offline-label');
      const pauseCb = el('input');
      pauseCb.type = 'checkbox';
      pauseCb.checked = Boolean(opts.pauseBeforeReading);
      pauseCb.setAttribute('data-action', 'toggle-pause-before-reading');
      pauseCb.addEventListener('change', () => onTogglePause(Boolean(pauseCb.checked)));
      pauseLabel.append(pauseCb, el('span', '', 'Pause before reading'));
      bar.append(pauseLabel);
    }
    section.append(bar);
  }

  section.append(
    renderCandidateCards(list, terms, {
      triage,
      maxPapers: opts.maxPapers,
      onSaveReadingList: opts.onSaveReadingList,
    }),
  );
  return section;
}

/**
 * Render the planned search batch and shared read-only `approval_card.js` panel (package X7).
 * Shows every planned query, target services, and session network grant without auto-approving.
 */
export function renderSearchPlanApproval(planApproval, opts = {}) {
  if (!planApproval || typeof planApproval !== 'object') return null;
  const queries = Array.isArray(planApproval.queries) ? planApproval.queries : [];
  const grants = Array.isArray(planApproval.grants) ? planApproval.grants : [];
  if (queries.length === 0 && grants.length === 0) return null;

  const panel = el('section', 'idea-where-panel');
  panel.setAttribute('data-idea-section', 'plan-approval');
  panel.setAttribute('aria-label', 'Search plan approval');

  const services = Array.isArray(planApproval.services) ? planApproval.services : [];
  const round = Number(planApproval.round || 1);
  panel.append(
    el('h3', 'idea-card-title', `Search plan (round ${round})`),
    el(
      'p',
      'idea-card-meta',
      services.length > 0
        ? `Services: ${services.join(', ')} · ${queries.length} planned quer${queries.length === 1 ? 'y' : 'ies'}`
        : `${queries.length} planned quer${queries.length === 1 ? 'y' : 'ies'}`,
    ),
  );

  if (queries.length > 0) {
    const qList = el('div', 'idea-query-list');
    for (const q of queries) {
      const row = el('div', 'idea-query-chip');
      row.append(
        el('span', 'idea-query-provider', `${q.qid || ''} ${q.provider || ''}`.trim()),
        el('span', 'idea-query-text', q.q || ''),
      );
      qList.append(row);
    }
    panel.append(qList);
  }

  if (grants.length > 0) {
    const cardHost = el('div', 'idea-approval-host');
    createApprovalCard(cardHost, {
      title: 'Approvals for this search batch',
      grants,
      preTurnSupported: true,
      onAllowGrant: typeof opts.onAllowGrant === 'function' ? opts.onAllowGrant : null,
      onDenyGrant: typeof opts.onDenyGrant === 'function' ? opts.onDenyGrant : null,
    });
    panel.append(cardHost);
  }

  return panel;
}

/**
 * Render the live `"Where I looked"` panel (`explore.md` 3 & 4.1 Coverage, package X7).
 * Shows per-service F12 status, `"Retry <service>"` buttons, library summary, and known-paper recall.
 */
export function renderWhereILooked(whereILooked, opts = {}) {
  const data = whereILooked && typeof whereILooked === 'object' ? whereILooked : {};
  const onRetryService = typeof opts.onRetryService === 'function' ? opts.onRetryService : null;
  const panel = el('section', 'idea-where-panel');
  panel.setAttribute('data-idea-section', 'where-i-looked');
  panel.setAttribute('aria-label', 'Where I looked');

  panel.append(el('h3', 'idea-card-title', 'Where I looked'));

  if (data.scope_summary) {
    panel.append(el('p', 'idea-card-meta', String(data.scope_summary)));
  }

  const providers = Array.isArray(data.providers) ? data.providers : [];
  if (providers.length > 0) {
    const grid = el('div', 'idea-provider-grid');
    for (const p of providers) {
      const cell = el('div', 'idea-provider-cell');
      const provName = String(p.name || '');
      const provLabel = String(p.label || provName);
      cell.setAttribute('data-provider', provName);
      const badgeSpec = providerStateTone(p.state);
      cell.append(
        el('strong', '', provLabel),
        createBadge(badgeSpec),
        el(
          'span',
          'idea-card-meta',
          p.summary
            ? String(p.summary)
            : `${Number(p.queries_ok || 0)} ok · ${Number(p.queries_failed || 0)} failed · ${Number(p.hits || 0)} hits`,
        ),
      );
      if (p.retryable && onRetryService) {
        const retryBtn = el('button', 'pill-btn sm', `Retry ${provLabel}`);
        retryBtn.type = 'button';
        retryBtn.setAttribute('data-action', 'retry-service');
        retryBtn.setAttribute('data-provider', provName);
        retryBtn.addEventListener('click', () => onRetryService(provName));
        cell.append(retryBtn);
      }
      grid.append(cell);
    }
    panel.append(grid);
  }

  const lib = data.library || { hits: 0, owned_hits: 0 };
  const libText = lib.summary
    ? `${lib.summary} (${Number(lib.owned_hits || 0)} owned locally) — searched locally, not counted as a literature service`
    : `Your library: ${Number(lib.hits || 0)} matched (${Number(lib.owned_hits || 0)} owned locally) — searched locally, not counted as a literature service`;
  panel.append(el('div', 'idea-card-meta', libText));

  const recall = data.known_paper_recall;
  if (recall && typeof recall === 'object') {
    const total = Array.isArray(recall.given) ? recall.given.length : Number(recall.total_seeds || 0);
    if (total > 0) {
      const found = Array.isArray(recall.found_by_search)
        ? recall.found_by_search.length
        : Number(recall.found_seeds || 0);
      const missed = Array.isArray(recall.missed) ? recall.missed : [];
      const suffix = missed.length > 0 ? ` (missed: ${missed.join(', ')})` : '';
      const recallEl = el(
        'div',
        'idea-card-meta',
        `Known-paper recall: ${found} of ${total} seed paper(s) found by search${suffix}`,
      );
      recallEl.setAttribute('data-idea-recall', `${found}/${total}`);
      panel.append(recallEl);
    }
  }

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
