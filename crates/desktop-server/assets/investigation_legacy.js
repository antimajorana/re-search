// Investigate V9 read-side legacy UI module (investigate.md §0.2 #6, §8, §9 V9).
// Pure DOM builder (createElement + textContent; zero innerHTML) for "Earlier map"
// sections on skeleton node cards, "Unplaced (older map)" trays, read-only overview &
// conventions, and relabelling legacy "Find main results" -> "Rebuild earlier map".

import { BORDER_DASHED, BORDER_SOLID, createBadge } from './ui_badge.js';

export const LEGACY_SECTION_TITLE = 'Earlier map';
export const LEGACY_UNPLACED_TITLE = 'Unplaced (older map)';
export const LEGACY_EDGE_ROLE_LABEL = 'unknown (older map)';
export const LEGACY_OVERVIEW_TITLE = 'Overview (older session)';
export const LEGACY_CONVENTIONS_TITLE = 'Conventions (older session)';
export const LEGACY_ACTIVITY_TITLE = 'Earlier questions & checks';
export const LEGACY_REBUILD_BUTTON_LABEL = 'Rebuild earlier map';
const FIND_MAIN_RESULTS_LABEL = 'Find main results';
const DEFAULT_FALLBACK_NOTE = 'Source files changed or could not be checked; showing the earlier view.';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function formatAnchorLine(anchor) {
  if (!anchor || typeof anchor !== 'object') return '';
  const path = String(anchor.path || '').trim();
  const start = Number(anchor.start_line || 0);
  const end = Number(anchor.end_line || 0);
  const loc = path && start > 0
    ? `${path}:${end > start ? `${start}-${end}` : start}`
    : (path || (start > 0 ? `line ${start}` : 'no physical line range'));
  const snippet = String(anchor.quote || anchor.excerpt || '').trim();
  return snippet ? `${loc} — "${snippet}"` : loc;
}

function buildLegacyNodeCard(item, opts = {}) {
  const card = el('article', 'inv-legacy-node-card');
  card.setAttribute('data-legacy-id', String(item?.id || ''));
  card.setAttribute('data-grade', 'model');
  card.setAttribute('data-border', BORDER_DASHED);

  const head = el('div', 'inv-legacy-node-head');
  head.appendChild(createBadge({
    grade: 'model',
    text: String(item?.id || 'legacy:unknown'),
    border: BORDER_DASHED,
  }));
  const kindLabel = item?.main
    ? `${String(item?.kind || 'claim')} · main (older map)`
    : String(item?.kind || 'claim');
  head.appendChild(el('span', 'ui-node-card-flag', kindLabel));
  card.appendChild(head);

  card.appendChild(el('p', 'inv-legacy-sentence', String(item?.sentence || '')));

  if (Array.isArray(item?.anchors) && item.anchors.length > 0) {
    const anchorsList = el('ul', 'inv-legacy-anchors');
    for (const anchor of item.anchors) {
      anchorsList.appendChild(el('li', 'inv-legacy-anchor-item', formatAnchorLine(anchor)));
    }
    card.appendChild(anchorsList);
  }

  if (Array.isArray(item?.edges) && item.edges.length > 0) {
    const edgesWrap = el('div', 'inv-legacy-edges');
    for (const edge of item.edges) {
      const chip = el('div', 'inv-legacy-edge');
      chip.setAttribute('data-border', BORDER_DASHED);
      chip.setAttribute('data-edge-id', String(edge?.id || ''));
      const arrow = edge?.direction === 'incoming' ? 'From' : 'To';
      const peer = String(edge?.peer_id || (edge?.direction === 'incoming' ? edge?.src : edge?.dst) || '');
      const role = LEGACY_EDGE_ROLE_LABEL;
      const badge = createBadge({
        grade: 'model',
        text: `${arrow} ${peer} · ${role}`,
        border: BORDER_DASHED,
      });
      if (edge?.peer_tag && typeof opts.onSelectNode === 'function') {
        const jumpBtn = el('button', 'inv-legacy-jump-btn');
        jumpBtn.type = 'button';
        jumpBtn.appendChild(badge);
        jumpBtn.addEventListener('click', () => opts.onSelectNode(edge.peer_tag));
        chip.appendChild(jumpBtn);
      } else {
        chip.appendChild(badge);
      }
      const reason = String(edge?.reason || '').trim();
      if (reason) chip.appendChild(el('span', 'inv-legacy-edge-reason', reason));
      edgesWrap.appendChild(chip);
    }
    card.appendChild(edgesWrap);
  }

  if (Array.isArray(item?.corrections) && item.corrections.length > 0) {
    const corrWrap = el('div', 'inv-legacy-corrections');
    for (const note of item.corrections) {
      const box = el('div', 'inv-legacy-correction');
      box.setAttribute('data-grade', 'user');
      box.setAttribute('data-border', BORDER_SOLID);
      box.appendChild(createBadge({
        grade: 'user',
        text: 'Note (older map)',
        border: BORDER_SOLID,
      }));
      box.appendChild(el('p', 'inv-legacy-correction-text', String(note?.text || '')));
      corrWrap.appendChild(box);
    }
    card.appendChild(corrWrap);
  }

  return card;
}

/**
 * Mounts attached legacy nodes under `"Earlier map"` inside a skeleton node card's Block A.
 * Clears or skips mounting cleanly when `legacyNodes` is empty.
 */
export function mountLegacyEarlierMap(nodeHost, legacyNodes, opts = {}) {
  if (!nodeHost) return null;
  const list = Array.isArray(legacyNodes) ? legacyNodes.filter(Boolean) : [];
  const existing = [...(nodeHost.children || [])].find((c) => c?.classList?.contains?.('inv-legacy-earlier-map'));
  if (!list.length) {
    if (existing && typeof nodeHost.replaceChildren === 'function') {
      nodeHost.replaceChildren(...[...nodeHost.children].filter((c) => c !== existing));
    }
    return null;
  }

  const section = el('section', 'inv-legacy-earlier-map');
  section.setAttribute('aria-label', LEGACY_SECTION_TITLE);
  const header = el('div', 'inv-legacy-section-header');
  header.appendChild(el('h4', 'inv-legacy-heading', LEGACY_SECTION_TITLE));
  header.appendChild(el('span', 'inv-legacy-count', `${list.length} ${list.length === 1 ? 'entry' : 'entries'}`));
  section.appendChild(header);

  for (const item of list) {
    section.appendChild(buildLegacyNodeCard(item, opts));
  }

  if (opts.replaceHost) {
    nodeHost.replaceChildren(section);
  } else if (existing) {
    const next = [...nodeHost.children].map((c) => (c === existing ? section : c));
    nodeHost.replaceChildren(...next);
  } else {
    nodeHost.appendChild(section);
  }
  return { element: section, count: list.length };
}

/**
 * Relabels `"Find main results"` -> `"Rebuild earlier map"` on active old sessions
 * (`investigate.md` §8 #6), and restores it when switching to a session without legacy records.
 */
export function relabelOldSessionButtons(root, legacyPayload) {
  const isLegacy = Boolean(legacyPayload?.has_legacy);
  const doc = root || (typeof document !== 'undefined' ? document : null);
  if (!doc) return 0;

  let buttons = [];
  if (typeof doc.querySelectorAll === 'function') {
    buttons = [...doc.querySelectorAll('button')];
  } else if (typeof doc.getElementById === 'function') {
    for (const id of ['investigation-setup', 'investigation-answer']) {
      const container = doc.getElementById(id);
      if (container?.querySelectorAll) buttons.push(...container.querySelectorAll('button'));
    }
  }

  let updated = 0;
  for (const btn of buttons) {
    if (!btn) continue;
    const text = String(btn.textContent || '').trim();
    if (isLegacy && text === FIND_MAIN_RESULTS_LABEL) {
      btn.textContent = LEGACY_REBUILD_BUTTON_LABEL;
      btn.setAttribute?.('data-legacy-relabelled', 'true');
      updated += 1;
    } else if (!isLegacy && btn.getAttribute?.('data-legacy-relabelled') === 'true') {
      btn.textContent = FIND_MAIN_RESULTS_LABEL;
      btn.removeAttribute?.('data-legacy-relabelled');
      updated += 1;
    }
  }
  return updated;
}

function renderSummaryBlock(summary) {
  if (!summary || typeof summary !== 'object') return null;
  const block = el('section', 'inv-legacy-subsection inv-legacy-overview');
  block.setAttribute('aria-label', LEGACY_OVERVIEW_TITLE);
  block.appendChild(el('h4', 'inv-legacy-heading', LEGACY_OVERVIEW_TITLE));
  if (summary.title) block.appendChild(el('div', 'inv-legacy-overview-title', String(summary.title)));
  for (const [key, label] of [['what', 'What'], ['why', 'Why'], ['how', 'How']]) {
    const part = summary[key];
    if (!part || typeof part !== 'object') continue;
    const text = part.unstated ? 'Not stated in available source.' : String(part.text || '').trim();
    if (!text) continue;
    const row = el('div', 'inv-legacy-overview-row');
    row.appendChild(el('strong', 'inv-legacy-overview-key', `${label}: `));
    row.appendChild(el('span', 'inv-legacy-overview-text', text));
    block.appendChild(row);
  }
  return block;
}

function renderConventionsBlock(conventions) {
  if (!conventions || typeof conventions !== 'object') return null;
  const items = Array.isArray(conventions.items) ? conventions.items : [];
  const lims = Array.isArray(conventions.limitations) ? conventions.limitations : [];
  if (!items.length && !lims.length) return null;

  const block = el('section', 'inv-legacy-subsection inv-legacy-conventions');
  block.setAttribute('aria-label', LEGACY_CONVENTIONS_TITLE);
  block.appendChild(el('h4', 'inv-legacy-heading', LEGACY_CONVENTIONS_TITLE));

  if (items.length) {
    const ul = el('ul', 'inv-legacy-conventions-list');
    for (const item of items) {
      if (!item || typeof item !== 'object') continue;
      const head = String(item.symbol || item.term || item.name || item.id || '').trim();
      const desc = String(item.meaning || item.definition || item.statement || item.text || '').trim();
      const line = head && desc ? `${head}: ${desc}` : (head || desc);
      if (line) ul.appendChild(el('li', 'inv-legacy-convention-item', line));
    }
    block.appendChild(ul);
  }
  for (const lim of lims) {
    if (String(lim || '').trim()) block.appendChild(el('p', 'inv-legacy-limitation', String(lim)));
  }
  return block;
}

function renderActivitiesAndAnswers(legacyPayload) {
  const answers = Array.isArray(legacyPayload.answers) ? legacyPayload.answers : [];
  const activities = Array.isArray(legacyPayload.activities) ? legacyPayload.activities : [];
  const unattachedNotes = (Array.isArray(legacyPayload.corrections) ? legacyPayload.corrections : [])
    .filter((c) => !c?.node_id && String(c?.text || '').trim());
  if (!answers.length && !activities.length && !unattachedNotes.length) return null;

  const block = el('section', 'inv-legacy-subsection inv-legacy-history');
  block.setAttribute('aria-label', LEGACY_ACTIVITY_TITLE);
  block.appendChild(el('h4', 'inv-legacy-heading', LEGACY_ACTIVITY_TITLE));

  for (const ans of answers) {
    const item = el('div', 'inv-legacy-history-item');
    item.appendChild(el('div', 'inv-legacy-history-q', `Q: ${String(ans?.question || '').trim()}`));
    const body = String(ans?.text || ans?.error || ans?.status || '').trim();
    if (body) item.appendChild(el('div', 'inv-legacy-history-a', body));
    block.appendChild(item);
  }
  for (const act of activities) {
    const item = el('div', 'inv-legacy-history-item');
    const q = String(act?.question || act?.prompt || act?.kind || 'Check').trim();
    item.appendChild(el('div', 'inv-legacy-history-q', q));
    const res = String(act?.result || act?.status || '').trim();
    if (res) item.appendChild(el('div', 'inv-legacy-history-a', res));
    block.appendChild(item);
  }
  for (const note of unattachedNotes) {
    const item = el('div', 'inv-legacy-correction');
    item.appendChild(createBadge({ grade: 'user', text: 'Note (older map)', border: BORDER_SOLID }));
    item.appendChild(el('p', 'inv-legacy-correction-text', String(note.text)));
    block.appendChild(item);
  }
  return block;
}

/**
 * Mounts `"Unplaced (older map)"`, read-only legacy overview & conventions panels,
 * fallback diagnostic banner (if `fallback_old_view`), and relabels `"Find main results"`
 * to `"Rebuild earlier map"` on old sessions (`investigate.md` §8 #6).
 */
export function mountLegacyPanel(shellHost, legacyPayload, opts = {}) {
  relabelOldSessionButtons(opts.root, legacyPayload);
  if (!shellHost) return null;

  const prior = [...(shellHost.children || [])].filter((c) => c?.classList?.contains?.('inv-legacy-panel'));
  if (!legacyPayload || !legacyPayload.has_legacy) {
    if (prior.length && typeof shellHost.replaceChildren === 'function') {
      shellHost.replaceChildren(...[...shellHost.children].filter((c) => !c?.classList?.contains?.('inv-legacy-panel')));
    }
    return null;
  }

  const panel = el('section', 'inv-legacy-panel');
  panel.setAttribute('aria-label', 'Older map details');

  if (legacyPayload.fallback_old_view) {
    const diag = el('div', 'inv-legacy-fallback-banner', String(legacyPayload.diagnostic_note || DEFAULT_FALLBACK_NOTE));
    diag.setAttribute('role', 'status');
    panel.appendChild(diag);
  }

  const unplaced = Array.isArray(legacyPayload.unplaced) ? legacyPayload.unplaced : [];
  if (unplaced.length > 0) {
    const sec = el('section', 'inv-legacy-subsection inv-legacy-unplaced');
    sec.setAttribute('aria-label', LEGACY_UNPLACED_TITLE);
    const hdr = el('div', 'inv-legacy-section-header');
    hdr.appendChild(el('h4', 'inv-legacy-heading', LEGACY_UNPLACED_TITLE));
    hdr.appendChild(el('span', 'inv-legacy-count', String(unplaced.length)));
    sec.appendChild(hdr);
    for (const node of unplaced) {
      sec.appendChild(buildLegacyNodeCard(node, opts));
    }
    panel.appendChild(sec);
  }

  const overview = renderSummaryBlock(legacyPayload.summary);
  if (overview) panel.appendChild(overview);

  const conventions = renderConventionsBlock(legacyPayload.conventions);
  if (conventions) panel.appendChild(conventions);

  const history = renderActivitiesAndAnswers(legacyPayload);
  if (history) panel.appendChild(history);

  if (opts.replaceHost) {
    shellHost.replaceChildren(panel);
  } else if (prior.length) {
    let inserted = false;
    const next = [];
    for (const c of shellHost.children) {
      if (c?.classList?.contains?.('inv-legacy-panel')) {
        if (!inserted) {
          next.push(panel);
          inserted = true;
        }
      } else {
        next.push(c);
      }
    }
    shellHost.replaceChildren(...next);
  } else {
    shellHost.appendChild(panel);
  }

  return { element: panel };
}
