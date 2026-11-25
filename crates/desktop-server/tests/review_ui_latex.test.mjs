// Pass 2 independent review tests for UI LaTeX & Markdown fixes (F1, F3, F4, M1, M2).
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
const texMod = await import(pathToFileURL(path.join(assetsDir, 'tex_preview.js')).href);
const {
  latexDocumentToRenderedHtml, markdownToHtml, markdownToRenderedHtml, resetMathMacros,
} = mathMod;
const { harvestPreambleMacros } = texMod;

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

test('review_ui_f3_per_decl_replay_unsupported_does_not_lose_macros', () => {
  const replayInputs = [];
  const spyKatex = {
    renderToString(tex, opts) {
      if (opts && opts.throwOnError && /^\\(?:(?:re|provide)?newcommand|(?:[gex])?def)/.test(tex)) {
        replayInputs.push(tex);
      }
      return katex.renderToString(tex, opts);
    },
  };

  const doc = [
    '\\documentclass{article}',
    '\\newcommand{\\myA}{\\alpha}',
    '\\edef\\bad{\\undefinedcmd}',
    '\\def\\badarg#2{#1}',
    '\\newcommand{\\norm}[2][2]{\\|#2\\|_{#1}}',
    '\\providecommand{\\bad}{\\beta}',
    '\\begin{document}',
    'Values $\\myA$, $\\norm{x}$, $\\norm[\\infty]{x}$, $\\bad$.',
    '\\end{document}',
  ].join('\n');

  const html = latexDocumentToRenderedHtml(doc, { katex: spyKatex });
  assert.ok(!html.includes('katex-error') && !html.includes('var(--status-err)'), `Unexpected error: ${html}`);
  // Every replayed preamble declaration must be invoked individually, never joined across lines.
  assert.ok(replayInputs.length >= 5, `Expected per-decl replay calls, got ${replayInputs.length}`);
  for (const call of replayInputs) {
    assert.ok(!call.includes('\n'), `Whole-preamble batched replay detected: ${JSON.stringify(call)}`);
  }
  const txt = visibleText(html).replace(/\u200b/g, '');
  assert.ok(txt.includes('α'), `Missing \\myA output: ${txt}`);
  assert.ok(txt.includes('∥x∥2'), `Missing default optional-arg \\norm{x}: ${txt}`);
  assert.ok(txt.includes('∥x∥∞'), `Missing explicit optional-arg \\norm[\\infty]{x}: ${txt}`);
  assert.ok(txt.includes('β'), `Failed \\edef\\bad should not block subsequent \\providecommand{\\bad}: ${txt}`);
});

test('review_ui_m1_providecommand_does_not_overwrite_seed_macro', () => {
  const doc = [
    '\\documentclass{article}',
    '\\providecommand{\\R}{\\mathbf{R}}',
    '\\providecommand{\\Tr}[1][2]{\\text{BAD}_{#1}}',
    '\\newcommand{\\myop}{\\mathcal{A}}',
    '\\providecommand{\\myop}{\\mathcal{B}}',
    '\\providecommand{\\fresh}[2][2]{\\|#2\\|_{#1}}',
    '\\begin{document}',
    'Spaces $\\R$, $\\Tr(M)$, $\\myop$, $\\fresh{v}$.',
    '\\end{document}',
  ].join('\n');

  const html = latexDocumentToRenderedHtml(doc, { katex });
  assert.ok(!html.includes('katex-error') && !html.includes('var(--status-err)'), `Unexpected error: ${html}`);
  // \R must keep SEED_MACROS (\mathbb{R}), not be overwritten by \mathbf{R}.
  assert.ok(html.includes('mathbb'), `\\providecommand{\\R} overwrote seed macro \\mathbb{R}: ${html}`);
  assert.ok(!html.includes('mathbf'), `\\providecommand{\\R} emitted \\mathbf: ${html}`);
  const txt = visibleText(html).replace(/\u200b/g, '');
  assert.ok(txt.includes('Tr(M)') && !txt.includes('BAD'), `Optional-arg \\providecommand{\\Tr} overwrote seed macro: ${txt}`);
  assert.ok(txt.includes('A') && !txt.includes('B'), `\\providecommand{\\myop} overwrote earlier preamble macro: ${txt}`);
  assert.ok(txt.includes('∥v∥2'), `Undefined optional-arg \\providecommand{\\fresh} failed to define: ${txt}`);
});

test('review_ui_m2_def_parameter_pattern_bounded_to_line', () => {
  const preamble = [
    '\\def\\foo\\bar',
    '\\newcommand{\\myvec}[1]{\\mathbf{#1}}',
    '\\def\\pair#1#2{',
    '  (#1, #2)',
    '}',
  ].join('\n');

  const decls = harvestPreambleMacros(preamble);
  assert.deepEqual(decls.map((d) => d.name), ['myvec', 'pair'], 'Broken \\def\\foo must not swallow next line \\newcommand');

  const crlfDecls = harvestPreambleMacros('\\def\\foo\\bar\r\n\\newcommand{\\ok}{\\gamma}');
  assert.deepEqual(crlfDecls.map((d) => d.name), ['ok'], 'CRLF boundary must also bound \\def parameter pattern');

  const doc = [
    '\\documentclass{article}',
    preamble,
    '\\begin{document}',
    'Check $\\myvec{v} + \\pair{1}{2}$.',
    '\\end{document}',
  ].join('\n');
  const html = latexDocumentToRenderedHtml(doc, { katex });
  assert.ok(!html.includes('katex-error') && !html.includes('var(--status-err)'), `Unexpected error: ${html}`);
  const txt = visibleText(html);
  assert.ok(txt.includes('v') && txt.includes('(1,2)'), `Expected myvec and pair rendered, got: ${txt}`);
});

test('review_ui_f1_unclosed_streaming_display_and_currency_dollar', () => {
  // Currency $5 before mid-line $$ opener must not desync display pairing.
  const currency = render([
    'It costs $5 and gives: $$',
    'E = mc^2',
    '$$',
    'followed by:',
    '$$',
    'F = ma',
    '$$',
  ].join('\n'));
  assert.equal(countDisplay(currency), 2, `Expected 2 displays after $5, got: ${currency}`);
  assert.ok(visibleText(currency).includes('$5'), 'Literal currency $5 was lost');
  assert.ok(!visibleText(currency).includes('$$'), `Leaked $$ delimiter: ${visibleText(currency)}`);

  // Inline $a$ and escaped \$ before mid-line $$ opener must not desync.
  const inlineBefore = render([
    'For $a$ and \\$5 and $b + \\$ + c$ we get: $$',
    'x = 1',
    '$$',
    'and next:',
    '$$',
    'y = 2',
    '$$',
  ].join('\n'));
  assert.equal(countInline(inlineBefore), 2);
  assert.equal(countDisplay(inlineBefore), 2);
  assert.ok(!visibleText(inlineBefore).includes('$$'));

  // Inline math containing \[ must not trigger false unclosed para display.
  const bracketInInline = render('Token $\\text{\\[}$ here\n$$\nz = 3\n$$');
  assert.equal(countInline(bracketInInline), 1);
  assert.equal(countDisplay(bracketInInline), 1);

  // Half-arrived unclosed streaming $$ stays as plain text/source with no error.
  const streaming = render('We compute $$\nE = mc^2');
  assert.equal(countDisplay(streaming), 0);
  assert.ok(!streaming.includes('katex-error') && !streaming.includes('math-error'));
  assert.ok(visibleText(streaming).includes('$$') && visibleText(streaming).includes('E = mc^2'));
  assert.equal(markdownToHtml('We compute $$\nE = mc^2'), '<p>We compute $$ E = mc^2</p>');

  // findParaDisplayClose stops at ATX headings, fences, blank lines, and blockquotes.
  const stops = [
    { name: 'heading', sep: '## Heading', expect: /<h2>Heading<\/h2>/ },
    { name: 'fence', sep: '```\ncode\n```', expect: /<pre><code>code<\/code><\/pre>/ },
    { name: 'blank', sep: '', expect: /<p>Intro \$\$ x = 1<\/p>/ },
    { name: 'blockquote', sep: '> quoted', expect: /<blockquote><p>quoted<\/p><\/blockquote>/ },
  ];
  for (const { name, sep, expect } of stops) {
    const html = render(`Intro $$\nx = 1\n${sep}\n$$\ny = 2\n$$`);
    assert.match(html, expect, `Unclosed para $$ crossed ${name} boundary: ${html}`);
    assert.equal(countDisplay(html), 1, `Subsequent display after ${name} failed to render: ${html}`);
  }
});

test('review_ui_f4_table_and_quad_dollar_after_midline_close', () => {
  const cases = [
    { name: 'unordered list', md: '$$\nx = 1\n$$ - item', forbid: '<ul', expect: /<p>- item<\/p>/ },
    { name: 'ordered list', md: '$$\nx = 1\n$$ 1. item', forbid: '<ol', expect: /<p>1\. item<\/p>/ },
    { name: 'blockquote', md: '$$\nx = 1\n$$ > quote', forbid: '<blockquote', expect: /<p>&gt; quote<\/p>/ },
    { name: 'heading', md: '$$\nx = 1\n$$ # heading', forbid: '<h1', expect: /<p># heading<\/p>/ },
    {
      name: 'table',
      md: '$$\nx = 1\n$$ | a | b |\n|---|---|\n| 1 | 2 |',
      forbid: '<table',
      expect: /<p>\| a \| b \| \|---\|---\| \| 1 \| 2 \|<\/p>/,
    },
  ];
  for (const { name, md, forbid, expect } of cases) {
    const html = render(md);
    assert.equal(countDisplay(html), 1, `Display failed for trailing ${name}: ${html}`);
    assert.ok(!html.includes(forbid), `Trailing text after $$ became ${name}: ${html}`);
    assert.match(html, expect, `Trailing ${name} did not render as paragraph: ${html}`);
  }

  // Case 18 parity: $$$$ still closes first display and opens the next.
  const quad = render('$$\nU = s/V\n$$$$\nH = N^2 s\n$$');
  assert.equal(countDisplay(quad), 2);
  assert.ok(!quad.includes('katex-error') && !visibleText(quad).includes('$'));

  // Chained displays followed by trailing list-like prose keep both displays and paragraph prose.
  const chained = render('$$a = 1$$$$b = 2$$ - trailing');
  assert.equal(countDisplay(chained), 2);
  assert.ok(!chained.includes('<ul'));
  assert.match(chained, /<p>- trailing<\/p>/);
});
