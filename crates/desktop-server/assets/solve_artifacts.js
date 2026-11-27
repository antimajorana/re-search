// Solve artifact gallery and download module (S18 Part A).
// Uses DOM createElement/textContent only; PNGs preview via authenticated
// blob URLs while SVG, PDF, CSV, JSON, and text artifacts are download-only.

export const SOLVE_ARTIFACTS_TAB = Object.freeze({
  id: 'artifacts',
  label: 'Artifacts',
});

export function registerSolveTab(registry, tab = SOLVE_ARTIFACTS_TAB) {
  if (!registry || !tab || !tab.id) return tab;
  if (typeof registry === 'function') {
    registry(tab);
    return tab;
  }
  if (typeof registry.registerTab === 'function') {
    registry.registerTab(tab);
    return tab;
  }
  if (typeof Map !== 'undefined' && registry instanceof Map) {
    registry.set(tab.id, tab);
    return tab;
  }
  if (Array.isArray(registry)) {
    const idx = registry.findIndex((item) => item && item.id === tab.id);
    if (idx >= 0) registry[idx] = tab;
    else registry.push(tab);
    return tab;
  }
  registry[tab.id] = tab;
  return tab;
}

export function artifactDownloadUrl(sessionId, artifactId) {
  return (
    `/api/sessions/${encodeURIComponent(String(sessionId || ''))}` +
    `/solve/artifacts/${encodeURIComponent(String(artifactId || ''))}`
  );
}

export function formatArtifactBytes(bytes) {
  const n = Number(bytes);
  if (!Number.isFinite(n) || n < 0) return '0 B';
  if (n < 1024) return `${Math.floor(n)} B`;
  const kb = n / 1024;
  if (kb < 1024) return `${kb >= 10 ? kb.toFixed(0) : kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  return `${mb >= 10 ? mb.toFixed(0) : mb.toFixed(1)} MB`;
}

export function isPreviewableImageMime(mime) {
  return String(mime || '').trim().toLowerCase() === 'image/png';
}

export function mimeShortLabel(mime) {
  switch (String(mime || '').trim().toLowerCase()) {
    case 'image/png':
      return 'PNG';
    case 'image/svg+xml':
      return 'SVG';
    case 'application/pdf':
      return 'PDF';
    case 'application/json':
      return 'JSON';
    case 'text/csv':
      return 'CSV';
    case 'text/plain':
      return 'TEXT';
    default:
      return 'FILE';
  }
}

export function safeDownloadName(name, fallbackId) {
  const raw = String(name || '').replace(/\\/g, '/');
  const leaf = raw.split('/').pop() || '';
  const cleaned = leaf.replace(/[\x00-\x1f\x7f<>:"/\\|?*]+/g, '_').trim();
  if (cleaned && cleaned !== '.' && cleaned !== '..') return cleaned;
  return String(fallbackId || 'artifact');
}

function normalizeArtifacts(viewOrList) {
  const raw = Array.isArray(viewOrList)
    ? viewOrList
    : Array.isArray(viewOrList?.artifacts)
      ? viewOrList.artifacts
      : [];
  return raw.filter((item) => item && typeof item === 'object' && item.artifact_id);
}

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function resolveDoc(container, opts) {
  if (opts?.document) return opts.document;
  if (container?.ownerDocument) return container.ownerDocument;
  if (typeof document !== 'undefined') return document;
  return null;
}

function resolveUrlApi(opts) {
  if (opts?.urlApi) return opts.urlApi;
  if (typeof URL !== 'undefined') return URL;
  return null;
}

function buildAuthHeaders(opts) {
  const headers = {};
  const token = typeof opts?.getToken === 'function' ? opts.getToken() : opts?.token;
  if (token) headers['X-ReSearch-Token'] = String(token);
  return headers;
}

async function fetchArtifactBlob(sessionId, artifact, opts) {
  const fetchFn = opts?.fetchFn || (typeof fetch === 'function' ? fetch : null);
  if (typeof fetchFn !== 'function') {
    throw new Error('Artifact fetch unavailable');
  }
  const url = artifactDownloadUrl(sessionId, artifact.artifact_id);
  const res = await fetchFn(url, {
    method: 'GET',
    headers: buildAuthHeaders(opts),
  });
  if (!res || !res.ok) {
    const status = res?.status ?? 0;
    throw new Error(`Artifact request failed (${status})`);
  }
  if (typeof res.blob === 'function') {
    return res.blob();
  }
  if (typeof res.arrayBuffer === 'function' && typeof Blob !== 'undefined') {
    const buf = await res.arrayBuffer();
    return new Blob([buf], { type: String(artifact.mime || 'application/octet-stream') });
  }
  throw new Error('Artifact response missing blob body');
}

export function renderSolveArtifacts(container, viewOrArtifacts, opts = {}) {
  const doc = resolveDoc(container, opts);
  const urlApi = resolveUrlApi(opts);
  const trackedUrls = new Set();
  let destroyed = false;
  let seq = 0;

  function revokeTrackedUrls() {
    if (!urlApi || typeof urlApi.revokeObjectURL !== 'function') {
      trackedUrls.clear();
      return;
    }
    for (const u of trackedUrls) {
      try {
        urlApi.revokeObjectURL(u);
      } catch (_) {
        // Ignore mock cleanup failures.
      }
    }
    trackedUrls.clear();
  }

  function triggerBlobDownload(blob, filename) {
    if (!doc || !urlApi || typeof urlApi.createObjectURL !== 'function') return;
    const blobUrl = urlApi.createObjectURL(blob);
    trackedUrls.add(blobUrl);
    const link = doc.createElement('a');
    link.href = blobUrl;
    link.download = filename;
    link.rel = 'noopener noreferrer';
    link.style.display = 'none';
    const hostParent = doc.body || container;
    if (hostParent && typeof hostParent.appendChild === 'function') {
      hostParent.appendChild(link);
    }
    try {
      if (typeof link.click === 'function') link.click();
    } finally {
      if (typeof link.remove === 'function') link.remove();
      const revokeLater = () => {
        if (trackedUrls.delete(blobUrl) && typeof urlApi.revokeObjectURL === 'function') {
          try {
            urlApi.revokeObjectURL(blobUrl);
          } catch (_) {
            // Ignore revoke failure.
          }
        }
      };
      if (typeof opts?.scheduleRevoke === 'function') {
        opts.scheduleRevoke(revokeLater);
      } else if (typeof setTimeout === 'function') {
        setTimeout(revokeLater, 0);
      } else {
        revokeLater();
      }
    }
  }

  function render(nextViewOrList) {
    if (destroyed || !container || !doc) return;
    const renderId = ++seq;
    revokeTrackedUrls();
    const items = normalizeArtifacts(nextViewOrList);
    if (typeof container.replaceChildren === 'function') {
      container.replaceChildren();
    }
    if (items.length === 0) {
      container.hidden = true;
      return;
    }
    container.hidden = false;

    const header = el(doc, 'div', 'solve-artifacts-head');
    const title = el(doc, 'h3', 'solve-artifacts-title', `Artifacts (${items.length})`);
    const hint = el(
      doc,
      'span',
      'solve-artifacts-hint',
      'PNG previews inline; SVG, PDF, JSON, CSV, and text download safely.',
    );
    header.append(title, hint);

    const grid = el(doc, 'div', 'solve-artifacts-grid');
    const sessionId = opts.sessionId || nextViewOrList?.session_id || '';

    for (const art of items) {
      const card = el(doc, 'article', 'solve-artifact-card');
      card.setAttribute('data-artifact-id', String(art.artifact_id));
      card.setAttribute('data-artifact-mime', String(art.mime || ''));

      const top = el(doc, 'div', 'solve-artifact-meta');
      const nameEl = el(doc, 'span', 'solve-artifact-name', art.name || art.artifact_id);
      const badgeRow = el(doc, 'span', 'solve-artifact-badges');
      const mimeBadge = el(doc, 'span', 'solve-artifact-badge', mimeShortLabel(art.mime));
      const sizeEl = el(doc, 'span', 'solve-artifact-size', formatArtifactBytes(art.bytes));
      badgeRow.append(mimeBadge, sizeEl);
      top.append(nameEl, badgeRow);
      card.appendChild(top);

      const previewBox = el(doc, 'div', 'solve-artifact-preview');
      if (isPreviewableImageMime(art.mime)) {
        const statusEl = el(doc, 'span', 'solve-artifact-preview-status', 'Loading PNG preview...');
        const img = el(doc, 'img', 'solve-artifact-img');
        img.alt = String(art.name || 'PNG artifact');
        img.hidden = true;
        previewBox.append(statusEl, img);

        if (sessionId && urlApi && typeof urlApi.createObjectURL === 'function') {
          fetchArtifactBlob(sessionId, art, opts)
            .then((blob) => {
              if (destroyed || renderId !== seq) return;
              const objUrl = urlApi.createObjectURL(blob);
              if (destroyed || renderId !== seq) {
                if (typeof urlApi.revokeObjectURL === 'function') urlApi.revokeObjectURL(objUrl);
                return;
              }
              trackedUrls.add(objUrl);
              img.src = objUrl;
              img.hidden = false;
              statusEl.hidden = true;
            })
            .catch((err) => {
              if (destroyed || renderId !== seq) return;
              img.hidden = true;
              statusEl.hidden = false;
              statusEl.textContent = err?.message || 'Preview unavailable';
            });
        } else {
          statusEl.textContent = 'Preview unavailable';
        }
      } else {
        const dlNotice = el(
          doc,
          'span',
          'solve-artifact-download-only',
          `${mimeShortLabel(art.mime)} download only`,
        );
        previewBox.appendChild(dlNotice);
      }
      card.appendChild(previewBox);

      const footer = el(doc, 'div', 'solve-artifact-actions');
      const statusMsg = el(doc, 'span', 'solve-artifact-action-status', '');
      const dlBtn = el(doc, 'button', 'solve-btn solve-artifact-download', 'Download');
      dlBtn.type = 'button';
      dlBtn.addEventListener('click', async () => {
        if (destroyed || !sessionId) return;
        dlBtn.disabled = true;
        statusMsg.textContent = 'Downloading...';
        try {
          const blob = await fetchArtifactBlob(sessionId, art, opts);
          if (destroyed) return;
          triggerBlobDownload(blob, safeDownloadName(art.name, art.artifact_id));
          statusMsg.textContent = '';
        } catch (err) {
          if (!destroyed) {
            statusMsg.textContent = err?.message || 'Download failed';
          }
        } finally {
          if (!destroyed) dlBtn.disabled = false;
        }
      });
      footer.append(dlBtn, statusMsg);
      card.appendChild(footer);

      grid.appendChild(card);
    }

    container.append(header, grid);
  }

  render(viewOrArtifacts);

  return {
    update(nextViewOrList) {
      render(nextViewOrList);
    },
    destroy() {
      if (destroyed) return;
      destroyed = true;
      seq += 1;
      revokeTrackedUrls();
      if (container && typeof container.replaceChildren === 'function') {
        container.replaceChildren();
      }
    },
    get activeObjectUrlCount() {
      return trackedUrls.size;
    },
  };
}

export function mountSolveArtifactsSection(host, viewOrArtifacts, opts = {}) {
  const doc = resolveDoc(host, opts);
  if (!host || !doc) return null;
  let section = null;
  if (typeof host.querySelector === 'function') {
    section = host.querySelector('.solve-artifacts-section');
  }
  if (!section) {
    section = doc.createElement('section');
    section.className = 'solve-artifacts-section';
    section.setAttribute('data-solve-artifacts', 'true');
    if (typeof host.appendChild === 'function') {
      host.appendChild(section);
    }
  }
  const renderer = renderSolveArtifacts(section, viewOrArtifacts, {
    ...opts,
    document: doc,
  });
  return {
    section,
    update(nextViewOrList) {
      renderer.update(nextViewOrList);
    },
    destroy() {
      renderer.destroy();
      if (typeof section.remove === 'function') {
        section.remove();
      }
    },
    get activeObjectUrlCount() {
      return renderer.activeObjectUrlCount;
    },
  };
}
