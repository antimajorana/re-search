// Explore Conversation multi-paper line-citation UI (package X15).
// Renders matched line-citation chips through F6 `quote_chip.js` and explicit unread/wrong-paper
// warning badges through `ui_badge.js` using safe DOM node/text APIs only (Rule S8 compliant).
import {
  createQuoteChip,
  formatLineRange,
  formatPageLabel,
} from './quote_chip.js';
import {
  BORDER_DASHED,
  BORDER_SOLID,
  QUOTE_MATCHED_WORD,
  createBadge,
} from './ui_badge.js';

// Owner-accepted display strings (`00-overview.md` 3.2 row 20, `explore.md` Rule S8).
export const EVENT_IDEA_CHAT_CITATIONS = 'idea.chat_citations';
export const CHAT_CITE_MATCHED_WORD = QUOTE_MATCHED_WORD;
export const CHAT_CITE_UNREAD_WORD = 'unread';
export const CHAT_CITE_UNREAD_DEFAULT_REASON = 'Lines not read from this paper in this conversation';
export const CHAT_CITE_INLINE_TITLE =
  'Opens these matched lines in the source viewer. The model chose this citation.';

const SKIP_TAGS = new Set(['CODE', 'PRE', 'BUTTON', 'A', 'SCRIPT', 'STYLE']);
const INLINE_CITE_BRACKET_RE = /\[([^\[\]\n]+)\]/g;
const INLINE_RANGE_PART_RE = /L(\d+)(?:[–-](\d+))?/g;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function isPositiveInt(n) {
  return Number.isInteger(n) && n >= 1;
}

export function normalizeChatCitation(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const startLine = Number(raw.start_line ?? raw.start);
  const endLine = Number(raw.end_line ?? raw.end ?? startLine);
  const paperRef = String(raw.paper_ref ?? raw.paper_key ?? raw.source_id ?? '').trim();
  if (!paperRef) return null;
  const matched = raw.matched === true || String(raw.status || '').toLowerCase() === CHAT_CITE_MATCHED_WORD;
  const status = matched ? CHAT_CITE_MATCHED_WORD : CHAT_CITE_UNREAD_WORD;
  const textBasis = raw.text_basis ? String(raw.text_basis) : null;
  const ocr = Boolean(raw.ocr_tag) || textBasis === 'scan_text_layer' || textBasis === 'ocr';
  const page = isPositiveInt(Number(raw.page)) ? Number(raw.page) : null;
  const linePart = isPositiveInt(startLine) && isPositiveInt(endLine)
    ? (startLine === endLine ? `L${startLine}` : `L${startLine}-${endLine}`)
    : 'L?';

  return {
    raw: String(raw.raw || `[${paperRef}: ${linePart}]`),
    paper_ref: paperRef,
    paper_key: raw.paper_key ? String(raw.paper_key) : null,
    source_id: raw.source_id ? String(raw.source_id) : null,
    path: raw.path ? String(raw.path) : null,
    start_line: isPositiveInt(startLine) ? startLine : 0,
    end_line: isPositiveInt(endLine) ? endLine : 0,
    matched,
    status,
    reason: raw.reason ? String(raw.reason) : (matched ? null : CHAT_CITE_UNREAD_DEFAULT_REASON),
    page,
    ocr,
    ocr_tag: ocr ? 'OCR' : null,
    text_basis: textBasis,
    quote: raw.quote ? String(raw.quote) : (raw.excerpt ? String(raw.excerpt) : ''),
  };
}

export function partitionChatCitations(citations) {
  const matched = [];
  const unread = [];
  for (const item of Array.isArray(citations) ? citations : []) {
    const norm = normalizeChatCitation(item);
    if (!norm) continue;
    if (norm.matched) matched.push(norm);
    else unread.push(norm);
  }
  return { matched, unread };
}

/**
 * Extract checked citation payloads from a session's `events` array (optionally filtered to one
 * `turn_seq`). Pure read-only helper; tolerates missing or unknown fields.
 */
export function extractChatCitationsFromEvents(events, turnSeq = null) {
  if (!Array.isArray(events)) return [];
  const records = [];
  for (const ev of events) {
    if (!ev || ev.kind !== EVENT_IDEA_CHAT_CITATIONS) continue;
    const payload = ev.payload && typeof ev.payload === 'object' ? ev.payload : {};
    if (turnSeq != null && Number(payload.turn_seq) !== Number(turnSeq)) continue;
    records.push({
      seq: ev.seq ?? null,
      turn_seq: payload.turn_seq ?? null,
      caller: payload.caller ?? null,
      node_id: payload.node_id ?? null,
      matched_count: Number(payload.matched_count || 0),
      unread_count: Number(payload.unread_count || 0),
      citations: Array.isArray(payload.citations) ? payload.citations : [],
    });
  }
  return records;
}

/**
 * Render one matched multi-paper line citation using F6's shared `createQuoteChip`.
 */
export function renderMatchedCitationChip(cite, options = {}) {
  const norm = normalizeChatCitation(cite);
  if (!norm || !norm.matched) return null;
  const paperLabel = norm.paper_key || norm.source_id || norm.paper_ref;
  const chip = createQuoteChip(
    {
      paper: paperLabel,
      source_id: norm.source_id,
      path: norm.path,
      start_line: norm.start_line,
      end_line: norm.end_line,
      page: norm.page,
      ocr: norm.ocr,
      basis: norm.ocr ? 'ocr' : 'fulltext',
      quote: norm.quote,
    },
    {
      onOpen: options.onOpen,
      onOpenPageImage: options.onOpenPageImage,
    },
  );
  chip.setAttribute('data-chat-cite-status', CHAT_CITE_MATCHED_WORD);
  chip.setAttribute('data-paper-ref', norm.paper_ref);
  if (norm.source_id) chip.setAttribute('data-source-id', norm.source_id);
  chip.setAttribute('data-start-line', String(norm.start_line));
  chip.setAttribute('data-end-line', String(norm.end_line));
  return chip;
}

/**
 * Render an explicit dashed warning card for an unread, wrong-paper, or uningested citation.
 * Never renders a clickable matched quote chip.
 */
export function renderUnreadCitationWarning(cite) {
  const norm = normalizeChatCitation(cite);
  if (!norm || norm.matched) return null;
  const card = el('div', 'idea-chat-cite-warning');
  card.setAttribute('data-chat-cite-status', CHAT_CITE_UNREAD_WORD);
  card.setAttribute('data-border', BORDER_DASHED);
  card.setAttribute('data-paper-ref', norm.paper_ref);
  card.setAttribute('role', 'status');

  const head = el('div', 'idea-chat-cite-warning-head');
  head.append(
    createBadge({
      text: CHAT_CITE_UNREAD_WORD,
      icon: 'warn',
      tone: 'warn',
      border: BORDER_DASHED,
    }),
    el('code', 'idea-chat-cite-token', norm.raw),
  );
  const locParts = [];
  const lineLabel = formatLineRange(norm.start_line, norm.end_line);
  if (lineLabel) locParts.push(lineLabel);
  const pageLabel = formatPageLabel(norm.page);
  if (pageLabel) locParts.push(pageLabel);
  if (locParts.length > 0) {
    head.append(el('span', 'idea-chat-cite-loc', locParts.join(' · ')));
  }

  const reason = el('p', 'idea-chat-cite-reason', norm.reason || CHAT_CITE_UNREAD_DEFAULT_REASON);
  card.append(head, reason);
  return card;
}

/**
 * Render the complete multi-paper citation panel for an Explore Conversation answer:
 * summary badges ("N matched", "M unread"), matched F6 quote chips, and unread warnings.
 */
export function renderExploreChatCitations(citations, options = {}) {
  const { matched, unread } = partitionChatCitations(citations);
  if (matched.length === 0 && unread.length === 0) return null;

  const section = el('section', 'idea-chat-citations');
  section.setAttribute('data-idea-section', 'chat-citations');
  section.setAttribute('aria-label', 'Conversation citations');

  const head = el('div', 'idea-chat-citations-head');
  head.append(el('strong', 'idea-chat-citations-title', 'Line citations'));
  if (matched.length > 0) {
    head.append(
      createBadge({
        text: `${matched.length} ${CHAT_CITE_MATCHED_WORD}`,
        icon: 'quote',
        tone: 'ok',
        border: BORDER_SOLID,
      }),
    );
  }
  if (unread.length > 0) {
    head.append(
      createBadge({
        text: `${unread.length} ${CHAT_CITE_UNREAD_WORD}`,
        icon: 'warn',
        tone: 'warn',
        border: BORDER_DASHED,
      }),
    );
  }
  section.append(head);

  if (matched.length > 0) {
    const matchedList = el('div', 'idea-chat-citations-matched');
    matchedList.setAttribute('data-chat-cite-group', CHAT_CITE_MATCHED_WORD);
    for (const item of matched) {
      const chip = renderMatchedCitationChip(item, options);
      if (chip) matchedList.append(chip);
    }
    section.append(matchedList);
  }

  if (unread.length > 0) {
    const unreadList = el('div', 'idea-chat-citations-unread');
    unreadList.setAttribute('data-chat-cite-group', CHAT_CITE_UNREAD_WORD);
    for (const item of unread) {
      const warn = renderUnreadCitationWarning(item);
      if (warn) unreadList.append(warn);
    }
    section.append(unreadList);
  }

  return section;
}

function shouldSkipNode(node) {
  if (!node || node.nodeType === 3) return false;
  if (SKIP_TAGS.has(String(node.tagName || '').toUpperCase())) return true;
  if (node.classList?.contains?.('katex') || node.classList?.contains?.('katex-display')) return true;
  const classes = String(node.className || '').split(/\s+/);
  return classes.includes('katex') || classes.includes('katex-display');
}

function buildCitationLookup(citations) {
  const byKey = new Map();
  for (const raw of Array.isArray(citations) ? citations : []) {
    const norm = normalizeChatCitation(raw);
    if (!norm) continue;
    const key = `${norm.paper_ref.toLowerCase()}|${norm.start_line}|${norm.end_line}`;
    byKey.set(key, norm);
  }
  return byKey;
}

function parseInlineSegment(segment) {
  const trimmed = segment.trim();
  const colon = trimmed.lastIndexOf(':');
  if (colon <= 0) return null;
  const paperRef = trimmed.slice(0, colon).trim();
  const rangeList = trimmed.slice(colon + 1).trim();
  if (!paperRef || !/^L\d+(?:[–-]\d+)?(?:,[ \t]*L\d+(?:[–-]\d+)?)*$/.test(rangeList)) {
    return null;
  }
  return { paperRef, rangeList };
}

function splitInlineBracketSegments(inner) {
  if (inner.includes(';')) {
    const segs = inner.split(';').map(parseInlineSegment);
    if (segs.every(Boolean)) return segs;
  }
  const single = parseInlineSegment(inner);
  return single ? [single] : null;
}

function replaceTextWithMultiPaperCites(text, lookup, options) {
  const onOpen = typeof options.onOpen === 'function' ? options.onOpen : null;
  const onOpenPageImage = typeof options.onOpenPageImage === 'function' ? options.onOpenPageImage : null;
  INLINE_CITE_BRACKET_RE.lastIndex = 0;
  const nodes = [];
  let last = 0;
  let changed = false;

  for (const match of text.matchAll(INLINE_CITE_BRACKET_RE)) {
    const full = match[0];
    const inner = match[1];
    const segments = splitInlineBracketSegments(inner);
    if (!segments) continue;

    const bracketPieces = [];
    let groupChanged = false;
    for (let sIdx = 0; sIdx < segments.length; sIdx += 1) {
      if (sIdx > 0) bracketPieces.push(document.createTextNode('; '));
      const { paperRef, rangeList } = segments[sIdx];
      bracketPieces.push(document.createTextNode(`${paperRef}: `));
      let partLast = 0;
      INLINE_RANGE_PART_RE.lastIndex = 0;
      for (const part of rangeList.matchAll(INLINE_RANGE_PART_RE)) {
        if (part.index > partLast) {
          bracketPieces.push(document.createTextNode(rangeList.slice(partLast, part.index)));
        }
        const start = Number(part[1]);
        const end = part[2] !== undefined ? Number(part[2]) : start;
        const entry = lookup.get(`${paperRef.toLowerCase()}|${start}|${end}`);
        if (entry && entry.matched) {
          const btn = el('button', 'line-cite idea-chat-inline-matched', part[0]);
          btn.type = 'button';
          const pageSuffix = entry.page ? `, ${formatPageLabel(entry.page)}` : '';
          btn.title = entry.ocr
            ? `${CHAT_CITE_INLINE_TITLE} (OCR${pageSuffix})`
            : `${CHAT_CITE_INLINE_TITLE}${pageSuffix ? ` (${pageSuffix.slice(2)})` : ''}`;
          btn.setAttribute('data-chat-cite-status', CHAT_CITE_MATCHED_WORD);
          btn.setAttribute('data-paper-ref', entry.paper_ref);
          if (entry.source_id) btn.setAttribute('data-source-id', entry.source_id);
          if (entry.ocr) btn.setAttribute('data-ocr', 'true');
          btn.setAttribute('data-start-line', String(entry.start_line));
          btn.setAttribute('data-end-line', String(entry.end_line));
          const target = { ...entry, openPageImage: Boolean(entry.ocr) };
          const handler = entry.ocr && onOpenPageImage ? onOpenPageImage : onOpen;
          if (handler) btn.addEventListener('click', (ev) => handler(target, ev));
          bracketPieces.push(btn);
          groupChanged = true;
        } else if (entry && !entry.matched) {
          const warn = el('span', 'idea-chat-inline-unread', `${part[0]} (${CHAT_CITE_UNREAD_WORD})`);
          warn.title = entry.reason || CHAT_CITE_UNREAD_DEFAULT_REASON;
          warn.setAttribute('data-chat-cite-status', CHAT_CITE_UNREAD_WORD);
          warn.setAttribute('data-paper-ref', entry.paper_ref);
          bracketPieces.push(warn);
          groupChanged = true;
        } else {
          bracketPieces.push(document.createTextNode(part[0]));
        }
        partLast = part.index + part[0].length;
      }
      if (partLast < rangeList.length) {
        bracketPieces.push(document.createTextNode(rangeList.slice(partLast)));
      }
    }
    if (!groupChanged) continue;

    if (match.index > last) nodes.push(document.createTextNode(text.slice(last, match.index)));
    nodes.push(document.createTextNode('['), ...bracketPieces, document.createTextNode(']'));
    last = match.index + full.length;
    changed = true;
  }

  if (!changed) return null;
  if (last < text.length) nodes.push(document.createTextNode(text.slice(last)));
  return nodes;
}

function walkAndLink(node, lookup, options) {
  if (!node || shouldSkipNode(node)) return;
  const list = [...(node.childNodes || node.children || [])];
  for (const child of list) {
    if (child.nodeType === 3) {
      const value = child.nodeValue ?? child.textContent ?? '';
      if (!value.includes('[') || !value.includes(':')) continue;
      const replacement = replaceTextWithMultiPaperCites(value, lookup, options);
      if (!replacement) continue;
      if (typeof child.replaceWith === 'function') {
        child.replaceWith(...replacement);
      } else if (typeof node.insertBefore === 'function' && typeof node.removeChild === 'function') {
        for (const item of replacement) node.insertBefore(item, child);
        node.removeChild(child);
      }
    } else {
      walkAndLink(child, lookup, options);
    }
  }
}

/**
 * Link matched multi-paper `[paper_ref: Lstart-end]` citations inline inside an Explore
 * Conversation answer container and tag unread citations with explicit inline warnings.
 */
export function linkExploreChatCitations(container, citations, options = {}) {
  if (!container) return;
  const lookup = buildCitationLookup(citations);
  if (!lookup.size) return;
  walkAndLink(container, lookup, options);
}

/**
 * Standalone mounter for the Explore Conversation citation bar (exported for Conversation tab
 * integration without pushing `idea.js` over the 600-line ceiling).
 *
 * Accepts an array of citations, a single `idea.chat_citations` payload, or `{ events, turn_seq }`.
 * Returns `{ update, applySseEvent, destroy }`.
 */
export function mountExploreChatCitations(hostEl, initialSource = null, options = {}) {
  if (!hostEl) {
    return { update: () => null, applySseEvent: () => null, destroy: () => {} };
  }
  let currentCitations = resolveCitationsInput(initialSource, options.turnSeq ?? null);

  function render() {
    const panel = renderExploreChatCitations(currentCitations, options);
    if (typeof hostEl.replaceChildren === 'function') {
      if (panel) hostEl.replaceChildren(panel);
      else hostEl.replaceChildren();
    } else {
      hostEl.children = panel ? [panel] : [];
    }
    return panel;
  }

  function update(nextSource, turnSeq = options.turnSeq ?? null) {
    currentCitations = resolveCitationsInput(nextSource, turnSeq);
    return render();
  }

  function applySseEvent(kindOrEnvelope, maybePayload) {
    const kind =
      kindOrEnvelope && typeof kindOrEnvelope === 'object' ? kindOrEnvelope.kind : kindOrEnvelope;
    const payload =
      kindOrEnvelope && typeof kindOrEnvelope === 'object'
        ? (kindOrEnvelope.payload ?? kindOrEnvelope.data ?? {})
        : (maybePayload || {});
    if (kind !== EVENT_IDEA_CHAT_CITATIONS) return null;
    if (options.turnSeq != null && Number(payload.turn_seq) !== Number(options.turnSeq)) {
      return null;
    }
    currentCitations = Array.isArray(payload.citations) ? payload.citations : [];
    return render();
  }

  function destroy() {
    currentCitations = [];
    if (typeof hostEl.replaceChildren === 'function') hostEl.replaceChildren();
    else hostEl.children = [];
  }

  render();
  return { update, applySseEvent, destroy };
}

function resolveCitationsInput(source, turnSeq) {
  if (Array.isArray(source)) return source;
  if (!source || typeof source !== 'object') return [];
  if (Array.isArray(source.citations)) return source.citations;
  if (Array.isArray(source.events)) {
    const recs = extractChatCitationsFromEvents(source.events, turnSeq);
    if (recs.length === 0) return [];
    return recs[recs.length - 1].citations;
  }
  return [];
}
