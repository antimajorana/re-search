import * as api from '/assets/api.js';
import { renderRichInto, renderMathTextInto } from '/assets/math.js';
import { linksForPaper, sessionSourcesUsed } from '/assets/source_links.js';
import { renderProofEvidence } from '/assets/canvas_dag_detail.js';

// Keep source offsets intact so a review can be bound to the exact saved answer.
// Sentence boundaries inside math, URLs and code are never selection boundaries.
const protectedText = /^(?:[ \t]*)(`{3,}|~{3,})[^\n]*\n[\s\S]*?(?:^\s*\1[ \t]*(?=\n|$)|$(?![\s\S]))|\$\$[\s\S]*?\$\$|\\\[[\s\S]*?\\\]|\\\([\s\S]*?\\\)|\\begin\{([^{}]+)\}[\s\S]*?\\end\{\2\}|\$(?:\\.|[^$\n])+\$|`[^`\n]+`|\[[^\]\n]*\]\(https?:\/\/[^\s]+\)|https?:\/\/[^\s)]+/gm;
export function splitClaims(text) {
  const cuts = new Set([text.length]);
  const masked = text.replace(protectedText, (s, fence, env, offset) => {
    if (s.startsWith('$$') || s.startsWith('\\[') || env || fence) {
      cuts.add(offset);
      // Keep a citation or sentence punctuation immediately after a displayed equation.
      const suffix = text.slice(offset + s.length).match(/^(?:[ \t]*\[(?:e-\d+(?:\s*,\s*e-\d+)*)\])*[.,;:]?/);
      cuts.add(offset + s.length + suffix[0].length);
    }
    return 'X'.repeat(s.length);
  });
  const boundary = /[.!?](?:[*_"’”)]*)\s+|\n[ \t]*\n+|\n(?=[ \t]*(?:[-*#>]|\d+[.)])\s)/g;
  for (const match of masked.matchAll(boundary)) cuts.add(match.index + match[0].length);
  const pieces = [];
  let start = 0;
  for (const end of [...cuts].sort((a, b) => a - b)) {
    const part = text.slice(start, end).trim();
    if (part) pieces.push(part);
    start = end;
  }
  return pieces;
}

function matchText(value) {
  return String(value || '').trim().replace(/^[#>]+ +|^[-*] +/g, '').replace(/[.!?]+$/, '').replace(/\s+/g, ' ').trim();
}

const mathPattern = /\$\$([\s\S]*?)\$\$|\\\[([\s\S]*?)\\\]|\\\(([\s\S]*?)\\\)|\$((?:\\.|[^$\n])+)\$/g;
function mathExpressions(text) {
  return [...text.matchAll(mathPattern)].map(m => matchText(m.slice(1).find(v => v !== undefined)));
}

export function linkedSteps(claim, entries, answerSeq) {
  const prose = claim.replace(protectedText, s => /^`e-\d+`$/.test(s) ? s : ' ');
  const mentioned = new Set(prose.match(/(?<![\w-])e-\d+(?![\w-])/g) || []);
  const normalized = matchText(claim);
  const equations = new Set(mathExpressions(claim));
  const result = [];
  for (const entry of entries) {
    if (entry.seq > answerSeq) continue;
    const id = entry.entry_id;
    const summary = matchText(entry.summary);
    const equation = entry.latex && (mathExpressions(entry.latex)[0] || matchText(entry.latex));
    let reason;
    if (mentioned.has(id)) reason = 'Cited proof step';
    else if (entry.entry_type !== 'final_answer' && summary.length >= 12 && summary === normalized) reason = 'Matching recorded statement';
    else if (entry.entry_type !== 'final_answer' && equation?.length >= 5 && equations.has(equation)) reason = 'Matching recorded equation';
    if (reason) result.push({ entry, reason });
  }
  return result;
}

function element(tag, text, className) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text) el.textContent = text;
  return el;
}
function button(text, handler) {
  const el = element('button', text, 'pill-btn sm');
  el.type = 'button';
  el.addEventListener('click', handler);
  return el;
}

function showReview(container, review) {
  container.replaceChildren(element('strong', 'Claim review · ' + review.verdict));
  if (review.model) container.append(element('span', ' · ' + review.model));
  const summary = element('p');
  renderMathTextInto(summary, review.summary || '');
  container.append(summary);
  for (const issue of review.issues || []) {
    const detail = element('p');
    renderMathTextInto(detail, typeof issue === 'string' ? issue : issue.description || issue.message || '');
    container.append(detail);
  }
  if (review.counterexample) {
    const example = element('p');
    renderMathTextInto(example, 'Counterexample: ' + review.counterexample);
    container.append(example);
  }
}

export function attachAnswerTrace(row, answer, seq, sessionId, { inspectStep, openSource }) {
  const body = row.querySelector('.msg-assistant-body');
  if (!body || !sessionId || !Number.isSafeInteger(seq)) return;
  const tools = element('div', '', 'claim-trace-tools');
  const status = element('span', '', 'claim-trace-help');
  const toggle = button('Trace a claim', () => toggleTrace());
  toggle.setAttribute('aria-expanded', 'false');
  tools.append(toggle, status);
  const panel = element('section', '', 'claim-trace-panel');
  panel.setAttribute('aria-label', 'Evidence for selected claim');
  panel.tabIndex = -1;
  panel.hidden = true;
  row.append(tools, panel);
  let open = false;
  let snapshot;
  let selection = 0;
  const pendingReviews = new Map();
  const newReviews = new Map();

  async function trace(claim, pick) {
    const request = ++selection;
    for (const choice of body.querySelectorAll('.claim-choice')) choice.setAttribute('aria-pressed', String(choice === pick));
    panel.hidden = false;
    panel.replaceChildren(element('h4', 'Evidence for this claim'), element('p', 'Reading the session record…'));
    panel.focus({ preventScroll: true });
    panel.scrollIntoView({ block: 'nearest' });
    try {
      snapshot ||= Promise.all([api.getSession(sessionId), api.getDag(sessionId)]);
      const [session, graph] = await snapshot;
      if (request !== selection || !row.isConnected) return;
      const events = session.events || [];
      const papers = sessionSourcesUsed({ ...session, events: events.filter(e => e.seq <= seq) }, claim);
      const matches = linkedSteps(claim, graph.entries || [], seq);
      panel.replaceChildren(element('h4', 'Evidence for this claim'));
      const quote = element('blockquote');
      renderRichInto(quote, claim);
      panel.append(quote);
      if (!matches.length && !papers.length) panel.append(element('p', 'No proof step, calculation or citation is linked to this wording in the saved answer.'));
      for (const { paper, source } of papers) {
        const line = element('div', '', 'claim-trace-source');
        const link = linksForPaper(paper)[0];
        line.append('Cited source · ');
        if (link) {
          const a = element('a', paper.title);
          a.href = link.url;
          a.target = '_blank';
          a.rel = 'noopener noreferrer';
          line.append(a);
        } else {
          line.append(paper.title || paper.url || 'Saved paper');
          if (paper.url && paper.title !== paper.url) line.append(' · ' + paper.url);
        }
        if (source?.primary_path) line.append(button('Read saved source', () => openSource({ source_id: source.source_id, path: source.primary_path })));
        panel.append(line);
      }
      for (const { entry, reason } of matches) {
        const group = element('details', '', 'claim-step-evidence');
        const summary = element('summary');
        renderMathTextInto(summary, reason + ' · ' + entry.entry_id + ' · ' + (entry.summary || entry.entry_type));
        group.append(summary);
        if (['superseded', 'rejected'].includes(String(entry.status).toLowerCase())) group.append(element('p', 'This step is ' + entry.status + '.'));
        group.append(button('Open proof step ' + entry.entry_id, () => inspectStep(entry.entry_id)));
        const evidence = element('div');
        group.append(evidence);
        let started = false;
        const load = async () => {
          if (!group.open || started) return;
          started = true;
          evidence.textContent = 'Reading linked checks and passages…';
          try {
            const result = await api.getProofEvidence(sessionId, entry.entry_id);
            const record = result.entries?.find(item => item.entry_id === entry.entry_id);
            renderProofEvidence(evidence, entry, record, inspectStep, openSource);
          } catch (error) {
            evidence.textContent = error.message;
            started = false;
          }
        };
        group.addEventListener('toggle', load);
        panel.append(group);
        if (matches.length === 1) { group.open = true; load(); }
      }
      const reviewPanel = element('div', '', 'claim-review-result');
      reviewPanel.setAttribute('role', 'status');
      const recorded = newReviews.get(claim) || events.findLast(e => e.kind === 'claim.review' && e.payload?.answer_seq === seq && e.payload.claim === claim)?.payload;
      if (recorded) showReview(reviewPanel, recorded);
      const review = button(recorded ? 'Review this claim again' : 'Review this claim', async () => {
        if (pendingReviews.has(claim)) return;
        review.disabled = true;
        reviewPanel.textContent = 'Requesting an independent model review…';
        const pending = api.reviewClaim(sessionId, { claim, answer_seq: seq });
        pendingReviews.set(claim, pending);
        try {
          const result = await pending;
          newReviews.set(claim, result);
          if (request === selection) {
            showReview(reviewPanel, result);
            review.textContent = 'Review this claim again';
          }
        } catch (error) {
          if (request === selection) reviewPanel.textContent = 'Review unavailable. ' + error.message;
        } finally {
          pendingReviews.delete(claim);
          review.disabled = false;
          if (request !== selection && pick.getAttribute('aria-pressed') === 'true' && open && pick.isConnected) trace(claim, pick);
        }
      });
      if (pendingReviews.has(claim)) { review.disabled = true; reviewPanel.textContent = 'Review in progress…'; }
      panel.append(review, reviewPanel);
    } catch (error) {
      snapshot = undefined;
      if (request === selection) panel.replaceChildren(element('p', 'Could not read the evidence. ' + error.message));
    }
  }
  function toggleTrace() {
    open = !open;
    ++selection;
    panel.hidden = true;
    panel.replaceChildren();
    toggle.setAttribute('aria-expanded', String(open));
    toggle.textContent = open ? 'Close claim tracing' : 'Trace a claim';
    status.textContent = open ? 'Choose a statement or equation to inspect.' : '';
    if (!open) { renderRichInto(body, answer); return; }
    snapshot = undefined;
    const fragment = document.createDocumentFragment();
    for (const claim of splitClaims(answer)) {
      const pick = button('', () => trace(claim, pick));
      pick.className = 'claim-choice';
      pick.setAttribute('aria-pressed', 'false');
      renderMathTextInto(pick, claim);
      fragment.append(pick);
    }
    body.replaceChildren(fragment);
    body.querySelector('.claim-choice')?.focus();
  }
}
