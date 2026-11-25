// Failing proof tests for UI audit findings in LaTeX & Markdown modules (F1-F7, F14, F15, F17).
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');
const katex = createRequire(import.meta.url)(path.join(crateRoot, 'vendor', 'katex', 'katex.min.js'));

const mathMod = await import(pathToFileURL(path.join(assetsDir, 'math.js')).href);
const {
  latexDocumentToRenderedHtml, markdownToRenderedHtml, mathTextToHtml, resetMathMacros,
} = mathMod;

function render(md, opts = {}) {
  resetMathMacros();
  return markdownToRenderedHtml(md, { katex, ...opts });
}

const countDisplay = (h) => (h.match(/class="math-display(?:\s|")/g) || []).length;
const visibleText = (h) => h
  .replace(/<pre\b[^>]*>[\s\S]*?<\/pre>/gi, '')
  .replace(/<code\b[^>]*>[\s\S]*?<\/code>/gi, '')
  .replace(/<[^>]+>/g, '')
  .replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&quot;', '"')
  .replaceAll('&#39;', "'").replaceAll('&amp;', '&');

test('F1 [critical] markdown.js:284-301: mid-line $$ opener closed at line start must not phase-shift subsequent displays', () => {
  const html = render([
    'We obtain the Hamiltonian: $$',
    'H = p^2 / 2m',
    '$$',
    'and therefore the energy is:',
    '$$',
    'E \\ge 0',
    '$$',
  ].join('\n'));

  assert.equal(countDisplay(html), 2, 'Expected both H = p^2 / 2m and E >= 0 to render as displays');
  assert.ok(!html.includes('data-tex="and therefore the energy is:"'), 'Prose between equations was rendered as display math');
  assert.ok(!visibleText(html).includes('$$'), `Unrendered $$ delimiter leaked: ${visibleText(html)}`);
});

test('F2 [critical] math_katex.js:99-105: repairTextCmd must not corrupt subscripts and exponents inside nested $...$ in \\text{...}', () => {
  const html = render('$$\n\\begin{cases} 1 & \\text{if $x^2 < 1$ and $x_i > 0$} \\\\ 0 & \\text{otherwise} \\end{cases}\n$$');
  const txt = visibleText(html);
  assert.ok(!txt.includes('ˆ'), `Exponent ^2 inside \\text{$x^2$} was corrupted into circumflex accent: ${txt}`);
  assert.ok(!txt.includes('x_i'), `Subscript _i inside \\text{$x_i$} was corrupted into literal underscore: ${txt}`);
});

test('F3 [critical] tex_preview.js:48-87: preamble \\newcommand with optional default arg must not misbind arguments', () => {
  const html = latexDocumentToRenderedHtml([
    '\\documentclass{article}',
    '\\newcommand{\\norm}[2][2]{\\|#2\\|_{#1}}',
    '\\begin{document}',
    'We bound $\\norm{x} + y$.',
    '\\end{document}',
  ].join('\n'), { katex });
  const txt = visibleText(html).replace(/\u200b/g, '');
  assert.ok(!txt.includes('∥+∥'), `\\norm{x} + y consumed "+" as #2 and rendered wrong formula: ${txt}`);
  assert.ok(txt.includes('∥x∥2+y') || txt.includes('∥x∥2 + y'), `Expected ||x||_2 + y, got: ${txt}`);
});

test('F4 [critical] markdown.js:294-297: trailing text after mid-line display closer must become a paragraph, not blockquote or list', () => {
  const gtHtml = render('$$\na = b\n$$ > 0 for all inputs.');
  assert.ok(!gtHtml.includes('<blockquote>'), `Leading ">" in trailing prose became <blockquote>, dropping inequality: ${gtHtml}`);
  assert.match(gtHtml, /<p>&gt; 0 for all inputs\.<\/p>/);

  const minusHtml = render('$$\nx = 1\n$$ - 2 = -1.');
  assert.ok(!minusHtml.includes('<ul>'), `Leading "-" in trailing prose became <ul>, dropping minus sign: ${minusHtml}`);
  assert.match(minusHtml, /<p>- 2 = -1\.<\/p>/);
});

test('F5 [major] math_equations.js:51-78,131,218: TeX % comments in numbered equation and align must not swallow \\tag or \\\\', () => {
  const eqHtml = render('\\begin{equation}\nE = mc^2 % rest energy\n\\end{equation}\n\nSee \\eqref{1}.');
  assert.equal((eqHtml.match(/class="katex-tag"/g) || []).length, 1, 'Tag (1) was swallowed by trailing % comment');

  const alignHtml = render('\\begin{align}\na &= b % first row\n\\\\\nc &= d\n\\end{align}');
  const txt = visibleText(alignHtml);
  assert.ok(txt.includes('(1)') && txt.includes('(2)'), `Row 1 tag and \\\\ break were swallowed by % comment: ${txt}`);
});

test('F6 [major] math_equations.js:173-227: \\begin{subequations} with multiple equations must keep distinct equations and numbers', () => {
  const html = render([
    '\\begin{subequations}',
    '\\begin{equation} a = 1 \\label{eq-a} \\end{equation}',
    '\\begin{equation} b = 2 \\label{eq-b} \\end{equation}',
    '\\end{subequations}',
    '',
    'Refs \\eqref{eq-a} and \\eqref{eq-b}.',
  ].join('\n'));
  const refs = [...html.matchAll(/<a class="eqref-link"[^>]*>([^<]+)<\/a>/g)].map((m) => m[1]);
  assert.equal(refs.length, 2);
  assert.notEqual(refs[0], refs[1], `Distinct equations inside subequations received identical ref ${refs[0]}`);
  assert.ok(!visibleText(html).includes('1b'), `Two equations were concatenated into "a = 1b = 2": ${visibleText(html)}`);
});

test('F7 [major] math_equations.js:6 & math_katex.js:109: \\tag{...} with nested braces must not cause Multiple \\tag or inline error', () => {
  const dispHtml = render('\\begin{equation}\nE = mc^2 \\tag{\\mathrm{I}} \\label{eq-rom}\n\\end{equation}\nSee \\eqref{eq-rom}.');
  assert.ok(!dispHtml.includes('katex-error'), `Nested braces in \\tag{\\mathrm{I}} caused KaTeX error: ${dispHtml}`);

  const inlineHtml = mathTextToHtml('Step $E = mc^2 \\tag{1\\text{a}}$', { inlineOnly: true, katex });
  assert.ok(!inlineHtml.includes('katex-error'), `Nested braces in inlineOnly \\tag{1\\text{a}} caused KaTeX error: ${inlineHtml}`);
});

test('F14 [minor] markdown.js:14-19: stripComments must preserve HTML comments inside blockquote/list fences and double-backtick code', () => {
  const bqFence = render('> ```html\n> <!-- keep in blockquote -->\n> ```');
  assert.ok(bqFence.includes('&lt;!-- keep in blockquote --&gt;'), `Comment stripped inside blockquote fence: ${bqFence}`);

  const listFence = render('10. Step:\n    ```html\n    <!-- keep in list -->\n    ```');
  assert.ok(listFence.includes('&lt;!-- keep in list --&gt;'), `Comment stripped inside 4-space list fence: ${listFence}`);

  const dblTick = render('`` `a` <!-- keep in inline code --> ``');
  assert.ok(dblTick.includes('&lt;!-- keep in inline code --&gt;'), `Comment stripped inside double-backtick code: ${dblTick}`);
});

test('F15 [minor] math_katex.js:75-97,220-235: \\newcommand on text-mode built-in \\ae and streaming cache hit on pass 2', () => {
  const aeHtml = render('$\\newcommand{\\ae}{\\alpha} \\ae$');
  assert.ok(!aeHtml.includes('katex-error'), `\\newcommand{\\ae} failed to rewrite to \\renewcommand: ${aeHtml}`);

  resetMathMacros();
  let calls = 0;
  const spyKatex = { renderToString: (...a) => { calls += 1; return katex.renderToString(...a); } };
  const md = '$\\newcommand{\\foo}{x^2}$ then $\\foo$';
  markdownToRenderedHtml(md, { katex: spyKatex });
  const pass1Calls = calls;
  markdownToRenderedHtml(md, { katex: spyKatex });
  assert.equal(calls, pass1Calls, `Second streaming pass re-invoked KaTeX (${pass1Calls} -> ${calls}) instead of hitting cache`);
});

test('F17 [minor] investigation.css:14 & markdown.js:100-101: Answer card .rich-prose paragraphs must not double <br>\\n line breaks', () => {
  const invCss = fs.readFileSync(path.join(assetsDir, 'investigation.css'), 'utf8');
  const html = render('First line  \nSecond line');
  const hasNewlineAfterBr = html.includes('<br>\n');
  const overridesPreLine = /\.investigation-inner\s+\.rich-prose\s+p\s*\{[^}]*white-space\s*:\s*normal/.test(invCss)
    || /\.investigation-answer-body\s+p\s*\{[^}]*white-space\s*:\s*normal/.test(invCss);
  assert.ok(
    !hasNewlineAfterBr || overridesPreLine,
    'markdown.js emits "<br>\\n" while investigation.css sets ".investigation-inner p { white-space: pre-line }" with no .rich-prose override',
  );
});
