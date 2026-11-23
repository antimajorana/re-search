// Structured scientific activity cards for tool calls (SymPy, Proof Ledger, Literature, Workspace)
// and Independent Verifier verdict banners. Every untrusted field is passed through escapeHtml()
// or DOM textContent — never raw HTML.
import { escapeHtml, highlightPython, renderRichInto } from '/assets/math.js';
import { linksForPaper, renderSourceLinkChips } from '/assets/source_links.js';

let seqCounter = 0;

function tryParseJson(val) {
  if (val && typeof val === 'object') return val;
  if (typeof val !== 'string') return null;
  const trimmed = val.trim();
  if (!trimmed || (trimmed[0] !== '{' && trimmed[0] !== '[')) return null;
  try {
    return JSON.parse(trimmed);
  } catch {
    return null;
  }
}

function statusTone(status, okFlag) {
  const s = String(status || (okFlag === false ? 'error' : 'ok')).toLowerCase();
  if (s === 'error' || s === 'denied' || s === 'rejected') return 'err';
  if (s === 'running' || s === 'pending') return 'warn';
  return 'ok';
}

function formatSympyCard(argsObj, outObj, rawOut) {
  const code = (argsObj && typeof argsObj.code === 'string')
    ? argsObj.code
    : (argsObj && typeof argsObj.expr === 'string' ? argsObj.expr : '');
  const codeHtml = code ? `<div class="sc-code">${highlightPython(code)}</div>` : '';

  let resHtml = '';
  if (outObj && typeof outObj === 'object') {
    const val = outObj.result !== undefined && outObj.result !== null ? String(outObj.result) : '';
    const stdout = outObj.stdout ? String(outObj.stdout).trim() : '';
    const stderr = outObj.stderr ? String(outObj.stderr).trim() : '';
    const errMsg = outObj.error ? String(outObj.error).trim() : '';
    if (val) {
      resHtml += `<div class="sc-result-row"><span class="sc-k">Result</span><code class="sc-v">${escapeHtml(val.slice(0, 900))}</code></div>`;
    }
    if (stdout && stdout !== val) {
      resHtml += `<pre class="trace-io">${escapeHtml(stdout.slice(0, 900))}</pre>`;
    }
    if (errMsg || stderr) {
      resHtml += `<pre class="trace-io sc-err-io">${escapeHtml((errMsg || stderr).slice(0, 600))}</pre>`;
    }
  } else if (rawOut) {
    resHtml = `<pre class="trace-io">${escapeHtml(String(rawOut).slice(0, 900))}</pre>`;
  }
  return {
    title: 'SymPy / Python Sandbox',
    tag: 'AppContainer',
    bodyHtml: `${codeHtml}${resHtml}`,
    latexPreview: outObj && typeof outObj.latex === 'string' ? outObj.latex : '',
  };
}

function formatLedgerCard(toolName, argsObj, outObj) {
  const isSupersede = toolName === 'ledger_supersede';
  const entryType = isSupersede
    ? 'correction'
    : String((argsObj && argsObj.entry_type) || (outObj && outObj.entry_type) || 'derivation');
  const entryId = String(
    (outObj && (outObj.entry_id || outObj.new_entry_id)) ||
    (argsObj && argsObj.entry_id) ||
    ''
  );
  const summary = String((argsObj && argsObj.summary) || (outObj && outObj.summary) || '');
  const deps = Array.isArray(argsObj && argsObj.depends_on) ? argsObj.depends_on : [];
  const depsHtml = deps.length > 0
    ? `<div class="sc-meta-line">Depends on: ${deps.map((d) => `<code>${escapeHtml(String(d))}</code>`).join(' ')}</div>`
    : '';
  const summaryHtml = summary
    ? `<div class="sc-summary-line">${escapeHtml(summary)}</div>`
    : '';
  const rawLatex = String((argsObj && argsObj.latex) || '').trim();
  return {
    title: `Proof Step · ${entryType.replace(/_/g, ' ')}`,
    tag: entryId || 'DAG',
    stepId: entryId,
    bodyHtml: `${summaryHtml}${rawLatex ? '<div class="sc-latex rich-prose"></div>' : ''}${depsHtml}`,
    latexPreview: rawLatex
      ? (/^(\$\$|\\\[|\\begin\{)/.test(rawLatex) ? rawLatex : `$$\n${rawLatex}\n$$`)
      : '',
  };
}

function formatLiteratureCard(toolName, argsObj, outObj, rawOut) {
  const src = String((argsObj && argsObj.source) || (toolName === 'arxiv_search' ? 'arXiv' : 'Literature'));
  const q = String((argsObj && (argsObj.query || argsObj.arxiv_id || argsObj.id || argsObj.origin)) || '');
  const items = Array.isArray(outObj)
    ? outObj
    : Array.isArray(outObj && (outObj.results || outObj.papers || outObj.hits || outObj.entries))
      ? (outObj.results || outObj.papers || outObj.hits || outObj.entries)
      : null;

  let hitsHtml = '';
  if (items) {
    const top = items.slice(0, 3).map((it) => {
      const id = it.arxiv_id || it.id || '';
      const title = it.title || it.primary_path || 'Entry';
      const links = linksForPaper({ ...it, source: src });
      const primary = links[0];
      const titleHtml = primary
        ? `<a class="paper-title-link" href="${escapeHtml(primary.url)}" target="_blank" rel="noopener noreferrer">${escapeHtml(String(title).slice(0, 120))} ↗</a>`
        : escapeHtml(String(title).slice(0, 120));
      const extraChips = links.length > 1 ? `<div class="sc-hit-links">${renderSourceLinkChips(links.slice(0, 3))}</div>` : '';
      return `<li>${id ? `<code>${escapeHtml(String(id))}</code> ` : ''}${titleHtml}${extraChips}</li>`;
    }).join('');
    hitsHtml = `<div class="sc-meta-line">${items.length} result${items.length === 1 ? '' : 's'} returned</div>${top ? `<ul class="sc-hit-list">${top}</ul>` : ''}`;
  } else if (outObj && outObj.source_id) {
    hitsHtml = `<div class="sc-meta-line">Ingested <code>${escapeHtml(String(outObj.primary_path || outObj.source_id))}</code></div>`;
  } else if (rawOut) {
    hitsHtml = `<pre class="trace-io">${escapeHtml(String(rawOut).slice(0, 600))}</pre>`;
  }

  return {
    title: `Literature · ${src}`,
    tag: q ? q.slice(0, 34) : 'Read',
    bodyHtml: hitsHtml,
  };
}

function formatWorkspaceCard(toolName, argsObj, outObj, rawOut) {
  const path = String((argsObj && (argsObj.path || argsObj.rel_path || argsObj.tex_path)) || '');
  let note = '';
  if (outObj && typeof outObj === 'object') {
    if (typeof outObj.ok === 'boolean' && outObj.blocker_count !== undefined) {
      note = outObj.ok
        ? 'TeX audit clean (0 blockers)'
        : `${Number(outObj.blocker_count) || 0} blockers, ${Number(outObj.error_count) || 0} errors`;
    } else if (Array.isArray(outObj.entries || outObj.files)) {
      const arr = outObj.entries || outObj.files;
      note = `Listed ${arr.length} workspace file${arr.length === 1 ? '' : 's'}`;
    } else if (outObj.bytes_written !== undefined || outObj.patched) {
      note = 'Updated workspace file cleanly';
    }
  }
  const detail = note
    ? `<div class="sc-meta-line">${escapeHtml(note)}</div>`
    : (rawOut ? `<pre class="trace-io">${escapeHtml(String(rawOut).slice(0, 500))}</pre>` : '');
  return {
    title: `Workspace · ${toolName}`,
    tag: path || 'local',
    bodyHtml: detail,
  };
}

const DELEGATION_TITLES = {
  delegate_compute: 'Compute Worker',
  delegate_literature: 'Literature Worker',
  delegate_lean: 'Lean Prover Worker',
  delegate_explore: 'Parallel Explorer (Ultramath)',
  lean_check: 'Lean 4 Checker',
};

function formatDelegationCard(toolName, argsObj, outObj, rawOut) {
  const title = toolName === 'delegate_subagent'
    ? `Subagent · ${(outObj && outObj.worker_name) || (argsObj && argsObj.subagent_id) || 'Custom'}`
    : (DELEGATION_TITLES[toolName] || toolName);
  const tag = String(
    (outObj && outObj.model) ||
    (toolName === 'lean_check' && outObj && outObj.status) ||
    'Running...'
  );

  const metaParts = [];
  if (outObj && typeof outObj === 'object') {
    if (outObj.turns !== undefined && outObj.turns !== null) {
      const t = Number(outObj.turns) || 0;
      metaParts.push(`${t} turn${t === 1 ? '' : 's'}`);
    }
    if (outObj.input_tokens !== undefined || outObj.output_tokens !== undefined) {
      const totalTok = (Number(outObj.input_tokens) || 0) + (Number(outObj.output_tokens) || 0);
      metaParts.push(`${totalTok} tokens`);
    }
    if (outObj.branch_count !== undefined && outObj.branch_count !== null) {
      const bc = Number(outObj.branch_count) || 0;
      metaParts.push(`${bc} branch${bc === 1 ? '' : 'es'}`);
    } else if (Array.isArray(outObj.branches)) {
      const bc = outObj.branches.length;
      metaParts.push(`${bc} branch${bc === 1 ? '' : 'es'}`);
    }
    if (outObj.routing_reason) metaParts.push(String(outObj.routing_reason));
    if (Array.isArray(outObj.tools_used) && outObj.tools_used.length > 0) {
      metaParts.push(`tools: ${outObj.tools_used.map(String).join(', ')}`);
    }
    if (toolName === 'lean_check') {
      if (outObj.status) metaParts.push(`status: ${String(outObj.status)}`);
      if (Array.isArray(outObj.axioms)) {
        metaParts.push(`axioms: ${outObj.axioms.length ? outObj.axioms.map(String).join(', ') : 'none'}`);
      } else if (outObj.axioms) {
        metaParts.push(`axioms: ${String(outObj.axioms)}`);
      }
      if (Array.isArray(outObj.diagnostics)) {
        metaParts.push(`${outObj.diagnostics.length} diagnostic${outObj.diagnostics.length === 1 ? '' : 's'}`);
      } else if (typeof outObj.diagnostics === 'number') {
        metaParts.push(`${outObj.diagnostics} diagnostic${outObj.diagnostics === 1 ? '' : 's'}`);
      }
      if (outObj.hint) metaParts.push(`hint: ${String(outObj.hint)}`);
    }
  }
  const metaHtml = metaParts.length > 0
    ? `<div class="sc-meta-line">${escapeHtml(metaParts.join(' · '))}</div>`
    : '';

  const taskText = String((argsObj && (argsObj.task || argsObj.problem || argsObj.statement || argsObj.path)) || '').trim();
  const taskHtml = taskText
    ? `<div class="sc-meta-line">${escapeHtml(taskText.slice(0, 260))}${taskText.length > 260 ? '…' : ''}</div>`
    : '';

  const proofEntries = outObj && Array.isArray(outObj.proof_entries) ? outObj.proof_entries : [];
  const jumpsHtml = proofEntries.length > 0
    ? `<div class="sc-head-left">${proofEntries
        .map((eid) => String(eid || '').trim())
        .filter(Boolean)
        .map((eid) => `<button type="button" class="pill-btn sm sc-step-jump" data-step-id="${escapeHtml(eid)}">Inspect ${escapeHtml(eid)} ↗</button>`)
        .join('')}</div>`
    : '';

  let bodyDetail = '';
  if (outObj && Array.isArray(outObj.branches) && outObj.branches.length > 0) {
    const items = outObj.branches.slice(0, 6).map((b, i) => {
      const bo = b && typeof b === 'object' ? b : { summary: String(b || '') };
      const label = String(bo.approach || bo.name || bo.label || bo.branch || `Branch ${i + 1}`);
      const stat = bo.status !== undefined ? String(bo.status) : (typeof bo.ok === 'boolean' ? (bo.ok ? 'ok' : 'error') : '');
      const bModel = bo.model ? ` · ${String(bo.model)}` : '';
      const bSum = String(bo.summary || bo.result || bo.error || '').trim();
      return `<li><strong>${escapeHtml(label)}</strong>${stat || bModel ? ` <code>${escapeHtml(`${stat}${bModel}`.trim())}</code>` : ''}${bSum ? `: ${escapeHtml(bSum.slice(0, 220))}${bSum.length > 220 ? '…' : ''}` : ''}</li>`;
    }).join('');
    bodyDetail += `<ul class="sc-hit-list">${items}</ul>`;
  }
  if (outObj && outObj.summary) {
    const sum = String(outObj.summary).trim();
    bodyDetail += `<div class="sc-summary-line">${escapeHtml(sum.slice(0, 500))}${sum.length > 500 ? '…' : ''}</div>`;
  }
  if (toolName === 'lean_check' && outObj && Array.isArray(outObj.diagnostics) && outObj.diagnostics.length > 0) {
    const diags = outObj.diagnostics.slice(0, 4).map((d) => {
      const msg = typeof d === 'string' ? d : String((d && (d.message || d.msg || d.data)) || JSON.stringify(d));
      return `<li>${escapeHtml(msg.slice(0, 220))}</li>`;
    }).join('');
    bodyDetail += `<ul class="sc-hit-list">${diags}</ul>`;
  }
  if (outObj && outObj.error) {
    bodyDetail += `<pre class="trace-io sc-err-io">${escapeHtml(String(outObj.error).slice(0, 600))}</pre>`;
  } else if (!outObj && rawOut) {
    bodyDetail += `<pre class="trace-io">${escapeHtml(String(rawOut).slice(0, 600))}</pre>`;
  }

  return {
    title,
    tag,
    bodyHtml: `${metaHtml}${taskHtml}${jumpsHtml}${bodyDetail}`,
  };
}

export function buildStructuredTraceCard(tr, opts = {}) {
  seqCounter += 1;
  const toolName = String((tr && (tr.tool || tr.tool_id)) || 'tool');
  const status = String((tr && tr.status) || (tr && tr.ok === false ? 'error' : 'ok'));
  const tone = statusTone(status, tr && tr.ok);
  const argsObj = tryParseJson(tr && (tr.args !== undefined ? tr.args : tr.input));
  const rawArgsStr = typeof (tr && tr.args) === 'string'
    ? tr.args
    : JSON.stringify((tr && (tr.args || tr.input)) || {}, null, 2);
  const rawOutStr = tr && tr.output !== undefined
    ? (typeof tr.output === 'string' ? tr.output : JSON.stringify(tr.output, null, 2))
    : '';
  const outObj = tryParseJson(tr && tr.output);

  let spec = null;
  if (toolName === 'sympy_eval') {
    spec = formatSympyCard(argsObj, outObj, rawOutStr);
  } else if (toolName === 'ledger_append' || toolName === 'ledger_supersede') {
    spec = formatLedgerCard(toolName, argsObj, outObj);
  } else if (toolName === 'arxiv_search' || toolName === 'literature_search' || toolName === 'ingest_source') {
    spec = formatLiteratureCard(toolName, argsObj, outObj, rawOutStr);
  } else if (
    toolName === 'delegate_compute' || toolName === 'delegate_literature' ||
    toolName === 'delegate_lean' || toolName === 'delegate_explore' ||
    toolName === 'delegate_subagent' || toolName === 'lean_check'
  ) {
    spec = formatDelegationCard(toolName, argsObj, outObj, rawOutStr);
  } else if (/^(fs_|latex_|tex_audit)/.test(toolName)) {
    spec = formatWorkspaceCard(toolName, argsObj, outObj, rawOutStr);
  }

  const box = document.createElement('div');
  box.className = 'trace-item structured-trace';
  box.dataset.traceKey = (tr && tr.call_id) || `t-${seqCounter}`;
  box.dataset.toolName = toolName;
  box.dataset.argsStr = rawArgsStr;

  if (!spec) {
    box.innerHTML = `
      <div class="trace-head">
        <strong>${escapeHtml(toolName)}</strong>
        <span class="badge ${tone}">${escapeHtml(status)}</span>
      </div>
      ${rawArgsStr && rawArgsStr !== '{}' && rawArgsStr !== 'null' ? `<pre class="trace-io">${escapeHtml(rawArgsStr.slice(0, 600))}</pre>` : ''}
      ${rawOutStr ? `<pre class="trace-io">${escapeHtml(rawOutStr.slice(0, 900))}</pre>` : ''}
    `;
    return box;
  }

  const stepBtnHtml = spec.stepId
    ? `<button type="button" class="pill-btn sm sc-step-jump" data-step-id="${escapeHtml(spec.stepId)}">Inspect ${escapeHtml(spec.stepId)} ↗</button>`
    : (spec.tag ? `<span class="badge">${escapeHtml(spec.tag)}</span>` : '');

  box.innerHTML = `
    <div class="trace-head">
      <div class="sc-head-left">
        <strong>${escapeHtml(spec.title)}</strong>
        ${stepBtnHtml}
      </div>
      <span class="badge ${tone}">${escapeHtml(status)}</span>
    </div>
    ${spec.bodyHtml || ''}
  `;

  if (spec.latexPreview) {
    const texTarget = box.querySelector('.sc-latex');
    if (texTarget) {
      renderRichInto(texTarget, spec.latexPreview);
    }
  }

  if (typeof opts.onInspectStep === 'function') {
    const btns = typeof box.querySelectorAll === 'function'
      ? box.querySelectorAll('.sc-step-jump')
      : [box.querySelector && box.querySelector('.sc-step-jump')].filter(Boolean);
    for (const btn of btns) {
      const targetId = (btn.dataset && btn.dataset.stepId) || spec.stepId;
      if (targetId) btn.addEventListener('click', () => opts.onInspectStep(targetId));
    }
  }
  return box;
}

export function buildRecoveryPrompt(action, ctx = {}) {
  const target = ctx.targetId ? ` (${ctx.targetId})` : '';
  const summary = String(ctx.summary || '').trim();
  const topIssues = (Array.isArray(ctx.issues) ? ctx.issues : [])
    .slice(0, 3)
    .map((iss) => String(iss.description || iss.message || ''))
    .filter(Boolean)
    .join('; ');
  if (action === 'alternative') {
    return (
      `Verification did not accept the draft${target}${summary ? ` (${summary})` : ''}. ` +
      `Propose and pursue an alternative proof strategy (compare candidate approaches if helpful). ` +
      `If the exact claim cannot hold as stated, state clearly what is provable under modified assumptions or give an explicit counterexample.`
    );
  }
  if (action === 'fix') {
    return (
      `Revise the draft${target} to resolve every flagged Verifier issue` +
      `${topIssues ? `: ${topIssues}` : ''}. Preserve sound earlier steps, supersede any flawed Proof-DAG entries, and run verification again.`
    );
  }
  return (
    `Explain why verification rejected ${ctx.targetId || 'the draft'}` +
    `${summary ? ` (${summary})` : ''}, which parts are already sound, and what alternative approaches or modified hypotheses would succeed.`
  );
}

export function buildVerificationVerdictCard(msgPayload, onInspectStep, onRecoveryAction) {
  if (!msgPayload) return null;
  const verifications = Array.isArray(msgPayload.verifications) ? msgPayload.verifications : [];
  const status = msgPayload.verified_status || '';
  if (!status && verifications.length === 0) return null;

  const latest = verifications.length > 0 ? verifications[verifications.length - 1] : null;
  const isVerified = status === 'verified' || (!status && latest && latest.verdict === 'accepted');
  const rounds = Number(msgPayload.verification_rounds) || verifications.length || 1;
  const targetId = String(msgPayload.verified_entry_id || (latest && latest.target_entry_id) || '');
  const summary = String((latest && latest.summary) || (isVerified ? 'All active Proof-DAG steps passed stateless verification.' : 'Independent Verifier reported unresolved issues.'));
  const issues = Array.isArray(latest && latest.issues) ? latest.issues : [];
  const recovery = typeof msgPayload.recovery === 'string' ? msgPayload.recovery.trim() : '';

  const card = document.createElement('div');
  card.className = `verdict-card ${isVerified ? 'is-verified' : 'is-unverified'}`;

  const roundLabel = `${rounds} verification round${rounds === 1 ? '' : 's'}`;
  const stepJumpHtml = targetId
    ? `<button type="button" class="pill-btn sm verdict-step-btn">Inspect ${escapeHtml(targetId)} in DAG ↗</button>`
    : '';
  const issuesHtml = (!isVerified && issues.length > 0)
    ? `<details class="verdict-issues" open><summary>${issues.length} flagged issue${issues.length === 1 ? '' : 's'}</summary><ul>${
      issues.map((iss) => {
        const sev = escapeHtml(String(iss.severity || 'issue'));
        const loc = iss.entry_id ? `<code>${escapeHtml(String(iss.entry_id))}</code> ` : '';
        const desc = escapeHtml(String(iss.description || iss.message || JSON.stringify(iss)));
        return `<li><span class="badge warn">${sev}</span> ${loc}<span class="verdict-issue-desc">${desc}</span></li>`;
      }).join('')
    }</ul></details>`
    : '';
  const recoveryHtml = !isVerified
    ? `<div class="verdict-recovery">
        ${recovery ? `<div class="verdict-recovery-body rich-prose">${escapeHtml(recovery)}</div>` : ''}
        <div class="verdict-actions">
          <button type="button" class="pill-btn sm primary verdict-recover-btn" data-recovery="alternative">Try Another Approach</button>
          <button type="button" class="pill-btn sm verdict-recover-btn" data-recovery="fix">Fix Flagged Issues</button>
          <button type="button" class="pill-btn sm verdict-recover-btn" data-recovery="explain">Discuss in Explain</button>
        </div>
      </div>`
    : '';

  card.innerHTML = `
    <div class="verdict-head">
      <div class="verdict-title-wrap">
        <span class="badge ${isVerified ? 'ok' : 'warn'}">${isVerified ? '✓ Independently Verified' : '⚠ Unverified Draft'}</span>
        <span class="verdict-meta">${escapeHtml(roundLabel)}</span>
      </div>
      ${stepJumpHtml}
    </div>
    <div class="verdict-summary rich-prose">${escapeHtml(summary)}</div>
    ${issuesHtml}
    ${recoveryHtml}
  `;

  const sumEl = card.querySelector('.verdict-summary');
  if (sumEl) renderRichInto(sumEl, summary);
  if (recovery) {
    const recEl = card.querySelector('.verdict-recovery-body');
    if (recEl) renderRichInto(recEl, recovery);
  }
  if (!isVerified && typeof card.querySelectorAll === 'function') {
    const issueEls = card.querySelectorAll('.verdict-issue-desc');
    issueEls.forEach((el, idx) => {
      const iss = issues[idx];
      if (iss) renderRichInto(el, String(iss.description || iss.message || ''));
    });
    if (typeof onRecoveryAction === 'function') {
      for (const actBtn of card.querySelectorAll('.verdict-recover-btn')) {
        actBtn.addEventListener('click', () => {
          onRecoveryAction(actBtn.dataset.recovery || 'alternative', { targetId, summary, issues });
        });
      }
    }
  }

  const btn = card.querySelector('.verdict-step-btn');
  if (btn && targetId && typeof onInspectStep === 'function') {
    btn.addEventListener('click', () => onInspectStep(targetId));
  }
  return card;
}

export function buildHonestyNoticeCard(honesty) {
  if (!honesty || !Array.isArray(honesty.findings) || honesty.findings.length === 0) return null;
  const card = document.createElement('div');
  card.className = 'trace-item structured-trace verdict-card is-unverified honesty-notice';
  const count = honesty.findings.length;
  const summaryHtml = honesty.summary
    ? `<div class="sc-summary-line">${escapeHtml(String(honesty.summary))}</div>`
    : '';
  const itemsHtml = honesty.findings.map((f) => {
    const fo = f && typeof f === 'object' ? f : { reason: String(f || '') };
    const sev = String(fo.severity || 'warn');
    const sevTone = /^(error|blocker|critical|high)$/i.test(sev) ? 'err' : 'warn';
    const claim = fo.claim ? `<code>${escapeHtml(String(fo.claim))}</code> ` : '';
    const reason = fo.reason ? escapeHtml(String(fo.reason)) : '';
    const suggestion = fo.suggestion
      ? `<div class="sc-meta-line">Suggestion: ${escapeHtml(String(fo.suggestion))}</div>`
      : '';
    return `<li><span class="badge ${sevTone}">${escapeHtml(sev)}</span> ${claim}${reason}${suggestion}</li>`;
  }).join('');

  card.innerHTML = `
    <div class="trace-head">
      <div class="sc-head-left">
        <span class="badge warn">⚠ Honesty Linter</span>
        <span class="verdict-meta">${count} finding${count === 1 ? '' : 's'}</span>
      </div>
    </div>
    ${summaryHtml}
    <ul class="sc-hit-list">${itemsHtml}</ul>
  `;
  return card;
}
