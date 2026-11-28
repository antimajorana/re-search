// Explore citation-network panel, "Cited by your core papers but unread",
// "Who cited this since" control (with open-question caveat), and Thorough Round 2 progress (X16).
// Zero innerHTML writes: every title, author, key, and status uses textContent / createTextNode.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any rendered label.
import { createBadge } from './ui_badge.js';

// Named constants for owner-accepted on-screen wording (`explore.md` 3, 12 risk 12; `00-overview.md` 3.2/3.3).
export const UNREAD_CORE_HEADING = 'Cited by your core papers but unread';
export const CITED_SINCE_BUTTON_LABEL = 'Who cited this since';
export const CITED_SINCE_ZERO_CALLS_HINT = '0 model calls';
export const CITED_SINCE_NOT_ANSWERED_NOTE =
  '"Cited since" does not mean "answered": follow-up citations must be read to check whether an open question was resolved.';
export const SEARCH_MORE_NODE_LABEL = 'Search more about this node';
export const ROUND2_HEADING = 'Thorough Round 2 self-check';
export const ROUND2_TRIGGER_LABEL = 'Run Thorough Round 2';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function round2StatusBadge(status) {
  switch (String(status || 'idle').toLowerCase()) {
    case 'running':
      return { text: 'Running', icon: 'info', tone: 'info', border: 'dashed' };
    case 'stopped_at_cap':
      return { text: 'Paused: budget cap reached', icon: 'pause', tone: 'warn', border: 'solid' };
    case 'paused':
      return { text: 'Paused', icon: 'pause', tone: 'warn', border: 'solid' };
    case 'done':
      return { text: 'Done', icon: 'check', tone: 'ok', border: 'solid' };
    case 'failed':
      return { text: 'Failed', icon: 'error', tone: 'err', border: 'dashed' };
    default:
      return { text: 'Idle', icon: 'info', tone: 'muted', border: 'dashed' };
  }
}

function parseValidYear(raw) {
  if (raw === null || raw === undefined) return null;
  const s = String(raw).trim();
  if (!s) return null;
  const n = Number(s);
  return Number.isInteger(n) ? n : null;
}

/**
 * Render a `"Who cited this since"` action bar (0 model calls) for a paper or map node.
 * On open-question targets, ALWAYS renders the `"cited since" != "answered"` note (`explore.md` 12 risk 12).
 */
export function renderCitedSinceControl(target, opts = {}) {
  if (!target || typeof target !== 'object') return null;
  const key = String(target.paper_key || target.key || '').trim();
  const nodeId = target.node_id ? String(target.node_id).trim() : '';
  if (!key && !nodeId) return null;

  const isOpenQ = Boolean(target.isOpenQuestion || target.is_open_question || target.kind === 'open_question');
  const sinceYear = parseValidYear(target.since_year ?? target.stated_year ?? target.year);
  const onCitedSince = typeof opts.onCitedSince === 'function' ? opts.onCitedSince : null;

  const box = el('div', 'idea-cited-since-box');
  box.setAttribute('data-cited-since-target', key || nodeId);

  const row = el('div', 'idea-chip-row');
  if (onCitedSince) {
    const btn = el('button', 'pill-btn sm', CITED_SINCE_BUTTON_LABEL);
    btn.type = 'button';
    btn.setAttribute('data-action', 'cited-since');
    if (key) btn.setAttribute('data-paper-key', key);
    if (nodeId) btn.setAttribute('data-node-id', nodeId);
    btn.addEventListener('click', () =>
      onCitedSince({
        paper_key: key || undefined,
        node_id: nodeId || undefined,
        since_year: sinceYear != null ? sinceYear : undefined,
        is_open_question: isOpenQ,
      }),
    );
    row.append(btn);
  }
  row.append(
    createBadge({
      text: CITED_SINCE_ZERO_CALLS_HINT,
      icon: 'code',
      tone: 'muted',
      border: 'solid',
    }),
  );
  if (sinceYear != null) {
    row.append(el('span', 'idea-card-meta', `since ${sinceYear}`));
  }
  box.append(row);

  if (isOpenQ) {
    const note = el('p', 'idea-card-meta idea-open-question-caveat', CITED_SINCE_NOT_ANSWERED_NOTE);
    note.setAttribute('data-cited-since-caveat', 'true');
    box.append(note);
  }
  return box;
}

/**
 * Render `"Cited by your core papers but unread"` candidates ranked by core citations & in-network rank.
 */
export function renderUnreadCoreCitations(unreadList, opts = {}) {
  const list = Array.isArray(unreadList) ? unreadList : [];
  if (list.length === 0) return null;

  const section = el('div', 'idea-unread-core-section');
  section.setAttribute('data-idea-section', 'unread-core-citations');
  section.append(
    el('h4', 'idea-field-label', `${UNREAD_CORE_HEADING} (${list.length})`),
  );

  const onCitedSince = typeof opts.onCitedSince === 'function' ? opts.onCitedSince : null;
  for (const item of list) {
    const key = String(item.key || '');
    const card = el('article', 'idea-candidate-card');
    card.setAttribute('data-unread-core-key', key);

    const head = el('div', 'idea-candidate-head');
    head.append(el('strong', 'idea-candidate-title', item.title || key || 'Untitled paper'));

    const badges = el('div', 'idea-candidate-badges');
    const coreCount = Math.max(0, Number(item.cited_by_core_count || 0));
    badges.append(
      createBadge({
        text: `cited by ${coreCount} core ${coreCount === 1 ? 'paper' : 'papers'}`,
        icon: 'code',
        tone: 'info',
        border: 'solid',
      }),
      createBadge({
        text: `in-network rank ${Math.max(0, Number(item.in_network_rank || 0))}`,
        icon: 'info',
        tone: 'muted',
        border: 'solid',
      }),
    );
    head.append(badges);
    card.append(head);

    const metaParts = [];
    const authors = Array.isArray(item.authors) ? item.authors.slice(0, 6).join(', ') : '';
    if (authors) metaParts.push(authors);
    if (item.year != null) metaParts.push(String(item.year));
    if (key) metaParts.push(key);
    if (item.citation_count != null) metaParts.push(`${item.citation_count} citations`);
    if (metaParts.length > 0) {
      card.append(el('div', 'idea-candidate-meta', metaParts.join(' · ')));
    }

    if (Array.isArray(item.cited_by_core) && item.cited_by_core.length > 0) {
      card.append(el('p', 'idea-card-meta', `Cited by: ${item.cited_by_core.join(', ')}`));
    }
    if (onCitedSince && key) {
      const ctrl = renderCitedSinceControl({ key, year: item.year, isOpenQuestion: false }, { onCitedSince });
      if (ctrl) card.append(ctrl);
    }
    section.append(card);
  }
  return section;
}

/**
 * Render recorded `"Who cited this since"` lookup results, preserving open-question caveats.
 */
export function renderCitedSinceRuns(runs) {
  const list = Array.isArray(runs) ? runs : [];
  if (list.length === 0) return null;

  const wrap = el('div', 'idea-cited-since-runs');
  wrap.setAttribute('data-idea-section', 'cited-since-runs');
  wrap.append(el('h4', 'idea-field-label', `${CITED_SINCE_BUTTON_LABEL} (${list.length})`));

  for (const run of list) {
    const card = el('div', 'idea-preview-box');
    const target = String(run.target_key || run.node_id || '');
    card.setAttribute('data-cited-since-run', target);
    const rawCiting = Array.isArray(run.citing_papers) ? run.citing_papers : [];
    const citing = rawCiting.filter((cp) => cp && cp.matches_since_year !== false);
    const runYear = parseValidYear(run.since_year);
    const sinceTxt = runYear != null ? ` since ${runYear}` : '';
    card.append(
      el('div', 'idea-card-meta', `${target}${sinceTxt}: ${citing.length} citing ${citing.length === 1 ? 'paper' : 'papers'} (${CITED_SINCE_ZERO_CALLS_HINT})`),
    );
    if (run.is_open_question) {
      const note = el('p', 'idea-card-meta idea-open-question-caveat', String(run.note || CITED_SINCE_NOT_ANSWERED_NOTE));
      note.setAttribute('data-cited-since-caveat', 'true');
      card.append(note);
    }
    if (citing.length > 0) {
      const ul = el('ul', 'idea-not-searched');
      for (const cp of citing) {
        const parts = [cp.title || cp.key || 'Untitled paper'];
        if (cp.year != null) parts.push(`(${cp.year})`);
        if (cp.key) parts.push(`[${cp.key}]`);
        ul.append(el('li', '', parts.join(' ')));
      }
      card.append(ul);
    }
    wrap.append(card);
  }
  return wrap;
}

/**
 * Render Thorough Round 2 self-check progress, ceilings, thin statements, and trigger button.
 */
export function renderRound2Progress(round2, opts = {}) {
  if (!round2 || typeof round2 !== 'object') return null;
  const thin = Array.isArray(round2.thin_statements) ? round2.thin_statements : [];
  const status = String(round2.status || 'idle');
  const eligible = Boolean(round2.eligible);
  const onRunRound2 = typeof opts.onRunRound2 === 'function' ? opts.onRunRound2 : null;
  const onSearchMoreNode = typeof opts.onSearchMoreNode === 'function' ? opts.onSearchMoreNode : null;
  if (!eligible && status === 'idle' && thin.length === 0) return null;

  const box = el('div', 'idea-round2-panel');
  box.setAttribute('data-idea-section', 'round2');
  box.setAttribute('data-round2-status', status);

  const head = el('div', 'idea-chip-row');
  head.append(el('h4', 'idea-field-label', ROUND2_HEADING), createBadge(round2StatusBadge(status)));
  if (eligible && onRunRound2) {
    const btn = el('button', 'pill-btn sm', ROUND2_TRIGGER_LABEL);
    btn.type = 'button';
    btn.disabled = status === 'running';
    btn.setAttribute('data-action', 'run-round2');
    btn.addEventListener('click', () => { if (!btn.disabled) onRunRound2(); });
    head.append(btn);
  }
  box.append(head);

  const c = round2.ceilings || {};
  const u = round2.usage || {};
  const capSummary = `${round2.preset_label || round2.preset || 'Thorough'} ceilings: `
    + `${Number(u.queries_used || 0)}/${Number(c.max_queries || 36)} queries · `
    + `${Number(u.screened_used || 0)}/${Number(c.max_screened || 80)} screened · `
    + `${Number(u.papers_fetched || 0)}/${Number(c.max_papers || 12)} papers · `
    + `<= ${Number(c.calls_per_paper || 4)} calls/paper · `
    + `${Number(u.model_calls_used || 0)}/${Number(c.max_model_calls || 55)} model calls`;
  box.append(el('p', 'idea-card-meta', capSummary));

  if (round2.reason) {
    box.append(el('p', 'idea-card-meta', String(round2.reason)));
  }

  if (thin.length > 0) {
    box.append(el('span', 'idea-field-label', `Thin statements for self-check (${thin.length})`));
    const ul = el('ul', 'idea-not-searched');
    for (const item of thin) {
      const li = el('li', '');
      li.setAttribute('data-thin-target', String(item.target_id || ''));
      li.append(el('span', '', `${item.target_id || ''}: ${item.label || ''} — ${item.summary || ''}`));
      if (item.target_kind === 'node' && item.target_id && onSearchMoreNode) {
        const moreBtn = el('button', 'pill-btn sm', SEARCH_MORE_NODE_LABEL);
        moreBtn.type = 'button';
        moreBtn.setAttribute('data-action', 'search-more-node');
        moreBtn.setAttribute('data-node-id', String(item.target_id));
        moreBtn.addEventListener('click', () => onSearchMoreNode(String(item.target_id)));
        li.append(moreBtn);
      }
      ul.append(li);
    }
    box.append(ul);
  }
  return box;
}

/**
 * Composite citation-expansion + Round 2 panel embedded inside `"Where I looked"` (`idea_search.js`).
 */
export function renderCitationNetPanel(whereILooked, opts = {}) {
  if (!whereILooked || typeof whereILooked !== 'object') return null;
  const ce = whereILooked.citation_expansion || null;
  const net = whereILooked.citation_net || null;
  const round2 = whereILooked.round2 || null;

  const seeds = Number(ce?.seeds ?? (Array.isArray(net?.seeds) ? net.seeds.length : 0));
  const hops = Number(ce?.hops ?? net?.hops ?? 0);
  const unreadEl = renderUnreadCoreCitations(net?.unread_core_citations, opts);
  const runsEl = renderCitedSinceRuns(net?.cited_since_runs);
  const round2El = renderRound2Progress(round2, opts);

  if (seeds === 0 && !unreadEl && !runsEl && !round2El) return null;

  const wrap = el('div', 'idea-citation-net-panel');
  wrap.setAttribute('data-idea-section', 'citation-net');
  if (seeds > 0 || hops > 0) {
    const edgeCount = Number(net?.edges_count ?? (Array.isArray(net?.edges) ? net.edges.length : 0));
    const summary = el(
      'div',
      'idea-card-meta',
      `Citation expansion: ${seeds} ${seeds === 1 ? 'seed' : 'seeds'}, ${hops} ${hops === 1 ? 'hop' : 'hops'}, ${edgeCount} citation ${edgeCount === 1 ? 'link' : 'links'}`,
    );
    summary.setAttribute('data-citation-expansion', `${seeds}/${hops}`);
    wrap.append(summary);
  }
  if (unreadEl) wrap.append(unreadEl);
  if (runsEl) wrap.append(runsEl);
  if (round2El) wrap.append(round2El);
  return wrap;
}
