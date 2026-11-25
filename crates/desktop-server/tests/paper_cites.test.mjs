import assert from 'node:assert/strict';
import test from 'node:test';

const { linkLineCitations } = await import(
  new URL('../assets/paper_cites.js', import.meta.url).href
);

function makeText(value) {
  return {
    nodeType: 3,
    nodeValue: String(value ?? ''),
    get textContent() { return this.nodeValue; },
    parentNode: null,
    replaceWith(...nodes) {
      const parent = this.parentNode;
      if (!parent) return;
      const idx = parent.childNodes.indexOf(this);
      if (idx < 0) return;
      for (const n of nodes) n.parentNode = parent;
      parent.childNodes.splice(idx, 1, ...nodes);
    },
  };
}

function makeEl(tag, className = '') {
  const listeners = new Map();
  return {
    nodeType: 1,
    tagName: tag.toUpperCase(),
    className,
    dataset: {},
    type: '',
    title: '',
    childNodes: [],
    parentNode: null,
    get children() { return this.childNodes.filter(n => n.nodeType === 1); },
    get textContent() { return this.childNodes.map(n => n.textContent ?? '').join(''); },
    set textContent(v) {
      const t = makeText(v);
      t.parentNode = this;
      this.childNodes = [t];
    },
    append(...nodes) {
      for (const n of nodes) {
        const child = typeof n === 'string' ? makeText(n) : n;
        child.parentNode = this;
        this.childNodes.push(child);
      }
    },
    addEventListener(name, fn) { listeners.set(name, fn); },
    click() { listeners.get('click')?.({ currentTarget: this }); },
    querySelectorAll(sel) {
      const out = [];
      const match = n => sel.startsWith('.')
        ? String(n.className || '').split(/\s+/).includes(sel.slice(1))
        : n.tagName === sel.toUpperCase();
      const visit = cur => {
        for (const c of cur.childNodes) {
          if (c.nodeType === 1) {
            if (match(c)) out.push(c);
            visit(c);
          }
        }
      };
      visit(this);
      return out;
    },
  };
}

function withDom(t) {
  const prev = { document: globalThis.document };
  globalThis.document = {
    createElement: tag => makeEl(tag),
    createTextNode: text => makeText(text),
  };
  t.after(() => Object.assign(globalThis, prev));
}

test('parses [L12], [L3–5], and [L3-5, L9] and opens exact line ranges on click', t => {
  withDom(t);
  const root = makeEl('div');
  const p = makeEl('p');
  p.append('Single [L12], en-dash [L3–5], and multi [L3-5, L9].');
  root.append(p);

  const opened = [];
  linkLineCitations(
    root,
    [
      { start: 12, end: 12, read: true },
      { start: 3, end: 5, read: true },
      { start: 9, end: 9, read: true },
    ],
    (start, end) => opened.push([start, end]),
  );

  const buttons = root.querySelectorAll('.line-cite');
  assert.deepEqual(buttons.map(b => b.textContent), ['L12', 'L3–5', 'L3-5', 'L9']);
  assert.equal(p.textContent, 'Single [L12], en-dash [L3–5], and multi [L3-5, L9].');
  for (const btn of buttons) {
    assert.equal(btn.type, 'button');
    assert.equal(
      btn.title,
      'Opens these lines with context. The model chose this citation; the app has not checked that it supports the sentence.',
    );
    btn.click();
  }
  assert.deepEqual(opened, [[12, 12], [3, 5], [3, 5], [9, 9]]);
});

test('leaves unread and reversed ranges as plain text', t => {
  withDom(t);
  const root = makeEl('div');
  const p = makeEl('p');
  p.append('Read [L4], unread [L99], reversed [L8-3], mixed [L4, L99], zero [L0].');
  root.append(p);

  linkLineCitations(
    root,
    [
      { start: 4, end: 4, read: true },
      { start: 99, end: 99, read: false },
      { start: 8, end: 3, read: true },
      { start: 0, end: 0, read: true },
    ],
    () => {},
  );

  const buttons = root.querySelectorAll('.line-cite');
  assert.deepEqual(buttons.map(b => b.textContent), ['L4', 'L4']);
  assert.equal(p.textContent, 'Read [L4], unread [L99], reversed [L8-3], mixed [L4, L99], zero [L0].');
});

test('skips code, pre, and .katex subtrees', t => {
  withDom(t);
  const root = makeEl('div');
  const code = makeEl('code'); code.append('[L12]');
  const pre = makeEl('pre'); pre.append('[L12]');
  const katex = makeEl('span', 'katex'); katex.append('[L12]');
  const prose = makeEl('p'); prose.append('Prose [L12].');
  root.append(code, pre, katex, prose);

  linkLineCitations(root, [{ start: 12, end: 12, read: true }], () => {});

  assert.equal(code.querySelectorAll('.line-cite').length, 0);
  assert.equal(pre.querySelectorAll('.line-cite').length, 0);
  assert.equal(katex.querySelectorAll('.line-cite').length, 0);
  assert.equal(prose.querySelectorAll('.line-cite').length, 1);
});

test('surrounding HTML stays plain text and never creates elements', t => {
  withDom(t);
  const root = makeEl('div');
  const p = makeEl('p');
  p.append('<img src=x onerror=1> [L7] <script>alert(1)</script>');
  root.append(p);

  linkLineCitations(root, [{ start: 7, end: 7, read: true }], () => {});

  assert.equal(root.querySelectorAll('img').length, 0);
  assert.equal(root.querySelectorAll('script').length, 0);
  assert.equal(root.querySelectorAll('.line-cite').length, 1);
  assert.equal(p.textContent, '<img src=x onerror=1> [L7] <script>alert(1)</script>');
});
