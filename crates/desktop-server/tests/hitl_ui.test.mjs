import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

function mockDom() {
  const makeEl = (tag = 'div') => {
    const classes = new Set();
    const listeners = {};
    const el = {
      tagName: String(tag).toUpperCase(),
      className: '',
      dataset: {},
      children: [],
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
      getAttribute: (k) => el.attributes[k] ?? null,
      appendChild: (c) => { el.children.push(c); return c; },
      addEventListener: (ev, fn) => { (listeners[ev] ||= []).push(fn); },
      querySelector: () => null,
      querySelectorAll: () => [],
      focus: () => {},
    };
    return el;
  };
  return {
    createElement: (tag) => makeEl(tag),
  };
}

async function importAsset(name, replacements = {}) {
  let code = fs.readFileSync(path.join(assetsDir, name), 'utf8');
  for (const [k, v] of Object.entries(replacements)) {
    code = code.replaceAll(k, v);
  }
  return import(`data:text/javascript;base64,${Buffer.from(code, 'utf8').toString('base64')}`);
}

test('Stage 14/15 UI contracts: interactive rigor, session conventions, CLI steer guard, delegation & honesty cards', async () => {
  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const apiJs = fs.readFileSync(path.join(assetsDir, 'api.js'), 'utf8');
  const workbenchJs = fs.readFileSync(path.join(assetsDir, 'workbench.js'), 'utf8');
  const composerJs = fs.readFileSync(path.join(assetsDir, 'composer.js'), 'utf8');
  const appJs = fs.readFileSync(path.join(assetsDir, 'app.js'), 'utf8');
  const sourcesJs = fs.readFileSync(path.join(assetsDir, 'canvas_sources.js'), 'utf8');
  const streamJs = fs.readFileSync(path.join(assetsDir, 'stream.js'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');

  assert.match(indexHtml, /data-rigor-preset="interactive"/);
  assert.match(indexHtml, /id="pause-explore-chip"/);
  assert.match(indexHtml, /<select id="subagent-rigor-select"[\s\S]*?<option value="interactive">Interactive<\/option>/);
  assert.match(apiJs, /export function setSessionConventions\(/);
  assert.match(apiJs, /\/conventions`/);
  assert.match(workbenchJs, /api\.setSessionConventions\(sid,\s*trimmed\)/);
  assert.match(workbenchJs, /worker_input_tokens/);
  assert.match(workbenchJs, /delegation_count/);
  assert.match(workbenchJs, /honesty_warnings/);
  assert.match(composerJs, /CLI_ROUTES\s*=\s*new Set\(/);
  assert.match(composerJs, /export function setComposerRoute\(/);
  assert.match(composerJs, /defaultSendHtml/);
  assert.match(appJs, /getActiveSessionId:\s*\(\)\s*=>\s*state\.activeSessionId/);
  assert.match(appJs, /setComposerRoute\(/);
  assert.match(sourcesJs, /ansText\.length > 1500/);
  assert.match(streamJs, /pendingCps\.length === 0/);
  assert.match(fs.readFileSync(path.join(assetsDir, 'hitl.js'), 'utf8'), /branch-summary rich-prose/);
  assert.match(workbenchJs, /wireResponsiveDrawers/);
  assert.match(fs.readFileSync(path.join(assetsDir, 'layout.css'), 'utf8'), /@media \(max-width:\s*780px\)\s*\{\s*#app-shell\s*\{\s*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  assert.ok(assetsRs.includes('"/assets/hitl.js"'));

  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const srcLinksCode = fs.readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  const srcLinksDataUrl = JSON.stringify(`data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`);
  const traceMod = await importAsset('trace_cards.js', {
    "'/assets/math.js'": mathUrl,
    "'/assets/source_links.js'": srcLinksDataUrl,
  });

  const apiStubDataUrl = JSON.stringify(`data:text/javascript;base64,${Buffer.from('export async function answerCheckpoint() { return { ok: true }; }', 'utf8').toString('base64')}`);
  const hitlMod = await importAsset('hitl.js', {
    "'/assets/api.js'": apiStubDataUrl,
    "'/assets/math.js'": mathUrl,
  });

  const prevDoc = globalThis.document;
  try {
    globalThis.document = mockDom();

    const expCard = traceMod.buildStructuredTraceCard({
      call_id: 'exp-1',
      tool: 'delegate_explore',
      status: 'ok',
      args: { problem: 'Compare Chern invariant derivations', branches: [{ approach: 'Berry' }, { approach: 'Kitaev <script>x</script>' }] },
      output: JSON.stringify({
        model: 'claude-haiku-4-5',
        branch_count: 2,
        turns: 4,
        input_tokens: 120,
        output_tokens: 45,
        branches: [
          { approach: 'Berry', ok: true, summary: 'Converged cleanly' },
          { approach: 'Kitaev', ok: false, error: 'Missing <script>y</script>' },
        ],
      }),
    });
    assert.match(expCard.innerHTML, /Parallel Explorer \(Ultramath\)/);
    assert.match(expCard.innerHTML, /2 branches/);
    assert.doesNotMatch(expCard.innerHTML, /<script>/i);

    const leanCard = traceMod.buildStructuredTraceCard({
      call_id: 'lean-1',
      tool: 'lean_check',
      status: 'ok',
      args: { path: 'Main.lean' },
      output: JSON.stringify({ status: 'verified', axioms: ['propext'], diagnostics: [] }),
    });
    assert.match(leanCard.innerHTML, /Lean 4 Checker/);
    assert.match(leanCard.innerHTML, /axioms: propext/);

    const hCard = traceMod.buildHonestyNoticeCard({
      passed: false,
      summary: 'Overclaim detected',
      findings: [{ severity: 'warning', claim: 'q.e.d.', reason: 'Symbolic step not verified <img src=x onerror=alert(1)>' }],
    });
    assert.ok(hCard);
    assert.match(hCard.innerHTML, /Honesty Linter/);
    assert.doesNotMatch(hCard.innerHTML, /<img\b/i);

    const stream = mockDom().createElement('section');
    hitlMod.appendSteerBadge(stream, 'Use gauge <script>alert(1)</script>');
    const inner = stream.children[0];
    assert.equal(inner.children[0].className, 'steer-badge-row');
    assert.doesNotMatch(inner.children[0].innerHTML, /<script>/i);
  } finally {
    globalThis.document = prevDoc;
  }
});
