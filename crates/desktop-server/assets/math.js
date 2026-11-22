// Safe offline Markdown, LaTeX document preview, and KaTeX math rendering helpers.

export function escapeHtml(raw) {
  return String(raw ?? '')
    .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll("'", '&#39;');
}

const KATEX_NATIVE_ENVS = (
  'equation equation* align align* gather gather* alignat alignat* split aligned gathered '
  + 'cases dcases rcases matrix pmatrix bmatrix Bmatrix vmatrix Vmatrix smallmatrix array CD'
).split(' ');
const EXTRA_MATH_ENVS = 'flalign flalign* multline multline* eqnarray eqnarray* subequations'.split(' ');
const MATH_ENVS = new Set([...KATEX_NATIVE_ENVS, ...EXTRA_MATH_ENVS]);

const DELIMITERS_NO_DOLLAR = [
  { left: '$$', right: '$$', display: true },
  { left: '\\[', right: '\\]', display: true },
  ...[...KATEX_NATIVE_ENVS, ...EXTRA_MATH_ENVS].map((e) => ({ left: `\\begin{${e}}`, right: `\\end{${e}}`, display: true })),
  { left: '\\(', right: '\\)', display: false },
];
const DELIMITERS = [...DELIMITERS_NO_DOLLAR, { left: '$', right: '$', display: false }];

const TEX_STRUCT_ENVS = new Set(
  'abstract theorem lemma proposition corollary definition remark example proof conjecture assumption claim note'.split(' ')
);

function normalizeEqnarrayBody(body) {
  return body.split('\\\\').map((r) => r.replace(/&\s*([^&\n]+?)\s*&/, '&$1 ')).join('\\\\');
}

function normalizeMathTex(rawTex, isDisplay) {
  let s = String(rawTex ?? '').replace(/\\label\{[^{}]*\}/g, '');
  if (isDisplay) {
    s = s.replace(/\\(?:begin|end)\{subequations\}/g, '')
      .replace(/\\(begin|end)\{multline\}/g, '\\$1{gather}')
      .replace(/\\(begin|end)\{multline\*\}/g, '\\$1{gathered}')
      .replace(/\\(begin|end)\{flalign\}/g, '\\$1{align}')
      .replace(/\\(begin|end)\{flalign\*\}/g, '\\$1{align*}')
      .replace(/\\begin\{eqnarray\}([\s\S]*?)\\end\{eqnarray\}/g, (_, b) => `\\begin{align}${normalizeEqnarrayBody(b)}\\end{align}`)
      .replace(/\\begin\{eqnarray\*\}([\s\S]*?)\\end\{eqnarray\*\}/g, (_, b) => `\\begin{aligned}${normalizeEqnarrayBody(b)}\\end{aligned}`);
  }
  return s.trim();
}

function isEscapedAt(text, pos) {
  let bs = 0;
  for (let k = pos - 1; k >= 0 && text[k] === '\\'; k--) bs += 1;
  return bs % 2 === 1;
}

function findMatchingEnvEnd(text, env, pos) {
  const openTag = `\\begin{${env}}`;
  const closeTag = `\\end{${env}}`;
  let depth = 1;
  while (pos < text.length) {
    const o = text.indexOf(openTag, pos);
    const c = text.indexOf(closeTag, pos);
    if (c === -1) return -1;
    if (o !== -1 && o < c) { depth += 1; pos = o + openTag.length; }
    else { depth -= 1; pos = c + closeTag.length; if (depth === 0) return pos; }
  }
  return -1;
}

function findClosingInlineDollar(text, startIdx) {
  const first = text[startIdx];
  if (!first || /\s/.test(first) || first === '$') return -1;
  for (let j = startIdx; j < text.length; j++) {
    const ch = text[j];
    if (ch === '\u0000' || (ch === '\n' && text[j + 1] === '\n')) return -1;
    if (ch === '\\' && (text[j + 1] === '[' || text[j + 1] === '(' || text.startsWith('\\begin{', j))) return -1;
    if (ch === '$' && !isEscapedAt(text, j)) {
      if (text[j + 1] === '$') return -1;
      const prev = text[j - 1];
      if (!prev || /\s/.test(prev) || /[0-9]/.test(text[j + 1] || '')) return -1;
      return j;
    }
  }
  return -1;
}

function findClosingDelim(text, delim, pos) {
  while (pos < text.length) {
    const idx = text.indexOf(delim, pos);
    if (idx === -1 || !isEscapedAt(text, idx)) return idx;
    pos = idx + 1;
  }
  return -1;
}

function extractMath(text, mathTokens, opts = {}) {
  const protectCode = opts.protectInlineCode !== false;
  const allowDollar = opts.allowSingleDollar !== false;
  let out = '';
  let i = 0;
  const push = (raw, display) => {
    const tex = normalizeMathTex(raw, display);
    if (!tex) return '';
    const id = mathTokens.length;
    mathTokens.push({ tex, display });
    return `\u0000M${id}\u0000`;
  };
  while (i < text.length) {
    const ch = text[i];
    if (ch === '\u0000') {
      const end = text.indexOf('\u0000', i + 1);
      if (end !== -1) { out += text.slice(i, end + 1); i = end + 1; continue; }
    }
    if (protectCode && ch === '`') {
      let ticks = 1;
      while (text[i + ticks] === '`') ticks += 1;
      const marker = '`'.repeat(ticks);
      let close = text.indexOf(marker, i + ticks);
      while (close !== -1 && (text[close - 1] === '`' || text[close + ticks] === '`')) close = text.indexOf(marker, close + 1);
      if (close !== -1 && !text.slice(i + ticks, close).includes('\n\n')) { out += text.slice(i, close + ticks); i = close + ticks; continue; }
      out += marker; i += ticks; continue;
    }
    if (ch === '\\') {
      const next = text[i + 1];
      if (next === '$') { out += '\\$'; i += 2; continue; }
      if (next === '[' || next === '(') {
        const disp = next === '[';
        const end = findClosingDelim(text, disp ? '\\]' : '\\)', i + 2);
        if (end !== -1) { out += push(text.slice(i + 2, end), disp); i = end + 2; continue; }
      }
      if (text.startsWith('\\begin{', i)) {
        const bEnd = text.indexOf('}', i + 7);
        const env = bEnd !== -1 ? text.slice(i + 7, bEnd) : '';
        if (MATH_ENVS.has(env)) {
          const end = findMatchingEnvEnd(text, env, bEnd + 1);
          if (end !== -1) { out += push(text.slice(i, end), true); i = end; continue; }
        }
      }
    }
    if (ch === '$' && text[i + 1] === '$' && !isEscapedAt(text, i)) {
      const end = findClosingDelim(text, '$$', i + 2);
      if (end !== -1 && text.slice(i + 2, end).trim()) { out += push(text.slice(i + 2, end), true); i = end + 2; continue; }
    }
    if (allowDollar && ch === '$' && text[i + 1] !== '$' && !isEscapedAt(text, i)) {
      const end = findClosingInlineDollar(text, i + 1);
      if (end !== -1) { out += push(text.slice(i + 1, end), false); i = end + 1; continue; }
    }
    out += ch; i += 1;
  }
  return out;
}

export function renderMathInContainer(el, opts = {}) {
  if (!el || typeof window === 'undefined') return;
  const allowSingleDollar = opts.allowSingleDollar !== false;
  const delimiters = allowSingleDollar ? DELIMITERS : DELIMITERS_NO_DOLLAR;
  if (typeof window.renderMathInElement === 'function') {
    window.renderMathInElement(el, { delimiters, throwOnError: false, strict: false, preProcess: (t) => normalizeMathTex(t, true) });
    return;
  }
  if (window.katex && typeof window.katex.renderToString === 'function') {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const nodes = [];
    for (let cur = walker.nextNode(); cur; cur = walker.nextNode()) {
      const p = cur.parentElement;
      if (p && !/^(CODE|PRE|SCRIPT|STYLE|TEXTAREA)$/.test(p.tagName) && /(\$\$|\$|\\\(|\\\[|\\begin\{)/.test(cur.nodeValue || '')) {
        nodes.push(cur);
      }
    }
    for (const node of nodes) {
      const span = document.createElement('span');
      span.innerHTML = renderInlineMathFallback(node.nodeValue || '', { allowSingleDollar });
      node.parentNode.replaceChild(span, node);
    }
  }
}

function renderInlineMathFallback(text, opts = {}) {
  const tokens = [];
  const marked = extractMath(String(text ?? '').replace(/\u0000/g, ''), tokens, {
    protectInlineCode: false, allowSingleDollar: opts.allowSingleDollar !== false,
  });
  let out = '';
  let last = 0;
  for (const m of marked.matchAll(/\u0000M(\d+)\u0000/g)) {
    out += escapeHtml(marked.slice(last, m.index));
    const tok = tokens[Number(m[1])];
    if (tok) {
      try { out += window.katex.renderToString(tok.tex, { displayMode: tok.display, throwOnError: false, strict: false }); }
      catch { out += escapeHtml(tok.display ? `$$${tok.tex}$$` : `\\(${tok.tex}\\)`); }
    }
    last = m.index + m[0].length;
  }
  return out + escapeHtml(marked.slice(last));
}

function formatMathTokenHtml(tok) {
  return !tok ? '' : tok.display ? `$$${escapeHtml(tok.tex)}$$` : `\\(${escapeHtml(tok.tex)}\\)`;
}

function applyEmphasis(escapedText) {
  return escapedText
    .replace(/\*\*\*(?=\S)([^*\n]*?\S)\*\*\*/g, '<strong><em>$1</em></strong>')
    .replace(/\*\*(?=\S)(.+?)(?<=\S)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[^\w])__(?=\S)(.+?)(?<=\S)__(?=[^\w]|$)/g, '$1<strong>$2</strong>')
    .replace(/(^|[^\w*])\*(?=\S)([^*\n]*?\S)\*(?=[^\w*]|$)/g, '$1<em>$2</em>')
    .replace(/(^|[^\w])_(?=\S)([^_\n]*?\S)_(?=[^\w]|$)/g, '$1<em>$2</em>');
}

function renderInlineMarkdown(text, mathTokens) {
  const codeTokens = [];
  const linkTokens = [];
  let s = String(text ?? '')
    .replace(/(`+)(?!`)([^\n]+?)(?<!`)\1(?!`)/g, (_, _t, code) => {
      codeTokens.push(`<code>${escapeHtml(code)}</code>`);
      return `\u0000C${codeTokens.length - 1}\u0000`;
    })
    .replace(/\\\$/g, '\u0000D\u0000')
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)<>"'\u0000]+)\)/g, (_, label, url) => {
      linkTokens.push(`<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">${applyEmphasis(escapeHtml(label))}</a>`);
      return `\u0000L${linkTokens.length - 1}\u0000`;
    });
  return applyEmphasis(escapeHtml(s))
    .replace(/\u0000L(\d+)\u0000/g, (_, i) => linkTokens[Number(i)] || '')
    .replace(/\u0000C(\d+)\u0000/g, (_, i) => codeTokens[Number(i)] || '')
    .replace(/\u0000D\u0000/g, '$')
    .replace(/\u0000M(\d+)\u0000/g, (_, i) => formatMathTokenHtml(mathTokens[Number(i)]));
}

function extractFencedCodeBlocks(src, codeBlocks) {
  const lines = src.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    const open = lines[i].match(/^[ \t]{0,3}(`{3,}|~{3,})([^\n]*)$/);
    if (!open) { out.push(lines[i]); continue; }
    const closeRx = new RegExp(`^[ \\t]{0,3}${open[1][0]}{${open[1].length},}[ \\t]*$`);
    const lang = open[2].trim().split(/\s+/)[0] || '';
    const body = [];
    for (i += 1; i < lines.length && !closeRx.test(lines[i]); i++) body.push(lines[i]);
    const cls = lang ? ` class="lang-${escapeHtml(lang)}"` : '';
    codeBlocks.push(`<pre><code${cls}>${escapeHtml(body.join('\n'))}</code></pre>`);
    out.push(`\u0000B${codeBlocks.length - 1}\u0000`);
  }
  return out.join('\n');
}

function splitTableRow(line) {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !isEscapedAt(s, s.length - 1)) s = s.slice(0, -1);
  const cells = [];
  let cur = '';
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\' && s[i + 1] === '|') { cur += '|'; i += 1; }
    else if (s[i] === '|') { cells.push(cur.trim()); cur = ''; }
    else cur += s[i];
  }
  cells.push(cur.trim());
  return cells;
}

function isTableDelimiterLine(line) {
  const t = line.trim();
  if (!t.includes('-')) return false;
  const cells = splitTableRow(t);
  return cells.length >= 1 && cells.every((c) => /^:?-{3,}:?$/.test(c));
}

function renderParagraphLines(paraLines, mathTokens) {
  let html = '';
  for (let k = 0; k < paraLines.length; k++) {
    const raw = paraLines[k];
    const hasNext = k < paraLines.length - 1;
    if (hasNext && / {2,}$/.test(raw)) html += `${renderInlineMarkdown(raw.replace(/ {2,}$/, ''), mathTokens)}<br>\n`;
    else if (hasNext && raw.endsWith('\\') && !isEscapedAt(raw, raw.length - 1)) html += `${renderInlineMarkdown(raw.slice(0, -1), mathTokens)}<br>\n`;
    else html += renderInlineMarkdown(raw, mathTokens) + (hasNext ? ' ' : '');
  }
  return `<p>${html}</p>`;
}

function renderMarkdownBlocks(lines, codeBlocks, mathTokens) {
  const out = [];
  let para = [];
  let quote = [];
  const lists = [];

  const flushPara = () => { if (para.length) { out.push(renderParagraphLines(para, mathTokens)); para = []; } };
  const flushQuote = () => { if (quote.length) { out.push(`<blockquote>${renderMarkdownBlocks(quote, codeBlocks, mathTokens)}</blockquote>`); quote = []; } };
  const closeLists = () => { while (lists.length) out.push(`</li></${lists.pop().tag}>`); };
  const flushAll = () => { flushPara(); flushQuote(); closeLists(); };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const bMatch = line.match(/^\s*\u0000B(\d+)\u0000\s*$/);
    if (bMatch) { flushAll(); out.push(codeBlocks[Number(bMatch[1])] || ''); continue; }
    const mMatch = line.match(/^\s*\u0000M(\d+)\u0000\s*$/);
    if (mMatch && mathTokens[Number(mMatch[1])]?.display) {
      flushAll();
      out.push(`<div class="math-block">$$\n${escapeHtml(mathTokens[Number(mMatch[1])].tex)}\n$$</div>`);
      continue;
    }
    if (!line.trim()) { flushAll(); continue; }
    if (/^\s*>\s?/.test(line)) { flushPara(); closeLists(); quote.push(line.replace(/^\s*>\s?/, '')); continue; }
    flushQuote();

    const hMatch = line.match(/^(#{1,6})\s+(.+)$/);
    if (hMatch) { flushPara(); closeLists(); out.push(`<h${hMatch[1].length}>${renderInlineMarkdown(hMatch[2], mathTokens)}</h${hMatch[1].length}>`); continue; }
    if (/^[ \t]*(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/.test(line)) { flushPara(); closeLists(); out.push('<hr>'); continue; }
    if (line.includes('|') && i + 1 < lines.length && isTableDelimiterLine(lines[i + 1])) {
      flushPara(); closeLists();
      const headers = splitTableRow(line);
      const aligns = splitTableRow(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : 'left'));
      const ths = headers.map((c, idx) => `<th style="text-align:${aligns[idx] || 'left'}">${renderInlineMarkdown(c, mathTokens)}</th>`).join('');
      const rows = [];
      for (i += 2; i < lines.length && lines[i].trim() && lines[i].includes('|') && !/^\s*\u0000B\d+\u0000\s*$/.test(lines[i]); i++) {
        const cols = splitTableRow(lines[i]);
        rows.push(`<tr>${headers.map((_, idx) => `<td style="text-align:${aligns[idx] || 'left'}">${renderInlineMarkdown(cols[idx] ?? '', mathTokens)}</td>`).join('')}</tr>`);
      }
      i -= 1;
      out.push(`<table><thead><tr>${ths}</tr></thead><tbody>${rows.join('')}</tbody></table>`);
      continue;
    }
    const ulMatch = line.match(/^([ \t]*)([-*+])\s+(.+)$/);
    const olMatch = !ulMatch && line.match(/^([ \t]*)\d+[.)]\s+(.+)$/);
    if (ulMatch || olMatch) {
      flushPara();
      const tag = ulMatch ? 'ul' : 'ol';
      const indent = (ulMatch ? ulMatch[1] : olMatch[1]).replace(/\t/g, '  ').length;
      const body = renderInlineMarkdown(ulMatch ? ulMatch[3] : olMatch[2], mathTokens);
      while (lists.length && indent < lists[lists.length - 1].indent) out.push(`</li></${lists.pop().tag}>`);
      if (lists.length && indent === lists[lists.length - 1].indent && lists[lists.length - 1].tag !== tag) {
        out.push(`</li></${lists.pop().tag}>`);
      }
      if (!lists.length || indent > lists[lists.length - 1].indent) {
        lists.push({ tag, indent });
        out.push(`<${tag}><li>${body}`);
      } else {
        out.push(`</li>\n<li>${body}`);
      }
      continue;
    }
    closeLists();
    para.push(line);
  }
  flushAll();
  return out.join('\n');
}

export function markdownToHtml(raw) {
  const src = String(raw ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n');
  const codeBlocks = [];
  const afterFences = extractFencedCodeBlocks(src, codeBlocks);
  const mathTokens = [];
  const text = extractMath(afterFences, mathTokens, { protectInlineCode: true });
  return renderMarkdownBlocks(text.split('\n'), codeBlocks, mathTokens);
}

function stripTexComments(src) {
  const kept = [];
  for (const line of src.split('\n')) {
    if (/^[ \t]*%/.test(line)) continue;
    let cut = line.length;
    for (let i = 0; i < line.length; i++) {
      if (line[i] === '\\') { i += 1; continue; }
      if (line[i] === '%') { cut = i; break; }
    }
    kept.push(line.slice(0, cut));
  }
  return kept.join('\n');
}

function readBalancedGroup(src, openIdx, openChar = '{', closeChar = '}') {
  if (src[openIdx] !== openChar) return null;
  let depth = 1;
  for (let i = openIdx + 1; i < src.length; i++) {
    if (src[i] === '\\') { i += 1; continue; }
    if (src[i] === openChar) depth += 1;
    else if (src[i] === closeChar && --depth === 0) return { content: src.slice(openIdx + 1, i), end: i + 1 };
  }
  return null;
}

function extractTexCommand(src, cmdName) {
  const m = new RegExp(`\\\\${cmdName}\\*?\\s*\\{`).exec(src);
  const grp = m ? readBalancedGroup(src, m.index + m[0].length - 1) : null;
  return grp ? grp.content.trim() : '';
}

function renderTexInline(src, mathTokens) {
  let out = '';
  let i = 0;
  while (i < src.length) {
    if (src[i] === '\u0000') {
      const m = src.slice(i).match(/^\u0000M(\d+)\u0000/);
      if (m) { out += formatMathTokenHtml(mathTokens[Number(m[1])]); i += m[0].length; continue; }
    }
    if (src[i] === '~') { out += '&nbsp;'; i += 1; continue; }
    if (src.startsWith('``', i)) { out += '&ldquo;'; i += 2; continue; }
    if (src.startsWith("''", i)) { out += '&rdquo;'; i += 2; continue; }
    if (src.startsWith('---', i)) { out += '&mdash;'; i += 3; continue; }
    if (src.startsWith('--', i)) { out += '&ndash;'; i += 2; continue; }
    if (src[i] === '\\') {
      const next = src[i + 1] || '';
      if (next === '\\') {
        i += 2;
        if (src[i] === '[') { const opt = readBalancedGroup(src, i, '[', ']'); if (opt) i = opt.end; }
        out += '<br>'; continue;
      }
      if ('%$&_#{}'.includes(next)) { out += escapeHtml(next); i += 2; continue; }
      const cmdMatch = src.slice(i).match(/^\\([a-zA-Z]+)\*?/);
      if (cmdMatch) {
        const cmd = cmdMatch[1];
        let pos = i + cmdMatch[0].length;
        if (cmd === 'newline') { out += '<br>'; i = pos; continue; }
        if (cmd === 'quad' || cmd === 'qquad') { out += '&emsp;'; i = pos; continue; }
        if (/^(?:noindent|medskip|bigskip|smallskip|hfill|centering|maketitle|tableofcontents)$/.test(cmd)) { i = pos; continue; }
        while (src[pos] === ' ') pos += 1;
        let optArg = '';
        if (src[pos] === '[') { const opt = readBalancedGroup(src, pos, '[', ']'); if (opt) { optArg = opt.content; pos = opt.end; } }
        const grp = src[pos] === '{' ? readBalancedGroup(src, pos) : null;
        if (grp) {
          if (cmd === 'textbf') { out += `<strong>${renderTexInline(grp.content, mathTokens)}</strong>`; i = grp.end; continue; }
          if (cmd === 'textit' || cmd === 'emph') { out += `<em>${renderTexInline(grp.content, mathTokens)}</em>`; i = grp.end; continue; }
          if (cmd === 'texttt') { out += `<code>${escapeHtml(grp.content)}</code>`; i = grp.end; continue; }
          if (cmd === 'cite') { out += `<code>[cite:${escapeHtml(grp.content + (optArg ? `, ${optArg}` : ''))}]</code>`; i = grp.end; continue; }
          if (/^(?:eq|auto|c|C|page)?ref$/.test(cmd)) { out += `<code>[ref:${escapeHtml(grp.content)}]</code>`; i = grp.end; continue; }
          if (/^(?:label|bibliographystyle|bibliography|documentclass|usepackage|date|title|author)$/.test(cmd)) { i = grp.end; continue; }
        }
      }
    }
    out += escapeHtml(src[i]); i += 1;
  }
  return out;
}

function renderTexBlocks(src, mathTokens) {
  const out = [];
  let para = [];
  const flushPara = () => {
    const joined = para.map((l) => l.trim()).filter(Boolean).join(' ');
    if (joined) out.push(`<p>${renderTexInline(joined, mathTokens)}</p>`);
    para = [];
  };
  let i = 0;
  while (i < src.length) {
    const rest = src.slice(i);
    const envOpen = rest.match(/^[ \t]*\\begin\{([a-zA-Z*]+)\}(?:\[([^\]]*)\])?/);
    if (envOpen) {
      const env = envOpen[1];
      const endPos = findMatchingEnvEnd(src, env, i + envOpen[0].length);
      if (endPos !== -1 && (TEX_STRUCT_ENVS.has(env) || env === 'itemize' || env === 'enumerate')) {
        flushPara();
        const inner = src.slice(i + envOpen[0].length, endPos - `\\end{${env}}`.length);
        if (env === 'itemize' || env === 'enumerate') {
          const tag = env === 'itemize' ? 'ul' : 'ol';
          const items = inner.split(/\\item(?:\[[^\]]*\])?/).slice(1)
            .map((it) => `<li>${renderTexInline(it.trim().replace(/\n+/g, ' '), mathTokens)}</li>`).join('');
          out.push(`<${tag}>${items}</${tag}>`);
        } else {
          const label = env[0].toUpperCase() + env.slice(1);
          const suffix = envOpen[2] ? ` (${renderTexInline(envOpen[2], mathTokens)})` : '';
          out.push(`<div class="tex-env tex-env-${escapeHtml(env)}"><div class="tex-env-title">${escapeHtml(label)}${suffix}</div>${renderTexBlocks(inner, mathTokens)}</div>`);
        }
        i = endPos; continue;
      }
    }
    const secMatch = rest.match(/^[ \t]*\\(section|subsection|subsubsection|paragraph)\*?\s*\{/);
    if (secMatch) {
      const grp = readBalancedGroup(src, i + secMatch[0].length - 1);
      if (grp) {
        flushPara();
        const lvl = { section: 2, subsection: 3, subsubsection: 4, paragraph: 5 }[secMatch[1]];
        out.push(`<h${lvl}>${renderTexInline(grp.content, mathTokens)}</h${lvl}>`);
        i = grp.end; continue;
      }
    }
    const nl = src.indexOf('\n', i);
    const line = src.slice(i, nl === -1 ? src.length : nl);
    i = nl === -1 ? src.length : nl + 1;
    const mMatch = line.match(/^\s*\u0000M(\d+)\u0000\s*$/);
    if (mMatch && mathTokens[Number(mMatch[1])]?.display) {
      flushPara();
      out.push(`<div class="math-block">$$\n${escapeHtml(mathTokens[Number(mMatch[1])].tex)}\n$$</div>`);
      continue;
    }
    if (!line.trim() || /^\s*\\par\b/.test(line)) { flushPara(); continue; }
    if (/^\s*\\(?:maketitle|tableofcontents|documentclass|usepackage|bibliographystyle|bibliography|date|title|author|label)\b/.test(line)) {
      if (renderTexInline(line, mathTokens).trim()) para.push(line);
      continue;
    }
    para.push(line);
  }
  flushPara();
  return out.join('\n');
}

export function latexDocumentToHtml(texSource) {
  const cleaned = stripTexComments(String(texSource ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n'));
  if (!cleaned.trim()) return '';
  const beginDoc = cleaned.indexOf('\\begin{document}');
  const preamble = beginDoc >= 0 ? cleaned.slice(0, beginDoc) : '';
  let body = beginDoc >= 0 ? cleaned.slice(beginDoc + '\\begin{document}'.length) : cleaned;
  const endDoc = body.indexOf('\\end{document}');
  if (endDoc >= 0) body = body.slice(0, endDoc);

  const rawTitle = extractTexCommand(preamble, 'title') || extractTexCommand(body, 'title');
  const rawAuthor = (extractTexCommand(preamble, 'author') || extractTexCommand(body, 'author')).replace(/\s*\\and\s*/g, ', ');
  const mathTokens = [];
  const titleMarked = extractMath(rawTitle, mathTokens, { protectInlineCode: false });
  const authorMarked = extractMath(rawAuthor, mathTokens, { protectInlineCode: false });
  const bodyMarked = extractMath(body, mathTokens, { protectInlineCode: false });

  const parts = [];
  if (titleMarked || authorMarked) {
    const h1 = titleMarked ? `<h1>${renderTexInline(titleMarked, mathTokens)}</h1>` : '';
    const auth = authorMarked ? `<p class="tex-doc-author">${renderTexInline(authorMarked, mathTokens)}</p>` : '';
    parts.push(`<header class="tex-doc-header">${h1}${auth}</header>`);
  }
  const bodyHtml = renderTexBlocks(bodyMarked, mathTokens);
  if (bodyHtml) parts.push(bodyHtml);
  return parts.join('\n');
}

export function renderRichInto(el, rawText) {
  if (!el) return;
  el.innerHTML = markdownToHtml(rawText);
  renderMathInContainer(el, { allowSingleDollar: false });
}

export function renderLatexDocumentPreview(el, texSource) {
  if (!el) return;
  const html = latexDocumentToHtml(texSource);
  if (!html) { el.innerHTML = '<p style="color:var(--ink-muted)">Empty TeX buffer.</p>'; return; }
  el.innerHTML = html;
  renderMathInContainer(el, { allowSingleDollar: false });
}

export function highlightUnifiedDiff(diffText) {
  return String(diffText ?? '').split('\n').map((line) => {
    const cls = line.startsWith('+++') || line.startsWith('---') || line.startsWith('@@')
      ? ' diff-hunk' : line.startsWith('+') ? ' diff-add' : line.startsWith('-') ? ' diff-del' : '';
    return `<span class="diff-line${cls}">${escapeHtml(line)}</span>`;
  }).join('');
}

export function highlightPython(codeText) {
  const kw = /\b(import|from|as|def|return|if|elif|else|for|in|while|with|try|except|True|False|None|and|or|not)\b/g;
  return String(codeText ?? '').split('\n').map((line) => {
    const hashIdx = line.indexOf('#');
    const head = hashIdx >= 0 ? line.slice(0, hashIdx) : line;
    const tail = hashIdx >= 0 ? line.slice(hashIdx) : '';
    let html = escapeHtml(head)
      .replace(/(&quot;[^&]*?&quot;|&#39;[^&]*?&#39;)/g, '<span class="py-str">$1</span>')
      .replace(kw, '<span class="py-kw">$1</span>');
    if (tail) html += `<span class="py-com">${escapeHtml(tail)}</span>`;
    return `<span class="diff-line">${html}</span>`;
  }).join('');
}
