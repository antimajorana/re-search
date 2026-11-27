// Standalone Investigate V7 Orient UI helpers: RunBar state/actions, Auto-orient toggle,
// selected model propagation, and startup/connect DOM watcher (00-overview §3.3, investigate §3/§6.19).

import { createRunBar, normalizeRunState } from './run_bar.js';

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

  const startOrient = (isAuto = false) => {
    const req = apiMod?.startInvestigationTask?.(sessionId, buildOrientPayload(hooks));
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
  }

  if (statusHost) {
    if (!latest) {
      statusHost.hidden = true;
      statusHost.replaceChildren();
    } else {
      const rawState = latest.run_state ?? latest.status ?? latest.state ?? 'done';
      const mc = latest.model_calls && typeof latest.model_calls === 'object' ? latest.model_calls : null;
      const callsUsed = mc ? mc.used : (latest.model_calls_used ?? latest.model_calls);
      const callsMax = mc ? mc.cap : (latest.cap ?? latest.model_calls_max);
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
        reason: latest.reason || latest.error || '',
        elapsedMs: latest.elapsed_ms,
        modelCallsUsed: callsUsed,
        modelCallsMax: callsMax,
        openItems: Array.isArray(latest.open_items) ? latest.open_items : [],
      }, {
        onStop: () => {
          void apiMod?.stopTurn?.(sessionId)?.then?.(onRefresh)
            ?.catch?.((err) => hooks?.showStatus?.(err?.message || 'Could not stop Orient.'));
        },
        onContinue: () => startOrient(false),
        onRetry: () => startOrient(false),
      });
    }
  }

  maybeAutoOrient();
  const stopWatch = watchModelConnection(isConnected, maybeAutoOrient);
  return { running, stopWatch };
}
