// Safe offline Markdown, LaTeX document preview, and KaTeX math rendering helpers.
import { escapeHtml, highlightPython, highlightUnifiedDiff } from './code_highlight.js';
import {
  extractMath, findClosingDelim, findClosingInlineDollar, hasUnescapedMathDelim,
} from './math_scan.js';
import { clearMathCache, katexOptions, renderFormulaHtml, resetMathMacros } from './math_katex.js';
import { installEqrefNavigation, numberEquations } from './math_equations.js';
import { markdownToHtml, markdownToRenderedHtml } from './markdown.js';
import { latexDocumentToHtml, latexDocumentToRenderedHtml } from './tex_preview.js';

export {
  clearMathCache, escapeHtml, highlightPython, highlightUnifiedDiff, installEqrefNavigation,
  katexOptions, latexDocumentToHtml, latexDocumentToRenderedHtml, markdownToHtml,
  markdownToRenderedHtml, numberEquations, renderFormulaHtml, resetMathMacros,
};

function renderMathTextInternal(text, opts = {}) {
  const tokens = [];
  const marked = extractMath(String(text ?? '').replace(/\u0000/g, ''), tokens, {
    protectInlineCode: opts.protectInlineCode === true,
    allowSingleDollar: opts.allowSingleDollar !== false,
  });
  let out = '';
  let last = 0;
  for (const m of marked.matchAll(/\u0000M(\d+)\u0000/g)) {
    out += escapeHtml(marked.slice(last, m.index));
    const tok = tokens[Number(m[1])];
    if (tok) {
      out += renderFormulaHtml(tok, { katex: opts.katex, inlineOnly: Boolean(opts.inlineOnly) });
    }
    last = m.index + m[0].length;
  }
  return out + escapeHtml(marked.slice(last));
}

export function renderMathInContainer(el, opts = {}) {
  if (!el || typeof window === 'undefined' || typeof document === 'undefined') return;
  const katexImpl = opts.katex ?? window.katex;
  if (!katexImpl || typeof katexImpl.renderToString !== 'function') return;
  const allowSingleDollar = opts.allowSingleDollar !== false;
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
    span.innerHTML = renderMathTextInternal(node.nodeValue || '', { allowSingleDollar, katex: katexImpl });
    node.parentNode.replaceChild(span, node);
  }
}

function hasRenderableMath(text, katexImpl = globalThis.window?.katex) {
  return typeof katexImpl?.renderToString === 'function'
    && /\$|\\[\[(]|\\begin\{/.test(text);
}

// Math with literal surrounding text, for labels, excerpts, and activity logs.
export function mathTextToHtml(rawText, { inlineOnly = false, katex = undefined } = {}) {
  const text = String(rawText ?? '');
  const katexImpl = katex ?? globalThis.window?.katex;
  return hasRenderableMath(text, katexImpl)
    ? renderMathTextInternal(text, { protectInlineCode: true, inlineOnly, katex: katexImpl })
    : escapeHtml(text);
}

export function renderMathTextInto(el, rawText, opts) {
  if (!el) return;
  const text = String(rawText ?? '');
  const katexImpl = opts?.katex ?? globalThis.window?.katex;
  if (hasRenderableMath(text, katexImpl)) el.innerHTML = mathTextToHtml(text, opts);
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
  const original = String(rawLatex ?? '').trim();
  if (!original || /^(\$\$|\\\[|\\begin\{)/.test(original)) return original;
  let inner = null;
  if (original.startsWith('\\(') && findClosingDelim(original, '\\)', 2) === original.length - 2) {
    inner = original.slice(2, -2);
  } else if (original.startsWith('$') && findClosingInlineDollar(original, 1) === original.length - 1) {
    inner = original.slice(1, -1);
  }
  if (inner !== null) {
    if (hasUnescapedMathDelim(inner)) return original;
    return `$$\n${inner}\n$$`;
  }
  if (hasUnescapedMathDelim(original)) return original;
  return `$$\n${original}\n$$`;
}

export function renderRichInto(el, rawText, opts = {}) {
  if (!el) return;
  el.innerHTML = markdownToRenderedHtml(rawText, opts);
  installEqrefNavigation(el);
}

export function renderLatexDocumentPreview(el, texSource, opts = {}) {
  if (!el) return;
  const html = latexDocumentToRenderedHtml(texSource, opts);
  if (!html) { el.innerHTML = '<p style="color:var(--ink-muted)">Empty TeX buffer.</p>'; return; }
  el.innerHTML = html;
  installEqrefNavigation(el);
}
