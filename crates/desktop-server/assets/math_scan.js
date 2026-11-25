// Math delimiter scanner and TeX environment normalization helpers.
import { escapeHtml } from './code_highlight.js';

export const KATEX_NATIVE_ENVS = (
  'equation equation* align align* gather gather* alignat alignat* split aligned gathered '
  + 'cases dcases rcases matrix pmatrix bmatrix Bmatrix vmatrix Vmatrix smallmatrix array CD'
).split(' ');
export const EXTRA_MATH_ENVS = 'flalign flalign* multline multline* eqnarray eqnarray* subequations'.split(' ');
export const MATH_ENVS = new Set([...KATEX_NATIVE_ENVS, ...EXTRA_MATH_ENVS]);

export const DELIMITERS_NO_DOLLAR = [
  { left: '$$', right: '$$', display: true },
  { left: '\\[', right: '\\]', display: true },
  ...[...KATEX_NATIVE_ENVS, ...EXTRA_MATH_ENVS].map((e) => ({ left: `\\begin{${e}}`, right: `\\end{${e}}`, display: true })),
  { left: '\\(', right: '\\)', display: false },
];
export const DELIMITERS = [...DELIMITERS_NO_DOLLAR, { left: '$', right: '$', display: false }];

export function normalizeEqnarrayBody(body) {
  return body.split('\\\\').map((r) => r.replace(/&\s*([^&\n]+?)\s*&/, '&$1 ')).join('\\\\');
}

export function normalizeMathTex(rawTex, isDisplay) {
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

export function isEscapedAt(text, pos) {
  let bs = 0;
  for (let k = pos - 1; k >= 0 && text[k] === '\\'; k--) bs += 1;
  return bs % 2 === 1;
}

export function findMatchingEnvEnd(text, env, pos) {
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

export function findClosingInlineDollar(text, startIdx, protectCode = false) {
  const first = text[startIdx];
  if (!first || /\s/.test(first) || first === '$') return -1;
  for (let j = startIdx; j < text.length; j++) {
    const ch = text[j];
    if (ch === '\u0000' || (ch === '\n' && text[j + 1] === '\n')) return -1;
    if (protectCode && ch === '`' && !isEscapedAt(text, j)) return -1;
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

export function findClosingDelim(text, delim, pos) {
  while (pos < text.length) {
    const idx = text.indexOf(delim, pos);
    if (idx === -1 || !isEscapedAt(text, idx)) return idx;
    pos = idx + 1;
  }
  return -1;
}

export function extractMath(text, mathTokens, opts = {}) {
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
      const end = findClosingInlineDollar(text, i + 1, protectCode);
      if (end !== -1) { out += push(text.slice(i + 1, end), false); i = end + 1; continue; }
    }
    out += ch; i += 1;
  }
  return out;
}

export function formatMathTokenHtml(tok) {
  return !tok ? '' : tok.display ? `$$${escapeHtml(tok.tex)}$$` : `\\(${escapeHtml(tok.tex)}\\)`;
}
