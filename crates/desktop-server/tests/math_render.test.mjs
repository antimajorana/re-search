import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
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
const { escapeHtml, markdownToHtml, latexDocumentToHtml } = mathMod;

function unescapeHtml(s) {
  return s
    .replaceAll('&lt;', '<')
    .replaceAll('&gt;', '>')
    .replaceAll('&quot;', '"')
    .replaceAll('&#39;', "'")
    .replaceAll('&amp;', '&');
}

function extractMathFromHtml(html) {
  const found = [];
  let rest = html.replace(/<div class="math-block">\$\$([\s\S]*?)\$\$<\/div>/g, (_, body) => {
    found.push({ tex: unescapeHtml(body).trim(), display: true });
    return '';
  });
  rest = rest.replace(/\$\$([\s\S]*?)\$\$/g, (_, body) => {
    found.push({ tex: unescapeHtml(body).trim(), display: true });
    return '';
  });
  rest.replace(/\\\(([\s\S]*?)\\\)/g, (_, body) => {
    found.push({ tex: unescapeHtml(body).trim(), display: false });
    return '';
  });
  return found;
}

function assertAllKatexValid(html, minCount = 1) {
  const items = extractMathFromHtml(html);
  assert.ok(items.length >= minCount, `Expected >= ${minCount} math expressions, got ${items.length} in:\n${html}`);
  for (const item of items) {
    const rendered = katex.renderToString(item.tex, {
      displayMode: item.display,
      throwOnError: true,
      strict: false,
    });
    assert.ok(rendered.includes('katex-base'), `KaTeX 0.19.1 output missing katex-base for: ${item.tex}`);
    if (item.display) {
      assert.ok(rendered.includes('katex-display'), `Display math missing katex-display for: ${item.tex}`);
    }
  }
  return items;
}

function assertNoXss(html) {
  assert.doesNotMatch(html, /<script\b/i, `Unescaped <script> found in:\n${html}`);
  assert.doesNotMatch(html, /<img\b/i, `Unescaped <img> found in:\n${html}`);
  for (const tag of html.match(/<[^>]+>/g) || []) {
    assert.doesNotMatch(tag, /\bon[a-z]+\s*=/i, `Event handler attribute found in tag ${tag}`);
    assert.doesNotMatch(tag, /href\s*=\s*["']\s*javascript:/i, `javascript: href found in tag ${tag}`);
  }
}

test('Vendored KaTeX is 0.19.1 offline build', () => {
  assert.equal(katex.version, '0.19.1');
});

test('CSS audit: starter-chip scoped to > span and no bare descendant span/div/* inside KaTeX containers', () => {
  const componentsCss = fs.readFileSync(path.join(assetsDir, 'components.css'), 'utf8');
  const layoutCss = fs.readFileSync(path.join(assetsDir, 'layout.css'), 'utf8');
  const workbenchCss = fs.readFileSync(path.join(assetsDir, 'workbench.css'), 'utf8');

  assert.doesNotMatch(componentsCss, /\.starter-chip\s+span\b/, 'components.css must not have unscoped .starter-chip span');
  assert.match(componentsCss, /\.starter-chip\s*>\s*span\b/, 'components.css must scope .starter-chip > span');
  assert.match(componentsCss, /\.starter-chip\s*>\s*strong\b/, 'components.css must scope .starter-chip > strong');

  // Topbar regression check: long Windows workspace paths must ellipsize inside #active-workspace-pill
  // without crushing #active-session-title or colliding with .topbar-right (Ready / Canvas).
  assert.match(layoutCss, /\.topbar-left\s*\{[^}]*overflow:\s*hidden/s, '.topbar-left must clip overflow');
  assert.match(layoutCss, /\.topbar-right\s*\{[^}]*flex-shrink:\s*0/s, '.topbar-right must not shrink under long paths');
  assert.match(layoutCss, /#active-workspace-pill\s*\{[^}]*text-overflow:\s*ellipsis/s, '#active-workspace-pill must ellipsize long paths');

  const katexContainers = [
    '.starter-chip', '.rich-prose', '.msg-bubble-user', '.msg-assistant-body',
    '#latex-preview', '.dag-detail-card', '.paper-card', '.source-card', '#sidetalk-stream', '.sc-latex',
  ];

  for (const [name, css] of [['components.css', componentsCss], ['layout.css', layoutCss], ['workbench.css', workbenchCss]]) {
    const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
    const ruleHeads = stripped.split('}').map((r) => r.split('{')[0].trim()).filter(Boolean);
    for (const head of ruleHeads) {
      if (head.startsWith('@')) continue;
      for (const sel of head.split(',').map((s) => s.trim())) {
        for (const container of katexContainers) {
          if (!sel.includes(container)) continue;
          const after = sel.slice(sel.indexOf(container) + container.length);
          assert.doesNotMatch(
            after,
            /^\s+(span|div|\*)(?:\b|$)/,
            `Unsafe bare descendant selector in ${name}: "${sel}"`
          );
        }
      }
    }
  }
});

test('All LaTeX math environments render cleanly in real KaTeX 0.19.1 without splitting rows', () => {
  const envSamples = [
    ['equation', '\\begin{equation}\nE = mc^2\n\\end{equation}'],
    ['equation*', '\\begin{equation*}\nH = -t \\sum_j c_j^\\dagger c_{j+1}\n\\end{equation*}'],
    ['align', '\\begin{align}\na &= b + c \\\\\nd &= e + f\n\\end{align}'],
    ['align*', '\\begin{align*}\nx_1 &= 1 \\\\\nx_2 &= 2\n\\end{align*}'],
    ['gather', '\\begin{gather}\na = b \\\\\nc = d\n\\end{gather}'],
    ['gather*', '\\begin{gather*}\n\\alpha = 0 \\\\\n\\beta = 1\n\\end{gather*}'],
    ['alignat', '\\begin{alignat}{2}\na &= b & c &= d \\\\\ne &= f & g &= h\n\\end{alignat}'],
    ['alignat*', '\\begin{alignat*}{2}\na &= b & c &= d\n\\end{alignat*}'],
    ['flalign', '\\begin{flalign}\na &= b \\\\\nc &= d\n\\end{flalign}'],
    ['flalign*', '\\begin{flalign*}\na &= b \\\\\nc &= d\n\\end{flalign*}'],
    ['multline', '\\begin{multline}\na + b + c \\\\\n+ d + e\n\\end{multline}'],
    ['multline*', '\\begin{multline*}\na + b \\\\\n+ c + d\n\\end{multline*}'],
    ['eqnarray', '\\begin{eqnarray}\na &=& b \\\\\nc &=& d\n\\end{eqnarray}'],
    ['eqnarray*', '\\begin{eqnarray*}\na &=& b \\\\\nc &=& d\n\\end{eqnarray*}'],
    ['split', '\\begin{split}\na &= b \\\\\n  &= c\n\\end{split}'],
    ['aligned', '\\begin{aligned}\na &= b \\\\\nc &= d\n\\end{aligned}'],
    ['gathered', '\\begin{gathered}\na = 1 \\\\\nb = 2\n\\end{gathered}'],
    ['cases', '\\begin{cases}\nx & x > 0 \\\\\n-x & x \\le 0\n\\end{cases}'],
    ['dcases', '\\begin{dcases}\n\\frac{1}{2} & x > 0 \\\\\n0 & x \\le 0\n\\end{dcases}'],
    ['rcases', '\\begin{rcases}\nx = 1 \\\\\ny = 2\n\\end{rcases}'],
    ['matrix', '\\begin{matrix}\n1 & 2 \\\\\n3 & 4\n\\end{matrix}'],
    ['pmatrix', '\\begin{pmatrix}\n1 & 0 \\\\\n0 & -1\n\\end{pmatrix}'],
    ['bmatrix', '\\begin{bmatrix}\na & b \\\\\nc & d\n\\end{bmatrix}'],
    ['Bmatrix', '\\begin{Bmatrix}\n1 & 2 \\\\\n3 & 4\n\\end{Bmatrix}'],
    ['vmatrix', '\\begin{vmatrix}\na & b \\\\\nc & d\n\\end{vmatrix}'],
    ['Vmatrix', '\\begin{Vmatrix}\na & b \\\\\nc & d\n\\end{Vmatrix}'],
    ['smallmatrix', '\\begin{smallmatrix}\n1 & 2 \\\\\n3 & 4\n\\end{smallmatrix}'],
    ['array', '\\begin{array}{cc}\n1 & 2 \\\\\n3 & 4\n\\end{array}'],
    ['CD', '\\begin{CD}\nA @>>> B \\\\\n@VVV @VVV \\\\\nC @>>> D\n\\end{CD}'],
    ['subequations', '\\begin{subequations}\n\\label{eq:sub}\n\\begin{align}\na &= b \\\\\nc &= d\n\\end{align}\n\\end{subequations}'],
    ['$$', '$$\n\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}\n$$'],
    ['\\[\\]', '\\[\n\\sum_{n=1}^{\\infty} \\frac{1}{n^2} = \\frac{\\pi^2}{6}\n\\]'],
    ['\\(\\)', 'Inline \\(\\alpha^2 + \\beta^2 = 1\\) here.'],
    ['$...$', 'Inline $\\mu = 0$ and $\\Delta > 0$.'],
  ];

  for (const [label, sample] of envSamples) {
    const html = markdownToHtml(sample);
    if (label !== '\\(\\)' && label !== '$...$') {
      assert.ok(html.includes('<div class="math-block">'), `${label} should render inside .math-block:\n${html}`);
      assert.doesNotMatch(html, /<\/p>\s*<p>/, `${label} must not split rows across <p> tags:\n${html}`);
    }
    assertAllKatexValid(html, 1);
  }
});

test('Nested environments stay atomic and render cleanly in KaTeX 0.19.1', () => {
  const md = [
    'Before:',
    '\\begin{equation}',
    '\\label{eq:nested}',
    '\\begin{aligned}',
    'H &= -t \\sum_j c_j^\\dagger c_{j+1} \\\\',
    '  &\\quad + \\Delta \\sum_j c_j c_{j+1}',
    '\\end{aligned}',
    '\\end{equation}',
    '',
    '$$',
    'f(x) = \\begin{cases} 1 & x > 0 \\\\ 0 & x \\le 0 \\end{cases}',
    '$$',
  ].join('\n');

  const html = markdownToHtml(md);
  const items = assertAllKatexValid(html, 2);
  assert.equal(items.length, 2, 'Nested environments must stay atomic (2 blocks total)');
  assert.ok(!items[0].tex.includes('\\label'), '\\label must be stripped from KaTeX input');
});

test('Starter prompts from stream.js render cleanly in real KaTeX 0.19.1', () => {
  const streamJs = fs.readFileSync(path.join(assetsDir, 'stream.js'), 'utf8');
  assert.match(streamJs, /\$\\\\mu = 0\$/);
  assert.match(streamJs, /\\omega\(k\)/);

  const p1 = 'Compute the ground-state energy gap of the 1D Kitaev chain at $\\mu = 0$ and verify the Majorana edge-mode commutator algebra in SymPy.';
  const p3 = 'Derive the dispersion relation $\\omega(k) = \\sqrt{\\Delta^2 + (2t \\cos k)^2}$, append structured proof steps, and run deterministic verification.';

  const h1 = markdownToHtml(p1);
  const h3 = markdownToHtml(p3);
  assert.equal(assertAllKatexValid(h1, 1).length, 1);
  assert.equal(assertAllKatexValid(h3, 1).length, 1);
});

test('Markdown block structure: paragraphs, hard breaks, headings h1-h6, tables, nested lists, blockquotes, fences, hr', () => {
  const md = [
    '# H1',
    '#### H4 heading',
    '##### H5 heading',
    '###### H6 heading',
    '',
    'Line one of paragraph',
    'line two joined softly.',
    '',
    'Hard break spaces  ',
    'next line\\',
    'third line.',
    '',
    '---',
    '',
    '> Quote line 1',
    '> Quote line 2',
    '>',
    '> Second quote paragraph',
    '',
    '- Top A',
    '  - Nested A1',
    '  + Nested A2',
    '    1) Ordered deep 1',
    '    2) Ordered deep 2',
    '- Top B',
    '',
    '| Symbol | Meaning | Value |',
    '| :--- | :---: | ---: |',
    '| $|x|$ | Magnitude of **x** | $\\ge 0$ |',
    '| `code` | Literal | 42 |',
    '',
    '```python',
    'x = "$not_math$"',
    '```',
    '',
    '~~~tex',
    '\\begin{align} unclosed',
  ].join('\n');

  const html = markdownToHtml(md);
  assert.match(html, /<h1>H1<\/h1>/);
  assert.match(html, /<h4>H4 heading<\/h4>/);
  assert.match(html, /<h5>H5 heading<\/h5>/);
  assert.match(html, /<h6>H6 heading<\/h6>/);
  assert.match(html, /<p>Line one of paragraph line two joined softly\.<\/p>/);
  assert.match(html, /<p>Hard break spaces<br>\nnext line<br>\nthird line\.<\/p>/);
  assert.match(html, /<hr>/);
  assert.match(html, /<blockquote><p>Quote line 1 Quote line 2<\/p>\n<p>Second quote paragraph<\/p><\/blockquote>/);
  assert.match(html, /<ul><li>Top A\s*<ul><li>Nested A1\s*<\/li>\s*<li>Nested A2\s*<ol><li>Ordered deep 1\s*<\/li>\s*<li>Ordered deep 2\s*<\/li><\/ol>\s*<\/li><\/ul>\s*<\/li>\s*<li>Top B\s*<\/li><\/ul>/);
  assert.match(html, /<table><thead><tr><th/);
  assert.match(html, /<strong>x<\/strong>/);
  assert.match(html, /<pre><code class="lang-python">x = &quot;\$not_math\$&quot;<\/code><\/pre>/);
  assert.match(html, /<pre><code class="lang-tex">\\begin\{align\} unclosed<\/code><\/pre>/);
  assertAllKatexValid(html, 2);
});

test('Inline edge cases: currency prices, escaped dollars, arithmetic *, snake_case _, and inline code', () => {
  const md = [
    'It costs $5 and $10, or $100 total, between $5-$10, and $ 5 + 3 $.',
    '',
    'Costs $5 and $10, while $\\mu = 0$ is exact.',
    '',
    'Escaped \\$5 and \\$10 stay plain, while $\\alpha = 1$ is math.',
    '',
    'Check a*b + c*d, 2 * 3 * 4, foo_bar_baz, and `inline $x^2$` stays code.',
    '',
    'Real **bold**, __bold2__, *italic*, and _italic2_ still work.',
  ].join('\n');

  const html = markdownToHtml(md);
  assert.match(html, /<p>It costs \$5 and \$10, or \$100 total, between \$5-\$10, and \$ 5 \+ 3 \$\.<\/p>/);
  assert.match(html, /<p>Costs \$5 and \$10, while \\\(\\mu = 0\\\) is exact\.<\/p>/);
  assert.match(html, /Escaped \$5 and \$10 stay plain, while \\\(\\alpha = 1\\\) is math\./);
  assert.match(html, /Check a\*b \+ c\*d, 2 \* 3 \* 4, foo_bar_baz, and <code>inline \$x\^2\$<\/code> stays code\./);
  assert.match(html, /<strong>bold<\/strong>, <strong>bold2<\/strong>, <em>italic<\/em>, and <em>italic2<\/em>/);

  const items = assertAllKatexValid(html, 2);
  assert.equal(items.length, 2, 'Only $\\mu = 0$ and $\\alpha = 1$ should be extracted as math');
  assert.equal(items[0].tex, '\\mu = 0');
  assert.equal(items[1].tex, '\\alpha = 1');
});

test('latexDocumentToHtml handles full TeX article cleanly without dropping lines or breaking math', () => {
  const tex = `
\\documentclass[11pt]{article}
\\usepackage{amsmath,amssymb}
\\title{Majorana Edge Modes at $\\mu = 0$}
\\author{A. Physicist \\and B. Theorist}
% Preamble comment should vanish
\\begin{document}
\\maketitle

\\begin{abstract}
We study a 1D chain with 50\\% disorder. % inline comment vanishes
\\end{abstract}

\\section*{Introduction}
First line of a paragraph
% comment inside paragraph must not split it
continues with \\textbf{bold}, \\emph{italic}, \\texttt{sympy},
citation \\cite{kitaev2001}, and reference \\eqref{eq:disp}.

\\subsection{Model}
\\subsubsection{Bulk Spectrum}
\\paragraph{Exact Limit}
Line with break\\\\
and~nonbreaking \`\`quotes''.

\\begin{equation*}
\\label{eq:ham}
H = \\sum_j \\left(-t c_j^\\dagger c_{j+1} + \\Delta c_j c_{j+1}\\right)
\\end{equation*}

\\begin{theorem}[Spectral Gap]
\\label{thm:gap}
At $\\mu = 0$, the dispersion satisfies
\\begin{align*}
\\omega(k) &= \\sqrt{\\Delta^2 + (2t \\cos k)^2} \\label{eq:disp} \\\\
\\Delta_{\\min} &= |\\Delta|.
\\end{align*}
\\end{theorem}

\\begin{itemize}
\\item Item one with $x^2 + 1$
\\item Item two referencing \\ref{thm:gap}
\\end{itemize}

\\begin{cases}
1 & x > 0 \\\\
0 & x \\le 0
\\end{cases}

\\end{document}
TRAILING GARBAGE AFTER END DOCUMENT MUST NOT APPEAR
`;

  const html = latexDocumentToHtml(tex);
  assert.match(html, /<header class="tex-doc-header">/);
  assert.match(html, /<h1>Majorana Edge Modes at \\\(\\mu = 0\\\)<\/h1>/);
  assert.match(html, /A\. Physicist, B\. Theorist/);
  assert.ok(!html.includes('documentclass'), 'Preamble must be stripped');
  assert.ok(!html.includes('Preamble comment'), 'Preamble comment must be stripped');
  assert.ok(!html.includes('inline comment vanishes'), 'Inline comment must be stripped');
  assert.ok(!html.includes('TRAILING GARBAGE'), 'Content after \\end{document} must be ignored');
  assert.match(html, /50% disorder\./, 'Escaped \\% must be preserved');
  assert.match(html, /First line of a paragraph continues with <strong>bold<\/strong>, <em>italic<\/em>, <code>sympy<\/code>/);
  assert.match(html, /<code>\[cite:kitaev2001\]<\/code>/);
  assert.match(html, /<code>\[ref:eq:disp\]<\/code>/);
  assert.match(html, /<h2>Introduction<\/h2>/);
  assert.match(html, /<h3>Model<\/h3>/);
  assert.match(html, /<h4>Bulk Spectrum<\/h4>/);
  assert.match(html, /<h5>Exact Limit<\/h5>/);
  assert.match(html, /<div class="tex-env tex-env-abstract">/);
  assert.match(html, /<div class="tex-env tex-env-theorem">/);
  assert.match(html, /Theorem \(Spectral Gap\)/);
  assert.match(html, /<ul><li>Item one with \\\(x\^2 \+ 1\\\)<\/li><li>Item two referencing <code>\[ref:thm:gap\]<\/code><\/li><\/ul>/);
  assert.match(html, /&ldquo;quotes&rdquo;/);
  assert.match(html, /and&nbsp;nonbreaking/);
  assert.match(html, /Line with break<br>/);

  const items = assertAllKatexValid(html, 6);
  assert.equal(items.length, 6, `Expected 6 math expressions in TeX doc, got ${items.length}`);

  const snippetHtml = latexDocumentToHtml('\\section{Quick Note}\nLet $\\alpha = 1$.\n\\begin{multline*}\na + b \\\\\n+ c\n\\end{multline*}');
  assert.match(snippetHtml, /<h2>Quick Note<\/h2>/);
  assertAllKatexValid(snippetHtml, 2);
});

test('renderMathInContainer fallback path renders \\begin{align*} and protects currency via real KaTeX 0.19.1', () => {
  const prevWin = globalThis.window;
  const prevDoc = globalThis.document;
  const prevNF = globalThis.NodeFilter;
  try {
    const replaced = [];
    const textNode = {
      nodeValue: 'Costs $5 and $10, while \\begin{align*} a &= b \\\\ c &= d \\end{align*} and $\\mu = 0$.',
      parentElement: { tagName: 'DIV' },
      parentNode: { replaceChild(next) { replaced.push(next); } },
    };
    let yielded = false;
    globalThis.NodeFilter = { SHOW_TEXT: 4 };
    globalThis.window = { katex };
    globalThis.document = {
      createTreeWalker: () => ({
        nextNode: () => {
          if (yielded) return null;
          yielded = true;
          return textNode;
        },
      }),
      createElement: () => ({ innerHTML: '' }),
    };

    mathMod.renderMathInContainer({});
    assert.equal(replaced.length, 1);
    assert.ok(replaced[0].innerHTML.includes('Costs $5 and $10, while '), 'Currency must stay plain text');
    assert.ok(replaced[0].innerHTML.includes('katex-display'), '\\begin{align*} must render in display mode');
    assert.ok(replaced[0].innerHTML.includes('katex-base'), 'Real KaTeX 0.19.1 output expected');
  } finally {
    globalThis.window = prevWin;
    globalThis.document = prevDoc;
    globalThis.NodeFilter = prevNF;
  }
});

test('XSS and placeholder spoofing payloads are neutralized in Markdown and LaTeX preview', () => {
  const mdPayload = [
    '<script>alert(1)</script>',
    '<img src=x onerror=alert(1)>',
    '[click](javascript:alert(1))',
    '[attr](https://example.com"onclick="alert(1))',
    '\u0000M0\u0000 \u0000B0\u0000 \u0000C0\u0000',
    'Math XSS: $<script>alert(1)</script>$',
    '| <img src=x onerror=alert(1)> | $x$ |',
    '| --- | --- |',
    '| <script>alert(2)</script> | [x](javascript:void(0)) |',
  ].join('\n');

  const mdHtml = markdownToHtml(mdPayload);
  assertNoXss(mdHtml);
  assert.ok(!mdHtml.includes('\u0000'), 'Null bytes must be stripped');
  assert.match(mdHtml, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);

  const texPayload = [
    '\\title{<script>alert(1)</script>}',
    '\\begin{document}',
    '\\section{<img src=x onerror=alert(1)>}',
    '\\textbf{<script>alert(2)</script>} \\cite{<img src=x onerror=alert(3)>}',
    '\\begin{theorem}[<script>alert(4)</script>]',
    '\u0000M0\u0000 $<script>alert(5)</script>$',
    '\\end{theorem}',
    '\\end{document}',
  ].join('\n');

  const texHtml = latexDocumentToHtml(texPayload);
  assertNoXss(texHtml);
  assert.ok(!texHtml.includes('\u0000'), 'Null bytes must be stripped from TeX output');
  assert.equal(escapeHtml('<>"&\''), '&lt;&gt;&quot;&amp;&#39;');
});

test('Stage 10 UI wiring: Stop button, live text_delta streaming, session literature approval, and in-place workspace attach', () => {
  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const streamJs = fs.readFileSync(path.join(assetsDir, 'stream.js'), 'utf8');
  const composerJs = fs.readFileSync(path.join(assetsDir, 'composer.js'), 'utf8');
  const appJs = fs.readFileSync(path.join(assetsDir, 'app.js'), 'utf8');
  const apiJs = fs.readFileSync(path.join(assetsDir, 'api.js'), 'utf8');

  assert.match(indexHtml, /id="composer-stop"/, 'Composer must render a Stop button');
  assert.match(streamJs, /Approve for Session/, 'Literature approval card must offer Approve for Session');
  assert.match(streamJs, /export function handleTextDelta/, 'Stream must handle incremental text_delta events');
  assert.match(composerJs, /pickWorkspaceFolder/, 'Workspace button must invoke native folder picker endpoint');
  assert.match(appJs, /setSessionWorkspace\(state\.activeSessionId/, 'Workspace attach must update active session in-place');
  assert.match(apiJs, /'text_delta'/, 'EventSource client must subscribe to text_delta');
});

test('Stage 11 Workbench UI: pipeline stepper, rigor presets, conventions popover, resizable Canvas, and XSS-safe trace/verdict cards', async () => {
  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');

  assert.match(indexHtml, /id="pipeline-bar"/, 'Live multi-agent pipeline bar must exist');
  assert.match(indexHtml, /data-rigor-preset="verified"/, 'Rigor segmented control must expose Verified preset');
  assert.match(indexHtml, /data-rigor-preset="deep"/, 'Rigor segmented control must expose Deep preset');
  assert.match(indexHtml, /id="conventions-popover"/, 'Quick Pinned Conventions popover must exist');
  assert.match(indexHtml, /id="canvas-resizer"/, 'Right Canvas resizer handle must exist');
  assert.match(indexHtml, /data-latex-mode="preview"/, 'LaTeX Canvas must expose Split/Edit/Preview modes');
  assert.match(indexHtml, /id="session-search"/, 'Session filter input must exist');
  assert.match(indexHtml, /id="rail-tab-archived"/, 'Archived chats tab must exist');
  assert.match(indexHtml, /id="topbar-stop-btn"/, 'Topbar Stop button must exist');
  assert.match(indexHtml, /id="topbar-archive-btn"/, 'Topbar Archive button must exist');
  assert.match(indexHtml, /id="topbar-delete-btn"/, 'Topbar Delete button must exist');
  assert.match(indexHtml, /id="session-grants-pill"/, 'Session tool grants pill must exist');
  assert.match(assetsRs, /"\/assets\/workbench\.css"/);
  assert.match(assetsRs, /"\/assets\/trace_cards\.js"/);
  assert.match(assetsRs, /"\/assets\/workbench\.js"/);
  assert.match(assetsRs, /"\/assets\/session_rail\.js"/);

  // Import trace_cards.js by mapping /assets/math.js and /assets/source_links.js cleanly.
  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const srcLinksCode = fs
    .readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  const srcLinksDataUrl = JSON.stringify(`data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`);
  const traceSrc = fs
    .readFileSync(path.join(assetsDir, 'trace_cards.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl)
    .replace("'/assets/source_links.js'", srcLinksDataUrl);
  const dataUrl = `data:text/javascript;base64,${Buffer.from(traceSrc, 'utf8').toString('base64')}`;
  const traceMod = await import(dataUrl);

  const prevDoc = globalThis.document;
  try {
    globalThis.document = {
      createElement: (tag) => ({
        tagName: tag.toUpperCase(),
        className: '',
        dataset: {},
        innerHTML: '',
        querySelector: () => null,
      }),
    };

    const sympyCard = traceMod.buildStructuredTraceCard({
      call_id: 'c1',
      tool: 'sympy_eval',
      status: 'ok',
      args: { code: 'import sympy as sp\nx = sp.Symbol("<script>alert(1)</script>")' },
      output: JSON.stringify({ ok: true, result: '2*pi + <img src=x onerror=alert(1)>' }),
    });
    assert.match(sympyCard.innerHTML, /SymPy \/ Python Sandbox/);
    assert.match(sympyCard.innerHTML, /sc-result-row/);
    assertNoXss(sympyCard.innerHTML);

    const ledgerCard = traceMod.buildStructuredTraceCard({
      call_id: 'c2',
      tool: 'ledger_append',
      status: 'ok',
      args: { entry_type: 'derivation', summary: 'Check <script>evil()</script>', depends_on: ['e-1'] },
      output: JSON.stringify({ entry_id: 'e-2' }),
    });
    assert.match(ledgerCard.innerHTML, /Proof Step · derivation/);
    assert.match(ledgerCard.innerHTML, /Inspect e-2/);
    assertNoXss(ledgerCard.innerHTML);

    const verdictCard = traceMod.buildVerificationVerdictCard({
      verified_status: 'unverified',
      verification_rounds: 2,
      verifications: [{
        verdict: 'rejected',
        target_entry_id: 'e-2',
        summary: 'Sign mismatch <img src=x onerror=alert(2)>',
        issues: [{ severity: 'major', entry_id: 'e-2', description: 'Missing <script>x</script> factor' }],
      }],
    });
    assert.ok(verdictCard && verdictCard.className.includes('is-unverified'));
    assert.match(verdictCard.innerHTML, /Unverified Draft/);
    assertNoXss(verdictCard.innerHTML);

    // Stage 12 literature card links + XSS neutralization
    const litCard = traceMod.buildStructuredTraceCard({
      call_id: 'c3',
      tool: 'arxiv_search',
      status: 'ok',
      args: { query: 'kitaev' },
      output: JSON.stringify({
        papers: [
          {
            id: '2301.12345v1',
            title: 'Kitaev Chain <script>x</script>',
            url: 'javascript:alert(1)',
            doi: '10.1103/PhysRevB.1.1',
          },
        ],
      }),
    });
    assert.match(litCard.innerHTML, /https:\/\/arxiv\.org\/abs\/2301\.12345v1/);
    assert.match(litCard.innerHTML, /https:\/\/doi\.org\/10\.1103\/PhysRevB\.1\.1/);
    assertNoXss(litCard.innerHTML);
  } finally {
    globalThis.document = prevDoc;
  }

  for (const id of ['effort-popover', 'composer-effort-btn', 'composer-subagent-btn', 'settings-tab-subagents', 'settings-tab-guide', 'source-reader-dialog', 'explain-export-md-btn', 'explain-export-html-btn', 'set-auto-approve-tools', 'subagent-model-select']) {
    assert.match(indexHtml, new RegExp(`id="${id}"`), `Missing #${id} in index.html`);
  }
  assert.match(indexHtml, /Orchestrator/, 'UI must label Orchestrator');
  assert.match(indexHtml, /Explain/, 'UI must label Explain');
  assert.match(fs.readFileSync(path.join(assetsDir, 'subagents.js'), 'utf8'), /agent-model-select/, 'subagents.js must render per-agent model selectors');
  for (const a of ['/assets/modals.css', '/assets/effort_picker.js', '/assets/subagents.js', '/assets/guide.js', '/assets/source_links.js']) {
    assert.ok(assetsRs.includes(a), `Missing static asset registration ${a}`);
  }
});
