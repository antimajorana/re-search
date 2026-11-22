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
  const arxivCand = extractArxivId(String(item.arxiv_id || item.base_id || item.id || ''));
  const doi = extractDoi(String(item.doi || ''));
  const srcName = String(item.source || '').toLowerCase();

  if (rawDirect) {
    pushLink(arxivCand ? 'Open on arXiv' : 'Open Paper', rawDirect, 'primary');
  }
  if (arxivCand) {
    const base = arxivCand.replace(/v\d+$/i, '');
    pushLink('arXiv Abstract', `https://arxiv.org/abs/${arxivCand}`, 'primary');
    pushLink('PDF', `https://arxiv.org/pdf/${base}`, 'pdf');
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

  if (src.url) pushLink('Open Original', String(src.url), 'primary');
  const arxivId =
    extractArxivId(String(src.origin || '')) ||
    extractArxivId(String(src.source_id || '')) ||
    extractArxivId(String(src.primary_path || ''));
  if (arxivId) {
    const base = arxivId.replace(/v\d+$/i, '');
    pushLink(`arXiv:${arxivId}`, `https://arxiv.org/abs/${arxivId}`, 'primary');
    pushLink('PDF', `https://arxiv.org/pdf/${base}`, 'pdf');
  }
  const doi = extractDoi(String(src.origin || ''));
  if (doi) {
    pushLink('DOI', `https://doi.org/${doi}`, 'doi');
  }
  return links;
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
