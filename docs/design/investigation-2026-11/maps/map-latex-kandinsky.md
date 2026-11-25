# Kandinsky LaTeX / Markdown-math rendering: behavioural spec

Paths are relative to `C:\Users\antimajorana\Documents\GitHub\kandinsky` (branch `fix/graph-zoom`). Behaviour is described in my own words; no source is reproduced.

## 0. How this was checked, and what was not

- **Read in full:** `docs/guides/MD-RENDERING.md`, `ui/src/lib/{math,markdown,preview,equations,tex-portable,rich-title,lazy-render,math-align,math-close}.ts`, `ui/src/components/Markdown.svelte`, `ui/src/lib/editor/math-preview.ts`, `ui/src/lib/editor/live/obsidian-syntax.js`, `ui/src/lib/editor/pigment-pills.ts`, `ui/src/lib/hydrate.ts`, `ui/src/styles/markdown.css`, `ui/package.json`, `ui/audit/render.spec.ts`, and the tests `md-render`, `equations`, `math-align`, `math-preview`, `live-math-source`, `math-close`.
- **Read in part:** `live-preview.js` (math parts only, of 2,149 lines), `note-extensions.ts` (150-300), `theme.ts` (28-47), `editor.css`, `chrome.css`, `Popover.svelte`, `html-paste.ts` (160-210), `code-fence-chrome.ts` (head and grep).
- **Not read:** `display-math.ts`, `slash-menu.ts`, `note-anchor.ts`, the mermaid and TikZ code, anything under `plugins/`.
- **Not run:** Kandinsky itself and its Playwright tests. `ui/node_modules` is absent in this checkout.
- **KaTeX probes:** I ran throwaway node scripts against re:Search's vendored KaTeX 0.19.1 (`crates/desktop-server/vendor/katex/katex.min.js`) with Kandinsky's option set. Kandinsky locks 0.19.0, so results marked "(probe)" are from a patch release later.
- **Probe scripts:** `<local scratch file>` and `<local scratch file>`.
- **"(derived)"** marks behaviour traced from the code but not seen in a running app.

## 1. Libraries and versions

| Library | Declared | Locked | Evidence |
|---|---|---|---|
| KaTeX | `^0.19.0` | 0.19.0 | `ui/package.json:57`, `ui/package-lock.json:4120` |
| markdown-it | `^15.0.2` | 15.0.2 | `ui/package.json:59`, `ui/package-lock.json:4520` |
| DOMPurify | `^3.4.16` | 3.4.16 | `ui/package.json:55` |
| highlight.js | `^11.12.0` | 11.12.0 | `ui/package.json:56` |
| CodeMirror 6 markdown / lezer | `^6.5.2` / `^1.7.2` | not checked | `ui/package.json:28,45` |

- KaTeX's stock stylesheet is loaded once, globally (`ui/src/main.ts:18`).
- No KaTeX extension is used: a grep of `ui/src` and `package.json` for mhchem, copy-tex, auto-render and `katex/contrib` found nothing.
- No markdown-it plugin package is used. Every Markdown extension is a hand-written rule in `ui/src/lib/markdown.ts`.
- `mermaid` and `@drgrice1/tikzjax` are dependencies (`ui/package.json:60,36`); I did not read how they are wired.
- For re:Search: the vendored `auto-render.min.js` is not part of Kandinsky's behaviour. Matching Kandinsky means doing delimiter detection yourself and calling KaTeX per formula.

## 2. Delimiters and environments

### 2a. Reading view (markdown-it rules, `ui/src/lib/markdown.ts`)

| Form | Where it is recognised | Result |
|---|---|---|
| `$…$` | inline (`markdown.ts:25-53`) | inline KaTeX |
| `$$…$$` at line start | block; may interrupt a paragraph, list, quote (`markdown.ts:56-106, 372`) | display KaTeX in `div.math-display` |
| `$$…$$` mid-paragraph | inline rule, first branch (`markdown.ts:28-35`) | display-mode KaTeX in `span.math-display`, drawn as its own centred block |
| `\[…\]` at line start | block, same rules as `$$` (`markdown.ts:373`) | display KaTeX |
| bare `\begin{equation\|align\|gather\|multline\|eqnarray}` (starred too) at line start | block, up to the matching `\end{…}` (`markdown.ts:108-138`) | display KaTeX |
| `\eqref{id}` in prose | inline, before backslash-escape handling (`markdown.ts:140-153, 370`) | a link reading `(n)` |
| `\(…\)` | **not recognised** | markdown-it's escape rule leaves literal parentheses |
| `\[…\]` mid-line | **not recognised** | literal text |

The last two rows matter for model-written answers. Models often emit `\(…\)`, so re:Search must add it deliberately rather than expect it from "matching Kandinsky".

**Inline `$…$` rules** (`markdown.ts:36-52`):
- The character after the opening `$` must not be a space, a tab or the end of text.
- The closer is the next `$` not preceded by a backslash. Empty content is rejected.
- The character before the closing `$` must not be a space or tab.
- The character after the closing `$` must not be a digit (the money heuristic; the code's own example is "$5 and $6").
- `\$` in prose is a literal dollar, because the escape rule runs before the math rule (`markdown.ts:371`).
- There is no newline check, so inline math can span a soft line break inside one paragraph (derived). The editor differs (2b).
- Up to four characters of trailing punctuation from the set `. , ; : ! ? ) ]` and the curly closing quotes `’ ”` are captured with the formula. They are emitted inside a `span.math-glue` with `white-space: nowrap`, so a line never breaks between a formula and its comma (`markdown.ts:49-51, 415`; `markdown.css:142`).

**Display block rules** (`markdown.ts:56-106`, `ui/src/lib/math-close.ts:7-27`, `docs/guides/MD-RENDERING.md:24-34`):
- The opener must be indented less than 4 columns relative to its container.
- **An unclosed opener is text.** The closer must appear before the next Markdown heading line (0-3 spaces, 1-6 `#`, then whitespace or end of line) or the end of the document. Otherwise the `$$` is not math and the rest of the note is unaffected. Blank lines inside a formula are allowed. The search scans in 8 KB chunks.
- The closer is the next `$$` anywhere, including mid-line. Prose after a mid-line closer becomes a following paragraph.
- `$$$$` on one line closes one formula and opens the next; two displays result.
- Content is collected line by line, so quote markers (`> `) and list indentation never reach the TeX. The content is then trimmed.
- A paragraph consisting only of `^block-id` right after a display is removed; it is an address, not text (`markdown.ts:224-229`).

**Math inside code is left alone:**
- Fenced code is claimed before the math block rule.
- An inline code span that starts before the `$` wins.
- The equation numbering scans skip fenced lines (`equations.ts:95`, `ui/src/lib/fence-walk.ts`).
- HTML-comment stripping preserves code (`markdown.ts:512-515`).
- **Indented code blocks are disabled** (`markdown.ts:369`). Text indented by 4 spaces or a tab stays prose and its math renders (test: `ui/tests/md-render.spec.ts:103-107`).

### 2b. Editor live preview (lezer, `obsidian-syntax.js`)

- **InlineMath** (`obsidian-syntax.js:216-246`): `$` not followed by `$`, up to the next unescaped `$` on the same line, non-empty. There are no whitespace or currency heuristics here, so "$5 and $6" would be math in the editor but not in the reading view (derived).
- **BlockMath** (`obsidian-syntax.js:327-405`): `$$`, `\[`, or `\begin{equation|align|gather|multline|eqnarray}[*]` at line start. It uses the same closed-before-next-heading test and the same mid-line pairing as the reader.
- **Eqref** (`obsidian-syntax.js:190-213`): `\eqref{id}` in prose.
- Indented code is removed here too (`obsidian-syntax.js:435`).
- **Reveal rule:** a formula shows its source whenever the selection touches its range, delimiters included; otherwise it is drawn (`live-preview.js:130-135, 1683-1686`).
- **While editing** (`math-preview.ts`): inline math floats in a tooltip above the line; display math is drawn in a card under the formula. A half-typed inline formula (odd number of unescaped `$` before the caret) is previewed before its closer exists (`math-preview.ts:61-70`).
- In `.tex` files only, the preview also understands `\(…\)` and `displaymath` (`math-preview.ts:75-85`).

## 3. KaTeX options

All formula rendering goes through one option builder (`ui/src/lib/math.ts:43-51`).

| Option | Value |
|---|---|
| `displayMode` | per formula: true for block and display forms, false for inline |
| `throwOnError` | `false` |
| `output` | `"html"` (no MathML, no TeX annotation in the DOM) |
| `strict` | `"ignore"` (unicode in math, etc., accepted silently) |
| `globalGroup` | `true` |
| `macros` | one shared, mutable, session-wide table seeded as below |
| `errorColor` | `"var(--red)"` (theme token, not a hex value) |
| `trust`, `maxSize`, `maxExpand`, `fleqn`, `leqno`, `minRuleThickness`, `colorIsTextColor` | **not set**, so KaTeX defaults apply |

Consequences of the unset options (probe):
- `\href`, `\includegraphics` and `\htmlClass` are not honoured; the command name is drawn as red text.
- A runaway macro stops with "Too many expansions".
- Displays are centred, with tags on the right.

**Predefined macros** (`ui/src/lib/tex-portable.ts:6-16`):

| Macro | Expansion |
|---|---|
| `\R` | `\mathbb{R}` |
| `\C` | `\mathbb{C}` |
| `\N` | `\mathbb{N}` |
| `\Z` | `\mathbb{Z}` |
| `\ket` | `\left| #1\right\rangle` |
| `\bra` | `\left\langle #1\right|` |
| `\braket` | `\left\langle #1\right\rangle` |
| `\Tr` | `\operatorname{Tr}` |
| `\mbox` | `\text{#1}` |

**Is a macro defined in one formula visible in later ones? Yes.**
- The table is a single module-level object and `globalGroup` is true, so `\def`, `\newcommand` and `\renewcommand` in any formula persist for every later formula (`math.ts:39-41`).
- This holds for the whole app session, across notes, not per document.
- Probe: with `globalGroup: true` a `\def` lands in the shared table; with `false` it does not.
- It is order-dependent: a formula drawn before the definition does not see it.

**Two call sites use different options:**
- Bibliographic titles (`ui/src/lib/rich-title.ts:16`) use `throwOnError: true`, `output: "html"`, and no macros.
- The "is this command already defined" probe (`math.ts:60`) uses `throwOnError: true` and an empty macro table.

## 4. Numbering, `\tag`, `\label`, `\eqref`

Numbering is done by the app, never left to KaTeX (`ui/src/lib/equations.ts`; tests in `ui/tests/equations.spec.ts`).

**Which displays are numbered** (`equations.ts:88-130`):
- A `$$` display is numbered if it carries `\label{id}`, or has a block id after the closer (`$$ ^id`), or contains a non-starred `equation`, `align`, `gather`, `multline` or `eqnarray`.
- Plain `$$…$$`, starred environments, and anything inside a code fence are not numbered.
- Numbers run 1, 2, 3… in document order. Adding an equation above renumbers those below.
- A display with its own `\tag{X}` keeps `X` and consumes no number. `\notag` or `\nonumber` means no number.
- `align`, `gather` and `eqnarray` are numbered per row. A row's `\label` names that row. Nested environments (`cases`, matrices) and braces are skipped when splitting rows. A trailing `\\` makes no extra row. A block id after the closer names the last numbered row.

**What KaTeX receives** (`equations.ts:144-166`, `markdown.ts:417-427`):
- Every `\label{…}` is deleted. Probe: KaTeX otherwise draws `\label` as red text.
- A single-number display gets `\tag{n}` appended, and its numbered environment is replaced by the unnumbered twin (see section 7).
- A multi-row display gets `\tag{n}` appended to each numbered row; `eqnarray` is renamed `align`.
- The wrapper `div.math-display` carries `data-eq-id` set to the first `\label` id, or failing that the first `\tag` text.

**`\eqref{id}`:**
- In prose it becomes a link with the classes `internal-link eqref-link`. The text is `(n)` if the label is known, otherwise `(id)` (`markdown.ts:404-408`).
- Inside a formula it is replaced before KaTeX by upright text `(n)` or `(id)`. An inline formula containing one is wrapped whole in a link to the first id (`markdown.ts:338-347, 409-416`).
- `\ref` and `\cref` are not rendered as links. They are recognised only by the Ctrl-hover preview (`preview.ts:35`).

**Wikilink references:** `[[#^id]]` reads `(3)`; `[[Note#^id]]` reads `Note (3)`, filled in asynchronously (`equations.ts:185-193`, `hydrate.ts:35-42`).

**Interaction** (`Markdown.svelte:48-57`, `preview.ts:33, 448-469`):
- Clicking an eqref link smooth-scrolls the matching `data-eq-id` element to the centre of the view.
- Hovering one opens a card after 420 ms (60 ms with Ctrl/Cmd held). The card is up to 460 px wide, shows the equation in display mode, and carries a badge "Equation (n)".
- Ctrl-hovering a bare `(3)` in prose opens the same card if such an equation exists.

**Link styling** (`markdown.css:31-32`): accent colour, weight 500, tabular numerals, a faint underline that strengthens on hover.

**Tag placement** (`markdown.css:136-140, 292`): a tagged display becomes a flex row with a spacer, the formula, then the tag right-aligned with 1.5em left padding. The formula stays centred and the number follows the formula rather than overlapping it when the formula is wide. The tag also gets 2px right padding to prevent a spurious scrollbar.

**Quirks found; do not copy blindly:**
- **Two or more numbered rows fail (derived + probe).** After per-row `\tag`s are added, the reading view still renames `align` to `aligned` (`markdown.ts:423-424`). KaTeX rejects several `\tag`s inside `aligned` with "Multiple \tag", while real `align` with per-row tags renders fine. So an `align` with two or more numbered rows appears to draw as an error in Kandinsky. No app-level test covers it; `equations.spec.ts` checks strings only. For re:Search, keep `align`/`gather` when there are per-row tags, and use the `aligned`/`gathered` twin only for a single whole-display tag.
- **`\[…\]` and bare `\begin{…}` blocks are numbered by a second scan** (`pigment-pills.ts:157-186`). It starts counting at "number of labelled `$$` equations + 1", which can disagree with the main count (derived).

## 5. A formula that fails

- **Syntax error** (probe: `x^`, `x^2^3`, an unbalanced brace): KaTeX does not throw. The whole formula is replaced by its raw source in a `span.katex-error`, coloured with the red token, with the full parse message as the hover title. CSS forces the colour (`markdown.css:146`).
- **Unknown command** (probe: `\dv`, `\ce`, `\label`, `\ref`): the rest of the formula still renders. Only the command name is drawn as red raw text.
- **Renderer throws anyway:** the source is shown HTML-escaped in `code.math-error`, red, monospace, .88em (`math.ts:89-91`, `markdown.css:143`).
- **Editor widget failure:** the source is shown as text with class `cm-math-error` (`live-preview.js:516-522`).
- **While typing** (`math-preview.ts:129-147`, `theme.ts:43-44`; test `ui/tests/math-preview.spec.ts:16-32`): the preview keeps the last good drawing at 45% opacity and shows KaTeX's reason under it in small red monospace, with the "KaTeX parse error:" prefix removed. This avoids flicker.
- **Audit:** the render audit counts every `.katex-error` on screen as a defect and reports its title (`ui/audit/render.spec.ts:42`).

## 6. Visual result

**Size and line height:**
- Body text is 15.5px × UI scale with line-height 1.68 (`markdown.css:5`).
- KaTeX is 1.08em of that (`markdown.css:133`), against KaTeX's own default of 1.21em. Inline math therefore sits close to the text x-height and does not push lines apart.
- No further inline line-height override exists.
- The editor's typing preview uses 1.05em (`theme.ts:36`).

**Display math** (`markdown.css:134-135`):
- The wrapper is a centred block with `overflow-x: auto` and `overflow-y: hidden`.
- It has .55em vertical padding and .7em vertical margin.
- KaTeX's own 1em display margin is zeroed.
- A wide formula scrolls horizontally inside its own box. It is never shrunk and never widens the page.

**Inline math:** in the editor it is `inline-block`, with the list hanging indent reset so a formula in a list item does not overlap the previous word (`markdown.css:141`, `editor.css:204-205`; test `md-render.spec.ts:109-119`).

**Colour and theme:** nothing math-specific. Glyphs inherit the text colour, so light and dark themes work through the `[data-theme]` token swap (`ui/src/styles/tokens.css:31,85`). Only errors use the red token. (Inferred from the CSS; not viewed on screen.)

**RTL notes:** math is forced left-to-right and bidi-isolated (`chrome.css:181-183`).

**Selection and copy:**
- Because output is HTML only and copy-tex is absent, selecting rendered math copies the glyph text, not TeX. There is no "copy as TeX".
- In the editor, the route to the source is clicking the formula. A rendered block has a neutral hover tint, a text cursor and a suppressed native selection highlight. A click anywhere except on links, buttons or inputs puts the caret in and reveals the source (`editor.css:99, 125-129`; `live-preview.js:1048-1060`).
- Pasting HTML that contains KaTeX, MathJax or MathML with a TeX annotation is converted back to TeX (`html-paste.ts:168-183`; read in part).

**Revealed source in the editor** (`markdown.css:144-145, 285-289`; test `ui/tests/live-math-source.spec.ts`):
- Monospace at .88em, delimiters faint, LaTeX highlighted by highlight.js.
- Bracket pairs are coloured by depth in three rotating colours, and the caret's pair is ringed.
- `\left` and `\right` are shrunk and faded.

## 7. Pre-processing that makes LaTeX "just work"

For a display block in the reading view, these steps run in order before KaTeX:

1. **Strip container prefixes and trim** (`markdown.ts:78-83`).
2. **Delete all `\label{…}`** and add number tags (`equations.ts:144-157`).
3. **Replace `\eqref{id}`** with upright text `(n)` or `(id)`.
4. **Rewrite environments** (`markdown.ts:423-424`): `equation` and `multline` wrappers are deleted with their content kept; `align` and `eqnarray` become `aligned`; `gather` becomes `gathered`; the star is dropped.
   - Probe: `eqnarray` and `multline` do not exist in KaTeX.
   - Probe: top-level `\\` line breaks are accepted in display mode, so a `multline` body still breaks lines.
   - Probe: non-starred `align`/`equation` left alone would be auto-numbered by KaTeX through a page-global CSS counter, which is the reason for the rewrite.
5. **`$$align` shorthand** (`ui/src/lib/math-align.ts`; test `ui/tests/math-align.spec.ts`):
   - A display whose first line is exactly `align` or `align*` is treated as plain lines and expanded into that environment.
   - Each line gets `&` before its first relation outside any brackets. Relations include `=`, `<`, `>`, `:=`, `\leq`, `\geq`, `\neq`, `\approx`, `\equiv`, `\sim`, `\simeq`, `\cong`, `\propto`, `\Rightarrow`, `\implies`, `\iff`, `\ll`, `\gg`; the full list is at `math-align.ts:5-7`.
   - Lines are joined with `\\`. A line starting with `+`, `-`, `\cdot`, `\times`, `\pm` or `\mp` is prefixed with `&\quad`. A line that already has its own `&` is left alone. A trailing `\\` is removed.
   - This runs after step 4, so the non-starred form reaches KaTeX as a real `align` and shows KaTeX's automatic page-counter numbers (derived + probe). That is probably unintended.
6. **`\newcommand` / `\renewcommand` normalisation** (`math.ts:54-76`; test `md-render.spec.ts:167-171`):
   - Either form, with or without braces around the name, is rewritten to whichever KaTeX will accept.
   - `renewcommand` is used if the name is already defined, as a built-in or from earlier in the session; otherwise `newcommand`.
   - This makes `\newcommand{\div}…` work, and makes rendering the same note twice work. Probe: both fail without the rewrite. Answers are cached per name.
7. **`\text{…}` repair** (`math.ts:76`): inside a `\text{}` with no nested braces, an unescaped `_` becomes `\_` and an unescaped `^` becomes `\^{}`. So `\text{Nb_3Ge}` renders (probe: it fails without the repair). It is not applied to `\mathrm`, `\textrm`, or the body of `\mbox`.
8. **`\mbox`** is supplied as a macro (section 3).
9. **Unicode** in math is accepted because `strict` is `"ignore"` (probe: `α + β ≤ γ → ∞` renders).

For inline math only steps 3 and 6-9 apply. An `align` in inline mode is an error (probe).

Not handled: `\(…\)`, mid-line `\[…\]`, mhchem `\ce`, the physics package (`\dv`), `\ref`. Each shows as literal text or red command text.

Editor paths apply the same label stripping and environment rewrite on their own (`note-extensions.ts:194-204`, `math-preview.ts:18-33`). Display math in the editor is in practice drawn through the reader's renderer (`live-preview.js:974-979`, `note-extensions.ts:265-277`), so both views agree.

## 8. Caching and lazy rendering

- **Lazy load:** KaTeX is fetched on first use (`math.ts:14-21`); the app shell also calls the loader at start (`ui/src/App.svelte:214`).
- **Pending placeholder:** until KaTeX arrives a formula is a `span.math-pending` holding its HTML-escaped source, with a `data-display` attribute for display math (`math.ts:30-31`).
- **On arrival:** every pending span in the document is rendered in place; a failure adds the class `math-error`. A "renderer arrived" signal then fires (`lazy-render.ts`). Reading views re-render (`Markdown.svelte:26`), block caches clear (`markdown.ts:575`), and editors redecorate.
- **Caches:**
  - Rendered-block HTML: 300 entries, keyed by note folder plus source text, oldest dropped first.
  - Inline raw-HTML fragments: 2,000 entries, same policy.
  - Both are cleared on a file-list change, on renderer arrival, and when the line-break setting changes (`markdown.ts:572-591`).
- **Editor widgets:** keyed by display/inline, renderer-ready flag, resolved equation number and source, so unchanged formulas keep their DOM across keystrokes (`live-preview.js:494-501, 1692`). Equation numbering is computed once per document state (`note-extensions.ts:177-185`).
- **No viewport laziness of its own:** there is no IntersectionObserver or idle queue for formulas. The reading view renders all formulas synchronously to strings. The editor relies on CodeMirror drawing only the viewport (inference).
- **Sanitising:** all reader HTML, KaTeX output included, passes through DOMPurify with `<style>` forbidden and a small attribute allow-list (`markdown.ts:542, 556-566`).

## 9. Markdown around math

**Parser settings** (`markdown.ts:350-357, 369`): raw HTML on (sanitised), linkify on, typographer off, indented code off. A single newline inside a paragraph is a visible line break by default; a setting restores the CommonMark rule (`MD-RENDERING.md:135-142`).

- **Tables:** wrapped in a horizontally scrollable `div.table-wrap`. Full width, .92em, row dividers only, cells wrap between words (`markdown.ts:496-497`, `markdown.css:111-119`). Inline math works in cells. A `|` inside a formula still splits the cell (inference from markdown-it; untested).
- **Lists:** standard, plus task boxes with any single status character (`markdown.ts:196-221`). Display math inside a list item or quote works because prefixes are stripped.
- **Callouts:** `> [!type] Title` becomes a titled box. `[!type]-` is a collapsed `<details>`, `[!type]+` an open one (`markdown.ts:297-321, 469-494`). There are colour and glyph families for note, tip, theorem, proof, warning, example and quote types (`markdown.css:148-166`). Math renders inside.
- **Code fences:** highlighted by highlight.js with languages loaded on demand and a language label. A copy button appears on hover, but not on math blocks (`markdown.ts:358-366`, `code-fence-chrome.ts:109`). Fence contents are never scanned for math.
- **Footnotes:** not supported in the reading view. No rule exists, so `[^1]` stays literal (derived). The editor styles `[^1]` as a superscript (`editor.css:224`; test `md-render.spec.ts:245`).
- **Other:** `==highlight==`, `#tags`, wikilinks and embeds, `[TOC]`, `<!-- comments -->` hidden outside code, and a `columns` fence.

## 10. Examples

| # | Input | Outcome | Basis |
|---|---|---|---|
| 1 | `Where $\phi_{0}$ and $\phi_{e}$ are orbitals.` | two inline formulas, no visible `$` | `md-render.spec.ts:15, 71-76` |
| 2 | A `$$` block whose closing line is `$$which in space…` | one display; "which in space…" is a paragraph after it; later formulas unaffected | same test |
| 3 | `$$` / `U_q = s/V` / `$$$$` / `H = N^2 s` / `$$` | two displays, no stray `$$` | `md-render.spec.ts:19, 78-82` |
| 4 | `$$` / `x = 1` / blank / `## Next part` … | the unclosed `$$` stays text; the heading renders; a later closed `$$ z = 2 $$` and inline `$y$` still render | `md-render.spec.ts:18, 84-90` |
| 5 | `$$ x #1 $$` | closed formula; `#` inside TeX is not a heading | `md-render.spec.ts:97` |
| 6 | A display with a blank line inside | still one formula | `md-render.spec.ts:94` |
| 7 | List item, then a 4-space or tab-indented line `However we choose $b$ here` | prose with rendered math, not a code block | `md-render.spec.ts:21, 103-107` |
| 8 | `Cases $a$, then $b$. And $(c)$; and $d$ alone` | `,` `.` `;` are each glued to their formula (no wrap between); `$d$` has no glue | `md-render.spec.ts:42, 303-310` |
| 9 | `$\newcommand{\div}[1]{\nabla \cdot #1}$` then `$\div{E}$` | no error; `\div{E}` draws ∇·E, also when the note is rendered twice | `md-render.spec.ts:30, 167-171` |
| 10 | `$\text{Nb_3Ge}$` | text "Nb_3Ge" with a literal underscore, no error | same test; probe |
| 11 | `$k_{\mbox{\tiny B}}T$` | renders; `\mbox` acts as `\text` | same test |
| 12 | `It costs $5 and $6 more` | no math in the reading view; both dollars literal | rule at `markdown.ts:44-47` (derived) |
| 13 | `Price \$5 and $x$` | literal `$5`; `x` as math | escape-rule order (derived) |
| 14 | `` `$x$` `` or a fenced block containing `$$ h \label{eq-example} $$` | shown verbatim as code; not numbered | `equations.spec.ts:21-23, 34` |
| 15 | `$$` / `a = b \label{eq-first}` / `$$` | display with `(1)` at the right; no "label" text; `\eqref{eq-first}` elsewhere reads `(1)` and scrolls to it | `equations.spec.ts:34-35, 48` |
| 16 | `$$ c = d $$` | display, no number | `equations.spec.ts:38` |
| 17 | `$$ g \tag{D1} \label{eq-own} $$` | shows `(D1)`; consumes no number; references read `(D1)` | `equations.spec.ts:34` |
| 18 | `$$ i \notag \label{eq-none} $$` | no number | `equations.spec.ts:34` |
| 19 | `\begin{equation} k = l \end{equation}` numbered 3 | KaTeX receives ` k = l\tag{3}` | `equations.spec.ts:49` |
| 20 | `\begin{eqnarray} a &=& b \\ c &=& d \end{eqnarray}` numbered 4, 5 | renamed to `align` with `\tag{4}` and `\tag{5}` on the rows; see the section 4 quirk for what happens next | `equations.spec.ts:78` |
| 21 | An `align` with rows: labelled; `\notag` with nested `cases`; `\tag{T}`; labelled | row numbers 1, none, T, 2; the nested `\\` does not split rows | `equations.spec.ts:56-77` |
| 22 | `$$` / `align` / `a = b + c` / `d \leq e` / `$$` | an `align` with rows `a &= b + c` and `d &\leq e` | `math-align.spec.ts:7` |
| 23 | A `$$align` block with lines `= b`, `+ c`, `f(x) &= 2 \\` | rows `&= b`, `&\quad + c`, `f(x) &= 2` | `math-align.spec.ts:10` |
| 24 | `\[` / `x^2 + y_1` / `\]` on their own lines | display formula; the `\[` lines disappear | `math-preview.spec.ts:83-95` |
| 25 | `see \(x^2\) here` | not math; literal parentheses | no rule (derived) |
| 26 | `$x^$` | raw `x^` in red with the parse message as tooltip; while typing, the last good drawing dimmed plus the reason | probe; `math-preview.spec.ts:23-28` |
| 27 | `$\dv{f}{x}$` or `$\ce{H2O}$` | the formula renders with `\dv` / `\ce` as red raw text | probe |
| 28 | `$\def\zz#1{[#1]}$` … later `$\zz{q}$` | the second formula uses the definition | probe |
| 29 | `$α + β ≤ γ$` | renders, no warning | probe |
| 30 | A very wide `$$…$$` | its own horizontal scrollbar; centred when it fits; page width unchanged | `markdown.css:134` |

## 11. Acceptance shapes Kandinsky tests for

The audit (`ui/audit/render.spec.ts:18-58`) opens every note and flags, in visible prose outside math and code:

- text that still looks like `$…$`;
- a line starting with `$$`;
- any `.katex-error`;
- raw HTML tags;
- a table whose scroll width exceeds its box by more than 2px;
- a block with a vertical margin (editor only).

These are directly reusable as re:Search UI test assertions on rendered answers: no visible raw-math text, and zero `.katex-error` for well-formed input.
