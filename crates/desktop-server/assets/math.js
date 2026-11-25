// Safe offline Markdown, LaTeX document preview, and KaTeX math rendering helpers.
import { escapeHtml, highlightPython, highlightUnifiedDiff } from './code_highlight.js';
import {
  DELIMITERS, DELIMITERS_NO_DOLLAR, extractMath, findClosingDelim,
  findClosingInlineDollar, normalizeMathTex,
} from './math_scan.js';
import { markdownToHtml } from './markdown.js';
import { latexDocumentToHtml } from './tex_preview.js';

export { escapeHtml, highlightPython, highlightUnifiedDiff, latexDocumentToHtml, markdownToHtml };

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
    protectInlineCode: opts.protectInlineCode === true, allowSingleDollar: opts.allowSingleDollar !== false,
  });
  let out = '';
  let last = 0;
  for (const m of marked.matchAll(/\u0000M(\d+)\u0000/g)) {
    out += escapeHtml(marked.slice(last, m.index));
    const tok = tokens[Number(m[1])];
    if (tok) {
      try { out += window.katex.renderToString(tok.tex, { displayMode: tok.display && !opts.inlineOnly, throwOnError: false, strict: false }); }
      catch { out += escapeHtml(tok.display ? `$$${tok.tex}$$` : `\\(${tok.tex}\\)`); }
    }
    last = m.index + m[0].length;
  }
  return out + escapeHtml(marked.slice(last));
}

function hasRenderableMath(text) {
  return typeof globalThis.window?.katex?.renderToString === 'function'
    && /\$|\\[\[(]|\\begin\{/.test(text);
}

// Math with literal surrounding text, for labels, excerpts, and activity logs.
export function mathTextToHtml(rawText, { inlineOnly = false } = {}) {
  const text = String(rawText ?? '');
  return hasRenderableMath(text)
    ? renderInlineMathFallback(text, { protectInlineCode: true, inlineOnly })
    : escapeHtml(text);
}

export function renderMathTextInto(el, rawText, opts) {
  if (!el) return;
  const text = String(rawText ?? '');
  if (hasRenderableMath(text)) el.innerHTML = mathTextToHtml(text, opts);
  else el.textContent = text;
}

// Segment placeholders rather than the TeX so a sentence never cuts a formula in two.
const sentenceBreaks = new Intl.Segmenter(undefined, { granularity: 'sentence' });
export function firstMathSentence(text) {
  const tokens = [];
  const marked = extractMath(String(text || '').replace(/\u0000/g, ''), tokens);
  const sentences = sentenceBreaks.segment(marked);
  const first = sentences[Symbol.iterator]().next().value?.segment.trim() || '';
  return first.replace(/\u0000M(\d+)\u0000/g, (_, index) => '\\(' + tokens[index].tex + '\\)');
}

export function displayMathSource(rawLatex) {
  let tex = String(rawLatex ?? '').trim();
  if (!tex || /^(\$\$|\\\[|\\begin\{)/.test(tex)) return tex;
  if (tex.startsWith('\\(') && findClosingDelim(tex, '\\)', 2) === tex.length - 2) {
    tex = tex.slice(2, -2);
  } else if (tex.startsWith('$') && findClosingInlineDollar(tex, 1) === tex.length - 1) {
    tex = tex.slice(1, -1);
  }
  return `$$\n${tex}\n$$`;
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
