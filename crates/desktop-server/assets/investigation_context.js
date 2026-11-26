// Investigate V11 Context Drawer UI (inv-v11-cited.md Checkpoint 3 & sections 7-8).
// Deterministic cited-paper lookup, budget counter, in-session cited overlay, and V12 section seam.
// Uses textContent and renderMathTextInto exclusively; OCR/non-math text always uses plain textContent.

import { BORDER_SOLID, OCR_TAG_WORD, PROVENANCE_STYLES, createBadge } from './ui_badge.js';
import { createQuoteChip } from './quote_chip.js';
import { formatSpanSummary, renderInvestigationNodeCard } from './investigation_node.js';
import { renderMathTextInto } from './math.js';

// Named constants for owner-accepted decisions (2026-11-25: 00-overview.md 2.10, 8.1-8.2; investigate.md 6.9, 6.16, 6.21, 13 #6/#8).
export const CITED_STATUSES = Object.freeze({
  found: 'found',
  candidates: 'candidates',
  paper_found_locator_not_found: 'paper_found_locator_not_found',
  no_tex: 'no_tex',
  unresolved: 'unresolved',
});
export const OUTSIDE_PAPER_HEADER_PREFIX = PROVENANCE_STYLES.outside_paper.headerPrefix;
export const DEFAULT_FETCH_BUDGET_LIMIT = 8;
export const FETCH_BUDGET_INCREMENT = 8;
export const BUDGET_EXCEEDED_CODE = 'fetch_budget_exceeded';
export const BUDGET_EXCEEDED_TITLE = 'External paper fetch limit reached';
export const NO_TEX_NEEDS_FILE_REASON = 'needs_file';
export const NO_TEX_EMPTY_TEXT_REASON = 'empty_text_layer';
export const INSPECT_IN_CITED_LABEL = 'Inspect in cited paper';
export const OPEN_NEW_SESSION_LABEL = 'Open full paper in new Investigate session';
export const RETURN_TO_CLAIM_LABEL = '← Return to original claim';
export const ATTACH_FILE_LABEL = 'Attach PDF / TeX';
export const RAISE_BUDGET_LABEL = `Allow more (+${FETCH_BUDGET_INCREMENT})`;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function nonNegInt(val, fallback) {
  const n = Number(val);
  return Number.isFinite(n) && n >= 0 ? Math.floor(n) : fallback;
}

export function formatFetchBudgetText(used, limit) {
  return `${nonNegInt(used, 0)} / ${nonNegInt(limit, DEFAULT_FETCH_BUDGET_LIMIT)} external papers fetched`;
}

function formatShortRef(result) {
  const raw = result?.short_ref || result?.bib_key || result?.bib?.key || result?.resolved?.key || 'Cited paper';
  return String(raw).trim() || 'Cited paper';
}

function formatOutsideHeader(result) {
  return `${OUTSIDE_PAPER_HEADER_PREFIX}: ${formatShortRef(result)}`;
}

function renderSafeText(target, text, { allowMath = true, katex = undefined } = {}) {
  const content = String(text ?? '');
  if (!allowMath) { target.textContent = content; return; }
  renderMathTextInto(target, content, { katex });
}

function createOutsideCardShell(result, status) {
  const prov = PROVENANCE_STYLES.outside_paper;
  const card = el('article', 'inv-cited-card inv-cited-card--outside');
  card.setAttribute('data-provenance', prov.key);
  card.setAttribute('data-border', prov.border);
  card.setAttribute('data-status', String(status || ''));
  const header = el('div', 'inv-cited-header');
  header.appendChild(createBadge({
    text: formatOutsideHeader(result),
    icon: prov.icon,
    tone: prov.tone,
    border: prov.border,
  }));
  if (result?.locator) {
    header.appendChild(el('span', 'inv-cited-locator', `Locator: ${String(result.locator)}`));
  }
  card.appendChild(header);
  return card;
}

function appendBibBlock(card, result) {
  const bib = result?.bib && typeof result.bib === 'object' ? result.bib : {};
  const key = String(result?.bib_key ?? bib.key ?? '').trim();
  const title = String(bib.title ?? result?.title ?? '').trim();
  const raw = String(bib.raw ?? result?.bib_raw ?? '').trim();
  const arxivId = String(bib.arxiv_id ?? result?.resolved?.ids?.arxiv ?? '').trim();
  const doi = String(bib.doi ?? result?.resolved?.ids?.doi ?? '').trim();
  if (!key && !title && !raw && !arxivId && !doi) return;
  const box = el('div', 'inv-cited-bib');
  if (key) box.appendChild(el('div', 'inv-cited-bib-key', `[${key}]`));
  if (title) box.appendChild(el('div', 'inv-cited-bib-title', title));
  if (arxivId || doi) {
    const ids = [arxivId ? `arXiv:${arxivId}` : '', doi ? `DOI:${doi}` : ''].filter(Boolean).join(' · ');
    box.appendChild(el('div', 'inv-cited-bib-ids', ids));
  }
  if (raw) box.appendChild(el('p', 'inv-cited-bib-raw', raw));
  card.appendChild(box);
}

function extractBudgetExceeded(errOrRes) {
  if (!errOrRes || typeof errOrRes !== 'object') return null;
  const body = errOrRes.payload && typeof errOrRes.payload === 'object' ? errOrRes.payload : errOrRes;
  if (body.code === BUDGET_EXCEEDED_CODE || errOrRes.code === BUDGET_EXCEEDED_CODE || errOrRes.status === 429) {
    return {
      budget: body.fetch_budget || errOrRes.fetch_budget || null,
      message: body.message || body.error || errOrRes.message || '',
    };
  }
  return null;
}

function appendOpenNewSessionButton(actionsRow, result, opts) {
  if (typeof opts.onOpenNewSession !== 'function') return;
  const btn = el('button', 'inv-node-action-btn inv-cited-new-session-btn', OPEN_NEW_SESSION_LABEL);
  btn.type = 'button';
  btn.setAttribute('data-action', 'open-new-session');
  btn.addEventListener('click', () => opts.onOpenNewSession(result));
  actionsRow.appendChild(btn);
}

function renderErrorCard(slot, message) {
  const card = el('article', 'inv-cited-card inv-cited-card--error');
  card.setAttribute('data-status', 'error');
  card.appendChild(el('p', 'inv-cited-reason', String(message || 'Cited lookup failed.')));
  slot.replaceChildren(card);
  return card;
}

export function createInvestigationContextDrawer(host, opts = {}) {
  const root = el('aside', 'inv-context-drawer');
  root.setAttribute('aria-label', 'Cited result and context drawer');

  const budgetBar = el('div', 'inv-context-budget');
  const budgetText = el('span', 'inv-context-budget-text');
  const raiseBtn = el('button', 'inv-node-action-btn inv-context-raise-btn', RAISE_BUDGET_LABEL);
  raiseBtn.type = 'button';
  raiseBtn.setAttribute('data-action', 'raise-budget');
  budgetBar.append(budgetText, raiseBtn);

  const citedSlot = el('div', 'inv-context-cited-slot');
  citedSlot.setAttribute('data-slot', 'cited');
  const overlaySlot = el('div', 'inv-context-overlay-slot');
  overlaySlot.setAttribute('data-slot', 'cited-overlay');
  overlaySlot.hidden = true;
  const sectionsSlot = el('div', 'inv-context-sections');
  sectionsSlot.setAttribute('data-slot', 'sections');

  root.append(budgetBar, citedSlot, overlaySlot, sectionsSlot);
  if (host) host.replaceChildren(root);

  let activeResult = null;
  let overlayOpen = false;
  let returnRef = null;
  let currentTag = null;
  let lookupToken = 0;
  let inspectToken = 0;
  let raisingBudget = false;
  let showingBudgetExceeded = false;
  const mountedSections = new Map();
  let fetchBudget = {
    used: nonNegInt(opts.fetchBudget?.used, 0),
    limit: nonNegInt(opts.fetchBudget?.limit, DEFAULT_FETCH_BUDGET_LIMIT),
  };

  function setFetchBudget(next = {}) {
    if (!next || typeof next !== 'object') return { ...fetchBudget };
    fetchBudget = {
      used: next.used !== undefined ? nonNegInt(next.used, fetchBudget.used) : fetchBudget.used,
      limit: next.limit !== undefined ? nonNegInt(next.limit, fetchBudget.limit) : fetchBudget.limit,
    };
    budgetText.textContent = formatFetchBudgetText(fetchBudget.used, fetchBudget.limit);
    budgetBar.setAttribute('data-used', String(fetchBudget.used));
    budgetBar.setAttribute('data-limit', String(fetchBudget.limit));
    return { ...fetchBudget };
  }
  setFetchBudget(fetchBudget);

  function setRaiseButtonsDisabled(disabled) {
    raiseBtn.disabled = disabled;
    const inlineBtn = citedSlot.querySelector?.('.inv-context-raise-btn');
    if (inlineBtn) inlineBtn.disabled = disabled;
  }

  async function handleRaiseBudget() {
    if (raisingBudget) return null;
    const newLimit = fetchBudget.limit + FETCH_BUDGET_INCREMENT;
    if (typeof opts.onRaiseBudget !== 'function') {
      setFetchBudget({ used: fetchBudget.used, limit: newLimit });
      if (showingBudgetExceeded) {
        showingBudgetExceeded = false;
        citedSlot.replaceChildren();
      }
      return { fetch_budget: { ...fetchBudget } };
    }
    raisingBudget = true;
    setRaiseButtonsDisabled(true);
    try {
      const res = await opts.onRaiseBudget({ action: 'raise_budget', new_limit: newLimit, tag: currentTag });
      if (!res || typeof res !== 'object') return null;
      const next = res.fetch_budget || ((res.used !== undefined || res.limit !== undefined) ? res : null);
      if (next) {
        setFetchBudget(next);
        if (showingBudgetExceeded) {
          showingBudgetExceeded = false;
          citedSlot.replaceChildren();
        }
      }
      return res;
    } catch (err) {
      renderErrorCard(citedSlot, err?.message || 'Could not raise external paper fetch limit.');
      showingBudgetExceeded = false;
      return null;
    } finally {
      raisingBudget = false;
      setRaiseButtonsDisabled(false);
    }
  }
  raiseBtn.addEventListener('click', () => { void handleRaiseBudget(); });

  function renderBudgetExceeded(exceeded) {
    if (exceeded.budget) setFetchBudget(exceeded.budget);
    showingBudgetExceeded = true;
    const card = el('article', 'inv-cited-card inv-cited-card--outside inv-cited-card--budget');
    card.setAttribute('data-provenance', PROVENANCE_STYLES.outside_paper.key);
    card.setAttribute('data-border', BORDER_SOLID);
    card.setAttribute('data-status', BUDGET_EXCEEDED_CODE);

    const header = el('div', 'inv-cited-header');
    header.appendChild(createBadge({
      text: BUDGET_EXCEEDED_TITLE,
      icon: 'warn',
      tone: 'warn',
      border: BORDER_SOLID,
    }));
    card.appendChild(header);
    card.appendChild(el('p', 'inv-cited-reason', exceeded.message
      ? String(exceeded.message)
      : `${formatFetchBudgetText(fetchBudget.used, fetchBudget.limit)}. Increase the limit to fetch more cited papers.`));

    const actions = el('div', 'inv-node-actions');
    const promptRaiseBtn = el('button', 'inv-node-action-btn inv-context-raise-btn', RAISE_BUDGET_LABEL);
    promptRaiseBtn.type = 'button';
    promptRaiseBtn.setAttribute('data-action', 'raise-budget');
    promptRaiseBtn.disabled = raisingBudget;
    promptRaiseBtn.addEventListener('click', () => { void handleRaiseBudget(); });
    actions.appendChild(promptRaiseBtn);
    card.appendChild(actions);
    citedSlot.replaceChildren(card);
    return card;
  }

  function renderCandidatePicker(card, result) {
    const candidates = Array.isArray(result?.candidates) ? result.candidates : [];
    if (!candidates.length) {
      card.appendChild(el('p', 'inv-cited-empty', 'No candidate statements available in this paper.'));
      return;
    }
    const list = el('div', 'inv-cited-candidates');
    list.setAttribute('role', 'list');
    for (const c of candidates) {
      const btn = el('button', 'inv-cited-candidate-btn');
      btn.type = 'button';
      btn.setAttribute('role', 'listitem');
      btn.setAttribute('data-candidate-tag', String(c?.tag ?? ''));

      const top = el('div', 'inv-cited-candidate-top');
      top.appendChild(el('span', 'inv-cited-candidate-label', String(c?.label || c?.title || c?.tag || 'Candidate')));
      if (c?.start_page != null) {
        const pageLabel = c.end_page != null && c.end_page !== c.start_page
          ? `p. ${c.start_page}–${c.end_page}` : `p. ${c.start_page}`;
        top.appendChild(createBadge({ text: pageLabel, icon: 'info', tone: 'neutral', border: BORDER_SOLID }));
      }
      if (c?.ocr === true) {
        top.appendChild(createBadge({ text: OCR_TAG_WORD, icon: 'ocr', tone: 'warn', border: BORDER_SOLID }));
      }
      btn.appendChild(top);
      const spanText = formatSpanSummary(c?.spans);
      if (spanText) btn.appendChild(el('div', 'inv-cited-candidate-spans', spanText));
      if (c?.excerpt) btn.appendChild(el('p', 'inv-cited-candidate-excerpt', String(c.excerpt)));

      btn.addEventListener('click', async () => {
        if (typeof opts.onPick !== 'function' || btn.disabled) return;
        const tok = ++lookupToken;
        btn.disabled = true;
        try {
          const picked = await opts.onPick({
            action: 'pick',
            tag: result?.tag ?? currentTag,
            bib_key: result?.bib_key ?? '',
            locator: result?.locator ?? '',
            external_source_id: result?.external_source_id ?? null,
            external_tag: c?.tag ?? '',
          });
          if (tok !== lookupToken) return;
          if (picked) renderResult(picked);
        } catch (err) {
          if (tok !== lookupToken) return;
          const exceeded = extractBudgetExceeded(err);
          if (exceeded) renderBudgetExceeded(exceeded);
          else renderErrorCard(citedSlot, err?.message || 'Could not select candidate statement.');
        } finally {
          btn.disabled = false;
        }
      });
      list.appendChild(btn);
    }
    card.appendChild(list);
  }

  function closeCitedOverlay() {
    const preserved = returnRef;
    inspectToken += 1;
    overlayOpen = false;
    overlaySlot.hidden = true;
    overlaySlot.replaceChildren();
    return preserved;
  }

  function openCitedOverlay(result) {
    if (!result || typeof result !== 'object') return null;
    if (result.return_ref !== undefined) returnRef = result.return_ref;
    overlayOpen = true;
    overlaySlot.hidden = false;

    const citedView = result.cited_view && typeof result.cited_view === 'object' ? result.cited_view : {};
    let currentMapview = citedView.mapview || result.mapview || { nodes: [], edges: [], bib: [] };
    let selectedTag = citedView.selected_tag || result.external_tag || result.statement?.tag || null;
    let currentStatement = result.statement && typeof result.statement === 'object' ? result.statement : {};

    const panel = el('section', 'inv-cited-overlay');
    panel.setAttribute('data-provenance', PROVENANCE_STYLES.outside_paper.key);
    panel.setAttribute('aria-label', formatOutsideHeader(result));

    const header = el('div', 'inv-cited-overlay-header');
    header.appendChild(createBadge({
      text: formatOutsideHeader(result),
      icon: PROVENANCE_STYLES.outside_paper.icon,
      tone: PROVENANCE_STYLES.outside_paper.tone,
      border: PROVENANCE_STYLES.outside_paper.border,
    }));
    const qualityText = String(citedView.quality_line || currentMapview?.quality_line || '').trim();
    if (qualityText) header.appendChild(el('div', 'inv-cited-overlay-quality', qualityText));

    const bar = el('div', 'inv-node-actions');
    const returnBtn = el('button', 'inv-node-action-btn inv-cited-return-btn', RETURN_TO_CLAIM_LABEL);
    returnBtn.type = 'button';
    returnBtn.setAttribute('data-action', 'return-to-claim');
    returnBtn.addEventListener('click', () => {
      const ref = closeCitedOverlay();
      opts.onReturnToClaim?.(ref);
    });
    bar.appendChild(returnBtn);
    appendOpenNewSessionButton(bar, result, opts);
    header.appendChild(bar);

    const nodeHost = el('div', 'inv-cited-overlay-node');
    panel.append(header, nodeHost);
    overlaySlot.replaceChildren(panel);

    const mountNode = (tag, stmt) => {
      const isOcr = Boolean(stmt?.ocr ?? result?.ocr);
      const allowMath = !isOcr && stmt?.render_math !== false && result?.render_math !== false;
      renderInvestigationNodeCard(nodeHost, currentMapview, tag, {
        nodeText: stmt?.text ?? '',
        proofText: stmt?.proof_text ?? '',
        renderMath: allowMath,
        katex: opts.katex,
        hasPdf: Boolean(isOcr || citedView?.builder === 'pdf-1'),
        modelConnected: Boolean(opts.modelConnected),
        onSelectNode: async (nextTag) => {
          const tok = ++inspectToken;
          selectedTag = nextTag;
          if (typeof opts.onInspectNode === 'function') {
            try {
              const inspected = await opts.onInspectNode({
                action: 'inspect_node',
                external_source_id: result.external_source_id ?? null,
                external_tag: nextTag,
              });
              if (tok !== inspectToken || !overlayOpen) return;
              if (inspected && typeof inspected === 'object') {
                if (inspected.cited_view?.mapview) currentMapview = inspected.cited_view.mapview;
                else if (inspected.mapview) currentMapview = inspected.mapview;
                currentStatement = (inspected.statement && typeof inspected.statement === 'object')
                  ? inspected.statement
                  : ((inspected.node && typeof inspected.node === 'object') ? inspected.node : inspected);
                selectedTag = inspected.cited_view?.selected_tag || inspected.selected_tag || nextTag;
              }
            } catch (err) {
              if (tok !== inspectToken || !overlayOpen) return;
              renderErrorCard(nodeHost, err?.message || 'Could not inspect cited node.');
              return;
            }
          }
          mountNode(selectedTag, currentStatement);
        },
        onAction: (actionId, node) => {
          const span = Array.isArray(node?.spans) ? node.spans[0] : null;
          if (actionId === 'source_lines' && span && result.external_source_id) {
            opts.onOpenSource?.({
              source_id: result.external_source_id,
              path: span.path,
              start_line: span.start_line,
              end_line: span.end_line,
            }, result);
          } else if (actionId === 'pdf_page' && result.external_source_id) {
            opts.onOpenPdf?.({
              source_id: result.external_source_id,
              path: span?.path ?? null,
              start_line: span?.start_line ?? null,
              page: currentStatement?.start_page ?? null,
            }, result);
          }
        },
      });
    };
    mountNode(selectedTag, currentStatement);
    return panel;
  }

  function renderResult(result) {
    showingBudgetExceeded = false;
    if (!result || typeof result !== 'object') {
      citedSlot.replaceChildren();
      activeResult = null;
      return null;
    }
    const exceeded = extractBudgetExceeded(result);
    if (exceeded) return renderBudgetExceeded(exceeded);

    activeResult = result;
    if (result.tag !== undefined) currentTag = result.tag;
    if (result.return_ref !== undefined) returnRef = result.return_ref;
    if (result.fetch_budget) setFetchBudget(result.fetch_budget);

    const status = String(result.status || CITED_STATUSES.unresolved);
    const card = createOutsideCardShell(result, status);
    if (result.version_caveat) card.appendChild(el('p', 'inv-cited-caveat', String(result.version_caveat)));

    if (status === CITED_STATUSES.found) {
      const stmt = result.statement && typeof result.statement === 'object' ? result.statement : {};
      const stmtText = String(stmt.text ?? '');
      const isOcr = Boolean(stmt.ocr ?? result.ocr);
      const allowMath = !isOcr && stmt.render_math !== false && result.render_math !== false;
      const stmtEl = el('div', 'inv-cited-statement');
      renderSafeText(stmtEl, stmtText, { allowMath, katex: opts.katex });
      card.appendChild(stmtEl);

      const firstSpan = Array.isArray(stmt.spans) ? stmt.spans[0] : null;
      card.appendChild(createQuoteChip({
        quote: stmtText,
        paper: formatShortRef(result),
        source_id: result.external_source_id ?? null,
        path: firstSpan?.path ?? null,
        start_line: firstSpan?.start_line ?? null,
        end_line: firstSpan?.end_line ?? null,
        page: stmt.start_page ?? null,
        ocr: isOcr,
      }, {
        onOpen: typeof opts.onOpenSource === 'function' ? (t, ev) => opts.onOpenSource(t, result, ev) : undefined,
        onOpenPageImage: typeof opts.onOpenPdf === 'function' ? (t, ev) => opts.onOpenPdf(t, result, ev) : undefined,
      }));

      const actions = el('div', 'inv-node-actions');
      const inspectBtn = el('button', 'inv-node-action-btn inv-cited-inspect-btn', INSPECT_IN_CITED_LABEL);
      inspectBtn.type = 'button';
      inspectBtn.setAttribute('data-action', 'inspect-cited');
      inspectBtn.addEventListener('click', () => openCitedOverlay(result));
      actions.appendChild(inspectBtn);
      appendOpenNewSessionButton(actions, result, opts);
      card.appendChild(actions);
    } else if (status === CITED_STATUSES.candidates || status === CITED_STATUSES.paper_found_locator_not_found) {
      const fallbackReason = status === CITED_STATUSES.paper_found_locator_not_found
        ? 'Paper found, but the citation locator did not match a numbered statement. Pick a statement below.'
        : 'Multiple or unconfirmed matches found; pick the intended statement below.';
      card.appendChild(el('p', 'inv-cited-reason', String(result.reason || fallbackReason)));
      renderCandidatePicker(card, result);
      if (typeof opts.onOpenNewSession === 'function') {
        const actions = el('div', 'inv-node-actions');
        appendOpenNewSessionButton(actions, result, opts);
        card.appendChild(actions);
      }
    } else {
      appendBibBlock(card, result);
      const rc = String(result.reason || '').trim();
      const explanation = status === CITED_STATUSES.no_tex
        ? (rc === NO_TEX_NEEDS_FILE_REASON
          ? 'No open TeX or PDF source is available automatically (needs_file). Attach a local PDF or TeX file.'
          : rc === NO_TEX_EMPTY_TEXT_REASON
            ? 'This PDF has an empty text layer (empty_text_layer — scanned pages without OCR). Attach a TeX source or searchable PDF.'
            : (rc || 'No usable TeX or text-layer PDF source was found for this citation.'))
        : (rc || 'No resolvable arXiv ID, DOI, or matching library paper was found for this citation.');
      card.appendChild(el('p', 'inv-cited-reason', explanation));
      const actions = el('div', 'inv-node-actions');
      const attachBtn = el('button', 'inv-node-action-btn inv-cited-attach-btn', ATTACH_FILE_LABEL);
      attachBtn.type = 'button';
      attachBtn.setAttribute('data-action', 'attach-file');
      attachBtn.addEventListener('click', () => opts.onAttachFile?.(result));
      actions.appendChild(attachBtn);
      card.appendChild(actions);
    }

    citedSlot.replaceChildren(card);
    return card;
  }

  async function openLookup({ tag, bibKey, bib_key, locator } = {}) {
    const tok = ++lookupToken;
    const resolvedKey = String(bibKey ?? bib_key ?? '').trim();
    const resolvedLocator = locator != null ? String(locator) : '';
    currentTag = tag ?? currentTag;
    closeCitedOverlay();
    activeResult = null;
    returnRef = null;
    showingBudgetExceeded = false;

    const loading = el('div', 'inv-cited-loading');
    loading.setAttribute('data-status', 'loading');
    loading.textContent = resolvedLocator
      ? `Looking up [${resolvedKey}, ${resolvedLocator}]...`
      : `Looking up [${resolvedKey}]...`;
    citedSlot.replaceChildren(loading);

    if (typeof opts.onLookup !== 'function') return null;
    try {
      const res = await opts.onLookup({
        action: 'lookup',
        tag: tag ?? null,
        bib_key: resolvedKey,
        locator: resolvedLocator,
      });
      if (tok !== lookupToken) return null;
      const exceeded = extractBudgetExceeded(res);
      if (exceeded) return renderBudgetExceeded(exceeded);
      return renderResult(res);
    } catch (err) {
      if (tok !== lookupToken) return null;
      const exceeded = extractBudgetExceeded(err);
      if (exceeded) return renderBudgetExceeded(exceeded);
      return renderErrorCard(citedSlot, err?.message || 'Cited lookup failed.');
    }
  }

  function mountContextSection(id, element) {
    const key = String(id ?? '').trim();
    if (!key) return false;
    if (element == null) return unmountContextSection(key);
    const existing = mountedSections.get(key);
    const wrapper = el('section', 'inv-context-section');
    wrapper.setAttribute('data-section-id', key);
    wrapper.appendChild(element);
    if (existing) {
      const children = [...sectionsSlot.children];
      const idx = children.indexOf(existing);
      if (idx >= 0) children[idx] = wrapper; else children.push(wrapper);
      sectionsSlot.replaceChildren(...children);
    } else {
      sectionsSlot.appendChild(wrapper);
    }
    mountedSections.set(key, wrapper);
    return true;
  }

  function unmountContextSection(id) {
    const key = String(id ?? '').trim();
    const existing = mountedSections.get(key);
    if (!existing) return false;
    mountedSections.delete(key);
    sectionsSlot.replaceChildren(...[...sectionsSlot.children].filter((c) => c !== existing));
    return true;
  }

  return {
    element: root, openCitedLookup: openLookup, renderResult, setFetchBudget,
    openCitedOverlay, closeCitedOverlay, mountContextSection, unmountContextSection,
    getState() {
      return {
        activeResult, overlayOpen, returnRef,
        fetchBudget: { ...fetchBudget },
        mountedSections: [...mountedSections.keys()],
      };
    },
  };
}

export function openCitedLookup(controller, params) {
  if (!controller || typeof controller.openCitedLookup !== 'function') {
    throw new TypeError('openCitedLookup requires a context drawer controller');
  }
  return controller.openCitedLookup(params);
}
