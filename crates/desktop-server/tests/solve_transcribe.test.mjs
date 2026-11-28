// Solve S17b Node tests: TeX-only check-a-derivation intake UI (`solve_transcribe.js`).
// Covers: PDF rejection (00-overview.md 2.12), two-column "your TeX" vs "as checked" (R5),
// hash-bound "transcription unconfirmed" flag cleared ONLY on matching `entry_sha256`,
// [Confirm] & [Confirm all] (D13), planted error highlight, R22 wording, and XSS safety.

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  PDF_REJECT_MESSAGE,
  SOLVE_TRANSCRIBE_TAB,
  TRANSCRIPTION_UNCONFIRMED_TEXT,
  createSolveTranscribeView,
  isPdfInput,
  isTranscriptionConfirmed,
  registerSolveTranscribeTab,
} from '../assets/solve_transcribe.js';
import { createSolveWorkspace, unregisterSolveTab } from '../assets/solve_steps.js';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

function makeFakeDom() {
  let innerHtmlWrites = 0;
  function makeEl(tag) {
    const attrs = new Map();
    const listeners = new Map();
    const classes = new Set();
    let textVal = '';
    const el = {
      tagName: String(tag).toUpperCase(),
      nodeType: 1,
      children: [],
      parentNode: null,
      disabled: false,
      hidden: false,
      checked: false,
      value: '',
      files: [],
      type: '',
      get className() { return Array.from(classes).join(' '); },
      set className(v) {
        classes.clear();
        for (const p of String(v || '').split(/\s+/).filter(Boolean)) classes.add(p);
      },
      classList: {
        add(...xs) { for (const x of xs) if (x) classes.add(x); },
        remove(...xs) { for (const x of xs) classes.delete(x); },
        contains(x) { return classes.has(x); },
      },
      get textContent() {
        return el.children.length === 0 ? textVal : el.children.map((c) => c.textContent).join('');
      },
      set textContent(v) { el.children = []; textVal = v == null ? '' : String(v); },
      get innerHTML() { return ''; },
      set innerHTML(_v) { innerHtmlWrites += 1; },
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'type') el.type = String(v); },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      getAttributes() { return attrs; },
      appendChild(child) {
        if (!child) return child;
        if (child.parentNode) child.parentNode.removeChild(child);
        child.parentNode = el;
        el.children.push(child);
        return child;
      },
      append(...nodes) { for (const n of nodes) el.appendChild(n); },
      removeChild(child) {
        const idx = el.children.indexOf(child);
        if (idx >= 0) { el.children.splice(idx, 1); child.parentNode = null; }
        return child;
      },
      replaceChildren(...nodes) {
        for (const c of el.children) c.parentNode = null;
        el.children = [];
        textVal = '';
        for (const n of nodes) el.appendChild(n);
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      async dispatch(type, extra = {}) {
        const ev = { type, target: el, preventDefault() {}, stopPropagation() {}, ...extra };
        for (const fn of [...(listeners.get(type) || [])]) await fn(ev);
      },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          const mAttr = sel.match(/^\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (mAttr) {
            const val = n.getAttribute(mAttr[1]);
            return mAttr[2] === undefined ? val !== null : val === mAttr[2];
          }
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          return n.tagName === sel.toUpperCase();
        };
        const walk = (n) => { for (const c of n.children) { if (match(c)) out.push(c); walk(c); } };
        walk(el);
        return out;
      },
      querySelector(sel) { return el.querySelectorAll(sel)[0] || null; },
    };
    return el;
  }
  const fakeDoc = {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    createTextNode: (t) => ({ nodeType: 3, textContent: String(t), children: [], parentNode: null, getAttributes: () => new Map() }),
  };
  const prevDoc = globalThis.document;
  globalThis.document = fakeDoc;
  return {
    doc: fakeDoc,
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

function makeFixtureView() {
  return {
    handles: [
      { name: 'lhs_1', version: 1, latex: 'E_0', str_preview: 'E0', portable: true },
      { name: 'rhs_1', version: 1, latex: '\\frac{1}{2}\\hbar\\omega', str_preview: 'hbar*omega/2', portable: true },
      { name: 'lhs_2', version: 1, latex: 'E_1', str_preview: 'E1', portable: true },
      { name: 'rhs_2', version: 1, latex: '\\hbar\\omega', str_preview: 'hbar*omega', portable: true },
    ],
    steps: [
      {
        entry_id: 'e-1',
        entry_type: 'derivation',
        origin: 'given',
        summary: 'Equation 1 (lines 4-6)',
        details: 'E_0 = \\frac{1}{2} \\hbar \\omega',
        latex: 'IGNORED_MODEL_LATEX_R5',
        depends_on: [],
        entry_sha256: 'sha-e1-current',
        transcription: { source_tex_sha256: 'tex-sha-1' },
        claims: [{ form: 'equation', lhs: 'lhs_1@1', rhs: 'rhs_1@1', relation: 'eq' }],
        badge: {
          own: 'verified', effective: 'verified', label: 'Verified',
          flags: ['transcription unconfirmed'], sublabels: ['symbolic'], stale: false,
        },
      },
      {
        entry_id: 'e-2',
        entry_type: 'derivation',
        origin: 'given',
        summary: 'Equation 2 (lines 7-9)',
        details: 'E_1 = \\hbar \\omega',
        depends_on: ['e-1'],
        entry_sha256: 'sha-e2-current',
        transcription: { source_tex_sha256: 'tex-sha-2' },
        claims: [{ form: 'equation', lhs: 'lhs_2@1', rhs: 'rhs_2@1', relation: 'eq' }],
        badge: {
          own: 'failed', effective: 'failed', label: 'Failed',
          flags: ['transcription unconfirmed'], sublabels: [], stale: false,
        },
      },
    ],
  };
}

test('S17b PDF rejection, side-by-side your TeX vs as checked (R5), and planted error highlight', async () => {
  const dom = makeFakeDom();
  try {
    assert.equal(isPdfInput('paper.PDF', ''), true);
    assert.equal(isPdfInput('deriv.tex', '  %PDF-1.7\n'), true);
    assert.equal(isPdfInput('deriv.tex', '\\begin{equation}x=1\\end{equation}'), false);

    const view = makeFixtureView();
    const host = dom.makeEl('div');
    const calls = [];
    const api = createSolveTranscribeView(host, view, {
      katex: null,
      onTranscribe: async (payload) => { calls.push(payload); return { ok: true, equation_count: 2, view }; },
    });

    // 1. Two-column rendering: left "your TeX" shows raw user TeX; right "as checked" uses solve.expr handles ONLY (R5).
    const cards = host.querySelectorAll('[data-testid="solve-transcribe-card"]');
    assert.equal(cards.length, 2);
    const left1 = cards[0].querySelector('[data-col="your-tex"]');
    const right1 = cards[0].querySelector('[data-col="as-checked"]');
    assert.ok(left1.textContent.includes('your TeX'));
    assert.ok(left1.textContent.includes('E_0 = \\frac{1}{2} \\hbar \\omega'));
    assert.ok(right1.textContent.includes('as checked'));
    assert.ok(right1.textContent.includes('\\frac{1}{2}\\hbar\\omega'));
    assert.ok(!right1.textContent.includes('IGNORED_MODEL_LATEX_R5'), 'R5: must ignore step.latex');

    // 2. Planted error step e-2 highlighted as failed on card and consecutive pair strip.
    assert.ok(cards[1].classList.contains('solve-transcribe-card--failed'));
    const failedPair = host.querySelector('[data-pair-to="e-2"]');
    assert.ok(failedPair);
    assert.equal(failedPair.getAttribute('data-failed'), 'true');
    assert.ok(failedPair.textContent.includes('e-1 → e-2'));

    // 3. Submitting a PDF file or %PDF- content is rejected client-side without calling backend.
    const form = host.querySelector('[data-testid="solve-transcribe-form"]');
    form.querySelector('[data-testid="solve-transcribe-filename"]').value = 'notes.pdf';
    form.querySelector('[data-testid="solve-transcribe-tex"]').value = '\\begin{equation}x=1\\end{equation}';
    await form.dispatch('submit');
    assert.equal(calls.length, 0);
    assert.equal(host.querySelector('[data-testid="solve-transcribe-error"]')?.textContent, PDF_REJECT_MESSAGE);

    // 4. Valid TeX submission reaches onTranscribe.
    form.querySelector('[data-testid="solve-transcribe-filename"]').value = 'osc.tex';
    form.querySelector('[data-testid="solve-transcribe-tex"]').value = '\\begin{equation}x=1\\end{equation}';
    await form.dispatch('submit');
    assert.equal(calls.length, 1);
    assert.equal(calls[0].file_name, 'osc.tex');
    api.destroy();
  } finally {
    dom.restore();
  }
});

test('S17b transcription unconfirmed cleared ONLY on matching entry_sha256, [Confirm] & [Confirm all] (D13)', async () => {
  const dom = makeFakeDom();
  try {
    const view = makeFixtureView();
    const step1 = view.steps[0];

    // Mismatched hash MUST NOT confirm step1!
    assert.equal(isTranscriptionConfirmed(step1, []), false);
    assert.equal(isTranscriptionConfirmed(step1, [{ entry_id: 'e-1', kind: 'transcription', entry_sha256: 'wrong-sha' }]), false);
    assert.equal(isTranscriptionConfirmed(step1, [{ entry_id: 'e-1', kind: 'hand_checked', entry_sha256: 'sha-e1-current' }]), false);
    assert.equal(isTranscriptionConfirmed(step1, [{ entry_id: 'e-1', kind: 'transcription', entry_sha256: 'sha-e1-current' }]), true);

    // If step is later edited (entry_sha256 changes), prior ack no longer confirms it!
    const editedStep = { ...step1, entry_sha256: 'sha-e1-edited', badge: { ...step1.badge, flags: [] } };
    assert.equal(
      isTranscriptionConfirmed(editedStep, [{ entry_id: 'e-1', kind: 'transcription', entry_sha256: 'sha-e1-current' }]),
      false,
    );

    // Clicking [Confirm] on e-1 sends exact entry_sha256; rejected ack leaves flag intact.
    const host = dom.makeEl('div');
    let failFirst = true;
    const acked = [];
    const api = createSolveTranscribeView(host, view, {
      katex: null,
      onAck: async (_step, payload) => {
        if (failFirst) { failFirst = false; throw new Error('entry_sha256 mismatch (409)'); }
        acked.push(payload);
        return { ok: true, ack: payload };
      },
    });

    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-unconfirmed"]').length, 2);
    assert.ok(host.textContent.includes(TRANSCRIPTION_UNCONFIRMED_TEXT));

    await host.querySelector('[data-action="confirm-transcription"]').dispatch('click');
    assert.equal(acked.length, 0);
    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-unconfirmed"]').length, 2);
    assert.ok(host.querySelector('[data-testid="solve-transcribe-error"]')?.textContent.includes('409'));

    // Now [Confirm all] confirms both e-1 and e-2 with exact entry_sha256 values.
    await host.querySelector('[data-testid="solve-confirm-all"]').dispatch('click');
    assert.equal(acked.length, 2);
    assert.deepEqual(acked[0], { entry_id: 'e-1', kind: 'transcription', entry_sha256: 'sha-e1-current' });
    assert.deepEqual(acked[1], { entry_id: 'e-2', kind: 'transcription', entry_sha256: 'sha-e2-current' });
    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-unconfirmed"]').length, 0);
    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-confirmed"]').length, 2);
    assert.equal(host.querySelector('[data-testid="solve-confirm-all"]').disabled, true);
    api.destroy();
  } finally {
    dom.restore();
  }
});

test('S17b registerSolveTab integration, R22 zero-verified sweep, and zero-innerHTML XSS audit', () => {
  const dom = makeFakeDom();
  try {
    // 1. Plug into solve_steps.js tab registry via registerSolveTranscribeTab.
    registerSolveTranscribeTab();
    const wsHost = dom.makeEl('div');
    const uncheckedView = {
      handles: [],
      steps: [{
        entry_id: 'e-1', entry_type: 'derivation', origin: 'given',
        summary: 'Equation 1 (line 2)', details: 'x = 1', depends_on: [],
        entry_sha256: 'sha-u1', transcription: { source_tex_sha256: 't1' }, claims: [],
        badge: { own: 'unchecked', effective: 'unchecked', label: 'Unchecked', flags: ['transcription unconfirmed'], stale: false },
      }],
      graph: { nodes: [], edges: [], legend: [] },
    };
    const ws = createSolveWorkspace(wsHost, uncheckedView, { katex: null, initialTab: SOLVE_TRANSCRIBE_TAB.id });
    assert.ok(ws.getTabs().some((t) => t.id === 'transcribe'));
    assert.ok(wsHost.querySelector('[data-testid="solve-transcribe-view"]'));
    // R22: zero verified steps => "verified", "proved", "novel" must NEVER appear.
    assert.ok(!/\b(verified|proved|novel)\b/i.test(wsHost.textContent));
    ws.destroy();
    unregisterSolveTab(SOLVE_TRANSCRIBE_TAB.id);

    // 2. XSS payloads produce zero innerHTML writes and zero injected tags/handlers.
    const xss = '<img src=x onerror=alert(1)><script>alert(2)</script>';
    const xssHost = dom.makeEl('div');
    const xssApi = createSolveTranscribeView(xssHost, {
      handles: [{ name: 'a', version: 1, latex: xss, str_preview: xss, portable: true }],
      steps: [{
        entry_id: 'e-1', entry_type: 'derivation', origin: 'given',
        summary: xss, details: xss, depends_on: [],
        entry_sha256: 'sha-x', transcription: { source_tex_sha256: 'tx' },
        claims: [{ form: 'equation', lhs: 'a@1', rhs: 'a@1', relation: 'eq' }],
        badge: { own: 'unchecked', effective: 'unchecked', label: 'Unchecked', flags: [xss, 'transcription unconfirmed'], stale: false },
      }],
    }, { katex: null });
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(xssHost.querySelectorAll('script').length, 0);
    assert.equal(xssHost.querySelectorAll('img').length, 0);
    xssApi.destroy();

    // 3. Static audit: zero raw HTML APIs and registered in assets.rs.
    const src = fs.readFileSync(path.join(assetsDir, 'solve_transcribe.js'), 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
    assert.ok(!/\b(innerHTML|outerHTML|insertAdjacentHTML)\b/.test(src));
    const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
    assert.ok(assetsRs.includes('"/assets/solve_transcribe.js"'));
  } finally {
    dom.restore();
  }
});

test('S17b audit regressions: workspace forwards onTranscribe/update, non-optimistic ack, drop PDF preserves draft, clean confirmed badge', async () => {
  const dom = makeFakeDom();
  try {
    // 1. Mounted SOLVE_TRANSCRIBE_TAB receives onTranscribe from workspace AND updates live on ws.update().
    registerSolveTranscribeTab();
    const wsHost = dom.makeEl('div');
    const sent = [];
    const view1 = makeFixtureView();
    const ws = createSolveWorkspace(wsHost, view1, {
      katex: null,
      initialTab: SOLVE_TRANSCRIBE_TAB.id,
      sessionId: 's-17',
      token: 'tok-xyz',
      onTranscribe: async (p) => { sent.push(p); return { ok: true, equation_count: 1, view: view1 }; },
    });
    const form = wsHost.querySelector('[data-testid="solve-transcribe-form"]');
    form.querySelector('[data-testid="solve-transcribe-filename"]').value = 'ok.tex';
    form.querySelector('[data-testid="solve-transcribe-tex"]').value = '\\begin{equation}x=1\\end{equation}';
    await form.dispatch('submit');
    assert.equal(sent.length, 1);
    assert.equal(sent[0].file_name, 'ok.tex');

    // Dropping a PDF (by MIME type even without .pdf extension) rejects and preserves draftTex.
    await form.dispatch('drop', {
      dataTransfer: { files: [{ name: 'paper_no_ext', type: 'application/pdf', text: async () => '%PDF-1.7' }] },
    });
    assert.equal(wsHost.querySelector('[data-testid="solve-transcribe-error"]')?.textContent, PDF_REJECT_MESSAGE);
    assert.equal(wsHost.querySelector('[data-testid="solve-transcribe-tex"]').value, '\\begin{equation}x=1\\end{equation}');

    // ws.update() updates the mounted transcribe tab live when server clears flags.
    const confirmedView = {
      ...view1,
      steps: view1.steps.map((s) => ({ ...s, badge: { ...s.badge, flags: [] } })),
    };
    ws.update(confirmedView);
    assert.equal(wsHost.querySelectorAll('[data-testid="solve-transcription-confirmed"]').length, 2);
    ws.destroy();
    unregisterSolveTab(SOLVE_TRANSCRIBE_TAB.id);

    // 2. Non-optimistic ack: returning `{ ok: true }` without matching `ack` or `view` MUST NOT clear unconfirmed flag.
    const host = dom.makeEl('div');
    const api = createSolveTranscribeView(host, view1, {
      katex: null,
      onAck: async () => ({ ok: true }),
    });
    await host.querySelector('[data-action="confirm-transcription"]').dispatch('click');
    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-unconfirmed"]').length, 2);
    assert.ok(host.querySelector('[data-testid="solve-transcribe-error"]')?.textContent.includes('entry_sha256'));

    // Confirm with valid ack clears BOTH the chip AND the badge's stale 'transcription unconfirmed' flag.
    api.destroy();
    const api2 = createSolveTranscribeView(host, view1, {
      katex: null,
      onAck: async (_s, p) => ({ ok: true, ack: p }),
    });
    await host.querySelector('[data-action="confirm-transcription"]').dispatch('click');
    const firstCard = host.querySelectorAll('[data-testid="solve-transcribe-card"]')[0];
    assert.equal(firstCard.getAttribute('data-confirmed'), 'true');
    assert.ok(!firstCard.textContent.includes('transcription unconfirmed'), 'confirmed card must not keep stale unconfirmed flag');

    // Subsequent update(nextView) resets localAcks so server-unconfirmed state is authoritative.
    api2.update(view1);
    assert.equal(host.querySelectorAll('[data-testid="solve-transcription-unconfirmed"]').length, 2);
    api2.destroy();
  } finally {
    dom.restore();
  }
});

