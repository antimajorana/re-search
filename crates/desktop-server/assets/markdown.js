// Offline Markdown parser and HTML renderer with math placeholder protection.
import { escapeHtml } from './code_highlight.js';
import { extractMath, formatMathTokenHtml, isEscapedAt } from './math_scan.js';

export function applyEmphasis(escapedText) {
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
