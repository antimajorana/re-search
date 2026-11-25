// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.
// Offline Markdown parser and HTML renderer with container-aware block display math, lists, tables, and headings.
import { escapeHtml } from './code_highlight.js';
import {
  HEADING_LINE_RE, MATH_ENVS, extractMath, findClosingDelim, findDisplayClose,
  findEnvBlockClose, formatMathTokenHtml, isEscapedAt, normalizeMathTex,
} from './math_scan.js';
import { renderFormulaHtml } from './math_katex.js';
import { numberEquations } from './math_equations.js';

const CITATION_BODY_RE = /^[\d,\s\u2013\u2014-]+$/;
const FENCE_OPEN_RE = /^[ \t]*(?:(?:[-*+]|\d+[.)])[ \t]+)?(`{3,}|~{3,})([^\r\n]*)$/;

function parseQuotePrefix(line) {
  let depth = 0;
  let pos = 0;
  while (pos < line.length) {
    const m = /^[ \t]*>[ \t]?/.exec(line.slice(pos));
    if (!m) break;
    depth += 1;
    pos += m[0].length;
  }
  return { depth, rest: line.slice(pos) };
}

function isFenceClose(rest, fence) {
  const m = /^[ \t]*(`{3,}|~{3,})[ \t]*$/.exec(rest);
  return Boolean(m && m[1][0] === fence.char && m[1].length >= fence.len);
}

/** Strips HTML comments (`<!-- ... -->`) outside fenced and inline code. */
export function stripComments(text) {
  const s = String(text ?? '');
  let out = '';
  let i = 0;
  let atLineStart = true;
  let inFence = null;

  while (i < s.length) {
    if (atLineStart) {
      const nl = s.indexOf('\n', i);
      const rawLine = nl === -1 ? s.slice(i) : s.slice(i, nl);
      const line = rawLine.endsWith('\r') ? rawLine.slice(0, -1) : rawLine;
      const { depth, rest } = parseQuotePrefix(line);
      if (inFence) {
        if (depth >= inFence.depth) {
          out += rawLine + (nl === -1 ? '' : '\n');
          i = nl === -1 ? s.length : nl + 1;
          if (depth === inFence.depth && isFenceClose(rest, inFence)) inFence = null;
          continue;
        }
        inFence = null;
      }
      const open = FENCE_OPEN_RE.exec(rest);
      if (open && (open[1][0] === '~' || !open[2].includes('`'))) {
        inFence = { char: open[1][0], len: open[1].length, depth };
        out += rawLine + (nl === -1 ? '' : '\n');
        i = nl === -1 ? s.length : nl + 1;
        continue;
      }
    }
    const ch = s[i];
    if (ch === '\n') { out += '\n'; i += 1; atLineStart = true; continue; }
    if (ch === '`') {
      let ticks = 1;
      while (s[i + ticks] === '`') ticks += 1;
      const marker = '`'.repeat(ticks);
      const nl = s.indexOf('\n', i + ticks);
      const limit = nl === -1 ? s.length : nl;
      let close = s.indexOf(marker, i + ticks);
      while (close !== -1 && close < limit && (s[close - 1] === '`' || s[close + ticks] === '`')) {
        close = s.indexOf(marker, close + 1);
      }
      if (close !== -1 && close < limit) {
        out += s.slice(i, close + ticks);
        i = close + ticks;
      } else {
        out += marker;
        i += ticks;
      }
      atLineStart = false;
      continue;
    }
    if (ch === '<' && s.startsWith('<!--', i)) {
      const end = s.indexOf('-->', i + 4);
      if (end !== -1) { i = end + 3; atLineStart = false; continue; }
    }
    out += ch;
    i += 1;
    atLineStart = false;
  }
  return out;
}

export function applyEmphasis(escapedText) {
  return escapedText
    .replace(/(^|[^\w*])\*\*\*(?=\S)([^*\n]*?\S)\*\*\*(?=[^\w*]|$)/g, '$1<strong><em>$2</em></strong>')
    .replace(/(^|[^\w*])\*\*(?=\S)(.+?)(?<=\S)\*\*(?=[^\w*]|$)/g, '$1<strong>$2</strong>')
    .replace(/(^|[^\w])__(?=\S)(.+?)(?<=\S)__(?=[^\w]|$)/g, '$1<strong>$2</strong>')
    .replace(/(^|[^\w*])\*(?=\S)([^*\n]*?\S)\*(?=[^\w*]|$)/g, '$1<em>$2</em>')
    .replace(/(^|[^\w])_(?=\S)([^_\n]*?\S)_(?=[^\w]|$)/g, '$1<em>$2</em>');
}

function scanInlineText(text, mathTokens, parseOpts) {
  return extractMath(text, mathTokens, {
    protectInlineCode: true,
    allowSingleDollar: true,
    captureGlue: parseOpts.rendered,
    dropFailedEscapes: parseOpts.rendered,
    preserveRaw: parseOpts.rendered,
    midSentenceEnv: true,
  });
}

function renderInlineMarkdown(text, mathTokens, parseOpts, alreadyExtracted = false) {
  const marked = alreadyExtracted ? String(text ?? '') : scanInlineText(String(text ?? ''), mathTokens, parseOpts);
  const codeTokens = [];
  const linkTokens = [];
  let s = marked
    .replace(/(`+)(?!`)([^\n]+?)(?<!`)\1(?!`)/g, (_, _t, code) => {
      codeTokens.push(`<code>${escapeHtml(code)}</code>`);
      return `\u0000C${codeTokens.length - 1}\u0000`;
    })
    .replace(/\\\$/g, '\u0000D\u0000')
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)<>"'\u0000]+)\)/g, (_, label, url) => {
      linkTokens.push(`<a href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer">${applyEmphasis(escapeHtml(label))}</a>`);
      return `\u0000L${linkTokens.length - 1}\u0000`;
    });
  if (parseOpts.rendered && parseOpts.eqrefTokens) {
    s = s.replace(/\\eqref\{([^}\n{}]+)\}/g, (m, rawId) => {
      const id = rawId.trim();
      if (!id) return m;
      const idx = parseOpts.eqrefTokens.length;
      parseOpts.eqrefTokens.push(id);
      return `\u0000E${idx}\u0000`;
    });
  }
  return applyEmphasis(escapeHtml(s))
    .replace(/\u0000L(\d+)\u0000/g, (_, i) => linkTokens[Number(i)] || '')
    .replace(/\u0000C(\d+)\u0000/g, (_, i) => codeTokens[Number(i)] || '')
    .replace(/\u0000D\u0000/g, '$');
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
  if (!t.includes('-') || !t.includes('|')) return false;
  const cells = splitTableRow(t);
  return cells.length >= 1 && cells.every((c) => /^:?-{1,}:?$/.test(c));
}

function renderParagraphLines(paraLines, mathTokens, parseOpts) {
  const joined = paraLines.join('\n');
  const marked = scanInlineText(joined, mathTokens, parseOpts);
  const segs = marked.split('\n');
  let html = '';
  for (let k = 0; k < segs.length; k++) {
    const raw = segs[k];
    const hasNext = k < segs.length - 1;
    if (hasNext && / {2,}$/.test(raw)) html += `${renderInlineMarkdown(raw.replace(/ {2,}$/, ''), mathTokens, parseOpts, true)}<br>\n`;
    else if (hasNext && raw.endsWith('\\') && !isEscapedAt(raw, raw.length - 1)) html += `${renderInlineMarkdown(raw.slice(0, -1), mathTokens, parseOpts, true)}<br>\n`;
    else html += renderInlineMarkdown(raw, mathTokens, parseOpts, true) + (hasNext ? ' ' : '');
  }
  return `<p>${html}</p>`;
}

function matchBlockDisplay(lines, i) {
  const line = lines[i];
  const dd = /^ {0,3}\$\$/.exec(line);
  if (dd) {
    const found = findDisplayClose(lines, i, dd[0].length, '$$');
    if (found && found.body.trim()) return found;
  }
  const br = /^ {0,3}\\\[(?!\[)/.exec(line);
  if (br) {
    const found = findDisplayClose(lines, i, br[0].length, '\\]');
    if (found && found.body.trim() && !(found.closeLine === i && CITATION_BODY_RE.test(found.body.trim()))) {
      return found;
    }
  }
  const envOpen = /^ {0,3}\\begin\{([a-zA-Z*]+)\}/.exec(line);
  if (envOpen && MATH_ENVS.has(envOpen[1])) {
    const openStart = line.indexOf('\\begin{');
    const found = findEnvBlockClose(lines, i, openStart, envOpen[1]);
    if (found && found.body.trim()) return found;
  }
  return null;
}

function findUnclosedParaDisplay(paraLines) {
  if (!paraLines.length) return null;
  const text = paraLines.join('\n');
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (ch === '`') {
      let ticks = 1;
      while (text[i + ticks] === '`') ticks += 1;
      const marker = '`'.repeat(ticks);
      let close = text.indexOf(marker, i + ticks);
      while (close !== -1 && (text[close - 1] === '`' || text[close + ticks] === '`')) {
        close = text.indexOf(marker, close + 1);
      }
      if (close !== -1 && !text.slice(i + ticks, close).includes('\n\n')) { i = close + ticks; continue; }
      i += ticks;
      continue;
    }
    if (ch === '\\') {
      const next = text[i + 1];
      if (next === '\\' || next === '$') { i += 2; continue; }
      if (next === '[') {
        const end = findClosingDelim(text, '\\]', i + 2);
        if (end === -1) return { close: '\\]', hasBody: Boolean(text.slice(i + 2).trim()) };
        i = end + 2;
        continue;
      }
      if (next === '(') {
        const end = findClosingDelim(text, '\\)', i + 2);
        i = end !== -1 ? end + 2 : i + 2;
        continue;
      }
      i += 1;
      continue;
    }
    if (ch === '$' && text[i + 1] === '$') {
      const end = findClosingDelim(text, '$$', i + 2);
      if (end === -1) return { close: '$$', hasBody: Boolean(text.slice(i + 2).trim()) };
      if (text.slice(i + 2, end).trim()) { i = end + 2; continue; }
      i += 2;
      continue;
    }
    i += 1;
  }
  return null;
}

function findParaDisplayClose(lines, startIdx, openPara) {
  let hasBody = openPara.hasBody;
  for (let k = startIdx; k < lines.length; k++) {
    const line = lines[k];
    if (!line.trim() || HEADING_LINE_RE.test(line) || /^[ \t]{0,3}(?:`{3,}|~{3,})/.test(line) || /^\s*>\s?/.test(line)) {
      return -1;
    }
    const col = findClosingDelim(line, openPara.close, 0);
    if (col !== -1) return (hasBody || line.slice(0, col).trim()) ? k : -1;
    if (line.trim()) hasBody = true;
  }
  return -1;
}

function leadingIndent(line) {
  const m = /^[ \t]*/.exec(line);
  return m ? m[0].replace(/\t/g, '  ').length : 0;
}

function stripIndent(line, cols) {
  let removed = 0;
  let i = 0;
  while (i < line.length && removed < cols) {
    if (line[i] === ' ') { removed += 1; i += 1; }
    else if (line[i] === '\t') { removed += 2; i += 1; }
    else break;
  }
  return line.slice(i);
}

function parseListMarker(line) {
  const ul = /^([ \t]*)([-*+])[ \t]+(.*)$/.exec(line);
  if (ul) {
    const indent = ul[1].replace(/\t/g, '  ').length;
    const afterMarker = line.slice(ul[1].length + 1);
    const spaces = (/^[ \t]+/.exec(afterMarker)?.[0] || ' ').replace(/\t/g, '  ').length;
    return { tag: 'ul', indent, contentIndent: indent + 1 + spaces, start: 1, firstLine: ul[3] };
  }
  const ol = /^([ \t]*)(\d+)[.)][ \t]+(.*)$/.exec(line);
  if (ol) {
    const indent = ol[1].replace(/\t/g, '  ').length;
    const markerLen = ol[2].length + 1;
    const afterMarker = line.slice(ol[1].length + markerLen);
    const spaces = (/^[ \t]+/.exec(afterMarker)?.[0] || ' ').replace(/\t/g, '  ').length;
    return {
      tag: 'ol',
      indent,
      contentIndent: indent + markerLen + spaces,
      start: Number.parseInt(ol[2], 10),
      firstLine: ol[3],
    };
  }
  return null;
}

function formatListItemHtml(itemLines, mathTokens, parseOpts) {
  const body = renderMarkdownBlocks(itemLines, mathTokens, parseOpts);
  const singlePara = /^<p>([\s\S]*?)<\/p>$/.exec(body);
  if (singlePara && !singlePara[1].includes('</p>')) return `<li>${singlePara[1]}</li>`;
  const leadParaList = /^<p>([\s\S]*?)<\/p>\n(<(?:ul|ol)\b[\s\S]*)$/.exec(body);
  if (leadParaList && !leadParaList[1].includes('</p>')) return `<li>${leadParaList[1]}\n${leadParaList[2]}</li>`;
  return `<li>${body}</li>`;
}

function consumeList(lines, startIdx, mathTokens, parseOpts) {
  const first = parseListMarker(lines[startIdx]);
  const { tag, indent: listIndent, start: startNum } = first;
  const items = [];
  let i = startIdx;

  while (i < lines.length) {
    const m = parseListMarker(lines[i]);
    if (!m || m.tag !== tag || m.indent !== listIndent) break;
    const itemLines = [m.firstLine];
    const { contentIndent } = m;
    let inFence = null;
    i += 1;

    while (i < lines.length) {
      const cur = lines[i];
      if (inFence) {
        const stripped = stripIndent(cur, contentIndent);
        itemLines.push(stripped);
        if (inFence.test(stripped)) inFence = null;
        i += 1;
        continue;
      }
      if (!cur.trim()) {
        let j = i + 1;
        while (j < lines.length && !lines[j].trim()) j += 1;
        if (j >= lines.length) break;
        const nextLine = lines[j];
        const nextMarker = parseListMarker(nextLine);
        if (nextMarker && nextMarker.tag === tag && nextMarker.indent === listIndent) {
          i = j;
          break;
        }
        if (leadingIndent(nextLine) >= contentIndent || (nextMarker && nextMarker.indent > listIndent)) {
          for (let k = i; k < j; k++) itemLines.push('');
          i = j;
          continue;
        }
        break;
      }
      const curIndent = leadingIndent(cur);
      const curMarker = parseListMarker(cur);
      if (curIndent >= contentIndent) {
        const stripped = stripIndent(cur, contentIndent);
        const fence = stripped.match(/^[ \t]{0,3}(`{3,}|~{3,})([^\n]*)$/);
        if (fence) inFence = new RegExp(`^[ \\t]{0,3}${fence[1][0]}{${fence[1].length},}[ \\t]*$`);
        itemLines.push(stripped);
        i += 1;
        continue;
      }
      if (curMarker && curMarker.indent > listIndent) {
        itemLines.push(stripIndent(cur, Math.min(curIndent, contentIndent)));
        i += 1;
        continue;
      }
      break;
    }
    items.push(formatListItemHtml(itemLines, mathTokens, parseOpts));
  }

  const startAttr = tag === 'ol' && Number.isInteger(startNum) && startNum !== 1 ? ` start="${startNum}"` : '';
  return { html: `<${tag}${startAttr}>${items.join('\n')}</${tag}>`, nextIdx: i };
}

function matchAtxHeading(line) {
  const m = /^ {0,3}(#{1,6})[ \t]+(.+)$/.exec(line);
  if (!m) return null;
  const text = m[2].replace(/[ \t]+#+[ \t]*$/, '').trim();
  return text ? { level: m[1].length, text } : null;
}

function renderMarkdownBlocks(inputLines, mathTokens, parseOpts) {
  const lines = inputLines.slice();
  const out = [];
  let para = [];
  let quote = [];

  const flushPara = () => { if (para.length) { out.push(renderParagraphLines(para, mathTokens, parseOpts)); para = []; } };
  const flushQuote = () => { if (quote.length) { out.push(`<blockquote>${renderMarkdownBlocks(quote, mathTokens, parseOpts)}</blockquote>`); quote = []; } };
  const flushAll = () => { flushPara(); flushQuote(); };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Fenced code block (``` or ~~~)
    const fenceOpen = line.match(/^[ \t]{0,3}(`{3,}|~{3,})([^\n]*)$/);
    if (fenceOpen) {
      flushAll();
      const closeRx = new RegExp(`^[ \\t]{0,3}${fenceOpen[1][0]}{${fenceOpen[1].length},}[ \\t]*$`);
      const lang = fenceOpen[2].trim().split(/\s+/)[0] || '';
      const body = [];
      for (i += 1; i < lines.length && !closeRx.test(lines[i]); i++) body.push(lines[i]);
      const cls = lang ? ` class="lang-${escapeHtml(lang)}"` : '';
      out.push(`<pre><code${cls}>${escapeHtml(body.join('\n'))}</code></pre>`);
      continue;
    }

    if (!line.trim()) { flushAll(); continue; }

    // Blockquote: collect prefix-stripped lines and recurse
    if (/^\s*>\s?/.test(line)) { flushPara(); quote.push(line.replace(/^\s*>\s?/, '')); continue; }
    flushQuote();

    // Complete an unclosed mid-line $$ or \[ opener in para before treating a line-start closer as a new block opener.
    const openPara = findUnclosedParaDisplay(para);
    if (openPara) {
      const closeLine = findParaDisplayClose(lines, i, openPara);
      if (closeLine !== -1) {
        for (let k = i; k <= closeLine; k++) para.push(lines[k]);
        i = closeLine;
        continue;
      }
    }

    // Block display math ($$, \[, \begin{env})
    const disp = matchBlockDisplay(lines, i);
    if (disp) {
      flushAll();
      const raw = disp.body.trim();
      const tex = parseOpts.rendered ? raw : normalizeMathTex(raw, true);
      if (tex) {
        const id = mathTokens.length;
        mathTokens.push({ raw, tex, display: true, inlineDisplay: false, glue: '' });
        out.push(`\u0000D${id}\u0000`);
      }
      if (disp.rest && disp.rest.trim()) {
        const tail = disp.rest.trimStart();
        const prevLine = lines[disp.closeLine];
        lines[disp.closeLine] = tail;
        if (matchBlockDisplay(lines, disp.closeLine)) {
          i = disp.closeLine - 1;
        } else {
          lines[disp.closeLine] = prevLine;
          para.push(tail);
          i = disp.closeLine;
        }
      } else {
        i = disp.closeLine;
      }
      continue;
    }

    const heading = matchAtxHeading(line);
    if (heading) {
      flushPara();
      out.push(`<h${heading.level}>${renderInlineMarkdown(heading.text, mathTokens, parseOpts)}</h${heading.level}>`);
      continue;
    }
    if (/^[ \t]*(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/.test(line)) { flushPara(); out.push('<hr>'); continue; }
    if (line.includes('|') && i + 1 < lines.length && isTableDelimiterLine(lines[i + 1])) {
      flushPara();
      const headMarked = scanInlineText(line, mathTokens, parseOpts);
      const headers = splitTableRow(headMarked);
      const aligns = splitTableRow(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : 'left'));
      const ths = headers.map((c, idx) => `<th style="text-align:${aligns[idx] || 'left'}">${renderInlineMarkdown(c, mathTokens, parseOpts, true)}</th>`).join('');
      const rows = [];
      for (i += 2; i < lines.length && lines[i].trim() && lines[i].includes('|') && !/^[ \t]{0,3}(`{3,}|~{3,})/.test(lines[i]); i++) {
        const rowMarked = scanInlineText(lines[i], mathTokens, parseOpts);
        const cols = splitTableRow(rowMarked);
        rows.push(`<tr>${headers.map((_, idx) => `<td style="text-align:${aligns[idx] || 'left'}">${renderInlineMarkdown(cols[idx] ?? '', mathTokens, parseOpts, true)}</td>`).join('')}</tr>`);
      }
      i -= 1;
      out.push(`<div class="table-wrap"><table><thead><tr>${ths}</tr></thead><tbody>${rows.join('')}</tbody></table></div>`);
      continue;
    }

    if (parseListMarker(line)) {
      flushPara();
      const { html, nextIdx } = consumeList(lines, i, mathTokens, parseOpts);
      out.push(html);
      i = nextIdx - 1;
      continue;
    }

    para.push(line);
  }
  flushAll();
  return out.join('\n');
}

function substituteTokens(template, mathTokens, eqrefTokens, renderCtx) {
  const eqRefs = renderCtx ? numberEquations(mathTokens) : null;
  const ctx = renderCtx ? { ...renderCtx, eqRefs } : null;
  return template
    .replace(/\u0000D(\d+)\u0000/g, (_, idx) => {
      const tok = mathTokens[Number(idx)];
      if (!tok) return '';
      return ctx
        ? renderFormulaHtml(tok, ctx)
        : `<div class="math-block">$$\n${escapeHtml(tok.tex)}\n$$</div>`;
    })
    .replace(/\u0000M(\d+)\u0000/g, (_, idx) => {
      const tok = mathTokens[Number(idx)];
      if (!tok) return '';
      return ctx ? renderFormulaHtml(tok, ctx) : formatMathTokenHtml(tok);
    })
    .replace(/\u0000E(\d+)\u0000/g, (_, idx) => {
      const id = eqrefTokens[Number(idx)];
      if (!id) return '';
      const resolved = eqRefs?.get(id);
      const label = `(${resolved !== undefined ? resolved : id})`;
      return `<a class="eqref-link" data-eq-ref="${escapeHtml(id)}">${escapeHtml(label)}</a>`;
    });
}

export function markdownToHtml(raw) {
  const src = stripComments(String(raw ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n'));
  const mathTokens = [];
  const template = renderMarkdownBlocks(src.split('\n'), mathTokens, { rendered: false });
  return substituteTokens(template, mathTokens, [], null);
}

export function markdownToRenderedHtml(raw, opts = {}) {
  const src = stripComments(String(raw ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n'));
  const mathTokens = [];
  const eqrefTokens = [];
  const template = renderMarkdownBlocks(src.split('\n'), mathTokens, { rendered: true, eqrefTokens });
  return substituteTokens(template, mathTokens, eqrefTokens, {
    katex: opts.katex,
    state: opts.state,
    inlineOnly: Boolean(opts.inlineOnly),
  });
}
