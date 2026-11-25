// Answer card for question-first Investigation turns.
import { renderMathTextInto, renderRichInto } from '/assets/math.js';
import { linkLineCitations } from '/assets/paper_cites.js';

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function renderRowBody(parent, row, openCite) {
  if (row?.status === 'running') {
    parent.append(el('p', 'investigation-meta', 'Working on your question…'));
    return;
  }
  if (row?.status === 'stopped') {
    parent.append(el('p', 'investigation-meta', 'Stopped before the answer was written. Ask again below.'));
    return;
  }
  if (row?.status === 'failed') {
    parent.append(el('p', 'investigation-meta', row.error || 'The model could not answer this question.'));
    return;
  }
  if (row?.status === 'interrupted') {
    parent.append(el('p', 'investigation-meta', row.error || 'Stopped before the answer was written. Ask again below.'));
    return;
  }
  const body = el('div', 'investigation-answer-body rich-prose');
  renderRichInto(body, String(row?.text || ''));
  linkLineCitations(body, row?.cites, openCite);
  parent.append(body);
  parent.append(el('p', 'investigation-answer-note investigation-meta', 'Written by the model. Check the cited lines.'));
}

export function createAnswerCard(host, { onFindMainResults, onOpenConversation, openCite } = {}) {
  let signature = '';
  let earlierOpen = false;
  let pendingMap = false;
  let epoch = 0;

  function reset() {
    epoch += 1;
    signature = '';
    earlierOpen = false;
    pendingMap = false;
    if (!host) return;
    host.hidden = true;
    host.replaceChildren();
  }

  function update(answers, { hasMap = false, running = false, sourceId = null, primaryPath = null } = {}) {
    if (!host) return;
    const rows = Array.isArray(answers) ? answers : [];
    const next = JSON.stringify([rows, hasMap, running, sourceId, primaryPath, pendingMap]);
    if (next === signature) return;
    signature = next;
    host.replaceChildren();
    if (!rows.length) {
      host.hidden = true;
      return;
    }
    host.hidden = false;
    const newest = rows.at(-1);
    const older = rows.slice(0, -1);

    host.append(el('h2', '', 'Answer'));
    if (newest?.question) {
      const q = el('p', 'investigation-answer-question');
      renderMathTextInto(q, String(newest.question));
      host.append(q);
    }
    renderRowBody(host, newest, openCite);

    const actions = el('div', 'investigation-actions');
    if (!hasMap) {
      const mapBtn = el('button', 'pill-btn sm primary', 'Find main results');
      mapBtn.type = 'button';
      mapBtn.disabled = running || pendingMap;
      mapBtn.addEventListener('click', async () => {
        if (pendingMap || mapBtn.disabled || typeof onFindMainResults !== 'function') return;
        const start = epoch;
        pendingMap = true;
        mapBtn.disabled = true;
        try {
          await onFindMainResults(String(newest?.question || ''));
        } finally {
          if (start === epoch) {
            pendingMap = false;
            mapBtn.disabled = running;
          }
        }
      });
      actions.append(mapBtn);
    }
    const chatBtn = el('button', 'pill-btn sm', 'Open in Conversation');
    chatBtn.type = 'button';
    chatBtn.addEventListener('click', () => onOpenConversation?.());
    actions.append(chatBtn);
    host.append(actions);

    if (older.length) {
      const details = el('details', 'investigation-earlier-answers');
      details.open = earlierOpen;
      details.append(el('summary', '', `Earlier questions (${older.length})`));
      let rendered = false;
      const renderOlder = () => {
        if (rendered) return;
        rendered = true;
        for (const item of [...older].reverse()) {
          const article = el('article', 'investigation-earlier-item');
          if (item?.question) {
            const q = el('p', 'investigation-answer-question');
            renderMathTextInto(q, String(item.question));
            article.append(q);
          }
          renderRowBody(article, item, openCite);
          details.append(article);
        }
      };
      if (details.open) renderOlder();
      details.addEventListener('toggle', () => {
        earlierOpen = Boolean(details.open);
        if (details.open) renderOlder();
      });
      host.append(details);
    }
  }

  return { reset, update };
}
