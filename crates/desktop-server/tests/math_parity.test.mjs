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
const {
  clearMathCache, displayMathSource, katexOptions, latexDocumentToRenderedHtml,
  markdownToHtml, markdownToRenderedHtml, mathTextToHtml, renderFormulaHtml,
  renderRichInto, resetMathMacros,
} = mathMod;

function render(md, { keepMacros = false, ...opts } = {}) {
  if (!keepMacros) resetMathMacros();
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

function assertCleanDelims(html, { allowDollar = false, allowParen = false, allowBracket = false, allowBegin = false } = {}) {
  const t = visibleText(html);
  if (!allowDollar) assert.ok(!t.includes('$'), `Unexpected "$" in visible text: ${t}`);
  if (!allowParen) assert.ok(!t.includes('\\('), `Unexpected "\\(" in visible text: ${t}`);
  if (!allowBracket) assert.ok(!t.includes('\\['), `Unexpected "\\[" in visible text: ${t}`);
  if (!allowBegin) assert.ok(!t.includes('\\begin{'), `Unexpected "\\begin{" in visible text: ${t}`);
}

function assertHtmlOnlyNoEqnNum(html) {
  assert.ok(!html.includes('<math'), `Unexpected <math in output: ${html}`);
  assert.ok(!html.includes('<annotation'), `Unexpected <annotation in output: ${html}`);
  assert.ok(!html.includes('eqn-num'), `Unexpected eqn-num in output: ${html}`);
}

// --- WP1 & WP2 parity cases (1, 10-13, 44a-54, 61-62) ---

test('Case 62 (WP1): markdownToHtml source form stays exact', () => {
  assert.equal(
    markdownToHtml('Costs $5 and $10, while $\\mu = 0$ is exact.'),
    '<p>Costs $5 and $10, while \\(\\mu = 0\\) is exact.</p>',
  );
});

test('Case 1 (WP2): inline math with subscripts renders cleanly', () => {
  const html = render('Where $\\phi_{0}$ and $\\phi_{e}$ are orbitals.');
  assert.equal(countInline(html), 2);
  assert.equal(countDisplay(html), 0);
  assert.ok(!html.includes('katex-error'));
  assertCleanDelims(html);
  assertHtmlOnlyNoEqnNum(html);
});

test('Cases 10-13 (WP2): escaped dollars and code spans/fences protect text', () => {
  const c10 = render('Price \\$5 and $x$');
  assert.equal(countInline(c10), 1);
  assert.ok(visibleText(c10).includes('Price $5 and '));
  assertHtmlOnlyNoEqnNum(c10);

  const c11 = render('Literal \\$\\$x^2\\$\\$ should stay text.');
  assert.equal(countInline(c11), 0);
  assert.equal(countDisplay(c11), 0);
  assert.equal(visibleText(c11).trim(), 'Literal $$x^2$$ should stay text.');

  const c12 = render('`$x$` and $y$');
  assert.match(c12, /<code>\$x\$<\/code>/);
  assert.equal(countInline(c12), 1);
  assertCleanDelims(c12);

  const c13 = render('```bash\necho $HOME $((1+2)) \\(no\\)\n```\n\nthen $x$.');
  assert.match(c13, /<pre><code class="lang-bash">echo \$HOME \$\(\(1\+2\)\) \\\(no\\\)<\/code><\/pre>/);
  assert.equal(countInline(c13), 1);
  assertCleanDelims(c13);
});

test('Cases 44a, 44b, 45, 46 (WP2): \\newcommand rewrite, \\def persistence, and resetMathMacros()', () => {
  const md44 = '$\\newcommand{\\div}[1]{\\nabla \\cdot #1}$ then $\\div{E}$';
  const r1 = render(md44);
  const r2 = render(md44, { keepMacros: true });
  assert.ok(!r1.includes('katex-error') && !r2.includes('katex-error'));
  assert.equal(countInline(r1), 2);
  assert.equal(r2, r1);
  clearMathCache(); // 44b: clear cache only, keep macro table
  const r3 = render(md44, { keepMacros: true });
  assert.ok(!r3.includes('katex-error'));
  assert.equal(countInline(r3), 2);

  const c45 = render('$\\def\\zz#1{[#1]}$ and $\\zz{q}$');
  assert.ok(!c45.includes('katex-error') && !c45.includes('var(--status-err)'));
  assert.ok(visibleText(c45).includes('[q]'));

  resetMathMacros(); // 46
  const c46 = render('$\\zz{q}$', { keepMacros: true });
  assert.ok(c46.includes('var(--status-err)') && visibleText(c46).includes('\\zz'));
});

test('Cases 47-49 (WP2): \\text repair, seed macros, and Unicode math', () => {
  const c47 = render('$\\text{Nb_3Ge}$');
  assert.ok(!c47.includes('katex-error') && visibleText(c47).includes('Nb_3Ge'));
  assert.equal(countInline(c47), 1);

  const c48 = render('$k_{\\mbox{\\tiny B}}T$ and $\\Tr\\rho$ and $\\C^n$ and $\\ket{\\psi}$');
  assert.equal(countInline(c48), 4);
  assert.ok(!c48.includes('katex-error') && !c48.includes('var(--status-err)'));
  assertCleanDelims(c48);

  const c49 = render('$α + β ≤ γ$');
  assert.equal(countInline(c49), 1);
  assert.ok(!c49.includes('katex-error'));
  assertCleanDelims(c49);
});

test('Cases 50-54 (WP2): errors, unknown commands, trust off, XSS escape, HTML-only output', () => {
  const c50 = render('$x^$');
  assert.equal((c50.match(/class="katex-error"/g) || []).length, 1);
  assert.match(c50, /title="[^"]+"/);
  assert.ok(c50.includes('color:var(--status-err)') && !c50.includes('#cc0000'));
  assert.ok(visibleText(c50).includes('x^'));

  const c51 = render('$\\dv{f}{x}$');
  assert.equal(countInline(c51), 1);
  assert.ok(!c51.includes('katex-error') && c51.includes('color:var(--status-err)'));
  assert.ok(visibleText(c51).includes('\\dv'));

  assert.equal(katexOptions(false).trust, undefined);
  const c52 = render('$\\href{javascript:alert(1)}{x}$');
  assert.ok(!c52.includes('<a') && c52.includes('color:var(--status-err)'));

  const c53 = render('$<img src=x onerror=1>^$');
  assert.ok(!c53.includes('<img') && c53.includes('&lt;img src=x onerror=1&gt;'));
  for (const tag of c53.match(/<[^>]+>/g) || []) {
    const bare = tag.replace(/="[^"]*"/g, '').replace(/='[^']*'/g, '');
    assert.doesNotMatch(bare, /\bonerror\b/i);
  }

  assertHtmlOnlyNoEqnNum(render('Inline $x^2 + y^2 = 1$'));
  assertHtmlOnlyNoEqnNum(render('$$\nE = mc^2\n$$'));
  assertHtmlOnlyNoEqnNum(render('\\begin{equation}\na = b\n\\end{equation}'));
});

test('Case 61 & WP2 fallbacks: per-formula cache, throwing KaTeX, and missing KaTeX', () => {
  resetMathMacros();
  let calls = 0;
  const spyKatex = { renderToString: (...a) => { calls += 1; return katex.renderToString(...a); } };
  const md = '$\\def\\zz#1{[#1]}$ then $\\zz{q}$ and $\\zz{q}$ and $x^2$';
  const first = markdownToRenderedHtml(md, { katex: spyKatex });
  assert.equal(calls, 3);
  const second = markdownToRenderedHtml(md, { katex: spyKatex });
  assert.equal(second, first);
  assert.equal(calls, 3);

  const errHtml = renderFormulaHtml({ tex: 'x < y', display: false }, { katex: { renderToString() { throw new Error('boom'); } } });
  assert.equal(errHtml, '<code class="math-error">x &lt; y</code>');

  const prevWin = globalThis.window;
  try {
    globalThis.window = {};
    assert.equal(renderFormulaHtml({ tex: 'a + b', display: false }), '<span class="math-pending" data-display="0">a + b</span>');
    assert.equal(renderFormulaHtml({ tex: 'a + b', display: true }), '<span class="math-pending" data-display="1">a + b</span>');
  } finally {
    globalThis.window = prevWin;
  }
});

// --- WP3 parity cases (2-9, 14-26, 57-60) ---

test('Cases 2-6 (WP3): \\(...\\), inline env inside $, bare pmatrix mid-sentence vs alone', () => {
  const c2 = render('see \\(x^2\\) here');
  assert.equal(countInline(c2), 1);
  assert.equal(countDisplay(c2), 0);
  assertCleanDelims(c2);

  const c3 = render('The **background \\(m\\) per unit cell** and background-\\(m\\) line.');
  assert.equal(countInline(c3), 2);
  assert.match(c3, /<strong>background <span class="katex">[\s\S]*?<\/span> per unit cell<\/strong>/);
  assertCleanDelims(c3);

  const c4 = render('Let $M = \\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix}$ be diagonal.');
  assert.equal(countInline(c4), 1);
  assert.equal(countDisplay(c4), 0);
  assert.ok(!c4.includes('katex-error'));
  assertCleanDelims(c4);

  const c5 = render('The matrix \\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix} is diagonal.');
  assert.equal(countInline(c5), 1);
  assert.equal(countDisplay(c5), 0);
  assert.equal((c5.match(/<p>/g) || []).length, 1);
  assertCleanDelims(c5);

  const c6 = render('\\begin{pmatrix}1&0\\\\0&-1\\end{pmatrix}');
  assert.equal(countDisplay(c6), 1);
  assert.equal(countInline(c6), 0);
  assertCleanDelims(c6);
});

test('Cases 7-9 (WP3): currency guards reject non-math dollars', () => {
  for (const input of [
    'It costs $5 and $6 more',
    'I paid $5 for apples and my friend paid 10$ for pears.',
    'between $5-$10, and $ 5 + 3 $.',
  ]) {
    const html = render(input);
    assert.equal(countInline(html), 0, `Unexpected inline math in: ${input}`);
    assert.equal(countDisplay(html), 0, `Unexpected display math in: ${input}`);
    assert.equal(visibleText(html).trim(), input);
  }
});

test('Case 14 (WP3): punctuation glue wraps trailing punctuation on inline formulas only', () => {
  const html = render('Cases $a$, then $b$. And $(c)$; and $d$ alone');
  assert.equal((html.match(/class="math-glue"/g) || []).length, 3);
  assert.ok(html.includes(',</span>') && html.includes('.</span>') && html.includes(';</span>'));
  assert.equal(countInline(html), 4);
  assertCleanDelims(html);
});

test('Cases 15-21 (WP3): $$ blocks, blank lines, trailing prose, $$$$, heading bound, #1 error, unclosed $$', () => {
  const c15 = render('Before\n\n$$\nE = mc^2\n$$\n\nAfter');
  assert.equal(countDisplay(c15), 1);
  assert.match(c15, /^<p>Before<\/p>\n<div class="math-display math-block"[\s\S]*<\/div>\n<p>After<\/p>$/);
  assertCleanDelims(c15);

  const c16 = render('$$\na = b\n\n+ c\n$$');
  assert.equal(countDisplay(c16), 1);
  assert.ok(!c16.includes('katex-error'));
  assertCleanDelims(c16);

  const c17 = render('$$\nx = 1\n$$which follows.');
  assert.equal(countDisplay(c17), 1);
  assert.match(c17, /<\/div>\n<p>which follows\.<\/p>$/);
  assertCleanDelims(c17);

  const c18 = render('$$\nU = s/V\n$$$$\nH = N^2 s\n$$');
  assert.equal(countDisplay(c18), 2);
  assert.ok(!c18.includes('katex-error'));
  assertCleanDelims(c18);

  const c19 = render('$$\nx = 1\n\n## Next part\n\n$$ z = 2 $$ and $y$');
  assert.match(c19, /<p>\$\$ x = 1<\/p>/);
  assert.match(c19, /<h2>Next part<\/h2>/);
  assert.equal(countDisplay(c19), 1);
  assert.equal(countInline(c19), 1);

  const c20 = render('$$ x #1 $$');
  assert.equal(countDisplay(c20), 1);
  assert.equal((c20.match(/class="katex-error"/g) || []).length, 1);
  assert.equal((c20.match(/class="katex-display"/g) || []).length, 0);
  assert.ok(!c20.includes('<h1') && visibleText(c20).includes('x #1'));

  const c21 = render('Type $$ to open a display.\n\nSecond paragraph with $x$.');
  assert.equal(countDisplay(c21), 0);
  assert.equal(countInline(c21), 1);
  assert.match(c21, /<p>Type \$\$ to open a display\.<\/p>/);
});

test('Cases 22-26 (WP3): \\[...\\], citations, \\\\[4pt], unclosed env, and blockquote display', () => {
  const c22 = render('\\[\n\\sum_{n=1}^{\\infty} \\frac{1}{n^2}\n\\]');
  assert.equal(countDisplay(c22), 1);
  assert.ok(!c22.includes('katex-error'));
  assertCleanDelims(c22);

  const c23 = render('As shown in \\[1\\] and \\[2, 3\\].');
  assert.equal(countDisplay(c23), 0);
  assert.equal(countInline(c23), 0);
  assert.equal(visibleText(c23).trim(), 'As shown in [1] and [2, 3].');
  assertCleanDelims(c23);

  const c24 = render('Row one \\\\[4pt] row two, later \\[ y \\] done.');
  assert.equal(countDisplay(c24), 1);
  assert.match(c24, /<span class="math-display"/);
  assert.ok(visibleText(c24).includes('Row one \\\\[4pt] row two, later '));
  assert.ok(!visibleText(c24).includes('\\[ y') && !visibleText(c24).includes('\\]'));
  assertCleanDelims(c24, { allowBracket: true });

  const c25 = render('Use \\begin{align} for alignment, e.g. \\(x^2\\) or $y$.');
  assert.equal(countDisplay(c25), 0);
  assert.equal(countInline(c25), 2);
  assert.ok(visibleText(c25).includes('\\begin{align}'));
  assertCleanDelims(c25, { allowBegin: true });

  const c26 = render('> $$\n> a = b\n> $$');
  assert.match(c26, /<blockquote>[\s\S]*<div class="math-display math-block"[\s\S]*<\/blockquote>/);
  assert.equal(countDisplay(c26), 1);
  assert.ok(!c26.includes('data-tex=">') && !c26.includes('data-tex="&gt;'));
  assertCleanDelims(c26);
});

test('Cases 57-60 (WP3): displayMathSource mixed prose, TeX preview \\\\[2mm] & preamble macros, streaming', () => {
  const c57 = render(displayMathSource('$a$ and $b$'));
  assert.equal(countInline(c57), 2);
  assert.equal(countDisplay(c57), 0);
  assert.ok(visibleText(c57).includes('and'));
  assertCleanDelims(c57);

  const c58 = latexDocumentToRenderedHtml('Line one\\\\[2mm]\nLine two is prose.\n\n\\[ x = 1 \\]\nAfter.', { katex });
  assert.equal(countDisplay(c58), 1);
  assert.match(c58, /Line one<br> Line two is prose\./);
  assert.match(c58, /data-tex="x = 1"/);
  assert.match(c58, /<p>After\.<\/p>/);
  assertCleanDelims(c58);

  const c59 = latexDocumentToRenderedHtml(
    '\\documentclass{article}\n\\newcommand{\\tr}{\\operatorname{tr}}\n\\begin{document}\n$\\tr \\rho$\n\\end{document}',
    { katex },
  );
  assert.equal(countInline(c59), 1);
  assert.ok(!c59.includes('var(--status-err)'));
  assert.ok(render('$\\tr \\rho$').includes('var(--status-err)'), 'Preamble macro must not leak to session table');

  const partial = render('Energy \\(E = mc');
  assert.equal(countInline(partial), 0);
  assert.ok(!partial.includes('katex-error') && !partial.includes('math-error'));
  const complete = render('Energy \\(E = mc^2\\) done');
  assert.equal(countInline(complete), 1);
  assert.ok(!complete.includes('katex-error'));
  assertCleanDelims(complete);
});

// --- WP4 parity cases (33-43, 55-56, 65) ---

test('Cases 33-37 (WP4): app equation numbering, \\label, \\tag, \\notag, and equation blocks', () => {
  const c33 = render('$$\na = b \\label{eq-first}\n$$\n\nSee \\eqref{eq-first}.');
  assert.equal(countDisplay(c33), 1);
  assert.ok(c33.includes('data-eq-id="eq-first"') && c33.includes('class="katex-tag"'));
  assert.ok(!visibleText(c33).includes('label'));
  assert.match(c33, /<a class="eqref-link" data-eq-ref="eq-first">\(1\)<\/a>/);
  assert.ok(!c33.includes('href='));
  assertHtmlOnlyNoEqnNum(c33);

  const c34 = render('$$ c = d $$');
  assert.equal(countDisplay(c34), 1);
  assert.ok(!c34.includes('katex-tag'));
  assertHtmlOnlyNoEqnNum(c34);

  const c35 = render('$$ g \\tag{D1} \\label{eq-own} $$\n\nSee \\eqref{eq-own}.\n\n$$ h \\label{x} $$');
  assert.equal(countDisplay(c35), 2);
  assert.match(c35, /<a class="eqref-link" data-eq-ref="eq-own">\(D1\)<\/a>/);
  assert.ok(visibleText(c35).includes('(D1)') && visibleText(c35).includes('(1)') && !visibleText(c35).includes('(2)'));
  assertHtmlOnlyNoEqnNum(c35);

  const c36 = render('$$ i \\notag \\label{eq-none} $$');
  assert.equal(countDisplay(c36), 1);
  assert.ok(!c36.includes('katex-tag'));
  assertHtmlOnlyNoEqnNum(c36);

  const c37 = render('\\begin{equation}\na=b\n\\end{equation}\n\n\\begin{equation}\nc=d\n\\end{equation}');
  assert.equal(countDisplay(c37), 2);
  assert.equal((c37.match(/class="katex-tag"/g) || []).length, 2);
  assert.ok(visibleText(c37).includes('(1)') && visibleText(c37).includes('(2)'));
  assertHtmlOnlyNoEqnNum(c37);
});

test('Cases 38-41 (WP4): per-row align/cases/eqnarray numbering and starred align*', () => {
  const c38 = render([
    '\\begin{align}',
    'a &= b \\label{r1} \\\\',
    'c &= d \\notag \\\\',
    'e &= f \\tag{T} \\\\',
    'g &= h \\label{r2}',
    '\\end{align}',
    '',
    'Refs \\eqref{r1} and \\eqref{r2}.',
  ].join('\n'));
  assert.ok(!c38.includes('katex-error'));
  assert.equal(countDisplay(c38), 1);
  assert.match(c38, /<a class="eqref-link" data-eq-ref="r1">\(1\)<\/a>/);
  assert.match(c38, /<a class="eqref-link" data-eq-ref="r2">\(2\)<\/a>/);
  const t38 = visibleText(c38);
  assert.ok(t38.includes('(1)') && t38.includes('(T)') && t38.includes('(2)') && !t38.includes('(3)'));
  assertHtmlOnlyNoEqnNum(c38);

  const c39 = render('\\begin{align}\nf &= \\begin{cases} 1 \\\\ 0 \\end{cases} \\\\\ng &= 2\n\\end{align}');
  assert.ok(!c39.includes('katex-error'));
  assert.equal(countDisplay(c39), 1);
  assert.ok(visibleText(c39).includes('(1)') && visibleText(c39).includes('(2)') && !visibleText(c39).includes('(3)'));
  assertHtmlOnlyNoEqnNum(c39);

  const c40 = render('\\begin{eqnarray}\na &=& b \\\\\nc &=& d\n\\end{eqnarray}');
  assert.ok(!c40.includes('katex-error'));
  assert.equal(countDisplay(c40), 1);
  assert.ok(visibleText(c40).includes('(1)') && visibleText(c40).includes('(2)'));
  assertHtmlOnlyNoEqnNum(c40);

  const c41 = render('\\begin{align*}\na &= b \\\\\nc &= d\n\\end{align*}');
  assert.equal(countDisplay(c41), 1);
  assert.ok(!c41.includes('katex-tag'));
  assertHtmlOnlyNoEqnNum(c41);
});

test('Cases 42, 43, 55, 56, 65 (WP4): unknown \\eqref, formula \\eqref, inlineOnly cleanup, delegated nav', () => {
  assert.match(render('See \\eqref{nope}.'), /<a class="eqref-link" data-eq-ref="nope">\(nope\)<\/a>/);

  const c43 = render('$$\na = b \\label{eq-first}\n$$\n\nInline $x \\eqref{eq-first}$ here.');
  assert.equal(countInline(c43), 1);
  assert.ok(!c43.includes('katex-error') && !c43.includes('var(--status-err)') && visibleText(c43).includes('(1)'));

  const c55 = mathTextToHtml('Step $E=mc^2 \\tag{3}$', { inlineOnly: true, katex });
  assert.equal(countInline(c55), 1);
  assert.equal(countDisplay(c55), 0);
  assert.ok(!c55.includes('katex-error'));

  const c56 = mathTextToHtml('\\begin{align} a&=b \\end{align}', { inlineOnly: true, katex });
  assert.equal(countInline(c56), 1);
  assert.equal(countDisplay(c56), 0);
  assert.ok(!c56.includes('katex-error'));

  // Case 65: fake root without addEventListener + idempotent listener on real-ish root
  const fakeRoot = { innerHTML: '' };
  assert.doesNotThrow(() => renderRichInto(fakeRoot, '$$\na = b \\label{eq-1}\n$$\nSee \\eqref{eq-1}.', { katex }));

  const listeners = [];
  const scrolled = [];
  const root = {
    innerHTML: '',
    addEventListener(type, fn) { listeners.push({ type, fn }); },
    querySelector: (sel) => (sel === '[data-eq-id="eq-1"]' ? { scrollIntoView: (o) => scrolled.push(o) } : null),
  };
  renderRichInto(root, '$$\na = b \\label{eq-1}\n$$\nSee \\eqref{eq-1}.', { katex });
  renderRichInto(root, '$$\na = b \\label{eq-1}\n$$\nSee \\eqref{eq-1}.', { katex });
  assert.equal(listeners.length, 1);

  const prevCss = globalThis.CSS;
  try {
    globalThis.CSS = { escape: (s) => s };
    let prevented = false;
    listeners[0].fn({
      target: { closest: (sel) => (sel.includes('eqref-link') ? { getAttribute: () => 'eq-1' } : null) },
      preventDefault: () => { prevented = true; },
    });
    assert.equal(prevented, true);
    assert.deepEqual(scrolled, [{ block: 'center', behavior: 'smooth' }]);
  } finally {
    globalThis.CSS = prevCss;
  }
});

// --- WP5 parity cases (27-32, 63-64) & container edge checks ---

test('Cases 27-29 (WP5): container-aware list items, loose lists, and ordered list start', () => {
  const c27 = render('1. First step:\n   $$x = 1$$\n   continues here\n2. Second step');
  assert.equal((c27.match(/<ol\b/g) || []).length, 1);
  assert.equal((c27.match(/<li>/g) || []).length, 2);
  assert.equal(countDisplay(c27), 1);
  assert.match(c27, /<li>[\s\S]*First step:[\s\S]*class="math-display math-block"[\s\S]*continues here[\s\S]*<\/li>/);
  assertCleanDelims(c27);

  const c28 = render('1. first\n\n2. second\n\n3. third');
  assert.equal((c28.match(/<ol\b/g) || []).length, 1);
  assert.equal((c28.match(/<li>/g) || []).length, 3);

  const c29 = render('3. three\n4. four');
  assert.match(c29, /<ol start="3">/);
  assert.equal((c29.match(/<li>/g) || []).length, 2);
});

test('Cases 30-32, 63-64 (WP5): tables, headings, 4-space prose, ** flanking, HTML comments', () => {
  const c30 = render('| a | b |\n|--|--|\n| $|x|$ | \\(\\tfrac{21}{4}\\) |');
  assert.match(c30, /<div class="table-wrap"><table>/);
  assert.equal(countInline(c30), 2);
  assert.equal((c30.match(/<th\b/g) || []).length, 2);
  assert.equal((c30.match(/<td\b/g) || []).length, 2);
  assertCleanDelims(c30);

  const c31 = render('   ## Result for $x^2$ ##');
  assert.match(c31, /<h2>Result for <span class="katex">/);
  assert.equal(countInline(c31), 1);
  assert.ok(!visibleText(c31).includes('#'));
  assertCleanDelims(c31);

  const c32 = render('    However we choose $b$ here');
  assert.match(c32, /^<p>/);
  assert.ok(!c32.includes('<pre>'));
  assert.equal(countInline(c32), 1);
  assertCleanDelims(c32);

  const c63 = render('2**3 and 4**5');
  assert.ok(!c63.includes('<strong>'));
  assert.equal(visibleText(c63).trim(), '2**3 and 4**5');

  assert.equal(visibleText(render('a <!-- note --> b')), 'a  b');
  const codeHtml = render('`<!-- keep -->` and a <!-- drop --> b');
  assert.match(codeHtml, /<code>&lt;!-- keep --&gt;<\/code>/);
  assert.ok(!visibleText(codeHtml).includes('drop'));
});

test('Extra container & eqref safety checks: fences in lists/quotes, forward & mid-para \\eqref, unsafe id', () => {
  const listFence = render('1. Run:\n   ```bash\n   echo $HOME\n   ```\n2. Done');
  assert.equal((listFence.match(/<ol\b/g) || []).length, 1);
  assert.equal((listFence.match(/<li>/g) || []).length, 2);
  assert.match(listFence, /<pre><code class="lang-bash">echo \$HOME<\/code><\/pre>/);
  assert.equal(countInline(listFence), 0);

  const quoteFence = render('> ```js\n> const s = "$x$";\n> ```');
  assert.match(quoteFence, /<blockquote><pre><code class="lang-js">const s = &quot;\$x\$&quot;;<\/code><\/pre><\/blockquote>/);
  assert.equal(countInline(quoteFence), 0);

  const fwdAndMid = render('Before \\eqref{eq-mid}, then inline $$ y = 2 \\label{eq-mid} $$ after.');
  assert.match(fwdAndMid, /<a class="eqref-link" data-eq-ref="eq-mid">\(1\)<\/a>/);
  assert.match(fwdAndMid, /<span class="math-display" data-eq-id="eq-mid"/);
  assert.ok(!visibleText(fwdAndMid).includes('label'));
  assertHtmlOnlyNoEqnNum(fwdAndMid);

  const unsafeId = render('$x \\eqref{bad\\cmd}$');
  assert.ok(!unsafeId.includes('katex-error') && !unsafeId.includes('<a'));
});
