// Offline LaTeX document preview renderer with preamble macro harvesting.
import { escapeHtml } from './code_highlight.js';
import { extractMath, findMatchingEnvEnd, formatMathTokenHtml } from './math_scan.js';
import {
  createMathState, isMacroDefined, katexOptions, renderFormulaHtml, rewriteNewcommands, syncMathMacros,
} from './math_katex.js';
import { numberEquations } from './math_equations.js';

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
  let braceDepth = 0;
  for (let i = openIdx + 1; i < src.length; i++) {
    if (src[i] === '\\') { i += 1; continue; }
    if (openChar !== '{') {
      if (src[i] === '{') { braceDepth += 1; continue; }
      if (src[i] === '}' && braceDepth > 0) { braceDepth -= 1; continue; }
    }
    if (braceDepth === 0) {
      if (src[i] === openChar) depth += 1;
      else if (src[i] === closeChar && --depth === 0) return { content: src.slice(openIdx + 1, i), end: i + 1 };
    }
  }
  return null;
}

function extractTexCommand(src, cmdName) {
  const m = new RegExp(`\\\\${cmdName}\\*?\\s*\\{`).exec(src);
  const grp = m ? readBalancedGroup(src, m.index + m[0].length - 1) : null;
  return grp ? grp.content.trim() : '';
}

function skipSpaces(src, pos) {
  let i = pos;
  while (i < src.length && /[ \t\r\n]/.test(src[i])) i += 1;
  return i;
}

export function harvestPreambleMacros(preamble) {
  const decls = [];
  const src = String(preamble ?? '');
  let i = 0;
  while (i < src.length) {
    if (src[i] === '\\' && src[i + 1] === '\\') { i += 2; continue; }
    const rest = src.slice(i);
    const nc = /^\\(renewcommand|newcommand|providecommand)\*?\s*(?:\{\\([A-Za-z]+)\}|\\([A-Za-z]+))/.exec(rest);
    if (nc) {
      const kind = nc[1];
      const name = nc[2] || nc[3];
      let pos = skipSpaces(src, i + nc[0].length);
      let argc = null;
      let optDefault = null;
      if (src[pos] === '[') {
        argc = readBalancedGroup(src, pos, '[', ']');
        if (argc) pos = skipSpaces(src, argc.end);
      }
      if (argc && src[pos] === '[') {
        optDefault = readBalancedGroup(src, pos, '[', ']');
        if (optDefault) pos = skipSpaces(src, optDefault.end);
      }
      const body = src[pos] === '{' ? readBalancedGroup(src, pos) : null;
      if (body) {
        const argcTrim = argc ? argc.content.trim() : '';
        const argcNum = argc ? (/^[0-9]+$/.test(argcTrim) ? parseInt(argcTrim, 10) : -1) : 0;
        if (optDefault && argcNum >= 1 && argcNum <= 9) {
          const helper = `reSearchOpt${name}`;
          const restParams = Array.from({ length: argcNum - 1 }, (_, k) => `#${k + 2}`).join('');
          const tex = `\\def\\${name}{\\@ifnextchar[{\\${helper}}{\\${helper}[${optDefault.content}]}}\\def\\${helper}[#1]${restParams}{${body.content}}`;
          decls.push({ kind, name, hasOpt: true, tex });
        } else if (argcNum >= 0 && argcNum <= 9) {
          const argcPart = argc ? `[${argcTrim}]` : '';
          const cmd = kind === 'providecommand' ? 'providecommand' : 'newcommand';
          decls.push({
            kind,
            name,
            hasOpt: false,
            argcPart,
            body: body.content,
            tex: `\\${cmd}{\\${name}}${argcPart}{${body.content}}`,
          });
        }
        i = body.end;
        continue;
      }
    }
    const dmo = /^\\DeclareMathOperator(\*?)\s*(?:\{\\([A-Za-z]+)\}|\\([A-Za-z]+))/.exec(rest);
    if (dmo) {
      const star = dmo[1];
      const name = dmo[2] || dmo[3];
      const pos = skipSpaces(src, i + dmo[0].length);
      const body = src[pos] === '{' ? readBalancedGroup(src, pos) : null;
      if (body) {
        const opBody = `\\operatorname${star}{${body.content}}`;
        decls.push({
          kind: 'newcommand',
          name,
          hasOpt: false,
          argcPart: '',
          body: opBody,
          tex: `\\newcommand{\\${name}}{${opBody}}`,
        });
        i = body.end;
        continue;
      }
    }
    const def = /^\\(?:g|e|x)?def\s*\\([A-Za-z]+)([^{]*)/.exec(rest);
    if (def && !/\n\s*\n/.test(def[2])) {
      const pos = i + def[0].length;
      const body = src[pos] === '{' ? readBalancedGroup(src, pos) : null;
      if (body) {
        decls.push({ kind: 'def', name: def[1], hasOpt: false, tex: src.slice(i, body.end) });
        i = body.end;
        continue;
      }
    }
    i += 1;
  }
  return decls;
}

function replayPreambleMacros(decls, katexImpl, docState) {
  if (!decls.length || !katexImpl || typeof katexImpl.renderToString !== 'function') return;
  const defined = new Set();
  const emitted = [];
  for (const d of decls) {
    const already = defined.has(d.name) || isMacroDefined(d.name, katexImpl, docState);
    if (d.kind === 'providecommand') {
      if (already) continue;
      emitted.push(d.tex);
      defined.add(d.name);
    } else if (d.kind === 'def' || d.hasOpt) {
      emitted.push(d.tex);
      defined.add(d.name);
    } else {
      emitted.push(`\\${already ? 'renewcommand' : 'newcommand'}{\\${d.name}}${d.argcPart}{${d.body}}`);
      defined.add(d.name);
    }
  }
  if (!emitted.length) return;
  const opts = { ...katexOptions(false, docState), throwOnError: true };
  try {
    katexImpl.renderToString(emitted.join('\n'), opts);
  } catch {
    for (const decl of emitted) {
      try {
        katexImpl.renderToString(rewriteNewcommands(decl, katexImpl, docState), opts);
      } catch {
        // Ignore unsupported LaTeX-only preamble declarations.
      }
    }
  }
  syncMathMacros(docState);
}

function renderTexInline(src, mathTokens, renderCtx) {
  let out = '';
  let i = 0;
  while (i < src.length) {
    if (src[i] === '\u0000') {
      const m = src.slice(i).match(/^\u0000M(\d+)\u0000/);
      if (m) {
        const tok = mathTokens[Number(m[1])];
        out += renderCtx ? renderFormulaHtml(tok, renderCtx) : formatMathTokenHtml(tok);
        i += m[0].length; continue;
      }
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
          if (cmd === 'textbf') { out += `<strong>${renderTexInline(grp.content, mathTokens, renderCtx)}</strong>`; i = grp.end; continue; }
          if (cmd === 'textit' || cmd === 'emph') { out += `<em>${renderTexInline(grp.content, mathTokens, renderCtx)}</em>`; i = grp.end; continue; }
          if (cmd === 'texttt') { out += `<code>${escapeHtml(grp.content)}</code>`; i = grp.end; continue; }
          if (cmd === 'cite') { out += `<code>[cite:${escapeHtml(grp.content + (optArg ? `, ${optArg}` : ''))}]</code>`; i = grp.end; continue; }
          if (cmd === 'eqref' && renderCtx) {
            const id = grp.content.trim();
            const resolved = renderCtx.eqRefs?.get(id);
            const label = `(${resolved !== undefined ? resolved : id})`;
            out += `<a class="eqref-link" data-eq-ref="${escapeHtml(id)}">${escapeHtml(label)}</a>`;
            i = grp.end; continue;
          }
          if (/^(?:eq|auto|c|C|page)?ref$/.test(cmd)) { out += `<code>[ref:${escapeHtml(grp.content)}]</code>`; i = grp.end; continue; }
          if (/^(?:label|bibliographystyle|bibliography|documentclass|usepackage|date|title|author)$/.test(cmd)) { i = grp.end; continue; }
        }
      }
    }
    out += escapeHtml(src[i]); i += 1;
  }
  return out;
}

function renderTexBlocks(src, mathTokens, renderCtx) {
  const out = [];
  let para = [];
  const flushPara = () => {
    const joined = para.map((l) => l.trim()).filter(Boolean).join(' ');
    if (joined) out.push(`<p>${renderTexInline(joined, mathTokens, renderCtx)}</p>`);
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
            .map((it) => `<li>${renderTexInline(it.trim().replace(/\n+/g, ' '), mathTokens, renderCtx)}</li>`).join('');
          out.push(`<${tag}>${items}</${tag}>`);
        } else {
          const label = env[0].toUpperCase() + env.slice(1);
          const suffix = envOpen[2] ? ` (${renderTexInline(envOpen[2], mathTokens, renderCtx)})` : '';
          out.push(`<div class="tex-env tex-env-${escapeHtml(env)}"><div class="tex-env-title">${escapeHtml(label)}${suffix}</div>${renderTexBlocks(inner, mathTokens, renderCtx)}</div>`);
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
        out.push(`<h${lvl}>${renderTexInline(grp.content, mathTokens, renderCtx)}</h${lvl}>`);
        i = grp.end; continue;
      }
    }
    const nl = src.indexOf('\n', i);
    const line = src.slice(i, nl === -1 ? src.length : nl);
    i = nl === -1 ? src.length : nl + 1;
    const mMatch = line.match(/^\s*\u0000M(\d+)\u0000\s*$/);
    if (mMatch && mathTokens[Number(mMatch[1])]?.display) {
      flushPara();
      const tok = mathTokens[Number(mMatch[1])];
      out.push(renderCtx
        ? renderFormulaHtml(tok, renderCtx)
        : `<div class="math-block">$$\n${escapeHtml(tok.tex)}\n$$</div>`);
      continue;
    }
    if (!line.trim() || /^\s*\\par\b/.test(line)) { flushPara(); continue; }
    if (/^\s*\\(?:maketitle|tableofcontents|documentclass|usepackage|bibliographystyle|bibliography|date|title|author|label)\b/.test(line)) {
      if (renderTexInline(line, mathTokens, renderCtx).trim()) para.push(line);
      continue;
    }
    para.push(line);
  }
  flushPara();
  return out.join('\n');
}

function buildTexDocumentHtml(texSource, renderCtx) {
  const cleaned = stripTexComments(String(texSource ?? '').replace(/\u0000/g, '').replace(/\r\n?/g, '\n'));
  if (!cleaned.trim()) return '';
  const beginDoc = cleaned.indexOf('\\begin{document}');
  const preamble = beginDoc >= 0 ? cleaned.slice(0, beginDoc) : '';
  let body = beginDoc >= 0 ? cleaned.slice(beginDoc + '\\begin{document}'.length) : cleaned;
  const endDoc = body.indexOf('\\end{document}');
  if (endDoc >= 0) body = body.slice(0, endDoc);

  let effectiveCtx = renderCtx;
  if (renderCtx) {
    const katexImpl = renderCtx.katex ?? globalThis.window?.katex;
    let state = renderCtx.state;
    if (!state) {
      state = createMathState();
      replayPreambleMacros(harvestPreambleMacros(preamble), katexImpl, state);
    }
    effectiveCtx = { ...renderCtx, state };
  }

  const rawTitle = extractTexCommand(preamble, 'title') || extractTexCommand(body, 'title');
  const rawAuthor = (extractTexCommand(preamble, 'author') || extractTexCommand(body, 'author')).replace(/\s*\\and\s*/g, ', ');
  const mathTokens = [];
  const scanOpts = { protectInlineCode: false, texMode: true, preserveRaw: Boolean(effectiveCtx) };
  const titleMarked = extractMath(rawTitle, mathTokens, scanOpts);
  const authorMarked = extractMath(rawAuthor, mathTokens, scanOpts);
  const bodyMarked = extractMath(body, mathTokens, scanOpts);
  if (effectiveCtx) {
    effectiveCtx = { ...effectiveCtx, eqRefs: numberEquations(mathTokens) };
  }

  const parts = [];
  if (titleMarked || authorMarked) {
    const h1 = titleMarked ? `<h1>${renderTexInline(titleMarked, mathTokens, effectiveCtx)}</h1>` : '';
    const auth = authorMarked ? `<p class="tex-doc-author">${renderTexInline(authorMarked, mathTokens, effectiveCtx)}</p>` : '';
    parts.push(`<header class="tex-doc-header">${h1}${auth}</header>`);
  }
  const bodyHtml = renderTexBlocks(bodyMarked, mathTokens, effectiveCtx);
  if (bodyHtml) parts.push(bodyHtml);
  return parts.join('\n');
}

export function latexDocumentToHtml(texSource) {
  return buildTexDocumentHtml(texSource, null);
}

export function latexDocumentToRenderedHtml(texSource, opts = {}) {
  return buildTexDocumentHtml(texSource, { katex: opts.katex, state: opts.state });
}
