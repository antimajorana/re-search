// Node tests for F5 UI pieces: left-rail New mode menu (new_menu.js) and mode rail tags (session_rail.js).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  MODE_MENU_ENTRIES,
  MORE_MENU_LABEL,
  NOT_BUILT_YET_LABEL,
  bindNewMenu,
  renderNewMenu,
} from '../assets/new_menu.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(here, '..');
const assetsDir = path.join(crateRoot, 'assets');

function makeFakeDom() {
  let innerHtmlWrites = 0;
  const byId = new Map();

  const makeEl = (tag = 'div') => {
    let idVal = '';
    let text = '';
    let html = '';
    const attrs = new Map();
    const classes = new Set();
    const listeners = new Map();

    const el = {
      tagName: String(tag).toUpperCase(),
      children: [],
      style: {},
      type: '',
      title: '',
      hidden: false,
      disabled: false,
      get id() { return idVal; },
      set id(v) {
        if (idVal && byId.get(idVal) === el) byId.delete(idVal);
        idVal = String(v || '');
        if (idVal) byId.set(idVal, el);
      },
      get className() { return [...classes].join(' '); },
      set className(v) {
        classes.clear();
        for (const c of String(v || '').split(/\s+/).filter(Boolean)) classes.add(c);
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, force) => (force ? classes.add(c) : classes.delete(c)),
        contains: (c) => classes.has(c),
      },
      get textContent() {
        return this.children.length ? this.children.map((c) => c.textContent ?? '').join('') : text;
      },
      set textContent(v) { this.children = []; text = String(v ?? ''); html = ''; },
      get innerHTML() { return html; },
      set innerHTML(v) { innerHtmlWrites += 1; this.children = []; html = String(v ?? ''); },
      setAttribute(k, v) { attrs.set(k, String(v)); if (k === 'id') this.id = v; },
      getAttribute(k) { return attrs.has(k) ? attrs.get(k) : null; },
      append(...nodes) { this.children.push(...nodes); },
      appendChild(node) { this.children.push(node); return node; },
      replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      dispatch(type, event = {}) {
        const ev = {
          prevented: false,
          stopped: false,
          preventDefault() { this.prevented = true; },
          stopPropagation() { this.stopped = true; },
          target: this,
          ...event,
        };
        for (const fn of listeners.get(type) || []) fn(ev);
        return ev;
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
  globalThis.document = {
    createElement: (tag) => makeEl(tag),
    getElementById: (id) => byId.get(id) || null,
  };
  return {
    makeEl,
    get innerHtmlWrites() { return innerHtmlWrites; },
    restore() { globalThis.document = prevDoc; },
  };
}

async function importSessionRail() {
  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const src = fs
    .readFileSync(path.join(assetsDir, 'session_rail.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  return import(`data:text/javascript;base64,${Buffer.from(src).toString('base64')}`);
}

test('new_menu.js disables Explore and Solve with "not built yet" and keeps Research under More', () => {
  const dom = makeFakeDom();
  try {
    const mount = dom.makeEl('div');
    const selected = [];
    renderNewMenu(mount, { onSelect: (mode) => selected.push(mode) });

    assert.equal(dom.innerHtmlWrites, 0, 'new_menu.js must not write innerHTML');
    assert.equal(MODE_MENU_ENTRIES.length, 4);

    const inv = mount.querySelector('#new-investigation-btn');
    const exp = mount.querySelector('#new-explore-btn');
    const sol = mount.querySelector('#new-solve-btn');
    const more = mount.querySelector('#rail-more-menu');
    const res = more?.querySelector('#new-research-btn');

    assert.ok(inv && !inv.disabled, 'Investigate button must be enabled');
    assert.ok(more && more.tagName === 'DETAILS', 'New research must live inside More details menu');
    assert.equal(more.querySelector('summary')?.textContent, MORE_MENU_LABEL);
    assert.ok(res && !res.disabled, 'New research button inside More must be enabled');

    for (const [name, btn] of [['Explore', exp], ['Solve', sol]]) {
      assert.ok(btn, `${name} button must be listed`);
      assert.equal(btn.disabled, true, `${name} must be disabled`);
      assert.equal(btn.getAttribute('aria-disabled'), 'true', `${name} must set aria-disabled`);
      assert.ok(
        btn.textContent.includes(NOT_BUILT_YET_LABEL),
        `${name} button must include "${NOT_BUILT_YET_LABEL}", got "${btn.textContent}"`
      );
      // Even if a caller forces click dispatch after clearing disabled, nothing starts.
      btn.disabled = false;
      const ev = btn.dispatch('click');
      assert.equal(ev.prevented, true, `click on unbuilt ${name} must be prevented`);
      assert.equal(selected.length, 0, `click on unbuilt ${name} must start nothing`);
    }

    inv.dispatch('click');
    res.dispatch('click');
    assert.deepEqual(selected, ['investigation', 'research']);

    // bindNewMenu guards statically rendered markup identically.
    const staticRoot = dom.makeEl('div');
    const staticExplore = dom.makeEl('button');
    staticExplore.id = 'new-explore-btn';
    staticRoot.append(staticExplore);
    const boundClicks = [];
    bindNewMenu(staticRoot, { onSelect: (m) => boundClicks.push(m) });
    assert.equal(staticExplore.disabled, true);
    assert.equal(staticExplore.getAttribute('aria-disabled'), 'true');
    staticExplore.dispatch('click');
    assert.deepEqual(boundClicks, []);
  } finally {
    dom.restore();
  }
});

test('new_menu.js escapes XSS payloads in custom entries without innerHTML', () => {
  const dom = makeFakeDom();
  try {
    const mount = dom.makeEl('div');
    renderNewMenu(mount, {
      entries: [
        {
          id: 'explore',
          apiMode: 'explore',
          buttonId: 'new-explore-btn',
          label: '<img src=x onerror=alert(1)>',
          title: '<script>alert(2)</script>',
          built: false,
          underMore: false,
        },
      ],
    });
    assert.equal(dom.innerHtmlWrites, 0);
    assert.equal(mount.querySelectorAll('img').length, 0);
    assert.equal(mount.querySelectorAll('script').length, 0);
  } finally {
    dom.restore();
  }
});

test('session_rail.js renders mode rail tags for all modes, falls back legacy sessions, filters by mode, and escapes XSS', async () => {
  const rail = await importSessionRail();
  const dom = makeFakeDom();
  try {
    assert.deepEqual(rail.formatModeTag('investigation'), {
      mode: 'investigate',
      label: 'Investigate',
      tone: 'mode-investigate',
    });
    assert.deepEqual(rail.formatModeTag('investigate'), {
      mode: 'investigate',
      label: 'Investigate',
      tone: 'mode-investigate',
    });
    assert.deepEqual(rail.formatModeTag('explore'), {
      mode: 'explore',
      label: 'Explore',
      tone: 'mode-explore',
    });
    assert.deepEqual(rail.formatModeTag('solve'), {
      mode: 'solve',
      label: 'Solve',
      tone: 'mode-solve',
    });
    assert.deepEqual(rail.formatModeTag('research'), {
      mode: 'research',
      label: 'Research (earlier)',
      tone: 'mode-research',
    });
    assert.deepEqual(rail.formatModeTag(undefined), {
      mode: 'research',
      label: 'Research (earlier)',
      tone: 'mode-research',
    });
    assert.deepEqual(rail.formatModeTag('<script>alert(1)</script>'), {
      mode: 'research',
      label: 'Research (earlier)',
      tone: 'mode-research',
    });

    const listEl = dom.makeEl('nav');
    const sessions = [
      { id: 's-inv', mode: 'investigation', title: 'Paper map', archived: false },
      { id: 's-exp', mode: 'explore', title: 'Idea check', archived: false },
      { id: 's-sol', mode: 'solve', title: 'Integral calc', archived: false },
      { id: 's-old', title: 'Legacy thread <img src=x onerror=alert(1)>', archived: false },
    ];
    rail.renderSessionRailList(listEl, { sessions, activeSessionId: 's-inv' });

    assert.equal(listEl.children.length, 4);
    const htmlByRow = listEl.children.map((row) => row.children[0].innerHTML);
    assert.match(htmlByRow[0], /data-mode-tag="investigate"[^>]*>Investigate<\/span>/);
    assert.match(htmlByRow[1], /data-mode-tag="explore"[^>]*>Explore<\/span>/);
    assert.match(htmlByRow[2], /data-mode-tag="solve"[^>]*>Solve<\/span>/);
    assert.match(htmlByRow[3], /data-mode-tag="research"[^>]*>Research \(earlier\)<\/span>/);
    assert.ok(!htmlByRow[3].includes('<img '), 'untrusted session title must be escaped');

    // Mode tag labels participate in search filtering.
    rail.renderSessionRailList(listEl, { sessions, query: 'solve' });
    assert.equal(listEl.children.length, 1);
    assert.match(listEl.children[0].children[0].innerHTML, /Integral calc/);
  } finally {
    dom.restore();
  }
});

test('index.html and assets.rs wire /assets/new_menu.js and disabled unbuilt modes', () => {
  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');

  assert.ok(assetsRs.includes('"/assets/new_menu.js"'), 'assets.rs must register /assets/new_menu.js');
  assert.match(indexHtml, /id="rail-new-menu"/);
  assert.match(indexHtml, /id="new-investigation-btn"/);
  assert.match(indexHtml, /id="new-explore-btn"[^>]*disabled/);
  assert.match(indexHtml, /id="new-solve-btn"[^>]*disabled/);
  assert.match(indexHtml, /id="rail-more-menu"/);
  assert.match(indexHtml, /id="new-research-btn"/);
  assert.match(indexHtml, /not built yet/);
  assert.match(indexHtml, /src="\/assets\/new_menu\.js"/);
});
