import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');
const vendorKatexPath = path.join(crateRoot, 'vendor', 'katex', 'katex.min.js');

const require = createRequire(import.meta.url);
const katex = require(vendorKatexPath);

const mathMod = await import(pathToFileURL(path.join(assetsDir, 'math.js')).href);
const {
  clearMathCache, displayMathSource, katexOptions, latexDocumentToRenderedHtml,
  markdownToHtml, markdownToRenderedHtml, renderFormulaHtml, resetMathMacros,
} = mathMod;

function render(md, opts = {}) {
  return markdownToRenderedHtml(md, { katex, ...opts });
}

function countInline(html) {
  const allKatex = (html.match(/class="katex"/g) || []).length;
  const displays = (html.match(/class="katex-display"/g) || []).length;
  return allKatex - displays;
}

function countDisplay(html) {
  return (html.match(/class="math-display(?:\s|")/g) || []).length;
}

function visibleText(html) {
  return html
    .replace(/<pre\b[^>]*>[\s\S]*?<\/pre>/gi, '')
    .replace(/<code\b[^>]*>[\s\S]*?<\/code>/gi, '')
    .replace(/<[^>]+>/g, '')
    .replaceAll('&lt;', '<')
    .replaceAll('&gt;', '>')
    .replaceAll('&quot;', '"')
    .replaceAll('&#39;', "'")
    .replaceAll('&amp;', '&');
}

function assertCleanDelims(html, { allowDollar = false, allowParen = false, allowBracket = false, allowBegin = false } = {}) {
  const text = visibleText(html);
  if (!allowDollar) assert.ok(!text.includes('$'), `Unexpected "$" in visible text: ${text}`);
  if (!allowParen) assert.ok(!text.includes('\\('), `Unexpected "\\(" in visible text: ${text}`);
  if (!allowBracket) assert.ok(!text.includes('\\['), `Unexpected "\\[" in visible text: ${text}`);
  if (!allowBegin) assert.ok(!text.includes('\\begin{'), `Unexpected "\\begin{" in visible text: ${text}`);
}

function assertHtmlOnlyNoEqnNum(html) {
  assert.ok(!html.includes('<math'), `Unexpected <math in output: ${html}`);
  assert.ok(!html.includes('<annotation'), `Unexpected <annotation in output: ${html}`);
  assert.ok(!html.includes('eqn-num'), `Unexpected eqn-num in output: ${html}`);
}

// --- WP1 & WP2 parity cases ---

test('Case 62 (WP1): markdownToHtml source form stays exact', () => {
  assert.equal(
    markdownToHtml('Costs $5 and $10, while $\\mu = 0$ is exact.'),
    '<p>Costs $5 and $10, while \\(\\mu = 0\\) is exact.</p>',
  );
});

test('Case 1 (WP2): inline math with subscripts renders cleanly', () => {
  resetMathMacros();
  const html = render('Where $\\phi_{0}$ and $\\phi_{e}$ are orbitals.');
  assert.equal(countInline(html), 2);
  assert.equal(countDisplay(html), 0);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
  assertHtmlOnlyNoEqnNum(html);
});

test('Case 10 (WP2): escaped dollar stays literal beside inline math', () => {
  resetMathMacros();
  const html = render('Price \\$5 and $x$');
  assert.equal(countInline(html), 1);
  assert.ok(visibleText(html).includes('Price $5 and '));
  assertHtmlOnlyNoEqnNum(html);
});

test('Case 11 (WP2): escaped double dollars stay literal text in single pass', () => {
  resetMathMacros();
  const html = render('Literal \\$\\$x^2\\$\\$ should stay text.');
  assert.equal(countInline(html), 0);
  assert.equal(countDisplay(html), 0);
  assert.equal(visibleText(html).trim(), 'Literal $$x^2$$ should stay text.');
});

test('Case 12 (WP2): inline code protects math delimiters', () => {
  resetMathMacros();
  const html = render('`$x$` and $y$');
  assert.match(html, /<code>\$x\$<\/code>/);
  assert.equal(countInline(html), 1);
  assertCleanDelims(html);
});

test('Case 13 (WP2): fenced bash block stays untouched before inline math', () => {
  resetMathMacros();
  const md = '```bash\necho $HOME $((1+2)) \\(no\\)\n```\n\nthen $x$.';
  const html = render(md);
  assert.match(html, /<pre><code class="lang-bash">echo \$HOME \$\(\(1\+2\)\) \\\(no\\\)<\/code><\/pre>/);
  assert.equal(countInline(html), 1);
  assertCleanDelims(html);
});

test('Case 44a & 44b (WP2): \\newcommand over built-in \\div rewrites cleanly across renders and cache clears', () => {
  resetMathMacros();
  const md = '$\\newcommand{\\div}[1]{\\nabla \\cdot #1}$ then $\\div{E}$';
  const first = render(md);
  const second = render(md);
  assert.ok(!first.includes('katex-error'), `First render errored: ${first}`);
  assert.ok(!second.includes('katex-error'), `Second render errored: ${second}`);
  assert.equal(countInline(first), 2);
  assert.equal(second, first);

  // 44b: clear cache only (keep macro table), render again
  clearMathCache();
  const third = render(md);
  assert.ok(!third.includes('katex-error'), `Third render after cache clear errored: ${third}`);
  assert.equal(countInline(third), 2);
});

test('Case 45 & 46 (WP2): \\def persists across formulas and resets on resetMathMacros()', () => {
  resetMathMacros();
  const html = render('$\\def\\zz#1{[#1]}$ and $\\zz{q}$');
  assert.ok(!html.includes('katex-error'));
  assert.ok(!html.includes('var(--status-err)'), `Defined macro \\zz must not render red: ${html}`);
  assert.ok(visibleText(html).includes('[q]'));

  // Case 46: reset clears macro table and cache
  resetMathMacros();
  const afterReset = render('$\\zz{q}$');
  assert.ok(afterReset.includes('var(--status-err)'), `After reset, \\zz should render in error color: ${afterReset}`);
  assert.ok(visibleText(afterReset).includes('\\zz'));
});

test('Case 47 (WP2): \\text{Nb_3Ge} repairs bare underscore inside \\text{}', () => {
  resetMathMacros();
  const html = render('$\\text{Nb_3Ge}$');
  assert.ok(!html.includes('katex-error'));
  assert.ok(visibleText(html).includes('Nb_3Ge'));
  assert.equal(countInline(html), 1);
});

test('Case 48 (WP2): seed macros \\mbox, \\Tr, \\C, \\ket render without error', () => {
  resetMathMacros();
  const html = render('$k_{\\mbox{\\tiny B}}T$ and $\\Tr\\rho$ and $\\C^n$ and $\\ket{\\psi}$');
  assert.equal(countInline(html), 4);
  assert.ok(!html.includes('katex-error'));
  assert.ok(!html.includes('var(--status-err)'), `Seed macros must not render red: ${html}`);
  assertCleanDelims(html);
});

test('Case 49 (WP2): Unicode Greek and relations render with strict: ignore', () => {
  resetMathMacros();
  const html = render('$α + β ≤ γ$');
  assert.equal(countInline(html), 1);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
});

test('Case 50 (WP2): parse error uses katex-error with title and theme error token', () => {
  resetMathMacros();
  const html = render('$x^$');
  assert.equal((html.match(/class="katex-error"/g) || []).length, 1);
  assert.match(html, /title="[^"]+"/);
  assert.ok(html.includes('color:var(--status-err)'));
  assert.ok(!html.includes('#cc0000'));
  assert.ok(visibleText(html).includes('x^'));
});

test('Case 51 (WP2): unknown command renders red without katex-error element', () => {
  resetMathMacros();
  const html = render('$\\dv{f}{x}$');
  assert.equal(countInline(html), 1);
  assert.ok(!html.includes('katex-error'));
  assert.ok(html.includes('color:var(--status-err)'));
  assert.ok(visibleText(html).includes('\\dv'));
});

test('Case 52 (WP2): trust stays off so \\href emits no anchor and renders red', () => {
  resetMathMacros();
  const opts = katexOptions(false);
  assert.equal(opts.trust, undefined);
  const html = render('$\\href{javascript:alert(1)}{x}$');
  assert.ok(!html.includes('<a'), `\\href must not emit <a>: ${html}`);
  assert.ok(html.includes('color:var(--status-err)'));
});

test('Case 53 (WP2): HTML injection inside malformed math is escaped', () => {
  resetMathMacros();
  const html = render('$<img src=x onerror=1>^$');
  assert.ok(!html.includes('<img'), `Must not emit <img tag: ${html}`);
  assert.ok(html.includes('&lt;img src=x onerror=1&gt;'));
  // Ensure no unescaped onerror attribute exists outside quoted attribute values
  for (const tag of html.match(/<[^>]+>/g) || []) {
    const withoutQuotedAttrs = tag.replace(/="[^"]*"/g, '').replace(/='[^']*'/g, '');
    assert.doesNotMatch(withoutQuotedAttrs, /\bonerror\b/i, `Unescaped onerror outside attribute value in ${tag}`);
  }
});

test('Case 54 (WP2): HTML-only output has no MathML or eqn-num', () => {
  resetMathMacros();
  assertHtmlOnlyNoEqnNum(render('Inline $x^2 + y^2 = 1$'));
  assertHtmlOnlyNoEqnNum(render('$$\nE = mc^2\n$$'));
  assertHtmlOnlyNoEqnNum(render('\\begin{equation}\na = b\n\\end{equation}'));
});

test('Case 61 (WP2): per-formula cache avoids re-rendering on second pass including \\def', () => {
  resetMathMacros();
  let calls = 0;
  const spyKatex = {
    renderToString(...args) {
      calls += 1;
      return katex.renderToString(...args);
    },
  };
  const md = '$\\def\\zz#1{[#1]}$ then $\\zz{q}$ and $\\zz{q}$ and $x^2$';
  const first = markdownToRenderedHtml(md, { katex: spyKatex });
  assert.equal(calls, 3, 'First pass should invoke KaTeX once per distinct formula (3)');
  const second = markdownToRenderedHtml(md, { katex: spyKatex });
  assert.equal(second, first);
  assert.equal(calls, 3, 'Second pass should hit cache for all formulas including \\def');
});

test('WP2 fallback: throwing KaTeX renders code.math-error and missing KaTeX renders span.math-pending', () => {
  resetMathMacros();
  const throwingKatex = { renderToString() { throw new Error('boom'); } };
  const errHtml = renderFormulaHtml({ tex: 'x < y', display: false }, { katex: throwingKatex });
  assert.equal(errHtml, '<code class="math-error">x &lt; y</code>');

  const prevWin = globalThis.window;
  try {
    globalThis.window = {};
    const pendingInline = renderFormulaHtml({ tex: 'a + b', display: false });
    assert.equal(pendingInline, '<span class="math-pending" data-display="0">a + b</span>');
    const pendingDisplay = renderFormulaHtml({ tex: 'a + b', display: true });
    assert.equal(pendingDisplay, '<span class="math-pending" data-display="1">a + b</span>');
  } finally {
    globalThis.window = prevWin;
  }
});

// --- WP3 parity cases ---

test('Case 2 (WP3): inline \\(...\\) renders as inline math', () => {
  resetMathMacros();
  const html = render('see \\(x^2\\) here');
  assert.equal(countInline(html), 1);
  assert.equal(countDisplay(html), 0);
  assertCleanDelims(html);
});

test('Case 3 (WP3): \\(...\\) inside bold and after hyphen renders inline', () => {
  resetMathMacros();
  const html = render('The **background \\(m\\) per unit cell** and background-\\(m\\) line.');
  assert.equal(countInline(html), 2);
  assert.match(html, /<strong>background <span class="katex">[\s\S]*?<\/span> per unit cell<\/strong>/);
  assertCleanDelims(html);
});

test('Case 4 (WP3): $...$ holding a pmatrix environment renders inline', () => {
  resetMathMacros();
  const html = render('Let $M = \\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix}$ be diagonal.');
  assert.equal(countInline(html), 1);
  assert.equal(countDisplay(html), 0);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
});

test('Case 5 (WP3): bare pmatrix mid-sentence renders inline in same paragraph', () => {
  resetMathMacros();
  const html = render('The matrix \\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix} is diagonal.');
  assert.equal(countInline(html), 1);
  assert.equal(countDisplay(html), 0);
  assert.equal((html.match(/<p>/g) || []).length, 1);
  assertCleanDelims(html);
});

test('Case 6 (WP3): bare pmatrix alone on a line renders as display', () => {
  resetMathMacros();
  const html = render('\\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix}');
  assert.equal(countDisplay(html), 1);
  assert.equal(countInline(html), 0);
  assertCleanDelims(html);
});

test('Case 7 (WP3): currency "$5 and $6" produces no math', () => {
  resetMathMacros();
  const html = render('It costs $5 and $6 more');
  assert.equal(countInline(html), 0);
  assert.equal(countDisplay(html), 0);
  assert.equal(visibleText(html).trim(), 'It costs $5 and $6 more');
});

test('Case 8 (WP3): currency guard rejects "$5 for apples ... 10$"', () => {
  resetMathMacros();
  const html = render('I paid $5 for apples and my friend paid 10$ for pears.');
  assert.equal(countInline(html), 0);
  assert.equal(countDisplay(html), 0);
  assert.equal(visibleText(html).trim(), 'I paid $5 for apples and my friend paid 10$ for pears.');
});

test('Case 9 (WP3): "$5-$10" and "$ 5 + 3 $" produce no math', () => {
  resetMathMacros();
  const html = render('between $5-$10, and $ 5 + 3 $.');
  assert.equal(countInline(html), 0);
  assert.equal(countDisplay(html), 0);
  assert.equal(visibleText(html).trim(), 'between $5-$10, and $ 5 + 3 $.');
});

test('Case 14 (WP3): punctuation glue wraps trailing punctuation on inline formulas only', () => {
  resetMathMacros();
  const html = render('Cases $a$, then $b$. And $(c)$; and $d$ alone');
  const glues = [...html.matchAll(/<span class="math-glue">[\s\S]*?<\/span>([.,;:!?)\]\u2019\u201D]+)<\/span>/g)];
  assert.equal((html.match(/class="math-glue"/g) || []).length, 3);
  assert.ok(glues.length <= 3);
  assert.ok(html.includes(',</span>'));
  assert.ok(html.includes('.</span>'));
  assert.ok(html.includes(';</span>'));
  assert.equal(countInline(html), 4);
  assertCleanDelims(html);
});

test('Case 15 (WP3): standalone $$ display between paragraphs', () => {
  resetMathMacros();
  const html = render('Before\n\n$$\nE = mc^2\n$$\n\nAfter');
  assert.equal(countDisplay(html), 1);
  assert.match(html, /^<p>Before<\/p>\n<div class="math-display math-block"[\s\S]*<\/div>\n<p>After<\/p>$/);
  assertCleanDelims(html);
});

test('Case 16 (WP3): blank line inside display math stays one display', () => {
  resetMathMacros();
  const html = render('$$\na = b\n\n+ c\n$$');
  assert.equal(countDisplay(html), 1);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
});

test('Case 17 (WP3): prose after mid-line $$ closer becomes a following paragraph', () => {
  resetMathMacros();
  const html = render('$$\nx = 1\n$$which follows.');
  assert.equal(countDisplay(html), 1);
  assert.match(html, /<\/div>\n<p>which follows\.<\/p>$/);
  assertCleanDelims(html);
});

test('Case 18 (WP3): $$$$ closes one display and opens the next', () => {
  resetMathMacros();
  const html = render('$$\nU = s/V\n$$$$\nH = N^2 s\n$$');
  assert.equal(countDisplay(html), 2);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
});

test('Case 19 (WP3): unclosed $$ stops at ATX heading and stays literal', () => {
  resetMathMacros();
  const html = render('$$\nx = 1\n\n## Next part\n\n$$ z = 2 $$ and $y$');
  assert.match(html, /<p>\$\$ x = 1<\/p>/);
  assert.match(html, /<h2>Next part<\/h2>/);
  assert.equal(countDisplay(html), 1);
  assert.equal(countInline(html), 1);
});

test('Case 20 (WP3): $$ x #1 $$ gives display wrapper with 1 katex-error and 0 katex-display', () => {
  resetMathMacros();
  const html = render('$$ x #1 $$');
  assert.equal(countDisplay(html), 1);
  assert.equal((html.match(/class="katex-error"/g) || []).length, 1);
  assert.equal((html.match(/class="katex-display"/g) || []).length, 0);
  assert.ok(!html.includes('<h1'));
  assert.ok(visibleText(html).includes('x #1'));
});

test('Case 21 (WP3): unclosed mid-paragraph $$ stays literal without eating next paragraph', () => {
  resetMathMacros();
  const html = render('Type $$ to open a display.\n\nSecond paragraph with $x$.');
  assert.equal(countDisplay(html), 0);
  assert.equal(countInline(html), 1);
  assert.match(html, /<p>Type \$\$ to open a display\.<\/p>/);
});

test('Case 22 (WP3): \\[...\\] at line start renders as display block', () => {
  resetMathMacros();
  const html = render('\\[\n\\sum_{n=1}^{\\infty} \\frac{1}{n^2}\n\\]');
  assert.equal(countDisplay(html), 1);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
});

test('Case 23 (WP3): citation brackets \\[1\\] and \\[2, 3\\] are not math and drop backslash', () => {
  resetMathMacros();
  const html = render('As shown in \\[1\\] and \\[2, 3\\].');
  assert.equal(countDisplay(html), 0);
  assert.equal(countInline(html), 0);
  assert.equal(visibleText(html).trim(), 'As shown in [1] and [2, 3].');
  assertCleanDelims(html);
});

test('Case 24 (WP3): \\\\[4pt] is not math, later \\[ y \\] is a display span', () => {
  resetMathMacros();
  const html = render('Row one \\\\[4pt] row two, later \\[ y \\] done.');
  assert.equal(countDisplay(html), 1);
  assert.match(html, /<span class="math-display"/);
  assert.ok(visibleText(html).includes('Row one \\\\[4pt] row two, later '));
  assert.ok(!visibleText(html).includes('\\[ y'));
  assert.ok(!visibleText(html).includes('\\]'));
  assertCleanDelims(html, { allowBracket: true });
});

test('Case 25 (WP3): unclosed \\begin{align} stays literal while later math still renders', () => {
  resetMathMacros();
  const html = render('Use \\begin{align} for alignment, e.g. \\(x^2\\) or $y$.');
  assert.equal(countDisplay(html), 0);
  assert.equal(countInline(html), 2);
  assert.ok(visibleText(html).includes('\\begin{align}'));
  assertCleanDelims(html, { allowBegin: true });
});

test('Case 26 (WP3): display math inside blockquote strips > before TeX', () => {
  resetMathMacros();
  const html = render('> $$\n> a = b\n> $$');
  assert.match(html, /<blockquote>[\s\S]*<div class="math-display math-block"[\s\S]*<\/blockquote>/);
  assert.equal(countDisplay(html), 1);
  assert.ok(!html.includes('data-tex=">'), `> leaked into TeX attribute: ${html}`);
  assert.ok(!html.includes('data-tex="&gt;'), `> leaked into TeX attribute: ${html}`);
  assertCleanDelims(html);
});

test('Case 57 (WP3): displayMathSource preserves mixed prose "$a$ and $b$"', () => {
  resetMathMacros();
  const html = render(displayMathSource('$a$ and $b$'));
  assert.equal(countInline(html), 2);
  assert.equal(countDisplay(html), 0);
  assert.ok(visibleText(html).includes('and'));
  assertCleanDelims(html);
});

test('Case 58 (WP3): TeX preview treats \\\\[2mm] as line break and renders \\[ x = 1 \\] cleanly', () => {
  const tex = 'Line one\\\\[2mm]\nLine two is prose.\n\n\\[ x = 1 \\]\nAfter.';
  const html = latexDocumentToRenderedHtml(tex, { katex });
  assert.equal(countDisplay(html), 1);
  assert.match(html, /Line one<br> Line two is prose\./);
  assert.match(html, /data-tex="x = 1"/);
  assert.match(html, /<p>After\.<\/p>/);
  assertCleanDelims(html);
});

test('Case 59 (WP3): TeX preview harvests preamble macros into document-scoped table', () => {
  resetMathMacros();
  const tex = '\\documentclass{article}\n\\newcommand{\\tr}{\\operatorname{tr}}\n\\begin{document}\n$\\tr \\rho$\n\\end{document}';
  const html = latexDocumentToRenderedHtml(tex, { katex });
  assert.equal(countInline(html), 1);
  assert.ok(!html.includes('var(--status-err)'), `Preamble macro \\tr must not render red: ${html}`);
  // Verify preamble macro did not leak into the global session macro table
  const leaked = render('$\\tr \\rho$');
  assert.ok(leaked.includes('var(--status-err)'), 'Document preamble macro must not leak to session table');
});

test('Case 60 (WP3): streaming partial \\(E = mc renders without error, then closes cleanly', () => {
  resetMathMacros();
  const partial = render('Energy \\(E = mc');
  assert.equal(countInline(partial), 0);
  assert.ok(!partial.includes('katex-error'));
  assert.ok(!partial.includes('math-error'));
  const complete = render('Energy \\(E = mc^2\\) done');
  assert.equal(countInline(complete), 1);
  assert.ok(!complete.includes('katex-error'));
  assertCleanDelims(complete);
});
