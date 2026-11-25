// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.
// Offline Markdown parser and HTML renderer with container-aware block display math.
import { escapeHtml } from './code_highlight.js';
import {
  MATH_ENVS, extractMath, findDisplayClose, findEnvBlockClose,
  formatMathTokenHtml, isEscapedAt, normalizeMathTex,
} from './math_scan.js';
import { renderFormulaHtml } from './math_katex.js';
import { numberEquations } from './math_equations.js';

const CITATION_BODY_RE = /^[\d,\s\u2013\u2014-]+$/;

export function applyEmphasis(escapedText) {
  return escapedText
    .replace(/\*\*\*(?=\S)([^*\n]*?\S)\*\*\*/g, '<strong><em>$1</em></strong>')
    .replace(/\*\*(?=\S)(.+?)(?<=\S)\*\*/g, '<strong>$1</strong>')
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
  if (!t.includes('-')) return false;
  const cells = splitTableRow(t);
  return cells.length >= 1 && cells.every((c) => /^:?-{3,}:?$/.test(c));
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

function renderMarkdownBlocks(inputLines, mathTokens, parseOpts) {
  const lines = inputLines.slice();
  const out = [];
  let para = [];
  let quote = [];
  const lists = [];

  const flushPara = () => { if (para.length) { out.push(renderParagraphLines(para, mathTokens, parseOpts)); para = []; } };
  const flushQuote = () => { if (quote.length) { out.push(`<blockquote>${renderMarkdownBlocks(quote, mathTokens, parseOpts)}</blockquote>`); quote = []; } };
  const closeLists = () => { while (lists.length) out.push(`</li></${lists.pop().tag}>`); };
  const flushAll = () => { flushPara(); flushQuote(); closeLists(); };

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
    if (/^\s*>\s?/.test(line)) { flushPara(); closeLists(); quote.push(line.replace(/^\s*>\s?/, '')); continue; }
    flushQuote();

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
        lines[disp.closeLine] = disp.rest.trimStart();
        i = disp.closeLine - 1;
      } else {
        i = disp.closeLine;
      }
      continue;
    }

    const hMatch = line.match(/^(#{1,6})\s+(.+)$/);
    if (hMatch) { flushPara(); closeLists(); out.push(`<h${hMatch[1].length}>${renderInlineMarkdown(hMatch[2], mathTokens, parseOpts)}</h${hMatch[1].length}>`); continue; }
    if (/^[ \t]*(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/.test(line)) { flushPara(); closeLists(); out.push('<hr>'); continue; }
    if (line.includes('|') && i + 1 < lines.length && isTableDelimiterLine(lines[i + 1])) {
      flushPara(); closeLists();
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
      out.push(`<table><thead><tr>${ths}</tr></thead><tbody>${rows.join('')}</tbody></table>`);
      continue;
    }
    const ulMatch = line.match(/^([ \t]*)([-*+])\s+(.+)$/);
    const olMatch = !ulMatch && line.match(/^([ \t]*)\d+[.)]\s+(.+)$/);
    if (ulMatch || olMatch) {
      flushPara();
      const tag = ulMatch ? 'ul' : 'ol';
      const indent = (ulMatch ? ulMatch[1] : olMatch[1]).replace(/\t/g, '  ').length;
      const body = renderInlineMarkdown(ulMatch ? ulMatch[3] : olMatch[2], mathTokens, parseOpts);
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
  const src = String(raw ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n');
  const mathTokens = [];
  const template = renderMarkdownBlocks(src.split('\n'), mathTokens, { rendered: false });
  return substituteTokens(template, mathTokens, [], null);
}

export function markdownToRenderedHtml(raw, opts = {}) {
  const src = String(raw ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n');
  const mathTokens = [];
  const eqrefTokens = [];
  const template = renderMarkdownBlocks(src.split('\n'), mathTokens, { rendered: true, eqrefTokens });
  return substituteTokens(template, mathTokens, eqrefTokens, {
    katex: opts.katex,
    state: opts.state,
    inlineOnly: Boolean(opts.inlineOnly),
  });
}
