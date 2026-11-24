// Rich readable node inspector for Tab 2 (Proof DAG).
import { displayMathSource, escapeHtml, renderMathTextInto, renderRichInto } from '/assets/math.js';

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

// Breadth-first discovery reports the shortest recorded dependency distance.
export function assumptionDependents(entries, root) {
  const children = new Map();
  for (const entry of entries) {
    for (const parent of entry.depends_on || []) {
      if (!children.has(parent)) children.set(parent, []);
      children.get(parent).push(entry);
    }
  }
  const distance = new Map([[root, 0]]);
  const queue = [root];
  const affected = [];
  for (let i = 0; i < queue.length; i++) {
    const parent = queue[i];
    for (const entry of children.get(parent) || []) {
      if (distance.has(entry.entry_id)) continue;
      distance.set(entry.entry_id, distance.get(parent) + 1);
      queue.push(entry.entry_id);
      affected.push(entry);
    }
  }
  return affected.map(entry => ({
    entry, direct: distance.get(entry.entry_id) === 1,
    via: [...new Set(entry.depends_on)].filter(id => distance.has(id) && distance.get(id) < distance.get(entry.entry_id)),
  }));
}

export function renderAssumptionImpact(host, entries, assumption, jump) {
  if (!host) return;
  host.replaceChildren();
  host.hidden = assumption?.entry_type !== 'assumption';
  if (host.hidden) return;
  const affected = assumptionDependents(entries, assumption.entry_id);
  const heading = document.createElement('strong');
  heading.textContent = 'If this assumption changes';
  const description = document.createElement('p');
  description.textContent = affected.length
    ? 'Recheck these downstream steps. The list follows the dependencies recorded in this proof.'
    : 'No downstream step records a dependency on this assumption.';
  host.append(heading, description);
  for (const { entry, via, direct } of affected) {
    const line = document.createElement('div');
    const link = document.createElement('button');
    link.type = 'button';
    link.className = 'dag-dep-chip';
    renderMathTextInto(link, entry.entry_id + ' · ' + (entry.summary || formatTypeLabel(entry.entry_type)));
    link.addEventListener('click', () => jump(entry.entry_id));
    const retired = ['superseded', 'rejected'].includes(String(entry.status).toLowerCase()) ? ' · ' + entry.status : '';
    const relation = document.createElement('small');
    relation.textContent = (direct ? 'Direct dependency' : 'Through ' + via.join(', ')) + retired;
    line.append(link, relation);
    host.append(line);
  }
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

const EVIDENCE_LABEL = {
  computation: 'SymPy computation',
  lean: 'Lean check',
  source_read: 'Recorded source read',
  source_ingested: 'Ingested source',
  literature: 'Literature result',
  paper_passages: 'Paper passages',
  formal_library: 'Lean library search',
  validation_decision: 'Frozen validation plan',
  citation: 'Researcher citation',
  unavailable: 'Unresolved evidence link',
};

const EVIDENCE_STATUS = {
  completed: 'completed',
  failed: 'failed',
  verified: 'type-checked',
  rejected_sorry: 'incomplete proof',
  rejected_axiom: 'axiom rejected',
  source_changed: 'source changed during check',
  lean_not_found: 'Lean unavailable',
  timeout: 'timed out',
  declined: 'declined',
  recorded: 'recorded',
  matches: 'matches recorded',
  declarations_found: 'declarations found',
  no_match: 'no match',
  library_not_found: 'library unavailable',
  frozen_checks_passed: 'checks passed',
  frozen_checks_failed: 'checks failed',
  researcher_recorded: 'researcher-entered',
  result_unavailable: 'result unavailable',
  source_record_unavailable: 'source record unavailable',
  unsupported_reference: 'unsupported tool record',
  unknown_reference: 'record unavailable',
};

const VALIDATION_LABEL = {
  symbolic_identity: 'Symbolic identity',
  numerical_convergence: 'Numerical convergence',
};

const VALIDATION_STATUS = {
  symbolic_agreement: 'zero symbolic residual',
  counterexample: 'counterexample',
  undetermined: 'undetermined',
  within_tolerance: 'within tolerance',
  not_stable: 'unstable at the chosen levels',
  outside_reference_tolerance: 'outside reference tolerance',
  insufficient_data: 'insufficient data',
  evaluation_failed: 'evaluation failed',
};

const CONVERGENCE_AXIS = {
  resolution: 'Resolution',
  basis: 'Basis size',
  precision: 'Precision (dps)',
};

function addEvidenceText(parent, tag, value, className) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (tag === 'pre' || tag === 'code') node.textContent = value;
  else renderMathTextInto(node, value);
  parent.appendChild(node);
  return node;
}

function addEvidenceField(parent, label, value, preformatted = false) {
  if (typeof value !== 'string' || !value.trim()) return;
  addEvidenceText(parent, 'div', label, 'dag-evidence-meta');
  return addEvidenceText(parent, preformatted ? 'pre' : 'p', value, preformatted ? 'trace-io' : '');
}

function addEvidenceExcerpt(parent, label, value) {
  renderMathTextInto(addEvidenceField(parent, label, value, true), value);
}

function addSourceReaderButton(parent, anchor, onOpenSource) {
  if (!anchor?.source_id || !anchor.path || !anchor.file_sha256 || !onOpenSource) return;
  const label = anchor.start_line ? 'Open cited lines' : 'Open recorded file';
  const button = addEvidenceText(parent, 'button', label, 'pill-btn sm dag-evidence-open');
  button.type = 'button';
  button.addEventListener('click', () => onOpenSource(anchor));
}

function addEvidenceSource(parent, source, onOpenSource) {
  if (!source) return;
  addEvidenceField(parent, 'Ledger source', source.title || source.source_id);
  addEvidenceField(parent, 'Materialized file', source.path);
  addEvidenceField(parent, 'Raw source SHA-256', source.raw_sha256, true);
  addEvidenceField(parent, 'Recorded file SHA-256', source.file_sha256, true);
  if (source.seq) addEvidenceText(parent, 'div', `Ingestion event ${source.seq}`, 'dag-evidence-meta');
  addSourceReaderButton(parent, source.reader, onOpenSource);
}

function validationValue(value) {
  if (value == null) return '—';
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  if (typeof value !== 'object') return String(value);
  return Object.entries(value).map(([key, part]) => `${key} = ${part}`).join(', ') || '—';
}

function readableStatus(status) {
  return typeof status === 'string' && status ? status.replaceAll('_', ' ') : '—';
}

function addValidationField(parent, label, value, preformatted = false) {
  if (value != null) addEvidenceField(parent, label, validationValue(value), preformatted);
}

function addValidationTable(parent, label, rows, columns) {
  if (!Array.isArray(rows) || !rows.length) return;
  addEvidenceText(parent, 'div', label, 'dag-evidence-meta');
  const wrapper = document.createElement('div');
  wrapper.className = 'dag-validation-table';
  wrapper.tabIndex = 0;
  wrapper.setAttribute('aria-label', label);
  const table = document.createElement('table');
  table.setAttribute('aria-label', label);
  const head = document.createElement('thead');
  const headers = document.createElement('tr');
  for (const [heading] of columns) {
    addEvidenceText(headers, 'th', heading).scope = 'col';
  }
  head.appendChild(headers);
  table.appendChild(head);
  const body = document.createElement('tbody');
  for (const row of rows) {
    const tr = document.createElement('tr');
    for (const [, field] of columns) addEvidenceText(tr, 'td', field(row || {}));
    body.appendChild(tr);
  }
  table.appendChild(body);
  wrapper.appendChild(table);
  parent.appendChild(wrapper);
}

function addSymbolicValidation(parent, report) {
  addValidationField(parent, 'Left-hand expression', report.lhs, true);
  addValidationField(parent, 'Right-hand expression', report.rhs, true);
  addValidationField(parent, 'Simplified residual', report.residual, true);
  for (const [symbol, flags] of Object.entries(report.assumptions || {})) {
    addValidationField(parent, `Assumptions for ${symbol}`, flags);
  }
  addValidationField(parent, 'Requested domain', report.domain?.requested);
  if (report.domain?.effective !== report.domain?.requested && report.domain?.effective !== report.domain?.common) {
    addValidationField(parent, 'Effective domain', report.domain.effective);
  }
  addValidationField(parent, 'Common domain', report.domain?.common);
  if (report.domain?.status) {
    addValidationField(parent, 'Domain analysis', readableStatus(report.domain.status));
  }
  if (report.dimensions?.status) {
    addValidationField(parent, 'Dimensions', readableStatus(report.dimensions.status));
    addValidationField(parent, 'Left-hand dimension', report.dimensions.lhs);
    addValidationField(parent, 'Right-hand dimension', report.dimensions.rhs);
    addValidationField(parent, 'Dimension diagnostic', report.dimensions.error);
  }
  const comparisons = [
    ['LHS', (row) => validationValue(row.lhs)],
    ['RHS', (row) => validationValue(row.rhs)],
    ['Result', (row) => readableStatus(row.status)],
  ];
  addValidationTable(parent, 'Sample points', report.samples, [
    ['Point', (row) => validationValue(row.point)],
    ...comparisons,
  ]);
  addValidationTable(parent, 'Limits', report.limits, [
    ['Point', (row) => validationValue(row.point)],
    ['Direction', (row) => validationValue(row.direction)],
    ...comparisons,
  ]);
}

function addConvergenceValidation(parent, report) {
  const axis = CONVERGENCE_AXIS[report.axis] || 'Level';
  const summary = document.createElement('div');
  summary.className = 'dag-validation-summary';
  parent.appendChild(summary);
  addValidationField(summary, 'Refinement axis', axis);
  addValidationField(summary, 'Stable across tail', report.stable ?? 'Undetermined');
  addValidationField(summary, 'Within reported tolerance', report.within_tolerance ?? 'Undetermined');
  addValidationField(summary, 'Required adjacent comparisons', report.tail);
  addValidationField(summary, 'Analysis precision (dps)', report.analysis_dps);
  addValidationField(summary, 'Absolute tolerance', report.atol);
  addValidationField(summary, 'Relative tolerance', report.rtol);
  addValidationField(summary, 'Reference', report.reference);
  addValidationField(summary, 'Reference within tolerance', report.reference_within_tolerance);
  const columns = [
    [axis, (row) => validationValue(row.level)],
    ['Work dps', (row) => validationValue(row.work_dps)],
    ['Value', (row) => validationValue(row.value)],
    ['Change', (row) => validationValue(row.delta)],
    ['Tolerance', (row) => validationValue(row.threshold)],
    ['Within tolerance', (row) => validationValue(row.passes)],
  ];
  if (report.reference != null) {
    columns.push(
      ['Reference error', (row) => validationValue(row.reference_error)],
      ['Reference tolerance', (row) => validationValue(row.reference_threshold)],
      ['Reference passes', (row) => validationValue(row.reference_passes)],
    );
  }
  if (report.samples?.some((row) => row?.error)) {
    columns.push(['Evaluation error', (row) => [row.error?.type, row.error?.message].filter(Boolean).join(' · ') || '—']);
  }
  addValidationTable(parent, 'Refinement levels', report.samples, columns);
}

function addEvidenceRecord(parent, entry, item, onSelectNode, onOpenSource) {
  const section = document.createElement('details');
  section.className = 'dag-evidence-row';
  const validation = item.kind === 'computation' && item.validation?.schema_version === 1
    && VALIDATION_LABEL[item.validation.kind] && VALIDATION_STATUS[item.validation.status]
    ? item.validation : null;
  const title = validation ? VALIDATION_LABEL[validation.kind] : EVIDENCE_LABEL[item.kind] || 'Evidence';
  const status = validation ? VALIDATION_STATUS[validation.status] : EVIDENCE_STATUS[item.status] || 'unavailable';
  addEvidenceText(section, 'summary', `${title} · ${status}`);
  if (item.via_entry_id && item.via_entry_id !== entry.entry_id) {
    const button = addEvidenceText(section, 'button', `Linked through ${item.via_entry_id}`, 'dag-dep-chip');
    button.type = 'button';
    button.addEventListener('click', () => onSelectNode?.(item.via_entry_id));
  }
  if (item.request_seq) {
    const sequence = item.result_seq ? `Request ${item.request_seq} · outcome ${item.result_seq}` : `Request ${item.request_seq}`;
    addEvidenceText(section, 'div', sequence, 'dag-evidence-meta');
  }
  addEvidenceField(section, 'Tool reference', item.tool_call_ref);
  if (item.kind === 'computation') {
    if (validation) {
      addEvidenceText(section, 'div', 'Computational evidence', 'dag-evidence-meta');
      if (validation.kind === 'symbolic_identity') addSymbolicValidation(section, validation);
      if (validation.kind === 'numerical_convergence') addConvergenceValidation(section, validation);
      if (validation.rows_omitted) addEvidenceText(section, 'p', 'Showing selected rows from the recorded report.', 'dag-evidence-meta');
    } else {
      addEvidenceField(section, 'Value', item.value, true);
      if (typeof item.latex === 'string' && item.latex.trim()) {
        addEvidenceText(section, 'div', 'LaTeX', 'dag-evidence-meta');
        const formula = addEvidenceText(section, 'div', '', 'rich-prose');
        renderRichInto(formula, displayMathSource(item.latex));
      }
    }
    addEvidenceExcerpt(section, 'Standard output', item.stdout);
    addEvidenceField(section, 'Standard error', item.stderr, true);
    if (item.worker_truncated) addEvidenceText(section, 'p', 'The worker truncated its recorded output.');
    if (item.validation_truncated) addEvidenceText(section, 'p', 'The structured report exceeded the worker’s recording limit.');
  }
  if (item.kind === 'lean') {
    addEvidenceField(section, 'Recorded Lean statement and code', item.code, true);
    addEvidenceField(section, 'Checked file', item.path);
    addEvidenceField(section, 'Checked source SHA-256', item.source_sha256, true);
    if (Array.isArray(item.axioms) && item.axioms.length > 0) {
      addEvidenceField(section, 'Declared axioms', item.axioms.join(', '));
    }
  }
  if (item.kind === 'source_read') {
    addEvidenceField(section, 'Recorded path', [item.root, item.path].filter(Boolean).join('/'));
    if (item.start_line && item.end_line) {
      addEvidenceText(section, 'div', `Lines ${item.start_line}–${item.end_line}`, 'dag-evidence-meta');
    } else if (typeof item.match_count === 'number') {
      addEvidenceText(section, 'div', `${item.match_count} recorded matches`, 'dag-evidence-meta');
    }
    addSourceReaderButton(section, item.reader, onOpenSource);
  }
  if (item.kind === 'citation') {
    addEvidenceField(section, 'Source', item.source_title || item.source_id || 'No source selected');
    addEvidenceField(section, 'DOI', item.source_doi);
    addEvidenceField(section, 'Recorded location', item.location);
    addEvidenceExcerpt(section, 'Researcher-entered quote', item.quote);
  }
  if (item.kind === 'validation_decision') {
    addEvidenceField(section, 'Plan', item.plan_id);
    if (item.result_seq) addEvidenceText(section, 'div', `Decision event ${item.result_seq}`, 'dag-evidence-meta');
    addValidationField(section, 'Candidate value', item.candidate_value, true);
    addValidationField(section, 'Known-pass control passed', item.known_pass_passed);
    addValidationField(section, 'Planted-fail control rejected', item.planted_fail_rejected);
    addEvidenceField(section, 'Plan commitment SHA-256', item.commit_hash, true);
  }
  for (const passage of Array.isArray(item.passages) ? item.passages : []) {
    const lines = passage.start_line === passage.end_line ? `line ${passage.start_line}`
      : `lines ${passage.start_line}–${passage.end_line}`;
    addEvidenceField(section, passage.title || 'Source passage', `${passage.path} · ${lines}`);
    addEvidenceExcerpt(section, 'Recorded excerpt', passage.excerpt);
    addEvidenceField(section, 'Source file SHA-256', passage.file_sha256, true);
    addSourceReaderButton(section, passage.reader, onOpenSource);
  }
  for (const declaration of Array.isArray(item.declarations) ? item.declarations : []) {
    addEvidenceField(section, declaration.name, [declaration.module, declaration.path,
      declaration.line ? `line ${declaration.line}` : null].filter(Boolean).join(' · '));
    addEvidenceField(section, 'Source declaration', declaration.signature, true);
    addEvidenceField(section, 'Source file SHA-256', declaration.source_sha256, true);
  }
  for (const record of Array.isArray(item.records) ? item.records : []) {
    const label = [record.title, record.arxiv_id, record.doi].filter(Boolean).join(' · ');
    const warning = record.withdrawn ? ' · withdrawn' : record.retracted ? ' · retracted' : '';
    if (label) addEvidenceText(section, 'p', label + warning);
  }
  addEvidenceSource(section, item.source, onOpenSource);
  addEvidenceExcerpt(section, 'Recorded excerpt', item.excerpt);
  addEvidenceField(section, 'Recorded error', item.error);
  parent.appendChild(section);
}

export function renderProofEvidence(container, entry, data, onSelectNode, onOpenSource) {
  container.replaceChildren();
  if (!data) {
    addEvidenceText(container, 'p', 'Evidence is unavailable for this entry.', 'dag-evidence-meta');
    return;
  }
  const reviews = Array.isArray(data.model_reviews) ? data.model_reviews : [];
  const support = Array.isArray(data.support) ? data.support : [];
  if (!reviews.length && !support.length) {
    addEvidenceText(container, 'p', 'No recorded evidence is linked to this entry.', 'dag-evidence-meta');
    return;
  }
  if (data.retired) addEvidenceText(container, 'p', 'Historical evidence for a superseded entry.', 'dag-evidence-meta');
  for (const review of reviews) {
    const section = document.createElement('details');
    section.className = 'dag-evidence-row';
    const scope = review.scope === 'full' ? 'Full-solution model review' : review.scope === 'step' ? 'Step model review' : 'Model review';
    addEvidenceText(section, 'summary', `${scope} · ${review.verdict}`);
    addEvidenceText(section, 'div', `Review event ${review.seq}`, 'dag-evidence-meta');
    addEvidenceField(section, 'Summary', review.summary);
    for (const issue of Array.isArray(review.issues) ? review.issues : []) addEvidenceText(section, 'p', issue);
    addEvidenceField(section, 'Reviewed answer SHA-256', review.answer_sha256, true);
    container.appendChild(section);
  }
  for (const item of support) addEvidenceRecord(container, entry, item, onSelectNode, onOpenSource);
  if (data.truncated) addEvidenceText(container, 'p', 'Some linked records are omitted. Open a dependency to inspect its evidence.', 'dag-evidence-meta');
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
    <div class="dag-section assumption-impact" id="dag-assumption-impact" hidden></div>
    <div class="dag-section">
      <div class="dag-section-title">Linked Evidence</div>
      <div id="dag-evidence" aria-live="polite"></div>
    </div>
    <div class="dag-detail-actions">
      ${typeof opts.onReviewStep === 'function' && !entry.superseded_by && statusText !== 'superseded' ? '<button id="node-review-btn" type="button" class="pill-btn sm" title="Ask the configured verifier model to review this step and its dependencies">Review this step</button>' : ''}
      <button id="node-explain-btn" type="button" class="pill-btn sm primary">Explain in Detail</button>
      <button id="node-fork-step-btn" type="button" class="pill-btn sm">Fork from Here</button>
      ${typeof opts.onPromoteStep === 'function' ? '<button id="node-promote-btn" type="button" class="pill-btn sm">Save Finding</button>' : ''}
    </div>
  `;

  if (summaryText) {
    renderRichInto(detailEl.querySelector('.dag-summary-box'), summaryText);
  }
  if (rawLatex) {
    renderRichInto(detailEl.querySelector('.dag-eq-box'), displayMathSource(rawLatex));
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

  renderAssumptionImpact(detailEl.querySelector('#dag-assumption-impact'), entries, entry, (id) => opts.onSelectNode?.(id));
  const evidence = detailEl.querySelector('#dag-evidence');
  if (evidence && opts.loadEvidence) {
    evidence.textContent = 'Loading linked evidence…';
    opts.loadEvidence(entry.entry_id).then((result) => {
      if (detailEl.querySelector('#dag-evidence') !== evidence) return;
      const record = result.entries?.find((item) => item.entry_id === entry.entry_id);
      renderProofEvidence(evidence, entry, record, opts.onSelectNode, opts.onOpenSource);
    }).catch((error) => {
      if (detailEl.querySelector('#dag-evidence') === evidence) {
        evidence.textContent = error?.message || 'The evidence request failed.';
      }
    });
  } else if (evidence) {
    renderProofEvidence(evidence, entry, null, opts.onSelectNode);
  }

  detailEl.querySelector('#node-explain-btn')?.addEventListener('click', () => {
    if (typeof opts.onExplainStep === 'function') opts.onExplainStep(entry);
  });
  const review = detailEl.querySelector('#node-review-btn');
  review?.addEventListener('click', async () => {
    review.disabled = true;
    review.textContent = 'Reviewing…';
    try {
      await opts.onReviewStep(entry.entry_id);
      opts.onNotify?.(`Review recorded for ${entry.entry_id}. Open Linked Evidence to inspect the verdict.`);
    } catch (error) {
      opts.onNotify?.(error.message || 'The review could not be completed.');
    } finally {
      review.disabled = false;
      review.textContent = 'Review this step';
    }
  });
  detailEl.querySelector('#node-fork-step-btn')?.addEventListener('click', () => {
    if (typeof opts.onForkAtStep === 'function') opts.onForkAtStep(entry);
  });
  detailEl.querySelector('#node-promote-btn')?.addEventListener('click', () => opts.onPromoteStep(entry));
}
