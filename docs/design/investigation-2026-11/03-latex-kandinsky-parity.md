# LaTeX parity plan: re:Search adopts Kandinsky's math rendering behaviour (final)

## 1. Summary for the user

- re:Search will draw math the way Kandinsky's reading view does: the same KaTeX settings, the same nine shortcut macros, macros that persist, red errors in the theme colour, centred displays that scroll on their own, and equation numbers that sit to the right of the formula and never overlap it.
- re:Search will number equations itself and stop relying on KaTeX's page-wide counter. `\label` and `\eqref` become clickable `(n)` links.
- Answers will be rendered in one pass, and each formula will be cached. That fixes about twenty scanner bugs: stray `$`, `>` leaking into TeX, broken lists, and `[1]` turning into math. It also stops the whole message being re-typeset on every streaming frame.
- In a few places re:Search deliberately keeps or improves on its own behaviour, and each is labelled "deviation": `\(…\)` stays math, single newlines still join, per-row equation numbers work, and macros reset when you switch session.
- No new dependency is needed. It uses only the vendored KaTeX 0.19.1. The work reimplements behaviour; no Kandinsky code is copied.
- The work is six packages, each testable with `node --test`. A final pass in the real WebView2 window checks the visual details.

## 0. Basis and limits

- **Inputs:** the original plan, the verifier's report, and my own checks of the points the verifier disputed.
- **What I checked myself (read-only):**
  - Kandinsky `ui/src/styles/markdown.css:131-146` and the bidi/direction grep across `ui/src/styles`.
  - Kandinsky `ui/src/lib/markdown.ts:419-424` and `ui/src/lib/equations.ts:16,151-165` (grep).
  - re:Search `tests/ui_math.rs` (whole file), `assets/tokens.css:95-106`, `assets/math.js:195-215,558-575`, and greps for `displayMathSource`, `selectSession` and `auto-render`.
- **Probes I ran:** two KaTeX probes against `crates/desktop-server/vendor/katex/katex.min.js`, saved as `<local scratch file>` and `<local scratch file>`.
- **Verifier claims I confirmed:**
  - Kandinsky forces LTR and bidi isolation only in the editor (`chrome.css:181-182`). The reading view sets only the size (`markdown.css:133`).
  - Kandinsky's math CSS is scoped to `.markdown`. Only the error colour and tag padding rules are global (`markdown.css:146,292`).
  - Kandinsky skips app numbering when a display already contains `\tag` (`markdown.ts:421`), and it renames `align`/`gather`/`eqnarray` to `aligned`/`gathered` (`markdown.ts:424`).
  - `aligned` with two `\tag`s gives a `katex-error`.
  - `$$ x #1 $$` gives one `katex-error` and **no** `katex-display`.
  - `align*` and `gather*` with per-row `\tag`/`\notag`, including a nested `cases`, render with no error.
  - `\text{(1)}` renders.
  - KaTeX 0.19.1 marks tags `katex-tag` and auto-numbers with `eqn-num`, a CSS counter reset on `body` in `katex.min.css`.
  - `tests/ui_math.rs:8-10` runs only `math_render.test.mjs`.
  - `displayMathSource` feeds `renderRichInto` (`assets/trace_cards.js:348`, `assets/canvas_dag_detail.js:348,515`).
  - `auto-render.min.js` is referenced at `assets/index.html:765`, `src/assets.rs:215-217` and `tests/research_display_browser.py:24`.
  - Session switching lives in `assets/app.js:531` (`selectSession`).
- **Taken from the verifier, not re-checked by me:** the Kandinsky line references in `math.ts`, `math-align.ts`, `equations.ts:98,103,148-153`, `pigment-pills.ts:157-183`, `Markdown.svelte:49-51`, `preview.ts` and `markdown.ts:28-34,40,108,126-130,371,512-515,568-591`; and `math_render.test.mjs:61-68,75-91,392-427,528-534`.
- **Not verified by anyone:**
  - What lands on the clipboard when rendered math is copied, in either app.
  - Kandinsky's 15.5px body size.
  - Mermaid/tikz, footnotes, linkify, callout collapsing.
  - How auto-render behaves after an unmatched opener (D2b).
  - `crates/export/src/html.rs`.
  - The full text of `canvas_latex.js`.
- **Not done:** no cargo, no Playwright, no browser. Nothing was edited in either repo. Every layout statement is inference from CSS.

## 2. Final parity table

Status values:

- **P**: match Kandinsky (K).
- **D**: deliberate deviation; the reason is given.
- **F**: fix of a re:Search defect that K does not have, or does not address.
- **K-quirk**: K behaviour we do not copy.

### Delimiters

| Behaviour | Kandinsky reading view | re:Search today | Target | Status |
|---|---|---|---|---|
| Inline `$…$` | Opener not followed by whitespace or end. Closer: next `$` not preceded by whitespace, not followed by a digit, and not escaped by a single backslash. Non-empty. May cross a soft break. | Same core rules, but aborts on `\begin{`, `\[`, `\(` and a blank line (D1). Counts the full backslash run. | K rules. Keep the full backslash-run count (better than K's single-backslash check). Keep the paragraph bound. Do not abort on `\begin{`. Extra currency guard: a body that starts with a digit and contains a space-separated word is not math. | P + D (currency guard, escape run) |
| `$…$` holding an environment | Renders inline | Stray `$` plus a display | Inline formula | P |
| `\$` | Literal `$` | Literal at first, but `\$\$x\$\$` later becomes math (D2a) | Literal; single pass removes the re-scan | P/F |
| `$$…$$` at line start | Block display. Interrupts paragraph, list and quote. Indented by fewer than 4 columns. | Extracted before block parsing | Block rule run inside the container parser | P |
| `$$…$$` mid-paragraph | Display-mode formula as its own centred block in the paragraph. K does not strip `\label` there, does not check escapes and applies no heading bound. | `$$` re-emitted, then auto-render | `span.math-display` with a display render, using the same preparation as block displays (labels removed, escape check, heading bound) | P + D (consistent preparation) |
| Closer mid-line; `$$$$` | Text after a mid-line closer becomes a following paragraph. `$$$$` closes one display and opens the next. | Not handled | Implement both | P |
| Unclosed `$$` / `\[` | Stays text. The closer must come before the next ATX heading or the end of the text. | Pairs with any later `$$` (D4) | Same as K. While streaming, a half-arrived display shows as source. | P |
| Blank line inside a display | Allowed | Allowed | Keep | P |
| `\[…\]` at line start | Block display | Display anywhere | Block rule | P |
| `\[…\]` mid-line | Not math. Backslash-escape processing drops the backslash, so `\[1\]` shows as `[1]`. | Display anywhere, including `\[1\]` (D8) and `\\[4pt]` (D3) | Math only when: the opener is not preceded by an odd backslash run; the closer is in the same paragraph; and the body is not only digits, commas, spaces and dashes. Otherwise literal (see Decision 3 for the backslash). Keeps test `math_render.test.mjs:75-91`. | D |
| `\(…\)` | Not math (shows `(x)`) | Inline | Keep as inline math, bounded to one paragraph, opener not escaped. Models emit it heavily. | D |
| Bare `\begin{env}` at line start | Display only for `equation`, `align`, `gather`, `multline`, `eqnarray` (and starred forms). Other environments are prose. Trailing text after `\end{…}` on the same line goes into the TeX. | Display for 30 environments, anywhere (D6) | All 30 environments at line start (superset; required by `math_render.test.mjs:142-188`). Text after `\end{…}` becomes a following paragraph. Mid-sentence: inline for matrix types, `cases`, `aligned`, `gathered`, `split`, `array`, `smallmatrix`; a display span for the numbered families. | D |
| Math in code | Never scanned | Same in stage 1; stage 2 skips `pre`/`code` | Keep | P |
| Indented code | Disabled; a 4-space line is prose | Not supported | Keep | P |
| Math in blockquote / list item | Prefixes never reach the TeX | `>` lands in the TeX (D5); the list closes (D11) | Container-aware block parser | P |
| Punctuation glue | Up to 4 trailing characters from `. , ; : ! ? ) ]` and curly closing quotes wrapped with an inline formula in a no-wrap span | None | `span.math-glue`, rendered mode only, inline formulas only | P |
| HTML comments | `<!-- … -->` outside code removed, even inside formulas | Escaped and shown | See Decision 4 (default: strip outside code) | P (default) |

### Environments

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| `equation`, `multline` | Wrapper removed | `multline`→`gather`; `equation` auto-numbered by KaTeX | Wrapper removed; the app adds `\tag{n}` when the display is numbered | P |
| `align`, `gather`, `eqnarray` | Renamed to `aligned`/`gathered`; two or more tags then fail with "Multiple \tag" | Passed to KaTeX and auto-numbered | Unnumbered or single-number display: `aligned`/`gathered` plus one trailing `\tag`. Per-row numbers: the starred twin (`align*`, `gather*`) with one `\tag`/`\notag` per row (probe: works). | P + K-quirk avoided |
| `alignat`, `flalign`, `subequations`, `eqnarray` column fix | Not handled; un-starred `alignat`/`flalign` leak a KaTeX auto-number | Handled (`normalizeMathTex`) | Keep. Every un-starred numbered environment is rewritten to its starred twin before KaTeX sees it. | F |
| `$$` + `align` shorthand on the first line | Plain lines expanded into an aligned block, with an un-starred leak | None | Optional (WP6). If built, emit the starred form. | P (optional) |
| Inline-forced surfaces (DAG cards, `inlineOnly`, `firstMathSentence`) | n/a | `\tag` or `align` in inline mode errors (D20) | Strip `\tag{…}`, `\notag`, `\nonumber`; `align`→`aligned`, `gather`→`gathered`; drop the `equation` wrapper | F |

### KaTeX options and macros

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| Call style | One option builder; KaTeX called per formula | Two paths; auto-render re-scans | One `katexOptions(display)` builder; `renderToString` per formula | P |
| `output` | `html` | HTML + MathML | `html` | P |
| `strict` | `ignore` | `false` | `ignore` | P |
| `throwOnError` | `false` | `false` | `false` | P |
| `globalGroup` | `true` | unset | `true` | P |
| `errorColor` | theme red | `#cc0000` | The constant `var(--status-err)` | P |
| `trust`, `maxExpand`, `maxSize`, `fleqn`, `leqno` | unset | unset | Unset. `trust` stays off. | P |
| Seed macros | `\R \C \N \Z \ket \bra \braket \Tr \mbox` | none | The same nine names, with standard definitions written fresh | P |
| Macro persistence | One table for the whole app session | `\gdef` within one render only | Shared table, reset to the seed when the re:Search session changes | D (session isolation) |
| Internal `\@…` keys | Not addressed | n/a | Delete keys starting with `\@` after each render (defensive: only un-starred numbered environments leak them, and those never reach KaTeX) | F |
| `\newcommand` / `\renewcommand` | Rewritten per name to whichever form KaTeX accepts | none | Same. Required, because streaming re-renders the same text. | P |
| `\text{…}` repair | Unescaped `_`→`\_` and `^`→`\^{}` inside brace-free `\text{}` | none | Same | P |
| TeX-preview preamble macros | n/a | Discarded (T04) | Harvest `\newcommand`, `\renewcommand`, `\def`, `\DeclareMathOperator` into a document-scoped table | F |

### Numbering, tags, references

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| Who numbers | The app, except where an un-starred environment leaks through (shorthand, `alignat`, `flalign`) | KaTeX page-global counter (D19) | The app, per render root; no `eqn-num` ever emitted | P (stricter) |
| Which displays get a number | `$$` displays with a `\label` or an un-starred numbered environment. Bare `\begin{…}` blocks only when labelled (separate count that can collide). `\[…\]` never. | Every un-starred environment | One rule for every display delimiter: numbered if it has a `\label` or contains an un-starred `equation`/`align`/`gather`/`multline`/`eqnarray`/`alignat`/`flalign`. Plain `$$`, `\[`, starred: no number. One counter per root. | D |
| Display that already has `\tag` | App numbering skipped for the whole display | KaTeX handles it | A `\tag` row keeps its tag; other rows are still numbered (per-row) | D |
| `\notag` / `\nonumber` | No number for that row | Passed to KaTeX | Handled in the numbering pass | P |
| Per-row numbering | Effectively broken in the reading view ("Multiple \tag") | KaTeX auto | Row splitter at brace depth 0 and environment depth 0; trailing `\\` adds no row; starred twin with per-row `\tag` | D (works where K errors) |
| `\label` | Removed (except in mid-paragraph `$$`); the first label becomes `data-eq-id` | Removed, no id | Removed everywhere; `data-eq-id` = first label, else the first tag text | P + F |
| `\tag{X}` as a reference target | `X` registered as an id; `\eqref{label}` on a self-tagged equation reads `(X)` | n/a | Same | P |
| `\eqref{id}` in prose | Link `(n)`, or `(id)` when unknown; no `href` needed | Raw text (D18) | `<a class="eqref-link" data-eq-ref="…">(n)</a>`, no `href` | P |
| `\eqref` inside a formula | Upright `(n)` replaced before KaTeX | Red command | `\text{(n)}`; ids restricted to `[A-Za-z0-9:._-]` | P |
| Click | Smooth-scroll the target to the centre within the same root; Ctrl/Cmd-click follows the link | none | Same root scoping; no `href`, so Ctrl-click does nothing special | P |
| Hover card | ~420 ms preview with an "Equation (n)" badge; also on `\ref`, `\cref` and literal `(eq)` text | none | Optional (WP6), `\eqref` links only | P (optional, reduced) |
| Unclosed `$$` stops numbering | Later equations unnumbered | n/a | Do not copy; numbering follows the rendering bound | K-quirk avoided |
| Tag placement | Flex row: a leading flexible spacer and the tag with equal flex share, so the formula stays centred; tag right-aligned with 1.5em left padding and 2px right padding; row at least max-content wide; never overlaps | Absolute tag, can overlap (D27) | Same visual effect via `:has()` | P |

### Errors

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| Parse error | Escaped source in `.katex-error`, theme red, message in `title` | Same, `#cc0000` | Token colour through `errorColor` plus a `.katex .katex-error` colour rule with `!important` | P |
| Unknown command | Formula renders; the command is red | Same, fixed red | Token colour | P |
| Renderer throws | `code.math-error`, escaped, red, monospace, .88em | Re-throws out of `renderRichInto` | `try/catch` everywhere; same fallback | P |
| KaTeX not loaded | `span.math-pending`, upgraded everywhere when KaTeX arrives (K has no CSS for it) | Raw source | `span.math-pending` with escaped source, muted monospace (own style). No upgrade pass needed: KaTeX loads synchronously (`index.html:764-766`). | P (reduced) |

### Layout, theme, copy

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| Font size | `.markdown .katex` 1.08em | Global `.katex` 1.06em (`tokens.css:97-99`) | `.rich-prose .katex` 1.08em; labels and cards keep 1.06em | P (scoped) |
| Display wrapper | `.math-display`: block, centred, `overflow-x:auto`, `overflow-y:hidden`, padding .55em 0, margin .7em 0; inner `.katex-display` margin 0 (scoped) | Two nested scrollers (`.math-block` and `.katex-display`) | One scroller: `.math-display` (alias `.math-block`) gets these metrics; only `.math-display > .katex-display` is reset to margin 0, padding 0, overflow visible. The bare `.katex-display` rule in `tokens.css:101-105` stays for labels and cards. | P (scoped) |
| Inline glue | `.math-glue { white-space:nowrap }` | none | Same | P |
| Tables | Horizontally scrollable wrapper | `width:100%`, no wrapper | `div.table-wrap { overflow-x:auto }` | P |
| Direction | Reading view: nothing. Editor only: LTR + isolate. | nothing | `.rich-prose .katex { direction:ltr; unicode-bidi:isolate }` | D (cheap protection for RTL papers) |
| Theme | Glyphs inherit; errors red token | Errors fixed red | Token | P |
| Copy | HTML-only output, no copy-tex | MathML present | HTML only. Add escaped `data-tex` and `aria-label` on the wrapper (re:Search has no editor that reveals source). Clipboard behaviour unverified. | P + D (attributes) |

### Pipeline and Markdown around math

| Behaviour | Kandinsky | re:Search today | Target | Status |
|---|---|---|---|---|
| Passes | One | Two (D2) | One; `renderMathInElement` never called on model or file text | P |
| Display preparation order | Strip prefixes → labels/number tags → `\eqref` → environment rewrite → (shorthand) → macro normalisation → `\text` repair | `normalizeMathTex` only | Same order in `prepareTex` | P |
| Caching | Editor-only memos keyed by source (reading view not memoised per formula) | none; re-typesets every frame (D26) | Own per-formula cache (see WP2) | F |
| Sanitising | DOMPurify | Escape everything + fixed tag set | Keep re:Search's | D (deliberate) |
| Single newline | Line break by default | Joined (pinned by test) | Keep | D |
| Lists with display math, continuation lines, fences | Work | Close the list (D11) | Container-aware items | P |
| Loose lists, `start` | Standard | Split, restart at 1 (D9, D10) | Keep the list; emit `start` | P |
| Blockquote with math or fence | Works | Leaks (D5) | Strip the prefix, recurse | P |
| Table delimiter | Any dash count; a pipe in math splits the cell | Needs ≥3 dashes (D12); protects pipes in math | ≥1 dash; keep pipe protection | P + D |
| Headings | 0-3 spaces; closing `#` run dropped | Column 0 only; swallows lines (D13) | Same as K | P |
| Raw HTML | Sanitised and rendered | Escaped | Keep escaped (safety invariant) | D |
| Emphasis vs math | Math first | `2**3 and 4**5` bolds (D14) | Flanking rules | F (low priority) |
| Callouts | Titled box | Plain quote | Optional (WP6) | P (optional) |

## 3. Work packages

All paths are under `crates/desktop-server/`. Line estimates are approximate; every file must stay under 600 lines. Use relative imports between new modules (`./math_scan.js`); `math.js` is imported by file URL in Node tests (`tests/math_render.test.mjs:16`). Every new asset needs a `StaticAsset` entry in `src/assets.rs` (pattern at `:107-109`; asserted at `math_render.test.mjs:495-498,648`). Do one package per PR, in order.

### WP1: Split `math.js` without changing behaviour

- **Files owned:**
  - `assets/math.js` (596 → ~150 lines).
  - New `assets/markdown.js`, `assets/tex_preview.js`, `assets/code_highlight.js`, `assets/math_scan.js` (`extractMath` moved as is).
  - `src/assets.rs`.
- **Behaviour:** none changes. `math.js` re-exports every current name: `escapeHtml`, `renderMathInContainer`, `mathTextToHtml`, `renderMathTextInto`, `firstMathSentence`, `displayMathSource`, `markdownToHtml`, `latexDocumentToHtml`, `renderRichInto`, `renderLatexDocumentPreview`, `highlightUnifiedDiff`, `highlightPython`.
- **Acceptance:**
  - The existing `math_render.test.mjs` passes unmodified.
  - `node --check` passes on every new file.
  - All files are under 600 lines.

### WP2: Single-pass rendering, KaTeX options, macros, errors, cache, CSS

- **Files owned:**
  - New `assets/math_katex.js` (~230 lines).
  - `assets/math.js`.
  - `assets/components.css` (at 515 lines; if it would pass 600, create `assets/math.css` with an `assets.rs` entry and a `<link>` in `index.html`).
  - `assets/tokens.css` (no change to the existing `.katex` / `.katex-display` rules).
  - `assets/app.js` (one import and one call; see Decision 6).
  - `src/assets.rs`.
  - New `tests/math_parity.test.mjs`.
  - `tests/ui_math.rs` (add a second `node --test` invocation for the new file by name; CI already globs `*.test.mjs`, `.github/workflows/workbench-checks.yml:34`).
- **Behaviour:**
  - **Options.** `katexOptions(display)` returns `displayMode`, `output:'html'`, `strict:'ignore'`, `throwOnError:false`, `globalGroup:true`, `errorColor:'var(--status-err)'` and the shared `macros` object. `trust` is never set.
  - **Macro table.**
    - Seeded with nine entries: `\R`, `\C`, `\N`, `\Z` as double-struck letters; `\ket`, `\bra`, `\braket` with auto-sized angle and bar delimiters; `\Tr` as an upright operator; `\mbox` as text.
    - `resetMathMacros()` restores the seed and clears the cache.
    - After each render, delete keys that start with `\@`.
  - **Rewriting `\newcommand` / `\renewcommand` per name.** For each defined name, decide which form KaTeX accepts:
    - If the name is in the shared table or is a KaTeX built-in, use `\renewcommand`; otherwise use `\newcommand`.
    - Built-in detection: render the bare name against an empty macro table with `throwOnError:true`. A success counts as "defined". So does an error saying an argument or group is missing; only an "undefined control sequence" error counts as undefined.
    - Cache the decision per name.
  - **`\text` repair.** Inside `\text{…}` bodies that contain no braces, escape bare `_` as `\_` and bare `^` as `\^{}`.
  - **Rendering a formula.** `renderFormulaHtml(tok, ctx)`:
    - Calls `ctx.katex ?? globalThis.window?.katex`. Keep `window.katex` working, because `math_render.test.mjs:392-427` supplies only that.
    - Wraps every call in `try/catch`. On a throw it returns `<code class="math-error">` holding `escapeHtml(source)`.
    - With no KaTeX it returns `<span class="math-pending" data-display="0|1">` with the escaped source.
    - Wraps a display in `<div class="math-display math-block" data-tex="…" aria-label="…">`. Model text in those attributes is escaped.
    - Wraps an inline formula plus up to 4 trailing glue characters in `<span class="math-glue">`.
    - Never adds an element whose class is exactly `katex`, which would break the count at `math_render.test.mjs:75-91`.
  - **Cache.**
    - A `Map` keyed by display flag + prepared TeX + macro fingerprint, capped at ~500 entries with the oldest dropped.
    - The fingerprint is a counter. It goes up only when a definition adds a name or changes the expansion of an existing name. Re-defining a name with an identical body does not change it, so a streaming message that defines a macro still hits the cache on the next frame.
    - `resetMathMacros()` clears the cache.
  - **Rendered output.** New export `markdownToRenderedHtml(raw, {katex})` builds the final HTML in one pass.
    - `renderRichInto(el, raw)` assigns `innerHTML` once and does not call auto-render.
    - `renderLatexDocumentPreview` does the same with a document-scoped macro table.
    - `mathTextToHtml` and `renderMathTextInto` use the same renderer. `inlineOnly` applies the inline-forced clean-up.
    - `renderMathInContainer` keeps its TreeWalker / `createElement` / `replaceChild` surface (no fragments, Ranges or `insertAdjacentHTML`), and calls the new scanner and renderer instead of `renderMathInElement`.
    - `markdownToHtml` (source form) stays byte-identical for the existing tests.
  - **Session switch.** Call `resetMathMacros()` in `selectSession` (`app.js:531`).
  - **CSS** (class-scoped only; the CSS audit at `math_render.test.mjs:102-140` forbids bare `span`, `div` and `*` descendants, so extend its list with `.math-display`, `.math-glue`, `.table-wrap`):
    - `.rich-prose .katex`: 1.08em, LTR, bidi isolate.
    - `.math-display`: block, centred, `overflow-x:auto`, `overflow-y:hidden`, padding .55em 0, margin .7em 0.
    - `.math-display > .katex-display`: margin 0, padding 0, overflow visible.
    - `.math-glue`: nowrap.
    - `.katex .katex-error`: `var(--status-err)` with `!important`.
    - `.math-error`: same colour, monospace, .88em.
    - `.math-pending`: monospace, muted.
- **Acceptance:**
  - Cases 1, 10-13, 44-54, 61 and 62 pass in `math_parity.test.mjs`.
  - The existing test file passes unmodified.
  - `cargo test -p desktop-server --test ui_math` runs both files (run it when the build slot is free).
  - No `renderMathInElement` call remains on model or file text.

### WP3: Scanner fixes and block-level display math

- **Files owned:** `assets/math_scan.js` (~260 lines), `assets/markdown.js`, `assets/tex_preview.js`, `assets/math.js` (`displayMathSource`).
- **Behaviour:** the delimiter rows of section 2 marked P or D. In particular:
  - **Escapes.** An opener preceded by an odd backslash run is not an opener.
  - **Display closer.** `findDisplayClose(lines, i)`:
    - The search stops at the next ATX heading line or the end of the text.
    - Supports `$$$$`.
    - Text after a mid-line closer becomes a following paragraph.
  - **Display inside containers.** Display math is a block rule run after container prefixes (`>`, list indentation) are stripped.
  - **Mid-line `\[`.** Guarded as described in the parity table.
  - **`\(…\)`.** Bounded to one paragraph.
  - **Environments.** Classified as inline or display by name. Text after `\end{…}` on the opening line becomes a following paragraph.
  - **TeX preview.**
    - Treat `\\[2mm]` as a line break, not a display.
    - Harvest preamble macros.
  - **`displayMathSource`.** If one outer delimiter layer is removed and an unescaped delimiter remains, return the text unchanged, so `renderRichInto` renders it as mixed prose.
- **Acceptance:** cases 2-9, 14-32, 57-60 pass; existing tests unmodified.

### WP4: App numbering, `\label`, `\eqref`, tag layout

- **Files owned:** new `assets/math_equations.js` (~200 lines), `assets/math_katex.js` (`prepareTex` hook), CSS file from WP2.
- **Behaviour:**
  - **Counting.** `numberEquations(tokens)` walks the display tokens of one render root in order and assigns numbers by the rule in section 2. The counter is per root.
  - **Single-number displays.** The `equation`/`multline` wrapper is removed. `align`/`eqnarray` become `aligned` and `gather` becomes `gathered`. One `\tag{n}` is appended after the body, unless the display already carries `\tag` or `\notag`.
  - **Per-row displays** (`align`, `gather`, `eqnarray`, `alignat`, `flalign`):
    - Rewrite the environment to its starred twin; `eqnarray`'s `&=&` becomes `&=`.
    - Split rows at `\\` that sit at brace depth 0 and environment depth 0; a trailing `\\` adds no row.
    - A row with `\tag{X}` keeps it. A row with `\notag` or `\nonumber` gets no number (remove the command). Any other row gets `\tag{n}`.
  - **Labels.** Remove all `\label{…}` before KaTeX sees the TeX. The first label, or failing that the first tag text, becomes `data-eq-id` on the `.math-display` wrapper.
  - **Reference map.** The map records `label → displayed tag text`, so an equation with its own `\tag{T}` is referenced as `(T)`. Every `\tag{X}` text is also registered as an id.
  - **`\eqref` in prose.** Rendered as `<a class="eqref-link" data-eq-ref="ID">(n)</a>`, or `(ID)` when the id is unknown. It has no `href`, and ID is escaped.
  - **`\eqref` inside a formula.** Replaced with `\text{(n)}` before KaTeX. An id outside `[A-Za-z0-9:._-]` is left as escaped literal text.
  - **Click navigation.** `installEqrefNavigation(root)`:
    - It is idempotent: a marker property on the root means it installs once even though `renderRichInto` runs every animation frame.
    - It is a no-op when `root?.addEventListener` is not a function.
    - It uses one delegated click listener. That listener finds the target with `root.querySelector` on `[data-eq-id="…"]` (value through `CSS.escape` when available, else skip) and scrolls it into view, centred, with smooth behaviour.
  - **Tag CSS.**
    - `.math-display > .katex-display > .katex > .katex-html:has(> .katex-tag)` is a flex row with baseline alignment and `min-width:max-content`.
    - Its `::before` is an empty flexible item with the same flex share as the tag.
    - The `.katex-tag` inside is static, right-aligned, with 1.5em left padding and 2px right padding.
    - `.eqref-link` and a base `.rich-prose a` rule: accent colour, weight 500, tabular numerals, faint underline that gets stronger on hover.
- **Acceptance:**
  - Cases 33-43 and 55-56 pass.
  - No output contains `eqn-num`.
  - Rendering the same root twice installs one listener.

### WP5: Lists, tables, headings, emphasis

- **Files owned:** `assets/markdown.js`, CSS file.
- **Behaviour:**
  - **List items.** An item owns following lines indented to its content column, including display math and fences.
  - **Loose lists.** A blank line between items keeps one list.
  - **`start`.** Emitted, parsed to an integer, when the first number is not 1.
  - **Tables.**
    - Delimiter cells accept one or more dashes.
    - Pipes inside math stay protected.
    - Every table is wrapped in `div.table-wrap` (`overflow-x:auto`).
  - **Headings.**
    - ATX with 0-3 leading spaces.
    - A closing `#` run is stripped.
    - A heading line always ends a paragraph.
  - **Emphasis.** `**` must be left- or right-flanking: a non-space character inside, and no word character directly outside.
  - **HTML comments.** Stripped outside code, per Decision 4.
- **Acceptance:** cases 26-32 and 63-64 pass; existing tests unmodified.

### WP6: Optional

Each item below is a separate small PR:

- `$$` + `align` shorthand (starred output).
- Equation hover card for `.eqref-link`.
- `> [!type]` callouts as a titled `div.callout`.
- Copy-as-TeX handler (own code using `data-tex`).
- Removing `auto-render.min.js`, which needs coordinated edits to `index.html:765`, `src/assets.rs:215-217` and `tests/research_display_browser.py:24`.

### Escaping rules (all packages)

- **Inserted unescaped:** only KaTeX output (with `trust` off) and fixed tags. Probes: the error span escapes its source; `\href` emits no `<a>`; `errorColor` yields `style="color:var(--status-err)"`.
- **Attributes:** model-derived values that become attributes (`data-eq-id`, `data-eq-ref`, `data-tex`, `aria-label`) go through `escapeHtml`, which covers `"` and `'` (`math.js:3-7`). `start` is an integer.
- **Placeholders:** NUL stripping and the `\0M<n>\0` placeholder discipline stay.
- **Testing case 53:** do not use `assertNoXss` (`math_render.test.mjs:61-68`). It flags `onerror=` inside escaped attribute values, which KaTeX's `title` and the new `data-tex` legitimately contain. Assert instead that the output contains no `<img` and no unescaped `onerror` outside attribute values.

### Test cases

These go in `tests/math_parity.test.mjs`. Each input goes through `markdownToRenderedHtml` with the vendored KaTeX unless the row says otherwise.

- `⏎` is a newline.
- "inline n" counts `class="katex"` elements outside `.katex-display`.
- "display n" counts `.math-display` wrappers.
- Every case also checks that visible text (tags stripped, code removed) contains no unintended `$`, `\(`, `\[` or `\begin{`.
- The "D" column marks expected results that deliberately differ from Kandinsky.

| # | WP | Input | Expected | D |
|---|---|---|---|---|
| 1 | 2 | `Where $\phi_{0}$ and $\phi_{e}$ are orbitals.` | inline 2; no error | |
| 2 | 3 | `see \(x^2\) here` | inline 1 | D |
| 3 | 3 | `The **background \(m\) per unit cell** and background-\(m\) line.` | bold holds text + 1 formula; inline 2 | D |
| 4 | 3 | `Let $M = \begin{pmatrix}1&0\\0&-1\end{pmatrix}$ be diagonal.` | inline 1; display 0 | |
| 5 | 3 | `The matrix \begin{pmatrix}1&0\\0&-1\end{pmatrix} is diagonal.` | inline 1, same paragraph | D |
| 6 | 3 | `\begin{pmatrix}1&0\\0&-1\end{pmatrix}` alone on a line | display 1 (K shows prose) | D |
| 7 | 3 | `It costs $5 and $6 more` | no math | |
| 8 | 3 | `I paid $5 for apples and my friend paid 10$ for pears.` | no math | D (currency guard) |
| 9 | 3 | `between $5-$10, and $ 5 + 3 $.` | no math | |
| 10 | 2 | `Price \$5 and $x$` | literal `$5`; inline 1 | |
| 11 | 2 | `Literal \$\$x^2\$\$ should stay text.` | text `$$x^2$$`; no math | |
| 12 | 2 | `` `$x$` and $y$ `` | code shows `$x$`; inline 1 | |
| 13 | 2 | Fenced `bash` block with `$HOME`, `$((1+2))`, `\(no\)`, then `then $x$.` | code untouched; inline 1 | |
| 14 | 3 | `Cases $a$, then $b$. And $(c)$; and $d$ alone` | 3 `math-glue` holding `,` `.` `;`; none on `$d$` | |
| 15 | 3 | `Before⏎⏎$$⏎E = mc^2⏎$$⏎⏎After` | paragraph, display 1, paragraph | |
| 16 | 3 | `$$⏎a = b⏎⏎+ c⏎$$` | display 1 | |
| 17 | 3 | `$$⏎x = 1⏎$$which follows.` | display 1, then paragraph "which follows." | |
| 18 | 3 | `$$⏎U = s/V⏎$$$$⏎H = N^2 s⏎$$` | display 2; no stray `$$` | |
| 19 | 3 | `$$⏎x = 1⏎⏎## Next part⏎⏎$$ z = 2 $$ and $y$` | first `$$` literal; `<h2>`; display 1; inline 1 | |
| 20 | 3 | `$$ x #1 $$` | display 1 wrapper holding exactly 1 `katex-error` with the escaped source; 0 `katex-display`; no heading | |
| 21 | 3 | `Type $$ to open a display.⏎⏎Second paragraph with $x$.` | `$$` literal; inline 1 | |
| 22 | 3 | `\[⏎\sum_{n=1}^{\infty} \frac{1}{n^2}⏎\]` | display 1 | |
| 23 | 3 | `As shown in \[1\] and \[2, 3\].` | no math. Text per Decision 3 (default `[1]` and `[2, 3]`, backslashes dropped as in K) | |
| 24 | 3 | `Row one \\[4pt] row two, later \[ y \] done.` | `\\[4pt]` not math; `\[ y \]` is a display span | D |
| 25 | 3 | `Use \begin{align} for alignment, e.g. \(x^2\) or $y$.` | `\begin{align}` literal; inline 2 | D |
| 26 | 3 | `> $$⏎> a = b⏎> $$` | blockquote with display 1; TeX has no `>` | |
| 27 | 5 | `1. First step:⏎   $$x = 1$$⏎   continues here⏎2. Second step` | one `<ol>`, 2 items; item 1 holds display 1 and "continues here" | |
| 28 | 5 | `1. first⏎⏎2. second⏎⏎3. third` | one `<ol>`, 3 items | |
| 29 | 5 | `3. three⏎4. four` | `<ol start="3">` | |
| 30 | 5 | `\| a \| b \|⏎\|--\|--\|⏎\| $\|x\|$ \| \(\tfrac{21}{4}\) \|` | table inside `.table-wrap`; inline 2; 2 cells per row | D (pipe protection) |
| 31 | 5 | `   ## Result for $x^2$ ##` | `<h2>` with inline 1; no trailing `#` | |
| 32 | 5 | `    However we choose $b$ here` | paragraph, inline 1 | |
| 33 | 4 | `$$⏎a = b \label{eq-first}⏎$$⏎⏎See \eqref{eq-first}.` | tag `(1)`; no "label" text; `eqref-link` reading `(1)`; `data-eq-id="eq-first"` | |
| 34 | 4 | `$$ c = d $$` | display 1, no tag | |
| 35 | 4 | `$$ g \tag{D1} \label{eq-own} $$`, then `\eqref{eq-own}`, then `$$ h \label{x} $$` | tag `(D1)`; link reads `(D1)`; next numbered display `(1)` | |
| 36 | 4 | `$$ i \notag \label{eq-none} $$` | no tag | |
| 37 | 4 | Two blocks `\begin{equation} a=b \end{equation}` and `\begin{equation} c=d \end{equation}` | tags `(1)`, `(2)`; no `eqn-num` (K: no tags) | D |
| 38 | 4 | `\begin{align} a &= b \label{r1} \\ c &= d \notag \\ e &= f \tag{T} \\ g &= h \label{r2} \end{align}`, then `\eqref{r1} \eqref{r2}` | no error; row tags `(1)`, none, `(T)`, `(2)`; links `(1)` `(2)` (K: only `(T)`) | D |
| 39 | 4 | `\begin{align} f &= \begin{cases} 1 \\ 0 \end{cases} \\ g &= 2 \end{align}` | tags `(1)`, `(2)`; nested `\\` does not split (K: no tags) | D |
| 40 | 4 | `\begin{eqnarray} a &=& b \\ c &=& d \end{eqnarray}` | no error; tags `(1)`, `(2)` (K: no tags) | D |
| 41 | 4 | `\begin{align*} a &= b \\ c &= d \end{align*}` | display 1, no tags | |
| 42 | 4 | `See \eqref{nope}.` | link text `(nope)` | |
| 43 | 4 | `$x \eqref{eq-first}$` after case 33 | inline 1 with upright `(1)`; no red | |
| 44a | 2 | `$\newcommand{\div}[1]{\nabla \cdot #1}$ then $\div{E}$` rendered twice | no error either time | |
| 44b | 2 | As 44a, clear the cache only (keep the macro table), render again | no error (exercises the rewrite between `\newcommand` and `\renewcommand`) | |
| 45 | 2 | `$\def\zz#1{[#1]}$ … $\zz{q}$` | second formula uses the definition | |
| 46 | 2 | Case 45, `resetMathMacros()`, then `$\zz{q}$` | `\zz` red (table and cache reset) | D (scope) |
| 47 | 2 | `$\text{Nb_3Ge}$` | text "Nb_3Ge"; no error | |
| 48 | 2 | `$k_{\mbox{\tiny B}}T$ and $\Tr\rho$ and $\C^n$ and $\ket{\psi}$` | inline 4; no red | |
| 49 | 2 | `$α + β ≤ γ$` | inline 1; no error | |
| 50 | 2 | `$x^$` | 1 `katex-error` with `x^` and a `title`; `color:var(--status-err)`; no `#cc0000` | |
| 51 | 2 | `$\dv{f}{x}$` | renders; red `\dv`; no `katex-error` element | |
| 52 | 2 | `$\href{javascript:alert(1)}{x}$` | no `<a`; red `\href` | |
| 53 | 2 | `$<img src=x onerror=1>^$` | no `<img`; escaped source (not via `assertNoXss`) | |
| 54 | 2 | Any formula | no `<math`, no `<annotation`, no `eqn-num` | |
| 55 | 4 | `mathTextToHtml('Step $E=mc^2 \\tag{3}$', {inlineOnly:true})` | inline 1; no error | |
| 56 | 4 | `mathTextToHtml('\\begin{align} a&=b \\end{align}', {inlineOnly:true})` | inline 1; no error | |
| 57 | 3 | `renderRichInto` path of `displayMathSource('$a$ and $b$')` (as `trace_cards.js:348` uses it), asserted on `markdownToRenderedHtml(displayMathSource(...))` | inline 2; "and" as text; display 0 | |
| 58 | 3 | TeX preview: `Line one\\[2mm]⏎Line two is prose.⏎⏎\[ x = 1 \]⏎After.` | prose kept; display 1 holding `x = 1` only | |
| 59 | 3 | TeX preview: preamble `\newcommand{\tr}{\operatorname{tr}}`, body `$\tr \rho$` | inline 1; no red | |
| 60 | 3 | Streaming partials `Energy \(E = mc`, then `…^2\) done` | first: raw text, no error span; second: inline 1 | |
| 61 | 2 | Same text, including a `\def`, rendered twice | identical HTML; KaTeX wrapper called once per distinct formula, including on the second pass | |
| 62 | 1 | `markdownToHtml('Costs $5 and $10, while $\\mu = 0$ is exact.')` | exactly `<p>Costs $5 and $10, while \(\mu = 0\) is exact.</p>` | |
| 63 | 5 | `2**3 and 4**5` | no `<strong>` | |
| 64 | 5 | `a <!-- note --> b` | per Decision 4 (default: text `a  b`) | |
| 65 | 4 | Fake root without `addEventListener`; `renderRichInto` twice on a real-ish root | no throw; one listener | |

## 4. Visual checks on the Windows laptop (WebView2, after WP2-WP5)

Check each item in light and dark theme. Do not use Vivaldi.

1. **Size and baseline.** Inline math in assistant prose at 1.08em sits on the baseline without opening up line spacing. DAG cards, Investigation map nodes and convention labels are unchanged at 1.06em.
2. **Display spacing.** There is one margin above and below a display, with no doubled gap. The DAG box and the hero still get their own scroller from `tokens.css:101-105`.
3. **Wide display.** One horizontal scrollbar on its own box. The message column does not widen. No vertical scrollbar. No spurious scrollbar on tagged displays.
4. **Tags.** The number is right-aligned and the formula stays centred. On a wide formula the number follows it, with no overlap. Per-row tags line up with their rows. (These rules need `:has()`.)
5. **Glue.** At a narrow width, a line never breaks between a formula and its comma or full stop.
6. **Errors.** Red uses `--status-err` in both themes and is legible on dark. A theme switch leaves no stale colours (`.theme-switching *`, `tokens.css:112`).
7. **Fonts.** No 404s in the network log; `\mathscr`, `\mathfrak`, `\mathbb` and bold italic all render.
8. **Streaming.** No flicker as formulas close. Scroll-to-bottom keeps tracking. Frame time is acceptable on a long answer with macros.
9. **Eqref.** A click scrolls the right equation to the centre, in the stream and in the canvas pane, and never to an equation in another message.
10. **Special containers.** Check `.thinking-notes` (`pre-wrap`), `.investigation-inner p` (`pre-line`), `.dag-node-summary`, and `.verdict-issue-desc` (a span that receives block output).
11. **Tables.** A wide table scrolls inside `.table-wrap`; math in cells is not clipped.
12. **Copy.** Record what the clipboard receives when rendered math is selected. Neither the plan nor the verifier could check this.
13. **Other surfaces.** The hero, the worked-example dialog, the manuscript preview, and the `.tex` reader on a real paper with preamble macros.
14. **Export (note only).** The standalone HTML export (`crates/export/src/html.rs`) is untouched by this plan and may now look different from the app.

## 5. Decisions for the user (recommended default first)

1. **Licence:** Kandinsky is AGPL-3.0 and re:Search is MIT OR Apache-2.0, so this plan reimplements observed behaviour in new code and copies no Kandinsky source, CSS text or test text. **Default: approve the clean-room reimplementation.**
2. **`\(…\)` stays math** although Kandinsky shows it as plain text. **Default: keep it**; models emit it constantly.
3. **Failed mid-line `\[` / `\(` openers** (such as `\[1\]`). **Default: drop the backslash like Kandinsky's escape handling, showing `[1]`**; the alternative is to show `\[1\]` literally.
4. **HTML comments outside code.** **Default: strip them like Kandinsky**; the alternative is to keep showing them escaped.
5. **Equation numbering covers all delimiters and per-row numbers**, which Kandinsky numbers only partly and sometimes errors on. **Default: number by the one rule in section 2 (deviation).**
6. **Macro scope.** **Default: reset on session switch**, with a 2-line addition to `app.js:531`. `app.js` is already 928 lines, over the 600 limit, so approve this exception or ask for a separate session-change hook module.
7. **Bidi isolation on math** (not in Kandinsky's reading view). **Default: add it, scoped to `.rich-prose`.**
8. **Removing `auto-render.min.js`.** **Default: later (WP6)**, after `index.html`, `assets.rs` and the browser test are updated together.
9. **Optional extras** (align shorthand, hover card, callouts, copy-as-TeX). **Default: defer until WP1-WP5 ship.**
10. **Standalone HTML export parity.** **Default: out of scope for this plan; decide separately.**