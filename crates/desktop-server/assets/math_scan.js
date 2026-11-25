// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.
// Math delimiter scanner, block display closer, and TeX environment normalization helpers.
import { escapeHtml } from './code_highlight.js';

export const KATEX_NATIVE_ENVS = (
  'equation equation* align align* gather gather* alignat alignat* split aligned gathered '
  + 'cases dcases rcases matrix pmatrix bmatrix Bmatrix vmatrix Vmatrix smallmatrix array CD'
).split(' ');
export const EXTRA_MATH_ENVS = 'flalign flalign* multline multline* eqnarray eqnarray* subequations'.split(' ');
export const MATH_ENVS = new Set([...KATEX_NATIVE_ENVS, ...EXTRA_MATH_ENVS]);
export const INLINE_MATH_ENVS = new Set(
  'aligned gathered split cases dcases rcases matrix pmatrix bmatrix Bmatrix vmatrix Vmatrix smallmatrix array CD'.split(' ')
);

export const HEADING_LINE_RE = /^[ \t]{0,3}#{1,6}(?:[ \t]|$)/;
const CITATION_BODY_RE = /^[\d,\s\u2013\u2014-]+$/;
const GLUE_RE = /^[.,;:!?)\]\u2019\u201D]+/;

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

/** Strips unescaped TeX `%...` line comments while preserving `\%` and line breaks. */
export function stripTexComments(tex) {
  const s = String(tex ?? '');
  let out = '';
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\' && i + 1 < s.length) {
      out += s[i] + s[i + 1];
      i += 1;
      continue;
    }
    if (s[i] === '%') {
      while (i < s.length && s[i] !== '\n' && s[i] !== '\r') i += 1;
      if (i < s.length) out += s[i];
      continue;
    }
    out += s[i];
  }
  return out;
}

export function findMatchingEnvEnd(text, env, pos, stopOnHeading = false) {
  const openTag = `\\begin{${env}}`;
  const closeTag = `\\end{${env}}`;
  let depth = 1;
  while (pos < text.length) {
    const o = text.indexOf(openTag, pos);
    const c = text.indexOf(closeTag, pos);
    if (c === -1) return -1;
    if (stopOnHeading) {
      const seg = text.slice(pos, o !== -1 && o < c ? o : c);
      if (/\n[ \t]{0,3}#{1,6}(?:[ \t]|$)/m.test(seg)) return -1;
    }
    if (o !== -1 && o < c) { depth += 1; pos = o + openTag.length; }
    else { depth -= 1; pos = c + closeTag.length; if (depth === 0) return pos; }
  }
  return -1;
}

function isCurrencyBody(body) {
  if (!/^\d/.test(body)) return false;
  const stripped = body.replace(/\\[A-Za-z]+(?:\{[^{}]*\})?/g, ' ');
  return /[ \t][A-Za-z]{2,}\b/.test(stripped);
}

export function findClosingInlineDollar(text, startIdx, protectCode = false) {
  const first = text[startIdx];
  if (!first || /\s/.test(first) || first === '$') return -1;
  for (let j = startIdx; j < text.length; j++) {
    const ch = text[j];
    if (ch === '\u0000' || (ch === '\n' && text[j + 1] === '\n')) return -1;
    if (protectCode && ch === '`' && !isEscapedAt(text, j)) return -1;
    if (ch === '$' && !isEscapedAt(text, j)) {
      if (text[j + 1] === '$') return -1;
      const prev = text[j - 1];
      if (!prev || /\s/.test(prev) || /[0-9]/.test(text[j + 1] || '')) return -1;
      if (isCurrencyBody(text.slice(startIdx, j))) return -1;
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

function findClosingDelimInPara(text, delim, pos, protectCode = false) {
  let limit = text.length;
  const blank = text.indexOf('\n\n', pos);
  if (blank !== -1 && blank < limit) limit = blank;
  const nul = text.indexOf('\u0000', pos);
  if (nul !== -1 && nul < limit) limit = nul;
  while (pos < limit) {
    const idx = text.indexOf(delim, pos);
    if (idx === -1 || idx >= limit) return -1;
    if (protectCode) {
      const tick = text.indexOf('`', pos);
      if (tick !== -1 && tick < idx && !isEscapedAt(text, tick)) return -1;
    }
    if (!isEscapedAt(text, idx)) return idx;
    pos = idx + 1;
  }
  return -1;
}

function findClosingDoubleDollarBounded(text, pos) {
  const seg = text.slice(pos);
  const headMatch = /\n[ \t]{0,3}#{1,6}(?:[ \t]|$)/.exec(seg);
  const limit = headMatch ? pos + headMatch.index : text.length;
  let cur = pos;
  while (cur < limit) {
    const idx = text.indexOf('$$', cur);
    if (idx === -1 || idx >= limit) return -1;
    if (!isEscapedAt(text, idx)) return idx;
    cur = idx + 2;
  }
  return -1;
}

function matchGlue(text, pos) {
  const m = GLUE_RE.exec(text.slice(pos, pos + 4));
  if (!m) return '';
  let glue = m[0];
  const bracketIdx = glue.indexOf(']');
  if (bracketIdx >= 0 && text[pos + bracketIdx + 1] === '(') {
    glue = glue.slice(0, bracketIdx);
  }
  return glue;
}

export function findDisplayClose(lines, startLine, openCol, closeDelim) {
  const firstEnd = findClosingDelim(lines[startLine], closeDelim, openCol);
  if (firstEnd !== -1) {
    return {
      body: lines[startLine].slice(openCol, firstEnd),
      closeLine: startLine,
      rest: lines[startLine].slice(firstEnd + closeDelim.length),
    };
  }
  for (let l = startLine + 1; l < lines.length; l++) {
    if (HEADING_LINE_RE.test(lines[l])) return null;
    const endCol = findClosingDelim(lines[l], closeDelim, 0);
    if (endCol !== -1) {
      const parts = [lines[startLine].slice(openCol)];
      for (let k = startLine + 1; k < l; k++) parts.push(lines[k]);
      parts.push(lines[l].slice(0, endCol));
      return {
        body: parts.join('\n'),
        closeLine: l,
        rest: lines[l].slice(endCol + closeDelim.length),
      };
    }
  }
  return null;
}

function unwrapDisplayDelims(segText) {
  const t = segText.trim();
  if (t.startsWith('\\[') && t.endsWith('\\]')) return t.slice(2, -2).trim();
  if (t.startsWith('$$') && t.endsWith('$$')) return t.slice(2, -2).trim();
  return t;
}

function attachOuterLabels(segText, outerLabels) {
  if (!outerLabels.length) return segText;
  const suffix = outerLabels.join('\n');
  if (segText.startsWith('\\[') && segText.endsWith('\\]')) {
    return `\\[\n${segText.slice(2, -2).trim()}\n${suffix}\n\\]`;
  }
  if (segText.startsWith('$$') && segText.endsWith('$$')) {
    return `$$\n${segText.slice(2, -2).trim()}\n${suffix}\n$$`;
  }
  return `${segText}\n${suffix}`;
}

function splitSubequationsSegments(inner) {
  const rawSegs = [];
  let buf = '';
  const flushOther = () => {
    if (buf) { rawSegs.push({ kind: 'other', text: buf }); buf = ''; }
  };

  for (let i = 0; i < inner.length;) {
    const ch = inner[i];
    if (ch === '%') {
      while (i < inner.length && inner[i] !== '\n' && inner[i] !== '\r') i += 1;
      if (i < inner.length) { buf += inner[i]; i += 1; }
      continue;
    }
    if (ch === '\\') {
      if (inner[i + 1] === '\\') { buf += '\\\\'; i += 2; continue; }
      if (inner.startsWith('\\begin{', i)) {
        const bEnd = inner.indexOf('}', i + 7);
        const env = bEnd !== -1 ? inner.slice(i + 7, bEnd) : '';
        if (MATH_ENVS.has(env)) {
          const end = findMatchingEnvEnd(inner, env, bEnd + 1, false);
          if (end !== -1) {
            flushOther();
            rawSegs.push({ kind: 'math', text: inner.slice(i, end).trim() });
            i = end;
            continue;
          }
        }
      }
      if (inner[i + 1] === '[') {
        const end = findClosingDelim(inner, '\\]', i + 2);
        if (end !== -1 && inner.slice(i + 2, end).trim()) {
          flushOther();
          rawSegs.push({ kind: 'math', text: inner.slice(i, end + 2).trim() });
          i = end + 2;
          continue;
        }
      }
      buf += inner.slice(i, i + 2);
      i += Math.min(2, inner.length - i);
      continue;
    }
    if (ch === '$' && inner[i + 1] === '$') {
      const end = findClosingDelim(inner, '$$', i + 2);
      if (end !== -1 && inner.slice(i + 2, end).trim()) {
        flushOther();
        rawSegs.push({ kind: 'math', text: inner.slice(i, end + 2).trim() });
        i = end + 2;
        continue;
      }
    }
    buf += ch;
    i += 1;
  }
  flushOther();

  const outerLabels = [];
  const segs = [];
  let mathCount = 0;
  let hasProse = false;
  for (const seg of rawSegs) {
    if (seg.kind === 'math') {
      mathCount += 1;
      segs.push(seg);
      continue;
    }
    const clean = stripTexComments(seg.text);
    for (const m of clean.matchAll(/\\label\{[^{}]*\}/g)) outerLabels.push(m[0]);
    const prose = clean.replace(/\\label\{[^{}]*\}/g, '').trim();
    if (prose) {
      hasProse = true;
      segs.push({ kind: 'other', prose });
    }
  }

  if (mathCount === 0 || (mathCount === 1 && !hasProse)) return null;
  const firstMath = segs.find((s) => s.kind === 'math');
  if (firstMath && outerLabels.length) firstMath.text = attachOuterLabels(firstMath.text, outerLabels);
  return segs;
}

function splitSubequationsForBlocks(fullBody, rest) {
  const openTag = '\\begin{subequations}';
  const closeTag = '\\end{subequations}';
  if (!fullBody.startsWith(openTag) || !fullBody.endsWith(closeTag)) return null;
  const inner = fullBody.slice(openTag.length, -closeTag.length);
  const segs = splitSubequationsSegments(inner);
  if (!segs) return null;
  const firstIdx = segs.findIndex((s) => s.kind === 'math');
  if (firstIdx === -1) return null;

  const firstBody = unwrapDisplayDelims(segs[firstIdx].text);
  const tailLines = [];
  let lastKind = 'math';
  for (let idx = firstIdx + 1; idx < segs.length; idx++) {
    const seg = segs[idx];
    if (seg.kind === 'math') {
      tailLines.push(...seg.text.split('\n'));
      lastKind = 'math';
    } else if (seg.prose) {
      tailLines.push('', ...seg.prose.split('\n'), '');
      lastKind = 'other';
    }
  }
  if (rest && rest.trim()) {
    if (tailLines.length > 0 && lastKind === 'math') tailLines[tailLines.length - 1] += rest;
    else tailLines.push(rest.trimStart());
  }
  return { firstBody, tailLines };
}

export function findEnvBlockClose(lines, startLine, openStart, env) {
  const openTag = `\\begin{${env}}`;
  const closeTag = `\\end{${env}}`;
  let depth = 1;
  for (let l = startLine; l < lines.length; l++) {
    if (l > startLine && HEADING_LINE_RE.test(lines[l])) return null;
    let pos = l === startLine ? openStart + openTag.length : 0;
    while (pos < lines[l].length) {
      const o = lines[l].indexOf(openTag, pos);
      const c = lines[l].indexOf(closeTag, pos);
      if (c === -1) break;
      if (o !== -1 && o < c) { depth += 1; pos = o + openTag.length; }
      else {
        depth -= 1;
        pos = c + closeTag.length;
        if (depth === 0) {
          const parts = [];
          for (let k = startLine; k <= l; k++) {
            const from = k === startLine ? openStart : 0;
            const to = k === l ? pos : lines[k].length;
            parts.push(lines[k].slice(from, to));
          }
          const fullBody = parts.join('\n');
          const rest = lines[l].slice(pos);
          if (env === 'subequations') {
            const split = splitSubequationsForBlocks(fullBody, rest);
            if (split) {
              lines.splice(startLine, l - startLine + 1, '', ...split.tailLines);
              return { body: split.firstBody, closeLine: startLine, rest: '' };
            }
          }
          return { body: fullBody, closeLine: l, rest };
        }
      }
    }
  }
  return null;
}

export function hasUnescapedMathDelim(text) {
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '\\') {
      const next = text[i + 1];
      if (next === '\\') { i += 1; continue; }
      if (next === '(' || next === '[' || text.startsWith('\\begin{', i)) return true;
      if (next === '$') { i += 1; continue; }
      continue;
    }
    if (ch === '$') return true;
  }
  return false;
}

export function extractMath(text, mathTokens, opts = {}) {
  const protectCode = opts.protectInlineCode !== false;
  const allowDollar = opts.allowSingleDollar !== false;
  const captureGlue = opts.captureGlue === true;
  const dropFailedEscapes = opts.dropFailedEscapes === true;
  const preserveRaw = opts.preserveRaw === true;
  const midSentenceEnv = opts.midSentenceEnv === true;
  const texMode = opts.texMode === true;
  let out = '';
  let i = 0;

  const push = (raw, display, { inlineDisplay = false, glue = '' } = {}) => {
    const trimmed = String(raw ?? '').trim();
    if (!trimmed) return '';
    const tex = preserveRaw ? trimmed : normalizeMathTex(raw, display);
    if (!tex) return '';
    const id = mathTokens.length;
    mathTokens.push({ raw: trimmed, tex, display, inlineDisplay, glue });
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
      if (next === '\\') { out += '\\\\'; i += 2; continue; }
      if (next === '$') { out += '\\$'; i += 2; continue; }
      if (next === '[') {
        const end = texMode
          ? findClosingDelim(text, '\\]', i + 2)
          : findClosingDelimInPara(text, '\\]', i + 2, protectCode);
        if (end !== -1) {
          const body = text.slice(i + 2, end);
          if (body.trim() && (texMode || !CITATION_BODY_RE.test(body.trim()))) {
            out += push(body, true, { inlineDisplay: !texMode });
            i = end + 2;
            continue;
          }
        }
        if (dropFailedEscapes) { out += '['; i += 2; continue; }
        out += '\\['; i += 2; continue;
      }
      if (next === '(') {
        const end = texMode
          ? findClosingDelim(text, '\\)', i + 2)
          : findClosingDelimInPara(text, '\\)', i + 2, protectCode);
        if (end !== -1 && text.slice(i + 2, end).trim()) {
          const glue = captureGlue ? matchGlue(text, end + 2) : '';
          out += push(text.slice(i + 2, end), false, { glue });
          i = end + 2 + glue.length;
          continue;
        }
        if (dropFailedEscapes) { out += '('; i += 2; continue; }
        out += '\\('; i += 2; continue;
      }
      if (dropFailedEscapes && (next === ']' || next === ')')) {
        out += next; i += 2; continue;
      }
      if (text.startsWith('\\begin{', i)) {
        const bEnd = text.indexOf('}', i + 7);
        const env = bEnd !== -1 ? text.slice(i + 7, bEnd) : '';
        if (MATH_ENVS.has(env)) {
          const end = findMatchingEnvEnd(text, env, bEnd + 1, !texMode);
          if (end !== -1) {
            if (env === 'subequations') {
              const inner = text.slice(bEnd + 1, end - '\\end{subequations}'.length);
              const segs = splitSubequationsSegments(inner);
              if (segs) {
                const chunks = [];
                for (const seg of segs) {
                  if (seg.kind === 'math') {
                    chunks.push(push(unwrapDisplayDelims(seg.text), true, { inlineDisplay: midSentenceEnv }));
                  } else if (seg.prose) {
                    chunks.push(extractMath(seg.prose, mathTokens, opts));
                  }
                }
                out += chunks.filter(Boolean).join('\n');
                i = end;
                continue;
              }
            }
            const isInline = midSentenceEnv && INLINE_MATH_ENVS.has(env);
            const glue = isInline && captureGlue ? matchGlue(text, end) : '';
            out += push(text.slice(i, end), !isInline, { inlineDisplay: !isInline && midSentenceEnv, glue });
            i = end + glue.length;
            continue;
          }
        }
      }
    }
    if (ch === '$' && text[i + 1] === '$') {
      const end = texMode
        ? findClosingDelim(text, '$$', i + 2)
        : findClosingDoubleDollarBounded(text, i + 2);
      if (end !== -1 && text.slice(i + 2, end).trim()) {
        out += push(text.slice(i + 2, end), true, { inlineDisplay: !texMode });
        i = end + 2;
        continue;
      }
      out += '$$'; i += 2; continue;
    }
    if (allowDollar && ch === '$' && text[i + 1] !== '$') {
      const end = findClosingInlineDollar(text, i + 1, protectCode);
      if (end !== -1) {
        const glue = captureGlue ? matchGlue(text, end + 1) : '';
        out += push(text.slice(i + 1, end), false, { glue });
        i = end + 1 + glue.length;
        continue;
      }
    }
    out += ch; i += 1;
  }
  return out;
}

export function formatMathTokenHtml(tok) {
  if (!tok) return '';
  const body = tok.display ? `$$${escapeHtml(tok.tex)}$$` : `\\(${escapeHtml(tok.tex)}\\)`;
  return tok.glue ? `${body}${escapeHtml(tok.glue)}` : body;
}
