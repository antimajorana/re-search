# re:Search LaTeX / Markdown renderer — behavioural spec as of `main` @ 7c36a0e

## 0. Scope, method, limits

- **Read in full:** `crates/desktop-server/assets/math.js` (596 lines), `canvas_latex.js`, `crates/desktop-server/src/assets.rs`, `crates/desktop-server/tests/math_render.test.mjs`, `tests/ui_math.rs`, `vendor/katex/auto-render.min.js` (3.5 kB, minified), `investigation.css`.
- **Read in part:** every rule that can touch KaTeX or prose in `tokens.css`, `components.css`, `layout.css`, `workbench.css`, `dag.css`, `modals.css`; every call site of the `math.js` exports (grep plus surrounding lines); the relevant rules of `vendor/katex/katex.min.css`.
- **Probes:** node one-offs in `<local scratch file>` (`harness.mjs`, `probes1.mjs` to `probes4.mjs`). They import the real `math.js` and the vendored `katex.min.js`.
- **Stage 2 confirmation:** `probes3.mjs` runs the real vendored `auto-render.min.js` through the real `renderMathInContainer` against a minimal fake DOM. `katex.render` is stubbed to call `katex.renderToString` with the same options.
- **Existing suite:** `node --test crates/desktop-server/tests/math_render.test.mjs` gives 14 pass, 0 fail.
- **Not done:** no browser, no cargo, no Playwright. Every statement about visual layout (section 6) is inference from CSS, not observed pixels.
- **Not read:** the Kandinsky repo (not part of this task); the full body of `crates/export/src/html.rs` (only lines 1-20 and 40-70).
- **Repo state:** I made no change. `git status` now shows untracked `.gemini-task/` and `.github/workflows/gemini-task.yml`, which are not mine and appeared during the session.

## 1. Pipeline order

There are three entry points. All paths below are under `crates/desktop-server/assets/`.

### A. Rich Markdown: `renderRichInto(el, raw)` (`math.js:562-566`)

1. `markdownToHtml(raw)` (`math.js:387-394`):
   1. Strip all U+0000 and normalise CRLF/CR to LF (`:388`).
   2. `extractFencedCodeBlocks` (`:268-283`): fences become `<pre><code class="lang-X">` with an escaped body, replaced by placeholder `\0B<n>\0`.
   3. `extractMath(..., {protectInlineCode:true})` (`:93-148`), run over the **whole document before any block parsing**. Each formula becomes `\0M<n>\0`; the token stores `{tex, display}` after `normalizeMathTex`.
   4. `renderMarkdownBlocks` line by line (`:319-385`).
   5. `renderInlineMarkdown` per line or cell (`:248-266`), in this order: inline code to `<code>` tokens; `\$` to a placeholder; `[label](http…)` to link tokens; `escapeHtml` of everything left; `applyEmphasis`; then restore link, code, `$` and math tokens.
   6. Math is **not rendered here**. It is re-emitted as escaped text:
      - display token alone on a line: `<div class="math-block">$$\n{tex}\n$$</div>` (`:334-338`);
      - any other token: `$${tex}$$` or `\({tex}\)` inline (`formatMathTokenHtml`, `:235-237`).
2. `el.innerHTML = html` (`:564`).
3. `renderMathInContainer(el, {allowSingleDollar:false})` (`:150-157`) calls KaTeX auto-render (`window.renderMathInElement`). It **re-scans every text node of the container**, not only the placeholders.
   - Delimiters: `$$`, `\[`, the 30 `\begin{env}` pairs, `\(` (`:16-21`).
   - `preProcess` is `normalizeMathTex(t, true)`.
   - If auto-render is absent but `window.katex` exists, a TreeWalker fallback (`:158-172`) replaces each text node (not under CODE/PRE/SCRIPT/STYLE/TEXTAREA) with a `<span>` whose `innerHTML` comes from `renderInlineMathFallback`.

There is no separate sanitiser. Safety comes from escaping every text fragment and emitting only a fixed tag set: `p br h1-h6 hr blockquote ul ol li table thead tbody tr th td pre code strong em a div.math-block`. `<a>` is only produced for `https?://` URLs, with an escaped `href` and `target=_blank rel="noopener noreferrer"` (`:257-258`).

### B. Plain text with math: `mathTextToHtml(text, {inlineOnly})` / `renderMathTextInto(el, text, opts)` (`math.js:200-212`)

- If `window.katex.renderToString` exists and the text matches `/\$|\\[\[(]|\\begin\{/` (`:194-197`): `extractMath` with backtick protection and **single dollars allowed**, then `escapeHtml` on the text between tokens and `katex.renderToString` per token (`:175-192`).
- Otherwise: `escapeHtml` only (`mathTextToHtml`) or `textContent` (`renderMathTextInto`).
- **No Markdown at all.** Used for titles, labels, excerpts, activity notes and the whole Investigation map and conventions UI.

### C. TeX document preview: `renderLatexDocumentPreview` → `latexDocumentToHtml` (`math.js:535-574`)

1. Strip NUL and CRLF; strip `%` comments (`:396-408`).
2. Cut to the body between `\begin{document}` and `\end{document}`. **The preamble is discarded** except `\title` and `\author`.
3. `extractMath` with `protectInlineCode:false` on title, author and body.
4. `renderTexBlocks` (`:474-533`) handles:
   - `\section`, `\subsection`, `\subsubsection`, `\paragraph` → h2 to h5;
   - 13 theorem-like environments → `div.tex-env` with a title;
   - `itemize` / `enumerate` → flat `ul` / `ol`;
   - blank line or `\par` → paragraph break.
5. `renderTexInline` (`:427-472`) handles `~`, quote pairs, `--`, `---`, `\\`, `\textbf`, `\textit`, `\emph`, `\texttt`, `\cite` → `<code>[cite:…]</code>`, `\ref` / `\eqref` / … → `<code>[ref:…]</code>`, and drops a fixed list of commands.
6. Then `innerHTML` plus the same auto-render pass (`:568-574`).

Used by the manuscript canvas (`canvas_latex.js:64,402`) and the source reader for `.tex` / `.ltx` (`canvas_sources.js:840`).

### Helpers

- `displayMathSource(latex)` (`math.js:224-233`), for structured `latex` fields: returned unchanged if it starts with `$$`, `\[` or `\begin{`; otherwise one outer `\(..\)` or `$..$` layer is stripped and the result wrapped as `$$\n…\n$$`.
- `firstMathSentence` (`:216-222`): first `Intl.Segmenter` sentence with formulas kept atomic; **all math is re-emitted as inline `\(..\)`**.

## 2. Math delimiters and their rules (stage 1, `extractMath`, `math.js:93-148`)

| Delimiter | Mode | Rules |
|---|---|---|
| `\0…\0` placeholder | n/a | Skipped verbatim (`:107-110`). |
| Backtick span (when `protectInlineCode`) | not math | A run of n backticks to the next exact run of n, not crossing a blank line, is copied untouched (`:111-119`). |
| `\$` | literal | Never opens math (`:122`); printed as `$` by Markdown (`:256,264`). |
| `\[ … \]` | display | Closer is the first `\]` not preceded by an odd number of backslashes, anywhere later in the text; may cross lines, blank lines and paragraphs (`:123-127`, `:84-91`). **The opener is not checked for a preceding backslash.** |
| `\( … \)` | inline | Same closing rule with `\)`. |
| `\begin{env}…\end{env}` | always display | `env` must be one of 30 names (`:9-14`): equation(*), align(*), gather(*), alignat(*), split, aligned, gathered, cases, dcases, rcases, matrix, pmatrix, bmatrix, Bmatrix, vmatrix, Vmatrix, smallmatrix, array, CD, flalign(*), multline(*), eqnarray(*), subequations. Same-name nesting is counted (`:52-64`). Any length. The token keeps the `\begin`/`\end`. Unknown environments are not math. |
| `$$ … $$` | display | Opener not escaped; closer is the first unescaped `$$` anywhere later; content must be non-blank (`:137-140`). Multi-line and blank lines allowed. |
| `$ … $` | inline | See the rules below. Disabled in stage 2 of path A; enabled in stage 1 of A and C and in path B. |

Single-dollar rules (`findClosingInlineDollar`, `:66-82`):
- The character after the opening `$` must exist and be neither whitespace nor `$`.
- The scan **aborts** (no math; the `$` stays literal) on any of: a NUL; a blank line; an unescaped backtick (when code is protected); `\[`; `\(`; `\begin{`.
- At the first unescaped `$` it aborts if that `$` is followed by `$`, preceded by whitespace, or followed by a digit. Otherwise that `$` closes.
- Single newlines are allowed.

`normalizeMathTex` (`:32-44`):
- Removes every `\label{…}` (no nested braces).
- For display tokens (and for every formula in stage 2): drops `\begin/\end{subequations}`; maps `multline`→`gather`, `multline*`→`gathered`, `flalign`→`align`, `flalign*`→`align*`, `eqnarray`→`align`, `eqnarray*`→`aligned`; rewrites the first `& x &` per eqnarray row to `&x `; trims.
- An empty result drops the token entirely (`:100`).

Stage 2 (vendored auto-render) differs from stage 1:
- It takes the earliest left delimiter in each text run.
- It finds the right delimiter only at brace depth ≤ 0, skipping the character after any backslash.
- If no closer is found it **stops processing the rest of that text run**.
- It has no currency logic, but `$` is not in its delimiter list on paths A and C.

Currency summary:
- `$5 and $10` stays text (closing `$` preceded by a space).
- `$5-$10` stays text (closing `$` followed by a digit).
- `\$5` stays text.
- `paid $5 … got 10$ back` becomes math (see D7).

## 3. KaTeX options

| Path | Call | Explicit options |
|---|---|---|
| Auto-render (A, C, hero, worked-example dialog) | `window.renderMathInElement(el, …)` (`math.js:155`) | `delimiters`, `throwOnError:false`, `strict:false`, `preProcess` |
| Label/fallback (B, and the TreeWalker fallback) | `katex.renderToString(tex, …)` (`math.js:186`) | `displayMode: tok.display && !inlineOnly`, `throwOnError:false`, `strict:false` |

- On the auto-render path the option keys reaching `katex.render` were observed (probe R1) as: `delimiters, displayMode, errorCallback, ignoredClasses, ignoredTags, macros, preProcess, strict, throwOnError`.
- auto-render supplies:
  - `ignoredTags` = script, noscript, style, textarea, pre, code, option;
  - `ignoredClasses` = [];
  - `errorCallback` = console.error;
  - `macros` = `{}`, **one object shared by all formulas of that single call**.
- **Not set anywhere, so KaTeX 0.19.1 defaults apply:**
  - `output` — observed: both MathML and HTML are emitted;
  - `trust` — observed off: `\href`, `\htmlClass`, `\includegraphics` render as red command names;
  - `errorColor` — observed `#cc0000`;
  - `maxExpand` — observed to stop a runaway macro with "Too many expansions";
  - `macros` — none predefined;
  - `maxSize`, `fleqn`, `leqno`, `minRuleThickness`, `globalGroup`, `colorIsTextColor` — defaults not probed.
- Macro persistence (probe R6):
  - `\gdef` in one formula is visible to later formulas of the same container render;
  - `\newcommand` is local to its formula;
  - path B never shares macros.
- Vendored bundle (`crates/desktop-server/vendor/katex/`):
  - `katex.min.js` 0.19.1, `auto-render.min.js`, `katex.min.css`, `LICENSE` (MIT), and 20 `.woff2` fonts;
  - **no** `contrib/` — no mhchem, copy-tex, mathtex-script-type or render-a11y-string;
  - the CSS also references `.woff` and `.ttf` fallbacks that are not shipped.
- Loading and serving:
  - classic scripts at `index.html:764-765`, before the module `app.js` (`:766`); CSS at `index.html:10`;
  - all embedded with `include_bytes!` and served from fixed paths (`src/assets.rs:211-302`), `Cache-Control: public, max-age=3600` (`src/http_srv.rs:145`);
  - the page CSP (`src/http_srv.rs:132`) has no `style-src` / `script-src`, so KaTeX's inline `style=` attributes are allowed.

## 4. Markdown support (path A)

**Supported**

| Construct | Behaviour |
|---|---|
| Paragraphs | Soft line breaks join with a space. |
| Hard break | Two or more trailing spaces, or a trailing `\` (`:312-313`). |
| ATX headings | `#` to `######` at column 0, followed by whitespace (`:344`). |
| Horizontal rule | `***`, `---`, `___`, including spaced forms (`:346`). |
| Blockquote | `>` lines, recursive (`:341`, `:326`). |
| Lists | `-`, `*`, `+` and `1.` / `1)`; nested by indent, tab = 2 spaces (`:361-379`). |
| Tables | GFM style, with or without outer pipes; `:---`, `:---:`, `---:` alignment as inline `style`; `\|` escape (`:347-360`). |
| Fenced code | Backtick or `~~~`, ≤ 3 spaces indent, language class `lang-<first word>`; an unclosed fence runs to the end (`:268-283`). |
| Inline code | Any backtick run length, single line. |
| Emphasis | `***x***`, `**x**`, `__x__`, `*x*`, `_x_`; snake_case is protected (`:239-246`). |
| Links | `[label](http(s)://…)` only. |

**Not supported** (shown as literal or escaped text unless noted)

- Raw HTML, including `<br>`, `<sub>`, `<details>`.
- Images: `![a](url)` renders as `!` followed by a link.
- Relative, `mailto:` and reference links; link titles; autolinks `<https://…>`; bare-URL linkification.
- Footnotes, strikethrough, task-list checkboxes.
- Setext headings; a heading with leading spaces; 7 or more `#`.
- Indented code blocks.
- Fences inside blockquotes.
- Lazy or indented continuation lines inside list items.
- "Loose" lists (blank lines between items).
- The `start` number of an ordered list.
- Table delimiter cells with fewer than three dashes.
- `<p>`-wrapped list items, definition lists.

## 5. How formula errors are shown

- **Parse error** (unbalanced braces, `\tag` in inline math, `align` in inline math, `_` inside `\text{}`, unknown environment): the whole formula is replaced by `<span class="katex-error" title="ParseError: KaTeX parse error: …" style="color:#cc0000">raw TeX</span>`. The message is only in the hover tooltip.
- **Undefined control sequence** (`\foo`, `\dv`, `\abs`, `\qty`, `\slashed`, `\ce`, `\eqref`, `\ref`, `\mbox`, `\vspace`, `\unicode`, `\require`, and `\href` / `\htmlClass` / `\includegraphics` because trust is off): not an error. The rest of the formula renders and the command name appears in red (`color:#cc0000`). No `katex-error` class, no tooltip.
- **Auto-render path:** a non-ParseError exception from KaTeX is re-thrown out of `renderRichInto`.
- **Label path:** any exception falls back to escaped raw `$$tex$$` / `\(tex\)` (`math.js:187`).
- **Not recognised or unclosed math** (streaming partials, currency) stays as plain text with the delimiters visible.
- **KaTeX scripts not loaded:** path A shows raw `$$…$$` / `\(…\)`; path B shows escaped text.
- No project CSS targets `.katex-error`; the red is fixed in both light and dark themes.

## 6. Display-math layout CSS (inference from CSS; not seen in a browser)

- **Size:** `.katex { font-size: 1.06em }` (`tokens.css:97-99`) overrides KaTeX's 1.21em, so math is 1.06× the surrounding text. Body text is 15px (`layout.css:14`). KaTeX keeps its own `line-height: 1.2`; assistant prose uses 1.66 (`components.css:170`).
- **Alignment:** KaTeX default, centred (`.katex-display > .katex { text-align:center; white-space:nowrap }`). No `fleqn` or `leqno`.
- **Display wrapper:** `.katex-display { margin:.9em 0; padding:.2em .15em; overflow-x:auto; overflow-y:hidden }` (`tokens.css:101-106`). Wide formulas scroll horizontally inside their own block.
- **Markdown-level block:** also wrapped in `.rich-prose .math-block { margin:.75em 0; overflow-x:auto; overflow-y:hidden }` (`components.css:181`). That gives two nested scroll containers; the two margins most likely add rather than collapse.
- **Tags and numbering:**
  - `\tag{…}` works in display mode only; KaTeX positions `.katex-tag` absolutely at `right:0` of the formula box.
  - Un-starred `equation`, `align`, `gather` get automatic numbers from a CSS counter that KaTeX resets once on `body`. Numbers therefore run through the whole app page in DOM order, across unrelated messages and panes.
  - No project CSS touches tags or counters.
  - `\label` is stripped; `\eqref` / `\ref` never resolve.
- **Inline math:**
  - wrapping is KaTeX default (breaks only between top-level relation/binary chunks);
  - only the DAG cards override it: `.dag-node-summary .katex, .dag-list-sentence .katex { white-space: normal }` (`dag.css:89`);
  - nothing gives over-wide inline formulas or `.rich-prose table` (`width:100%`, `components.css:204`) a scroll wrapper.
- **Other containers:**
  - `.dag-eq-box { overflow-x:auto }` (`dag.css:216-222`);
  - `.thinking-notes { white-space: pre-wrap }` (`components.css:226`);
  - `.investigation-inner p { white-space: pre-line }` (`investigation.css:13`);
  - `.investigation-node > span { overflow-x:auto }` (`investigation.css:65`).
- **Prose:** `.rich-prose` rules are at `components.css:172-206`. There is no `.rich-prose a` or `img` rule. `#latex-preview` is 14px (`components.css:399`).
- **Descendant-selector rule:** "never use bare descendant span/div/* selectors inside KaTeX containers" is stated in `workbench.css:7` and `dag.css:2`. My scan of all CSS found only `.provider-callout span` (`components.css:114,116`), `label span` (`investigation.css:14`) and `.theme-switching *` (`tokens.css:112`). None wraps math today.

## 7. `innerHTML` insertion points for rendered output

Inside `math.js`:

| Site | Content | Escaped? |
|---|---|---|
| `:564` (`renderRichInto`) | `markdownToHtml` output | Yes; all text through `escapeHtml`; href escaped and http(s)-only. |
| `:572` | `latexDocumentToHtml` output | Yes (`renderTexInline` escapes every char; env name and title escaped). |
| `:571` | static "Empty TeX buffer." | Constant. |
| `:210` (`renderMathTextInto`) | `mathTextToHtml` | Text escaped; math is KaTeX output with trust off. |
| `:169` (TreeWalker fallback) | `renderInlineMathFallback` | Same as above. |

Callers that interpolate `mathTextToHtml(...)` into template strings assigned to `innerHTML` (model, tool or paper strings; all go through the escaping above):
- `stream.js:351`
- `session_rail.js:111`
- `hitl.js:54,231-234,247`
- `project_library.js:168-171`
- `canvas_sources.js:334-335,342,417,466`
- `trace_cards.js:40,43,70,73,98,127-128,231,251,257,486,493,495`

Adjacent values in those templates use `escapeHtml`.

Other notes:
- `trace_cards.js:430,447` first insert `escapeHtml(summary)` / `escapeHtml(recovery)`, then overwrite with `renderRichInto` (`:453-462`).
- `renderRichInto` targets: `stream.js:236,299,424,462,493,504`; `canvas_dag_detail.js:348,512,515,532`; `canvas_sources.js:612-613,841`; `claim_trace.js:163,259`; `hitl.js:108,110,115`; `projects.js:188-189,436`; `project_library.js:180`; `project_validation.js:247`; `trace_cards.js:348,453,456,462`.
- `renderMathInContainer` is called directly, with single dollars allowed, on static app markup only: the hero (`stream.js:217`) and the worked-example dialog (`stream.js:197`, markup at `index.html:741-755`).
- I found no path where model, provider or file text reaches `innerHTML` unescaped. KaTeX output itself is trusted, with `trust` off.

## 8. What the tests cover

`tests/math_render.test.mjs` (14 tests; run from cargo by `tests/ui_math.rs`; CI also runs all `*.test.mjs`, `.github/workflows/workbench-checks.yml:34`):

- KaTeX version is 0.19.1.
- Label path: text, currency, backtick code, escaped HTML, three formulas, no XSS.
- `displayMathSource` accepts bare, `$`, `\(`, `$$`, `\[` forms.
- CSS audit: no bare `span`/`div`/`*` descendant selectors after 10 named KaTeX containers, in `components.css`, `layout.css`, `workbench.css` only.
- All 30 environments plus `$$`, `\[`, `\(`, `$` produce a `.math-block` (or inline) whose extracted TeX compiles with `throwOnError:true`.
- Nested environments stay one block; `\label` stripped.
- Markdown block structure: h1/h4/h5/h6, soft and hard breaks, hr, blockquote, a nested tight list with `1)`, a table with alignment and `$|x|$`, backtick and `~~~` fences.
- Inline edge cases: prices, `\$`, `a*b`, snake_case, inline code, bold/italic.
- Full TeX article preview; XSS and NUL-placeholder spoofing in Markdown and TeX.
- TreeWalker fallback with a mocked DOM.
- Two unrelated UI-wiring tests.

Other tests touching math:
- `tests/investigation_origin.test.mjs:197-200` — KaTeX in convention labels.
- Playwright scripts `research_display_browser.py:118,163-171,189`, `investigation_browser.py:172,221,243`, `recovery_example_browser.py:60` — assert `.katex` / `.katex-display` visibility, and that DAG cards contain no `.katex-display`.

**Not covered by any test:**
- the real auto-render second pass;
- loose or renumbered lists; math inside list items or blockquotes;
- `$…$` containing an environment;
- `\tag`, `\eqref`, equation numbering;
- error rendering;
- wide-formula overflow;
- streaming partials;
- macro handling.

## 9. Probes (input → output)

"HTML" is `markdownToHtml` output. "S2" is what the real auto-render then renders (confirmed for R-numbered probes; emulated identically for the rest).

| # | Input | Output |
|---|---|---|
| P01 | `Before⏎⏎$$⏎E = mc^2⏎+ \frac{1}{2}mv^2⏎$$⏎⏎After` | `<p>Before</p>`, `<div class="math-block">$$…$$</div>`, `<p>After</p>`. S2: one display formula, ok. |
| P02 | `$$⏎a = b⏎⏎+ c⏎$$` (blank line inside) | One math-block; KaTeX ok. |
| P03 | `Text before⏎\begin{align*}…\end{align*}⏎text after` | Paragraph, math-block containing `$$⏎\begin{align*}…\end{align*}⏎$$`, paragraph; ok. |
| P04 | `$$⏎f(x) = \begin{cases} … \end{cases}⏎$$` | One math-block; ok. |
| P05 | bare `\begin{pmatrix} 1 & 0 \\ 0 & -1 \end{pmatrix}` on its own line | math-block, display, ok. |
| P05b | `The matrix \begin{pmatrix}…\end{pmatrix} is diagonal.` | `<p>The matrix $$…$$ is diagonal.</p>` — a centred display block in mid-sentence. |
| P05c / R1 | `Let $M = \begin{pmatrix}…\end{pmatrix}$ be diagonal.` | `<p>Let $M = $$\begin{pmatrix}…$$$ be diagonal.</p>`. DOM: `"Let $M = "` [display matrix] `"$ be diagonal."` — stray dollars, `M =` not math. |
| P05d | same with `\( … \)` | Inline, ok. |
| P06 | `$$ E = mc^2 \tag{1.2} $$` | Display, ok, tag rendered. |
| P06b | `Inline $E = mc^2 \tag{3}$ here.` | katex-error: "\tag works only in display equations". |
| P06c | `\begin{align} a &= b \tag{A} \\ c &= d \notag \\ e &= f \end{align}` | ok, with auto-number markup. |
| P07 / R7 | `\begin{equation}\label{eq:one} a = b \end{equation}` then `See \eqref{eq:one} and Eq. $\eqref{eq:one}$ and \ref{eq:one}.` | `\label` stripped; equation auto-numbered. Bare `\eqref{…}` / `\ref{…}` stay raw text; `$\eqref{eq:one}$` renders the command name in red. |
| P08 | `$\text{foo_bar} = x_1$`, `$\text{foo\_bar}$`, `$\mathrm{max\_iter}$`, prose `snake_case_name` | First: katex-error "Expected 'EOF', got '_'". Second and third ok. Prose untouched. |
| P09 | `A line with two prices like $5 and $10.` | Unchanged text, no math. |
| P09b | `I paid $5 for apples and my friend paid 10$ for pears.` | `\(5 for apples and my friend paid 10\)` — rendered as math. |
| P09c | `It costs $5 but $x^2$ is free, total $12.50.` | Only `x^2` is math. |
| P09d | `Escaped \$5 and \$10, then $a$.` | `Escaped $5 and $10, then \(a\).` |
| P10 | `Compute $a*b$ and $c*d$, then **bold $x*y$ bold** and *em $z$*.` | Four inline formulas intact; `<strong>` and `<em>` correct. |
| P11 | table with `:---` / `---:`, `$|x|$`, `**value**` | `<table>` with inline `text-align` styles; `\(|x|\)` stays in one cell. |
| P11b / P45 | delimiter row `|--|--|` or `|:-:|:-:|` | Not a table: `<p>| a | b | |--|--| | 1 | 2 |</p>`. |
| P12 | `- A⏎  - A1⏎    - A1a⏎  - A2⏎- B⏎  1. B1⏎  2. B2` | Correctly nested `ul > ul > ul` and `ul > ol`. |
| P13 | `1) first⏎2) second⏎3) third` | One `<ol>` with three items. |
| P13b | `1. first⏎⏎2. second⏎⏎3. third` | **Three separate `<ol>`**, each with one item. |
| P13c | `3. three⏎4. four` | `<ol>` with no `start`. |
| P13d | `1. First step:⏎   $$x = 1$$⏎   continues here⏎2. Second step` | `<ol>` (First step) closed; math-block; `<p>   continues here</p>`; new `<ol>` (Second step). |
| P14 | fenced bash with `$HOME`, `$5`, `$((1+2))`, `\(not math\)`, then `then $x$.` | `<pre><code class="lang-bash">` escaped and untouched; only the trailing `x` is math. |
| P14b | unclosed fence | Rendered as a code block to end of text. |
| P14c | fence indented inside a list item | List closed, code block (indent kept), new list. |
| P15 | bare `\begin{equation*}…\end{equation*}` | math-block, display, ok, unnumbered. |
| P15b | two bare `\begin{equation}` | Two math-blocks, each with auto-number markup. |
| P16 / R9 | `> $$⏎> a = b⏎> $$` | TeX handed to KaTeX is `> a = b⏎>`; the quote markers end up inside the formula. |
| P17 / R4 | `Type $$ to open a display.⏎⏎Second paragraph with $x$.⏎⏎Then $$ closes it.` | One `<p>`; everything between the two `$$` becomes one display formula, which is a katex-error ("Can't use function '$' in math mode"). |
| P18 / R2 | `Literal \$\$x^2\$\$ should stay text.` | Stage 1 gives `<p>Literal $$x^2$$ …</p>`; stage 2 then renders `x^2` as display math. |
| P19 / R3 | `Use \begin{align} for alignment, e.g. \(x^2\) or $y$.` | Nothing rendered; `\(x^2\)` and `\(y\)` stay raw. |
| P19b | `The opener \[ starts display math; inline is \(x\).` | Same: `\(x\)` stays raw. |
| P20 | `As shown in \[1\] and \[2, 3\].` | Two display formulas "1" and "2, 3". |
| P21 | image, relative link, mailto, autolink, bare URL, titled link | `!` + link for the image; the rest literal text. |
| P22 | `a<br>b <sub>2</sub> ~~gone~~`, task list, footnote | All literal or escaped. |
| P23 | `# h1`, `## h2 with $x^2$`, `####### seven`, `#nospace`, `   ### indented`, setext, `## trailing ##` | h1 and h2 (with math) ok; the next five lines merge into one `<p>`; `<h2>trailing ##</h2>`. |
| P24 | `a␠␠⏎b\⏎c⏎d<br>e` | `<p>a<br>⏎b<br>⏎c d&lt;br&gt;e</p>`. |
| P25 | 4-space indented code; nested quote; fence in quote | Indented code becomes a paragraph with math rendered; nested quote ok; fence in quote becomes a paragraph with literal backticks. |
| P29 | `Powers 2**3 and 4**5; …` | `2<strong>3 and 4</strong>5`. |
| P30 | unclosed `$x = \frac{1}{2` and `$$\int_0^1` | Both stay raw text. |
| P31 | `$x$2`, `$n$-th`, `5$x$`, `$ x $` | `$x$2` not math; `$n$-th` and `5$x$` math; spaced form not math. |
| P33 | eqnarray, multline, subequations+align | Rewritten to align, gather, align; all ok with auto-number. |
| P36 | `[a](https://en.wikipedia.org/wiki/Foo_(bar))` | href cut at `Foo_(bar`; a stray `)` follows the link. |
| P41 | `Row one \\[4pt] row two, later \[ y \] done.` | One display formula `4pt] row two, later \[ y`. |
| T01 | TeX preview: `Line one\\[2mm]⏎Line two is prose.⏎⏎\[ x = 1 \]⏎After.` | The prose is swallowed into one display formula `2mm]⏎Line two is prose.⏎⏎\[ x = 1`. |
| T02 | TeX preview with figure, tabular, `\footnote`, `\textsc`, `\url`, `\href` | All shown as raw TeX source in a paragraph. |
| T03 | TeX preview: enumerate containing itemize | Flat `<ol>` with literal `\begin{itemize}` / `\end{itemize}` text in items. |
| T04 | preamble `\newcommand{\ket}…`, `\DeclareMathOperator{\tr}{tr}`, body `$\tr \rho$` | Preamble discarded; `\tr` renders as a red command name. |
| M01 | `mathTextToHtml('**bold** _it_ `code $x$` # h⏎- item $y$')` | Text verbatim (asterisks, backticks, `#`, `-` all visible); only `y` is math. |
| M03 / M03b | `Result \[ a = b \] and $$c$$ …` | Three display blocks inside the label; with `inlineOnly:true` they render inline. |
| M04 | `Bad $\frac{1}{$ and $\foo$` | First: katex-error span with tooltip. Second: red `\foo`. |
| R6 | `$\gdef\myop{…} \myop x$ then $\myop y$ then $\newcommand{\zz}{Z}\zz$ then $\zz$` | `\myop` works in the second formula; `\zz` is red in the fourth. |
| R10 | direct auto-render with single dollars: `Costs $5 and $10; investigate $e^{A+B}=e^A e^B$ and \(x\).` | `5 and` rendered as math. |
| KaTeX direct | `\bra{\psi}\ket{\phi}\braket{a|b}`, `\bm`, `\boldsymbol`, `\mathbb`, `\mathscr`, `\cancel`, `\boxed`, `\hbar` | Supported. `\dv`, `\abs`, `\qty`, `\slashed`, `\ce`, `\mbox` unsupported (red). `\newcommand{\R}…` errors because `\R` is a KaTeX built-in. |
| `displayMathSource` | `E = mc^2 where m is mass` → `$$⏎E = mc^2 where m is mass⏎$$`; `$a$ and $b$` → `$$⏎$a$ and $b$⏎$$` | Mixed prose is forced into one display formula. |

## 10. Demonstrated defects

**Math detection**

- **D1. `$…$` containing any environment is broken** (P05c/R1). `findClosingInlineDollar` aborts on `\begin{` (`math.js:73`), so the dollars stay literal and the environment becomes a display block. `$M = \begin{pmatrix}…\end{pmatrix}$` is very common in model output.
- **D2. Double scanning.** Stage 2 re-parses all text, so stage-1 decisions are not final.
  - (a) `\$\$x\$\$` becomes display math (P18/R2).
  - (b) A literal unclosed `\begin{align}`, `\[`, `\(` or `$$` in a paragraph suppresses every later formula in that text run (P19, P19b, P37).
- **D3. `\\[` is treated as a display opener** (`math.js:120-127` never checks whether the backslash is itself escaped). A TeX line break with optional spacing swallows prose up to the next `\]` (T01, P41). This matters most for the `.tex` reader and manuscript preview.
- **D4. A stray `$$` pairs with the next `$$` anywhere in the document** and turns everything between into one failing formula (P17/R4). The same holds for `\[` and `\(`: there is no paragraph bound.
- **D5. Math inside blockquotes.** Math is extracted before block parsing, so `>` markers land inside the TeX (P16/R9).
- **D6. Bare matrix-type environments in mid-sentence become display blocks** (P05b): `pmatrix`, `cases`, `array`, `aligned` are always `display:true` (`math.js:19`).
- **D7. Currency false positive** (P09b): `$5 … 10$` renders as math. On the hero/dialog path, which uses raw auto-render with `$`, `$5 and $10` would too (R10); that path only sees static app text today.
- **D8. `\[1\]`-style escaped brackets become display math** (P20).

**Lists and blocks**

- **D9. Loose lists break** (P13b, P43). A blank line closes the list (`math.js:340`). Because `<ol>` has no `start`, each restarted list begins again at 1.
- **D10. Ordered-list start number is dropped** (P13c).
- **D11. List items cannot hold display math, continuation paragraphs or code fences** (P13d, P14c, P40); each closes the list.
- **D12. Tables need ≥ 3 dashes per delimiter cell** (P11b, P45); otherwise the table collapses into one paragraph.
- **D13. Headings:** an invalid heading line swallows the following lines into one paragraph; no setext; trailing `#`s are kept (P23).

**Inline Markdown**

- **D14.** `2**3 and 4**5` is bolded (P29).
- **D15.** URLs containing `)` are truncated (P36).
- **D16.** Images render as `!` plus a link (P21).
- **D17.** No raw HTML, strikethrough, task lists, footnotes, autolinks or indented code (P21, P22, P25). Some of this may be intended for safety.

**KaTeX behaviour and options**

- **D18. No cross-references.** `\label` is stripped; `\eqref` / `\ref` are raw text in prose and a red command name in math (P07/R7).
- **D19. Equation numbers are page-global.** Un-starred environments number from a `body`-level CSS counter, across all messages and panes. This is inference from KaTeX's CSS plus the observed auto-number markup; not seen in a browser.
- **D20. `\tag` in inline math makes the whole formula an error** (P06b). `firstMathSentence` and `inlineOnly` (DAG cards) force display formulas inline, so a recorded step that uses `\tag` or `align` fails on the card while rendering in the detail pane. The `\tag` half is probed; the `align`-inline half follows from the direct KaTeX check "{align} can be used only in display mode".
- **D21. No macro layer.** No `macros` option and no physics/braket-style shims (`\dv`, `\abs`, `\qty`, `\slashed`, `\ce` unsupported). `\newcommand` does not carry between formulas (R6). The TeX preview discards preamble definitions (T04).
- **D22. Errors are low-information.** A parse error shows raw red TeX with the reason only in a tooltip; undefined macros silently go red; `_` inside `\text{}` kills the formula (P08).

**Surface inconsistencies**

- **D23. Label surfaces do not render Markdown** (M01). Every `renderMathTextInto` / `mathTextToHtml` site, including the Investigation map, summary parts, branch results and conventions (`investigation_map.js:111-481`, `investigation_conventions.js:88-136`), shows `**bold**`, backticks and list markers literally.
- **D24. `displayMathSource` forces mixed prose-and-math `latex` fields into one display formula** (probe table).
- **D25. TeX preview gaps** (T02, T03): figure, tabular, footnote, `\textsc`, `\url`, `\href` and nested lists show as raw source.
- **D26. Cost during streaming** (code reading, not measured): `handleTextDelta` re-runs `renderRichInto` on the entire accumulated text every animation frame (`stream.js:412-432`), re-typesetting every formula each time.
- **D27. Layout risks** (CSS inference only): doubled vertical margin and nested scrollers around block math; absolutely positioned tags can overlap wide formulas; no scroll wrapper for wide tables or unbreakable inline math; no `.rich-prose a` styling; `.verdict-issue-desc` is a `<span>` without `.rich-prose` that receives block output (`trace_cards.js:424,462`).
- **D28. Separate renderer in export.** `crates/export/src/html.rs:61` writes `\[…\]` into its own `.math-block` and escapes prose into `<p style="white-space:pre-wrap">`. The standalone HTML export does not share this renderer. I did not find a KaTeX script in the parts I read, and did not read the whole file.
