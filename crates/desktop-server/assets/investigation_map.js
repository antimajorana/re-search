import { renderMathTextInto } from '/assets/math.js';
import { paperQuestionFocus, paperTopology, renderPaperGraph, resultDependencies, savedFocusMap } from '/assets/investigation_graph.js';

function element(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text) el.textContent = text;
  return el;
}

function math(tag, text, className) {
  const el = element(tag, className);
  renderMathTextInto(el, String(text || ''));
  return el;
}

function button(text, handler, cls = 'pill-btn sm') {
  const el = element('button', cls, text);
  el.type = 'button';
  el.addEventListener('click', handler);
  return el;
}

function foldWithIndexMap(raw) {
  let folded = '', pendingWs = -1;
  const map = [];
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (/\s/.test(ch)) { if (folded.length && pendingWs === -1) pendingWs = i; continue; }
    if (pendingWs !== -1) { folded += ' '; map.push(pendingWs); pendingWs = -1; }
    folded += ch; map.push(i);
  }
  return { folded, map };
}

function isEscaped(text, pos) {
  let bs = 0;
  for (let i = pos - 1; i >= 0 && text[i] === '\\'; i--) bs++;
  return bs % 2 === 1;
}

function mathSpans(text) {
  const spans = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '$' && !isEscaped(text, i)) {
      if (text[i + 1] === '$') {
        for (let j = i + 2; j < text.length; ) {
          const c = text.indexOf('$$', j);
          if (c === -1) break;
          if (!isEscaped(text, c)) { spans.push([i, c + 2]); i = c + 1; break; }
          j = c + 2;
        }
      } else if (text[i + 1] && !/\s/.test(text[i + 1])) {
        for (let j = i + 1; j < text.length && !(text[j] === '\n' && text[j + 1] === '\n'); j++) {
          if (text[j] === '$' && !isEscaped(text, j) && text[j + 1] !== '$' && !/\s/.test(text[j - 1])) {
            spans.push([i, j + 1]); i = j; break;
          }
        }
      }
    } else if (text[i] === '\\' && !isEscaped(text, i) && (text[i + 1] === '(' || text[i + 1] === '[')) {
      const c = text.indexOf(text[i + 1] === '(' ? '\\)' : '\\]', i + 2);
      if (c !== -1) { spans.push([i, c + 2]); i = c + 1; }
    }
  }
  return spans;
}

export function foldSearchSpan(excerpt, quote) {
  const raw = String(excerpt || ''), { folded: needle } = foldWithIndexMap(String(quote || ''));
  if (!needle) return null;
  const { folded, map } = foldWithIndexMap(raw), pos = folded.indexOf(needle);
  if (pos === -1) return null;
  let start = map[pos], end = map[pos + needle.length - 1] + 1;
  for (const [mStart, mEnd] of mathSpans(raw)) {
    if (start > mStart && start < mEnd) start = mStart;
    if (end > mStart && end < mEnd) end = mEnd;
  }
  return [start, end];
}

export function renderAnchorExcerptInto(el, excerpt, quote) {
  const raw = String(excerpt || ''), span = quote ? foldSearchSpan(raw, quote) : null;
  el.replaceChildren();
  if (!span) { renderMathTextInto(el, raw); return; }
  const [pre, mid, post] = [raw.slice(0, span[0]), raw.slice(span[0], span[1]), raw.slice(span[1])];
  if (pre) el.append(math('span', pre));
  el.append(math('mark', mid));
  if (post) el.append(math('span', post));
}

export function visibleMapNodes(nodes, full, query) {
  const term = query.trim().toLowerCase();
  if (term) return nodes.filter(n => [n.sentence, n.kind].some(s => String(s || '').toLowerCase().includes(term)));
  return full ? nodes : nodes.filter(n => n.main || n.kind === 'assumption');
}

export function claimNeighbors(map, id) {
  const nodes = new Map(map.nodes.map(n => [n.id, n]));
  const incoming = [], outgoing = [];
  for (const edge of map.edges || []) {
    if (edge.to === id && nodes.has(edge.from)) incoming.push({ edge, node: nodes.get(edge.from) });
    if (edge.from === id && nodes.has(edge.to)) outgoing.push({ edge, node: nodes.get(edge.to) });
  }
  return { incoming, outgoing };
}

const kindNames = { assumption: 'Assumption', definition: 'Definition', method: 'Method', equation: 'Equation',
  prediction: 'Prediction', measurement: 'Measurement', evidence: 'Evidence', result: 'Result',
  conclusion: 'Conclusion', reference: 'Reference', figure: 'Figure', table: 'Table', passage: 'Passage' };
const nameOf = kind => kindNames[kind] || 'Claim';
export const DEFAULT_PAPER_REQUEST = 'Identify the paper’s main results.';
const readingQuestion = inv => [DEFAULT_PAPER_REQUEST, 'Map the main results and reasoning in this paper.'].includes(inv.question) ? '' : inv.question || '';

export function createPaperMap(host, { openSource, openPdf, citedPaperLinks, act }) {
  let inv = {}, busy = false, full = false, query = '', selected = '', signature = '', generation = 0;
  const forms = new Map();
  const savedFocus = new Map();
  const expanded = new Set();
  let layout = 'graph', highlight = true, disconnectGraph = () => {};
  let indexScroll = 0, actionPending = false, notice = '', formType = '', resultId = '';
  let runEvents = [];
  let summarySignature = '';
  const panel = document.getElementById('investigation-panel');

  function reset(sessionChanged = true) {
    generation++;
    inv = {}; signature = ''; selected = ''; resultId = ''; query = ''; full = false; indexScroll = 0;
    forms.clear(); savedFocus.clear(); formType = ''; actionPending = false; notice = '';
    expanded.clear(); highlight = true; layout = 'graph'; disconnectGraph();
    host.replaceChildren();
    if (sessionChanged) {
      summarySignature = '';
      const summary = document.getElementById('investigation-summary');
      summary.hidden = true; summary.replaceChildren();
      panel.scrollTop = 0;
    }
  }

  function update(data, running, events) {
    runEvents = events || [];
    const next = JSON.stringify([data.map_seq, data.map, data.branches, data.corrections, data.activities, data.question,
      data.page_start, data.page_end, data.original_sha256, data.arxiv_cover_id, running]);
    if (next === signature) return;
    if (inv.map_seq !== data.map_seq) {
      reset(false);
      if (data.map?.nodes?.length && !visibleMapNodes(data.map.nodes, false, '').length) full = true;
    }
    signature = next; inv = data; busy = running;
    if (!inv.map?.nodes?.length) { host.replaceChildren(); return; }
    const focused = document.activeElement;
    const focusedBranch = focused?.name === 'branch-focus' && host.contains(focused);
    const caret = focusedBranch ? [focused.selectionStart, focused.selectionEnd] : null;
    const opened = [...host.querySelectorAll('.investigation-selected > details')].map(el => el.open);
    render();
    host.querySelectorAll('.investigation-selected > details').forEach((el, i) => { if (opened[i]) el.open = true; });
    if (caret) {
      const field = host.querySelector('[name="branch-focus"]');
      if (field && !field.disabled) { field.focus({preventScroll:true}); field.setSelectionRange(...caret); }
    }
  }

  function go(id, focus = true) {
    if (!selected) {
      indexScroll = panel.scrollTop;
      resultId = id; query = ''; full = false; expanded.clear();
    }
    selected = id; formType = ''; notice = '';
    render();
    if (focus) {
      const title = host.querySelector('.investigation-claim-title');
      title?.focus({ preventScroll: true }); title?.scrollIntoView({ block: 'nearest' });
    }
  }

  function card(node) {
    const link = button('', () => go(node.id), 'investigation-node');
    link.dataset.node = node.id;
    link.append(element('span', 'investigation-kind', nameOf(node.kind)), math('span', node.sentence));
    return link;
  }

  function sources(parent, anchors, { openFirst = false } = {}) {
    let first = true;
    for (const anchor of anchors || []) {
      const box = element('details', 'investigation-anchor');
      if (openFirst && first) box.open = true;
      first = false;
      box.dataset.anchor = JSON.stringify([anchor.source_id,anchor.path,anchor.start_line,anchor.end_line]);
      const location = anchor.start_line === anchor.end_line ? 'line ' + anchor.start_line :
        'lines ' + anchor.start_line + '–' + anchor.end_line;
      const title = element('summary', '', [anchor.label || nameOf(anchor.kind), location].join(' · '));
      const quote = element('blockquote');
      renderAnchorExcerptInto(quote, anchor.excerpt, anchor.quote);
      const open = button('Open cited lines', () => openSource(anchor));
      box.append(title, quote, element('small', '', anchor.path));
      if (Array.isArray(anchor.cited_range) && anchor.cited_range.length === 2) {
        const [a, b] = anchor.cited_range;
        const cited = a === b ? String(a) : a + '–' + b;
        box.append(element('small', 'investigation-anchor-note', 'lines corrected by the app (cited ' + cited + ')'));
      }
      if (anchor.excerpt_basis === 'evidence') {
        box.append(element('small', 'investigation-anchor-basis', 'located by reader'));
      }
      box.append(open);
      const original = openPdf?.(anchor, true);
      if (original) box.append(button('Open original PDF page', () => openPdf(anchor, false)));
      if (anchor.kind === 'reference') {
        for (const link of citedPaperLinks?.(anchor.excerpt) || []) {
          const external = element('a', 'pill-btn sm', link.label);
          external.href = link.url;
          external.target = '_blank';
          external.rel = 'noopener noreferrer';
          box.append(external);
        }
      }
      parent.append(box);
    }
  }

  function updateSummary(summary, running, hasPaper, current) {
    const box = document.getElementById('investigation-summary');
    const signature = JSON.stringify([summary, summary ? null : running, hasPaper, current]);
    if (summarySignature === signature) return;
    summarySignature = signature;
    const before = host.getBoundingClientRect().top;
    const preserve = !panel.hidden && panel.scrollTop > 0 && before <= panel.getBoundingClientRect().top;
    box.hidden = !(summary || running || hasPaper);
    box.replaceChildren();
    if (box.hidden) return;
    const heading = element('h2', '', 'What, Why, and How');
    heading.id = 'investigation-summary-title';
    box.append(heading);
    if (summary && current === false) box.append(element('p', 'investigation-meta', 'Saved overview from an earlier reading of this paper.'));
    if (!summary) {
      box.append(element('p', 'investigation-meta', running ? 'Reading the paper for a source-backed overview…' :
        'A source-backed overview is unavailable for this paper.'));
    } else {
      for (const [key, title] of [['what','What'], ['why','Why'], ['how','How']]) {
        const part = summary[key];
        const row = element('section', 'investigation-summary-part');
        row.append(element('h3', '', title));
        const body = element('div');
        if (part?.text && part.anchors?.length) {
          body.append(math('p', part.text));
          sources(body, part.anchors);
        } else {
          body.append(element('p', 'investigation-meta', part?.unstated ? 'The reviewed passages do not state this explicitly.' : 'No source-backed statement is available.'));
        }
        row.append(body); box.append(row);
      }
    }
    if (preserve) panel.scrollTop += host.getBoundingClientRect().top - before;
  }

  function overview() {
    const map = inv.map;
    host.append(math('h2', map.title || 'Paper map'));
    if (inv.original_sha256) {
      const version = element('details', 'investigation-anchor');
      version.append(element('summary', '', inv.arxiv_cover_id ? 'Source version · arXiv:' + inv.arxiv_cover_id : 'Source version'),
        element('small', '', 'Original file SHA-256 · ' + inv.original_sha256));
      host.append(version);
    }
    const question = readingQuestion(inv);
    if (question) host.append(math('p', question, 'investigation-question-text'));
    const modelSelected = Array.isArray(map.answer_node_ids);
    const matches = paperQuestionFocus(map, question);
    if (question && modelSelected && !matches.size) host.append(element('p', 'investigation-meta', 'The model identified no statement that directly answers this reading question.'));
    if (inv.page_start && inv.page_end) host.append(element('p', 'investigation-meta', 'PDF pages ' + inv.page_start + '–' + inv.page_end));
    const summary = element('p', 'investigation-meta', map.coverage === 'paper' ? 'Model reports paper-wide coverage' : 'Partial coverage');
    host.append(summary);
    if (map.limitations?.length) {
      const limitations = element('details', 'investigation-anchor');
      limitations.append(element('summary', '', 'Coverage and limitations'));
      for (const line of map.limitations) limitations.append(math('p', line));
      host.append(limitations);
    }
    const previous = [...(inv.corrections || []), ...(inv.activities || [])].filter(entry => entry.map_seq !== inv.map_seq);
    if (previous.length) {
      const history = element('details', 'investigation-anchor');
      history.append(element('summary', '', 'Earlier map revisions (' + previous.length + ')'));
      for (const entry of previous) {
        const item = element('article', 'investigation-branch');
        item.append(element('strong', '', 'Map ' + entry.map_seq + ' · ' + entry.node_id),
          math('p', entry.text || entry.result || entry.error || entry.status));
        history.append(item);
      }
      host.append(history);
    }
    host.append(element('h3', '', 'Main results'));
    let results = map.nodes.filter(n => n.main && n.kind !== 'assumption');
    if (!results.length) results = map.nodes.filter(n => ['result','conclusion','prediction'].includes(n.kind));
    if (!results.length) { host.append(element('p', '', 'No main result was identified in this map.')); return; }
    const index = element('div', 'investigation-index');
    for (const result of results) {
      const item = card(result);
      if (matches.has(result.id)) item.append(element('small', '', modelSelected ? 'Selected for your question' : 'Suggested-question text match'));
      const built = (inv.branches || []).some(b => b.map_seq === inv.map_seq && b.node_id === result.id && b.status === 'built');
      item.append(element('small', 'investigation-explore', built ? 'Open saved branch →' : 'Explore result →'));
      index.append(item);
    }
    host.append(index);
  }

  function branchGraph(map) {
    const modelSelected = Array.isArray(map.answer_node_ids);
    const matches = paperQuestionFocus(map, readingQuestion(inv));
    host.append(element('h3', '', 'Dependencies for this result'));
    const tools = element('div', 'investigation-map-tools');
    const layouts = element('div', 'segmented-group');
    layouts.setAttribute('role', 'group'); layouts.setAttribute('aria-label', 'Paper map layout');
    for (const [type, label] of [['graph', 'Graph'], ['list', 'Steps in order']]) {
      const choice = button(label, () => {
        layout = type; render(); host.querySelector('[data-layout="' + type + '"]')?.focus();
      }, 'seg-btn' + (layout === type ? ' is-active' : ''));
      choice.dataset.layout = type; choice.setAttribute('aria-pressed', String(layout === type)); layouts.append(choice);
    }
    host.append(layouts);
    const toggle = button(full ? 'Show main results and assumptions' : 'Show all statements', () => {
      full = !full; query = ''; expanded.clear(); render();
      host.querySelector('.investigation-map-tools button')?.focus();
    });
    toggle.setAttribute('aria-pressed', String(full));
    const search = element('input', 'text-input');
    search.type = 'search'; search.placeholder = 'Find a statement'; search.value = query;
    search.setAttribute('aria-label', 'Find a statement in this result’s branch');
    const groups = element('div', 'investigation-index');
    tools.append(search, toggle);
    const collapse = button('Collapse expanded branches', () => {
      expanded.clear(); render(); host.querySelector('.investigation-map-tools input')?.focus();
    });
    tools.append(collapse);
    if (matches.size) {
      const filter = button(modelSelected ? 'Highlight answers' : 'Highlight question text matches', () => {
        highlight = !highlight; render();
        host.querySelector('.investigation-highlight-toggle')?.focus();
      });
      filter.classList.add('investigation-highlight-toggle');
      filter.setAttribute('aria-pressed', String(highlight));
      filter.title = modelSelected ? 'Show the statements the model selected to answer your reading question and their prerequisites.' :
        'Matches words in your reading question against suggested questions on individual statements.';
      tools.append(filter);
    }
    host.append(tools, groups);
    function renderGroups(startingNode) {
      disconnectGraph(); disconnectGraph = () => {};
      groups.replaceChildren();
      const nodes = visibleMapNodes(map.nodes, full, query);
      collapse.hidden = !expanded.size || full || Boolean(query.trim()) || layout !== 'graph';
      if (layout === 'graph' && !query.trim()) {
        const shown = new Set([...nodes.map(n => n.id), ...expanded]);
        if (highlight) for (const id of matches.keys()) shown.add(id);
        disconnectGraph = renderPaperGraph(groups, map, shown, highlight ? matches : new Map(), card, (ids, origin) => {
          for (const id of ids) expanded.add(id);
          renderGroups(origin);
          const selected = [...groups.querySelectorAll('.investigation-node')].find(c => c.dataset.node === origin);
          selected?.focus({ preventScroll:true });
        }, startingNode);
        return;
      }
      const order = paperTopology(map).order;
      const sorted = new Map(nodes.map(n => [n.id, n]));
      const sections = [[query.trim() ? 'Matching statements' : full ? 'All statements in dependency order' : 'Main results and assumptions',
        order.filter(id => sorted.has(id)).map(id => sorted.get(id))]];
      for (const [label, rows] of sections) {
        if (!rows.length) continue;
        groups.append(element('h3', '', label));
        for (const node of rows) groups.append(card(node));
      }
      if (!nodes.length) groups.append(element('p', '', query ? 'No matching statements.' : 'This map has no marked main results or assumptions.'));
    }
    search.addEventListener('input', () => { query = search.value; renderGroups(); });
    renderGroups();
  }

  function relations(title, links, incoming, node, corrections) {
    const section = element('section', 'investigation-links');
    section.append(element('h3', '', title));
    for (const { edge, node: linked } of links) {
      const box = element('article', 'investigation-branch');
      box.append(card(linked));
      const label = edge.basis === 'explicit' ? 'Author-stated connection' : 'Inferred rationale';
      const relation = { depends_on: 'Dependency', supports: 'Support', cites: 'Citation' }[edge.kind];
      const detail = element('details', 'investigation-reason');
      detail.append(element('summary', '', [relation, label].filter(Boolean).join(' · ')), math('p', edge.reason));
      if (edge.anchor) sources(detail, [edge.anchor]);
      const changes = corrections.filter(c => c.node_id === (incoming ? node.id : linked.id) &&
        c.edge_from === (incoming ? linked.id : node.id));
      for (const c of changes) detail.append(element('strong', '', 'Your correction'), math('p', c.text));
      box.append(detail); section.append(box);
    }
    return section;
  }

  function form(node, incoming, kind) {
    const labels = { correction: 'Add correction', check: 'Check claim', recheck: 'Recheck claim',
      implications: 'Explore implications', research: 'Open linked Research' };
    const key = JSON.stringify([inv.map_seq, node.id, kind]);
    const draft = forms.get(key) || { text: '', edge: '' };
    const el = element('form', 'investigation-action-form');
    const question = element('label');
    question.textContent = kind === 'correction' ? 'Your correction' : kind === 'research' ? 'Research question' : 'What should the model examine? (optional)';
    const input = element('textarea', 'text-input');
    input.rows = 3;
    input.required = kind === 'research' || kind === 'correction';
    input.value = draft.text;
    if (kind === 'research' && !forms.has(key)) input.value = 'Under what conditions does this result hold?';
    const updateDraft = () => { draft.text = input.value; forms.set(key, draft); };
    input.addEventListener('input', updateDraft);
    if (['correction','check','recheck'].includes(kind) && incoming.length) {
      const label = element('label', '', 'Applies to');
      const choice = element('select', 'compact-select');
      choice.add(new Option('This statement', ''));
      for (const item of incoming) choice.add(new Option('Connection from ' + item.node.sentence, item.node.id));
      choice.value = draft.edge; choice.addEventListener('change', () => { draft.edge = choice.value; updateDraft(); });
      label.append(choice); el.append(label);
    }
    question.append(input); el.append(question);
    const submit = element('button', 'pill-btn sm primary', kind === 'correction' ? 'Save correction' : labels[kind]);
    submit.type = 'submit'; submit.disabled = actionPending || (busy && kind !== 'correction' && kind !== 'research');
    el.append(submit, button('Cancel', () => { formType = ''; notice = ''; render(); host.querySelector('[data-action="' + kind + '"]')?.focus(); }));
    el.addEventListener('submit', async event => {
      event.preventDefault();
      if (actionPending || submit.disabled) return;
      const text = input.value.trim();
      if (input.required && !text) { input.focus(); return; }
      const epoch = generation;
      actionPending = true; submit.disabled = true; notice = '';
      try {
        await act(kind, { node_id: node.id, map_seq: inv.map_seq }, text, draft.edge);
        if (generation !== epoch) return;
        forms.delete(key);
        if (selected === node.id) {
          formType = '';
          notice = kind === 'correction' ? 'Correction saved beside the original.' :
            kind === 'research' ? 'Linked Research opened.' : 'Request submitted. Its result will appear here.';
        }
      } catch (error) { if (generation === epoch && selected === node.id) notice = error.message; }
      finally { if (generation === epoch) { actionPending = false; render(); } }
    });
    return el;
  }

  function detail(node) {
    const branches = (inv.branches || []).filter(b => b.map_seq === inv.map_seq && b.node_id === resultId);
    const focus = savedFocus.get(resultId);
    const built = focus ? branches.findLast(b => b.status === 'built' && b.question === focus) : branches.findLast(b => b.status === 'built');
    const map = focus && !built ? resultDependencies({ ...inv.map, edges: [] }, resultId) :
      savedFocusMap(inv.map, resultId, inv.map_seq, built?.request_seq, runEvents);
    const { incoming, outgoing } = claimNeighbors(map, node.id);
    const corrections = (inv.corrections || []).filter(c => c.map_seq === inv.map_seq);
    host.append(button('← Main results', () => {
      const old = resultId; selected = ''; resultId = ''; formType = ''; notice = ''; render();
      panel.scrollTop = indexScroll;
      const prior = [...host.querySelectorAll('.investigation-node')].find(c => c.dataset.node === old);
      (prior || host.querySelector('.investigation-map-tools input'))?.focus({ preventScroll: true });
    }));
    if (node.id !== resultId) host.append(button('Back to focused result', () => go(resultId)));
    const main = element('article', 'investigation-selected');
    const heading = math('h2', node.sentence, 'investigation-claim-title');
    heading.tabIndex = -1;
    main.append(element('span', 'investigation-kind', nameOf(node.kind)), heading);
    sources(main, node.anchors, { openFirst: true });
    for (const change of corrections.filter(c => c.node_id === node.id)) {
      const note = element('aside', 'investigation-correction');
      const upstream = change.edge_from && inv.map.nodes.find(n => n.id === change.edge_from);
      note.append(element('strong', '', upstream ? 'Your correction to a connection' : 'Your correction'));
      if (upstream) note.append(math('p', 'From: ' + upstream.sentence));
      note.append(math('p', change.text)); main.append(note);
    }
    host.append(main);
    if (node.id === resultId) developResult(node, map, branches, built);
    if (incoming.length) host.append(relations('What this statement uses', incoming, true, node, corrections));
    if (outgoing.length) host.append(relations('What depends on this statement', outgoing, false, node, corrections));
    if (!incoming.length && !outgoing.length && node.id !== resultId) host.append(element('p', 'investigation-meta', 'No connections are recorded for this statement.'));
    if (node.questions?.length) {
      const questions = element('details', 'investigation-anchor');
      questions.append(element('summary', '', 'Open questions'));
      for (const q of node.questions) questions.append(math('p', q));
      host.append(questions);
    }
    const activity = (inv.activities || []).filter(a => a.node_id === node.id && a.map_seq === inv.map_seq);
    if (activity.length) {
      const section = element('section', 'investigation-activities');
      section.append(element('h3', '', 'Checks and further implications'));
      for (const a of activity) {
        const title = a.action === 'implications' ? 'Further implications' : a.action === 'recheck' ? 'Recheck' : 'Claim check';
        const details = element('details', 'investigation-anchor');
        details.append(element('summary', '', [title, a.status].filter(Boolean).join(' · ')));
        if (a.result || a.text || a.error) details.append(math('p', a.result || a.text || a.error));
        if (a.end_seq) {
          const records = runEvents.filter(e => e.seq > a.seq && e.seq < a.end_seq &&
            ['tool.request', 'tool.result', 'tool.denied', 'proof.verification'].includes(e.kind));
          if (records.length) {
            const evidence = element('details', 'investigation-anchor');
            evidence.append(element('summary', '', 'Run evidence (' + records.length + ')'));
            for (const record of records) {
              const row = element('details', 'investigation-anchor');
              row.append(element('summary', '', '#' + record.seq + ' · ' + [record.kind, record.payload?.tool || record.payload?.tool_id].filter(Boolean).join(' · ')),
                element('pre', '', JSON.stringify(record.payload, null, 2)));
              evidence.append(row);
            }
            details.append(evidence);
          } else if (a.action !== 'implications') {
            details.append(element('p', 'investigation-meta', 'No tool evidence was recorded for this check.'));
          }
        }
        section.append(details);
      }
      host.append(section);
    }
    const actions = element('div', 'investigation-actions');
    const recheck = activity.some(a => a.action === 'check' || a.action === 'recheck') || corrections.some(c => c.node_id === node.id);
    for (const [key, label] of [['correction', 'Add correction'],
      [recheck ? 'recheck' : 'check', recheck ? 'Recheck claim' : 'Check claim'],
      ['implications', 'Explore implications'], ['research', 'Open linked Research']]) {
      const trigger = button(label, () => { formType = formType === key ? '' : key; notice = ''; render(); host.querySelector('.investigation-action-form textarea')?.focus(); });
      trigger.setAttribute('aria-expanded', String(formType === key)); trigger.dataset.action = key;
      trigger.disabled = actionPending || (busy && ['check', 'recheck', 'implications'].includes(key));
      actions.append(trigger);
    }
    host.append(actions);
    if (formType) host.append(form(node, incoming, formType));
    if (notice) {
      const feedback = element('p', 'investigation-notice', notice);
      feedback.setAttribute('role', 'status'); host.append(feedback);
    }
  }

  function developResult(node, map, branches, built) {
    const key = JSON.stringify(['branch', inv.map_seq, node.id]);
    const completed = [...new Set(branches.filter(b => b.status === 'built').map(b => b.question))];
    if (completed.length > 1) {
      const label = element('label', 'investigation-saved-focus', 'Saved focus');
      const choice = element('select', 'compact-select');
      for (const question of completed) choice.add(new Option(question, question));
      choice.value = built?.question || '';
      choice.addEventListener('change', () => {
        savedFocus.set(node.id, choice.value); forms.set(key, choice.value); expanded.clear(); query = ''; render();
        host.querySelector('.investigation-saved-focus select')?.focus({preventScroll:true});
      });
      label.append(choice); host.append(label);
    }
    const form = element('form', 'investigation-action-form investigation-develop');
    const label = element('label', '', 'Focus question');
    const input = element('textarea', 'text-input');
    input.name = 'branch-focus'; input.rows = 2; input.required = true; input.maxLength = 2000;
    const broadDefault = 'What assumptions, methods and evidence support this result?';
    input.value = forms.get(key) ?? built?.question ?? branches.at(-1)?.question ??
      (readingQuestion(inv) || broadDefault);
    input.addEventListener('input', () => forms.set(key, input.value));
    label.append(input);
    const submit = element('button', 'pill-btn sm primary', 'Develop this result');
    submit.type = 'submit'; submit.disabled = busy || actionPending || branches.some(b => b.status === 'running');
    form.append(label, submit); host.append(form);
    for (const b of branches) {
      const details = element('details', 'investigation-anchor');
      const states = { built:'Saved branch', running:'Developing branch', failed:'Branch failed', interrupted:'Interrupted branch' };
      details.append(element('summary', '', (states[b.status] || 'Branch') + ' · ' + (b.question || 'Result dependencies')));
      if (b.result || b.error) details.append(math('p', b.result || b.error));
      for (const note of b.limitations || []) details.append(math('p', note));
      host.append(details);
    }
    if (map.edges.length) branchGraph(map);
    else if (!branches.length) host.append(element('p', 'investigation-meta', 'Develop this result to trace its assumptions, evidence and intermediate claims.'));
    form.addEventListener('submit', async e => {
      e.preventDefault();
      const question = input.value.trim();
      if (!question || submit.disabled || actionPending) return;
      if (completed.includes(question)) {
        savedFocus.set(node.id, question); notice = 'Showing the saved branch for this focus.'; render(); return;
      }
      const epoch = generation;
      forms.set(key, question); actionPending = true; submit.disabled = true;
      try {
        savedFocus.set(node.id, question);
        await act('branch', { map_seq:inv.map_seq, node_id:node.id }, question);
        if (epoch === generation && selected === node.id) notice = 'Branch requested.';
      } catch (error) {
        if (epoch === generation) savedFocus.delete(node.id);
        if (epoch === generation && selected === node.id) notice = error.message;
      } finally {
        if (epoch === generation) { actionPending = false; render(); }
      }
    });
  }

  function render() {
    disconnectGraph(); disconnectGraph = () => {};
    host.replaceChildren();
    if (!inv.map) return;
    const node = selected && inv.map.nodes.find(n => n.id === selected);
    if (node) detail(node); else overview();
  }
  function openNode(mapSeq, id) {
    if (mapSeq !== inv.map_seq || !inv.map?.nodes?.some(n => n.id === id)) return false;
    go(id);
    if (!inv.map.nodes.find(n => n.id === id)?.main) {
      const result = inv.map.nodes.find(n => n.main && resultDependencies(inv.map, n.id).nodes.some(ancestor => ancestor.id === id));
      if (result) { resultId = result.id; render(); }
    }
    return true;
  }
  return { reset, update, updateSummary, openNode, renderAnchors: sources };
}
