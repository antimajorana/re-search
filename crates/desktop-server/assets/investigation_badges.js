// Investigate V7 authors'-claim half-badge, deterministic cue/heading-cite display,
// quote popover, reader pin/unpin toggle, and V16 right-slot placeholder
// (00-overview.md §0.1, §2.6, §3.2 rows 20 & 24; investigate.md §3, §4.3, rule 6.6).
//
// Zero innerHTML writes: all untrusted paper/model/ledger text flows through textContent.
// Never uses the word "verified"; "new" appears ONLY in the authors' left half-badge.

import {
  BORDER_DASHED,
  BORDER_SOLID,
  MODEL_TEXT_SUFFIX,
  createBadge,
  createSplitBadge,
  formatModelText,
} from './ui_badge.js';
import { createQuoteChip } from './quote_chip.js';

// Named constants for owner-accepted decisions (00-overview.md §0.1, §3.2, §8; investigate.md §3, §13 #2).
export const AUTHORS_CLAIM_LABELS = Object.freeze({
  new: `Authors: New ${MODEL_TEXT_SUFFIX}`,
  restated: `Authors: Restated ${MODEL_TEXT_SUFFIX}`,
  background: `Authors: Background ${MODEL_TEXT_SUFFIX}`,
  not_stated: `Authors: Not stated ${MODEL_TEXT_SUFFIX}`,
});

export const AUTHORS_CLAIM_TONES = Object.freeze({
  new: 'info',
  restated: 'neutral',
  background: 'muted',
  not_stated: 'muted',
});

// Rule 6.6: exact cue-absent wording.
export const NO_EXPLICIT_CUE_TEXT = 'no explicit cue in the quoted lines';
// Owner decision (2026-11-25): V14 is deferred; right half-badge is reserved for V16 Explore hand-off.
export const RIGHT_SLOT_V16_LABEL = 'Earlier work: search wider in Explore';
export const RIGHT_SLOT_V16_TITLE = 'Connects to Explore in V16 (prior-work search inside Investigate is deferred).';
// Reader pin toggle labels (investigate.md §4.3).
export const PIN_ACTION_LABEL = 'Pin as main result';
export const UNPIN_ACTION_LABEL = 'Unpin main result';
export const MAIN_RESULT_MODEL_LABEL = `Main result ${MODEL_TEXT_SUFFIX}`;
export const MAIN_RESULT_USER_LABEL = 'Main result (you)';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function normalizeAuthorsClaimLabel(raw) {
  const clean = String(raw || '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  return Object.prototype.hasOwnProperty.call(AUTHORS_CLAIM_LABELS, clean) ? clean : null;
}

export function formatHeadingCiteLine(headCites) {
  if (!Array.isArray(headCites)) return '';
  const keys = [];
  for (const raw of headCites) {
    const clean = String(raw || '').trim();
    if (clean && !keys.includes(clean)) keys.push(clean);
  }
  return keys.length ? `cites [${keys.join(', ')}] in its heading` : '';
}

export function resolveEffectiveMainResult(node) {
  if (!node || typeof node !== 'object') return false;
  if (node.pin === 'not_main_result' || node.unpinned === true) return false;
  if (node.pinned === true || node.pin === 'main_result') return true;
  if (typeof node.effective_is_main_result === 'boolean') return node.effective_is_main_result;
  return String(node.role || '').trim().toLowerCase() === 'main_result';
}

function resolveAuthorsClaimObject(node) {
  if (!node || typeof node !== 'object') return null;
  if (node.authors_claim && typeof node.authors_claim === 'object') {
    const lbl = normalizeAuthorsClaimLabel(node.authors_claim.label);
    if (lbl) return { ...node.authors_claim, label: lbl };
  }
  const foldedCur = node.reading?.authors_claim?.current?.value;
  if (foldedCur && typeof foldedCur === 'object') {
    const lbl = normalizeAuthorsClaimLabel(foldedCur.label);
    if (lbl) return { ...foldedCur, label: lbl };
  }
  return null;
}

function resolveClaimQuotes(claim) {
  if (!claim) return [];
  const list = Array.isArray(claim.quotes) && claim.quotes.length
    ? claim.quotes
    : (Array.isArray(claim.anchors) ? claim.anchors : []);
  return list.filter((q) => q && typeof q === 'object' && String(q.quote || '').trim().length > 0);
}

/**
 * Renders the V7 novelty row (left authors' half-badge + disabled V16 right placeholder),
 * deterministic heading-citation flag, quote + cue popover, and reader pin/unpin toggle.
 */
export function renderInvestigationBadges(host, node, opts = {}) {
  const wrap = el('div', 'inv-v7-badges');
  if (!node || typeof node !== 'object') {
    if (host) host.replaceChildren(wrap);
    return { element: wrap, togglePopover: () => false };
  }

  const claim = resolveAuthorsClaimObject(node);
  const headingCiteLine = (claim?.heading_cite_line && String(claim.heading_cite_line).trim())
    || formatHeadingCiteLine(node.head_cites);
  const isMain = resolveEffectiveMainResult(node);
  const isUserPinned = node.pin === 'main_result' || node.pinned === true;

  // 1. Main-result indicator + reader pin/unpin toggle (investigate.md §4.3).
  const pinRow = el('div', 'inv-v7-pin-row');
  if (isMain) {
    pinRow.appendChild(createBadge({
      text: isUserPinned ? MAIN_RESULT_USER_LABEL : MAIN_RESULT_MODEL_LABEL,
      icon: isUserPinned ? 'pin' : 'model',
      tone: node.stale ? 'muted' : (isUserPinned ? 'ok' : 'info'),
      border: isUserPinned ? BORDER_SOLID : BORDER_DASHED,
    }));
  }

  if (typeof opts.onTogglePin === 'function' && !node.model_node) {
    const nextPin = isMain ? 'not_main_result' : 'main_result';
    const pinBtn = el('button', 'inv-v7-pin-btn', isMain ? UNPIN_ACTION_LABEL : PIN_ACTION_LABEL);
    pinBtn.type = 'button';
    pinBtn.setAttribute('data-pin-action', nextPin);
    pinBtn.setAttribute('aria-pressed', String(isMain));
    pinBtn.addEventListener('click', () => opts.onTogglePin(node, nextPin));
    pinRow.appendChild(pinBtn);
  }
  if (pinRow.children.length > 0) wrap.appendChild(pinRow);

  // 2. Deterministic heading-cite badge by code (never "attributed by the authors to").
  if (headingCiteLine) {
    const headRow = el('div', 'inv-v7-heading-cites');
    headRow.appendChild(createBadge({
      text: headingCiteLine,
      icon: 'code',
      tone: 'neutral',
      border: BORDER_SOLID,
    }));
    wrap.appendChild(headRow);
  }

  // 3. Split novelty row: left authors' half-badge + right disabled V16 slot.
  let popover = null;
  let popoverOpen = Boolean(opts.initialPopoverOpen);

  const togglePopover = (force) => {
    if (!popover) return false;
    popoverOpen = typeof force === 'boolean' ? force : !popoverOpen;
    popover.hidden = !popoverOpen;
    const leftBtn = wrap.querySelector?.('[data-half="left"]');
    if (leftBtn) leftBtn.setAttribute('aria-expanded', String(popoverOpen));
    return popoverOpen;
  };

  if (claim) {
    const labelText = AUTHORS_CLAIM_LABELS[claim.label];
    const tone = AUTHORS_CLAIM_TONES[claim.label] || 'muted';
    const split = createSplitBadge(
      {
        text: labelText,
        icon: 'model',
        tone: node.stale ? 'muted' : tone,
        border: BORDER_DASHED,
        title: 'Show authors-claim quote and cue words',
        onClick: () => togglePopover(),
      },
      {
        text: RIGHT_SLOT_V16_LABEL,
        icon: 'link',
        tone: 'muted',
        border: BORDER_SOLID,
        title: RIGHT_SLOT_V16_TITLE,
      },
    );
    const leftHalf = split.querySelector?.('[data-half="left"]');
    if (leftHalf) leftHalf.setAttribute('aria-expanded', String(popoverOpen));
    const rightHalf = split.querySelector?.('[data-half="right"]');
    if (rightHalf) {
      rightHalf.setAttribute('data-v16-slot', 'prior-work-explore');
      rightHalf.setAttribute('aria-disabled', 'true');
      rightHalf.classList.add('inv-v7-right-placeholder');
    }
    wrap.appendChild(split);

    // 4. Quote + cue popover (Rule 6.6).
    popover = el('section', 'inv-v7-claim-popover');
    popover.hidden = !popoverOpen;
    popover.setAttribute('aria-label', "Authors' claim details");

    const cues = Array.isArray(claim.cues)
      ? claim.cues.map((c) => String(c || '').trim()).filter(Boolean)
      : [];
    const cueText = (claim.cue_line && String(claim.cue_line).trim())
      || (cues.length ? `Cues in quoted lines: ${cues.join(', ')}` : NO_EXPLICIT_CUE_TEXT);
    const cueEl = el('div', 'inv-v7-cue-line', cueText);
    cueEl.setAttribute('data-has-cues', cues.length > 0 ? 'true' : 'false');
    popover.appendChild(cueEl);

    if (claim.relation_note) {
      popover.appendChild(el('div', 'inv-v7-relation-note', formatModelText(claim.relation_note)));
    }
    if (Array.isArray(claim.attributed_to) && claim.attributed_to.length > 0) {
      const attrKeys = claim.attributed_to.map((k) => `[${String(k)}]`).join(', ');
      popover.appendChild(el('div', 'inv-v7-attributed', `References named in claim: ${attrKeys}`));
    }
    if (claim.changed && claim.previous_label) {
      const prevNorm = normalizeAuthorsClaimLabel(claim.previous_label);
      const prevText = prevNorm ? AUTHORS_CLAIM_LABELS[prevNorm] : String(claim.previous_label);
      popover.appendChild(
        el('div', 'inv-v7-claim-diff', `Changed since you last opened this (earlier: ${prevText})`),
      );
    }

    const quotes = resolveClaimQuotes(claim);
    if (quotes.length > 0) {
      const quotesWrap = el('div', 'inv-v7-claim-quotes');
      for (const q of quotes) {
        quotesWrap.appendChild(createQuoteChip({
          quote: q.quote,
          source_id: q.source_id,
          path: q.path,
          start_line: q.start_line,
          end_line: q.end_line,
          page: q.page,
          ocr: node.page_basis === 'scan_text_layer',
        }, {
          onOpen: opts.onOpenQuote,
          onOpenPageImage: opts.onOpenPageImage,
        }));
      }
      popover.appendChild(quotesWrap);
    }

    wrap.appendChild(popover);
  }

  if (host) host.replaceChildren(wrap);
  return { element: wrap, togglePopover };
}
