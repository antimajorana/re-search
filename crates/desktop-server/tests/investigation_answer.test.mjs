import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const assetsDir = new URL('../assets/', import.meta.url);
const require = createRequire(import.meta.url);
const katex = require(fileURLToPath(new URL('../vendor/katex/katex.min.js', import.meta.url)));

function createElement(tag = 'div') {
  const listeners = new Map();
  const attrs = new Map();
  let text = '';
  let html = '';
  return {
    tagName: String(tag).toUpperCase(),
    dataset: {},
    children: [],
    hidden: false,
    disabled: false,
    open: false,
    type: '',
    className: '',
    get textContent() {
      if (this.children.length) return this.children.map(c => c.textContent ?? '').join('');
      return text;
    },
    set textContent(v) {
      this.children = [];
      text = String(v ?? '');
      html = '';
    },
    get innerHTML() {
      if (html) return html;
      if (this.children.length) return this.children.map(c => c.innerHTML || c.textContent || '').join('');
      return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    },
    set innerHTML(v) {
      this.children = [];
      html = String(v ?? '');
      text = html.replace(/<[^>]+>/g, '');
    },
    setAttribute(name, val) { attrs.set(name, String(val)); },
    getAttribute(name) { return attrs.get(name); },
    addEventListener(name, fn) { listeners.set(name, fn); },
    append(...nodes) { this.children.push(...nodes); },
    replaceChildren(...nodes) { this.children = [...nodes]; text = ''; html = ''; },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    querySelectorAll(sel) {
      const out = [];
      const match = node => {
        if (!node.tagName) return false;
        if (sel.startsWith('.')) return String(node.className || '').split(/\s+/).includes(sel.slice(1));
        return node.tagName === sel.toUpperCase();
      };
      const visit = node => {
        for (const child of node.children || []) {
          if (match(child)) out.push(child);
          visit(child);
        }
      };
      visit(this);
      return out;
    },
    async click() { return listeners.get('click')?.({ currentTarget: this, preventDefault() {} }); },
    dispatchToggle() { return listeners.get('toggle')?.({ currentTarget: this }); },
  };
}

function dataUrl(code) {
  return 'data:text/javascript;base64,' + Buffer.from(code).toString('base64');
}

async function loadAnswerCard(t) {
  const prev = { document: globalThis.document, window: globalThis.window, NodeFilter: globalThis.NodeFilter };
  globalThis.NodeFilter = { SHOW_TEXT: 4 };
  globalThis.document = {
    createElement,
    createTextNode: v => ({ textContent: String(v ?? ''), innerHTML: String(v ?? ''), children: [] }),
    createTreeWalker: root => {
      const html = root.innerHTML || '';
      const raw = html.replace(/<[^>]+>/g, '')
        .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&');
      let yielded = false;
      const textNode = {
        nodeValue: raw,
        parentElement: { tagName: 'P' },
        parentNode: {
          replaceChild(next) {
            root.innerHTML = next.innerHTML;
          },
        },
      };
      return {
        nextNode: () => {
          if (yielded || !raw) return null;
          yielded = true;
          return textNode;
        },
      };
    },
  };
  globalThis.window = { katex };
  t.after(() => Object.assign(globalThis, prev));

  const mathHref = new URL('math.js', assetsDir).href;
  const source = (await readFile(new URL('investigation_answer.js', assetsDir), 'utf8'))
    .replace("'/assets/math.js'", JSON.stringify(mathHref));
  return import(dataUrl(source));
}

test('answer card escapes HTML in question and body and renders \\(x\\) as one .katex', async t => {
  const { createAnswerCard } = await loadAnswerCard(t);
  const host = createElement('section');
  const calls = { map: [], chat: 0 };
  const card = createAnswerCard(host, {
    onFindMainResults: async q => { calls.map.push(q); },
    onOpenConversation: () => { calls.chat += 1; },
  });

  const payload = '<img src=x onerror=alert(1)>';
  card.update([
    {
      seq: 3,
      question: `What does ${payload} mean?`,
      status: 'answered',
      text: `Value \\(x\\) with ${payload} [L12].`,
    },
  ], { hasMap: false, running: false });

  assert.equal(host.hidden, false);
  assert.equal(host.querySelector('h2').textContent, 'Answer');
  const qEl = host.querySelector('.investigation-answer-question');
  const bodyEl = host.querySelector('.investigation-answer-body');
  assert.doesNotMatch(qEl.innerHTML, /<img/i);
  assert.doesNotMatch(bodyEl.innerHTML, /<img/i);
  assert.match(qEl.innerHTML, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.match(bodyEl.innerHTML, /&lt;img src=x onerror=alert\(1\)&gt;/);
  assert.match(qEl.textContent, /<img src=x onerror=alert\(1\)>/);
  assert.equal((bodyEl.innerHTML.match(/class="katex"/g) || []).length, 1, '\\(x\\) must render one .katex');
  assert.match(host.textContent, /Written by the model\. Check the cited lines\./);

  const buttons = host.querySelectorAll('button');
  assert.deepEqual(buttons.map(b => b.textContent), ['Find main results', 'Open in Conversation']);
  await buttons[0].click();
  assert.deepEqual(calls.map, [`What does ${payload} mean?`]);
  await buttons[1].click();
  assert.equal(calls.chat, 1);

  card.update([
    { seq: 3, question: 'Q', status: 'answered', text: 'A' },
  ], { hasMap: true, running: false });
  assert.deepEqual(host.querySelectorAll('button').map(b => b.textContent), ['Open in Conversation']);
});

test('answer card shows running placeholder, stopped copy, failed error text, and lazy earlier answers', async t => {
  const { createAnswerCard } = await loadAnswerCard(t);
  const host = createElement('section');
  const card = createAnswerCard(host);

  card.update([{ seq: 1, question: 'First?', status: 'running' }], { hasMap: false, running: true });
  assert.match(host.textContent, /Working on your question…/);
  assert.equal(host.querySelectorAll('button')[0].disabled, true);

  card.update([{ seq: 1, question: 'First?', status: 'stopped' }], { hasMap: false, running: false });
  assert.match(host.textContent, /Stopped before the answer was written\. Ask again below\./);

  card.update([{ seq: 1, question: 'First?', status: 'failed', error: '<script>boom</script>' }], { hasMap: false, running: false });
  assert.match(host.textContent, /<script>boom<\/script>/);
  const meta = host.querySelector('.investigation-meta');
  assert.equal(meta.children.length, 0, 'error must be set via textContent');

  card.update([
    { seq: 1, question: 'Old Q1', status: 'answered', text: 'Old A1' },
    { seq: 2, question: 'Old Q2', status: 'answered', text: 'Old A2' },
    { seq: 3, question: 'Latest Q', status: 'answered', text: 'Latest A' },
  ], { hasMap: false, running: false });
  const details = host.querySelector('details');
  assert.ok(details, 'earlier answers disclosure must exist');
  assert.equal(details.open, false);
  assert.equal(details.querySelector('summary').textContent, 'Earlier questions (2)');
  assert.equal(details.querySelectorAll('article').length, 0, 'earlier answers must not render until opened');

  details.open = true;
  details.dispatchToggle();
  const earlier = details.querySelectorAll('article');
  assert.equal(earlier.length, 2);
  assert.match(earlier[0].textContent, /Old Q2/);
  assert.match(earlier[1].textContent, /Old Q1/);
});
