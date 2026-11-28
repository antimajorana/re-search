import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const crateRoot = path.resolve(__dirname, '..');
export const assetsDir = path.join(crateRoot, 'assets');
export const mathAssetUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);

export function mockDom(ids = []) {
  const makeEl = (tag = 'div') => {
    const classes = new Set();
    const listeners = {};
    const el = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      children: [],
      get childElementCount() { return this.children.length; },
      open: false,
      attributes: {},
      style: {},
      value: '',
      textContent: '',
      _html: '',
      set innerHTML(v) {
        this._html = String(v);
        if (!v) this.children = [];
      },
      get innerHTML() {
        return this._html;
      },
      classList: {
        add: (...cs) => cs.forEach((c) => classes.add(c)),
        remove: (...cs) => cs.forEach((c) => classes.delete(c)),
        toggle: (c, f) => (f === undefined ? (classes.has(c) ? classes.delete(c) : classes.add(c)) : (f ? classes.add(c) : classes.delete(c))),
        contains: (c) => classes.has(c),
      },
      setAttribute: (k, v) => { el.attributes[k] = String(v); },
      removeAttribute: (k) => { delete el.attributes[k]; },
      getAttribute: (k) => el.attributes[k] ?? null,
      appendChild: (c) => { el.children.push(c); return c; },
      append: (...children) => el.children.push(...children),
      replaceChildren: (...children) => { el.children = children; },
      addEventListener: (ev, fn) => { (listeners[ev] ||= []).push(fn); },
      click: () => Promise.all((listeners.click || []).map((fn) => fn())),
      dispatch: (name, event = { preventDefault() {} }) => Promise.all((listeners[name] || []).map((fn) => fn(event))),
      showModal() { this.open = true; },
      close() { this.open = false; return this.dispatch('close'); },
      getClientRects() { return this.hidden ? [] : [{}]; },
      closest: () => null,
      contains: (node) => el === node || el.children.some((child) => child.contains(node)),
      querySelector: () => null,
      querySelectorAll: () => [],
      focus: () => {},
    };
    return el;
  };
  const elements = new Map(ids.map((id) => [id, makeEl()]));
  return {
    createElement: (tag) => makeEl(tag),
    createElementNS: (_ns, tag) => makeEl(tag),
    getElementById: (id) => elements.get(id) || null,
    addEventListener: () => {},
    querySelector: () => null,
    querySelectorAll: () => [],
  };
}

export async function importAsset(name, replacements = {}) {
  let code = fs.readFileSync(path.join(assetsDir, name), 'utf8');
  for (const [k, v] of Object.entries(replacements)) {
    code = code.replaceAll(k, v);
  }
  return import(`data:text/javascript;base64,${Buffer.from(code, 'utf8').toString('base64')}`);
}

export function assetStub(code) {
  return JSON.stringify(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
}
