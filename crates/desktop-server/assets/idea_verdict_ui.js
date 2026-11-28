// Explore Verdict table, F12 chip resolution, note indexing, and live quote-chip open helpers (X12).
// Split from idea_map.js to keep every file <= 600 lines. Zero innerHTML, zero Rule S8 forbidden words.
import { createQuoteChip } from './quote_chip.js';
import {
  BORDER_DASHED,
  BORDER_SOLID,
  MODEL_TEXT_SUFFIX,
  USER_TAG_WORD,
  createBadge,
} from './ui_badge.js';

// Named constants for owner-accepted on-screen strings (explore.md 4.1, V6, 13; overview 2.9, 2.10, 3.2 row 24).
export const VERDICT_STATUS_CHIPS = Object.freeze({
  judged_same_unconfirmed: 'Model judges: same',
  judged_same_confirmed: 'found (same)',
  judged_partial_unconfirmed: 'Model judges: partly the same',
  judged_partial_confirmed: 'found (partly)',
  judged_other_name_unconfirmed: 'Model judges: same under another name',
  judged_other_name_confirmed: 'found under another name',
  abstract_only: 'possibly related (abstracts only)',
  none_in_scope: 'not found in the searched scope',
  incomplete: 'not found (incomplete search)',
});

export const CONFIRM_SAME_LABEL = 'Looks the same to me';
export const REJECT_SAME_LABEL = 'Not the same';
export const WHAT_DIFFERS_LABEL = `what differs ${MODEL_TEXT_SUFFIX}`;
export const SUMMARY_MODEL_LABEL = `summary ${MODEL_TEXT_SUFFIX}`;
export const NOT_IN_CURRENT_MAP_LABEL = 'not in current map';

const JUDGED_STATUSES = new Set(['judged_same', 'judged_partial', 'judged_other_name']);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

/** Resolve F12 status chip text, icon, tone, and solid/dashed border (Rule V6 + overview 2.10). */
export function resolveVerdictChip(status, confirmedByUser = false, f12Chip = null) {
  const key = String(status || '').trim().toLowerCase();
  const confirmed = Boolean(confirmedByUser);
  const isJudged = JUDGED_STATUSES.has(key);
  let text = typeof f12Chip === 'string' && f12Chip.trim() ? f12Chip.trim() : '';

  if (key === 'judged_same') {
    text = confirmed ? VERDICT_STATUS_CHIPS.judged_same_confirmed : VERDICT_STATUS_CHIPS.judged_same_unconfirmed;
    return { text, icon: confirmed ? 'check' : 'model', tone: confirmed ? 'ok' : 'warn', border: confirmed ? BORDER_SOLID : BORDER_DASHED, isJudged };
  }
  if (key === 'judged_partial') {
    text = confirmed ? VERDICT_STATUS_CHIPS.judged_partial_confirmed : VERDICT_STATUS_CHIPS.judged_partial_unconfirmed;
    return { text, icon: confirmed ? 'check' : 'model', tone: confirmed ? 'ok' : 'warn', border: confirmed ? BORDER_SOLID : BORDER_DASHED, isJudged };
  }
  if (key === 'judged_other_name') {
    text = confirmed ? VERDICT_STATUS_CHIPS.judged_other_name_confirmed : VERDICT_STATUS_CHIPS.judged_other_name_unconfirmed;
    return { text, icon: confirmed ? 'check' : 'model', tone: confirmed ? 'ok' : 'warn', border: confirmed ? BORDER_SOLID : BORDER_DASHED, isJudged };
  }
  if (key === 'abstract_only') {
    return { text: text || VERDICT_STATUS_CHIPS.abstract_only, icon: 'abstract', tone: 'muted', border: BORDER_DASHED, isJudged: false };
  }
  if (key === 'none_in_scope') {
    return { text: text || VERDICT_STATUS_CHIPS.none_in_scope, icon: 'code', tone: 'ok', border: BORDER_SOLID, isJudged: false };
  }
  return { text: text || VERDICT_STATUS_CHIPS.incomplete, icon: 'warn', tone: 'warn', border: BORDER_SOLID, isJudged: false };
}

export function buildEvidenceIndex(view) {
  const byId = new Map();
  for (const item of Array.isArray(view?.evidence) ? view.evidence : []) {
    if (item?.id) byId.set(String(item.id), { ...item, _abstract: false });
  }
  for (const item of Array.isArray(view?.abstract_evidence) ? view.abstract_evidence : []) {
    if (item?.id) byId.set(String(item.id), { ...item, _abstract: true });
  }
  return byId;
}

/**
 * Index `idea.note` records by node ID and by verdict `subclaim_id`.
 * When `activeMapSeq` is known, verdict confirmation/rejection notes bound to an older `map_seq`
 * are ignored so stale rejections never leak onto an updated map.
 */
export function indexNotes(notes, activeMapSeq = null) {
  const byNode = new Map();
  const verdictLatest = new Map();
  const expectedMapSeq = Number.isInteger(activeMapSeq) && activeMapSeq > 0 ? activeMapSeq : null;

  for (const n of Array.isArray(notes) ? notes : []) {
    if (!n || typeof n !== 'object') continue;
    const kind = String(n.target_kind || '').trim();
    const tid = String(n.target_id || n.subclaim_id || '').trim();
    const action = String(n.action || '').trim();
    if (!tid || !action) continue;
    if (kind === 'node') {
      const list = byNode.get(tid) || [];
      list.push(n);
      byNode.set(tid, list);
    } else if (kind === 'verdict' && (action === 'confirm_same' || action === 'reject_same')) {
      const noteMapSeq = Number.isInteger(n.map_seq) && n.map_seq > 0 ? n.map_seq : null;
      if (expectedMapSeq != null && noteMapSeq != null && noteMapSeq !== expectedMapSeq) continue;
      verdictLatest.set(tid, action);
    }
  }
  return { byNode, verdictLatest };
}

function buildSubclaimsMap(view) {
  const map = new Map();
  const src = view?.sharpened_idea?.subclaims || view?.brief?.subclaims || [];
  for (const sc of Array.isArray(src) ? src : []) {
    if (sc?.id) map.set(String(sc.id), sc);
  }
  return map;
}

export function toQuoteChipSpec(evId, record) {
  if (!record) {
    const isAbs = String(evId).startsWith('xa-');
    return {
      evidence_id: String(evId),
      quote: '',
      basis: isAbs ? 'abstract' : 'fulltext',
      abstract_only: isAbs,
    };
  }
  const isAbs = Boolean(record._abstract) || String(evId).startsWith('xa-');
  const basisRaw = String(record.text_basis || '').toLowerCase();
  const isOcr = !isAbs && (Boolean(record.ocr_tag) || basisRaw === 'ocr' || basisRaw === 'scan_text_layer');
  return {
    evidence_id: String(evId),
    evidence_seq: record.seq ?? null,
    paper: record.paper_key || null,
    source_id: record.source_id || null,
    path: record.path || null,
    start_line: Number.isInteger(record.start_line) ? record.start_line : null,
    end_line: Number.isInteger(record.end_line) ? record.end_line : null,
    page: Number.isInteger(record.page) ? record.page : null,
    quote: String(record.quote || ''),
    basis: isAbs ? 'abstract' : (isOcr ? 'ocr' : 'fulltext'),
    ocr: isOcr,
    abstract_only: isAbs,
  };
}

/** Pure projection of one verdict row for deterministic UI testing (Rules V5, V6, S8). */
export function projectVerdictRow(verdict, subclaimsById, evidenceIndex, verdictLatest) {
  const scId = String(verdict?.subclaim_id || '');
  const sc = subclaimsById?.get?.(scId) || null;
  const ideaSentence = String(sc?.idea_quote || sc?.text || scId);
  const confirmed = Boolean(verdict?.confirmed_by_user);
  const chip = resolveVerdictChip(verdict?.status, confirmed, verdict?.chip);
  const lastAction = verdictLatest?.get?.(scId) || null;

  const quoteSpecs = [];
  const seenEv = new Set();
  const differences = [];
  for (const ov of Array.isArray(verdict?.overlaps) ? verdict.overlaps : []) {
    const evIds = Array.isArray(ov.evidence_ids) ? ov.evidence_ids : (Array.isArray(ov.evidence) ? ov.evidence : []);
    for (const id of evIds) {
      const sid = String(id);
      if (seenEv.has(sid)) continue;
      seenEv.add(sid);
      const rec = evidenceIndex?.get?.(sid) || null;
      quoteSpecs.push(toQuoteChipSpec(sid, rec ? { ...rec, paper_key: rec.paper_key || ov.paper_key } : { paper_key: ov.paper_key }));
    }
    const diff = String(ov.difference || '').trim();
    const otherTerm = typeof ov.other_name === 'string' ? ov.other_name.trim() : String(ov.other_name?.term || '').trim();
    if (diff || otherTerm) {
      differences.push({ paper: String(ov.paper_key || ''), difference: diff, otherName: otherTerm });
    }
  }
  for (const am of Array.isArray(verdict?.abstract_matches) ? verdict.abstract_matches : []) {
    const sid = String(am.evidence_id || '');
    if (sid && !seenEv.has(sid)) {
      seenEv.add(sid);
      quoteSpecs.push(toQuoteChipSpec(sid, { _abstract: true, paper_key: am.paper_key, quote: am.quote }));
    }
  }
  return {
    subclaimId: scId,
    ideaSentence,
    status: String(verdict?.status || 'incomplete'),
    confirmed,
    rejectedByUser: !confirmed && lastAction === 'reject_same',
    chip,
    verdictText: String(verdict?.text || ''),
    quoteSpecs,
    differences,
  };
}

/** Default live Explore source-reader bridge (keeps quote chips enabled without touching app.js). */
export async function openExploreSource(sessionId, target) {
  if (!sessionId || !target?.source_id) return;
  const mod = await import('./canvas_sources.js');
  if (typeof mod.openSessionSource === 'function') {
    return mod.openSessionSource(sessionId, target.source_id, target.path, target);
  }
}

/** Default live Explore OCR page-image viewer bridge (Rule C8 / overview 2.12). */
export async function openExplorePageImage(hostEl, sessionId, target) {
  if (!sessionId || !target?.source_id) return;
  const [viewerMod, apiMod] = await Promise.all([
    import('./page_image_viewer.js'),
    import('./api_idea.js'),
  ]);
  if (typeof viewerMod.createPageImageViewer !== 'function') return;
  const host = hostEl || (typeof document !== 'undefined' ? document.body : null);
  let viewer = host?._reSearchPageImageViewer || null;
  if (!viewer) {
    viewer = viewerMod.createPageImageViewer({ getToken: apiMod.resolveIdeaToken });
    if (host) {
      host._reSearchPageImageViewer = viewer;
      host.append?.(viewer.element);
    }
  }
  const openFn = viewerMod.bindQuoteChipPageImageViewer(viewer, sessionId);
  return openFn(target);
}

/** Render the prior-work Verdict table (one row per subclaim, Rule V5 + Rule V6). */
export function renderVerdictTable(view, callbacks = {}) {
  const section = el('section', 'idea-where-panel idea-verdict-section');
  section.setAttribute('data-idea-section', 'verdict-table');
  const head = el('div', 'idea-card-head');
  head.append(
    el('h3', 'idea-card-title', 'Prior-work verdicts'),
    createBadge({ text: 'by code', icon: 'code', tone: 'ok', border: BORDER_SOLID }),
  );
  section.append(head);

  const rawVerdicts = Array.isArray(view?.verdicts?.verdicts) ? view.verdicts.verdicts : [];
  if (rawVerdicts.length === 0) {
    section.append(el('p', 'idea-card-meta', 'No verdicts computed yet. Run Map after reading to evaluate each part of the idea.'));
    return section;
  }

  const subclaimsById = buildSubclaimsMap(view);
  const evidenceIndex = buildEvidenceIndex(view);
  const activeMapSeq = Number.isInteger(view?.verdicts?.map_seq) ? view.verdicts.map_seq : null;
  const { verdictLatest } = indexNotes(view?.notes, activeMapSeq);
  const retryableProviders = (Array.isArray(view?.where_i_looked?.providers) ? view.where_i_looked.providers : [])
    .filter((p) => p && (p.state === 'failed' || p.state === 'rate_limited' || p.state === 'partial'));

  const table = el('table', 'idea-verdict-table');
  const thead = el('thead');
  const hrow = el('tr');
  for (const col of ['Idea part', 'Status', 'Matched quote', WHAT_DIFFERS_LABEL, 'Confirm']) {
    hrow.append(el('th', '', col));
  }
  thead.append(hrow);
  table.append(thead);

  const tbody = el('tbody');
  for (const v of rawVerdicts) {
    const row = projectVerdictRow(v, subclaimsById, evidenceIndex, verdictLatest);
    const tr = el('tr', 'idea-verdict-row');
    tr.setAttribute('data-verdict-row', 'true');
    tr.setAttribute('data-subclaim-id', row.subclaimId);
    tr.setAttribute('data-verdict-status', row.status);
    tr.setAttribute('data-confirmed', String(row.confirmed));

    const tdPart = el('td', 'idea-verdict-part');
    tdPart.append(
      createBadge({ text: row.subclaimId || 'sc', icon: 'user', tone: 'neutral', border: BORDER_SOLID }),
      el('p', 'idea-verdict-sentence', row.ideaSentence),
    );

    const tdStatus = el('td', 'idea-verdict-status');
    tdStatus.append(
      createBadge({ text: row.chip.text, icon: row.chip.icon, tone: row.chip.tone, border: row.chip.border }),
    );
    if (row.verdictText) tdStatus.append(el('p', 'idea-card-meta', row.verdictText));
    if (row.status === 'incomplete' && retryableProviders.length > 0 && typeof callbacks.onRetryService === 'function') {
      const rWrap = el('div', 'idea-chip-row');
      for (const p of retryableProviders) {
        const rBtn = el('button', 'pill-btn sm', `Retry ${p.label || p.name}`);
        rBtn.type = 'button';
        rBtn.setAttribute('data-action', 'retry-service');
        rBtn.addEventListener('click', () => callbacks.onRetryService(String(p.name || '')));
        rWrap.append(rBtn);
      }
      tdStatus.append(rWrap);
    }

    const tdQuotes = el('td', 'idea-verdict-quotes');
    if (row.quoteSpecs.length === 0) {
      tdQuotes.append(el('span', 'idea-card-meta', 'No matched quote'));
    } else {
      for (const qSpec of row.quoteSpecs) {
        tdQuotes.append(createQuoteChip(qSpec, {
          onOpen: callbacks.onOpenSource,
          onOpenPageImage: callbacks.onOpenPageImage || callbacks.onOpenSource,
        }));
      }
    }

    const tdDiff = el('td', 'idea-verdict-diff');
    if (row.differences.length === 0) {
      tdDiff.append(el('span', 'idea-card-meta', '—'));
    } else {
      for (const d of row.differences) {
        const box = el('div', 'idea-diff-item');
        if (d.otherName) {
          box.append(createBadge({ text: `other name: ${d.otherName}`, icon: 'model', tone: 'info', border: BORDER_DASHED }));
        }
        if (d.difference) {
          box.append(el('p', 'idea-card-meta', `${d.difference} ${MODEL_TEXT_SUFFIX}`));
        }
        tdDiff.append(box);
      }
    }

    const tdConfirm = el('td', 'idea-verdict-confirm');
    if (row.chip.isJudged) {
      const actions = el('div', 'idea-chip-row');
      const noteMeta = activeMapSeq ? { map_seq: activeMapSeq } : {};
      const confirmBtn = el('button', row.confirmed ? 'pill-btn sm primary' : 'pill-btn sm', CONFIRM_SAME_LABEL);
      confirmBtn.type = 'button';
      confirmBtn.setAttribute('data-action', 'confirm-same');
      confirmBtn.setAttribute('data-subclaim-id', row.subclaimId);
      confirmBtn.setAttribute('aria-pressed', String(row.confirmed));
      if (row.confirmed) confirmBtn.disabled = true;
      confirmBtn.addEventListener('click', () => {
        callbacks.onNote?.({ target_kind: 'verdict', target_id: row.subclaimId, action: 'confirm_same', ...noteMeta });
      });

      const rejectBtn = el('button', 'pill-btn sm', REJECT_SAME_LABEL);
      rejectBtn.type = 'button';
      rejectBtn.setAttribute('data-action', 'reject-same');
      rejectBtn.setAttribute('data-subclaim-id', row.subclaimId);
      rejectBtn.setAttribute('aria-pressed', String(row.rejectedByUser));
      rejectBtn.addEventListener('click', () => {
        callbacks.onNote?.({ target_kind: 'verdict', target_id: row.subclaimId, action: 'reject_same', ...noteMeta });
      });
      actions.append(confirmBtn, rejectBtn);
      tdConfirm.append(actions);
      if (row.confirmed) {
        tdConfirm.append(createBadge({ text: `Confirmed (${USER_TAG_WORD})`, icon: 'user', tone: 'ok', border: BORDER_SOLID }));
      } else if (row.rejectedByUser) {
        tdConfirm.append(createBadge({ text: `Marked not the same (${USER_TAG_WORD})`, icon: 'user', tone: 'warn', border: BORDER_SOLID }));
      }
    } else {
      tdConfirm.append(el('span', 'idea-card-meta', 'By code'));
    }

    tr.append(tdPart, tdStatus, tdQuotes, tdDiff, tdConfirm);
    tbody.append(tr);
  }
  table.append(tbody);
  section.append(table);
  return section;
}
