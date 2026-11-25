// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.
// App-managed per-root equation numbering, \label/\tag extraction, \eqref rewriting, and click navigation.
import { normalizeEqnarrayBody } from './math_scan.js';

const LABEL_RE = /\\label\{([^{}]*)\}/g;
const OWN_TAG_RE = /\\tag\*?\s*\{([^{}]*)\}/;
const NO_TAG_RE = /\\(?:notag|nonumber)\b/g;
const MULTI_ROW_RE = /\\begin\{(align|gather|eqnarray|alignat|flalign)\}/;
const STARRED_MULTI_ROW_RE = /\\begin\{(align\*|gather\*|eqnarray\*|alignat\*|flalign\*)\}/;
const NUMBERED_SINGLE_RE = /\\begin\{(equation|multline)\}/;
const SAFE_EQ_ID_RE = /^[A-Za-z0-9:._-]+$/;
const EQREF_NAV_INSTALLED = Symbol('reSearchEqrefNav');

function extractLabels(text) {
  const out = [];
  for (const m of text.matchAll(LABEL_RE)) {
    const id = m[1].trim();
    if (id) out.push(id);
  }
  return out;
}

function readBalancedBracesEnd(tex, openIdx) {
  if (tex[openIdx] !== '{') return -1;
  let depth = 1;
  for (let i = openIdx + 1; i < tex.length; i++) {
    if (tex[i] === '\\') { i += 1; continue; }
    if (tex[i] === '{') depth += 1;
    else if (tex[i] === '}' && --depth === 0) return i + 1;
  }
  return -1;
}

/** Splits top-level rows of a multi-row environment as [start, end) offsets into `tex`. */
export function findEnvRowSpans(tex, envRe = MULTI_ROW_RE) {
  const m = envRe.exec(tex);
  if (!m) return null;
  const envName = m[1];
  let start = m.index + m[0].length;
  if (envName === 'alignat' || envName === 'alignat*') {
    let p = start;
    while (p < tex.length && /[ \t\r\n]/.test(tex[p])) p += 1;
    if (tex[p] === '{') {
      const endArg = readBalancedBracesEnd(tex, p);
      if (endArg !== -1) start = endArg;
    }
  }
  const rows = [];
  let depth = 0;
  let envEnd = -1;
  for (let i = start; i < tex.length; i++) {
    if (tex.startsWith('\\begin{', i) || tex.startsWith('\\end{', i)) {
      const env = /^\\(begin|end)\{[^}]*\}/.exec(tex.slice(i, i + 48));
      if (env) {
        if (env[1] === 'begin') depth += 1;
        else if (depth === 0) { rows.push([start, i]); envEnd = i; break; }
        else depth -= 1;
        i += env[0].length - 1;
        continue;
      }
    }
    if (tex.startsWith('\\\\', i)) {
      if (depth === 0) {
        rows.push([start, i]);
        let nextStart = i + 2;
        const opt = /^[ \t]*\[[^\]]*\]/.exec(tex.slice(nextStart));
        if (opt) nextStart += opt[0].length;
        start = nextStart;
        i = nextStart - 1;
      } else {
        i += 1;
      }
      continue;
    }
    if (tex[i] === '\\') { i += 1; continue; }
    if (tex[i] === '{') depth += 1;
    else if (tex[i] === '}') depth = Math.max(0, depth - 1);
  }
  if (envEnd === -1) return null;
  if (rows.length > 1) {
    const last = rows[rows.length - 1];
    if (!tex.slice(last[0], last[1]).trim()) rows.pop();
  }
  return rows.length ? { envName, envStart: m.index, envEnd, rows } : null;
}

function starredTwinOf(envName) {
  if (envName === 'eqnarray' || envName === 'eqnarray*' || envName === 'flalign' || envName === 'flalign*') return 'align*';
  return envName.endsWith('*') ? envName : `${envName}*`;
}

function normalizeSingleDisplayBody(tex) {
  return tex
    .replace(LABEL_RE, '')
    .replace(/\\(?:begin|end)\{subequations\}/g, '')
    .replace(/\\(?:begin|end)\{equation\*?\}/g, '')
    .replace(/\\(begin|end)\{multline\*?\}/g, '\\$1{gathered}')
    .replace(/\\(begin|end)\{flalign\*?\}/g, '\\$1{aligned}')
    .replace(/\\(begin|end)\{align\*?\}/g, '\\$1{aligned}')
    .replace(/\\(begin|end)\{gather\*?\}/g, '\\$1{gathered}')
    .replace(/\\begin\{eqnarray\*?\}([\s\S]*?)\\end\{eqnarray\*?\}/g, (_, b) => `\\begin{aligned}${normalizeEqnarrayBody(b)}\\end{aligned}`)
    .trim();
}

function processMultiRowDisplay(tex, envInfo, isStarred, next, refMap) {
  const { envName, rows } = envInfo;
  const allLabels = [];
  const allTags = [];
  const pieces = [];
  let cursor = 0;

  for (const [a, b] of rows) {
    pieces.push(tex.slice(cursor, a));
    let row = tex.slice(a, b);
    const rowLabels = extractLabels(row);
    for (const id of rowLabels) allLabels.push(id);
    const ownTag = OWN_TAG_RE.exec(row);
    const hasNoTag = new RegExp(NO_TAG_RE.source).test(row);
    row = row.replace(LABEL_RE, '');
    if (envName === 'eqnarray' || envName === 'eqnarray*') {
      row = normalizeEqnarrayBody(row);
    }

    let tagText = null;
    if (ownTag) {
      tagText = ownTag[1].trim();
    } else if (hasNoTag) {
      row = row.replace(NO_TAG_RE, '');
    } else if (!isStarred || rowLabels.length > 0) {
      tagText = next();
      row = `${row.trimEnd()} \\tag{${tagText}}`;
    }

    if (tagText !== null) {
      allTags.push(tagText);
      refMap.set(tagText, tagText);
      for (const id of rowLabels) refMap.set(id, tagText);
    }
    pieces.push(row);
    cursor = b;
  }
  pieces.push(tex.slice(cursor));

  const targetEnv = starredTwinOf(envName);
  const escEnv = envName.replace('*', '\\*');
  let out = pieces.join('')
    .replace(LABEL_RE, '')
    .replace(/\\(?:begin|end)\{subequations\}/g, '')
    .replace(new RegExp(`\\\\(begin|end)\\{${escEnv}\\}`, 'g'), `\\$1{${targetEnv}}`)
    .trim();

  // Strip any stray \label outside the multi-row env (e.g. inside subequations wrapper)
  const outerLabels = extractLabels(tex).filter((id) => !allLabels.includes(id));
  if (outerLabels.length && allTags.length) {
    for (const id of outerLabels) refMap.set(id, allTags[0]);
  }
  const combinedIds = [...new Set([...allLabels, ...outerLabels, ...allTags])];
  const eqId = allLabels[0] || outerLabels[0] || allTags[0] || '';
  return { prepared: out, eqId, eqIds: combinedIds };
}

/**
 * Walks all display tokens in a single render root, assigns equation numbers,
 * rewrites environments/tags, removes \label commands, and returns label -> tagText Map.
 */
export function numberEquations(tokens) {
  const refMap = new Map();
  let counter = 0;
  const next = () => String(++counter);

  for (const tok of tokens) {
    if (!tok || !tok.display) continue;
    const raw = String(tok.raw ?? tok.tex ?? '').replace(/\\(?:begin|end)\{subequations\}/g, '').trim();

    const multi = findEnvRowSpans(raw, MULTI_ROW_RE);
    if (multi) {
      const res = processMultiRowDisplay(raw, multi, false, next, refMap);
      tok.prepared = res.prepared;
      tok.eqId = res.eqId;
      tok.eqIds = res.eqIds;
      tok.eqNumbered = true;
      continue;
    }

    const starredMulti = findEnvRowSpans(raw, STARRED_MULTI_ROW_RE);
    if (starredMulti) {
      const res = processMultiRowDisplay(raw, starredMulti, true, next, refMap);
      tok.prepared = res.prepared;
      tok.eqId = res.eqId;
      tok.eqIds = res.eqIds;
      tok.eqNumbered = true;
      continue;
    }

    const labels = extractLabels(raw);
    const ownTag = OWN_TAG_RE.exec(raw);
    const hasNoTag = new RegExp(NO_TAG_RE.source).test(raw);
    const hasNumberedEnv = NUMBERED_SINGLE_RE.test(raw);

    if (ownTag) {
      const tagText = ownTag[1].trim();
      for (const id of labels) refMap.set(id, tagText);
      if (tagText) refMap.set(tagText, tagText);
      tok.prepared = normalizeSingleDisplayBody(raw);
      tok.eqId = labels[0] || tagText;
      tok.eqIds = [...new Set([...labels, ...(tagText ? [tagText] : [])])];
      tok.eqNumbered = true;
    } else if (hasNoTag) {
      tok.prepared = normalizeSingleDisplayBody(raw.replace(NO_TAG_RE, ''));
      tok.eqId = labels[0] || '';
      tok.eqIds = labels;
      tok.eqNumbered = true;
    } else if (labels.length > 0 || hasNumberedEnv) {
      const tagText = next();
      for (const id of labels) refMap.set(id, tagText);
      refMap.set(tagText, tagText);
      const body = normalizeSingleDisplayBody(raw);
      tok.prepared = `${body} \\tag{${tagText}}`;
      tok.eqId = labels[0] || tagText;
      tok.eqIds = [...new Set([...labels, tagText])];
      tok.eqNumbered = true;
    } else {
      tok.prepared = normalizeSingleDisplayBody(raw);
      tok.eqId = '';
      tok.eqIds = [];
      tok.eqNumbered = true;
    }
  }

  return refMap;
}

function escapeTexText(s) {
  return s.replace(/[\\{}$#%&_^~]/g, (c) => {
    if (c === '\\') return '\\textbackslash{}';
    if (c === '^') return '\\^{}';
    if (c === '~') return '\\~{}';
    return `\\${c}`;
  });
}

export function replaceEqrefsInTex(tex, refMap) {
  return String(tex ?? '').replace(/\\eqref\{([^{}]*)\}/g, (_m, rawId) => {
    const id = rawId.trim();
    if (!id) return '';
    if (!SAFE_EQ_ID_RE.test(id)) return `\\text{(${escapeTexText(id)})}`;
    const resolved = refMap?.get(id);
    const label = resolved !== undefined ? escapeTexText(resolved) : id;
    return `\\text{(${label})}`;
  });
}

export function installEqrefNavigation(root) {
  if (!root || typeof root.addEventListener !== 'function') return;
  if (root[EQREF_NAV_INSTALLED]) return;
  root[EQREF_NAV_INSTALLED] = true;
  root.addEventListener('click', (e) => {
    const link = e?.target?.closest?.('a.eqref-link[data-eq-ref]');
    if (!link) return;
    const ref = link.getAttribute?.('data-eq-ref') ?? link.dataset?.eqRef;
    if (!ref) return;
    if (typeof globalThis.CSS?.escape !== 'function' || typeof root.querySelector !== 'function') return;
    const escaped = globalThis.CSS.escape(ref);
    const target = root.querySelector(`[data-eq-id="${escaped}"]`)
      || root.querySelector(`[data-eq-ids~="${escaped}"]`);
    if (!target) return;
    e.preventDefault?.();
    target.scrollIntoView?.({ block: 'center', behavior: 'smooth' });
  });
}
