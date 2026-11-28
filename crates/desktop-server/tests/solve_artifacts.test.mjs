import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const ASSETS_DIR = path.join(REPO_ROOT, 'crates', 'desktop-server', 'assets');

import {
  SOLVE_ARTIFACTS_TAB,
  artifactDownloadUrl,
  formatArtifactBytes,
  mountSolveArtifactsSection,
  registerSolveTab,
  renderSolveArtifacts,
} from '../assets/solve_artifacts.js';
import { createModeStagesController } from '../assets/mode_stages.js';

function createMockDocument() {
  const byId = new Map();

  function makeStyle() {
    return {};
  }

  function matchesSelector(node, sel) {
    if (!sel) return false;
    if (sel.startsWith('#')) return node.id === sel.slice(1);
    if (sel.startsWith('.')) {
      const cls = sel.slice(1);
      return node.className.split(/\s+/).includes(cls);
    }
    const tagAttr = /^link\[href="([^"]+)"\]$/.exec(sel);
    if (tagAttr) return node.tagName === 'LINK' && node.href === tagAttr[1];
    return node.tagName === sel.toUpperCase();
  }

  function walkDescendants(root, predicate, out = []) {
    for (const child of root.children) {
      if (predicate(child)) out.push(child);
      walkDescendants(child, predicate, out);
    }
    return out;
  }

  function makeElement(tagName) {
    const attrs = new Map();
    const listeners = new Map();
    const children = [];
    let _id = '';
    let _className = '';
    let _textContent = '';

    const el = {
      tagName: String(tagName || 'div').toUpperCase(),
      children,
      ownerDocument: null,
      parentNode: null,
      hidden: false,
      disabled: false,
      style: makeStyle(),
      dataset: {},
      classList: {
        add(...names) {
          const set = new Set(_className.split(/\s+/).filter(Boolean));
          for (const n of names) set.add(n);
          _className = Array.from(set).join(' ');
        },
        remove(...names) {
          const drop = new Set(names);
          _className = _className.split(/\s+/).filter((n) => n && !drop.has(n)).join(' ');
        },
        contains(name) {
          return _className.split(/\s+/).includes(name);
        },
        toggle(name, force) {
          if (force === true) this.add(name);
          else if (force === false) this.remove(name);
          else if (this.contains(name)) this.remove(name);
          else this.add(name);
        },
      },
      get id() {
        return _id;
      },
      set id(val) {
        if (_id) byId.delete(_id);
        _id = String(val || '');
        if (_id) byId.set(_id, el);
      },
      get className() {
        return _className;
      },
      set className(val) {
        _className = String(val || '');
      },
      get textContent() {
        if (children.length === 0) return _textContent;
        return _textContent + children.map((c) => c.textContent).join('');
      },
      set textContent(val) {
        children.length = 0;
        _textContent = val === undefined || val === null ? '' : String(val);
      },
      get innerHTML() {
        throw new Error('innerHTML read forbidden');
      },
      set innerHTML(_) {
        throw new Error('innerHTML write forbidden');
      },
      set outerHTML(_) {
        throw new Error('outerHTML write forbidden');
      },
      insertAdjacentHTML() {
        throw new Error('insertAdjacentHTML forbidden');
      },
      setAttribute(k, v) {
        if (k === 'id') this.id = v;
        else if (k === 'class') this.className = v;
        else attrs.set(k, String(v));
      },
      getAttribute(k) {
        if (k === 'id') return _id || null;
        if (k === 'class') return _className || null;
        return attrs.has(k) ? attrs.get(k) : null;
      },
      removeAttribute(k) {
        attrs.delete(k);
      },
      get firstChild() {
        return children[0] || null;
      },
      get firstElementChild() {
        return children[0] || null;
      },
      appendChild(child) {
        if (child.parentNode && typeof child.remove === 'function') child.remove();
        child.parentNode = el;
        children.push(child);
        return child;
      },
      prepend(...nodes) {
        for (let i = nodes.length - 1; i >= 0; i -= 1) {
          const n = typeof nodes[i] === 'string' ? doc.createTextNode(nodes[i]) : nodes[i];
          if (n) {
            if (n.parentNode && typeof n.remove === 'function') n.remove();
            n.parentNode = el;
            children.unshift(n);
          }
        }
      },
      append(...nodes) {
        for (const n of nodes) {
          if (typeof n === 'string') {
            _textContent += n;
          } else if (n) {
            this.appendChild(n);
          }
        }
      },
      replaceChildren(...nodes) {
        for (const c of children) c.parentNode = null;
        children.length = 0;
        _textContent = '';
        this.append(...nodes);
      },
      remove() {
        if (!el.parentNode) return;
        const idx = el.parentNode.children.indexOf(el);
        if (idx >= 0) el.parentNode.children.splice(idx, 1);
        el.parentNode = null;
      },
      addEventListener(type, fn) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push(fn);
      },
      async click() {
        for (const fn of listeners.get('click') || []) {
          await fn({ type: 'click', target: el, preventDefault() {} });
        }
      },
      focus() {},
      querySelector(sel) {
        return this.querySelectorAll(sel)[0] || null;
      },
      querySelectorAll(sel) {
        return walkDescendants(el, (node) => matchesSelector(node, sel));
      },
      closest(sel) {
        let cur = el;
        while (cur) {
          if (matchesSelector(cur, sel)) return cur;
          cur = cur.parentNode;
        }
        return null;
      },
    };
    el.ownerDocument = doc;
    return el;
  }

  const doc = {
    createElement(tag) {
      return makeElement(tag);
    },
    createElementNS(_ns, tag) {
      return makeElement(tag);
    },
    createTextNode(txt) {
      const node = makeElement('#text');
      node.textContent = txt;
      return node;
    },
    getElementById(id) {
      return byId.get(id) || null;
    },
    querySelector(sel) {
      if (sel === 'meta[name="re-search-csrf"]') return null;
      return doc.body.querySelector(sel) || doc.head.querySelector(sel);
    },
    querySelectorAll(sel) {
      return doc.body.querySelectorAll(sel);
    },
  };
  doc.head = makeElement('head');
  doc.body = makeElement('body');
  return doc;
}

function makeUrlSpy() {
  let nextId = 1;
  const created = [];
  const revoked = [];
  return {
    created,
    revoked,
    createObjectURL(blob) {
      const u = `blob:test/${nextId++}`;
      created.push({ url: u, blob });
      return u;
    },
    revokeObjectURL(u) {
      revoked.push(u);
    },
  };
}

test('solve_artifacts static XSS and banned-wordliness audit', () => {
  const artifactsSrc = fs.readFileSync(path.join(ASSETS_DIR, 'solve_artifacts.js'), 'utf8');
  const modeStagesSrc = fs.readFileSync(path.join(ASSETS_DIR, 'mode_stages.js'), 'utf8');
  const assetsRs = fs.readFileSync(
    path.join(REPO_ROOT, 'crates', 'desktop-server', 'src', 'assets.rs'),
    'utf8',
  );

  const stripComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  for (const [label, src] of [
    ['solve_artifacts.js (raw)', artifactsSrc],
    ['solve_artifacts.js (code)', stripComments(artifactsSrc)],
    ['mode_stages.js (code)', stripComments(modeStagesSrc)],
  ]) {
    assert.equal(/\binnerHTML\b/.test(src), false, `${label} must have zero innerHTML`);
    assert.equal(/\bouterHTML\b/.test(src), false, `${label} must have zero outerHTML`);
    assert.equal(/\binsertAdjacentHTML\b/.test(src), false, `${label} must have zero insertAdjacentHTML`);
    assert.equal(/<svg\b/i.test(src), false, `${label} must have zero inline <svg>`);
    assert.equal(/\bverified\b/i.test(src), false, `${label} must never say Verified`);
  }
  assert.ok(assetsRs.includes('"/assets/solve_artifacts.js"'));
});

test('registerSolveTab registers artifacts tab in Map, Array, function, and plain object', () => {
  const mapReg = new Map();
  registerSolveTab(mapReg);
  assert.deepEqual(mapReg.get('artifacts'), SOLVE_ARTIFACTS_TAB);

  const arrReg = [{ id: 'setup', label: 'Setup' }];
  registerSolveTab(arrReg);
  registerSolveTab(arrReg);
  assert.equal(arrReg.length, 2);

  const objReg = {};
  registerSolveTab(objReg);
  assert.equal(objReg.artifacts.label, 'Artifacts');
  assert.equal(formatArtifactBytes(1536), '1.5 KB');
  assert.equal(artifactDownloadUrl('sess_1/2', 'art:3'), '/api/sessions/sess_1%2F2/solve/artifacts/art%3A3');
});

test('renderSolveArtifacts previews PNG via auth fetch + download-only SVG/CSV/JSON/PDF + safe XSS escaping', async () => {
  const doc = createMockDocument();
  const host = doc.createElement('div');
  doc.body.appendChild(host);
  const urlSpy = makeUrlSpy();
  const calls = [];
  const artifacts = [
    {
      artifact_id: 'art_png1',
      name: 'plots/<img src=x onerror=alert(1)>.png',
      mime: 'image/png',
      bytes: 2048,
    },
    {
      artifact_id: 'art_svg1',
      name: 'diagram.svg',
      mime: 'image/svg+xml',
      bytes: 512,
    },
    {
      artifact_id: 'art_csv1',
      name: 'table.csv',
      mime: 'text/csv',
      bytes: 128,
    },
    {
      artifact_id: 'art_pdf1',
      name: 'report.pdf',
      mime: 'application/pdf',
      bytes: 4096,
    },
  ];

  const handle = renderSolveArtifacts(host, artifacts, {
    sessionId: 'sess_test_1',
    document: doc,
    urlApi: urlSpy,
    getToken: () => 'tok-secret-123',
    scheduleRevoke: (fn) => fn(),
    fetchFn: async (url, init) => {
      calls.push({ url, headers: init.headers });
      return {
        ok: true,
        status: 200,
        async blob() {
          return { size: 42 };
        },
      };
    },
  });

  // Allow PNG preview fetch promise to resolve.
  await new Promise((r) => setTimeout(r, 10));

  assert.equal(calls.length, 1, 'only PNG should auto-fetch for inline preview');
  assert.equal(calls[0].url, '/api/sessions/sess_test_1/solve/artifacts/art_png1');
  assert.equal(calls[0].headers['X-ReSearch-Token'], 'tok-secret-123');

  const imgs = host.querySelectorAll('img');
  assert.equal(imgs.length, 1, 'non-PNG artifacts must never create <img> tags');
  assert.equal(imgs[0].src, 'blob:test/1');
  assert.equal(imgs[0].hidden, false);

  const cards = host.querySelectorAll('.solve-artifact-card');
  assert.equal(cards.length, 4);
  assert.equal(host.querySelectorAll('svg').length, 0);
  const firstName = cards[0].querySelector('.solve-artifact-name');
  assert.equal(firstName.textContent, 'plots/<img src=x onerror=alert(1)>.png');

  // Trigger download for SVG artifact.
  const buttons = host.querySelectorAll('.solve-artifact-download');
  assert.equal(buttons.length, 4);
  await buttons[1].click();
  assert.equal(calls.length, 2);
  assert.equal(calls[1].url, '/api/sessions/sess_test_1/solve/artifacts/art_svg1');
  assert.ok(urlSpy.revoked.includes('blob:test/2'), 'download blob URL must be revoked');

  handle.destroy();
  assert.ok(urlSpy.revoked.includes('blob:test/1'), 'preview blob URL must be revoked on destroy');
  assert.equal(handle.activeObjectUrlCount, 0);
});

test('mode_stages mounts solve_artifacts below solve_setup and cleans up on stage switch', async () => {
  const prevDoc = globalThis.document;
  const doc = createMockDocument();
  globalThis.document = doc;
  const urlSpy = makeUrlSpy();

  try {
    const appShell = doc.createElement('div');
    appShell.id = 'app-shell';
    const solveHost = doc.createElement('div');
    solveHost.id = 'solve-session-stage';
    const exploreHost = doc.createElement('div');
    exploreHost.id = 'explore-session-stage';
    appShell.append(solveHost, exploreHost);
    doc.body.appendChild(appShell);

    const viewPayload = {
      session_id: 'sess_solve_42',
      phase: 'running',
      contract: { goal: 'Check plot export', kind: 'analytic' },
      artifacts: [
        {
          artifact_id: 'art_p1',
          name: 'dispersion.png',
          mime: 'image/png',
          bytes: 1024,
        },
        {
          artifact_id: 'art_j1',
          name: 'fit.json',
          mime: 'application/json',
          bytes: 256,
        },
      ],
    };

    const ctrl = createModeStagesController({
      getElementById: (id) => doc.getElementById(id),
      getToken: () => 'tok-mode-stage',
      urlApi: urlSpy,
      scheduleRevoke: (fn) => fn(),
      fetchJson: async (method, reqPath) => {
        if (method === 'GET' && reqPath === '/api/sessions/sess_solve_42/solve/view') {
          return viewPayload;
        }
        return {};
      },
      fetchArtifact: async () => ({
        ok: true,
        status: 200,
        async blob() {
          return { size: 10 };
        },
      }),
    });

    await ctrl.syncFromRail('sess_solve_42', [{ id: 'sess_solve_42', mode: 'solve' }]);
    await new Promise((r) => setTimeout(r, 10));

    assert.deepEqual(ctrl.getSolveTabs().map((t) => t.id), ['artifacts', 'transcribe']);
    const sec = solveHost.querySelector('.solve-artifacts-section');
    assert.ok(sec, 'solve artifacts section should mount inside solve host');
    assert.equal(sec.querySelectorAll('.solve-artifact-card').length, 2);
    assert.equal(urlSpy.created.length, 1);

    await ctrl.syncFromRail(null, []);
    assert.equal(ctrl.getSolveArtifacts(), null);
    assert.equal(urlSpy.revoked.length, 1);
  } finally {
    globalThis.document = prevDoc;
  }
});

test('mountSolveArtifactsSection hides empty artifact lists', () => {
  const doc = createMockDocument();
  const host = doc.createElement('div');
  const mounted = mountSolveArtifactsSection(host, { artifacts: [] }, { document: doc });
  assert.equal(mounted.section.hidden, true);
  assert.equal(mounted.section.children.length, 0);
});
