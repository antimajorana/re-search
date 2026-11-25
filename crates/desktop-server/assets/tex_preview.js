// Offline LaTeX document preview renderer.
import { escapeHtml } from './code_highlight.js';
import { extractMath, findMatchingEnvEnd, formatMathTokenHtml } from './math_scan.js';

const TEX_STRUCT_ENVS = new Set(
  'abstract theorem lemma proposition corollary definition remark example proof conjecture assumption claim note'.split(' ')
);

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
