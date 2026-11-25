// Extra Node tests for Design 03 (03-latex-kandinsky-parity.md) cases not covered by math_parity.test.mjs.
import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');
const katex = createRequire(import.meta.url)(path.join(crateRoot, 'vendor', 'katex', 'katex.min.js'));

const mathMod = await import(pathToFileURL(path.join(assetsDir, 'math.js')).href);
const katexMod = await import(pathToFileURL(path.join(assetsDir, 'math_katex.js')).href);
const {
  latexDocumentToRenderedHtml, markdownToHtml, markdownToRenderedHtml,
  mathTextToHtml, resetMathMacros,
} = mathMod;
const { createMathState, getDefaultMathState } = katexMod;

function render(md, opts = {}) {
  resetMathMacros();
  return markdownToRenderedHtml(md, { katex, ...opts });
}

const countInline = (h) => (h.match(/class="katex"/g) || []).length - (h.match(/class="katex-display"/g) || []).length;
const countDisplay = (h) => (h.match(/class="math-display(?:\s|")/g) || []).length;
const visibleText = (h) => h
  .replace(/<pre\b[^>]*>[\s\S]*?<\/pre>/gi, '')
  .replace(/<code\b[^>]*>[\s\S]*?<\/code>/gi, '')
  .replace(/<[^>]+>/g, '')
  .replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&quot;', '"')
  .replaceAll('&#39;', "'").replaceAll('&amp;', '&');

test('Design 03 §2: all nine seeded Kandinsky shortcut macros render with no error', () => {
  const html = render('$\\R \\oplus \\C \\oplus \\N \\oplus \\Z$, $\\ket{\\psi}$, $\\bra{\\phi}$, $\\braket{a|b}$, $\\Tr(A)$, $\\mbox{ok}$');
  assert.equal(countInline(html), 6);
  assert.ok(!html.includes('katex-error') && !html.includes('var(--status-err)'));
});

test('Design 03 §2: inline $...$ crosses a single soft newline and respects full backslash-run parity', () => {
  const softBreak = render('Sum $a +\nb$ stays inline.');
  assert.equal(countInline(softBreak), 1);
  assert.ok(!softBreak.includes('katex-error'));

  // Even backslash run (\\) before opening $ does NOT escape the opener.
  const evenOpen = render('Break \\\\$x^2$ renders math.');
  assert.equal(countInline(evenOpen), 1);

  // Odd backslash run (\\\) before $ DOES escape the $ into literal text.
  const oddOpen = render('Literal \\\\\\$x^2\\$ stays prose.');
  assert.equal(countInline(oddOpen), 0);

  // Escaped \$ inside inline $...$ does not close the formula prematurely.
  const innerEscaped = render('Cost formula $a + \\$ + b$ renders as one inline span.');
  assert.equal(countInline(innerEscaped), 1);
  assert.ok(!innerEscaped.includes('katex-error'));
});

test('Design 03 §2: punctuation glue wraps up to 4 trailing chars, skips Markdown link ](, and omits glue in markdownToHtml', () => {
  const multiGlue = render('Quote $(x + y)$.)” next.');
  assert.equal((multiGlue.match(/class="math-glue"/g) || []).length, 1);
  assert.ok(multiGlue.includes('.)”</span>'));

  const mdLink = render('Read [section $x^2$](https://example.com/paper) now.');
  assert.match(mdLink, /<a href="https:\/\/example\.com\/paper"[^>]*>section <span class="katex">/);
  assert.equal((mdLink.match(/class="math-glue"/g) || []).length, 0);

  const sourceHtml = markdownToHtml('Cases $a$, then $b$.');
  assert.ok(!sourceHtml.includes('math-glue'));
});

test('Design 03 §2 & WP4: \\[...\\] with \\label, self-tagged \\tag{X} without \\label, and numbering after unclosed $$', () => {
  const bracketLabel = render('\\[\na + b = c \\label{eq-bracket}\n\\]\n\nRef \\eqref{eq-bracket}.');
  assert.equal(countDisplay(bracketLabel), 1);
  assert.ok(bracketLabel.includes('data-eq-id="eq-bracket"'));
  assert.match(bracketLabel, /<a class="eqref-link" data-eq-ref="eq-bracket">\(1\)<\/a>/);

  const tagOnlyTarget = render('$$ E = mc^2 \\tag{A.1} $$\n\nSee \\eqref{A.1}.');
  assert.equal(countDisplay(tagOnlyTarget), 1);
  assert.ok(tagOnlyTarget.includes('data-eq-id="A.1"'));
  assert.match(tagOnlyTarget, /<a class="eqref-link" data-eq-ref="A\.1">\(A\.1\)<\/a>/);

  const afterUnclosed = render('$$\nunclosed = 1\n\n## Section\n\n$$\nvalid = 2 \\label{eq-after}\n$$\n\nRef \\eqref{eq-after}.');
  assert.equal(countDisplay(afterUnclosed), 1);
  assert.match(afterUnclosed, /<a class="eqref-link" data-eq-ref="eq-after">\(1\)<\/a>/);
});

test('Design 03 §2 & WP4: multline, gather, alignat, flalign, and mid-sentence numbered environments', () => {
  const envs = render([
    '\\begin{multline}\na + b \\\\\n+ c + d \\label{eq-ml}\n\\end{multline}',
    '',
    '\\begin{gather}\nu = 1 \\label{eq-g1} \\\\\nv = 2 \\label{eq-g2}\n\\end{gather}',
    '',
    '\\begin{alignat}{2}\na &= b &\\quad c &= d \\label{eq-at1} \\\\\ne &= f &\\quad g &= h \\label{eq-at2}\n\\end{alignat}',
    '',
    '\\begin{flalign}\np &= q \\label{eq-fl1} \\\\\nr &= s \\label{eq-fl2}\n\\end{flalign}',
    '',
    'Refs \\eqref{eq-ml}, \\eqref{eq-g1}, \\eqref{eq-g2}, \\eqref{eq-at1}, \\eqref{eq-at2}, \\eqref{eq-fl1}, \\eqref{eq-fl2}.',
  ].join('\n'));
  assert.ok(!envs.includes('katex-error') && !envs.includes('eqn-num'));
  assert.equal(countDisplay(envs), 4);
  for (let n = 1; n <= 7; n++) {
    assert.ok(visibleText(envs).includes(`(${n})`), `Expected tag (${n}) in output`);
  }

  const midSentEq = render('In prose \\begin{equation} x = 1 \\label{eq-mid-s} \\end{equation} holds, see \\eqref{eq-mid-s}.');
  assert.match(midSentEq, /<span class="math-display" data-eq-id="eq-mid-s"/);
  assert.match(midSentEq, /<a class="eqref-link" data-eq-ref="eq-mid-s">\(1\)<\/a>/);

  const inlineAlignat = mathTextToHtml('$\\begin{alignat}{2} a &= b & c &= d \\end{alignat}$', { inlineOnly: true, katex });
  assert.equal(countInline(inlineAlignat), 1);
  assert.ok(!inlineAlignat.includes('katex-error'));
});

test('Design 03 WP2 & WP4: internal \\@ macros deleted after render, 500-entry FIFO cache cap, and TeX preview \\eqref', () => {
  resetMathMacros();
  const state = getDefaultMathState();
  state.macros['\\@leaked'] = '1';
  markdownToRenderedHtml('$x + 1$', { katex });
  assert.equal(Object.prototype.hasOwnProperty.call(state.macros, '\\@leaked'), false);

  const localState = createMathState();
  for (let i = 0; i < 510; i++) {
    markdownToRenderedHtml(`$x_{${i}}$`, { katex, state: localState });
  }
  assert.equal(localState.cache.size, 500);
  assert.equal([...localState.cache.keys()].some((k) => k.includes('x_{0}')), false, 'oldest entry evicted FIFO');
  assert.equal([...localState.cache.keys()].some((k) => k.includes('x_{509}')), true, 'newest entry retained');

  const texDoc = latexDocumentToRenderedHtml([
    '\\documentclass{article}',
    '\\DeclareMathOperator{\\diag}{diag}',
    '\\begin{document}',
    '\\begin{equation}\\diag(a, b) = M \\label{eq:diag}\\end{equation}',
    'As proven in \\eqref{eq:diag}.',
    '\\end{document}',
  ].join('\n'), { katex });
  assert.ok(!texDoc.includes('katex-error') && !texDoc.includes('var(--status-err)'));
  assert.ok(texDoc.includes('data-eq-id="eq:diag"'));
  assert.match(texDoc, /<a class="eqref-link" data-eq-ref="eq:diag">\(1\)<\/a>/);
});
