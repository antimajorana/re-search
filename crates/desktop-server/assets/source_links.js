// Validated direct external academic links (arXiv, DOI, INSPIRE-HEP, Semantic Scholar, Crossref)
// matching desktop-app/src/rules.rs's strict HTTPS allowlist, plus link extractors for
// literature cards, tool activity cards, and ingested session sources.
import { escapeHtml } from '/assets/math.js';

const ALLOWED_SOURCE_HOSTS = new Set([
  'arxiv.org',
  'export.arxiv.org',
  'doi.org',
  'dx.doi.org',
  'inspirehep.net',
  'semanticscholar.org',
  'www.semanticscholar.org',
  'crossref.org',
  'api.crossref.org',
  'chatgpt.com',
]);

export function safeSourceUrl(raw) {
  if (!raw || typeof raw !== 'string') return null;
  const trimmed = raw.trim();
  if (!trimmed || trimmed.length > 2048 || /[\s\x00-\x1f\\]/.test(trimmed)) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.port) return null;
    const host = parsed.hostname.toLowerCase();
    if (!ALLOWED_SOURCE_HOSTS.has(host)) return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

export function extractArxivId(text) {
  if (!text || typeof text !== 'string') return null;
  const mNew = text.match(/(?:arxiv:|abs\/|e-print\/|pdf\/)?(\d{4}\.\d{4,5}(?:v\d+)?)/i);
  if (mNew) return mNew[1];
  const mOld = text.match(/(?:arxiv:|abs\/)?([a-z-]+(?:\.[a-z-]+)?\/\d{7}(?:v\d+)?)/i);
  return mOld ? mOld[1] : null;
}

function versionedArxivId(...values) {
  const ids = values.map(extractArxivId).filter(Boolean);
  const first = ids[0];
  if (!first || /v\d+$/i.test(first)) return first;
  return ids.find((id) => /v\d+$/i.test(id) && id.replace(/v\d+$/i, '') === first) || first;
}

function dropsArxivVersion(url, arxivId) {
  if (!url || !arxivId || !/v\d+$/i.test(arxivId)) return false;
  const parsed = new URL(url);
  return ['arxiv.org', 'export.arxiv.org'].includes(parsed.hostname)
    && extractArxivId(parsed.pathname) === arxivId.replace(/v\d+$/i, '');
}

export function extractDoi(text) {
  if (!text || typeof text !== 'string') return null;
  const cleaned = text.trim().replace(/^https?:\/\/(?:dx\.)?doi\.org\//i, '').replace(/^doi:\s*/i, '');
  return /^10\.\d{4,9}\/[^\s"<>]+$/i.test(cleaned) ? cleaned : null;
}

export function linksForPaper(item) {
  if (!item || typeof item !== 'object') return [];
  const links = [];
  const seen = new Set();
  const pushLink = (label, rawUrl, kind = 'primary') => {
    const safe = safeSourceUrl(rawUrl);
    if (!safe || seen.has(safe)) return;
    seen.add(safe);
    links.push({ label, url: safe, kind });
  };

  const rawDirect = safeSourceUrl(item.url);
  const srcName = String(item.source || '').toLowerCase();
  const arxivDirect = /^https:\/\/(?:export\.)?arxiv\.org\//i.test(rawDirect || '');
  const arxivCand = versionedArxivId(item.arxiv_id, item.base_id, !srcName || srcName === 'arxiv' ? item.id : null, arxivDirect ? rawDirect : null);
  const doi = extractDoi(String(item.doi || ''));

  if (rawDirect && !dropsArxivVersion(rawDirect, arxivCand)) {
    pushLink(arxivDirect ? 'Open on arXiv' : 'Open Paper', rawDirect, 'primary');
  }
  if (arxivCand) {
    pushLink('arXiv Abstract', `https://arxiv.org/abs/${arxivCand}`, 'primary');
    pushLink('PDF', `https://arxiv.org/pdf/${arxivCand}`, 'pdf');
  } else if (srcName === 'inspire' && item.id && /^\d+$/.test(String(item.id).trim())) {
    pushLink('INSPIRE Record', `https://inspirehep.net/literature/${String(item.id).trim()}`, 'primary');
  } else if (srcName === 'semantic_scholar' && /^[a-f0-9]{40}$/i.test(item.id)) {
    pushLink('Semantic Scholar', `https://www.semanticscholar.org/paper/${item.id}`, 'primary');
  }
  if (doi) {
    pushLink('DOI', `https://doi.org/${doi}`, 'doi');
  }
  return links;
}

export function sessionPaperReferences(events, sources = []) {
  const papers = [];
  const seen = new Set();
  if (!Array.isArray(events)) return papers;
  const saved = new Map();
  for (const source of sources) {
    const origin = source.origin || '';
    if (/^(?:https:\/\/(?:export\.)?arxiv\.org\/|arxiv:)/i.test(origin)) {
      const id = extractArxivId(origin);
      if (id) saved.set(id.toLowerCase(), source);
    }
  }
  for (const event of events.slice().reverse()) {
    if (event?.kind !== 'literature.fetched') continue;
    const { provider, hits, papers: arxivPapers } = event.payload || {};
    const records = Array.isArray(arxivPapers) ? arxivPapers : Array.isArray(hits) ? hits : [];
    for (const item of records) {
      if (!item || typeof item !== 'object') continue;
      const title = String(item.title || item.id || item.base_id || item.doi || '').trim();
      if (!title) continue;
      const doi = extractDoi(item.doi);
      const arxiv = extractArxivId(item.arxiv_id || item.base_id || (provider === 'arxiv' ? item.id : ''));
      const keys = [
        doi && `doi:${doi.toLowerCase()}`,
        arxiv && `arxiv:${arxiv.toLowerCase().replace(/v\d+$/, '')}`,
        item.id && `${provider}:${String(item.id).toLowerCase()}`,
      ].filter(Boolean);
      if (!keys.length) keys.push(`${provider}:${title.toLowerCase()}`);
      const duplicate = keys.some(key => seen.has(key));
      for (const key of keys) seen.add(key);
      if (!duplicate) {
        const id = versionedArxivId(item.arxiv_id, provider === 'arxiv' ? item.id : null, item.base_id);
        const savedSource = item.source?.source_id ? item.source : saved.get(id?.toLowerCase());
        papers.push({ ...item, source: provider, title, savedSource });
      }
    }
  }
  return papers;
}

function paperKeys(paper) {
  const doi = extractDoi(paper.doi);
  const arxiv = versionedArxivId(paper.arxiv_id, paper.base_id, paper.source === 'arxiv' ? paper.id : null);
  return [
    doi && `doi:${doi.toLowerCase()}`,
    arxiv && `arxiv:${arxiv.toLowerCase()}`,
    paper.url && citationAddress(paper.url),
    ...linksForPaper(paper).map(link => link.url.toLowerCase()),
  ].filter(Boolean);
}

function citationAddress(raw) {
  if (typeof raw !== 'string' || raw.length > 2048 || !/^https?:\/\//i.test(raw) || /[\s\\]/.test(raw)) return null;
  try {
    const url = new URL(raw);
    return url.hostname && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}

function citationToken(raw) {
  const token = raw.replace(/[.,;:]+$/, '');
  let depth = 0;
  for (let i = 0; i < token.length; i++) {
    if (token[i] === '(') depth++;
    if (token[i] === ')') {
      if (!depth) return token.slice(0, i);
      depth--;
    }
  }
  return token;
}

function explicitPaperCitations(text) {
  const papers = [];
  for (const [match] of text.matchAll(/\b10\.\d{4,9}\/[^\s,"'<>{}\[\]]+/gi)) {
    const doi = extractDoi(citationToken(match));
    if (doi) papers.push({ doi, title: `DOI ${doi}` });
  }
  for (const [, arxiv_id] of text.matchAll(/(?:\barxiv\s*:\s*|\barxiv\.org\/(?:abs|pdf|e-print)\/)(\d{4}\.\d{4,5}(?:v\d+)?|[a-z-]+(?:\.[a-z-]+)?\/\d{7}(?:v\d+)?)/gi)) {
    papers.push({ arxiv_id, source: 'arxiv', title: `arXiv:${arxiv_id}` });
  }
  for (const [, label, address] of text.matchAll(/(?:\[([^\]\n]+)\]\()?((?:https?):\/\/[^\s"'<>{}\[\]]+)/gi)) {
    const url = citationAddress(citationToken(address));
    if (!url) continue;
    const parsed = new URL(url);
    const canonicalArxiv = /^(?:export\.)?arxiv\.org$/.test(parsed.hostname)
      && /^\/(?:abs|pdf|e-print)\//.test(parsed.pathname) && extractArxivId(parsed.pathname);
    if (canonicalArxiv || /\b10\.\d{4,9}\/[^\s/]+/i.test(url)) continue;
    papers.push({ url, title: label?.trim() || url });
  }
  return papers;
}

function finalAnswerTexts(events) {
  const answers = [];
  let blocks = [];
  let hasResponse = false;
  let cliResult = '';
  for (const { kind, payload } of events) {
    if (kind === 'session.start' || kind === 'session.resume') {
      blocks = [];
      hasResponse = false;
      cliResult = '';
    } else if (kind === 'model.response') {
      blocks = [];
      hasResponse = true;
    } else if (kind === 'model.text' && typeof payload?.text === 'string') {
      if (hasResponse) blocks.push(payload.text);
      else blocks = [payload.text];
    } else if (kind === 'autonomous.feedback') {
      blocks = [];
      hasResponse = false;
    } else if (kind === 'cli.result') {
      cliResult = typeof payload?.result === 'string' ? payload.result : '';
    } else if (kind === 'session.end') {
      const answer = blocks.join('\n') || cliResult;
      if (answer && payload?.outcome !== 'refusal') answers.push(answer);
      blocks = [];
      cliResult = '';
    } else if (kind === 'session.abort') {
      blocks = [];
      cliResult = '';
    }
  }
  return answers;
}

function activeSourceSteps(events) {
  const nodes = new Map();
  const superseded = new Set();
  const rejected = new Set();
  for (const { kind, payload, effective_verdict } of events) {
    if (kind === 'proof.entry' && payload.entry_id) nodes.set(payload.entry_id, payload);
    if (kind === 'proof.supersede') superseded.add(payload.entry_id);
    if (kind === 'proof.verification') {
      if ((effective_verdict ?? payload.verdict) === 'rejected') rejected.add(payload.target_entry_id);
      else rejected.delete(payload.target_entry_id);
    }
  }
  const active = new Map([...nodes].filter(([id]) => !superseded.has(id) && !rejected.has(id)));
  const pending = [...active.values()].filter(node => node.entry_type === 'final_answer').map(node => node.entry_id);
  if (!pending.length) return active;
  const used = new Map();
  while (pending.length) {
    const id = pending.pop();
    const entry = active.get(id);
    if (!entry || used.has(id)) continue;
    used.set(id, entry);
    if (Array.isArray(entry.depends_on)) pending.push(...entry.depends_on);
  }
  return used;
}

export function sessionSourcesUsed(session, claimText) {
  const events = session?.events || [];
  const sourceSteps = activeSourceSteps(events);
  const byKey = new Map();
  const bySourceId = new Map();
  const known = new Set();
  const addPaper = (paper, sourceId) => {
    const keys = paperKeys(paper);
    const titleKey = paper.title && `title:${paper.title.toLowerCase()}`;
    const titleMatch = !keys.length && titleKey && byKey.get(titleKey);
    const existing = keys.map(key => byKey.get(key)).find(Boolean)
      || (titleMatch && (!sourceId || !titleMatch.source || titleMatch.source.source_id === sourceId) && titleMatch);
    const record = existing || { paper, cited: false, steps: new Set(), readers: new Map(), paperClaims: new Set() };
    for (const key of keys) {
      byKey.set(key, record);
      if (key.startsWith('arxiv:')) {
        const base = key.replace(/v\d+$/, '');
        if (!byKey.has(base)) byKey.set(base, record);
      }
    }
    if (titleKey) byKey.set(titleKey, record);
    known.add(record);
    return record;
  };

  for (const paper of sessionPaperReferences(events)) addPaper(paper);
  for (const source of session?.sources || []) {
    const links = linksForIngestedSource(source);
    const paper = { title: resolveSourceTitle(source), url: links[0]?.url, doi: extractDoi(source.origin),
      arxiv_id: extractArxivId(links.find(link => /arxiv\.org\/(?:abs|pdf)\//i.test(link.url))?.url),
    };
    const record = addPaper(paper, source.source_id);
    record.source = source;
    if (source.source_id) bySourceId.set(source.source_id, record);
  }

  for (const { entry_id, paper, source_id, reader } of session?.linked_sources || []) {
    if (!sourceSteps.has(entry_id)) continue;
    const record = paper ? addPaper(paper) : bySourceId.get(reader?.source_id || source_id);
    if (!record) continue;
    record.steps.add(entry_id);
    if (reader) {
      const key = JSON.stringify([reader.source_id, reader.path, reader.start_line, reader.end_line, reader.file_sha256]);
      record.readers.set(key, reader);
    }
  }

  const useText = (text, entry) => {
    const normalized = text.toLowerCase().replace(/\s+/g, ' ');
    const urls = new Set([...text.matchAll(/https?:\/\/[^\s"'<>{}\[\]]+/gi)]
      .map(([url]) => safeSourceUrl(citationToken(url))?.toLowerCase()).filter(Boolean));
    const mark = (record) => {
      if (entry) record.steps.add(entry);
      else record.cited = true;
    };
    for (const record of known) {
      const title = record.paper.title?.toLowerCase().replace(/\s+/g, ' ').trim();
      const titleMentioned = title && title.length >= 12 && title.includes(' ') && normalized.includes(title);
      const links = linksForPaper(record.paper);
      if (titleMentioned || links.some(({ url }) => urls.has(url.toLowerCase()))) mark(record);
    }
    for (const paper of explicitPaperCitations(text)) mark(addPaper(paper));
  };
  if (typeof claimText === 'string') {
    useText(claimText);
    return [...known].filter(record => record.cited);
  }
  for (const answer of finalAnswerTexts(events)) useText(answer);

  // Count only the papers supporting claims in the selected map revision or its Research handoff.
  const paperEvent = events.findLast(event => event.kind === 'investigation.map' || event.kind === 'investigation.handoff');
  const mapNodes = paperEvent?.kind === 'investigation.map' ? paperEvent.payload.nodes
    : paperEvent?.payload?.dependencies;
  for (const node of mapNodes || []) {
    for (const anchor of node.anchors || []) {
      const record = bySourceId.get(anchor.source_id);
      if (record) record.paperClaims.add(node.id);
    }
  }
  for (const edge of paperEvent?.payload?.edges || []) {
    const record = bySourceId.get(edge.anchor?.source_id);
    if (record) record.paperClaims.add(edge.to);
  }

  for (const payload of sourceSteps.values()) {
    useText([payload.summary, payload.details, payload.latex].filter(Boolean).join('\n'), payload.entry_id);
  }
  return [...known].filter(record => record.cited || record.steps.size || record.paperClaims.size);
}

export function linksForIngestedSource(src) {
  if (!src || typeof src !== 'object') return [];
  const links = [];
  const seen = new Set();
  const pushLink = (label, rawUrl, kind = 'primary') => {
    const safe = safeSourceUrl(rawUrl);
    if (!safe || seen.has(safe)) return;
    seen.add(safe);
    links.push({ label, url: safe, kind });
  };

  const rawDirect = safeSourceUrl(src.url);
  const arxivId = versionedArxivId(src.arxiv_cover_id, src.origin, src.source_id, src.primary_path, rawDirect);
  if (rawDirect && !dropsArxivVersion(rawDirect, arxivId)) {
    pushLink('Open Original', rawDirect, 'primary');
  }
  if (arxivId) {
    pushLink(`arXiv:${arxivId}`, `https://arxiv.org/abs/${arxivId}`, 'primary');
    pushLink('PDF', `https://arxiv.org/pdf/${arxivId}`, 'pdf');
  }
  const doi = extractDoi(String(src.origin || ''));
  if (doi) {
    pushLink('DOI', `https://doi.org/${doi}`, 'doi');
  }
  return links;
}

export function resolveSourceTitle(src) {
  if (!src || typeof src !== 'object') return 'Source';
  const rawTitle = typeof src.title === 'string' ? src.title.trim() : '';
  if (rawTitle && !/^https?:\/\//i.test(rawTitle)) return rawTitle;
  const candidates = [
    src.primary_path,
    ...(Array.isArray(src.files) ? src.files.filter((f) => !String(f).endsWith('__assembled__.tex')) : []),
    src.assembled_path,
  ];
  for (const cand of candidates) {
    if (!cand || typeof cand !== 'string' || /^https?:\/\//i.test(cand.trim())) continue;
    const parts = cand.split('/').filter(Boolean);
    const leaf = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
    if (leaf && leaf !== '__assembled__.tex') return leaf;
  }
  const arxivId =
    extractArxivId(String(src.origin || '')) ||
    extractArxivId(String(src.source_id || ''));
  if (arxivId) return `arXiv Paper ${arxivId}`;
  return String(src.source_id || 'Uploaded Source');
}

export function resolveOriginInfo(src) {
  if (!src || typeof src !== 'object') return { text: '', url: null };
  const rawOrigin = String(src.origin || '').trim();
  const arxivId = extractArxivId(rawOrigin)
    && versionedArxivId(src.arxiv_cover_id, rawOrigin, src.source_id, src.primary_path, src.url);
  if (arxivId) {
    const absUrl = `https://arxiv.org/abs/${arxivId}`;
    return { text: absUrl, url: absUrl };
  }
  const safe = safeSourceUrl(rawOrigin);
  if (safe) return { text: safe, url: safe };
  return { text: rawOrigin, url: null };
}

export function renderSourceLinkChips(links) {
  if (!Array.isArray(links) || links.length === 0) return '';
  return links
    .map(
      (lk) =>
        `<a class="pill-btn sm source-link-btn" href="${escapeHtml(lk.url)}" target="_blank" rel="noopener noreferrer">${escapeHtml(lk.label)} ↗</a>`
    )
    .join('');
}
