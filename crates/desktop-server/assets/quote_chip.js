// Shared quote chip component across Investigate, Explore and Solve (overview 2.5, 2.6, 2.10, 2.12).
// Clamps quote text to at most 300 characters, displays paper/lines/optional page, and opens the source.
import {
  ABSTRACT_ONLY_WORD,
  BORDER_DASHED,
  BORDER_SOLID,
  OCR_TAG_WORD,
  QUOTE_MATCHED_WORD,
  createBadge,
} from './ui_badge.js';

// Overview 2.5 & brief item 3: quotes shown or exported are at most 300 characters.
export const MAX_QUOTE_CHARS = 300;
// Overview 2.12 & Decision 8.3 #3: scanned-page OCR wording.
export const QUOTE_OCR_MATCHED_PREFIX = `${QUOTE_MATCHED_WORD} in ${OCR_TAG_WORD} text`;
export const QUOTE_OPEN_SOURCE_LABEL = 'Open source';
export const QUOTE_OPEN_PAGE_IMAGE_LABEL = 'Open page image';

export function clampQuoteText(quote, maxChars = MAX_QUOTE_CHARS) {
  const limit = Math.max(1, Math.min(MAX_QUOTE_CHARS, Math.floor(Number(maxChars) || MAX_QUOTE_CHARS)));
  const clean = String(quote ?? '').trim();
  if (clean.length <= limit) return clean;
  if (limit === 1) return '…';
  return clean.slice(0, limit - 1).trimEnd() + '…';
}

export function formatPaperLabel(paper) {
  if (!paper) return '';
  if (typeof paper === 'string') return paper.trim();
  if (typeof paper === 'object') {
    const candidate = paper.label || paper.short_ref || paper.title || paper.key || paper.ids?.arxiv || paper.ids?.doi;
    return String(candidate ?? '').trim();
  }
  return String(paper).trim();
}

export function formatLineRange(startLine, endLine) {
  const s = Number.isInteger(startLine) && startLine > 0 ? startLine : null;
  const e = Number.isInteger(endLine) && endLine > 0 ? endLine : s;
  if (s == null) return '';
  if (e == null || e <= s) return `line ${s}`;
  return `lines ${s}–${e}`;
}

export function formatPageLabel(page) {
  const p = Number.isInteger(page) && page > 0 ? page : null;
  return p != null ? `p. ${p}` : '';
}

export function resolveQuoteBasis(spec = {}) {
  const raw = String(spec.basis || '').trim().toLowerCase();
  if (spec.abstract_only || raw === 'abstract' || raw === 'abstract_only') return 'abstract';
  if (spec.ocr || raw === 'ocr' || raw === 'scan' || raw === 'scan_text_layer') return 'ocr';
  return 'fulltext';
}

export function formatQuoteLocation(spec = {}) {
  const parts = [];
  const paper = formatPaperLabel(spec.paper);
  if (paper) parts.push(paper);
  const lines = formatLineRange(spec.start_line, spec.end_line);
  if (lines) parts.push(lines);
  const page = formatPageLabel(spec.page);
  if (page) parts.push(page);
  return parts.join(' · ');
}

export function createQuoteChip(spec = {}, options = {}) {
  const basis = resolveQuoteBasis(spec);
  const isOcr = basis === 'ocr';
  const isAbstract = basis === 'abstract';
  const border = isAbstract ? BORDER_DASHED : BORDER_SOLID;
  const grade = isAbstract ? 'quote_abstract' : (isOcr ? 'quote_ocr' : 'quote_fulltext');

  const chip = document.createElement('div');
  chip.className = `ui-quote-chip ui-quote-chip--${basis} ui-quote-chip--${border}`;
  chip.setAttribute('data-grade', grade);
  chip.setAttribute('data-border', border);
  chip.setAttribute('data-basis', basis);
  if (isOcr) chip.setAttribute('data-ocr', 'true');
  if (isAbstract) chip.setAttribute('data-abstract-only', 'true');

  const header = document.createElement('div');
  header.className = 'ui-quote-chip-header';

  const pageLabel = formatPageLabel(spec.page);
  let badgeSpec;
  if (isAbstract) {
    badgeSpec = {
      text: ABSTRACT_ONLY_WORD,
      icon: 'abstract',
      tone: 'muted',
      border: BORDER_DASHED,
      grade,
    };
  } else if (isOcr) {
    badgeSpec = {
      text: pageLabel ? `${QUOTE_OCR_MATCHED_PREFIX}, ${pageLabel}` : QUOTE_OCR_MATCHED_PREFIX,
      icon: 'ocr',
      tone: 'warn',
      border: BORDER_SOLID,
      grade,
    };
  } else {
    badgeSpec = {
      text: pageLabel ? `${QUOTE_MATCHED_WORD}, ${pageLabel}` : QUOTE_MATCHED_WORD,
      icon: 'quote',
      tone: 'ok',
      border: BORDER_SOLID,
      grade,
    };
  }
  header.appendChild(createBadge(badgeSpec));

  if (isOcr) {
    header.appendChild(createBadge({
      text: OCR_TAG_WORD,
      icon: 'ocr',
      tone: 'warn',
      border: BORDER_SOLID,
    }));
  }

  if (spec.evidence_id) {
    const idTag = document.createElement('code');
    idTag.className = 'ui-quote-chip-id';
    idTag.textContent = String(spec.evidence_id);
    header.appendChild(idTag);
  }
  chip.appendChild(header);

  const clamped = clampQuoteText(spec.quote, options.maxChars ?? MAX_QUOTE_CHARS);
  if (clamped) {
    const quoteEl = document.createElement('blockquote');
    quoteEl.className = 'ui-quote-chip-text';
    // Overview 2.12 rule 2: OCR text is never rendered as math; all quote text uses textContent.
    quoteEl.textContent = clamped;
    chip.appendChild(quoteEl);
  }

  const locationText = formatQuoteLocation(spec);
  const openTarget = {
    source_id: spec.source_id ?? null,
    path: spec.path ?? null,
    start_line: Number.isInteger(spec.start_line) ? spec.start_line : null,
    end_line: Number.isInteger(spec.end_line) ? spec.end_line : null,
    page: Number.isInteger(spec.page) ? spec.page : null,
    paper: spec.paper ?? null,
    evidence_id: spec.evidence_id ?? null,
    evidence_seq: spec.evidence_seq ?? null,
    ocr: isOcr,
    abstract_only: isAbstract,
    openPageImage: isOcr,
  };

  const openHandler = isOcr && typeof options.onOpenPageImage === 'function'
    ? options.onOpenPageImage
    : (typeof options.onOpen === 'function' ? options.onOpen : null);

  const footer = document.createElement('div');
  footer.className = 'ui-quote-chip-footer';

  const openBtn = document.createElement('button');
  openBtn.type = 'button';
  openBtn.className = 'ui-quote-chip-open';
  const fallbackAction = isOcr ? QUOTE_OPEN_PAGE_IMAGE_LABEL : QUOTE_OPEN_SOURCE_LABEL;
  openBtn.textContent = locationText || fallbackAction;
  openBtn.setAttribute('aria-label', locationText ? `${fallbackAction}: ${locationText}` : fallbackAction);
  if (!openHandler) {
    openBtn.disabled = true;
  } else {
    openBtn.addEventListener('click', (ev) => openHandler(openTarget, ev));
  }
  footer.appendChild(openBtn);
  chip.appendChild(footer);

  return chip;
}
