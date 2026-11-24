import { renderMathTextInto } from '/assets/math.js';
import { paperQuestionFocus, paperTopology, renderPaperGraph } from '/assets/investigation_graph.js';

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

export function createPaperMap(host, { openSource, openPdf, act }) {
  let inv = {}, busy = false, full = false, query = '', selected = '', signature = '', generation = 0;
  const forms = new Map();
  const expanded = new Set();
  let layout = 'graph', highlight = true, disconnectGraph = () => {};
  let indexScroll = 0, actionPending = false, notice = '', formType = '';
  let runEvents = [];
  let summarySignature = '';
  const panel = document.getElementById('investigation-panel');

  function reset(sessionChanged = true) {
    generation++;
    inv = {}; signature = ''; selected = ''; query = ''; full = false; indexScroll = 0;
    forms.clear(); formType = ''; actionPending = false; notice = '';
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
    const next = JSON.stringify([data.map_seq, data.map, data.corrections, data.activities, data.question,
      data.page_start, data.page_end, data.original_sha256, data.arxiv_cover_id, running]);
    if (next === signature) return;
    if (inv.map_seq !== data.map_seq) {
      reset(false);
      if (data.map?.nodes?.length && !visibleMapNodes(data.map.nodes, false, '').length) full = true;
    }
    signature = next; inv = data; busy = running;
    if (!inv.map?.nodes?.length) { host.replaceChildren(); return; }
    render();
  }

  function go(id, focus = true) {
    if (!selected) indexScroll = panel.scrollTop;
    selected = id; formType = ''; notice = '';
    render();
    if (focus) {
      panel.scrollTop = 0;
      host.querySelector('.investigation-claim-title')?.focus({ preventScroll: true });
    }
  }

  function card(node) {
    const link = button('', () => go(node.id), 'investigation-node');
    link.dataset.node = node.id;
    link.append(element('span', 'investigation-kind', nameOf(node.kind)), math('span', node.sentence));
    return link;
  }

  function sources(parent, anchors) {
    for (const anchor of anchors || []) {
      const box = element('details', 'investigation-anchor');
      const location = anchor.start_line === anchor.end_line ? 'line ' + anchor.start_line :
        'lines ' + anchor.start_line + '–' + anchor.end_line;
      const title = element('summary', '', [anchor.label || nameOf(anchor.kind), location].join(' · '));
      const quote = math('blockquote', anchor.excerpt);
      const open = button('Open cited lines', () => openSource(anchor));
      box.append(title, quote, element('small', '', anchor.path), open);
      const original = openPdf?.(anchor, true);
      if (original) box.append(button('Open original PDF page', () => openPdf(anchor, false)));
      parent.append(box);
    }
  }

  function updateSummary(summary, running, hasPaper) {
    const box = document.getElementById('investigation-summary');
    const signature = JSON.stringify([summary, summary ? null : running, hasPaper]);
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
    const question = inv.question === 'Map the main results and reasoning in this paper.' ? '' : inv.question;
    if (question) host.append(math('p', question, 'investigation-question-text'));
    const modelSelected = Array.isArray(map.answer_node_ids);
    const matches = paperQuestionFocus(map, question);
    if (question && modelSelected) {
      if (matches.size) {
        const answers = element('details', 'investigation-anchor');
        answers.append(element('summary', '', 'Statements selected for your question (' + matches.size + ')'));
        for (const id of matches.keys()) {
          const n = map.nodes.find(n => n.id === id);
          const link = button('', () => go(id), 'investigation-node');
          link.append(math('span', n.sentence)); answers.append(link);
        }
        host.append(answers);
      } else host.append(element('p', 'investigation-meta', 'The model identified no statement that directly answers this reading question.'));
    }
    if (inv.page_start && inv.page_end) host.append(element('p', 'investigation-meta', 'PDF pages ' + inv.page_start + '–' + inv.page_end));
    const summary = element('p', 'investigation-meta', [map.coverage === 'paper' ? 'Model reports paper-wide coverage' : 'Partial map',
      map.nodes.length + ' statements', (map.edges || []).length + ' connections'].join(' · '));
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
    search.setAttribute('aria-label', 'Find a statement in the whole paper map');
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
    const { incoming, outgoing } = claimNeighbors(inv.map, node.id);
    const corrections = (inv.corrections || []).filter(c => c.map_seq === inv.map_seq);
    host.append(button('← Paper overview', () => {
      const old = selected; selected = ''; formType = ''; notice = ''; render();
      panel.scrollTop = indexScroll;
      const prior = [...host.querySelectorAll('.investigation-node')].find(c => c.dataset.node === old);
      (prior || host.querySelector('.investigation-map-tools input'))?.focus({ preventScroll: true });
    }));
    const main = element('article', 'investigation-selected');
    const heading = math('h2', node.sentence, 'investigation-claim-title');
    heading.tabIndex = -1;
    main.append(element('span', 'investigation-kind', nameOf(node.kind)), heading);
    sources(main, node.anchors);
    for (const change of corrections.filter(c => c.node_id === node.id)) {
      const note = element('aside', 'investigation-correction');
      const upstream = change.edge_from && inv.map.nodes.find(n => n.id === change.edge_from);
      note.append(element('strong', '', upstream ? 'Your correction to a connection' : 'Your correction'));
      if (upstream) note.append(math('p', 'From: ' + upstream.sentence));
      note.append(math('p', change.text)); main.append(note);
    }
    host.append(main);
    if (incoming.length) host.append(relations('What this statement uses', incoming, true, node, corrections));
    if (outgoing.length) host.append(relations('What depends on this statement', outgoing, false, node, corrections));
    if (!incoming.length && !outgoing.length) host.append(element('p', 'investigation-meta', 'No connections are recorded for this statement.'));
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
    return true;
  }
  return { reset, update, updateSummary, openNode };
}
