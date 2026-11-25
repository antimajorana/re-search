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
  clearMathCache, katexOptions, markdownToHtml, markdownToRenderedHtml,
  renderFormulaHtml, resetMathMacros,
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
