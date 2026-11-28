import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  TOKENS_NOT_REPORTED,
  createUsageTrackerController,
  formatCallCount,
  formatCompactTokens,
  formatSessionUsageLine,
  formatTokenSummary,
  renderSessionUsageLine,
  renderUsagePanel,
} from '../assets/usage_tracker.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ASSETS_DIR = path.resolve(__dirname, '../assets');

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      id: '',
      type: '',
      hidden: false,
      children: [],
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
      setAttribute(k, v) { attrs.set(String(k), String(v)); if (k === 'class') this.className = v; if (k === 'id') this.id = String(v); },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) { (listeners.has(type) ? listeners : listeners.set(type, [])).get(type).push(fn); },
      async click() {
        for (const fn of listeners.get('click') || []) await fn({ preventDefault() {}, target: this });
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('#')) return n.id === sel.slice(1);
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
  globalThis.document = { createElement: (tag) => makeEl(tag) };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function collectTags(node, out = []) {
  if (node?.tagName) out.push(node.tagName);
  for (const child of node?.children || []) collectTags(child, out);
  return out;
}

test('formatters handle compact tokens, calls, and unreported CLI routes honestly', () => {
  assert.equal(formatCompactTokens(0), '0');
  assert.equal(formatCompactTokens(850), '850');
  assert.equal(formatCompactTokens(36000), '36.0k');
  assert.equal(formatCompactTokens(2400), '2.4k');
  assert.equal(formatCompactTokens(999_950), '1.00M');
  assert.equal(formatCompactTokens(1_250_000), '1.25M');
  assert.equal(formatCallCount(1), '1 call');
  assert.equal(formatCallCount(8), '8 calls');

  assert.equal(
    formatSessionUsageLine({ calls: 8, input_tokens: 36000, output_tokens: 2400, tokens_reported: true, unreported_calls: 0 }),
    '8 calls · 36.0k in / 2.4k out',
  );
  assert.equal(
    formatSessionUsageLine({ calls: 3, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 3 }),
    `3 calls · ${TOKENS_NOT_REPORTED}`,
  );
  assert.equal(
    formatSessionUsageLine({ calls: 0, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 0 }),
    '',
  );
  assert.equal(
    formatTokenSummary({ calls: 2, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 2 }),
    TOKENS_NOT_REPORTED,
  );
});

test('renderUsagePanel renders all sections, tokens not reported, zero cost estimates, and zero innerHTML/XSS', () => {
  const dom = makeFakeDom();
  try {
    const panel = dom.makeEl('div');
    const xss = '<img src=x onerror=alert(1)><script>evil()</script>';
    let refreshed = 0;

    renderUsagePanel(panel, {
      totals: {
        today: { calls: 5, input_tokens: 12000, output_tokens: 2400, unreported_calls: 2 },
        last_7_days: { calls: 9, input_tokens: 36000, output_tokens: 5100, unreported_calls: 3 },
        all_time: { calls: 12, input_tokens: 50000, output_tokens: 8000, unreported_calls: 3 },
      },
      by_provider_model: [
        { provider: 'anthropic', model: 'claude-sonnet-4-6', calls: 9, input_tokens: 50000, output_tokens: 8000, tokens_reported: true, unreported_calls: 0 },
        { provider: 'claude-cli', model: xss, calls: 3, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 3 },
      ],
      by_day: [
        { date: '2026-11-28', calls: 5, input_tokens: 12000, output_tokens: 2400, unreported_calls: 2 },
        { date: '2026-11-25', calls: 3, input_tokens: 0, output_tokens: 0, unreported_calls: 3 },
      ],
      by_session: [
        { session_id: 's-1', title: `Hostile ${xss}`, mode: 'solve', updated_at_ms: 200, calls: 9, input_tokens: 50000, output_tokens: 8000, tokens_reported: true, unreported_calls: 0 },
        { session_id: 's-cli', title: 'Subscription Run', mode: 'research', updated_at_ms: 100, calls: 3, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 3 },
      ],
    }, { onRefresh: () => { refreshed += 1; } });

    assert.equal(dom.innerHtmlWrites, 0, 'usage_tracker.js must never write innerHTML');
    const fullText = panel.textContent;
    assert.match(fullText, /Today/);
    assert.match(fullText, /Last 7 days/);
    assert.match(fullText, /All time/);
    assert.match(fullText, /2 calls without full token counts/);
    assert.match(fullText, /By provider & model/);
    assert.match(fullText, /By day/);
    assert.match(fullText, /By session/);
    assert.match(fullText, /tokens not reported/);
    assert.match(fullText, /50\.0k in \/ 8\.0k out/);
    assert.doesNotMatch(fullText, /\$|USD|\bcost\b|\bprice\b/i, 'UI must contain zero cost estimates');

    // Hostile XSS strings stay literal textContent; zero injected IMG or SCRIPT tags.
    const tags = collectTags(panel);
    assert.ok(!tags.includes('IMG') && !tags.includes('SCRIPT'), `unexpected injected tags: ${tags}`);
    assert.ok(fullText.includes(xss), 'hostile text should render literally as escaped textContent');

    const refreshBtn = panel.querySelector('#usage-refresh-btn');
    assert.ok(refreshBtn, 'expected #usage-refresh-btn');
    refreshBtn.click();
    assert.equal(refreshed, 1);
  } finally {
    dom.restore();
  }
});

test('controller loads Usage tab, deduplicates unchanged rail refreshes, clears stale line on switch, and race-guards', async () => {
  const dom = makeFakeDom();
  try {
    const panelEl = dom.makeEl('div');
    const sessionLineEl = dom.makeEl('span');
    sessionLineEl.hidden = true;

    let resolveSlow;
    const slowPromise = new Promise((r) => { resolveSlow = r; });
    const calls = [];
    const ctrl = createUsageTrackerController({
      panelEl,
      sessionLineEl,
      fetchUsage: (sid) => {
        calls.push(sid);
        if (sid === null) {
          return Promise.resolve({
            totals: {
              today: { calls: 1, input_tokens: 1000, output_tokens: 200, unreported_calls: 0 },
              last_7_days: { calls: 1, input_tokens: 1000, output_tokens: 200, unreported_calls: 0 },
              all_time: { calls: 1, input_tokens: 1000, output_tokens: 200, unreported_calls: 0 },
            },
            by_provider_model: [],
            by_day: [],
            by_session: [],
          });
        }
        if (sid === 's-slow') return slowPromise;
        if (sid === 's-cli') {
          return Promise.resolve({
            session: { calls: 3, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 3 },
          });
        }
        return Promise.resolve({
          session: { calls: 0, input_tokens: 0, output_tokens: 0, tokens_reported: false, unreported_calls: 0 },
        });
      },
    });

    await ctrl.onSettingsTab('usage');
    assert.equal(calls[0], null);
    assert.match(panelEl.textContent, /1\.0k in \/ 200 out/);

    // Race test: slow request starts first, then s-cli resolves before slow finishes.
    const pSlow = ctrl.syncSession('s-slow', [{ id: 's-slow', event_count: 1, updated_ms: 10, running: false }]);
    await ctrl.syncSession('s-cli', [{ id: 's-cli', event_count: 3, updated_ms: 20, running: false }]);
    assert.equal(sessionLineEl.hidden, false);
    assert.equal(sessionLineEl.textContent, '3 calls · tokens not reported');

    resolveSlow({ session: { calls: 99, input_tokens: 99000, output_tokens: 9000, tokens_reported: true, unreported_calls: 0 } });
    await pSlow;
    assert.equal(sessionLineEl.textContent, '3 calls · tokens not reported', 'stale response must not clobber active session');

    // Unchanged rail re-render (e.g. typing in search filter) skips redundant fetch.
    const countBefore = calls.length;
    await ctrl.syncSession('s-cli', [{ id: 's-cli', event_count: 3, updated_ms: 20, running: false }]);
    assert.equal(calls.length, countBefore, 'unchanged active session must not re-fetch');

    // Updated event_count triggers fresh fetch.
    await ctrl.syncSession('s-cli', [{ id: 's-cli', event_count: 4, updated_ms: 30, running: false }]);
    assert.equal(calls.length, countBefore + 1);

    // Transient failure must not poison dedup cache; next refresh retries cleanly.
    let failOnce = true;
    const retryCtrl = createUsageTrackerController({
      panelEl,
      sessionLineEl,
      fetchUsage: () => {
        if (failOnce) {
          failOnce = false;
          return Promise.reject(new Error('transient'));
        }
        return Promise.resolve({
          session: { calls: 2, input_tokens: 1500, output_tokens: 300, tokens_reported: true, unreported_calls: 0 },
        });
      },
    });
    const metaRow = [{ id: 's-retry', event_count: 2, updated_ms: 50, running: false }];
    await retryCtrl.syncSession('s-retry', metaRow);
    assert.equal(sessionLineEl.hidden, true);
    await retryCtrl.syncSession('s-retry', metaRow);
    assert.equal(sessionLineEl.hidden, false);
    assert.equal(sessionLineEl.textContent, '2 calls · 1.5k in / 300 out');

    // Switching to another session immediately hides stale usage line while fetch is in flight.
    const pEmpty = ctrl.syncSession('s-empty', [{ id: 's-empty', event_count: 0, updated_ms: 40, running: false }]);
    assert.equal(sessionLineEl.hidden, true, 'must clear stale session line immediately on switch');
    await pEmpty;
    assert.equal(sessionLineEl.hidden, true);
    assert.equal(sessionLineEl.textContent, '');

    // Direct renderSessionUsageLine helper check.
    renderSessionUsageLine(sessionLineEl, { calls: 8, input_tokens: 36000, output_tokens: 2400, tokens_reported: true, unreported_calls: 0 });
    assert.equal(sessionLineEl.hidden, false);
    assert.equal(sessionLineEl.textContent, '8 calls · 36.0k in / 2.4k out');
    assert.equal(dom.innerHtmlWrites, 0);
  } finally {
    dom.restore();
  }
});

test('index.html wires Settings Usage tab, session usage line, and usage_tracker.js without innerHTML in module', () => {
  const html = fs.readFileSync(path.join(ASSETS_DIR, 'index.html'), 'utf8');
  const js = fs.readFileSync(path.join(ASSETS_DIR, 'usage_tracker.js'), 'utf8');
  assert.match(html, /id="settings-tab-usage"[^>]*data-settings-tab="usage"/);
  assert.match(html, /id="settings-panel-usage"[^>]*data-settings-panel="usage"[^>]*tabindex="0"/);
  assert.match(html, /id="session-usage-line"/);
  assert.match(html, /<script type="module" src="\/assets\/usage_tracker\.js"><\/script>/);
  assert.doesNotMatch(js, /\.innerHTML\s*=/);
});
