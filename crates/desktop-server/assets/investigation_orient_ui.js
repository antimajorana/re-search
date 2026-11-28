// Standalone Investigate V7 Orient UI helpers: RunBar state/actions, Auto-orient toggle,
// selected model propagation, and startup/connect DOM watcher (00-overview §3.3, investigate §3/§6.19).

import { createRunBar, describeRunError, normalizeRunState } from './run_bar.js';

const ORIENT_CAP_MIN = 4;
const ORIENT_CAP_MAX = 24;
const ORIENT_CAP_DEFAULT = 8;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function findLatestOrientTask(mv) {
  const tasks = Array.isArray(mv?.tasks) ? mv.tasks : [];
  let best = null;
  for (const t of tasks) {
    if (t && t.task === 'orient') {
      if (!best || Number(t.request_seq || 0) >= Number(best.request_seq || 0)) best = t;
    }
  }
  return best;
}

export function isOrientTaskRunning(task) {
  const raw = task?.run_state ?? task?.status ?? task?.state;
  if (!raw) return false;
  const norm = normalizeRunState(raw);
  return norm.key === 'running' || norm.key === 'waiting';
}

export function orientReason(latest) {
  if (!latest || typeof latest !== 'object') return '';
  const rawState = latest.run_state ?? latest.status ?? latest.state ?? 'done';
  return describeRunError(latest.reason || latest.error, rawState, latest.model_calls);
}

export function buildTaskPayload(hooks, base) {
  const m = hooks?.getTaskModelOptions?.();
  return m?.provider && m?.model ? { ...base, provider: m.provider, model: m.model } : { ...base };
}

export function buildOrientPayload(hooks) {
  return buildTaskPayload(hooks, { task: 'orient' });
}

export function watchModelConnection(isConnectedFn, onChange) {
  if (typeof document === 'undefined' || typeof document.getElementById !== 'function') return () => {};
  const modelEl = document.getElementById('composer-model');
  const connectEl = document.getElementById('composer-connect');
  let last = Boolean(isConnectedFn?.());
  const check = () => {
    const next = Boolean(isConnectedFn?.());
    if (next !== last) {
      last = next;
      onChange?.(next);
    }
  };
  modelEl?.addEventListener?.('change', check);
  let obs = null;
  if (typeof MutationObserver === 'function' && (modelEl || connectEl)) {
    obs = new MutationObserver(check);
    if (modelEl) obs.observe(modelEl, { childList: true, attributes: true, attributeFilter: ['hidden', 'disabled'] });
    if (connectEl) obs.observe(connectEl, { attributes: true, attributeFilter: ['hidden'] });
  }
  return () => {
    modelEl?.removeEventListener?.('change', check);
    obs?.disconnect?.();
  };
}

export function mountOrientControls({ toolbar, statusHost, orientBtn, apiMod, sessionId, mv, st, hooks, onRefresh }) {
  const isConnected = () => Boolean(hooks?.isModelConnected?.());
  const latest = findLatestOrientTask(mv);
  const running = isOrientTaskRunning(latest);

  const startOrient = (isAuto = false, continueFrom = null) => {
    const base = continueFrom != null
      ? { task: 'orient', continue_from: continueFrom }
      : { task: 'orient' };
    const req = apiMod?.startInvestigationTask?.(sessionId, buildTaskPayload(hooks, base));
    void req?.then(onRefresh)?.catch((err) => {
      if (isAuto && err?.status === 409) {
        onRefresh?.();
        return;
      }
      hooks?.showStatus?.(err?.message || 'Could not start Orient.');
    });
  };

  if (orientBtn) {
    orientBtn.disabled = !isConnected() || running;
    orientBtn.addEventListener('click', () => startOrient(false));
  }

  const maybeAutoOrient = () => {
    const connected = isConnected();
    if (orientBtn) orientBtn.disabled = !connected || running;
    if (!mv?.orient_auto_start || !connected || st?.orientAutoStarted) return;
    if ((mv.tasks || []).some((t) => t.task === 'orient')) return;
    if (st) st.orientAutoStarted = true;
    startOrient(true);
  };

  const renderStatusBar = () => {
    if (!statusHost) return;
    if (!latest) {
      statusHost.hidden = true;
      statusHost.replaceChildren();
      return;
    }
    const rawState = latest.run_state ?? latest.status ?? latest.state ?? 'done';
    const mc = latest.model_calls && typeof latest.model_calls === 'object' ? latest.model_calls : null;
    const callsUsed = mc ? mc.used : (latest.model_calls_used ?? latest.model_calls);
    const callsMax = mc ? mc.cap : (latest.cap ?? latest.model_calls_max);
    const continueCap = Number(mv?.orient_call_cap) || Number(callsMax) || ORIENT_CAP_DEFAULT;
    const unresolvedRejected = Math.floor(Number(latest.unresolved_rejected_items) || 0);
    const extras = unresolvedRejected > 0
      ? [`${unresolvedRejected} section(s) not saved (quote outside section)`]
      : [];
    const detail = latest.detail || latest.focus || (
      typeof latest.saved_items === 'number' && latest.saved_items > 0
        ? `Orient (${latest.saved_items} readings)`
        : 'Orient'
    );
    statusHost.hidden = false;
    statusHost.replaceChildren();
    createRunBar(statusHost, {
      state: rawState,
      detail,
      reason: orientReason(latest),
      extras,
      elapsedMs: latest.elapsed_ms,
      modelCallsUsed: callsUsed,
      modelCallsMax: callsMax,
      openItems: Array.isArray(latest.open_items) ? latest.open_items : [],
    }, {
      continueLabel: `Continue (up to ${continueCap} more model calls)`,
      onStop: () => {
        void apiMod?.stopTurn?.(sessionId)?.then?.(onRefresh)
          ?.catch?.((err) => hooks?.showStatus?.(err?.message || 'Could not stop Orient.'));
      },
      onContinue: () => startOrient(false, latest.request_seq != null ? latest.request_seq : null),
      onRetry: () => startOrient(false),
    });
  };

  if (toolbar && typeof apiMod?.putInvestigateSettings === 'function') {
    const label = el('label', 'inv-v7-auto-orient-toggle');
    const box = el('input');
    box.type = 'checkbox';
    box.checked = Boolean(mv?.orient_auto_start);
    box.setAttribute('data-setting', 'orient_auto_start');
    box.addEventListener('change', async () => {
      const next = Boolean(box.checked);
      box.disabled = true;
      try {
        await apiMod.putInvestigateSettings({ orient_auto_start: next });
        if (mv && typeof mv === 'object') mv.orient_auto_start = next;
        if (next) maybeAutoOrient();
      } catch (err) {
        box.checked = !next;
        hooks?.showStatus?.(err?.message || 'Could not update Auto-orient setting.');
      } finally {
        box.disabled = false;
      }
    });
    label.append(box, el('span', '', 'Auto-orient on open'));
    toolbar.appendChild(label);

    const rawCap = Number(mv?.orient_call_cap);
    let savedCap = Number.isInteger(rawCap) && rawCap >= ORIENT_CAP_MIN && rawCap <= ORIENT_CAP_MAX
      ? rawCap
      : ORIENT_CAP_DEFAULT;
    const capLabel = el('label', 'inv-v7-orient-cap-setting');
    const capInput = el('input', 'inv-v7-orient-cap-input');
    capInput.type = 'number';
    capInput.min = String(ORIENT_CAP_MIN);
    capInput.max = String(ORIENT_CAP_MAX);
    capInput.setAttribute('min', String(ORIENT_CAP_MIN));
    capInput.setAttribute('max', String(ORIENT_CAP_MAX));
    capInput.setAttribute('data-setting', 'orient_call_cap');
    capInput.setAttribute('aria-label', 'Orient call cap');
    capInput.value = String(savedCap);
    capInput.addEventListener('change', async () => {
      const rawVal = String(capInput.value ?? '').trim();
      const next = Number(rawVal);
      if (!rawVal || !Number.isInteger(next) || next < ORIENT_CAP_MIN || next > ORIENT_CAP_MAX) {
        capInput.value = String(savedCap);
        hooks?.showStatus?.(`Orient call cap must be a whole number from ${ORIENT_CAP_MIN} to ${ORIENT_CAP_MAX}.`);
        return;
      }
      capInput.disabled = true;
      try {
        await apiMod.putInvestigateSettings({ orient_call_cap: next });
        savedCap = next;
        capInput.value = String(savedCap);
        if (mv && typeof mv === 'object') mv.orient_call_cap = savedCap;
        renderStatusBar();
      } catch (err) {
        capInput.value = String(savedCap);
        hooks?.showStatus?.(err?.message || 'Could not update Orient call cap.');
      } finally {
        capInput.disabled = false;
      }
    });
    capLabel.append(el('span', '', 'Orient cap'), capInput);
    toolbar.appendChild(capLabel);
  }

  renderStatusBar();

  maybeAutoOrient();
  const stopWatch = watchModelConnection(isConnected, maybeAutoOrient);
  return { running, stopWatch };
}
