// Investigate V12 Context Cards UI section (`investigate.md` §3, §4.3, §5.2, §9 V12; F6 provenance).
// Mounts into V11's Context drawer via `mountContextSection('context-cards', ...)` with zero innerHTML
// and matched-quote provenance wording.

import { buildTaskPayload } from './investigation_orient_ui.js';
import { createQuoteChip } from './quote_chip.js';
import { createRunBar, describeRunError, normalizeRunState } from './run_bar.js';
import { BORDER_DASHED, BORDER_SOLID, PROVENANCE_STYLES, createBadge } from './ui_badge.js';

export const PREREQUISITES_FOCUS = 'prerequisites';
export const COMPARE_HYPOTHESES_FOCUS = 'compare_hypotheses';
export const PREREQUISITES_BTN_LABEL = 'Prerequisites';
export const COMPARE_HYPOTHESES_BTN_LABEL = 'Compare hypotheses';
export const HIDE_UNCHECKED_LABEL = 'Hide unchecked';
export const HIDE_UNCHECKED_STORAGE_KEY = 'reSearch.investigate.hideUncheckedContextCards';

export const CARD_TYPE_LABELS = Object.freeze({
  symbol: 'Symbol',
  prerequisite: 'Prerequisite',
  cited_result: 'Cited result',
});

let memoryHideUnchecked = false;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function isModelBackgroundCard(card) {
  return card?.origin === 'model_background' || card?.provenance === 'model_background';
}

export function resolveCardProvenance(card) {
  const origin = String(card?.origin || card?.provenance || '').trim().toLowerCase();
  if (origin === 'paper' || origin === 'from_paper') return PROVENANCE_STYLES.from_paper;
  if (origin === 'external' || origin === 'outside_paper') return PROVENANCE_STYLES.outside_paper;
  return PROVENANCE_STYLES.model_background;
}

export function filterContextCards(cards, { hideUnchecked = false } = {}) {
  const list = Array.isArray(cards) ? cards : [];
  if (!hideUnchecked) return list;
  return list.filter((c) => !isModelBackgroundCard(c));
}

export function selectContextTaskForNode(mv, tag) {
  const cleanTag = String(tag || '').trim();
  if (!cleanTag) return null;
  const tasks = Array.isArray(mv?.tasks) ? mv.tasks : [];
  let best = null;
  for (const t of tasks) {
    if (!t || t.task !== 'context') continue;
    const tags = Array.isArray(t.tags) ? t.tags : [];
    if (!tags.includes(cleanTag)) continue;
    if (!best || Number(t.request_seq || 0) >= Number(best.request_seq || 0)) best = t;
  }
  return best;
}

export function readHideUncheckedPref() {
  try {
    const ls = globalThis?.localStorage;
    if (ls && typeof ls.getItem === 'function') {
      const val = ls.getItem(HIDE_UNCHECKED_STORAGE_KEY);
      if (val === '1' || val === 'true') {
        memoryHideUnchecked = true;
        return true;
      }
      if (val === '0' || val === 'false') {
        memoryHideUnchecked = false;
        return false;
      }
    }
  } catch {
    // Fall back to in-memory preference when storage is blocked.
  }
  return memoryHideUnchecked;
}

export function writeHideUncheckedPref(enabled) {
  const flag = Boolean(enabled);
  memoryHideUnchecked = flag;
  try {
    const ls = globalThis?.localStorage;
    if (ls && typeof ls.setItem === 'function') {
      ls.setItem(HIDE_UNCHECKED_STORAGE_KEY, flag ? '1' : '0');
    }
  } catch {
    // Non-fatal in private browsing or fake DOM tests.
  }
  return flag;
}

function formatFocusLabel(rawFocus) {
  const norm = String(rawFocus || '').trim().toLowerCase().replace(/[-\s]+/g, '_');
  return norm === COMPARE_HYPOTHESES_FOCUS
    ? COMPARE_HYPOTHESES_BTN_LABEL
    : PREREQUISITES_BTN_LABEL;
}

function buildQuoteSpec(anchor, paperTitle) {
  const fallbackPaper = anchor?.source_id ? `Source ${anchor.source_id}` : paperTitle || null;
  return {
    quote: anchor?.quote || '',
    paper: anchor?.paper || fallbackPaper,
    source_id: anchor?.source_id || null,
    path: anchor?.path || null,
    start_line: anchor?.start_line ?? null,
    end_line: anchor?.end_line ?? null,
    page: anchor?.page ?? null,
    end_page: anchor?.end_page ?? null,
    ocr: Boolean(anchor?.ocr),
    page_basis: anchor?.page_basis || null,
  };
}

function renderOneContextCard(card, { paperTitle, onOpenQuote, onOpenPageImage }) {
  const prov = resolveCardProvenance(card);
  const border = prov.border === BORDER_DASHED ? BORDER_DASHED : BORDER_SOLID;
  const article = el(
    'article',
    `inv-ctx-card inv-ctx-card--${prov.key} inv-ctx-card--${border}`,
  );
  article.setAttribute('data-provenance', prov.key);
  article.setAttribute('data-border', border);
  article.setAttribute('data-origin', String(card?.origin || prov.key));
  article.setAttribute('data-card-type', String(card?.type || 'prerequisite'));

  const header = el('div', 'inv-ctx-card-header');
  const headerText = prov.key === 'outside_paper' && (card?.short_ref || card?.bib_key)
    ? `${prov.headerPrefix}: ${card.short_ref || card.bib_key}`
    : prov.headerPrefix;
  header.appendChild(createBadge({
    text: headerText,
    icon: prov.icon,
    tone: prov.tone,
    border,
  }));

  const typeLabel = CARD_TYPE_LABELS[card?.type] || String(card?.type || 'Context');
  header.appendChild(el('span', 'inv-ctx-card-type', typeLabel));
  if (card?.bib_key) {
    header.appendChild(el('code', 'inv-ctx-card-bib', card.bib_key));
  }
  if (card?.stale) {
    header.appendChild(createBadge({
      text: 'Outdated after re-index',
      icon: 'warn',
      tone: 'warn',
      border: BORDER_DASHED,
    }));
  }
  article.appendChild(header);

  article.appendChild(el('h4', 'inv-ctx-card-title', card?.title || 'Context note'));
  if (card?.body) {
    article.appendChild(el('p', 'inv-ctx-card-body', card.body));
  }

  const chipOpts = { onOpen: onOpenQuote, onOpenPageImage };
  const hyps = Array.isArray(card?.hypotheses) ? card.hypotheses : [];
  if (hyps.length > 0) {
    const hypList = el('ul', 'inv-ctx-card-hypotheses');
    for (const hyp of hyps) {
      const li = el('li', 'inv-ctx-card-hypothesis');
      li.appendChild(el('span', 'inv-ctx-card-hyp-text', hyp?.text || ''));
      if (hyp?.anchor?.quote) {
        li.appendChild(createQuoteChip(buildQuoteSpec(hyp.anchor, paperTitle), chipOpts));
      }
      hypList.appendChild(li);
    }
    article.appendChild(hypList);
  }

  const anchors = Array.isArray(card?.anchors) ? card.anchors : [];
  if (anchors.length > 0) {
    const anchorWrap = el('div', 'inv-ctx-card-anchors');
    for (const anchor of anchors) {
      if (!anchor?.quote) continue;
      anchorWrap.appendChild(createQuoteChip(buildQuoteSpec(anchor, paperTitle), chipOpts));
    }
    if (anchorWrap.children.length > 0) article.appendChild(anchorWrap);
  }

  return article;
}

export function renderContextCardsSection(opts = {}) {
  const {
    mv = null,
    selectedTag = null,
    modelConnected = false,
    onStartContextTask = null,
    onStopTask = null,
    onOpenQuote = null,
    onOpenPageImage = null,
  } = opts;

  const nodes = Array.isArray(mv?.nodes) ? mv.nodes : [];
  const selectedNode = selectedTag ? nodes.find((n) => n.id === selectedTag) || null : null;
  const allCards = Array.isArray(selectedNode?.context_cards) ? selectedNode.context_cards : [];
  const paperTitle = mv?.paper?.title || '';

  const root = el('div', 'inv-ctx-cards');
  root.setAttribute('data-selected-tag', selectedTag || '');

  const top = el('div', 'inv-ctx-cards-top');
  const titleWrap = el('div', 'inv-ctx-cards-heading');
  titleWrap.appendChild(el('strong', 'inv-ctx-cards-title', 'Context cards'));
  if (selectedNode?.label) {
    titleWrap.appendChild(el('span', 'inv-ctx-cards-node', selectedNode.label));
  }
  top.appendChild(titleWrap);

  const toggleLabel = el('label', 'inv-ctx-hide-toggle');
  const toggleInput = document.createElement('input');
  toggleInput.type = 'checkbox';
  toggleInput.className = 'inv-ctx-hide-checkbox';
  toggleInput.checked = readHideUncheckedPref();
  toggleInput.setAttribute('aria-label', HIDE_UNCHECKED_LABEL);
  toggleLabel.appendChild(toggleInput);
  toggleLabel.appendChild(el('span', 'inv-ctx-hide-text', HIDE_UNCHECKED_LABEL));
  top.appendChild(toggleLabel);
  root.appendChild(top);

  const controls = el('div', 'inv-ctx-cards-actions');
  const taskObj = selectContextTaskForNode(mv, selectedTag);
  const taskNorm = taskObj ? normalizeRunState(taskObj.run_state || taskObj.stored_status) : null;
  const isRunning = Boolean(taskNorm && (taskNorm.key === 'running' || taskNorm.key === 'waiting'));

  const makeActionBtn = (label, focus) => {
    const btn = el('button', 'btn btn-small inv-ctx-run-btn', label);
    btn.type = 'button';
    btn.setAttribute('data-context-focus', focus);
    const disabled = !selectedTag || !modelConnected || isRunning || typeof onStartContextTask !== 'function';
    btn.disabled = disabled;
    if (!modelConnected) {
      btn.title = 'Connect a model provider to gather context cards.';
    } else if (!selectedTag) {
      btn.title = 'Select a node first.';
    }
    btn.addEventListener('click', () => {
      if (!btn.disabled) onStartContextTask?.(focus, selectedTag);
    });
    return btn;
  };
  controls.appendChild(makeActionBtn(PREREQUISITES_BTN_LABEL, PREREQUISITES_FOCUS));
  controls.appendChild(makeActionBtn(COMPARE_HYPOTHESES_BTN_LABEL, COMPARE_HYPOTHESES_FOCUS));
  root.appendChild(controls);

  if (taskObj) {
    const barHost = el('div', 'inv-ctx-runbar-host');
    const focusText = formatFocusLabel(taskObj.focus);
    const saved = Number(taskObj.saved_items || 0);
    const detail = taskNorm?.key === 'running'
      ? `${focusText} running`
      : `${focusText}: ${saved} saved`;
    const activeFocus = String(taskObj.focus || '').trim().toLowerCase().includes('compare')
      ? COMPARE_HYPOTHESES_FOCUS
      : PREREQUISITES_FOCUS;
    const mc = taskObj.model_calls && typeof taskObj.model_calls === 'object' ? taskObj.model_calls : null;
    const modelCallsUsed = mc
      ? mc.used
      : (typeof taskObj.model_calls === 'number' ? taskObj.model_calls : undefined);
    const modelCallsMax = mc?.cap ?? taskObj.cap;
    const errNote = describeRunError(taskObj.error, taskNorm?.key, taskObj.model_calls);
    createRunBar(barHost, {
      state: taskNorm?.key || 'paused',
      detail,
      modelCallsUsed,
      modelCallsMax,
      extras: errNote ? [errNote] : [],
    }, {
      onStop: typeof onStopTask === 'function' ? () => onStopTask() : undefined,
      onContinue: selectedTag && modelConnected && typeof onStartContextTask === 'function'
        ? () => onStartContextTask(activeFocus, selectedTag)
        : undefined,
      onRetry: selectedTag && modelConnected && typeof onStartContextTask === 'function'
        ? () => onStartContextTask(activeFocus, selectedTag)
        : undefined,
    });
    root.appendChild(barHost);
  }

  const listHost = el('div', 'inv-ctx-cards-list');
  const renderList = () => {
    const hideUnchecked = Boolean(toggleInput.checked);
    const visible = filterContextCards(allCards, { hideUnchecked });
    const hiddenCount = allCards.length - visible.length;
    root.setAttribute('data-hide-unchecked', hideUnchecked ? 'true' : 'false');
    const kids = [];
    if (!selectedTag) {
      kids.push(el('p', 'inv-ctx-cards-empty', 'Select a node on the map to inspect or gather context cards.'));
    } else if (visible.length === 0 && hiddenCount > 0) {
      kids.push(el(
        'p',
        'inv-ctx-cards-empty',
        `${hiddenCount} unchecked model background card${hiddenCount === 1 ? '' : 's'} hidden.`,
      ));
    } else if (visible.length === 0) {
      kids.push(el('p', 'inv-ctx-cards-empty', 'No context cards recorded for this node yet.'));
    } else {
      if (hiddenCount > 0) {
        kids.push(el(
          'p',
          'inv-ctx-cards-hidden-note',
          `${hiddenCount} unchecked model background card${hiddenCount === 1 ? '' : 's'} hidden.`,
        ));
      }
      for (const card of visible) {
        kids.push(renderOneContextCard(card, { paperTitle, onOpenQuote, onOpenPageImage }));
      }
    }
    listHost.replaceChildren(...kids);
  };

  toggleInput.addEventListener('change', () => {
    writeHideUncheckedPref(toggleInput.checked);
    renderList();
  });
  renderList();
  root.appendChild(listHost);
  return root;
}

export function syncContextCardsDrawer(contextDrawer, opts = {}) {
  if (!contextDrawer || typeof contextDrawer.mountContextSection !== 'function') return false;
  const {
    sessionId,
    mv,
    selectedTag,
    modelConnected,
    apiMod,
    hooks,
    refreshMv,
  } = opts;
  const elSection = renderContextCardsSection({
    mv,
    selectedTag,
    modelConnected,
    onStartContextTask: (focus, tag) => {
      if (!sessionId || !tag || !apiMod?.startInvestigationTask) return;
      const payload = buildTaskPayload(hooks, { task: 'context', tags: [tag], focus });
      void apiMod.startInvestigationTask(sessionId, payload)
        ?.then(() => refreshMv?.())
        ?.catch((err) => hooks?.showStatus?.(err?.message || 'Could not start context task.'));
    },
    onStopTask: () => {
      if (!sessionId || !apiMod?.stopTurn) return;
      void apiMod.stopTurn(sessionId)?.then(() => refreshMv?.());
    },
    onOpenQuote: (t) => {
      if (t?.source_id && t?.path) hooks?.openSource?.(sessionId, t);
    },
    onOpenPageImage: (t) => {
      if (t?.source_id) hooks?.openPdf?.(t);
    },
  });
  return contextDrawer.mountContextSection('context-cards', elSection);
}
