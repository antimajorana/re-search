// Node UI tests for F16b: "What I have learnt" page, "Remember this?" card,
// opt-in "(model)" draft trigger, "used N of your notes" disclosure, 300-char enforcement,
// asset registration, and zero-innerHTML XSS safety.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  LESSONS_MAX_CHARS,
  LESSONS_PAGE_TITLE,
  createLessonsManager,
  mountLessonsNavButton,
} from '../assets/lessons.js';
import {
  DEFAULT_SCOPE_LABEL,
  DRAFT_MODEL_BTN_LABEL,
  REMEMBER_CARD_TITLE,
  createRememberThisCard,
  createUsedLessonsDisclosure,
  mountUsedLessonsDisclosure,
} from '../assets/lesson_remember.js';
import { MODE_MENU_ENTRIES, renderNewMenu } from '../assets/new_menu.js';
import { createSolveWhyPopover } from '../assets/solve_why.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const assetsDir = path.join(crateRoot, 'assets');

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const makeEl = (tag = 'div') => {
    let text = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();
    const el = {
      tagName: String(tag).toUpperCase(),
      children: [],
      parentNode: null,
      style: {},
      type: '',
      name: '',
      value: '',
      placeholder: '',
      title: '',
      hidden: false,
      disabled: false,
      checked: false,
      selected: false,
      get firstChild() { return this.children[0] || null; },
      get firstElementChild() { return this.children[0] || null; },
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
      set textContent(v) { this.children = []; text = String(v ?? ''); },
      get innerHTML() { return ''; },
      set innerHTML(_) { innerHtmlWrites += 1; throw new Error('innerHTML write forbidden'); },
      set outerHTML(_) { innerHtmlWrites += 1; throw new Error('outerHTML write forbidden'); },
      insertAdjacentHTML() { innerHtmlWrites += 1; throw new Error('insertAdjacentHTML forbidden'); },
      setAttribute(k, v) {
        attrs.set(String(k), String(v));
        if (k === 'id') this.id = String(v);
        if (k === 'class') this.className = v;
        if (k === 'value') this.value = String(v);
      },
      getAttribute(k) { return attrs.has(String(k)) ? attrs.get(String(k)) : null; },
      removeAttribute(k) { attrs.delete(String(k)); },
      append(...nodes) {
        for (const n of nodes) {
          if (n && typeof n === 'object') n.parentNode = this;
          this.children.push(n);
          if (this.tagName === 'SELECT' && n?.tagName === 'OPTION' && (!this.value || n.selected)) {
            this.value = n.value ?? '';
          }
        }
      },
      appendChild(n) { this.append(n); return n; },
      prepend(...nodes) {
        for (const n of nodes) if (n && typeof n === 'object') n.parentNode = this;
        this.children.unshift(...nodes);
      },
      removeChild(n) {
        const i = this.children.indexOf(n);
        if (i >= 0) { this.children.splice(i, 1); n.parentNode = null; }
        return n;
      },
      replaceChildren(...nodes) {
        for (const c of this.children) if (c && typeof c === 'object') c.parentNode = null;
        this.children = [];
        text = '';
        this.append(...nodes);
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatch(type, event = {}) {
        const ev = { target: this, preventDefault() {}, stopPropagation() {}, ...event };
        for (const fn of [...(listeners.get(type) || [])]) fn(ev);
        return ev;
      },
      querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
      querySelectorAll(sel) {
        const out = [];
        const match = (n) => {
          if (!n || !n.tagName) return false;
          if (sel.startsWith('#')) return n.id === sel.slice(1);
          if (sel.startsWith('.')) return n.classList.contains(sel.slice(1));
          const attr = sel.match(/^([a-zA-Z0-9_-]+)?\[([^=\]]+)(?:="([^"]*)")?\]$/);
          if (attr) {
            if (attr[1] && n.tagName !== attr[1].toUpperCase()) return false;
            const val = n.getAttribute(attr[2]);
            return attr[3] === undefined ? val !== null : val === attr[3];
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
  const doc = {
    body: makeEl('body'),
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    getElementById: (id) => doc.body.querySelector(`#${id}`),
    querySelector: (sel) => doc.body.querySelector(sel),
  };
  globalThis.document = doc;
  return {
    doc,
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

async function flush() {
  for (let i = 0; i < 6; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 5));
}

test('F16b "What I have learnt" lists scope, origin NodeRef link, date, uses, enforces 300 chars, edits, deletes, and exports opt-in', async () => {
  const dom = makeFakeDom();
  try {
    let store = [
      {
        id: 'les-1',
        text: 'Use mostly-minus signature (+,-,-,-).',
        scope: { kind: 'mode_field', mode: 'solve', field: 'qft' },
        from: { session: 's-1', mode: 'solve', id: 'e-4' },
        model_summary: false,
        created_ms: Date.UTC(2026, 10, 1),
        updated_ms: Date.UTC(2026, 10, 1),
        uses: 3,
      },
      {
        id: 'les-2',
        text: 'Check loop symmetry factors. (model)',
        scope: { kind: 'global' },
        from: { session: 's-2', mode: 'explore', id: 'h-1' },
        model_summary: true,
        created_ms: Date.UTC(2026, 10, 2),
        updated_ms: Date.UTC(2026, 10, 2),
        uses: 1,
      },
    ];
    const calls = [];
    const navigated = [];
    const fetchJson = async (method, url, body) => {
      calls.push({ method, url, body });
      if (method === 'GET' && url === '/api/lessons/notes') return { lessons: [...store] };
      if (method === 'PUT' && url === '/api/lessons/notes/les-1') {
        store[0] = { ...store[0], text: body.text, scope: body.scope, model_summary: Boolean(body.model_summary) };
        return { ok: true, lesson: store[0] };
      }
      if (method === 'DELETE' && url === '/api/lessons/notes/les-2') {
        store = store.filter((l) => l.id !== 'les-2');
        return { ok: true, deleted: true };
      }
      if (method === 'GET' && url === '/api/lessons/notes/backup?confirm=true') {
        return { backup: { schema_version: 1, checksum_sha256: 'abc', lessons: [...store] } };
      }
      throw new Error(`unexpected ${method} ${url}`);
    };

    const host = dom.makeEl('div');
    const mgr = createLessonsManager(host, {
      document: dom.doc,
      fetchJson,
      onNavigateOrigin: (from) => navigated.push(from),
    });
    await mgr.refresh();
    assert.equal(calls.filter((c) => c.url.includes('backup')).length, 0);
    assert.equal(host.querySelectorAll('[data-testid="lesson-item"]').length, 2);
    assert.ok(host.textContent.includes(LESSONS_PAGE_TITLE));
    assert.ok(host.querySelector('[data-testid="lesson-uses"]').textContent.includes('3'));
    assert.ok(host.querySelector('[data-testid="lesson-model-badge"]'));

    // Click origin NodeRef link.
    host.querySelector('[data-testid="lesson-origin-link"]').dispatch('click');
    assert.deepEqual(navigated[0], { session: 's-1', mode: 'solve', id: 'e-4' });

    // Edit first lesson: > 300 chars blocked locally without network call.
    host.querySelector('[data-action="edit-lesson"]').dispatch('click');
    const editArea = host.querySelector('[data-testid="lesson-edit-input"]');
    editArea.value = 'α'.repeat(LESSONS_MAX_CHARS + 1);
    editArea.dispatch('input');
    assert.ok(host.querySelector('[data-testid="lesson-char-count"]').textContent.includes('301 / 300'));
    await host.querySelector('[data-testid="lesson-edit-form"]').dispatch('submit');
    assert.equal(calls.filter((c) => c.method === 'PUT').length, 0);

    // Valid edit saves and refreshes (re-query rendered textarea after validation re-render).
    const editAreaAfter = host.querySelector('[data-testid="lesson-edit-input"]');
    editAreaAfter.value = 'Always check Ward identities before gauge fixing.';
    editAreaAfter.dispatch('input');
    host.querySelector('[data-testid="lesson-edit-form"]').dispatch('submit');
    await flush();
    assert.equal(calls.filter((c) => c.method === 'PUT').length, 1);
    assert.ok(host.textContent.includes('Always check Ward identities before gauge fixing.'));

    // Delete requires two clicks (confirm) then calls DELETE.
    const delButtons = host.querySelectorAll('[data-action="delete-lesson"]');
    delButtons[1].dispatch('click');
    assert.equal(calls.filter((c) => c.method === 'DELETE').length, 0);
    host.querySelector('[data-action="confirm-delete-lesson"]').dispatch('click');
    await flush();
    assert.equal(calls.filter((c) => c.method === 'DELETE').length, 1);
    assert.equal(host.querySelectorAll('[data-testid="lesson-item"]').length, 1);

    // Opt-in backup runs only on click.
    host.querySelector('[data-action="export-lessons"]').dispatch('click');
    await flush();
    assert.ok(host.querySelector('[data-testid="lessons-backup-json"]').value.includes('checksum_sha256'));

    // Left-rail footer entry mounts cleanly alongside F5 MODE_MENU_ENTRIES without altering length 4.
    assert.equal(MODE_MENU_ENTRIES.length, 4);
    const railMenu = dom.makeEl('div');
    renderNewMenu(railMenu, { doc: dom.doc });
    const footer = dom.makeEl('div');
    mountLessonsNavButton(footer, { document: dom.doc, onOpen: () => {} });
    assert.ok(footer.querySelector('#open-lessons-btn').textContent.includes(LESSONS_PAGE_TITLE));
  } finally {
    dom.restore();
  }
});

test('F16b "Remember this?" uses user words with 0 model calls, defaults (mode, field), requires explicit save, and supports opt-in model draft', async () => {
  const dom = makeFakeDom();
  try {
    const saved = [];
    const modelCalls = [];
    const host = dom.makeEl('div');
    const pop = createSolveWhyPopover(host, {
      document: dom.doc,
      sessionId: 's-solve-1',
      field: 'qft',
      step: {
        entry_id: 'e-7',
        entry_type: 'calculation',
        summary: 'Compute one-loop diagram',
        badge: { own: 'assumed_by_model', effective: 'assumed_by_model', label: 'Assumed by model', border: 'dashed', flags: [] },
        claims: [],
        deps: [],
      },
      checks: [],
      onSaveLesson: async (payload) => { saved.push(payload); return { lesson: payload }; },
      onDraftLessonSummary: async (req) => {
        modelCalls.push(req);
        return { text: 'Track 1/2 symmetry factor on sunset graphs. (model)', model_summary: true };
      },
    });

    // Submitting step correction triggers "Remember this?" with 0 model calls and 0 saves.
    host.querySelector('[data-why-action="correct_step"]').dispatch('click');
    const corrForm = host.querySelector('[data-testid="solve-correct-form"]');
    corrForm.querySelector('[data-testid="solve-correct-input"]').value = 'You forgot the 1/2 symmetry factor here.';
    corrForm.dispatch('submit');

    assert.equal(modelCalls.length, 0);
    assert.equal(saved.length, 0);
    const remCard = host.querySelector('[data-testid="remember-lesson-card"]');
    assert.ok(remCard);
    assert.ok(remCard.textContent.includes(REMEMBER_CARD_TITLE));
    const ta = remCard.querySelector('[data-testid="remember-text-input"]');
    assert.equal(ta.value, 'You forgot the 1/2 symmetry factor here.');
    const scopeSel = remCard.querySelector('[data-testid="remember-scope-select"]');
    assert.equal(scopeSel.value, 'mode_field');
    assert.ok(scopeSel.children[0].textContent.includes(DEFAULT_SCOPE_LABEL));

    // Dismissing leaves 0 saved rows and 0 model calls.
    remCard.querySelector('[data-testid="remember-dismiss-btn"]').dispatch('click');
    assert.equal(host.querySelector('[data-testid="remember-lesson-card"]'), null);
    assert.equal(saved.length, 0);
    assert.equal(modelCalls.length, 0);

    // Direct card test: > 300 chars refused, opt-in model draft only runs on explicit button click & still awaits explicit save.
    const card2 = createRememberThisCard(dom.doc, {
      from: { session: 's-solve-1', mode: 'solve', id: 'e-8' },
      field: 'qft',
      userWords: 'Keep track of 1/2 factor.',
    }, {
      onSave: async (p) => { saved.push(p); return { lesson: p }; },
      onDraftSummary: async (req) => {
        modelCalls.push(req);
        return { text: 'Track 1/2 symmetry factor. (model)', model_summary: true };
      },
    });
    const el2 = card2.element;
    const ta2 = el2.querySelector('[data-testid="remember-text-input"]');
    ta2.value = 'β'.repeat(305);
    ta2.dispatch('input');
    el2.querySelector('[data-testid="remember-save-btn"]').dispatch('click');
    await flush();
    assert.equal(saved.length, 0);
    assert.ok(el2.querySelector('[data-testid="remember-error"]').textContent.includes('300'));

    ta2.value = 'Keep track of 1/2 factor.';
    ta2.dispatch('input');
    const draftBtn = el2.querySelector('[data-testid="remember-draft-model-btn"]');
    assert.equal(draftBtn.textContent, DRAFT_MODEL_BTN_LABEL);
    draftBtn.dispatch('click');
    await flush();
    assert.equal(modelCalls.length, 1);
    assert.equal(saved.length, 0, 'drafting summary must not auto-save');
    assert.equal(el2.querySelector('[data-testid="remember-text-input"]').value, 'Track 1/2 symmetry factor. (model)');
    assert.ok(el2.querySelector('[data-testid="remember-model-badge"]'));

    el2.querySelector('[data-testid="remember-save-btn"]').dispatch('click');
    await flush();
    assert.equal(saved.length, 1);
    assert.equal(saved[0].model_summary, true);
    assert.deepEqual(saved[0].scope, { kind: 'mode_field', mode: 'solve', field: 'qft' });
    pop.destroy();
  } finally {
    dom.restore();
  }
});

test('F16b "used N of your notes" disclosure renders ledger-frozen notes and hides when 0', () => {
  const dom = makeFakeDom();
  try {
    const empty = createUsedLessonsDisclosure(dom.doc, { count: 0, disclosure: null, records: [] });
    assert.equal(empty.element.hidden, true);
    assert.equal(empty.element.querySelector('[data-testid="used-lessons-pill"]'), null);

    const host = dom.makeEl('div');
    const clicked = [];
    const widget = mountUsedLessonsDisclosure(host, {
      count: 2,
      disclosure: 'used 2 of your notes',
      records: [{
        run_id: 'run-1',
        lesson_ids: ['les-1', 'les-2'],
        notes: [
          { id: 'les-1', text: 'Note one', scope_label: 'solve + qft', from: { session: 's-1', mode: 'solve', id: 'e-1' } },
          { id: 'les-2', text: 'Note two (model)', scope_label: 'Global', from: { session: 's-2', mode: 'explore', id: 'h-2' } },
        ],
      }],
    }, { document: dom.doc, onNavigateOrigin: (from) => clicked.push(from) });

    const pill = widget.element.querySelector('[data-testid="used-lessons-pill"]');
    assert.equal(pill.textContent, 'used 2 of your notes');
    assert.equal(widget.element.querySelector('[data-testid="used-lessons-panel"]'), null);
    pill.dispatch('click');
    const items = widget.element.querySelectorAll('[data-testid="used-lesson-item"]');
    assert.equal(items.length, 2);
    assert.ok(items[0].textContent.includes('Note one'));
    assert.ok(items[1].textContent.includes('Note two (model)'));
    widget.element.querySelector('[data-testid="used-lesson-origin"]').dispatch('click');
    assert.equal(clicked[0].session, 's-1');
  } finally {
    dom.restore();
  }
});

test('F16b XSS safety audit and assets.rs registration check', () => {
  const dom = makeFakeDom();
  try {
    const xss = '<script>alert(1)</script><img src=x onerror=alert(2)><svg onload=alert(3)>';
    const host = dom.makeEl('div');
    createLessonsManager(host, {
      document: dom.doc,
      initialLessons: [{
        id: 'les-xss',
        text: xss,
        scope: { kind: 'field', field: xss },
        from: { session: xss, mode: 'solve', id: xss },
        model_summary: true,
        created_ms: 1700000000000,
        uses: 2,
      }],
    });
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(host.querySelectorAll('script').length, 0);
    assert.equal(host.querySelectorAll('img').length, 0);
    assert.ok(host.textContent.includes('<script>alert(1)</script>'));

    const rem = createRememberThisCard(dom.doc, {
      from: { session: xss, mode: 'solve', id: xss },
      field: xss,
      userWords: xss,
    });
    assert.equal(rem.element.querySelectorAll('script').length, 0);

    const disc = createUsedLessonsDisclosure(dom.doc, {
      count: 1,
      disclosure: 'used 1 of your notes',
      records: [{ run_id: xss, lesson_ids: ['les-xss'], notes: [{ id: 'les-xss', text: xss, scope_label: xss, from: { session: xss, mode: 'solve', id: xss } }] }],
    });
    disc.element.querySelector('[data-testid="used-lessons-pill"]').dispatch('click');
    assert.equal(disc.element.querySelectorAll('svg').length, 0);
    assert.equal(dom.innerHtmlWrites, 0);

    for (const fname of ['lessons.js', 'lesson_remember.js']) {
      const src = fs.readFileSync(path.join(assetsDir, fname), 'utf8');
      assert.ok(!/\.innerHTML\b|\.outerHTML\b|insertAdjacentHTML\b/.test(src), `${fname} must not use HTML sinks`);
    }
    const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
    assert.ok(assetsRs.includes('"/assets/lessons.js"'));
    assert.ok(assetsRs.includes('"/assets/lesson_remember.js"'));
  } finally {
    dom.restore();
  }
});
