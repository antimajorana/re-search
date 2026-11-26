// Investigate V3 deterministic screen controller (investigate.md §3, §5.2, §8.4, §9 V3).
// Connects the normal Investigation app entry to /investigation/{skeleton,mapview,node-text},
// rendering the quality line, outline tree, shared GraphView, results list, and Block A node card
// with zero model calls (including when no model provider is connected).

import * as api from '/assets/api.js';
import { createGraphView } from '/assets/graph_view.js';
import { renderInvestigationEntry } from '/assets/investigation_entry.js';
import { renderInvestigationNodeCard } from '/assets/investigation_node.js';
import { createQualityBanner, renderInvestigationOutline } from '/assets/investigation_outline.js';
import { RESULT_KINDS, filterResultNodes, renderInvestigationResults } from '/assets/investigation_results.js';
import { lookupNearestSymbol, renderReadingPlanStrip, renderSymbolPopover } from '/assets/investigation_skip.js';

const GRAPH_STATEMENT_KINDS = new Set([
  ...RESULT_KINDS, 'definition', 'assumption', 'equation',
]);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = String(text);
  return node;
}

export function isTexFileName(name) {
  const s = String(name || '').trim().toLowerCase();
  return /\.(tex|ltx|tgz|zip)$/.test(s) || s.endsWith('.tar.gz');
}

export function isTexSource(source) {
  if (!source || typeof source !== 'object') return false;
  if (String(source.format || '').toLowerCase() === 'tex') return true;
  if (/\.(tex|ltx)$/i.test(String(source.primary_path || ''))) return true;
  const files = Array.isArray(source.files) ? source.files : [];
  const assembled = source.assembled_path ? String(source.assembled_path) : null;
  return files.some((rel) => {
    const path = String(rel || '');
    if (assembled && path === assembled && path !== source.primary_path) return false;
    return /\.(tex|ltx)$/i.test(path);
  });
}

export function isSkeletonFileName(name) {
  return isTexFileName(name) || /\.pdf$/i.test(String(name || '').trim());
}

export function isSkeletonSource(source) {
  if (isTexSource(source)) return true;
  if (!source || typeof source !== 'object') return false;
  if (String(source.status || '') === 'empty_text_layer') return false;
  return String(source.format || '').toLowerCase() === 'pdf';
}

function buildFocusedGraphData(mapview, selectedTag, wholeMap) {
  const allNodes = Array.isArray(mapview?.nodes) ? mapview.nodes : [];
  const allEdges = Array.isArray(mapview?.edges) ? mapview.edges : [];
  const keepIds = new Set();
  if (wholeMap) {
    for (const n of allNodes) {
      if (GRAPH_STATEMENT_KINDS.has(String(n.kind || '').toLowerCase())) keepIds.add(n.id);
    }
  } else if (selectedTag) {
    keepIds.add(selectedTag);
    for (const e of allEdges) {
      if (e.kind !== 'uses') continue;
      if (e.dst === selectedTag && e.src) keepIds.add(e.src);
      if (e.src === selectedTag && e.dst) keepIds.add(e.dst);
    }
  }
  if (keepIds.size === 0) {
    for (const n of allNodes) {
      if (RESULT_KINDS.includes(String(n.kind || '').toLowerCase())) keepIds.add(n.id);
    }
  }
  if (keepIds.size === 0) {
    for (const n of allNodes) {
      if (GRAPH_STATEMENT_KINDS.has(String(n.kind || '').toLowerCase())) keepIds.add(n.id);
    }
  }
  const nodes = allNodes.filter((n) => keepIds.has(n.id));
  const edges = allEdges.filter((e) => e.kind === 'uses' && keepIds.has(e.src) && keepIds.has(e.dst));
  return { nodes, edges, legend: Array.isArray(mapview?.legend) ? mapview.legend : [] };
}

function pickInitialNodeTag(mapview) {
  const results = filterResultNodes(mapview?.nodes, 'results');
  if (results.length > 0) return results[0].id;
  const defs = filterResultNodes(mapview?.nodes, 'definitions');
  if (defs.length > 0) return defs[0].id;
  const eqs = filterResultNodes(mapview?.nodes, 'equations');
  if (eqs.length > 0) return eqs[0].id;
  const first = Array.isArray(mapview?.nodes) ? mapview.nodes[0] : null;
  return first?.id || null;
}

export function createInvestigationV3Controller(deps = {}) {
  const apiMod = deps.api || api;
  const bySession = new Map();
  let activeSessionId = null;
  let activeKey = '';
  let loadToken = 0;

  function sessionState(sessionId) {
    if (!bySession.has(sessionId)) {
      bySession.set(sessionId, {
        etag: '',
        mapview: null,
        selectedTag: null,
        trail: [],
        wholeMap: false,
        filter: 'results',
        nodeTexts: new Map(),
      });
    }
    return bySession.get(sessionId);
  }

  async function fetchNodeEntry(sessionId, st, tag) {
    if (!tag || !st.mapview?.skeleton_seq) return null;
    const cacheKey = `${st.mapview.skeleton_seq}:${tag}`;
    if (st.nodeTexts.has(cacheKey)) return st.nodeTexts.get(cacheKey);
    try {
      const data = await apiMod.getInvestigationNodeText(sessionId, tag, st.mapview.skeleton_seq);
      st.nodeTexts.set(cacheKey, data);
      return data;
    } catch {
      return null;
    }
  }

  function renderShell(host, sessionId, sessionData, hooks) {
    const st = sessionState(sessionId);
    const mv = st.mapview;
    if (!mv || mv.skeleton_seq == null) {
      host.hidden = true;
      host.replaceChildren();
      return;
    }
    if (!st.selectedTag || !mv.nodes?.some((n) => n.id === st.selectedTag)) {
      st.selectedTag = pickInitialNodeTag(mv);
      st.trail = st.selectedTag ? [st.selectedTag] : [];
    }

    host.hidden = false;
    const banner = createQualityBanner(mv);
    const leftCol = el('div', 'inv-col inv-col--outline');
    const centerCol = el('div', 'inv-col inv-col--center');
    const rightCol = el('div', 'inv-col inv-col--node');

    const toolbar = el('div', 'inv-node-actions');
    const focusBtn = el('button', 'inv-node-action-btn', 'Focused argument');
    focusBtn.type = 'button';
    focusBtn.setAttribute('aria-pressed', st.wholeMap ? 'false' : 'true');
    const mapBtn = el('button', 'inv-node-action-btn', 'Whole paper graph');
    mapBtn.type = 'button';
    mapBtn.setAttribute('aria-pressed', st.wholeMap ? 'true' : 'false');
    toolbar.append(focusBtn, mapBtn);

    if (st.trail.length > 1) {
      const backBtn = el('button', 'inv-node-action-btn', '← Previous statement');
      backBtn.type = 'button';
      backBtn.addEventListener('click', () => {
        st.trail.pop();
        const prev = st.trail[st.trail.length - 1] || pickInitialNodeTag(mv);
        selectNode(prev, false);
      });
      toolbar.appendChild(backBtn);
    }

    const symInput = el('input', 'inv-symbol-input');
    symInput.type = 'search';
    symInput.placeholder = 'Symbol (e.g. E, \\alpha)';
    symInput.setAttribute('aria-label', 'Find symbol definition');
    const symBtn = el('button', 'inv-node-action-btn', 'Define symbol');
    symBtn.type = 'button';
    toolbar.append(symInput, symBtn);

    const skipHost = el('div', 'inv-skip-stage');
    const symHost = el('div', 'inv-symbol-stage');
    const graphHost = el('div', 'inv-graph-stage');
    const resultsHost = el('div', 'inv-results-stage');
    centerCol.append(toolbar, symHost, skipHost, graphHost, resultsHost);

    const outlineHost = el('div', 'inv-outline-stage');
    leftCol.appendChild(outlineHost);

    const plainTexts = {};
    const prefix = `${mv.skeleton_seq}:`;
    for (const [k, v] of st.nodeTexts.entries()) {
      if (k.startsWith(prefix) && v?.text) plainTexts[k.slice(prefix.length)] = v;
    }

    const selectNode = (tag, recordTrail = true) => {
      if (!tag) return;
      st.selectedTag = tag;
      if (recordTrail && st.trail[st.trail.length - 1] !== tag) {
        st.trail.push(tag);
        if (st.trail.length > 24) st.trail.shift();
      }
      renderShell(host, sessionId, sessionData, hooks);
      void ensureSelectedText(tag);
    };

    const outlineCtrl = renderInvestigationOutline(outlineHost, mv, {
      selectedTag: st.selectedTag,
      onSelectNode: (tag) => selectNode(tag, true),
    });
    const innerBanner = outlineHost.querySelector?.('.inv-quality-line');
    if (innerBanner && outlineHost.children?.length > 1) {
      outlineHost.replaceChildren(...[...outlineHost.children].filter((c) => c !== innerBanner));
    }

    const graphData = buildFocusedGraphData(mv, st.selectedTag, st.wholeMap);
    if (graphData.nodes.length > 0) {
      const gv = createGraphView(graphHost, graphData, {
        onSelect: (id) => selectNode(id, true),
      });
      if (st.selectedTag) gv.selectNode?.(st.selectedTag);
    } else {
      graphHost.hidden = true;
    }

    const resultsCtrl = renderInvestigationResults(resultsHost, mv, {
      filter: st.filter,
      selectedTag: st.selectedTag,
      nodeTexts: plainTexts,
      onFilterChange: (nextFilter) => { st.filter = nextFilter; },
      onSelectNode: (tag) => selectNode(tag, true),
    });

    if (mv.skip_plan) {
      renderReadingPlanStrip(skipHost, mv.skip_plan, {
        dimUnreferenced: st.dimUnreferenced !== false,
        onToggleDim: (enabled) => { st.dimUnreferenced = enabled; },
        onSelectSection: (secTag) => selectNode(secTag, true),
      });
    }

    const runSymbolLookup = () => {
      const q = symInput.value.trim();
      if (!q) return;
      const res = lookupNearestSymbol(mv.symbols || [], mv.nodes || [], q, st.selectedTag, null);
      renderSymbolPopover(symHost, res, {
        onJumpToDefinition: (found) => {
          if (found?.in_node) selectNode(found.in_node, true);
          else if (found?.span && mv.source_id) {
            hooks?.openSource?.(sessionId, {
              source_id: mv.source_id,
              path: found.span.path,
              start_line: found.span.start_line,
              end_line: Math.min(found.span.end_line || found.span.start_line, found.span.start_line + 64),
            });
          }
        },
      });
    };
    symBtn.addEventListener('click', runSymbolLookup);
    symInput.addEventListener('keydown', (ev) => {
      if (ev.key === 'Enter') { ev.preventDefault(); runSymbolLookup(); }
    });

    const modelConnected = Boolean(hooks?.isModelConnected?.());
    const hasPdf = Boolean(
      sessionData?.sources?.some((s) => s.source_id === mv.source_id && s.format === 'pdf'),
    );
    const currentEntry = st.selectedTag
      ? st.nodeTexts.get(`${mv.skeleton_seq}:${st.selectedTag}`) || null
      : null;

    const mountNodeCard = (entry) => {
      renderInvestigationNodeCard(rightCol, mv, st.selectedTag, {
        nodeText: entry ? entry.text : (st.selectedTag ? null : ''),
        proofText: entry?.proof_text || '',
        hasPdf,
        modelConnected,
        onSelectNode: (id) => selectNode(id, true),
        onSelectCitation: (cite) => {
          hooks?.showStatus?.(
            cite.arxiv_id
              ? `Citation [${cite.key}] → arXiv:${cite.arxiv_id} (cited-paper lookup arrives in V11).`
              : `Citation [${cite.key}]${cite.title ? ': ' + cite.title : ''}.`,
          );
        },
        onAction: (actionId, node) => {
          const span = Array.isArray(node?.spans) ? node.spans[0] : null;
          if (actionId === 'source_lines' && span && mv.source_id) {
            hooks?.openSource?.(sessionId, {
              source_id: mv.source_id,
              path: span.path,
              start_line: span.start_line,
              end_line: Math.min(span.end_line || span.start_line, span.start_line + 512),
            });
          } else if (actionId === 'pdf_page' && span && mv.source_id) {
            hooks?.openPdf?.({
              source_id: mv.source_id,
              path: span.path,
              start_line: span.start_line,
              label: node.label,
            });
          } else if (actionId === 'ask' || actionId === 'note') {
            const input = typeof document !== 'undefined' ? document.getElementById('composer-input') : null;
            if (input) {
              if (!input.value) input.value = `Regarding ${node.label} (${node.id}): `;
              input.focus?.();
            }
          } else if (actionId === 'derive_solve' || actionId === 'extend_explore') {
            hooks?.showStatus?.('Cross-mode hand-off buttons from Investigate nodes connect in V16.');
          }
        },
      });
    };
    mountNodeCard(currentEntry);

    focusBtn.addEventListener('click', () => {
      if (!st.wholeMap) return;
      st.wholeMap = false;
      renderShell(host, sessionId, sessionData, hooks);
    });
    mapBtn.addEventListener('click', () => {
      if (st.wholeMap) return;
      st.wholeMap = true;
      renderShell(host, sessionId, sessionData, hooks);
    });

    host.replaceChildren(banner, leftCol, centerCol, rightCol);
    outlineCtrl.setSelected(st.selectedTag);

    async function ensureSelectedText(tag) {
      const entry = await fetchNodeEntry(sessionId, st, tag);
      if (activeSessionId !== sessionId || st.selectedTag !== tag || !entry) return;
      resultsCtrl.setNodeText(tag, entry);
      mountNodeCard(entry);
    }

    if (st.selectedTag && !currentEntry) void ensureSelectedText(st.selectedTag);
    const previewTargets = filterResultNodes(mv.nodes, 'results').slice(0, 16);
    for (const item of previewTargets) {
      if (!st.nodeTexts.has(`${mv.skeleton_seq}:${item.id}`)) {
        void fetchNodeEntry(sessionId, st, item.id).then((entry) => {
          if (activeSessionId === sessionId && entry?.text) resultsCtrl.setNodeText(item.id, entry);
        });
      }
    }
  }

  let entryHost = null;
  let entryCtrl = null;
  let entrySessionId = null;

  function ensureEntryCard(sessionId, show, hooks) {
    if (typeof document === 'undefined') return;
    const setupEl = document.getElementById('investigation-setup');
    if (!setupEl || !setupEl.parentNode) return;
    if (!entryHost) {
      entryHost = el('div', 'inv-v4-entry-host');
      entryHost.id = 'investigation-v4-entry';
      setupEl.parentNode.insertBefore(entryHost, setupEl);
    }
    if (!show) {
      entryHost.hidden = true;
      return;
    }
    entryHost.hidden = false;
    if (entrySessionId === sessionId && entryCtrl) return;
    entrySessionId = sessionId;
    const pollJob = async (jobId) => {
      for (let i = 0; i < 120; i += 1) {
        if (activeSessionId !== sessionId) return;
        const res = await apiMod.getInvestigationOpenStatus(sessionId, jobId);
        const snap = res?.job || res;
        entryCtrl?.setSnapshot(snap);
        if (snap?.metadata) entryCtrl?.setMetadata(snap.metadata);
        if (snap?.status && snap.status !== 'running') {
          entryCtrl?.setBusy(false);
          if (snap.status === 'done') await hooks?.refresh?.(sessionId);
          return;
        }
        await new Promise((r) => setTimeout(r, 250));
      }
      entryCtrl?.setBusy(false);
    };
    const startOpen = async (payload) => {
      entryCtrl?.setBusy(true);
      try {
        const res = await apiMod.openInvestigationPaper(sessionId, payload);
        const snap = res?.job || res;
        entryCtrl?.setSnapshot(snap);
        if (snap?.metadata) entryCtrl?.setMetadata(snap.metadata);
        if (snap?.status === 'running' && (res?.job_id || snap?.job_id)) {
          await pollJob(res.job_id || snap.job_id);
        } else {
          entryCtrl?.setBusy(false);
          if (snap?.status === 'done') await hooks?.refresh?.(sessionId);
        }
      } catch (err) {
        entryCtrl?.setBusy(false);
        entryCtrl?.setSnapshot({ status: 'failed', error: err?.message || 'Could not open paper' });
      }
    };
    entryCtrl = renderInvestigationEntry(entryHost, {
      modelConnected: Boolean(hooks?.isModelConnected?.()),
      onOpenInput: (payload) => { void startOpen(payload); },
      onOpenLibraryKey: (key) => { void startOpen({ library_key: key }); },
      onRequestFile: () => { document.getElementById('investigation-file')?.click?.(); },
    });
    void apiMod.listLibraryPapers?.({ limit: '5' })?.then((res) => {
      if (entrySessionId === sessionId) entryCtrl?.setRecentPapers(res?.papers || res?.items || []);
    }).catch(() => {});
  }

  return {
    isTexFileName,
    isTexSource,
    isSkeletonFileName,
    isSkeletonSource,
    async pinAndOpen(sessionId, sourceId, question) {
      const res = await apiMod.pinInvestigationSkeleton(sessionId, {
        source_id: sourceId,
        pin: true,
        ...(question ? { question } : {}),
      });
      const st = sessionState(sessionId);
      st.etag = '';
      st.mapview = null;
      return res;
    },
    async syncSession(sessionId, sessionData, hooks = {}) {
      const host = typeof document !== 'undefined'
        ? document.getElementById('investigation-v3-shell')
        : null;
      activeSessionId = sessionId || null;
      if (!host) return false;
      if (!sessionId || sessionData?.mode !== 'investigation') {
        activeKey = '';
        ensureEntryCard(sessionId, false, hooks);
        host.hidden = true;
        host.replaceChildren();
        return false;
      }
      const inv = sessionData.investigation || {};
      const pinned = inv.source_id
        ? sessionData.sources?.find((s) => s.source_id === inv.source_id)
        : null;
      const showEntry = !inv.source_id && !inv.map && !(inv.answers?.length);
      ensureEntryCard(sessionId, showEntry, hooks);
      const eligible = inv.skeleton_seq != null || Boolean(inv.skeleton) || isSkeletonSource(pinned);
      if (!eligible) {
        activeKey = '';
        host.hidden = true;
        host.replaceChildren();
        return false;
      }
      const st = sessionState(sessionId);
      const nextKey = `${sessionId}:${inv.skeleton_seq ?? 'auto'}:${inv.source_id ?? ''}`;
      if (st.mapview && activeKey === nextKey) {
        renderShell(host, sessionId, sessionData, hooks);
        return true;
      }
      const token = ++loadToken;
      try {
        const mv = await apiMod.getInvestigationMapview(sessionId);
        if (token !== loadToken || activeSessionId !== sessionId) return false;
        if (!mv || mv.skeleton_seq == null) {
          host.hidden = true;
          host.replaceChildren();
          return false;
        }
        st.mapview = mv;
        st.etag = mv.etag || '';
        activeKey = `${sessionId}:${mv.skeleton_seq}:${mv.source_id ?? ''}`;
        const setupEl = document.getElementById('investigation-setup');
        if (setupEl) setupEl.hidden = true;
        ensureEntryCard(sessionId, false, hooks);
        renderShell(host, sessionId, sessionData, hooks);
        return true;
      } catch (err) {
        if (token === loadToken) {
          host.hidden = true;
          hooks?.showStatus?.(err?.message || 'Could not load paper structure.');
        }
        return false;
      }
    },
  };
}

if (typeof globalThis !== 'undefined') {
  globalThis.__reSearchInvestigationV3 = createInvestigationV3Controller();
}
