// Investigate V3 node card built on F6's shared node-card shell (00-overview.md §2.6, investigate.md §3).
// Block A shows source-backed content by code (solid); Block B holds model explanations (dashed).
import { BORDER_DASHED, BORDER_SOLID, createBadge, createNodeCardShell } from './ui_badge.js';
import { renderMathTextInto } from './math.js';

// Named wording constants (00-overview.md §2.6, §3.2, investigate.md §3).
export const NODE_BLOCK_A_LABEL = 'From source (by code)';
export const NODE_BLOCK_B_LABEL = 'Explanation (model)';
export const NODE_NO_MODEL_TEXT = 'Connect a model to get an explanation';
// Edge role labels: heavy solid = statement uses (code), light solid = proof uses (code), dashed = unchecked.
export const STATEMENT_USES_LABEL = 'Statement uses';
export const PROOF_USES_LABEL = 'Proof uses';
export const USED_BY_LABEL = 'Used by';
export const CITATIONS_LABEL = 'Citations';

// Action bar entries (investigate.md §3).
export const NODE_ACTIONS = Object.freeze([
  Object.freeze({ id: 'source_lines', label: 'Source lines' }),
  Object.freeze({ id: 'pdf_page', label: 'PDF page' }),
  Object.freeze({ id: 'proof', label: 'Proof' }),
  Object.freeze({ id: 'note', label: 'Note' }),
  Object.freeze({ id: 'ask', label: 'Ask' }),
  Object.freeze({ id: 'derive_solve', label: 'Derive in Solve' }),
  Object.freeze({ id: 'extend_explore', label: 'Has this been extended?' }),
]);

const LOADING_STATEMENT_TEXT = 'Loading statement from source...';
const MODEL_CONNECTED_EMPTY_TEXT = 'Open to generate an explanation (model)';

function formatOneSpan(span) {
  if (!span || typeof span !== 'object') return '';
  const path = String(span.path || '').trim();
  const s = Number.isInteger(span.start_line) ? span.start_line : null;
  const e = Number.isInteger(span.end_line) ? span.end_line : s;
  const lines = s != null && s > 0 ? (e != null && e > s ? `${s}-${e}` : `${s}`) : '';
  if (path && lines) return `${path}:${lines}`;
  if (path) return path;
  return lines ? (lines.includes('-') ? `lines ${lines}` : `line ${lines}`) : '';
}

export function formatSpanSummary(spans) {
  if (!Array.isArray(spans) || !spans.length) return '';
  const parts = spans.map(formatOneSpan).filter(Boolean);
  if (!parts.length) return '';
  return parts.length <= 2 ? parts.join(', ') : `${parts[0]} (+${parts.length - 1} spans)`;
}

function normalizeLocators(rawLocators, singleLocator) {
  const out = [];
  const add = (val) => {
    const text = typeof val === 'string' ? val.trim() : String(val?.text ?? '').trim();
    if (text && !out.includes(text)) out.push(text);
  };
  if (singleLocator != null) add(singleLocator);
  if (Array.isArray(rawLocators)) for (const item of rawLocators) add(item);
  return out;
}

function buildBibIndex(rawBib) {
  const index = new Map();
  if (Array.isArray(rawBib)) {
    for (const entry of rawBib) if (entry?.key) index.set(String(entry.key), entry);
  } else if (rawBib && typeof rawBib === 'object') {
    for (const [key, entry] of Object.entries(rawBib)) {
      if (entry) index.set(String(entry.key || key), entry);
    }
  }
  return index;
}

function resolveUseEdge(edge, targetId, nodeById) {
  const target = nodeById.get(targetId);
  const basis = String(edge.basis || 'heuristic');
  return {
    id: targetId,
    label: String(target?.label || target?.title || targetId),
    kind: String(target?.kind || ''),
    basis,
    rule: String(edge.rule || ''),
    dashed: basis !== 'code' && basis !== 'evidence',
  };
}

function dedupeRelations(items) {
  const byId = new Map();
  for (const item of items) {
    const prev = byId.get(item.id);
    if (!prev || (prev.dashed && !item.dashed)) byId.set(item.id, item);
  }
  return [...byId.values()];
}

// Pure helper resolving statement uses, proof uses, reverse uses, citations, and proof node.
export function collectNodeRelations(mapview, tag) {
  const nodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const edges = Array.isArray(mapview?.edges) ? mapview.edges : [];
  const bibIndex = buildBibIndex(mapview?.bib);
  const nodeById = new Map();
  for (const n of nodes) if (n?.id != null) nodeById.set(n.id, n);

  const node = nodeById.get(tag) || null;
  if (!node) {
    return { node: null, statementUses: [], proofUses: [], usedBy: [], citations: [], proofNode: null };
  }

  const rawStatementUses = [];
  const rawProofUses = [];
  const rawUsedBy = [];
  for (const e of edges) {
    if (!e || e.kind !== 'uses') continue;
    if (e.dst === tag && e.role === 'statement' && e.src != null) {
      rawStatementUses.push(resolveUseEdge(e, e.src, nodeById));
    } else if (e.dst === tag && e.role === 'proof' && e.src != null) {
      rawProofUses.push(resolveUseEdge(e, e.src, nodeById));
    } else if (e.src === tag && e.dst != null) {
      rawUsedBy.push(resolveUseEdge(e, e.dst, nodeById));
    }
  }

  const citeMap = new Map();
  const addCite = (rawItem, fromHeading) => {
    const isObj = rawItem != null && typeof rawItem === 'object';
    const key = String(isObj ? (rawItem.key ?? '') : (rawItem ?? '')).trim();
    if (!key) return;
    const bibEntry = bibIndex.get(key) || null;
    const itemLocators = isObj ? normalizeLocators(rawItem.locators, rawItem.locator) : [];
    const bibLocators = bibEntry ? normalizeLocators(bibEntry.locators, bibEntry.locator) : [];
    const locators = itemLocators.length ? itemLocators : bibLocators;
    const inHeading = Boolean(fromHeading || (isObj && (rawItem.inHeading || rawItem.in_heading)));

    const existing = citeMap.get(key);
    if (existing) {
      existing.inHeading = existing.inHeading || inHeading;
      for (const loc of locators) if (!existing.locators.includes(loc)) existing.locators.push(loc);
      return;
    }
    citeMap.set(key, {
      key,
      title: String(bibEntry?.title ?? (isObj ? rawItem.title : '') ?? bibEntry?.raw ?? '').trim(),
      arxiv_id: bibEntry?.arxiv_id ?? (isObj ? rawItem.arxiv_id : null) ?? null,
      doi: bibEntry?.doi ?? (isObj ? rawItem.doi : null) ?? null,
      locators,
      inHeading,
    });
  };

  for (const hc of Array.isArray(node.head_cites) ? node.head_cites : []) addCite(hc, true);
  for (const c of Array.isArray(node.cites) ? node.cites : []) addCite(c, false);

  const proofNode = node.proof_tag ? (nodes.find((n) => n.id === node.proof_tag) || null) : null;
  return {
    node,
    statementUses: dedupeRelations(rawStatementUses),
    proofUses: dedupeRelations(rawProofUses),
    usedBy: dedupeRelations(rawUsedBy),
    citations: [...citeMap.values()],
    proofNode,
  };
}

function createChipRow(rowClass, label) {
  const row = document.createElement('div');
  row.className = `inv-chip-row ${rowClass}`;
  const labelEl = document.createElement('span');
  labelEl.className = 'inv-chip-row-label';
  labelEl.textContent = label;
  const list = document.createElement('div');
  list.className = 'inv-chip-list';
  row.append(labelEl, list);
  return { row, list };
}

function buildNodeChipRow(rowClass, chipRoleClass, label, items, onSelectNode) {
  if (!items.length) return null;
  const { row, list } = createChipRow(rowClass, label);
  for (const item of items) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `inv-chip ${chipRoleClass} ${item.dashed ? 'inv-chip--dashed' : 'inv-chip--solid'}`;
    btn.setAttribute('data-node-id', item.id);
    btn.setAttribute('data-border', item.dashed ? BORDER_DASHED : BORDER_SOLID);
    btn.setAttribute('data-basis', item.basis);
    if (item.rule) btn.setAttribute('data-rule', item.rule);
    btn.textContent = item.label;
    btn.title = [item.label, item.kind, item.dashed ? `${item.basis} (not checked by code)` : `${item.basis} (by code)`]
      .filter(Boolean).join(' · ');
    btn.addEventListener('click', () => onSelectNode?.(item.id));
    list.appendChild(btn);
  }
  return row;
}

function buildCitationsRow(citations, onSelectCitation) {
  if (!citations.length) return null;
  const { row, list } = createChipRow('inv-chip-row--cites', CITATIONS_LABEL);
  for (const cite of citations) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `inv-chip inv-chip--cite inv-chip--solid${cite.inHeading ? ' inv-chip--heading' : ''}`;
    btn.setAttribute('data-cite-key', cite.key);
    btn.setAttribute('data-border', BORDER_SOLID);
    if (cite.inHeading) btn.setAttribute('data-in-heading', 'true');
    btn.textContent = cite.locators.length ? `[${cite.key}, ${cite.locators.join('; ')}]` : `[${cite.key}]`;
    const tip = [
      cite.title,
      cite.arxiv_id ? `arXiv:${cite.arxiv_id}` : '',
      cite.doi ? `DOI:${cite.doi}` : '',
      cite.inHeading ? 'Cited in heading' : '',
    ].filter(Boolean);
    if (tip.length) btn.title = tip.join(' · ');
    btn.addEventListener('click', () => onSelectCitation?.(cite));
    list.appendChild(btn);
  }
  return row;
}

export function renderInvestigationNodeCard(host, mapview, tag, opts = {}) {
  const relations = collectNodeRelations(mapview, tag);
  const { node, statementUses, proofUses, usedBy, citations, proofNode } = relations;
  if (!node) {
    const empty = document.createElement('div');
    empty.className = 'inv-node-empty';
    empty.textContent = tag ? 'Statement not found in map.' : 'Select a statement to inspect.';
    if (host) host.replaceChildren(empty);
    return { card: null, relations, toggleProof: () => false };
  }

  const blockAContent = document.createElement('div');
  blockAContent.className = 'inv-node-code-body';

  // 1. Verbatim statement rendered safely via renderMathTextInto.
  const statementEl = document.createElement('div');
  statementEl.className = 'inv-node-statement';
  if (opts.nodeText == null) {
    statementEl.classList.add('inv-node-statement--loading');
    statementEl.textContent = LOADING_STATEMENT_TEXT;
  } else {
    renderMathTextInto(statementEl, opts.nodeText, { katex: opts.katex });
  }
  blockAContent.appendChild(statementEl);

  // 2. Expandable inline Proof preview when proofText is provided.
  const hasProofText = typeof opts.proofText === 'string' && opts.proofText.length > 0;
  let proofSection = null;
  let proofActionBtn = null;
  if (hasProofText) {
    proofSection = document.createElement('section');
    proofSection.className = 'inv-node-proof';
    proofSection.hidden = true;
    proofSection.setAttribute('aria-label', 'Proof preview');
    const proofHeader = document.createElement('div');
    proofHeader.className = 'inv-node-proof-header';
    proofHeader.appendChild(createBadge({
      text: proofNode?.label || 'Proof',
      secondaryText: formatSpanSummary(proofNode?.spans),
      icon: 'code',
      tone: 'neutral',
      border: BORDER_SOLID,
    }));
    const proofBody = document.createElement('div');
    proofBody.className = 'inv-node-proof-body';
    renderMathTextInto(proofBody, opts.proofText, { katex: opts.katex });
    proofSection.append(proofHeader, proofBody);
    blockAContent.appendChild(proofSection);
  }

  const toggleProof = (force) => {
    if (!proofSection) return false;
    const nextOpen = typeof force === 'boolean' ? force : proofSection.hidden;
    proofSection.hidden = !nextOpen;
    if (proofActionBtn) proofActionBtn.setAttribute('aria-expanded', String(nextOpen));
    return nextOpen;
  };

  // 3-6. Statement uses, Proof uses, Used by, and Citations chip rows.
  const rows = [
    buildNodeChipRow('inv-chip-row--statement', 'inv-chip--statement', STATEMENT_USES_LABEL, statementUses, opts.onSelectNode),
    buildNodeChipRow('inv-chip-row--proof', 'inv-chip--proof', PROOF_USES_LABEL, proofUses, opts.onSelectNode),
    buildNodeChipRow('inv-chip-row--used-by', 'inv-chip--used-by', USED_BY_LABEL, usedBy, opts.onSelectNode),
    buildCitationsRow(citations, opts.onSelectCitation),
  ];
  for (const row of rows) if (row) blockAContent.appendChild(row);

  // 7. Action bar.
  const actionsBar = document.createElement('div');
  actionsBar.className = 'inv-node-actions';
  actionsBar.setAttribute('role', 'toolbar');
  actionsBar.setAttribute('aria-label', 'Statement actions');
  for (const action of NODE_ACTIONS) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'inv-node-action-btn';
    btn.setAttribute('data-action', action.id);
    btn.textContent = action.label;
    if (action.id === 'pdf_page' && !opts.hasPdf) btn.disabled = true;
    if (action.id === 'proof') {
      proofActionBtn = btn;
      btn.disabled = !proofNode && !hasProofText;
      if (proofSection) btn.setAttribute('aria-expanded', 'false');
    }
    btn.addEventListener('click', () => {
      if (action.id === 'proof' && proofSection) toggleProof();
      opts.onAction?.(action.id, node);
    });
    actionsBar.appendChild(btn);
  }
  blockAContent.appendChild(actionsBar);

  const card = createNodeCardShell({
    title: node.label,
    tag: node.id,
    grade: node.grade || 'code',
    provenance: 'from_paper',
    provenanceRef: formatSpanSummary(node.spans),
    badges: node.badges,
    flags: node.flags,
    stale: Boolean(node.stale),
    dim: Boolean(node.dim),
    codeBlock: { label: NODE_BLOCK_A_LABEL, content: blockAContent },
    modelBlock: {
      label: NODE_BLOCK_B_LABEL,
      content: opts.modelContent ?? null,
      emptyText: opts.modelConnected ? MODEL_CONNECTED_EMPTY_TEXT : NODE_NO_MODEL_TEXT,
    },
  });

  if (host) host.replaceChildren(card);
  return { card, relations, toggleProof };
}
