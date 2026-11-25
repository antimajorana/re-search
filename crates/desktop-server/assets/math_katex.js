// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.
// KaTeX option builder, session/document macro tables, compatibility rewrites, caching, and error rendering.
import { escapeHtml } from './code_highlight.js';
import { normalizeEqnarrayBody } from './math_scan.js';
import { replaceEqrefsInTex } from './math_equations.js';

export const SEED_MACROS = Object.freeze({
  '\\R': '\\mathbb{R}',
  '\\C': '\\mathbb{C}',
  '\\N': '\\mathbb{N}',
  '\\Z': '\\mathbb{Z}',
  '\\ket': '\\left| #1\\right\\rangle',
  '\\bra': '\\left\\langle #1\\right|',
  '\\braket': '\\left\\langle #1\\right\\rangle',
  '\\Tr': '\\operatorname{Tr}',
  '\\mbox': '\\text{#1}',
});

const CACHE_LIMIT = 500;
const builtinDefined = new Map();

function macroSignature(val) {
  if (typeof val === 'string') return `s:${val}`;
  if (val && typeof val === 'object' && Array.isArray(val.tokens)) {
    const toks = val.tokens.map((t) => (t && typeof t.text === 'string' ? t.text : '')).join('\u0001');
    const delims = Array.isArray(val.delimiters)
      ? val.delimiters.map((d) => (Array.isArray(d) ? d.join('\u0002') : '')).join('\u0003')
      : '';
    return `t:${val.numArgs ?? 0}:${val.unexpandable ? 1 : 0}:${delims}:${toks}`;
  }
  return `o:${String(val)}`;
}

function buildSigs(macros) {
  const sigs = new Map();
  for (const [k, v] of Object.entries(macros)) sigs.set(k, macroSignature(v));
  return sigs;
}

export function createMathState(extraMacros = null) {
  const macros = { ...SEED_MACROS, ...(extraMacros || {}) };
  return { macros, sigs: buildSigs(macros), cache: new Map(), fp: 0 };
}

const defaultState = createMathState();

export function getDefaultMathState() {
  return defaultState;
}

export function resetMathMacros() {
  for (const k of Object.keys(defaultState.macros)) delete defaultState.macros[k];
  Object.assign(defaultState.macros, SEED_MACROS);
  defaultState.sigs = buildSigs(defaultState.macros);
  defaultState.cache.clear();
  defaultState.fp += 1;
}

export function clearMathCache(state = defaultState) {
  state.cache.clear();
}

export function katexOptions(display, state = defaultState) {
  return {
    displayMode: Boolean(display),
    output: 'html',
    strict: 'ignore',
    throwOnError: false,
    globalGroup: true,
    errorColor: 'var(--status-err)',
    macros: state.macros,
  };
}

function isDefined(name, katexImpl, state) {
  if (Object.prototype.hasOwnProperty.call(state.macros, `\\${name}`)) return true;
  let hit = builtinDefined.get(name);
  if (hit === undefined && katexImpl && typeof katexImpl.renderToString === 'function') {
    try {
      katexImpl.renderToString(`\\${name}`, { throwOnError: true, macros: {}, output: 'html', strict: 'ignore' });
      hit = true;
    } catch (e) {
      hit = !String(e?.message ?? e).includes('Undefined control sequence');
    }
    builtinDefined.set(name, hit);
  }
  return Boolean(hit);
}

export function rewriteNewcommands(tex, katexImpl, state = defaultState) {
  const seen = new Set();
  return tex.replace(/\\(re)?newcommand\*?\s*\{?\\([A-Za-z]+)\}?/g, (_m, _re, name) => {
    const exists = seen.has(name) || isDefined(name, katexImpl, state);
    seen.add(name);
    return `\\${exists ? 'renewcommand' : 'newcommand'}{\\${name}}`;
  });
}

export function repairTextCmd(tex) {
  return tex.replace(/\\text\{([^{}]*)\}/g, (_m, body) => {
    const fixed = body.replace(/(\\*)(_|[\^])/g, (m, bs, c) =>
      (bs.length % 2 === 1 ? m : `${bs}${c === '_' ? '\\_' : '\\^{}'}`));
    return `\\text{${fixed}}`;
  });
}

export function stripTagCommands(tex) {
  return String(tex ?? '')
    .replace(/\\tag\*?\s*\{[^{}]*\}/g, '')
    .replace(/\\(?:notag|nonumber)\b/g, '');
}

export function prepareInlineTex(rawTex) {
  let s = stripTagCommands(String(rawTex ?? '').replace(/\\label\{[^{}]*\}/g, ''));
  s = s.replace(/\\(?:begin|end)\{subequations\}/g, '')
    .replace(/\\(?:begin|end)\{equation\*?\}/g, '')
    .replace(/\\(begin|end)\{multline\*?\}/g, '\\$1{gathered}')
    .replace(/\\(begin|end)\{gather\*?\}/g, '\\$1{gathered}')
    .replace(/\\(begin|end)\{align\*?\}/g, '\\$1{aligned}')
    .replace(/\\(begin|end)\{flalign\*?\}/g, '\\$1{aligned}')
    .replace(/\\(begin|end)\{alignat\*?\}/g, '\\$1{alignedat}')
    .replace(/\\begin\{eqnarray\*?\}([\s\S]*?)\\end\{eqnarray\*?\}/g, (_, b) => `\\begin{aligned}${normalizeEqnarrayBody(b)}\\end{aligned}`);
  return s.trim();
}

export function normalizeDisplayEnvsForKatex(rawTex) {
  let s = String(rawTex ?? '').replace(/\\label\{[^{}]*\}/g, '');
  s = s.replace(/\\(?:begin|end)\{subequations\}/g, '')
    .replace(/\\(?:begin|end)\{equation\*?\}/g, '')
    .replace(/\\(begin|end)\{multline\*?\}/g, '\\$1{gathered}')
    .replace(/\\(begin|end)\{flalign\*?\}/g, '\\$1{align*}')
    .replace(/\\(begin|end)\{align\}/g, '\\$1{align*}')
    .replace(/\\(begin|end)\{gather\}/g, '\\$1{gather*}')
    .replace(/\\(begin|end)\{alignat\}/g, '\\$1{alignat*}')
    .replace(/\\begin\{eqnarray\*?\}([\s\S]*?)\\end\{eqnarray\*?\}/g, (_, b) => `\\begin{align*}${normalizeEqnarrayBody(b)}\\end{align*}`);
  return s.trim();
}

export function prepareTex(rawTex, {
  display = false, inlineOnly = false, katex = null, state = defaultState,
  eqPrep = null, eqRefs = null, eqNumbered = false,
} = {}) {
  let s = String(rawTex ?? '');
  if (typeof eqPrep === 'function') s = eqPrep(s);
  if (s.includes('\\eqref{')) s = replaceEqrefsInTex(s, eqRefs);
  if (inlineOnly || !display) s = prepareInlineTex(s);
  else if (!eqNumbered && typeof eqPrep !== 'function') s = normalizeDisplayEnvsForKatex(s);
  s = rewriteNewcommands(s, katex, state);
  s = repairTextCmd(s);
  return s;
}

function cleanupInternalMacros(state) {
  for (const k of Object.keys(state.macros)) {
    if (k.startsWith('\\@') || k.startsWith('@')) delete state.macros[k];
  }
}

function syncMacroFingerprint(state) {
  const changed = [];
  for (const [k, v] of Object.entries(state.macros)) {
    const sig = macroSignature(v);
    if (state.sigs.get(k) !== sig) {
      state.sigs.set(k, sig);
      changed.push(k);
    }
  }
  for (const k of [...state.sigs.keys()]) {
    if (!Object.prototype.hasOwnProperty.call(state.macros, k)) {
      state.sigs.delete(k);
      changed.push(k);
    }
  }
  if (changed.length > 0) state.fp += 1;
  return changed;
}

function usedBeforeDefinition(tex, macroKey) {
  const name = macroKey.startsWith('\\') ? macroKey.slice(1) : macroKey;
  if (!/^[A-Za-z]+$/.test(name)) return true;
  const defRe = new RegExp(`\\\\(?:[gxep]?def|let|(?:re)?newcommand\\*?)\\s*\\{?\\\\${name}(?![A-Za-z])`);
  const useRe = new RegExp(`\\\\${name}(?![A-Za-z])`);
  const defMatch = defRe.exec(tex);
  const useMatch = useRe.exec(tex);
  if (!defMatch || !useMatch) return false;
  return useMatch.index < defMatch.index;
}

function setCached(cache, key, val) {
  if (cache.size >= CACHE_LIMIT) {
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(key, val);
}

export function renderFormulaHtml(tok, ctx = {}) {
  if (!tok) return '';
  const katexImpl = ctx.katex ?? globalThis.window?.katex;
  const state = ctx.state ?? defaultState;
  const display = Boolean(tok.display) && !ctx.inlineOnly;
  const source = String(tok.raw ?? tok.tex ?? '');
  const glue = !display && tok.glue ? String(tok.glue) : '';

  if (!katexImpl || typeof katexImpl.renderToString !== 'function') {
    const pending = `<span class="math-pending" data-display="${display ? '1' : '0'}">${escapeHtml(source)}</span>`;
    return glue ? `<span class="math-glue">${pending}${escapeHtml(glue)}</span>` : pending;
  }

  const prepared = prepareTex(tok.prepared ?? tok.tex ?? source, {
    display,
    inlineOnly: Boolean(ctx.inlineOnly),
    katex: katexImpl,
    state,
    eqPrep: tok.eqPrep ?? ctx.eqPrep ?? null,
    eqRefs: ctx.eqRefs ?? null,
    eqNumbered: Boolean(tok.eqNumbered),
  });

  const key = `${display ? '1' : '0'}\u0000${prepared}\u0000${state.fp}`;
  let rendered = state.cache.get(key);
  let threw = false;

  if (rendered === undefined) {
    try {
      rendered = katexImpl.renderToString(prepared, katexOptions(display, state));
      cleanupInternalMacros(state);
      const changed = syncMacroFingerprint(state);
      if (changed.length === 0) {
        setCached(state.cache, key, rendered);
      } else if (!changed.some((k) => usedBeforeDefinition(prepared, k))) {
        const postKey = `${display ? '1' : '0'}\u0000${prepared}\u0000${state.fp}`;
        setCached(state.cache, postKey, rendered);
      }
    } catch {
      cleanupInternalMacros(state);
      threw = true;
      rendered = `<code class="math-error">${escapeHtml(source)}</code>`;
    }
  }

  if (display) {
    const tag = tok.inlineDisplay ? 'span' : 'div';
    const cls = tok.inlineDisplay ? 'math-display' : 'math-display math-block';
    const eqIdAttr = tok.eqId ? ` data-eq-id="${escapeHtml(tok.eqId)}"` : '';
    const extraIds = Array.isArray(tok.eqIds) && tok.eqIds.length > 1
      ? ` data-eq-ids="${escapeHtml(tok.eqIds.join(' '))}"`
      : '';
    const texAttr = ` data-tex="${escapeHtml(source)}" aria-label="${escapeHtml(source)}"`;
    return `<${tag} class="${cls}"${eqIdAttr}${extraIds}${texAttr}>${rendered}</${tag}>`;
  }

  if (threw) return glue ? `<span class="math-glue">${rendered}${escapeHtml(glue)}</span>` : rendered;
  return glue ? `<span class="math-glue">${rendered}${escapeHtml(glue)}</span>` : rendered;
}
