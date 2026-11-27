import assert from 'node:assert/strict';
import test from 'node:test';

import {
  UNKNOWN_TOKENS_LABEL,
  WORKER_SUMMARY_UNVERIFIED_LABEL,
  createWorkerActivityInspector,
  formatTokenCount,
  formatUsageSummary,
  mountWorkerActivityButton,
} from '../assets/worker_activity.js';

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div', ns = null) => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      namespaceURI: ns,
      children: [],
      style: {},
      type: '',
      value: '',
      placeholder: '',
      title: '',
      hidden: false,
      disabled: false,
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      attributes: attrs,
      setAttribute(k, v) { attrs.set(String(k), String(v)); if (k === 'class') this.className = v; },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      async dispatchAsync(type, event = {}) {
        const ev = { preventDefault() {}, target: this, ...event };
        for (const fn of listeners.get(type) || []) await fn(ev);
      },
      click() { return this.dispatchAsync('click'); },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const attr = sel.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (attr) {
            const val = n.getAttribute(attr[1]);
            return attr[2] === undefined ? val !== null : val === attr[2];
          }
          return n.tagName === sel.toUpperCase();
        };
        const visit = (n) => {
          for (const c of n.children || []) {
            if (match(c)) out.push(c);
            visit(c);
          }
        };
        visit(this);
        return out;
      },
    };
    return el;
  };
  const prevDoc = globalThis.document;
  globalThis.document = {
    createElement: (tag) => makeEl(tag, null),
    createElementNS: (ns, tag) => makeEl(tag, ns),
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function walk(node, fn) {
  fn(node);
  for (const child of node.children || []) walk(child, fn);
}

test('formatTokenCount and formatUsageSummary distinguish unknown from genuine 0', () => {
  assert.equal(formatTokenCount(null), UNKNOWN_TOKENS_LABEL);
  assert.equal(formatTokenCount(undefined), UNKNOWN_TOKENS_LABEL);
  assert.equal(formatTokenCount(0), '0');
  assert.equal(formatTokenCount(42), '42');

  assert.match(formatUsageSummary({ calls: 2, input_tokens: null, output_tokens: null, unknown_usage_calls: 2 }), /unknown/);
  assert.doesNotMatch(formatUsageSummary({ calls: 2, input_tokens: null, output_tokens: null, unknown_usage_calls: 2 }), /\b0 in/);
  assert.equal(formatUsageSummary({ calls: 1, input_tokens: 0, output_tokens: 0, unknown_usage_calls: 0 }), '0 in / 0 out');
  assert.match(formatUsageSummary({ calls: 3, input_tokens: 120, output_tokens: null, unknown_usage_calls: 2 }), /120 in \/ unknown out \+ 2 calls without usage/);
});

test('worker_activity renders lazily, exposes missing artifacts & controls, and has zero innerHTML/XSS', async () => {
  const dom = makeFakeDom();
  try {
    const calls = [];
    const xss = '<img src=x onerror=alert(1)><script>evil()</script>';
    const listPayload = {
      session: 's-1',
      roles: {
        paper_reader: { status: 'pinned', provider: 'anthropic', model: 'claude-haiku-4-5', routing_reason: 'auto_smaller_tier' },
        compute_blind: { status: 'unavailable', provider: 'openai', model: 'gpt-4.1-mini', error: `missing key ${xss}` },
      },
      usage_rollup: {
        roles: [
          { role: 'paper_reader', provider: 'anthropic', used_model: 'claude-haiku-4-5', calls: 2, input_tokens: null, output_tokens: null, unknown_usage_calls: 2 },
        ],
        totals: { calls: 2, input_tokens: null, output_tokens: null, unknown_usage_calls: 2 },
      },
      controls: { focus_notes: [], pinned_items: [], dismissed_items: [], pause_after_stage: false },
      jobs: [
        {
          job_id: 'call-1',
          role: 'paper_reader',
          provider: 'anthropic',
          used_model: 'claude-haiku-4-5',
          calls_used: 2,
          usage: { calls: 2, input_tokens: null, output_tokens: null, unknown_usage_calls: 2 },
          disposition: 'partial',
          disposition_reason: `partial ${xss}`,
          artifact_refs: [{ kind: 'blob', id: 'out', blob: 'deadbeef', missing: true }],
        },
      ],
    };
    const detailPayload = {
      job: listPayload.jobs[0],
      artifact_missing: true,
      worker_summary: { text: `Summary ${xss}`, missing: false },
      context_supplied: [{ source_id: 'src-1', path: 'src-1/main.tex', start_line: 10, end_line: 25 }],
      tool_receipts: [{ call_id: 't1', tool: 'read_paper_lines', ok: true, artifact_missing: false }],
      returned_items: [{ id: 'ev-5', quote: `Quote ${xss}`, pinned: false, dismissed: false }],
      request_blobs: [{ blob: 'abcd1234', missing: true }],
      response_blobs: [],
    };

    const fetchJson = async (method, url, body) => {
      calls.push({ method, url, body });
      if (method === 'GET' && url === '/api/sessions/s-1/workers') return listPayload;
      if (method === 'GET' && url === '/api/sessions/s-1/workers/call-1') return detailPayload;
      if (method === 'POST' && url === '/api/sessions/s-1/run-control') {
        return {
          ok: true,
          controls: {
            focus_notes: body.action === 'focus_note' ? [{ seq: 1, note: body.note }] : [],
            pinned_items: body.action === 'pin_worker_item' ? [body.item_id] : [],
            dismissed_items: [],
            pause_after_stage: body.action === 'pause_after_stage' ? body.enabled : false,
          },
        };
      }
      throw new Error(`unexpected ${method} ${url}`);
    };

    const host = dom.makeEl('div');
    const mounted = mountWorkerActivityButton(host, { sessionId: 's-1', fetchJson });
    assert.equal(calls.length, 0, 'mounting button makes 0 network calls until clicked');

    await mounted.button.click();
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, '/api/sessions/s-1/workers');
    assert.match(mounted.drawer.textContent, /unknown/);
    assert.match(mounted.drawer.textContent, /Missing artifact/);

    // Lazy detail fetch only when Inspect is clicked.
    const inspectBtn = mounted.drawer.querySelector('[data-inspect-job="call-1"]');
    await inspectBtn.click();
    assert.equal(calls.length, 2);
    assert.equal(calls[1].url, '/api/sessions/s-1/workers/call-1');
    assert.ok(mounted.drawer.textContent.includes(WORKER_SUMMARY_UNVERIFIED_LABEL));
    assert.match(mounted.drawer.textContent, /Artifact missing/);
    assert.match(mounted.drawer.textContent, /ev-5/);

    // Reader controls submit run.control payloads.
    await mounted.drawer.querySelector('[data-action="pin"]').click();
    assert.equal(calls[2].method, 'POST');
    assert.equal(calls[2].body.action, 'pin_worker_item');
    assert.equal(calls[2].body.item_id, 'ev-5');

    await mounted.drawer.querySelector('[data-control="pause_after_stage"]').click();
    assert.equal(calls[3].body.action, 'pause_after_stage');
    assert.equal(calls[3].body.enabled, true);

    const focusInput = mounted.drawer.querySelector('.wa-focus-input');
    focusInput.value = 'Focus on Section 3 conventions';
    await mounted.drawer.querySelector('[data-control="focus_note"]').click();
    assert.equal(calls[4].body.action, 'focus_note');

    assert.equal(dom.innerHtmlWrites, 0, 'worker_activity.js must never write innerHTML');
    const forbidden = new Set(['IMG', 'SCRIPT', 'IFRAME', 'A', 'OBJECT', 'EMBED']);
    walk(host, (node) => {
      assert.equal(forbidden.has(node.tagName), false, `unexpected injected tag <${node.tagName}>`);
      for (const [k, v] of node.attributes.entries()) {
        assert.equal(/^on/i.test(k), false, `unexpected event attr ${k}`);
        assert.equal(/^\s*javascript:/i.test(v), false, `unexpected javascript URL in ${k}`);
      }
    });
  } finally {
    dom.restore();
  }
});
