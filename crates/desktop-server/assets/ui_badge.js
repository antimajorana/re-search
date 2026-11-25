// Shared badge, evidence-grade, provenance, and node-card shell components (overview 2.6, 2.10, 3.2).
// Every badge is text plus icon, never colour alone. All model/paper/ledger text uses textContent.

const SVG_NS = 'http://www.w3.org/2000/svg';

// Named constants for owner-accepted decisions (overview 0.1, 3.2, 8.1-8.3).
// Decision 8.1 & 8.3 #11: "Verified" is reserved for Solve exact symbolic checks; numeric is "Checked numerically".
export const SOLVE_EXACT_WORD = 'Verified';
export const SOLVE_NUMERIC_WORD = 'Checked numerically';
// Decision 8.1 & 3.2 row 20: quotes are always "matched", never "verified".
export const QUOTE_MATCHED_WORD = 'matched';
// Decision 8.3 #3 & overview 2.10: OCR / scanned-page tag.
export const OCR_TAG_WORD = 'OCR';
// Overview 2.10: abstract-only tag and user-statement tag.
export const ABSTRACT_ONLY_WORD = 'abstract only';
export const USER_TAG_WORD = 'you';
// Overview 3.2 row 24: two patterns for model statements (verdicts vs other model text).
export const MODEL_TEXT_SUFFIX = '(model)';
export const MODEL_VERDICT_PREFIX = 'Model judges:';
export const MODEL_AGREED_LABEL = 'Model agreed';
// Decision 8.2 #4 & 3.2 row 25: dashed always means model-inferred, abstract-only or unchecked.
export const BORDER_SOLID = 'solid';
export const BORDER_DASHED = 'dashed';

export const ALLOWED_TONES = Object.freeze(['ok', 'info', 'warn', 'err', 'muted', 'neutral']);

// Static allowlist of inline SVG icon paths (16x16 viewBox). Never populated from untrusted input.
export const BADGE_ICONS = Object.freeze({
  code: { path: 'M5.5 4L2 8l3.5 4M10.5 4L14 8l-3.5 4', stroke: true },
  check: { path: 'M3 8.5l3.2 3.2L13 4.8', stroke: true },
  numeric: { path: 'M2.5 6.5c1.5-1.5 3.5-1.5 5 0s3.5 1.5 5 0M2.5 10.5c1.5-1.5 3.5-1.5 5 0s3.5 1.5 5 0', stroke: true },
  quote: { path: 'M3.5 5.5h3v3l-1.5 2H3.5v-5zm6 0h3v3l-1.5 2H9.5v-5z', stroke: true },
  ocr: { path: 'M3 3h10v10H3zM5.5 6.5h5M5.5 9.5h3.5', stroke: true },
  abstract: { path: 'M3.5 3h9v10h-9zM5.5 6h5M5.5 9h5', stroke: true, dashed: true },
  model: { path: 'M8 2.2l5.2 5.8L8 13.8 2.8 8z', stroke: true, dashed: true },
  user: { path: 'M8 3.2a2.3 2.3 0 1 1 0 4.6 2.3 2.3 0 0 1 0-4.6zM3.5 13c.7-2.2 2.4-3.2 4.5-3.2s3.8 1 4.5 3.2', stroke: true },
  warn: { path: 'M8 2.5L14 13H2L8 2.5zM8 6.5v3.2M8 11.4v.4', stroke: true },
  error: { path: 'M4 4l8 8M12 4l-8 8', stroke: true },
  running: { path: 'M8 2.5a5.5 5.5 0 1 1-4.8 2.8', stroke: true },
  pause: { path: 'M5.5 3.5v9M10.5 3.5v9', stroke: true },
  pin: { path: 'M5 3h6l-1 4 2 2H4l2-2-1-4zm3 6v4.5', stroke: true },
  link: { path: 'M6 4h6v6M12 4L5 11', stroke: true },
  group: { path: 'M2.5 4h5v4h-5zM8.5 8h5v4h-5zM8.5 4h5v2.5h-5z', stroke: true },
  info: { path: 'M8 2.5a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11zM8 7v4M8 5.2v.3', stroke: true },
});

// Overview 2.10: one evidence scale for display across Investigate, Explore and Solve.
export const EVIDENCE_GRADES = Object.freeze({
  code: Object.freeze({
    grade: 'code',
    border: BORDER_SOLID,
    icon: 'code',
    tone: 'ok',
    defaultText: 'By code',
    tag: null,
  }),
  quote_fulltext: Object.freeze({
    grade: 'quote_fulltext',
    border: BORDER_SOLID,
    icon: 'quote',
    tone: 'ok',
    defaultText: `Quote ${QUOTE_MATCHED_WORD}`,
    tag: null,
  }),
  quote_ocr: Object.freeze({
    grade: 'quote_ocr',
    border: BORDER_SOLID,
    icon: 'ocr',
    tone: 'warn',
    defaultText: `Quote ${QUOTE_MATCHED_WORD} (${OCR_TAG_WORD})`,
    tag: OCR_TAG_WORD,
  }),
  quote_abstract: Object.freeze({
    grade: 'quote_abstract',
    border: BORDER_DASHED,
    icon: 'abstract',
    tone: 'muted',
    defaultText: ABSTRACT_ONLY_WORD,
    tag: ABSTRACT_ONLY_WORD,
  }),
  model: Object.freeze({
    grade: 'model',
    border: BORDER_DASHED,
    icon: 'model',
    tone: 'muted',
    defaultText: `Model ${MODEL_TEXT_SUFFIX}`,
    tag: MODEL_TEXT_SUFFIX,
  }),
  user: Object.freeze({
    grade: 'user',
    border: BORDER_SOLID,
    icon: 'user',
    tone: 'neutral',
    defaultText: USER_TAG_WORD,
    tag: USER_TAG_WORD,
  }),
});

const GRADE_ALIASES = Object.freeze({
  code: 'code',
  by_code: 'code',
  verified: 'code',
  checked_numerically: 'code',
  quote: 'quote_fulltext',
  quote_fulltext: 'quote_fulltext',
  fulltext: 'quote_fulltext',
  cited: 'quote_fulltext',
  evidence: 'quote_fulltext',
  ocr: 'quote_ocr',
  quote_ocr: 'quote_ocr',
  scan: 'quote_ocr',
  scan_text_layer: 'quote_ocr',
  abstract: 'quote_abstract',
  abstract_only: 'quote_abstract',
  quote_abstract: 'quote_abstract',
  model: 'model',
  model_statement: 'model',
  unchecked: 'model',
  user: 'user',
  user_statement: 'user',
  given: 'user',
  you: 'user',
});

export const PROVENANCE_STYLES = Object.freeze({
  from_paper: Object.freeze({
    key: 'from_paper',
    border: BORDER_SOLID,
    icon: 'quote',
    tone: 'neutral',
    headerPrefix: 'From this paper',
  }),
  outside_paper: Object.freeze({
    key: 'outside_paper',
    border: BORDER_SOLID,
    icon: 'link',
    tone: 'info',
    headerPrefix: 'Outside this paper',
  }),
  model_background: Object.freeze({
    key: 'model_background',
    border: BORDER_DASHED,
    icon: 'model',
    tone: 'muted',
    headerPrefix: 'Model background, not checked',
  }),
});

export function describeEvidenceGrade(rawGrade) {
  const key = GRADE_ALIASES[String(rawGrade || '').trim().toLowerCase()] || 'model';
  return { ...EVIDENCE_GRADES[key] };
}

export function normalizeTone(tone, fallback = 'neutral') {
  const clean = String(tone || '').trim().toLowerCase();
  return ALLOWED_TONES.includes(clean) ? clean : fallback;
}

export function resolveIconName(icon, fallback = 'info') {
  const clean = String(icon || '').trim().toLowerCase();
  if (Object.prototype.hasOwnProperty.call(BADGE_ICONS, clean)) return clean;
  return Object.prototype.hasOwnProperty.call(BADGE_ICONS, fallback) ? fallback : 'info';
}

export function formatModelText(text, { isVerdict = false } = {}) {
  const raw = String(text ?? '').trim();
  if (!raw) return isVerdict ? MODEL_VERDICT_PREFIX : MODEL_TEXT_SUFFIX;
  if (isVerdict) {
    if (raw.startsWith(MODEL_VERDICT_PREFIX) || raw.startsWith(MODEL_AGREED_LABEL)) return raw;
    return `${MODEL_VERDICT_PREFIX} ${raw}`;
  }
  if (raw.endsWith(MODEL_TEXT_SUFFIX)) return raw;
  return `${raw} ${MODEL_TEXT_SUFFIX}`;
}

export function createIconSvg(iconName, { size = 14 } = {}) {
  const resolved = resolveIconName(iconName);
  const spec = BADGE_ICONS[resolved];
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('viewBox', '0 0 16 16');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('focusable', 'false');
  svg.setAttribute('class', `ui-icon ui-icon--${resolved}`);
  const path = document.createElementNS(SVG_NS, 'path');
  path.setAttribute('d', spec.path);
  path.setAttribute('fill', 'none');
  path.setAttribute('stroke', 'currentColor');
  path.setAttribute('stroke-width', '1.6');
  path.setAttribute('stroke-linecap', 'round');
  path.setAttribute('stroke-linejoin', 'round');
  if (spec.dashed) path.setAttribute('stroke-dasharray', '2.5 2');
  svg.appendChild(path);
  return svg;
}

export function createBadge(spec = {}) {
  const gradeDesc = spec.grade ? describeEvidenceGrade(spec.grade) : null;
  const iconName = resolveIconName(spec.icon, gradeDesc?.icon || 'info');
  const tone = normalizeTone(spec.tone, gradeDesc?.tone || 'neutral');
  const border = spec.border === BORDER_DASHED || spec.border === BORDER_SOLID
    ? spec.border
    : (gradeDesc?.border || BORDER_SOLID);
  const labelText = String(spec.text ?? '').trim() || gradeDesc?.defaultText || 'Unlabelled';
  const secondaryText = String(spec.secondaryText ?? '').trim();
  const isInteractive = typeof spec.onClick === 'function';

  const el = document.createElement(isInteractive ? 'button' : 'span');
  if (isInteractive) el.type = 'button';
  el.className = `ui-badge ui-badge--${tone} ui-badge--${border}`;
  el.setAttribute('data-tone', tone);
  el.setAttribute('data-icon', iconName);
  el.setAttribute('data-border', border);
  if (gradeDesc) el.setAttribute('data-grade', gradeDesc.grade);
  if (spec.title) el.title = String(spec.title);

  const iconWrap = document.createElement('span');
  iconWrap.className = 'ui-badge-icon';
  iconWrap.setAttribute('aria-hidden', 'true');
  iconWrap.appendChild(createIconSvg(iconName));

  const textEl = document.createElement('span');
  textEl.className = 'ui-badge-text';
  textEl.textContent = labelText;

  el.append(iconWrap, textEl);

  if (secondaryText) {
    const subEl = document.createElement('span');
    subEl.className = 'ui-badge-secondary';
    subEl.textContent = secondaryText;
    el.appendChild(subEl);
  }

  el.setAttribute('aria-label', secondaryText ? `${labelText} — ${secondaryText}` : labelText);
  if (isInteractive) el.addEventListener('click', (ev) => spec.onClick(ev));
  return el;
}

// Investigate novelty row: two half-badges that never merge, each text plus icon.
export function createSplitBadge(leftSpec, rightSpec) {
  const row = document.createElement('div');
  row.className = 'ui-split-badge';
  row.setAttribute('role', 'group');
  row.setAttribute('aria-label', 'Statement provenance and earlier work');
  const left = createBadge(leftSpec || {});
  left.setAttribute('data-half', 'left');
  const right = createBadge(rightSpec || {});
  right.setAttribute('data-half', 'right');
  row.append(left, right);
  return row;
}

function appendBlockBody(container, body) {
  if (body == null) return;
  if (typeof body === 'string') {
    const p = document.createElement('p');
    p.className = 'ui-node-card-text';
    p.textContent = body;
    container.appendChild(p);
    return;
  }
  if (Array.isArray(body)) {
    for (const item of body) appendBlockBody(container, item);
    return;
  }
  if (typeof body === 'object' && body.tagName) {
    container.appendChild(body);
  }
}

// Shared node-card shell (overview 2.6, investigate.md 3): instant code block on top, model block below.
export function createNodeCardShell(spec = {}) {
  const gradeDesc = describeEvidenceGrade(spec.grade || 'code');
  const provKey = Object.prototype.hasOwnProperty.call(PROVENANCE_STYLES, spec.provenance)
    ? spec.provenance
    : null;
  const prov = provKey ? PROVENANCE_STYLES[provKey] : null;
  const border = prov ? prov.border : gradeDesc.border;

  const card = document.createElement('article');
  card.className = `ui-node-card ui-node-card--${border}`;
  card.setAttribute('data-grade', gradeDesc.grade);
  card.setAttribute('data-border', border);
  if (prov) card.setAttribute('data-provenance', prov.key);
  if (spec.stale) card.setAttribute('data-stale', 'true');
  if (spec.dim) card.setAttribute('data-dim', 'true');

  if (prov) {
    const provBar = document.createElement('div');
    provBar.className = `ui-node-card-provenance ui-provenance--${prov.key}`;
    const suffix = spec.provenanceRef ? `: ${String(spec.provenanceRef)}` : '';
    provBar.appendChild(createBadge({
      text: `${prov.headerPrefix}${suffix}`,
      icon: prov.icon,
      tone: spec.stale ? 'muted' : prov.tone,
      border: prov.border,
    }));
    card.appendChild(provBar);
  }

  const headerEl = document.createElement('header');
  headerEl.className = 'ui-node-card-header';
  const titleEl = document.createElement('h3');
  titleEl.className = 'ui-node-card-title';
  titleEl.textContent = String(spec.title ?? spec.label ?? '').trim() || 'Untitled node';
  headerEl.appendChild(titleEl);

  if (spec.tag) {
    const tagEl = document.createElement('code');
    tagEl.className = 'ui-node-card-tag';
    tagEl.textContent = String(spec.tag);
    headerEl.appendChild(tagEl);
  }

  const badgesList = Array.isArray(spec.badges) ? spec.badges : [];
  if (badgesList.length || spec.stale) {
    const badgeBar = document.createElement('div');
    badgeBar.className = 'ui-node-card-badges';
    for (const b of badgesList) {
      badgeBar.appendChild(createBadge({
        ...b,
        tone: spec.stale ? 'muted' : b.tone,
      }));
    }
    if (spec.stale) {
      badgeBar.appendChild(createBadge({ text: 'Stale', icon: 'warn', tone: 'muted' }));
    }
    headerEl.appendChild(badgeBar);
  }

  const flags = Array.isArray(spec.flags) ? spec.flags : [];
  if (flags.length) {
    const flagBar = document.createElement('div');
    flagBar.className = 'ui-node-card-flags';
    for (const flag of flags) {
      const chip = document.createElement('span');
      chip.className = 'ui-node-card-flag';
      chip.textContent = String(flag);
      flagBar.appendChild(chip);
    }
    headerEl.appendChild(flagBar);
  }
  card.appendChild(headerEl);

  // Block A: instant content computed by code (always on top).
  const blockA = document.createElement('section');
  blockA.className = 'ui-node-card-block ui-node-card-block--code';
  blockA.setAttribute('data-block', 'code');
  blockA.setAttribute('data-border', BORDER_SOLID);
  const labelA = document.createElement('div');
  labelA.className = 'ui-node-card-block-label';
  labelA.appendChild(createBadge({
    text: String(spec.codeBlock?.label || 'From source (by code)'),
    icon: 'code',
    tone: 'ok',
    border: BORDER_SOLID,
  }));
  blockA.appendChild(labelA);
  appendBlockBody(blockA, spec.codeBlock?.content ?? spec.codeBlock?.text);
  card.appendChild(blockA);

  // Block B: model explanation below, clearly labelled and dashed.
  const blockB = document.createElement('section');
  blockB.className = 'ui-node-card-block ui-node-card-block--model';
  blockB.setAttribute('data-block', 'model');
  blockB.setAttribute('data-border', BORDER_DASHED);
  const labelB = document.createElement('div');
  labelB.className = 'ui-node-card-block-label';
  labelB.appendChild(createBadge({
    text: String(spec.modelBlock?.label || `Explanation ${MODEL_TEXT_SUFFIX}`),
    icon: 'model',
    tone: 'muted',
    border: BORDER_DASHED,
  }));
  blockB.appendChild(labelB);
  const modelContent = spec.modelBlock?.content ?? spec.modelBlock?.text;
  if (modelContent == null || modelContent === '') {
    appendBlockBody(blockB, String(spec.modelBlock?.emptyText || 'Connect a model to get an explanation'));
  } else {
    appendBlockBody(blockB, modelContent);
  }
  card.appendChild(blockB);

  return card;
}
