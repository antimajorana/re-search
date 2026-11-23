// Rich readable node inspector for Tab 2 (Proof DAG).
import { escapeHtml, renderRichInto } from '/assets/math.js';

const TYPE_LABELS = {
  assumption: 'Assumption',
  derivation: 'Derivation',
  result: 'Result',
  symbolic_computation: 'Symbolic Check',
  numerical_computation: 'Numerical Check',
  reference_lookup: 'Reference',
  verification: 'Verification',
  correction: 'Correction',
  final_answer: 'Final Answer',
};

function formatTypeLabel(raw) {
  const key = String(raw || 'step').toLowerCase();
  return TYPE_LABELS[key] || key.replace(/_/g, ' ');
}

function wrapDisplayLatex(rawLatex) {
  const trimmed = String(rawLatex || '').trim();
  if (!trimmed) return '';
  if (/^(\$\$|\\\[|\\begin\{)/.test(trimmed)) return trimmed;
  return `$$\n${trimmed}\n$$`;
}

function statusBadgeTone(status) {
  const s = String(status || 'pending').toLowerCase();
  if (s === 'accepted') return 'ok';
  if (s === 'rejected') return 'err';
  if (s === 'superseded') return '';
  return 'warn';
}

function renderDepRow(container, label, ids, byId, onJump) {
  if (!ids || ids.length === 0) return;
  const row = document.createElement('div');
  row.className = 'dag-dep-row';
  const lbl = document.createElement('span');
  lbl.className = 'dag-dep-label';
  lbl.textContent = label;
  row.appendChild(lbl);

  for (const id of ids) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'dag-dep-chip';
    btn.textContent = id;
    const target = byId.get(id);
    if (target && target.summary) {
      btn.title = String(target.summary);
    }
    btn.addEventListener('click', () => onJump(id));
    row.appendChild(btn);
  }
  container.appendChild(row);
}

export function renderDagNodeDetail(detailEl, entry, allEntries, opts = {}) {
  if (!detailEl) return;
  if (!entry) {
    detailEl.innerHTML = '<div class="dag-inspector-empty">Click any proof node above to inspect its step explanation, LaTeX equations, and dependencies.</div>';
    return;
  }

  const entries = Array.isArray(allEntries) ? allEntries : [];
  const byId = new Map(entries.map((e) => [e.entry_id, e]));
  const deps = (Array.isArray(entry.depends_on) ? entry.depends_on : []).filter(Boolean);
  const usedBy = entries
    .filter((e) => Array.isArray(e.depends_on) && e.depends_on.includes(entry.entry_id))
    .map((e) => e.entry_id);

  const statusText = String(entry.status || 'pending');
  const tone = statusBadgeTone(statusText);
  const typeLabel = formatTypeLabel(entry.entry_type);
  const rawLatex = String(entry.latex || '').trim();
  const detailsText = String(entry.details || '').trim();
  const summaryText = String(entry.summary || '').trim();
  const showDetails = detailsText && detailsText !== summaryText;
  const supText = entry.superseded_by
    ? `<span class="badge">Replaced by ${escapeHtml(String(entry.superseded_by))}</span>`
    : '';

  detailEl.innerHTML = `
    <div class="dag-detail-head">
      <div class="dag-detail-badges">
        <span class="dag-detail-id">${escapeHtml(String(entry.entry_id || ''))}</span>
        <span class="badge accent">${escapeHtml(typeLabel)}</span>
        <span class="badge ${tone}">${escapeHtml(statusText)}</span>
        ${supText}
      </div>
      <span class="dag-seq-label">seq ${escapeHtml(String(entry.seq || 0))}</span>
    </div>
    ${summaryText ? `
      <div class="dag-section">
        <div class="dag-section-title">Summary</div>
        <div class="dag-summary-box rich-prose"></div>
      </div>` : ''}
    ${rawLatex ? `
      <div class="dag-section">
        <div class="dag-section-title">
          <span>Key Equation</span>
          <button type="button" class="pill-btn sm" id="dag-copy-latex-btn">Copy LaTeX</button>
        </div>
        <div class="dag-eq-box rich-prose"></div>
      </div>` : ''}
    <div class="dag-section">
      <div class="dag-section-title">Step Explanation</div>
      <div class="dag-explanation-box rich-prose"></div>
    </div>
    <div class="dag-section" id="dag-deps-section" hidden>
      <div class="dag-section-title">Dependency Links</div>
      <div id="dag-deps-rows"></div>
    </div>
    <div class="dag-section" id="dag-verif-section" hidden>
      <div class="dag-section-title">Verifier Note</div>
      <div class="dag-verification-box rich-prose"></div>
    </div>
    <div class="dag-detail-actions">
      <button id="node-explain-btn" type="button" class="pill-btn sm primary">Explain in Detail</button>
      <button id="node-fork-step-btn" type="button" class="pill-btn sm">Fork from Here</button>
    </div>
  `;

  if (summaryText) {
    renderRichInto(detailEl.querySelector('.dag-summary-box'), summaryText);
  }
  if (rawLatex) {
    renderRichInto(detailEl.querySelector('.dag-eq-box'), wrapDisplayLatex(rawLatex));
    const copyBtn = detailEl.querySelector('#dag-copy-latex-btn');
    if (copyBtn) {
      copyBtn.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(rawLatex);
          if (typeof opts.onNotify === 'function') opts.onNotify(`Copied ${entry.entry_id} LaTeX.`);
        } catch {
          if (typeof opts.onNotify === 'function') opts.onNotify('Clipboard copy failed.');
        }
      });
    }
  }

  const expBox = detailEl.querySelector('.dag-explanation-box');
  if (expBox) {
    if (showDetails) {
      renderRichInto(expBox, detailsText);
    } else {
      expBox.innerHTML = '<span style="color:var(--ink-muted);">No expanded derivation notes attached to this node yet. Click <strong>Explain in Detail</strong> below for a step-by-step walkthrough.</span>';
    }
  }

  if (deps.length > 0 || usedBy.length > 0) {
    const depSec = detailEl.querySelector('#dag-deps-section');
    const depRows = detailEl.querySelector('#dag-deps-rows');
    if (depSec && depRows) {
      depSec.hidden = false;
      const jump = (id) => typeof opts.onSelectNode === 'function' && opts.onSelectNode(id);
      renderDepRow(depRows, 'Depends on:', deps, byId, jump);
      renderDepRow(depRows, 'Used by:', usedBy, byId, jump);
    }
  }

  if (entry.verification && typeof entry.verification === 'object') {
    const verSec = detailEl.querySelector('#dag-verif-section');
    const verBox = detailEl.querySelector('.dag-verification-box');
    if (verSec && verBox) {
      verSec.hidden = false;
      const vs = entry.verification.summary || entry.verification.verdict || '';
      const issList = Array.isArray(entry.verification.issues)
        ? entry.verification.issues.map((i) => `- **${i.severity || 'issue'}:** ${i.message || i.description || ''}`).join('\n')
        : '';
      renderRichInto(verBox, [vs, issList].filter(Boolean).join('\n\n'));
    }
  }

  detailEl.querySelector('#node-explain-btn')?.addEventListener('click', () => {
    if (typeof opts.onExplainStep === 'function') opts.onExplainStep(entry);
  });
  detailEl.querySelector('#node-fork-step-btn')?.addEventListener('click', () => {
    if (typeof opts.onForkAtStep === 'function') opts.onForkAtStep(entry);
  });
}
