// Explore reading-queue row status renderer, Attach PDF control, and live quote list (X9–X10).
// Zero innerHTML writes: all labels, quotes, and filenames use textContent / createTextNode.
// Rule S8: zero "verified", "new", "novel", or "proved" strings in any rendered label.
import { createBadge } from './ui_badge.js';

// Named constants for owner-accepted reading-queue labels (`explore.md` 3 & Rule C8).
export const READ_ROW_LABELS = Object.freeze({
  queued: 'queued',
  fetching: 'fetching',
  from_library: 'from your library',
  reading: 'reading',
  read_full: 'read: full',
  nothing_relevant: 'nothing relevant',
  unavailable: 'unavailable: not on arXiv and not in your library; attach PDF',
  scan_ocr: 'scan: quotes are from OCR text',
});

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

/**
 * Format the exact human-readable reading-queue status string for a paper (`explore.md` 3).
 */
export function formatReadRowLabel(paper) {
  if (!paper || typeof paper !== 'object') return '';
  const state = String(paper.row_state || paper.status || '').toLowerCase();
  const linesSent = Math.max(0, Number(paper.lines_sent || 0));
  const totalLines = Math.max(0, Number(paper.total_lines || 0));
  const quoteCount = Math.max(
    0,
    Number(paper.quote_count ?? (Array.isArray(paper.quotes) ? paper.quotes.length : 0)),
  );
  switch (state) {
    case 'queued':
      return READ_ROW_LABELS.queued;
    case 'fetching':
      return READ_ROW_LABELS.fetching;
    case 'from_library':
      return READ_ROW_LABELS.from_library;
    case 'extracting': {
      const page = Math.max(1, Number(paper.page || 1));
      const total = Math.max(page, Number(paper.total_pages || page));
      return `extracting text (page ${page} of ${total})`;
    }
    case 'reading':
      return READ_ROW_LABELS.reading;
    case 'fetched':
      return paper.origin === 'library' || paper.origin === 'user_library'
        ? READ_ROW_LABELS.from_library
        : 'fetched';
    case 'read_passages':
      return `read: passages (${linesSent} of ${totalLines} lines), ${quoteCount} ${quoteCount === 1 ? 'quote' : 'quotes'}`;
    case 'read_full':
      return quoteCount > 0
        ? `${READ_ROW_LABELS.read_full}, ${quoteCount} ${quoteCount === 1 ? 'quote' : 'quotes'}`
        : READ_ROW_LABELS.read_full;
    case 'nothing_relevant':
      return READ_ROW_LABELS.nothing_relevant;
    case 'unavailable':
      return READ_ROW_LABELS.unavailable;
    case 'failed':
      return paper.reason ? `failed: ${paper.reason}` : 'failed';
    default:
      return '';
  }
}

/**
 * True when the paper's text came from a scanned PDF page (`scan_text_layer` or `ocr`, Rule C8).
 */
export function isScannedOrOcrPaper(paper) {
  const basis = String(paper?.text_basis || '').toLowerCase();
  return basis === 'scan_text_layer' || basis === 'ocr';
}

function rowTone(state) {
  switch (String(state || '').toLowerCase()) {
    case 'read_full':
    case 'read_passages':
      return { icon: 'quote', tone: 'ok', border: 'solid' };
    case 'from_library':
    case 'fetched':
      return { icon: 'pin', tone: 'info', border: 'solid' };
    case 'queued':
    case 'fetching':
    case 'extracting':
    case 'reading':
      return { icon: 'info', tone: 'info', border: 'dashed' };
    case 'nothing_relevant':
      return { icon: 'info', tone: 'muted', border: 'solid' };
    case 'unavailable':
      return { icon: 'warn', tone: 'warn', border: 'dashed' };
    case 'failed':
      return { icon: 'error', tone: 'err', border: 'dashed' };
    default:
      return { icon: 'abstract', tone: 'muted', border: 'dashed' };
  }
}

/**
 * Render the reading-queue status bar, optional OCR notice, `"Attach PDF"` button, and live
 * matched full-text quotes for one candidate paper card (`explore.md` 3, X9–X10).
 */
export function renderPaperReadRow(paper, opts = {}) {
  if (!paper || typeof paper !== 'object') return null;
  const label = formatReadRowLabel(paper);
  const scanned = isScannedOrOcrPaper(paper);
  const quotes = Array.isArray(paper.quotes) ? paper.quotes : [];
  const state = String(paper.row_state || paper.status || '').toLowerCase();
  if (!label && !scanned && quotes.length === 0) return null;

  const box = el('div', 'idea-read-row');
  box.setAttribute('data-read-row-key', String(paper.key || ''));
  box.setAttribute('data-read-row-state', state);

  const head = el('div', 'idea-chip-row');
  if (label) {
    const t = rowTone(state);
    head.append(createBadge({ text: label, icon: t.icon, tone: t.tone, border: t.border }));
  }
  if (scanned) {
    head.append(
      createBadge({
        text: READ_ROW_LABELS.scan_ocr,
        icon: 'warn',
        tone: 'warn',
        border: 'solid',
      }),
    );
  }
  if (state === 'unavailable' && typeof opts.onAttachPdf === 'function') {
    const attachBtn = el('button', 'pill-btn sm', 'Attach PDF');
    attachBtn.type = 'button';
    attachBtn.setAttribute('data-action', 'attach-pdf');
    attachBtn.setAttribute('data-paper-key', String(paper.key || ''));
    attachBtn.addEventListener('click', () => opts.onAttachPdf(String(paper.key || '')));
    head.append(attachBtn);
  }
  box.append(head);

  if (quotes.length > 0) {
    const qList = el('div', 'idea-read-quotes');
    qList.setAttribute('data-read-quotes-for', String(paper.key || ''));
    for (const q of quotes) {
      const qCard = el('div', 'idea-preview-box');
      const qId = String(q.id || '');
      if (qId) qCard.setAttribute('data-evidence-id', qId);
      const qOcr = Boolean(q.ocr_tag) || String(q.text_basis || '') === 'ocr' || String(q.text_basis || '') === 'scan_text_layer';
      const locParts = [];
      if (q.page != null) locParts.push(`p. ${q.page}`);
      if (q.start_line != null && q.end_line != null) {
        locParts.push(
          q.start_line === q.end_line
            ? `line ${q.start_line}`
            : `lines ${q.start_line}-${q.end_line}`,
        );
      }
      const badgeParts = [qId || 'quote', ...locParts];
      if (qOcr) badgeParts.push('OCR');
      qCard.append(
        createBadge({
          text: `matched · ${badgeParts.join(' · ')}`,
          icon: 'quote',
          tone: qOcr ? 'warn' : 'ok',
          border: 'solid',
        }),
      );
      if (q.quote) {
        qCard.append(el('blockquote', 'idea-candidate-abstract', `"${q.quote}"`));
      }
      if (q.gist) {
        qCard.append(el('p', 'idea-card-meta', `${q.gist} (model)`));
      }
      qList.append(qCard);
    }
    box.append(qList);
  }

  return box;
}
