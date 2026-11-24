// Research Workbench enhancements:
// - Live multi-agent pipeline stepper (Orchestrator & Compute -> Proof Ledger -> Verifier -> Strategist)
// - Rigor mode presets (Direct / Verified / Deep) synced with Verifier & Strategist chips
// - Quick Pinned Conventions popover with physics/math presets
// - Drag-resizable Right Canvas + LaTeX Split/Editor/Preview switcher + tab count badges
// - Session search filter, connection readiness indicator, and keyboard shortcuts.
import * as api from '/assets/api.js';
import { escapeHtml } from '/assets/math.js';

const CANVAS_WIDTH_KEY = 're-search-canvas-width';
const CONVENTION_PRESETS = [
  { label: 'Natural units', text: 'Natural units: \\hbar = c = k_B = 1.' },
  { label: 'Mostly-plus (-,+,+,+)', text: 'Minkowski metric signature \\eta_{\\mu\\nu} = \\mathrm{diag}(-,+,+,+).' },
  { label: 'Mostly-minus (+,-,-,-)', text: 'Minkowski metric signature \\eta_{\\mu\\nu} = \\mathrm{diag}(+,-,-,-).' },
  { label: 'Fourier convention', text: 'Fourier transform: f(k) = \\int dx\\, e^{-i k x} f(x), f(x) = \\int \\frac{dk}{2\\pi}\\, e^{i k x} f(k).' },
];

let timerHandle = null;
let turnStartedMs = 0;
let activeRound = 1;
let maxRounds = 3;
let currentConventions = '';
let conventionsRequest = 0;
let connectedCount = 0;
let sympyWorker = null;
let callbacks = {};

export function initWorkbench(opts = {}) {
  callbacks = opts;
  wireResearchOptions();
  wireModePresets();
  wireConventionsPopover();
  wireCanvasResizerAndViews();
  wireResponsiveDrawers();
  wireKeyboardShortcuts();
  refreshConventionsState();
  refreshConnectionBadge(false);
}

export function getWorkbenchSnapshot() {
  return {
    connectedCount,
    sympyWorker,
    hasConventions: Boolean(currentConventions && currentConventions.trim()),
    conventions: currentConventions,
  };
}

// --- 1. Live Multi-Agent Pipeline Stepper ---

function setStageState(stageId, stateStr) {
  const node = document.querySelector(`#pipeline-bar [data-stage="${stageId}"]`);
  if (!node) return;
  node.setAttribute('data-state', stateStr || 'idle');
}

export function startLivePipeline(opts = {}) {
  const bar = document.getElementById('pipeline-bar');
  if (bar) bar.hidden = false;
  activeRound = 1;
  maxRounds = 3;
  turnStartedMs = Date.now();

  setStageState('solver', 'active');
  setStageState('ledger', 'idle');
  setStageState('verifier', opts.verification_mode === false ? 'skipped' : 'idle');
  setStageState('strategist', opts.use_strategist ? 'idle' : 'skipped');
  updatePipelineMeta(`Round 1/${maxRounds}`, '00:00');

  if (timerHandle) clearInterval(timerHandle);
  timerHandle = setInterval(() => {
    const sec = Math.max(0, Math.floor((Date.now() - turnStartedMs) / 1000));
    const mm = String(Math.floor(sec / 60)).padStart(2, '0');
    const ss = String(sec % 60).padStart(2, '0');
    const timerEl = document.getElementById('pipeline-timer');
    if (timerEl) timerEl.textContent = `${mm}:${ss}`;
  }, 500);
}

function updatePipelineMeta(roundText, timerText) {
  const rEl = document.getElementById('pipeline-round-badge');
  const tEl = document.getElementById('pipeline-timer');
  if (rEl && roundText !== null) {
    rEl.hidden = !roundText;
    rEl.textContent = roundText || '';
  }
  if (tEl && timerText !== null) {
    tEl.hidden = !timerText;
    tEl.textContent = timerText || '';
  }
}

export function updatePipelineFromEvent(kind, payload = {}) {
  const bar = document.getElementById('pipeline-bar');
  if (bar && bar.hidden) bar.hidden = false;
  if (payload.verification_invalidated) setStageState('verifier', 'idle');

  if (kind === 'solver_stream' || kind === 'model.request') {
    setStageState('solver', 'active');
  } else if (kind === 'tool_trace') {
    const tool = String(payload.tool || '');
    if (tool === 'ledger_append' || tool === 'ledger_supersede') {
      setStageState('ledger', 'active');
    } else {
      setStageState('solver', 'active');
    }
  } else if (kind === 'dag_updated') {
    const dagKind = String(payload.kind || '');
    if (dagKind === 'proof.entry' || dagKind === 'proof.supersede') {
      setStageState('ledger', 'done');
    } else if (dagKind === 'proof.verification' && payload.verified_status) {
      setStageState('solver', 'done');
      setStageState('ledger', 'done');
      setStageState('verifier', payload.verified_status === 'verified' ? 'done' : 'warn');
    }
  } else if (kind === 'thinking_delta') {
    const txt = String(payload.delta || payload.text || '');
    const m = txt.match(/\[Verifier round (\d+)\/(\d+)/i);
    if (m) {
      activeRound = Number(m[1]) || activeRound;
      maxRounds = Number(m[2]) || maxRounds;
      setStageState('solver', 'done');
      setStageState('ledger', 'done');
      setStageState('verifier', 'active');
      updatePipelineMeta(`Round ${activeRound}/${maxRounds}`, null);
    } else if (/Strategist/i.test(txt)) {
      setStageState('strategist', 'active');
    }
  } else if (kind === 'round_reset') {
    const nextRound = (Number(payload.round) || activeRound) + 1;
    activeRound = nextRound;
    setStageState('verifier', 'warn');
    setStageState('strategist', 'done');
    setStageState('solver', 'active');
    updatePipelineMeta(`Revising · Round ${activeRound}/${maxRounds}`, null);
  } else if (kind === 'session_event' && payload.kind === 'strategist.consult') {
    setStageState('strategist', 'active');
  }
}

export function finishLivePipeline(outcome = 'ok', stats = null) {
  if (timerHandle) {
    clearInterval(timerHandle);
    timerHandle = null;
  }
  const timerEl = document.getElementById('pipeline-timer');
  if (timerEl) timerEl.hidden = true;

  if (outcome === 'error' || outcome === 'stopped') {
    setStageState('solver', outcome === 'stopped' ? 'warn' : 'err');
    updatePipelineMeta(outcome === 'stopped' ? 'Stopped' : 'Halted', null);
    return;
  }
  setStageState('solver', 'done');
  if (stats) {
    applySessionTelemetry(stats, true);
  } else {
    updatePipelineMeta('Completed', null);
  }
}

export function applySessionTelemetry(stats, hasMessages = false) {
  if (timerHandle) return; // Don't overwrite live running state
  const bar = document.getElementById('pipeline-bar');
  if (!bar) return;
  const s = stats || {};
  const proofCount = Number(s.proof_entries) || 0;
  const rounds = Number(s.verification_rounds) || 0;
  const delegCount = Number(s.delegation_count) || 0;
  const honestyWarn = Number(s.honesty_warnings) || 0;
  const vStatus = String(s.verified_status || 'none');
  const tokTotal = (Number(s.input_tokens) || 0) + (Number(s.output_tokens) || 0);
  const workerTok = (Number(s.worker_input_tokens) || 0) + (Number(s.worker_output_tokens) || 0);
  const allTok = tokTotal + workerTok;

  if (!hasMessages && proofCount === 0 && rounds === 0 && delegCount === 0 && honestyWarn === 0) {
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  setStageState('solver', hasMessages ? 'done' : 'idle');
  setStageState('ledger', proofCount > 0 ? 'done' : 'idle');
  setStageState(
    'verifier',
    vStatus === 'verified' ? 'done' : vStatus === 'unverified' ? 'warn' : 'idle'
  );
  setStageState('strategist', rounds > 1 ? 'done' : 'idle');

  const metaParts = [];
  if (vStatus === 'verified') metaParts.push('✓ Verified');
  else if (vStatus === 'unverified') metaParts.push('⚠ Unverified');
  else if (vStatus === 'proof_reviewed') metaParts.push('Proof graph reviewed');
  if (proofCount > 0) metaParts.push(`${proofCount} DAG step${proofCount === 1 ? '' : 's'}`);
  if (delegCount > 0) metaParts.push(`${delegCount} worker${delegCount === 1 ? '' : 's'}`);
  if (honestyWarn > 0) metaParts.push(`⚠ ${honestyWarn} honesty`);
  if (rounds > 0) metaParts.push(`${rounds} check${rounds === 1 ? '' : 's'}`);
  if (allTok > 0) {
    const tokFmt = allTok >= 1000 ? `${(allTok / 1000).toFixed(1)}k tok` : `${allTok} tok`;
    metaParts.push(workerTok > 0 ? `${tokFmt} (+${workerTok >= 1000 ? `${(workerTok / 1000).toFixed(1)}k` : workerTok} w)` : tokFmt);
  }
  updatePipelineMeta(metaParts.join(' · ') || 'Session ready', null);
}

// --- 2. Rigor Mode Presets synced with Verifier & Strategist chips ---

function wireResearchOptions() {
  const options = document.getElementById('research-options');
  if (!options) return;
  const toggle = options.querySelector('summary');
  const close = () => { options.open = false; };
  document.addEventListener('pointerdown', (event) => {
    if (options.open && !options.contains(event.target)) close();
  });
  options.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && options.open) {
      event.preventDefault();
      event.stopPropagation();
      close();
      toggle.focus();
    }
  });
  // Return the space to the conversation when a choice opens its own panel.
  options.addEventListener('click', (event) => {
    if (event.target.closest('.option-controls button')) close();
  });
  options.addEventListener('focusout', (event) => {
    if (event.relatedTarget && !options.contains(event.relatedTarget)) close();
  });
}

function wireModePresets() {
  const group = document.getElementById('mode-preset-group');
  const presetBtns = document.querySelectorAll('[data-rigor-preset]');
  const vChip = document.getElementById('verifier-chip');
  const sChip = document.getElementById('strategist-chip');
  const pauseChip = document.getElementById('pause-explore-chip');

  const syncFromChips = () => {
    const isInteractive = Boolean(group && group.getAttribute('data-interactive') === 'true');
    const vOn = Boolean(vChip && vChip.getAttribute('aria-pressed') === 'true');
    const sOn = Boolean(sChip && sChip.getAttribute('aria-pressed') === 'true');
    const active = isInteractive ? 'interactive' : (!vOn && !sOn ? 'direct' : (vOn && sOn ? 'deep' : 'verified'));
    for (const b of presetBtns) {
      const match = b.getAttribute('data-rigor-preset') === active;
      b.classList.toggle('is-active', match);
      b.setAttribute('aria-pressed', match ? 'true' : 'false');
    }
  };

  for (const b of presetBtns) {
    b.addEventListener('click', () => {
      const mode = b.getAttribute('data-rigor-preset');
      if (mode === 'interactive') {
        if (group) group.setAttribute('data-interactive', 'true');
        if (vChip) vChip.setAttribute('aria-pressed', 'true');
        if (sChip) sChip.setAttribute('aria-pressed', 'false');
        if (pauseChip) pauseChip.setAttribute('aria-pressed', 'true');
      } else {
        if (group) group.removeAttribute('data-interactive');
        if (vChip) vChip.setAttribute('aria-pressed', mode === 'direct' ? 'false' : 'true');
        if (sChip) sChip.setAttribute('aria-pressed', mode === 'deep' ? 'true' : 'false');
        if (pauseChip) pauseChip.setAttribute('aria-pressed', 'false');
      }
      syncFromChips();
    });
  }
  if (pauseChip) {
    pauseChip.addEventListener('click', () => {
      const next = pauseChip.getAttribute('aria-pressed') !== 'true';
      pauseChip.setAttribute('aria-pressed', next ? 'true' : 'false');
    });
  }
  for (const chip of [vChip, sChip]) {
    if (chip) {
      chip.addEventListener('click', () => {
        if (group) group.removeAttribute('data-interactive');
        syncFromChips();
      });
    }
  }
  syncFromChips();
}

export function setRigorPreset(mode) {
  const target = document.querySelector(`[data-rigor-preset="${CSS.escape(String(mode))}"]`);
  if (target) target.click();
}

// --- 3. Quick Pinned Conventions Popover ---

function wireConventionsPopover() {
  const btn = document.getElementById('conventions-btn');
  const pop = document.getElementById('conventions-popover');
  const input = document.getElementById('quick-conventions-input');
  const presetsBox = document.getElementById('conventions-presets');
  const saveBtn = document.getElementById('conventions-save-btn');
  const clearBtn = document.getElementById('conventions-clear-btn');
  const closeBtn = document.getElementById('conventions-close-btn');

  if (presetsBox) {
    presetsBox.innerHTML = '';
    for (const p of CONVENTION_PRESETS) {
      const chip = document.createElement('button');
      chip.type = 'button';
      chip.className = 'pill-btn sm';
      chip.textContent = `+ ${p.label}`;
      chip.addEventListener('click', () => {
        if (!input) return;
        const cur = input.value.trim();
        input.value = cur ? `${cur}\n${p.text}` : p.text;
        input.focus();
      });
      presetsBox.appendChild(chip);
    }
  }

  const closePop = () => {
    if (!pop || pop.hidden) return;
    const restoreFocus = pop.contains(document.activeElement) || document.activeElement === document.body;
    pop.hidden = true;
    if (restoreFocus) document.querySelector('#research-options summary')?.focus();
  };
  pop?.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closePop();
    }
  });

  if (btn && pop) {
    btn.addEventListener('click', async () => {
      const opening = pop.hidden;
      pop.hidden = !opening;
      if (opening) {
        await refreshConventionsState();
        if (!pop.hidden) input?.focus();
      }
    });
  }
  if (closeBtn) closeBtn.addEventListener('click', closePop);
  if (clearBtn && input) {
    clearBtn.addEventListener('click', () => {
      input.value = '';
      input.focus();
    });
  }
  if (saveBtn && input) {
    saveBtn.addEventListener('click', async () => {
      saveBtn.disabled = true;
      const trimmed = input.value.trim();
      const sid = callbacks.getActiveSessionId?.();
      try {
        if (sid) await api.setSessionConventions(sid, trimmed);
        else await api.updateProviderSettings({ pinned_conventions: trimmed });
        if (sid === callbacks.getActiveSessionId?.()) {
          refreshConventionsState(trimmed);
          closePop();
        }
        notify(
          sid
            ? (trimmed ? 'Session conventions saved for Orchestrator, workers & Verifier.' : 'Session conventions cleared.')
            : (trimmed ? 'Default conventions saved for prompts & Verifier.' : 'Default conventions cleared.')
        );
      } catch (err) {
        notify(`Could not save conventions: ${err.message}`);
      } finally {
        saveBtn.disabled = false;
      }
    });
  }
}

export async function refreshConventionsState(overrideText) {
  const request = ++conventionsRequest;
  const sid = callbacks.getActiveSessionId?.();
  const apply = (text) => {
    if (request !== conventionsRequest || sid !== callbacks.getActiveSessionId?.()) return;
    currentConventions = text;
    syncConventionsIndicators();
  };
  if (typeof overrideText === 'string') {
    apply(overrideText);
    return;
  }
  if (sid && overrideText !== null) {
    try {
      const sess = await api.getSession(sid);
      if (typeof sess?.meta?.pinned_conventions === 'string') {
        apply(sess.meta.pinned_conventions);
        return;
      }
    } catch {
      // Fall back to global provider settings
    }
  }
  try {
    const s = await api.getProviderSettings();
    apply(String(s?.pinned_conventions || ''));
  } catch {
    // Keep existing cached conventions
  }
}

function syncConventionsIndicators() {
  const has = Boolean(currentConventions && currentConventions.trim());
  const btn = document.getElementById('conventions-btn');
  const qInput = document.getElementById('quick-conventions-input');
  const advInput = document.getElementById('set-conventions');
  const sid = typeof callbacks.getActiveSessionId === 'function' ? callbacks.getActiveSessionId() : null;
  if (btn) {
    btn.setAttribute('aria-pressed', has ? 'true' : 'false');
    btn.title = has ? `Pinned conventions active: ${currentConventions.slice(0, 120)}` : 'Pin metric signature, units, or notation conventions';
  }
  if (qInput && document.activeElement !== qInput) qInput.value = currentConventions;
  if (advInput && !sid && document.activeElement !== advInput) advInput.value = currentConventions;
}

// --- 4. Resizable Right Canvas + LaTeX View Switcher + Tab Badges ---

function wireCanvasResizerAndViews() {
  const shell = document.getElementById('app-shell');
  const resizer = document.getElementById('canvas-resizer');
  const wideBtn = document.getElementById('canvas-wide-btn');
  const latexPanel = document.getElementById('panel-latex');
  const viewBtns = document.querySelectorAll('[data-latex-mode]');

  const applyWidth = (px) => {
    if (!shell || !Number.isFinite(px)) return;
    const railOpen = shell.getAttribute('data-rail-open') !== 'false';
    const railPx = railOpen ? 264 : 0;
    const maxCanvas = Math.max(460, Math.min(window.innerWidth * 0.56, window.innerWidth - railPx - 460));
    const clamped = Math.round(Math.min(Math.max(px, 380), maxCanvas));
    shell.style.setProperty('--canvas-width', `${clamped}px`);
    try { window.localStorage.setItem(CANVAS_WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };

  try {
    const saved = Number(window.localStorage.getItem(CANVAS_WIDTH_KEY));
    if (saved >= 360) applyWidth(saved);
  } catch { /* ignore */ }

  if (resizer && shell) {
    let dragging = false;
    resizer.addEventListener('pointerdown', (ev) => {
      if (shell.getAttribute('data-canvas-open') === 'false') return;
      dragging = true;
      shell.classList.add('is-resizing');
      resizer.setPointerCapture(ev.pointerId);
    });
    resizer.addEventListener('pointermove', (ev) => {
      if (!dragging) return;
      applyWidth(window.innerWidth - ev.clientX);
    });
    const stopDrag = () => {
      if (!dragging) return;
      dragging = false;
      shell.classList.remove('is-resizing');
    };
    resizer.addEventListener('pointerup', stopDrag);
    resizer.addEventListener('pointercancel', stopDrag);
    resizer.addEventListener('dblclick', () => applyWidth(480));
  }

  if (wideBtn && shell) {
    wideBtn.addEventListener('click', () => {
      const cur = parseInt(getComputedStyle(shell).getPropertyValue('--canvas-width'), 10) || 460;
      const target = cur > 520 ? 460 : 620;
      setDrawerOpen('canvas', true);
      applyWidth(target);
      const actual = parseInt(getComputedStyle(shell).getPropertyValue('--canvas-width'), 10) || target;
      wideBtn.setAttribute('aria-pressed', actual > 520 ? 'true' : 'false');
    });
  }

  for (const btn of viewBtns) {
    btn.addEventListener('click', () => {
      const mode = btn.getAttribute('data-latex-mode') || 'split';
      if (latexPanel) latexPanel.setAttribute('data-latex-view', mode);
      for (const b of viewBtns) {
        const on = b === btn;
        b.classList.toggle('is-active', on);
        b.setAttribute('aria-pressed', on ? 'true' : 'false');
      }
    });
  }
}

export function updateCanvasTabCounts(counts = {}) {
  const mapping = [
    ['tab-dag-count', counts.dag],
    ['tab-sources-count', counts.sources],
    ['tab-sidetalk-count', counts.sidetalk],
  ];
  for (const [id, val] of mapping) {
    const node = document.getElementById(id);
    if (!node) continue;
    const n = Number(val) || 0;
    node.hidden = n <= 0;
    node.textContent = String(n);
  }
}

// --- 5. Connection Status & Keyboard Shortcuts ---

export async function refreshConnectionBadge(refresh = false) {
  try {
    const data = await api.getConnections(refresh);
    const list = Array.isArray(data && data.connections) ? data.connections : [];
    connectedCount = list.filter((c) => c && c.connected).length;
    sympyWorker = data?.capabilities?.sympy_worker || null;
  } catch {
    // Keep prior count
  }
  const btn = document.getElementById('settings-btn');
  if (btn) {
    btn.innerHTML = connectedCount > 0
      ? `<span class="conn-dot ok" aria-hidden="true"></span><span>Settings (${connectedCount})</span>`
      : `<span class="conn-dot warn" aria-hidden="true"></span><span>Settings &amp; Connect</span>`;
  }
  if (typeof callbacks.onConnectionsUpdated === 'function') {
    callbacks.onConnectionsUpdated(connectedCount);
  }
  return connectedCount;
}

export function setDrawerOpen(drawer, open) {
  const shell = document.getElementById('app-shell');
  const pane = document.getElementById(drawer === 'rail' ? 'left-rail' : 'canvas-drawer');
  const toggle = document.getElementById(`${drawer}-toggle`);
  if (open && window.innerWidth <= 960) {
    setDrawerOpen(drawer === 'rail' ? 'canvas' : 'rail', false);
  }
  shell?.setAttribute(`data-${drawer}-open`, String(open));
  toggle?.setAttribute('aria-expanded', String(open));
  if (pane) {
    if (!open && pane.contains(document.activeElement)) toggle?.focus();
    pane.inert = !open;
  }
}

function wireResponsiveDrawers() {
  if (!document.getElementById('app-shell')) return;
  let wasNarrow1120 = null;
  let wasNarrow780 = null;
  const syncForWidth = () => {
    const isNarrow1120 = window.innerWidth <= 1120;
    const isNarrow780 = window.innerWidth <= 780;
    if (isNarrow1120 && wasNarrow1120 !== true) {
      setDrawerOpen('canvas', false);
    }
    if (isNarrow780 && wasNarrow780 !== true) {
      setDrawerOpen('rail', false);
    }
    wasNarrow1120 = isNarrow1120;
    wasNarrow780 = isNarrow780;
  };
  syncForWidth();
  window.addEventListener('resize', syncForWidth);
}

function wireKeyboardShortcuts() {
  document.addEventListener('keydown', (ev) => {
    if (ev.key !== 'Escape' || ev.isComposing || ev.keyCode === 229) return;
    if (document.querySelector('dialog[open], #conventions-popover:not([hidden]), #effort-popover:not([hidden])')) return;
    const shell = document.getElementById('app-shell');
    const drawers = [
      ['rail', 'left-rail', 780],
      ['canvas', 'canvas-drawer', 1120],
    ];
    for (const [drawer, id, maxWidth] of drawers) {
      if (window.innerWidth > maxWidth || shell?.getAttribute(`data-${drawer}-open`) !== 'true') continue;
      if (!document.getElementById(id)?.contains(ev.target)) continue;
      ev.preventDefault();
      ev.stopPropagation();
      document.getElementById(`${drawer}-close-btn`)?.click();
      return;
    }
  }, { capture: true });
  document.addEventListener('keydown', (ev) => {
    const mod = ev.ctrlKey || ev.metaKey;
    if (!mod || ev.defaultPrevented || document.querySelector('dialog[open]')) return;
    if (ev.key.toLowerCase() === 'b' && !ev.shiftKey) {
      ev.preventDefault();
      const t = document.getElementById('rail-toggle');
      if (t) t.click();
    } else if (ev.key === '\\') {
      const t = document.getElementById('canvas-toggle');
      if (t && document.getElementById('app-shell')?.getAttribute('data-project-overview') !== 'true') {
        ev.preventDefault();
        t.click();
      }
    } else if (ev.key.toLowerCase() === 'k') {
      ev.preventDefault();
      const search = document.getElementById('session-search');
      setDrawerOpen('rail', true);
      if (search) search.focus();
    }
  });
}

function notify(msg) {
  if (typeof callbacks.onNotify === 'function') callbacks.onNotify(msg);
}
