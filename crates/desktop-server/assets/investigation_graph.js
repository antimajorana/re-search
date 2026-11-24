// Keep one card per paper statement. A collapsed connection represents paths whose
// internal statements are hidden; expanding it exposes all contributing statements.
export function paperTopology(map) {
  const incoming = new Map(map.nodes.map(n => [n.id, []]));
  const outgoing = new Map(map.nodes.map(n => [n.id, []]));
  for (const edge of map.edges || []) {
    if (!incoming.has(edge.to) || !outgoing.has(edge.from)) continue;
    incoming.get(edge.to).push(edge); outgoing.get(edge.from).push(edge);
  }
  const remaining = new Map([...incoming].map(([id, edges]) => [id, edges.length]));
  const order = map.nodes.filter(n => !remaining.get(n.id)).map(n => n.id);
  for (let i = 0; i < order.length; i++) for (const edge of outgoing.get(order[i])) {
    remaining.set(edge.to, remaining.get(edge.to) - 1);
    if (!remaining.get(edge.to)) order.push(edge.to);
  }
  return { order, incoming, outgoing };
}

export function projectedPaperGraph(map, shown) {
  const visible = new Set(shown);
  const { order, incoming, outgoing } = paperTopology(map);
  const edges = [];
  const level = new Map(order.map(id => [id, 0]));
  for (const from of order.filter(id => visible.has(id))) {
    const paths = new Map([[from, { via: new Set(), inferred: false, explicit: true, kinds: new Set() }]]);
    for (const id of order) {
      const current = paths.get(id);
      if (!current) continue;
      if (id !== from && visible.has(id)) {
        edges.push({ from, to: id, via: [...current.via], inferred: current.inferred,
          explicit: current.explicit, kinds: [...current.kinds] });
        level.set(id, Math.max(level.get(id), level.get(from) + 1));
        continue;
      }
      for (const edge of outgoing.get(id)) {
        const next = paths.get(edge.to) || { via: new Set(), inferred: false, explicit: false, kinds: new Set() };
        if (id !== from) next.via.add(id);
        for (const hidden of current.via) next.via.add(hidden);
        next.inferred ||= current.inferred || edge.basis === 'inferred';
        next.explicit ||= current.explicit && edge.basis === 'explicit';
        next.kinds.add(edge.kind);
        paths.set(edge.to, next);
      }
    }
  }
  const rows = [];
  const neighbors = new Map();
  for (const id of order.filter(id => visible.has(id))) {
    (rows[level.get(id)] ||= []).push(id);
    neighbors.set(id, [...new Set([...incoming.get(id).map(e => e.from), ...outgoing.get(id).map(e => e.to)])].filter(x => !visible.has(x)));
  }
  return { rows, edges, neighbors };
}

export function resultDependencies(map, resultId) {
  const { incoming, order } = paperTopology(map);
  const keep = new Set([resultId]);
  for (const id of [...order].reverse()) {
    if (keep.has(id)) for (const edge of incoming.get(id)) keep.add(edge.from);
  }
  return { ...map, nodes: map.nodes.filter(n => keep.has(n.id)),
    edges: (map.edges || []).filter(e => keep.has(e.from) && keep.has(e.to)),
    answer_node_ids: map.answer_node_ids?.filter(id => keep.has(id)) };
}

export function savedFocusMap(map, root, mapSeq, requestSeq, events) {
  const recorded = events.filter(e => e.kind === 'investigation.branch' && e.payload?.map_seq === mapSeq);
  const selected = recorded.find(e => e.payload.request_seq === requestSeq && e.payload.node_id === root);
  if (!selected) return resultDependencies(map, root);
  const base = events.find(e => e.seq === mapSeq && e.kind === 'investigation.map');
  const edges = new Map();
  const key = e => JSON.stringify([e.from, e.to, e.kind]);
  for (const e of base?.payload?.edges || []) edges.set(key(e), e);
  for (const record of recorded) {
    if (record.seq > selected.seq) continue;
    for (const e of record.payload.edges || []) edges.set(key(e), e);
  }
  // A focus may connect two old statements or deepen an earlier dependency.
  // Follow paths through its own edges, including their prior inputs and
  // continuations to the result. Unrelated earlier focuses stay separate.
  const snapshot = resultDependencies({ ...map, edges: [...edges.values()] }, root);
  const { order, incoming, outgoing } = paperTopology(snapshot);
  const focusedEdges = selected.payload.edges || [];
  const retained = new Set(focusedEdges.map(key));
  const upstream = new Set(focusedEdges.map(e => e.from));
  const downstream = new Set(focusedEdges.map(e => e.to));
  for (const id of [...order].reverse()) {
    if (!upstream.has(id)) continue;
    for (const e of incoming.get(id)) { retained.add(key(e)); upstream.add(e.from); }
  }
  for (const id of order) {
    if (!downstream.has(id)) continue;
    for (const e of outgoing.get(id)) { retained.add(key(e)); downstream.add(e.to); }
  }
  return resultDependencies({ ...map, edges: snapshot.edges.filter(e => retained.has(key(e))) }, root);
}

const questionStopWords = new Set(('a an the in on to and or of for with from which what how why when where ' +
  'does do did has have had is are was were be can could would should will this that these those it as by paper result results').split(' '));

export function mappedQuestionMatches(nodes, readingQuestion) {
  function words(text) {
    return new Set((String(text || '').toLocaleLowerCase().match(/[\p{L}\p{N}]+/gu) || []).filter(w => !questionStopWords.has(w)));
  }
  const ask = words(readingQuestion);
  if (!ask.size) return new Map();
  const matches = new Map();
  for (const node of nodes) {
    const questions = (node.questions || []).filter(question => {
      const terms = words(question);
      const shared = [...terms].filter(w => ask.has(w)).length;
      return shared >= 2 || (shared === 1 && ask.size === 1 && terms.size === 1);
    });
    if (questions.length) matches.set(node.id, questions);
  }
  return matches;
}

export function paperQuestionFocus(map, question) {
  if (!question?.trim()) return new Map();
  if (!Array.isArray(map.answer_node_ids)) return mappedQuestionMatches(map.nodes, question);
  const known = new Set(map.nodes.map(n => n.id));
  return new Map(map.answer_node_ids.filter(id => known.has(id)).map(id => [id, [question]]));
}

function svg(tag, attrs = {}) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, String(value));
  return el;
}

export function renderPaperGraph(host, map, shown, questionMatches, makeCard, expand, startingNode) {
  const { rows, edges, neighbors } = projectedPaperGraph(map, shown);
  const byId = new Map(map.nodes.map(n => [n.id, n]));
  const usage = new Map();
  for (const edge of edges) for (const id of edge.via) usage.set(id, (usage.get(id) || 0) + 1);
  const upstream = new Set(questionMatches.keys());
  const topology = paperTopology(map);
  for (const id of [...topology.order].reverse()) {
    if (upstream.has(id)) for (const edge of topology.incoming.get(id)) upstream.add(edge.from);
  }
  const legend = document.createElement('p');
  legend.className = 'investigation-graph-key';
  legend.textContent = 'Read downward from supporting statements to conclusions. Solid arrows show author-stated links. Dashed arrows include an inferred link. A + count marks hidden statements. Select an arrow to inspect it.';
  const modelSelected = Array.isArray(map.answer_node_ids);
  if (questionMatches.size) legend.append(modelSelected
    ? ' Highlighted paths lead to statements the model selected to answer your question.'
    : ' Highlighted paths lead to text matches in the map’s suggested questions.');
  const viewport = document.createElement('div');
  viewport.className = 'investigation-graph-viewport'; viewport.tabIndex = 0;
  viewport.setAttribute('role', 'region'); viewport.setAttribute('aria-label', 'Paper dependency graph');
  const stage = document.createElement('div');
  stage.className = 'investigation-graph-stage';
  const overlay = svg('svg', { class: 'investigation-connectors', 'aria-label': 'Connections' });
  stage.append(overlay); viewport.append(stage);
  const info = document.createElement('section'); info.className = 'investigation-edge-info'; info.hidden = true;
  info.tabIndex = -1;
  const scrollControls = document.createElement('div'); scrollControls.className = 'investigation-graph-scroll'; scrollControls.hidden = true;
  for (const [direction, label] of [[-1, 'Scroll graph left'], [1, 'Scroll graph right']]) {
    const control = document.createElement('button'); control.type = 'button'; control.className = 'pill-btn sm';
    control.textContent = direction < 0 ? '←' : '→'; control.setAttribute('aria-label', label); control.title = label;
    control.addEventListener('click', () => viewport.scrollBy({left: direction * Math.max(180, viewport.clientWidth * .7)}));
    scrollControls.append(control);
  }
  host.append(legend, scrollControls, viewport, info);
  const cards = new Map(), levels = new Map();
  rows.forEach((ids, i) => {
    const row = document.createElement('div'); row.className = 'investigation-graph-rank';
    ids.forEach(id => {
      levels.set(id, i);
      const box = document.createElement('div'); box.className = 'investigation-graph-node';
      const card = makeCard(byId.get(id)); box.append(card);
      if (questionMatches.has(id)) {
        box.classList.add('question-match');
        const flag = document.createElement('small'); flag.textContent = modelSelected ? 'Selected for your question' : 'Suggested-question text match';
        flag.title = questionMatches.get(id).join('\n'); card.append(flag);
      }
      const hidden = neighbors.get(id);
      if (hidden.length) {
        const show = document.createElement('button'); show.type = 'button'; show.className = 'investigation-expand';
        show.textContent = 'Expand ' + hidden.length + (hidden.length === 1 ? ' adjacent statement' : ' adjacent statements');
        show.addEventListener('click', () => expand(hidden, id)); box.append(show);
      }
      cards.set(id, box); row.append(box);
    });
    stage.append(row);
  });

  function showConnection(edge) {
    const from = byId.get(edge.from), to = byId.get(edge.to);
    info.hidden = false; info.replaceChildren();
    const title = document.createElement('h3'); title.textContent = 'Selected connection'; info.append(title);
    info.append(makeCard(from));
    const label = document.createElement('p');
    const relation = edge.inferred ? (edge.explicit ? 'Author-stated and inferred paths' : 'Includes inferred reasoning') : 'Author-stated reasoning';
    const kind = edge.via.length ? 'Depends through ' + edge.via.length + ' hidden ' + (edge.via.length === 1 ? 'statement' : 'statements') :
      edge.kinds.map(k => ({ supports: 'Supports', depends_on: 'Is a prerequisite for', cites: 'Contributes a citation to' })[k] || k).join(' / ');
    label.textContent = kind + ' ↓ · ' + relation; info.append(label, makeCard(to));
    if (edge.via.length) {
      const shared = edge.via.filter(id => usage.get(id) > 1);
      if (shared.length) {
        const note = document.createElement('p');
        note.textContent = shared.length === 1 ? 'One intermediate statement also appears in another connection.' :
          shared.length + ' intermediate statements also appear in other connections.';
        info.append(note);
      }
      const button = document.createElement('button'); button.type = 'button'; button.className = 'pill-btn sm';
      button.textContent = 'Expand connecting statements'; button.addEventListener('click', () => expand(edge.via, edge.to)); info.append(button);
    }
    info.focus();
  }

  let frame = 0, positionInitialView = true;
  function schedule() { cancelAnimationFrame(frame); frame = requestAnimationFrame(draw); }
  function draw() {
    if (!stage.isConnected || !stage.clientWidth) return;
    const bounds = stage.getBoundingClientRect();
    const rects = new Map([...cards].map(([id, card]) => [id, card.getBoundingClientRect()]));
    scrollControls.hidden = viewport.scrollWidth <= viewport.clientWidth + 1;
    if (positionInitialView && viewport.clientWidth) {
      positionInitialView = false;
      const first = rects.get(startingNode) || rects.get(questionMatches.keys().next().value) || rects.get(rows[0]?.[0]);
      if (first) viewport.scrollLeft = (first.left + first.right)/2 - bounds.left - viewport.clientWidth/2;
    }
    overlay.setAttribute('viewBox', '0 0 ' + stage.clientWidth + ' ' + stage.clientHeight);
    const focused = overlay.contains(document.activeElement) ? document.activeElement.dataset : null;
    overlay.replaceChildren();
    const defs = svg('defs');
    for (const [id, color] of [['paper-arrow', 'var(--ink-secondary)'], ['paper-question-arrow', 'var(--accent)']]) {
      const marker = svg('marker', { id, viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 6, markerHeight: 6, orient: 'auto' });
      marker.append(svg('path', { d: 'M0 0 L10 5 L0 10 z', fill: color })); defs.append(marker);
    }
    overlay.append(defs);
    edges.forEach((edge, index) => {
      const a = rects.get(edge.from), b = rects.get(edge.to);
      const x1 = (a.left + a.right)/2 - bounds.left, x2 = (b.left+b.right)/2 - bounds.left;
      const y1 = a.bottom - bounds.top + 2, y2 = b.top - bounds.top - 3;
      const middle = (y1+y2)/2;
      let d = 'M ' + x1 + ' ' + y1 + ' C ' + x1 + ' ' + middle + ' ' + x2 + ' ' + middle + ' ' + x2 + ' ' + y2;
      let labelX = (x1+x2)/2, labelY = middle;
      if (levels.get(edge.to) > levels.get(edge.from) + 1) {
        const lane = x1+x2 < stage.clientWidth ? 6 + index%4 * 3 : stage.clientWidth - 6 - index%4 * 3;
        const startLane = y1+15, endLane = y2-15;
        d = 'M '+x1+' '+y1+' V '+startLane+' H '+lane+' V '+endLane+' H '+x2+' V '+y2;
        labelX = lane; labelY = middle;
      }
      const highlighted = upstream.has(edge.from) && upstream.has(edge.to);
      const color = highlighted ? 'var(--accent)' : 'var(--ink-secondary)';
      const label = byId.get(edge.from).sentence + ' → ' + byId.get(edge.to).sentence +
        (edge.via.length ? ' · ' + edge.via.length + (edge.via.length === 1 ? ' hidden statement' : ' hidden statements') : '') +
        (edge.inferred ? ' · includes inference' : ' · author-stated') + (highlighted ? ' · on a reading-question path' : '');
      const group = svg('g', { tabindex: 0, role: 'button', 'aria-label': label, class: 'investigation-edge',
        'data-from': edge.from, 'data-to': edge.to, 'data-hidden': edge.via.length });
      const title = svg('title'); title.textContent = label;
      const line = svg('path', { d, fill: 'none', stroke: color, 'stroke-width': highlighted ? 2.4 : 1.6,
        'marker-end': 'url(#' + (highlighted ? 'paper-question-arrow' : 'paper-arrow') + ')' });
      if (edge.inferred) line.setAttribute('stroke-dasharray', '5 4');
      group.append(title, svg('path', { d, fill: 'none', stroke: 'transparent', 'stroke-width': 14 }), line);
      if (edge.via.length) {
        const badge = svg('text', { x: labelX, y: labelY, class: 'investigation-edge-count', 'text-anchor':'middle', dy: '.32em' });
        badge.textContent = '+' + edge.via.length; group.append(badge);
      }
      group.addEventListener('click', () => showConnection(edge));
      group.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); showConnection(edge); } });
      overlay.append(group);
      if (focused?.from === edge.from && focused.to === edge.to) group.focus({ preventScroll: true });
    });
  }
  const observer = new ResizeObserver(schedule);
  observer.observe(stage);
  for (const card of cards.values()) observer.observe(card);
  schedule();
  return () => { observer.disconnect(); cancelAnimationFrame(frame); };
}
