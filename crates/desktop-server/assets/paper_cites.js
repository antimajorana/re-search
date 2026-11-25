// Link checked [Lstart–end] paper citations inside Answer card prose.

const CITE_GROUP_RE = /\[L(\d+)(?:[–-](\d+))?(?:,\s*L(\d+)(?:[–-](\d+))?)*\]/g;
const CITE_PART_RE = /L(\d+)(?:[–-](\d+))?/g;
const SKIP_TAGS = new Set(['CODE', 'PRE', 'BUTTON', 'A', 'SCRIPT', 'STYLE']);
const CITE_TITLE = 'Opens these lines with context. The model chose this citation; the app has not checked that it supports the sentence.';

function shouldSkip(node) {
  if (!node || node.nodeType === 3) return false;
  if (SKIP_TAGS.has(String(node.tagName || '').toUpperCase())) return true;
  if (node.classList?.contains?.('katex') || node.classList?.contains?.('katex-display')) return true;
  const classes = String(node.className || '').split(/\s+/);
  return classes.includes('katex') || classes.includes('katex-display');
}

const MAX_CITE_SPAN = 512;

function buildAllowedSet(cites) {
  const allowed = new Set();
  for (const item of Array.isArray(cites) ? cites : []) {
    if (!item || item.read !== true) continue;
    const start = Number(item.start);
    const end = Number(item.end);
    if (Number.isInteger(start) && Number.isInteger(end) && start >= 1 && start <= end && end - start <= MAX_CITE_SPAN) {
      allowed.add(`${start}:${end}`);
    }
  }
  return allowed;
}

function replaceTextWithCites(text, allowed, open) {
  CITE_GROUP_RE.lastIndex = 0;
  const nodes = [];
  let last = 0;
  let changed = false;

  for (const match of text.matchAll(CITE_GROUP_RE)) {
    const group = match[0];
    const inner = group.slice(1, -1);
    const pieces = [];
    let partLast = 0;
    let groupChanged = false;

    CITE_PART_RE.lastIndex = 0;
    for (const part of inner.matchAll(CITE_PART_RE)) {
      if (part.index > partLast) pieces.push(document.createTextNode(inner.slice(partLast, part.index)));
      const start = Number(part[1]);
      const end = part[2] !== undefined ? Number(part[2]) : start;
      if (Number.isInteger(start) && Number.isInteger(end) && start >= 1 && start <= end && end - start <= MAX_CITE_SPAN && allowed.has(`${start}:${end}`)) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'line-cite';
        btn.textContent = part[0];
        btn.title = CITE_TITLE;
        if (btn.dataset) {
          btn.dataset.start = String(start);
          btn.dataset.end = String(end);
        }
        btn.addEventListener('click', () => open?.(start, end));
        pieces.push(btn);
        groupChanged = true;
      } else {
        pieces.push(document.createTextNode(part[0]));
      }
      partLast = part.index + part[0].length;
    }
    if (partLast < inner.length) pieces.push(document.createTextNode(inner.slice(partLast)));

    if (!groupChanged) continue;
    if (match.index > last) nodes.push(document.createTextNode(text.slice(last, match.index)));
    nodes.push(document.createTextNode('['), ...pieces, document.createTextNode(']'));
    last = match.index + group.length;
    changed = true;
  }

  if (!changed) return null;
  if (last < text.length) nodes.push(document.createTextNode(text.slice(last)));
  return nodes;
}

function walk(node, allowed, open) {
  if (!node || shouldSkip(node)) return;
  const list = [...(node.childNodes || node.children || [])];
  for (const child of list) {
    if (child.nodeType === 3) {
      const value = child.nodeValue ?? child.textContent ?? '';
      if (!value.includes('[L')) continue;
      const replacement = replaceTextWithCites(value, allowed, open);
      if (!replacement) continue;
      if (typeof child.replaceWith === 'function') {
        child.replaceWith(...replacement);
      } else if (typeof node.insertBefore === 'function' && typeof node.removeChild === 'function') {
        for (const item of replacement) node.insertBefore(item, child);
        node.removeChild(child);
      }
    } else {
      walk(child, allowed, open);
    }
  }
}

export function linkLineCitations(container, cites, open) {
  if (!container) return;
  const allowed = buildAllowedSet(cites);
  if (!allowed.size) return;
  walk(container, allowed, open);
}
