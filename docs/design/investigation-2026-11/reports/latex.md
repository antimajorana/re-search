# Agent report: `latex` (WP1–WP5)

- **Base commit:** `91753b86b4392814cbe0fe4dd213c9be1d15fc80` (`origin/gemini/runner`)
- **Work branch:** `work/latex`
- **Reference:** Kandinsky (`https://github.com/antimajorana/kandinsky`, commit `833a95f`); ported files carry the attribution header required by `README.md` (LaTeX decision 1). Nothing under `~/kandinsky/plugins/obsidian/` was read or copied.
- **Scope:** WP1 through WP5 of `03-latex-kandinsky-parity.md`. WP6 (optional extras and `auto-render.min.js` removal) and standalone HTML export (`crates/export/src/html.rs`) are out of scope.

---

## 1. Implementation by work package

### WP1 — Split `math.js` without changing behaviour (`6653799`)
- Extracted `crates/desktop-server/assets/code_highlight.js` (`escapeHtml`, `highlightUnifiedDiff`, `highlightPython`, 29 lines).
- Extracted `crates/desktop-server/assets/math_scan.js` (delimiter scanner & environment normalization).
- Extracted `crates/desktop-server/assets/markdown.js` (Markdown block/inline renderer).
- Extracted `crates/desktop-server/assets/tex_preview.js` (LaTeX document preview renderer).
- Slimmed `crates/desktop-server/assets/math.js` (117 lines) while re-exporting every existing public symbol.
- Registered all new modules in `crates/desktop-server/src/assets.rs`.
- Uses relative ES module imports (`./...`) so both `/assets/...` in the desktop server and `file://` URLs in Node tests resolve cleanly.

### WP2 — Single-pass rendering, KaTeX options, macros, errors, cache, CSS (`69ededc`, `f530567`)
- Created `crates/desktop-server/assets/math_katex.js` (255 lines):
  - `katexOptions(display, state)`: `displayMode`, `output: 'html'`, `strict: 'ignore'`, `throwOnError: false`, `globalGroup: true`, `errorColor: 'var(--status-err)'`, shared `macros`; `trust` is never set.
  - Seeded with the 9 Kandinsky macros (`\R`, `\C`, `\N`, `\Z`, `\ket`, `\bra`, `\braket`, `\Tr`, `\mbox`).
  - `resetMathMacros()` restores the seed and clears the cache; `clearMathCache()` clears only the cache (used by parity test 44b).
  - Deletes `\@*` / `@*` internal keys from `macros` after each render.
  - `rewriteNewcommands()` rewrites `\newcommand*` / `\renewcommand*` per macro name using built-in detection (`throwOnError: true` against empty macros, cached per name; any error other than `"Undefined control sequence"` counts as defined).
  - `repairTextCmd()` escapes unescaped `_` → `\_` and `^` → `\^{}` inside brace-free `\text{…}` bodies (counting full backslash runs).
  - Per-formula `Map` cache (500-entry cap, FIFO eviction) keyed by `display\0prepared\0fingerprint`, using a cycle-safe macro token signature so identical re-definitions do not invalidate cached entries and streaming re-renders hit the cache on pass 2 (including `\def`).
  - `renderFormulaHtml(tok, ctx)`: uses `ctx.katex ?? globalThis.window?.katex`, wraps calls in `try/catch` (`<code class="math-error">`), renders `<span class="math-pending" data-display="0|1">` when KaTeX is absent, wraps displays in `.math-display.math-block` (`div`) or `.math-display` (`span`) with escaped `data-tex` and `aria-label`, and wraps inline formulas with trailing punctuation in `<span class="math-glue">`.
- Added `markdownToRenderedHtml` and `latexDocumentToRenderedHtml` (with document-scoped macro state) while keeping `markdownToHtml` and `latexDocumentToHtml` source forms intact for existing tests.
- Removed all calls to `window.renderMathInElement`; `renderMathInContainer` retains its TreeWalker / `createElement` / `replaceChild` surface using our scanner and `renderFormulaHtml`.
- Added scoped math CSS rules to `crates/desktop-server/assets/components.css` (leaving `tokens.css` unchanged) and extended `katexContainers` in `tests/math_render.test.mjs` CSS audit with `.math-display`, `.math-glue`, `.table-wrap`.
- Added `crates/desktop-server/tests/math_parity.test.mjs` and wired a second `#[test]` in `crates/desktop-server/tests/ui_math.rs`.
- Isolated 2-line hook in `crates/desktop-server/assets/app.js` (`f530567`) importing and calling `resetMathMacros()` in `selectSession` when `changed` is true.

### WP3 — Scanner fixes and block-level display math (`c7d4398`)
- Upgraded `crates/desktop-server/assets/math_scan.js` (324 lines):
  - Backslash-run escape handling (`\\` consumed as a pair so `\\[4pt]` and `\\[2mm]` are never mistaken for `\[`).
  - Inline `$…$` keeps full backslash-run counts and paragraph bounds, no longer aborts on `\begin{`, and enforces the currency guard (digit-leading body with a space-separated multi-letter word outside TeX commands is not math).
  - Punctuation glue captures up to 4 trailing characters from `.,;:!?)]’”` on inline formulas in rendered mode (never stealing `]` before `(` in Markdown links).
  - `findDisplayClose` and `findEnvBlockClose` stop at ATX heading lines (`^[ \t]{0,3}#{1,6}(?:[ \t]|$)`), support `$$$$`, and convert mid-line closer trailing text into a following paragraph.
  - Guarded mid-line `\[…\]` rejects citation-only bodies (`[\d,\s–—-]+`); failed mid-line `\[` / `\]` / `\(` / `\)` drop the backslash per Decision 3.
  - `\(…\)` bounded to one paragraph; streaming partial `\(E = mc` stays text with no error span.
  - Bare `\begin{env}`: all 30 environments in `MATH_ENVS` at line start render as block display (with trailing text after `\end{…}` as a following paragraph); mid-sentence `INLINE_MATH_ENVS` render inline and numbered families render as `span.math-display`.
- Moved fenced code blocks and block display math into `renderMarkdownBlocks` in `assets/markdown.js` so blockquote prefixes (`> `) are stripped before scanning TeX or code.
- Updated `displayMathSource` in `assets/math.js` to return mixed-prose inputs unchanged when unescaped delimiters remain (Case 57).
- Added `harvestPreambleMacros()` in `assets/tex_preview.js` (`\newcommand`, `\renewcommand`, `\providecommand`, `\DeclareMathOperator(*)`, `\def`/`\gdef`) into a document-scoped macro state that does not leak into the session table, and fixed `\\[2mm]` line breaks (Cases 58–59).

### WP4 — App numbering, `\label`, `\eqref`, tag layout (`e93f17a`)
- Created `crates/desktop-server/assets/math_equations.js` (270 lines) and registered it in `src/assets.rs`:
  - `numberEquations(tokens)` numbers all display tokens in one render root using a single per-root counter across all delimiters (`$$`, `\[`, bare `\begin{…}`).
  - Single-number displays strip `equation`/`multline` wrappers (rewriting `multline` → `gathered`, `align*`/`eqnarray*` → `aligned`, `gather*` → `gathered`), append `\tag{n}`, and honour `\tag{X}` (without advancing counter) and `\notag`/`\nonumber`.
  - Per-row multi-row environments (`align`, `gather`, `eqnarray`, `alignat`, `flalign`) split rows at brace depth 0 and environment depth 0 (skipping `alignat`'s `{cols}` arg, nested environments such as `cases`, optional `\\[4pt]` spacing, and trailing `\\`), rewrite to their starred twins (`align*`, `gather*`, `alignat*`; `eqnarray` columns normalized), and emit per-row `\tag{n}` while preserving `\tag{X}` and stripping `\notag`/`\nonumber`.
  - Strips all `\label{…}` before KaTeX; sets `data-eq-id` to the first label (else first tag text) and `data-eq-ids` when a multi-row display carries additional labels/tags.
  - Prose `\eqref{id}` renders as `<a class="eqref-link" data-eq-ref="…">(n)</a>` (or `(id)` when unknown) with no `href` and all values escaped; formula `\eqref{id}` rewrites to `\text{(n)}` (ids outside `[A-Za-z0-9:._-]` are TeX-escaped literal text).
  - `installEqrefNavigation(root)`: idempotent via a `Symbol` marker, no-op without `addEventListener`, single delegated click listener using `CSS.escape` and `root.querySelector` to smooth-scroll the target to `block: 'center'`. Installed by `renderRichInto` and `renderLatexDocumentPreview`.
- Added Kandinsky `:has(> .katex-tag)` flex tag layout, `.katex .katex-tag` 2px right padding, and `.rich-prose a` / `.eqref-link` styles to `assets/components.css`.

### WP5 — Lists, tables, headings, emphasis, HTML comments (`8a1aa45`)
- Upgraded `crates/desktop-server/assets/markdown.js` (383 lines):
  - Container-aware list parser (`consumeList`): list items own lines indented to their content column (display math, fenced code blocks, nested lists, continuation paragraphs), loose lists with blank lines stay one list, ordered lists emit integer `start="N"` when `N !== 1`, and tight list items unwrap `<p>` so nested list markup stays compatible with existing tests.
  - Table delimiters accept 1+ dashes (`^:?-{1,}:?$`), pipes inside math stay protected via pre-split inline scanning, and every table is wrapped in `<div class="table-wrap">` (styled in `components.css`).
  - ATX headings allow 0–3 leading spaces, strip closing `#` runs preceded by whitespace, and terminate paragraphs.
  - `**` and `***` enforce flanking boundaries (`(^|[^\w*])` and `(?=[^\w*]|$)`) so `2**3 and 4**5` does not bold.
  - `stripComments()` strips `<!-- … -->` outside fenced and inline code.
- Compacted `tests/math_parity.test.mjs` to 516 lines while keeping all 65 parity cases plus fallback and container edge-case checks.

---

## 2. Parity table status (`03-latex-kandinsky-parity.md` §2)

| Section | Row | Status |
|---|---|---|
| **Delimiters** | Inline `$…$` (K rules, escape run, paragraph bound, no `\begin{` abort, currency guard) | Done (WP2/WP3) |
| | `$…$` holding an environment | Done (WP3) |
| | `\$` literal, single-pass no re-scan | Done (WP2) |
| | `$$…$$` at line start (< 4 cols, interrupts containers) | Done (WP3) |
| | `$$…$$` mid-paragraph (`span.math-display`, consistent preparation) | Done (WP3/WP4) |
| | Closer mid-line trailing paragraph; `$$$$` | Done (WP3) |
| | Unclosed `$$` / `\[` bounded by ATX heading / EOF | Done (WP3) |
| | Blank line inside a display | Done (WP3) |
| | `\[…\]` at line start | Done (WP3) |
| | `\[…\]` mid-line (guarded, Decision 3 drop backslash) | Done (WP3) |
| | `\(…\)` inline, paragraph-bounded | Done (WP3) |
| | Bare `\begin{env}` (30 envs at line start; inline vs display span mid-sentence; trailing prose) | Done (WP3) |
| | Math in code (fenced & inline) | Done (WP2/WP3) |
| | Indented code disabled (4-space line is prose) | Done (WP3/WP5) |
| | Math in blockquote / list item | Done (WP3/WP5) |
| | Punctuation glue (`span.math-glue`, rendered inline only) | Done (WP2/WP3) |
| | HTML comments stripped outside code (Decision 4) | Done (WP5) |
| **Environments** | `equation`, `multline` wrapper removal + app `\tag{n}` | Done (WP4) |
| | `align`, `gather`, `eqnarray` (single vs per-row starred twin with per-row `\tag`/`\notag`) | Done (WP4) |
| | `alignat`, `flalign`, `subequations`, `eqnarray` column fix (no un-starred leak) | Done (WP2/WP4) |
| | `$$` + `align` shorthand | Deferred (WP6, out of scope) |
| | Inline-forced surfaces (`inlineOnly`, DAG cards, `firstMathSentence`) | Done (WP2/WP4) |
| **KaTeX & macros** | One `katexOptions(display)` builder; `renderToString` per formula | Done (WP2) |
| | `output: 'html'`, `strict: 'ignore'`, `throwOnError: false`, `globalGroup: true`, `errorColor: 'var(--status-err)'`, `trust` unset | Done (WP2) |
| | 9 seed macros (`\R \C \N \Z \ket \bra \braket \Tr \mbox`) | Done (WP2) |
| | Macro persistence reset on session switch (`app.js`) | Done (WP2) |
| | Delete `\@` internal macro keys after render | Done (WP2) |
| | `\newcommand` / `\renewcommand` per-name rewrite | Done (WP2) |
| | `\text{…}` bare `_` / `^` repair | Done (WP2) |
| | TeX-preview preamble macro harvesting into document-scoped table | Done (WP3) |
| **Numbering & refs** | App numbering per render root, zero `eqn-num` | Done (WP4) |
| | Unified numbering rule across all display delimiters | Done (WP4) |
| | Existing `\tag` kept; other rows still numbered per-row | Done (WP4) |
| | `\notag` / `\nonumber` suppression | Done (WP4) |
| | Per-row numbering with depth-0 row splitter | Done (WP4) |
| | `\label` removed everywhere; `data-eq-id` set | Done (WP4) |
| | `\tag{X}` registered as reference target | Done (WP4) |
| | `\eqref{id}` in prose (`a.eqref-link`, no `href`) | Done (WP4) |
| | `\eqref` inside formula (`\text{(n)}`, safe id check) | Done (WP4) |
| | Delegated click scroll navigation (`installEqrefNavigation`) | Done (WP4) |
| | Hover card | Deferred (WP6, out of scope) |
| | Unclosed `$$` does not stop later numbering | Done (WP3/WP4) |
| | Flex tag placement via `:has(> .katex-tag)` | Done (WP4) |
| **Errors** | Parse error `.katex-error` with theme token + `!important` | Done (WP2) |
| | Unknown command in theme red | Done (WP2) |
| | Renderer throws → `code.math-error` escaped | Done (WP2) |
| | KaTeX not loaded → `span.math-pending` | Done (WP2) |
| **Layout, theme, copy** | `.rich-prose .katex` 1.08em, LTR, bidi isolate; global `.katex` 1.06em kept in `tokens.css` | Done (WP2) |
| | Single scroller `.math-display` / `.math-block`; inner `.katex-display` reset | Done (WP2) |
| | `.math-glue { white-space: nowrap }` | Done (WP2) |
| | `div.table-wrap { overflow-x: auto }` | Done (WP5) |
| | Escaped `data-tex` and `aria-label` on display wrappers | Done (WP2) |
| **Pipeline & Markdown** | Single-pass rendering, no `renderMathInElement` | Done (WP2) |
| | `prepareTex` preparation order | Done (WP2/WP4) |
| | Per-formula cache with macro fingerprint | Done (WP2) |
| | Escape-all + fixed tag set sanitising preserved | Done (WP1–WP5) |
| | Single newline joined softly | Done (WP1–WP5) |
| | Lists with display math, continuations, fences, loose lists, `start` | Done (WP5) |
| | Blockquotes with math or fences | Done (WP3) |
| | Table delimiter ≥ 1 dash + pipe protection in math | Done (WP3/WP5) |
| | Headings 0–3 spaces, closing `#` stripped, end paragraphs | Done (WP5) |
| | Raw HTML stays escaped | Done (WP1–WP5) |
| | `**` / `***` emphasis flanking rules | Done (WP5) |
| | Callouts | Deferred (WP6, out of scope) |

---

## 3. Decisions made unattended & minor deviations

1. **Math CSS kept in `assets/components.css` rather than a new `assets/math.css`:** Adding all WP2, WP4, and WP5 math/table/link rules brought `components.css` from 516 to 569 lines (under the 600-line cap). This avoids touching `assets/index.html` (owned by `ux-ui`) and ensures the existing CSS audit in `math_render.test.mjs` continues to audit every new rule automatically.
2. **Cycle-safe macro fingerprinting & post-definition cache key:** Probing vendored KaTeX 0.19.1 revealed that `\def` / `\newcommand` store token objects referencing `loc.lexer` (a circular graph back to `macros`), so `JSON.stringify` throws. `math_katex.js` computes a cycle-safe structural signature (`tokens[].text`, `numArgs`, `unexpandable`, `delimiters`). When a formula adds/changes a macro that is not referenced before its definition in the same formula, the rendered HTML is also cached under the post-definition fingerprint so a second render of the same document (including streaming and Case 61) hits cache for every formula including `\def`.
3. **Strip `*` on `\newcommand*` / `\renewcommand*`:** Probing KaTeX 0.19.1 showed `\renewcommand*` causes a parse error; `rewriteNewcommands` normalizes both `\newcommand*` and `\renewcommand*` to unstarred `\newcommand` / `\renewcommand`.
4. **Punctuation glue avoids breaking Markdown links:** `matchGlue` stops before `]` when followed immediately by `(` so `[see $x$](https://example.com)` remains a valid Markdown link.
5. **Multi-row display secondary labels (`data-eq-ids`):** `data-eq-id` strictly holds the first label (else first tag text) as specified, while `data-eq-ids` is additionally emitted when a multi-row display has multiple labels/tags so clicking `\eqref{r2}` on a later row also scrolls to the display block.
6. **File line counts:** All newly created and modified files are ≤ 600 lines (`code_highlight.js`: 29, `math.js`: 117, `math_katex.js`: 255, `tex_preview.js`: 255, `math_equations.js`: 270, `math_scan.js`: 324, `assets.rs`: 329, `markdown.js`: 383, `math_parity.test.mjs`: 516, `components.css`: 569, `ui_math.rs`: 60). The only touched files above 600 lines are the pre-existing `app.js` (930 lines, exempted by Decision 6) and `math_render.test.mjs` (653 lines, +1 line extending the CSS audit container list).

---

## 4. Merge notes (shared files)

| File | Function / Location | Our change | Notes for merge agent |
|---|---|---|---|
| `crates/desktop-server/assets/app.js` | Line 6 import; `selectSession` (~line 541, inside `if (changed)`) | Isolated in commit `f530567`: added `resetMathMacros` to `/assets/math.js` import and called `resetMathMacros()` at the top of `if (changed)`. | Owned by `ux-ui`; keep these 2 lines when merging `app.js`. |
| `crates/desktop-server/src/assets.rs` | `lookup_asset` match block (~lines 111–134, right after `"/assets/math.js"`) | Added 6 `StaticAsset` entries: `code_highlight.js`, `math_scan.js`, `math_katex.js`, `math_equations.js`, `markdown.js`, `tex_preview.js`. | Other briefs adding assets will add adjacent match arms; combine all arms cleanly. |
| `crates/desktop-server/tests/math_render.test.mjs` | Line 120 (`katexContainers` array in CSS audit) | Appended `'.math-display', '.math-glue', '.table-wrap'`. | Single-line addition only; zero existing assertions changed. |
| `crates/desktop-server/tests/ui_math.rs` | Appended `desktop_ui_math_parity_suite_passes` | Added second `node --test` integration test for `math_parity.test.mjs`. | Owned by `latex`. |

---

## 5. Verification & test results

| Command | Result |
|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Passed (0 syntax errors across all assets) |
| `node --test crates/desktop-server/tests/math_render.test.mjs` | **14 passed, 0 failed, 0 skipped** (existing suite unmodified except +1 CSS audit class list entry) |
| `node --test crates/desktop-server/tests/math_parity.test.mjs` | **19 passed, 0 failed, 0 skipped** (covering all 65 parity cases + WP2 fallbacks + container/eqref edge checks) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **82 passed, 0 failed, 0 skipped** |
| `cargo test -p desktop-server --test ui_math` | **2 passed, 0 failed, 0 ignored** (`desktop_ui_math_and_markdown_suite_passes`, `desktop_ui_math_parity_suite_passes`) |
| `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p desktop-server --no-fail-fast` | **104 passed, 0 failed, 0 ignored** across 27 test suites |
| `cargo clippy -p desktop-server --test ui_math -- -D warnings -A clippy::manual_unwrap_or` | Passed (0 warnings in `assets.rs` and `ui_math.rs`) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Failed on pre-existing lints in untouched files (`evidence.rs:737` `manual_unwrap_or`, `investigation.rs:384` `items_after_test_module`; see §6) |
| `RE_SEARCH_PYTHON=$(command -v python3) cargo test --workspace --no-fail-fast` | Ran 15 suites (**133 passed, 0 failed** across `agent`, `chatgpt-auth`, `cli` unit/api tests) before hanging > 150s in untouched `crates/cli/tests/desktop_and_ext.rs` (`cli_research_workspace_read_fork_diff_export_and_verify`); canceled and ran `cargo test -p desktop-server --no-fail-fast` in full above. |

---

## 6. Pre-existing failures & bugs noticed (not fixed)

- **`crates/desktop-server/src/evidence.rs:737`** — `cargo clippy --workspace --all-targets -- -D warnings` fails on Rust `1.100.0` (`clippy 0.1.100`) with `clippy::manual_unwrap_or` on `match result["status"].as_str() { Some(status @ ("verified" | ...)) => status, _ => "result_unavailable" }`. Confirmed identical on base commit `91753b86b4392814cbe0fe4dd213c9be1d15fc80` via `git show`. Note that clippy's suggested replacement (`unwrap_or("result_unavailable")`) would be a bug (a clippy false positive that drops the status allowlist guard); do not apply it blindly.
- **`crates/desktop-server/src/investigation.rs:384`** — `cargo clippy -p desktop-server --all-targets -- -D warnings` fails on `clippy::items_after_test_module` because `#[cfg(test)] mod tests` precedes `pub(crate) fn prepare` / `finish` / `handle`. Confirmed identical on base commit `91753b86b4392814cbe0fe4dd213c9be1d15fc80` via `git show`. Not touched because `ux-rust` owns `investigation.rs`.

---

## 7. What could not be compiled or run on Linux (Windows / WebView2 checks)

- `cfg(windows)` code paths (AppContainer sandbox, Job Objects, `desktop-app` WebView2 host) are not compiled on Linux.
- Playwright browser tests (`tests/*_browser.py`) are out of scope on Linux.
- Visual checks from `03-latex-kandinsky-parity.md` §4 must be verified on Windows 11 inside the WebView2 `re-search-app` window in both light and dark themes (not Vivaldi):
  1. **Size and baseline:** Inline math in `.rich-prose` at `1.08em` sits cleanly on the baseline without opening line spacing; DAG cards, Investigation map nodes, and convention labels stay at `1.06em`.
  2. **Display spacing:** Single margin above and below displays; DAG box and hero retain their scroller from `tokens.css:101-105`.
  3. **Wide display:** Single horizontal scrollbar on `.math-display`, message column does not widen, no vertical scrollbar, no spurious scrollbar on tagged displays.
  4. **Tags:** Number right-aligned while formula stays centred (`:has(> .katex-tag)` flex rule); wide formulas never overlap tags; per-row tags align with their rows.
  5. **Glue:** Narrow viewport never breaks a line between an inline formula and its trailing punctuation (`.math-glue`).
  6. **Errors:** `--status-err` legible in light and dark themes; theme switching leaves no stale colours.
  7. **Fonts:** Zero 404s in network log; `\mathscr`, `\mathfrak`, `\mathbb`, and bold italic render.
  8. **Streaming:** No flicker as formulas close; scroll-to-bottom tracking holds; frame time stays smooth on long macro-heavy answers.
  9. **Eqref:** Click smooth-scrolls the right equation to centre in stream and canvas pane, scoped strictly to the same render root.
  10. **Special containers:** `.thinking-notes` (`pre-wrap`), `.investigation-inner p` (`pre-line`), `.dag-node-summary`, `.verdict-issue-desc`.
  11. **Tables:** Wide tables scroll inside `.table-wrap`; math in cells is not clipped.
  12. **Copy:** Record what lands on the clipboard when selecting rendered HTML-only math.
  13. **Other surfaces:** Hero, worked-example dialog, manuscript preview, `.tex` source reader on a real paper with preamble macros.
  14. **Export note:** `crates/export/src/html.rs` is unchanged by this plan.
