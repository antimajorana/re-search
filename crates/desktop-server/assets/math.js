// Safe offline Markdown + KaTeX math rendering and syntax/diff formatting helpers.

export function escapeHtml(raw) {
  return String(raw ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

const DELIMITERS = [
  { left: '$$', right: '$$', display: true },
  { left: '\\[', right: '\\]', display: true },
  { left: '\\begin{equation}', right: '\\end{equation}', display: true },
  { left: '\\begin{align}', right: '\\end{align}', display: true },
  { left: '\\begin{align*}', right: '\\end{align*}', display: true },
  { left: '\\begin{gather}', right: '\\end{gather}', display: true },
  { left: '$', right: '$', display: false },
  { left: '\\(', right: '\\)', display: false },
];

export function renderMathInContainer(el) {
  if (!el) return;
  if (typeof window.renderMathInElement === 'function') {
    window.renderMathInElement(el, {
      delimiters: DELIMITERS,
      throwOnError: false,
      strict: false,
    });
    return;
  }
  if (window.katex && typeof window.katex.renderToString === 'function') {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const nodes = [];
    let cur = walker.nextNode();
    while (cur) {
      const parent = cur.parentElement;
      if (parent && !/^(CODE|PRE|SCRIPT|STYLE|TEXTAREA)$/.test(parent.tagName)) {
        if (/(\$\$|\$|\\\(|\\\[)/.test(cur.nodeValue || '')) {
          nodes.push(cur);
        }
      }
      cur = walker.nextNode();
    }
    for (const node of nodes) {
      const span = document.createElement('span');
      span.innerHTML = renderInlineMathFallback(node.nodeValue || '');
      node.parentNode.replaceChild(span, node);
    }
  }
}

function renderInlineMathFallback(text) {
  const rx = /(\$\$[\s\S]+?\$\$|\\\[[\s\S]+?\\\]|\$[^$\n]+?\$|\\\([\s\S]+?\\\))/g;
  let out = '';
  let last = 0;
  for (const m of text.matchAll(rx)) {
    out += escapeHtml(text.slice(last, m.index));
    const tok = m[0];
    const display = tok.startsWith('$$') || tok.startsWith('\\[');
    const inner = tok.slice(2, tok.length - 2);
    const expr = tok.startsWith('$') && !display ? tok.slice(1, -1) : inner;
    try {
      out += window.katex.renderToString(expr, { displayMode: display, throwOnError: false });
    } catch {
      out += escapeHtml(tok);
    }
    last = m.index + tok.length;
  }
  out += escapeHtml(text.slice(last));
  return out;
}

function renderInlineMarkdown(text, mathTokens) {
  let s = text.replace(/\u0000M(\d+)\u0000/g, (_, idx) => `\u0000M${idx}\u0000`);
  const codeTokens = [];
  s = s.replace(/`([^`]+)`/g, (_, code) => {
    const id = codeTokens.length;
    codeTokens.push(`<code>${escapeHtml(code)}</code>`);
    return `\u0000C${id}\u0000`;
  });
  s = escapeHtml(s);
  s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
  s = s.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, (_, label, url) => {
    return `<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">${label}</a>`;
  });
  s = s.replace(/\u0000C(\d+)\u0000/g, (_, i) => codeTokens[Number(i)] || '');
  s = s.replace(/\u0000M(\d+)\u0000/g, (_, i) => escapeHtml(mathTokens[Number(i)] || ''));
  return s;
}

export function markdownToHtml(raw) {
  const src = String(raw ?? '').replace(/\r\n/g, '\n');
  const codeBlocks = [];
  let text = src.replace(/```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g, (_, lang, body) => {
    const id = codeBlocks.length;
    const cls = lang ? ` class="lang-${escapeHtml(lang)}"` : '';
    codeBlocks.push(`<pre><code${cls}>${escapeHtml(body)}</code></pre>`);
    return `\n\u0000B${id}\u0000\n`;
  });

  const mathTokens = [];
  text = text.replace(
    /(\$\$[\s\S]+?\$\$|\\\[[\s\S]+?\\\]|\\begin\{(?:equation|align\*?|gather)\}[\s\S]+?\\end\{(?:equation|align\*?|gather)\}|\\\([\s\S]+?\\\)|\$(?:\\.|[^$\n\\])+\$)/g,
    (m) => {
      const id = mathTokens.length;
      mathTokens.push(m);
      return `\u0000M${id}\u0000`;
    }
  );

  const lines = text.split('\n');
  const out = [];
  let inUl = false;
  let inOl = false;

  const closeLists = () => {
    if (inUl) { out.push('</ul>'); inUl = false; }
    if (inOl) { out.push('</ol>'); inOl = false; }
  };

  for (const line of lines) {
    const blockMatch = line.match(/^\u0000B(\d+)\u0000$/);
    if (blockMatch) {
      closeLists();
      out.push(codeBlocks[Number(blockMatch[1])] || '');
      continue;
    }
    const hMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (hMatch) {
      closeLists();
      const lvl = hMatch[1].length;
      out.push(`<h${lvl}>${renderInlineMarkdown(hMatch[2], mathTokens)}</h${lvl}>`);
      continue;
    }
    if (/^>\s?/.test(line)) {
      closeLists();
      out.push(`<blockquote>${renderInlineMarkdown(line.replace(/^>\s?/, ''), mathTokens)}</blockquote>`);
      continue;
    }
    const ulMatch = line.match(/^[-*]\s+(.+)$/);
    if (ulMatch) {
      if (inOl) { out.push('</ol>'); inOl = false; }
      if (!inUl) { out.push('<ul>'); inUl = true; }
      out.push(`<li>${renderInlineMarkdown(ulMatch[1], mathTokens)}</li>`);
      continue;
    }
    const olMatch = line.match(/^\d+\.\s+(.+)$/);
    if (olMatch) {
      if (inUl) { out.push('</ul>'); inUl = false; }
      if (!inOl) { out.push('<ol>'); inOl = true; }
      out.push(`<li>${renderInlineMarkdown(olMatch[1], mathTokens)}</li>`);
      continue;
    }
    if (!line.trim()) {
      closeLists();
      continue;
    }
    closeLists();
    out.push(`<p>${renderInlineMarkdown(line, mathTokens)}</p>`);
  }
  closeLists();
  return out.join('\n');
}

export function renderRichInto(el, rawText) {
  if (!el) return;
  el.innerHTML = markdownToHtml(rawText);
  renderMathInContainer(el);
}

export function renderLatexDocumentPreview(el, texSource) {
  if (!el) return;
  const src = String(texSource ?? '');
  if (!src.trim()) {
    el.innerHTML = '<p style="color:var(--ink-muted)">Empty TeX buffer.</p>';
    return;
  }
  const normalized = src
    .replace(/\\section\*?\{([^}]*)\}/g, '\n## $1\n')
    .replace(/\\subsection\*?\{([^}]*)\}/g, '\n### $1\n')
    .replace(/\\textbf\{([^}]*)\}/g, '**$1**')
    .replace(/\\emph\{([^}]*)\}/g, '*$1*')
    .replace(/\\cite\{([^}]*)\}/g, '`[cite:$1]`')
    .replace(/\\(?:eq|auto|c|C|page)?ref\{([^}]*)\}/g, '`[ref:$1]`');
  renderRichInto(el, normalized);
}

export function highlightUnifiedDiff(diffText) {
  return String(diffText ?? '')
    .split('\n')
    .map((line) => {
      const esc = escapeHtml(line);
      if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('@@')) {
        return `<span class="diff-line diff-hunk">${esc}</span>`;
      }
      if (line.startsWith('+')) {
        return `<span class="diff-line diff-add">${esc}</span>`;
      }
      if (line.startsWith('-')) {
        return `<span class="diff-line diff-del">${esc}</span>`;
      }
      return `<span class="diff-line">${esc}</span>`;
    })
    .join('');
}

export function highlightPython(codeText) {
  const kw = /\b(import|from|as|def|return|if|elif|else|for|in|while|with|try|except|True|False|None|and|or|not)\b/g;
  return String(codeText ?? '')
    .split('\n')
    .map((line) => {
      const hashIdx = line.indexOf('#');
      const head = hashIdx >= 0 ? line.slice(0, hashIdx) : line;
      const tail = hashIdx >= 0 ? line.slice(hashIdx) : '';
      let html = escapeHtml(head)
        .replace(/(&quot;[^&]*?&quot;|&#39;[^&]*?&#39;)/g, '<span class="py-str">$1</span>')
        .replace(kw, '<span class="py-kw">$1</span>');
      if (tail) {
        html += `<span class="py-com">${escapeHtml(tail)}</span>`;
      }
      return `<span class="diff-line">${html}</span>`;
    })
    .join('');
}
