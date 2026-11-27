// Investigate V8 Block B model explanation component inside F6 createNodeCardShell
// (00-overview.md §2.6, §3.2, §3.3; investigate.md §3, §4.2-4.3, §13 #18, rules 6.4, 6.12).
//
// Zero innerHTML writes: all untrusted paper/model/ledger text flows through textContent.
// Never shows "verified", never shows "Partial" as a run state, and drops the word "branch".

import {
  BORDER_DASHED,
  BORDER_SOLID,
  MODEL_TEXT_SUFFIX,
  createBadge,
  formatModelText,
} from './ui_badge.js';
import { createRunBar, normalizeRunState } from './run_bar.js';
import { createQuoteChip } from './quote_chip.js';

// Named wording constants (investigate.md §3, §13 #18; 00-overview.md §3.2-3.3).
export const NO_MODEL_EXPLANATION_TEXT = 'Connect a model to get an explanation';
export const OPEN_NODE_ACTION_LABEL = 'Explain this statement';
export const OPEN_STEP_ACTION_LABEL = 'Explain this step';
export const REUSE_STEP_ACTION_LABEL = 'Open saved steps';
export const REOPEN_NODE_ACTION_LABEL = 'Re-run explanation';
export const HYPOTHESES_HEADING = `Hypotheses ${MODEL_TEXT_SUFFIX}`;
export const PROOF_STEPS_HEADING = `Proof steps ${MODEL_TEXT_SUFFIX}`;
export const OPEN_QUESTIONS_HEADING = `Open questions ${MODEL_TEXT_SUFFIX}`;
export const CHANGED_DIFF_HEADING = 'Changed since you last opened this';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

function indexNodes(mapview) {
  const byId = new Map();
  for (const n of Array.isArray(mapview?.nodes) ? mapview.nodes : []) {
    if (n && typeof n === 'object' && n.id) byId.set(String(n.id), n);
  }
  return byId;
}

/**
 * Walks `attach_to` / `parent` links (cycle-guarded) from the target node back to its
 * owning skeleton node, returning the breadcrumb trail `[root, ..., current]`.
 */
export function buildNodeBreadcrumb(mapview, tag) {
  const byId = indexNodes(mapview);
  const start = byId.get(String(tag || ''));
  if (!start) return [];
  const chain = [];
  const seen = new Set();
  let cur = start;
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    chain.push({
      id: cur.id,
      label: String(cur.label || cur.gist || cur.id),
      isModelNode: Boolean(cur.model_node || String(cur.id).startsWith('m-')),
    });
    const parentId = cur.attach_to || (cur.model_node ? cur.parent : null);
    if (!parentId || !byId.has(parentId)) break;
    cur = byId.get(parentId);
  }
  return chain.reverse();
}

/**
 * Returns direct model-created child nodes attached to `parentTag`, split into
 * proof/argument steps vs open-question children.
 */
export function collectModelChildren(mapview, parentTag) {
  const nodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const byId = indexNodes(mapview);
  const pid = String(parentTag || '');
  const steps = [];
  const openQuestions = [];

  for (const n of nodes) {
    if (!n || typeof n !== 'object') continue;
    const isModel = Boolean(n.model_node || String(n.id || '').startsWith('m-'));
    if (!isModel) continue;
    const owner = String(n.attach_to || n.parent || '');
    if (owner !== pid) continue;
    const kind = String(n.model_kind || n.kind || '').trim().toLowerCase();
    const childCount = Array.isArray(n.children)
      ? n.children.length
      : nodes.filter((c) => c && (c.attach_to === n.id || c.parent === n.id) && (c.model_node || String(c.id || '').startsWith('m-'))).length;
    const hasSavedSteps = childCount > 0;
    const entry = {
      id: n.id,
      label: String(n.label || n.gist || n.id),
      kind,
      gist: String(n.gist || n.reading?.gist?.current?.value || '').trim(),
      purpose: String(n.purpose || n.reading?.purpose?.current?.value || '').trim(),
      anchors: extractAnchors(n),
      stale: Boolean(n.stale),
      changed: Boolean(n.changed_since_last),
      childCount,
      hasSavedSteps,
      pageBasis: n.page_basis || byId.get(pid)?.page_basis || null,
    };
    if (kind === 'open_question') openQuestions.push(entry);
    else steps.push(entry);
  }
  return { steps, openQuestions };
}

function extractAnchors(node) {
  if (!node || typeof node !== 'object') return [];
  const direct = Array.isArray(node.anchors) ? node.anchors : null;
  const folded = Array.isArray(node.reading?.anchors?.current?.value)
    ? node.reading.anchors.current.value
    : [];
  const list = direct && direct.length ? direct : folded;
  return list.filter((a) => a && typeof a === 'object' && String(a.quote || '').trim().length > 0);
}

function extractHypotheses(node) {
  if (!node || typeof node !== 'object') return [];
  const direct = Array.isArray(node.hypotheses) ? node.hypotheses : null;
  const folded = Array.isArray(node.reading?.hypotheses?.current?.value)
    ? node.reading.hypotheses.current.value
    : [];
  const list = direct && direct.length ? direct : folded;
  return list.filter((h) => h && typeof h === 'object' && String(h.text || '').trim().length > 0);
}

/**
 * Selects the newest Investigate task (`open` targeting `tag`, or running `orient`) from `mapview.tasks`.
 */
export function findLatestNodeTask(mapview, tag) {
  const tasks = Array.isArray(mapview?.tasks) ? mapview.tasks : [];
  const id = String(tag || '');
  let best = null;
  for (const t of tasks) {
    if (!t || typeof t !== 'object') continue;
    const taskKind = String(t.task || '').trim().toLowerCase();
    const targets = Array.isArray(t.tags) ? t.tags : [];
    if (taskKind === 'open' && targets.includes(id)) {
      if (!best || (t.request_seq || 0) >= (best.request_seq || 0)) best = t;
    }
  }
  return best;
}

function buildRunBarState(task, node) {
  if (!task || typeof task !== 'object') return null;
  const rawState = task.run_state || task.stored_status || 'paused';
  const norm = normalizeRunState(rawState);
  const savedItems = Number(task.saved_items || 0);
  const used = task.model_calls?.used ?? task.modelCallsUsed ?? null;
  const cap = task.model_calls?.cap ?? task.modelCallsMax ?? 10;

  let reason = '';
  if (norm.key === 'paused') {
    if (task.stored_status === 'stopped') {
      reason = savedItems > 0 ? `stopped (${savedItems} saved)` : 'stopped by reader';
    } else if (savedItems > 0) {
      reason = `${savedItems} ${savedItems === 1 ? 'item' : 'items'} saved`;
    } else {
      reason = 'reached model-call cap';
    }
  } else if (norm.key === 'failed') {
    reason = String(task.error || 'task failed').trim();
  }

  return {
    state: norm.key,
    reason,
    detail: task.task === 'open' && node?.label ? `Open ${node.label}` : '',
    modelCallsUsed: used,
    modelCallsMax: cap,
  };
}

function renderQuoteList(anchors, pageBasis, opts) {
  if (!anchors.length) return null;
  const wrap = el('div', 'inv-v8-quotes');
  for (const a of anchors) {
    wrap.appendChild(createQuoteChip({
      quote: a.quote,
      source_id: a.source_id,
      path: a.path,
      start_line: a.start_line,
      end_line: a.end_line,
      page: a.page,
      ocr: Boolean(a.ocr || pageBasis === 'scan_text_layer' || pageBasis === 'ocr'),
    }, {
      onOpen: opts.onOpenQuote,
      onOpenPageImage: opts.onOpenPageImage,
    }));
  }
  return wrap;
}

/**
 * Renders Block B (model explanation, run bar, breadcrumb, hypotheses, proof steps, open questions)
 * for a skeleton `n-` or model-created `m-` node and returns an incremental update controller.
 */
export function renderInvestigationNodeModel(host, initialProps = {}) {
  let props = { ...initialProps };
  const root = el('div', 'inv-v8-model');
  let runBarCtrl = null;

  function render() {
    runBarCtrl?.destroy?.();
    runBarCtrl = null;

    const { mapview, tag, modelConnected = false, streamingText = '' } = props;
    const byId = indexNodes(mapview);
    const node = props.node || byId.get(String(tag || '')) || null;
    const task = props.task || findLatestNodeTask(mapview, node?.id || tag);
    const parts = [];

    if (!node) {
      root.replaceChildren();
      return;
    }

    // 1. Breadcrumb trail when inspecting a nested model step (or when trail has >1 entries).
    const trail = buildNodeBreadcrumb(mapview, node.id);
    if (trail.length > 1) {
      const nav = el('nav', 'inv-v8-breadcrumb');
      nav.setAttribute('aria-label', 'Explanation path');
      trail.forEach((crumb, idx) => {
        const isLast = idx === trail.length - 1;
        if (isLast) {
          const cur = el('span', 'inv-v8-crumb inv-v8-crumb--current', crumb.label);
          cur.setAttribute('aria-current', 'page');
          nav.appendChild(cur);
        } else {
          const btn = el('button', 'inv-v8-crumb', crumb.label);
          btn.type = 'button';
          btn.setAttribute('data-crumb-id', crumb.id);
          btn.addEventListener('click', () => props.onSelectNode?.(crumb.id));
          nav.appendChild(btn);
          nav.appendChild(el('span', 'inv-v8-crumb-sep', '›'));
        }
      });
      parts.push(nav);
    }

    // 2. Shared F6 run bar for active / paused / interrupted / failed / done Open task.
    const barState = buildRunBarState(task, node);
    if (barState) {
      const barHost = el('div', 'inv-v8-runbar-host');
      runBarCtrl = createRunBar(barHost, barState, {
        onStop: props.onStopTask ? () => props.onStopTask(task, node) : undefined,
        onContinue: props.onContinueTask ? () => props.onContinueTask(task, node) : undefined,
        onRetry: props.onRetryTask ? () => props.onRetryTask(task, node) : undefined,
      });
      parts.push(barHost);
    }

    // 3. Streaming live output while a task is actively running.
    const trimmedStream = String(streamingText || '').trim();
    if (trimmedStream) {
      const streamBox = el('section', 'inv-v8-streaming');
      streamBox.setAttribute('aria-live', 'polite');
      streamBox.appendChild(createBadge({
        text: `Streaming ${MODEL_TEXT_SUFFIX}`,
        icon: 'running',
        tone: 'info',
        border: BORDER_DASHED,
      }));
      streamBox.appendChild(el('p', 'inv-v8-stream-text', formatModelText(trimmedStream)));
      parts.push(streamBox);
    }

    const gist = String(node.gist || node.reading?.gist?.current?.value || '').trim();
    const purpose = String(node.purpose || node.reading?.purpose?.current?.value || '').trim();
    const hypotheses = extractHypotheses(node);
    const { steps, openQuestions } = collectModelChildren(mapview, node.id);
    const hasSavedContent = Boolean(
      gist || purpose || hypotheses.length || steps.length || openQuestions.length,
    );

    // 4. Additive re-run diff indicator (rule 6.12).
    const diffChanges = Array.isArray(node.changed_since_last?.changes)
      ? node.changed_since_last.changes
      : [];
    if (diffChanges.length > 0) {
      const diffBox = el('section', 'inv-v8-diff');
      diffBox.appendChild(createBadge({
        text: String(node.changed_since_last.label || CHANGED_DIFF_HEADING),
        icon: 'warn',
        tone: 'warn',
        border: BORDER_DASHED,
      }));
      const list = el('ul', 'inv-v8-diff-list');
      for (const ch of diffChanges) {
        const li = el(
          'li',
          'inv-v8-diff-item',
          `${String(ch.field || 'field')}: earlier "${String(ch.previous ?? '')}" → now "${String(ch.current ?? '')}"`,
        );
        list.appendChild(li);
      }
      diffBox.appendChild(list);
      parts.push(diffBox);
    }

    // 5. Gist & purpose (always suffixed with "(model)").
    if (gist) {
      parts.push(el('p', 'inv-v8-gist', formatModelText(gist)));
    }
    if (purpose) {
      parts.push(el('p', 'inv-v8-purpose', formatModelText(purpose)));
    }

    // 6. Hypotheses checklist with F6 quote chips.
    if (hypotheses.length > 0) {
      const sec = el('section', 'inv-v8-hypotheses');
      sec.appendChild(el('h4', 'inv-v8-section-title', HYPOTHESES_HEADING));
      const list = el('ul', 'inv-v8-hyp-list');
      for (const h of hypotheses) {
        const li = el('li', 'inv-v8-hyp-item');
        li.appendChild(el('div', 'inv-v8-hyp-text', formatModelText(h.text)));
        if (h.anchor && h.anchor.quote) {
          li.appendChild(createQuoteChip({
            quote: h.anchor.quote,
            source_id: h.anchor.source_id,
            path: h.anchor.path,
            start_line: h.anchor.start_line,
            end_line: h.anchor.end_line,
            page: h.anchor.page,
            ocr: Boolean(h.anchor.ocr || node.page_basis === 'scan_text_layer' || node.page_basis === 'ocr'),
          }, {
            onOpen: props.onOpenQuote,
            onOpenPageImage: props.onOpenPageImage,
          }));
        }
        list.appendChild(li);
      }
      sec.appendChild(list);
      parts.push(sec);
    }

    // 7. Proof steps as expandable child nodes (only the clicked child opens; saved steps reused).
    if (steps.length > 0) {
      const sec = el('section', 'inv-v8-steps');
      sec.appendChild(el('h4', 'inv-v8-section-title', PROOF_STEPS_HEADING));
      const list = el('ol', 'inv-v8-step-list');
      for (const step of steps) {
        const li = el('li', 'inv-v8-step-item');
        li.setAttribute('data-model-node-id', step.id);
        const header = el('div', 'inv-v8-step-header');
        header.appendChild(createBadge({
          text: step.kind ? `${step.kind.replace(/_/g, ' ')} ${MODEL_TEXT_SUFFIX}` : `step ${MODEL_TEXT_SUFFIX}`,
          icon: 'model',
          tone: step.stale ? 'warn' : 'muted',
          border: BORDER_DASHED,
        }));
        const stepBtn = el(
          'button',
          'inv-v8-step-open-btn',
          step.hasSavedSteps
            ? `${REUSE_STEP_ACTION_LABEL} (${step.childCount})`
            : OPEN_STEP_ACTION_LABEL,
        );
        stepBtn.type = 'button';
        stepBtn.setAttribute('data-step-action', step.hasSavedSteps ? 'reuse' : 'open');
        stepBtn.setAttribute('data-step-id', step.id);
        if (!step.hasSavedSteps && !modelConnected) {
          stepBtn.disabled = true;
          stepBtn.title = NO_MODEL_EXPLANATION_TEXT;
        }
        stepBtn.addEventListener('click', () => {
          if (step.hasSavedSteps) {
            props.onSelectNode?.(step.id, { reuseSaved: true });
          } else {
            props.onOpenNode?.(step.id, { parentId: node.id });
          }
        });
        header.appendChild(stepBtn);
        li.appendChild(header);

        if (step.gist) li.appendChild(el('p', 'inv-v8-step-gist', formatModelText(step.gist)));
        const qList = renderQuoteList(step.anchors, step.pageBasis, props);
        if (qList) li.appendChild(qList);
        list.appendChild(li);
      }
      sec.appendChild(list);
      parts.push(sec);
    }

    // 8. Open questions.
    if (openQuestions.length > 0) {
      const sec = el('section', 'inv-v8-open-questions');
      sec.appendChild(el('h4', 'inv-v8-section-title', OPEN_QUESTIONS_HEADING));
      const list = el('ul', 'inv-v8-oq-list');
      for (const q of openQuestions) {
        const li = el('li', 'inv-v8-oq-item', formatModelText(q.gist || q.label));
        list.appendChild(li);
      }
      sec.appendChild(list);
      parts.push(sec);
    }

    // 9. Empty / action footer (unconnected vs ready to explain vs re-run).
    const isRunning = barState?.state === 'running' || barState?.state === 'waiting';
    if (!hasSavedContent && !trimmedStream && !isRunning) {
      if (!modelConnected) {
        parts.push(el('p', 'inv-v8-no-model', NO_MODEL_EXPLANATION_TEXT));
      } else if (typeof props.onOpenNode === 'function') {
        const isModelNode = Boolean(node.model_node || String(node.id).startsWith('m-'));
        const btn = el(
          'button',
          'inv-v8-open-btn',
          isModelNode ? OPEN_STEP_ACTION_LABEL : OPEN_NODE_ACTION_LABEL,
        );
        btn.type = 'button';
        btn.setAttribute('data-open-node', node.id);
        btn.addEventListener('click', () => props.onOpenNode(node.id));
        parts.push(btn);
      }
    } else if (hasSavedContent && modelConnected && !isRunning && typeof props.onOpenNode === 'function') {
      const rerunBtn = el('button', 'inv-v8-rerun-btn', REOPEN_NODE_ACTION_LABEL);
      rerunBtn.type = 'button';
      rerunBtn.setAttribute('data-rerun-node', node.id);
      rerunBtn.addEventListener('click', () => props.onOpenNode(node.id));
      parts.push(rerunBtn);
    }

    // Preserve a solid indicator tag for matched model anchors if present on the node itself.
    const ownAnchors = extractAnchors(node);
    if (ownAnchors.length > 0 && (node.model_node || String(node.id).startsWith('m-'))) {
      const qList = renderQuoteList(ownAnchors, node.page_basis, props);
      if (qList) parts.push(qList);
    }

    root.replaceChildren(...parts);
  }

  render();
  if (host) host.replaceChildren(root);

  return {
    element: root,
    update(nextProps = {}) {
      props = { ...props, ...nextProps };
      render();
      return root;
    },
    destroy() {
      runBarCtrl?.destroy?.();
      runBarCtrl = null;
      if (host) host.replaceChildren();
    },
  };
}
