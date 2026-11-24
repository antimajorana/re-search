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
  const arxivCand = versionedArxivId(item.arxiv_id, item.base_id, item.id, rawDirect);
  const doi = extractDoi(String(item.doi || ''));
  const srcName = String(item.source || '').toLowerCase();

  if (rawDirect && !dropsArxivVersion(rawDirect, arxivCand)) {
    pushLink(/^https:\/\/(?:export\.)?arxiv\.org\//.test(rawDirect) ? 'Open on arXiv' : 'Open Paper', rawDirect, 'primary');
  }
  if (arxivCand) {
    pushLink('arXiv Abstract', `https://arxiv.org/abs/${arxivCand}`, 'primary');
    pushLink('PDF', `https://arxiv.org/pdf/${arxivCand}`, 'pdf');
  } else if (srcName === 'inspire' && item.id && /^\d+$/.test(String(item.id).trim())) {
    pushLink('INSPIRE Record', `https://inspirehep.net/literature/${String(item.id).trim()}`, 'primary');
  }
  if (doi) {
    pushLink('DOI', `https://doi.org/${doi}`, 'doi');
  }
  return links;
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
  const arxivId = versionedArxivId(src.origin, src.source_id, src.primary_path, rawDirect);
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
    && versionedArxivId(rawOrigin, src.source_id, src.primary_path, src.url);
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
