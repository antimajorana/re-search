# Audit report: `audit-ui`

- **Base commit:** `351b1287c9a54a700b0e33111b3891f195afd6d1` (`origin/gemini/work/merged`, compared against `origin/main` at `7c36a0e`)
- **Work branch:** `work/audit-ui` → `gemini/work/audit-ui`
- **Lens:** `ui` (`crates/desktop-server/assets` and UI-facing asset/route contracts)
- **Production/existing-test edits:** None (`0` modified files; only new `audit_ui_*.test.mjs` files and this report added)

## Summary of findings

18 findings ordered most severe first (`4 critical`, `7 major`, `6 minor`, `1 nit`):

- **17 findings (`F1`–`F17`)** are proven with failing automated Node tests in:
  - `crates/desktop-server/tests/audit_ui_latex_findings.test.mjs` (`10` failing proof tests)
  - `crates/desktop-server/tests/audit_ui_investigation_findings.test.mjs` (`7` failing proof tests)
- **1 finding (`F18`)** covers static dead/duplicated code proven by line-level inspection.
- **6 additional Design 03 parity tests** omitted by `math_parity.test.mjs` were added in `crates/desktop-server/tests/audit_ui_design03_extra.test.mjs` (`6/6` passing).

---

## Findings (most severe first)

### F1 — `critical` — Mid-line `$$` or `\[` opener closed at line start phase-shifts all subsequent display math (`crates/desktop-server/assets/markdown.js:107-120, 284-301`)

- **What is wrong:** `matchBlockDisplay` only recognizes `$$` or `\[` at the start of a line (`^ {0,3}\$\$`, `^ {0,3}\\\[`), whereas models frequently open a display at the end of a prose line (`We obtain: $$\nH = p^2 / 2m\n$$`). Lines 0 and 1 accumulate in `para` without flushing; when the loop reaches line 2 (`$$`), `matchBlockDisplay` treats that **closing** `$$` as a **new block display opener**, flushes `para` (leaving the first `$$` unclosed), and pairs line 2's `$$` with the **opening** `$$` of the next equation. Every subsequent display in the message is inverted: prose between equations renders inside KaTeX math mode (`andthereforetheenergyis:`), while the equations themselves drop out into plain unrendered text.
- **Concrete input:**
  ```markdown
  We obtain the Hamiltonian: $$
  H = p^2 / 2m
  $$
  and therefore the energy is:
  $$
  E \ge 0
  $$
  ```
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F1 [critical] markdown.js:284-301: mid-line $$ opener closed at line start must not phase-shift subsequent displays
    AssertionError [ERR_ASSERTION]: Expected both H = p^2 / 2m and E >= 0 to render as displays
    1 !== 2
  ```
  (The single rendered `.math-display` has `data-tex="and therefore the energy is:"`, while `H = p^2 / 2m` and `E \ge 0` remain unrendered prose with literal `$$`.)
- **Proposed fix:** Before treating a line-start `$$` or `\[` as a new block opener in `renderMarkdownBlocks`, check whether `para` already contains an unclosed mid-line `$$` or `\[` opener (via `hasUnescapedMathDelim` / `extractMath` scan state). If `para` holds an unclosed display opener, append lines through the matching closer to `para` (so `scanInlineText` extracts the `span.math-display`) instead of opening an inverted block display on the closer line.

---

### F2 — `critical` — `repairTextCmd` corrupts exponents and subscripts inside nested `$…$` / `\(…\)` math in `\text{…}` (`crates/desktop-server/assets/math_katex.js:99-105`)

- **What is wrong:** `repairTextCmd` matches `\text{([^{}]*)}` and unconditionally rewrites every unescaped `_` to `\_` and `^` to `\^{}`. Standard LaTeX frequently embeds inline math inside `\text{…}` (e.g. `\begin{cases} 1 & \text{if $x^2 < 1$ and $x_i > 0$} \end{cases}`). When brace-free math appears inside `\text{…}`, `x^2` is rewritten to `x\^{}2` and `x_i` to `x\_i`. In KaTeX math mode, `\^{}` is valid syntax for a circumflex accent over an empty group and `\_` is a literal underscore, so KaTeX silently renders the mathematically false formula `if xˆ2 < 1 and x_i > 0` with zero `.katex-error` warning.
- **Concrete input:**
  ```latex
  $$
  \begin{cases} 1 & \text{if $x^2 < 1$ and $x_i > 0$} \\ 0 & \text{otherwise} \end{cases}
  $$
  ```
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F2 [critical] math_katex.js:99-105: repairTextCmd must not corrupt subscripts and exponents inside nested $...$ in \text{...}
    AssertionError [ERR_ASSERTION]: Exponent ^2 inside \text{$x^2$} was corrupted into circumflex accent: {10if xˆ2<1 and x_i>0otherwise
  ```
- **Proposed fix:** Walk each `\text{…}` body character-by-character (supporting balanced braces as well) and toggle a `inNestedMath` flag across unescaped `$…$` and `\(…\)` spans, escaping `_` and `^` only while `inNestedMath === false`.

---

### F3 — `critical` — `harvestPreambleMacros` discards `[argc][default]` and `\def` parameter tokens, silently misbinding formula arguments (`crates/desktop-server/assets/tex_preview.js:48-87`)

- **What is wrong:** In `harvestPreambleMacros`, lines `58-65` read `[argc]` and `[default]` on `\newcommand` / `\renewcommand` / `\providecommand` and **discard both**, storing `macros[name] = body.content` as a plain string; lines `77-83` likewise match and discard `\def\foo#1#2` parameter tokens (`([^{]*)`). KaTeX's string macro expander determines arity from the highest `#n` placeholder in the string and has no knowledge of the discarded optional default. When a TeX file defines `\newcommand{\norm}[2][2]{\|#2\|_{#1}}` and writes `$\norm{x} + y$` (omitting the optional `[2]`), KaTeX treats `\norm` as a 2-mandatory-argument macro, binds `#1 = x` and `#2 = +`, and silently renders `∥+∥_x y` instead of `∥x∥_2 + y`. Similarly, `\newcommand{\blank}[1]{}` (where `#1` does not appear in the body) fails to consume `{arg}` and leaks the argument into the formula.
- **Concrete input:**
  ```latex
  \documentclass{article}
  \newcommand{\norm}[2][2]{\|#2\|_{#1}}
  \begin{document}
  We bound $\norm{x} + y$.
  \end{document}
  ```
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F3 [critical] tex_preview.js:48-87: preamble \newcommand with optional default arg must not misbind arguments
    AssertionError [ERR_ASSERTION]: \norm{x} + y consumed "+" as #2 and rendered wrong formula: We bound ∥+∥xy.
  ```
- **Proposed fix:** Instead of stripping `[argc][default]` into `state.macros` strings, replay harvested preamble `\newcommand`/`\renewcommand`/`\DeclareMathOperator`/`\def` declarations once through KaTeX (`katex.renderToString(preambleDefs, katexOptions(false, docState))` with `globalGroup: true`), so KaTeX's native macro engine records exact arities, optional-argument defaults, and delimited `\def` parameter patterns in `docState.macros`.

---

### F4 — `critical` — Trailing text after a mid-line display closer is re-parsed as a Markdown block container, dropping leading `>` inequalities and `-` signs (`crates/desktop-server/assets/markdown.js:294-297`)

- **What is wrong:** Design 03 (§2 lines 61 & 67, §3 lines 237 & 241) specifies that trailing text after a closing `$$`, `\]`, or `\end{…}` becomes a **following paragraph**. Instead, `renderMarkdownBlocks` mutates `lines[disp.closeLine] = disp.rest.trimStart(); i = disp.closeLine - 1;` and re-enters the top of the block loop. Continuation prose starting with mathematical operators or numbers is parsed as a block container:
  - `$$\na = b\n$$ > 0 for all inputs.` matches `^\s*>\s?` (`:280`), turning into `<blockquote><p>0 for all inputs.</p></blockquote>` and **deleting the `>` inequality sign**.
  - `$$\nx = 1\n$$ - 2 = -1.` matches `parseListMarker` (`:324`), turning into `<ul><li>2 = -1.</li></ul>` and **deleting the `-` minus sign**.
  - `$$\nx = 1\n$$ # case` becomes `<h1>case</h1>`.
- **Concrete input:** `$$\na = b\n$$ > 0 for all inputs.` and `$$\nx = 1\n$$ - 2 = -1.`
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F4 [critical] markdown.js:294-297: trailing text after mid-line display closer must become a paragraph, not blockquote or list
    AssertionError [ERR_ASSERTION]: Leading ">" in trailing prose became <blockquote>, dropping inequality: <div class="math-display math-block" data-tex="a = b" aria-label="a = b">...</div>
    <blockquote><p>0 for all inputs.</p></blockquote>
  ```
- **Proposed fix:** Push `disp.rest.trimStart()` directly onto `para` (`para.push(disp.rest.trimStart()); i = disp.closeLine;`) rather than rewriting `lines[disp.closeLine]` and re-running block-opener dispatch.

---

### F5 — `major` — TeX `%` comments inside numbered displays swallow `\tag{n}` and `align` `\\` row breaks (`crates/desktop-server/assets/math_equations.js:51-78, 131, 218`)

- **What is wrong:**
  1. In single-number displays (`:218`), `tok.prepared = `${body} \tag{${tagText}}`` appends `\tag{n}` to the end of `body` without stripping trailing `%` comments. If the last line has a TeX comment (`E = mc^2 % rest energy`), `\tag{1}` lands on the commented line and KaTeX ignores it: no `(1)` tag renders on the equation while `counter` still increments and `\eqref{1}` links to a tagless display.
  2. In multi-row environments (`:131`), `row = `${row.trimEnd()} \tag{${tagText}}`` strips the trailing newline after `% first row\n` and concatenates ` \tag{1}`—followed by `pieces.push(tex.slice(cursor, a))` (`\\`)—onto the same `%` comment line (`a &= b % first row \tag{1}\\`). Both `\tag{1}` and the `\\` row break are commented out, merging row 1 and row 2 onto a single line.
  3. `findEnvRowSpans` (`:51-78`) also fails to skip `%…\n` comments when scanning `\\`, `{`, `}`, `\begin`, or `\end`.
- **Concrete input:**
  ```latex
  \begin{equation}
  E = mc^2 % rest energy
  \end{equation}
  \begin{align}
  a &= b % first row
  \\
  c &= d
  \end{align}
  ```
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F5 [major] math_equations.js:51-78,131,218: TeX % comments in numbered equation and align must not swallow \tag or \\
    AssertionError [ERR_ASSERTION]: Tag (1) was swallowed by trailing % comment
    0 !== 1
  ```
- **Proposed fix:** Strip unescaped `%…` line comments (preserving `\%` and line breaks) at the start of `numberEquations`, `prepareInlineTex`, and `normalizeDisplayEnvsForKatex` (or skip `%…\n` in `findEnvRowSpans` and insert `\n\tag{n}\n` after any trailing `%` comment).

---

### F6 — `major` — `\begin{subequations}` containing multiple `equation` blocks concatenates formulas and collapses labels to one number (`crates/desktop-server/assets/math_equations.js:95-96, 173-227` & `math_scan.js:9`)

- **What is wrong:** `subequations` is in `MATH_ENVS`, so `findEnvBlockClose` captures the entire `\begin{subequations}…\end{subequations}` block as a single display token. When it contains two `\begin{equation}…\end{equation}` children (the standard LaTeX usage of `subequations`), `numberEquations` (`:173`) strips `\begin{subequations}`/`\end{subequations}`, finds no `MULTI_ROW_RE` match, and falls through to `normalizeSingleDisplayBody` (`:95-96`), which strips all inner `\begin{equation}` and `\end{equation}` tags without inserting a separator. KaTeX renders `a = 1b = 2 \tag{1}` as one mangled equation, and every `\label` inside `subequations` maps to `(1)`.
- **Concrete input:**
  ```latex
  \begin{subequations}
  \begin{equation} a = 1 \label{eq-a} \end{equation}
  \begin{equation} b = 2 \label{eq-b} \end{equation}
  \end{subequations}
  Refs \eqref{eq-a} and \eqref{eq-b}.
  ```
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F6 [major] math_equations.js:173-227: \begin{subequations} with multiple equations must keep distinct equations and numbers
    AssertionError [ERR_ASSERTION]: Distinct equations inside subequations received identical ref (1)
  ```
- **Proposed fix:** Either unwrap `\begin{subequations}` / `\end{subequations}` wrapper lines before display scanning (so inner `equation` / `align` blocks tokenize and number individually, or as `1a`, `1b`), or detect multiple inner display environments inside `numberEquations` and convert sequential `equation` blocks into rows of `gather*` with per-row tags (`1a`, `1b` or `1`, `2`).

---

### F7 — `major` — `\tag{…}` with nested braces triggers duplicate `\tag` error in display math and `\tag` error in `inlineOnly` surfaces (`crates/desktop-server/assets/math_equations.js:6` & `math_katex.js:109`)

- **What is wrong:** Both `OWN_TAG_RE = /\\tag\*?\s*\{([^{}]*)\}/` (`math_equations.js:6`) and `stripTagCommands` (`math_katex.js:109`) assume `\tag{…}` contains no inner braces. Tags with formatting commands (`\tag{\mathrm{I}}`, `\tag{1\text{a}}`, `\tag{\ref{eq1}'}`) fail to match:
  - In numbered displays (`equation`, `align`, or display with `\label`), `ownTag` is `null`, so `numberEquations` appends a second `\tag{1}` and KaTeX fails with `ParseError: KaTeX parse error: Multiple \tag`.
  - In `inlineOnly` surfaces (DAG cards, conventions summaries, claim headers), `stripTagCommands` leaves `\tag{1\text{a}}` intact, and KaTeX fails with `ParseError: KaTeX parse error: \tag works only in display equations`.
- **Concrete input:** `\begin{equation}\nE = mc^2 \tag{\mathrm{I}} \label{eq-rom}\n\end{equation}` and `mathTextToHtml('Step $E = mc^2 \\tag{1\\text{a}}$', { inlineOnly: true })`.
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F7 [major] math_equations.js:6 & math_katex.js:109: \tag{...} with nested braces must not cause Multiple \tag or inline error
    AssertionError [ERR_ASSERTION]: Nested braces in \tag{\mathrm{I}} caused KaTeX error: ...<span class="katex-error" title="ParseError: KaTeX parse error: Multiple \tag" style="color:var(--status-err)">E = mc^2 \tag{\mathrm{I}} \tag{1}</span>...
  ```
- **Proposed fix:** Parse `\tag` / `\tag*` arguments with the balanced-brace helper (`readBalancedBracesEnd`) in both `math_equations.js` and `stripTagCommands` in `math_katex.js`.

---

### F8 — `major` — Macro persistence breaks across collapsed/reversed Investigation answers on session switch and leaks on SSE stream re-renders (`crates/desktop-server/assets/investigation_answer.js:64-73, 102-127` & `app.js:425, 664, 776`)

- **What is wrong:**
  1. When switching away from and back to an Investigation session, `selectSession` (`app.js:542`) calls `resetMathMacros()`. `createAnswerCard.update` (`investigation_answer.js:64-73, 102-127`) then renders **only** `newest`, leaving `older` answers unrendered until `<details class="investigation-earlier-answers">` is toggled open—and when opened (`:110`), renders `older` in reverse chronological order (`[...older].reverse()`). Any macro defined in Q1's answer (`\def\Hop{\hat{H}_0}`) is missing from `state.macros` when Q2's answer renders on the Answer card (if the user stays in Paper view and Conversation messages do not carry Q1's answer text, or before `renderSessionStream` runs), producing undefined-control-sequence errors (`color:var(--status-err)`).
  2. Conversely, `renderSessionStream` (`stream.js:126`) does not reset macros, and `app.js` calls `renderSessionStream` on SSE reconnect (`:664`) and `turn_complete` (`:776`) without `resetMathMacros()`; `resetToBlankState` (`:425`) also omits `resetMathMacros()`. If message 1 renders `\foo` as `1` and message 2 redefines `\renewcommand{\foo}{2}`, a full stream re-render on reconnect/completion mutates message 1 to `2`.
- **Concrete input:** Two Investigation answers where Q1 defines `$\def\Hop{\hat{H}_0}\Hop$` and Q2 uses `$\Hop \ge 0$`, followed by switching sessions (`resetMathMacros()`) and rendering the Answer card.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F8 [major] investigation_answer.js:64-73,102-127: macro defined in earlier answer must resolve in newest answer after session switch
    AssertionError [ERR_ASSERTION]: Newest answer rendered \Hop as undefined macro error because Q1 is collapsed/unrendered: <p>Using <span class="math-glue">...<span class="mord" style="color:var(--status-err);">\Hop</span>...</span></p>
  ```
- **Proposed fix:** In `createAnswerCard.update`, pre-render or replay answers in chronological order `rows[0..n-1]` (storing rendered fragments for `older` items even while `<details>` is closed, and appending their DOM nodes in reverse display order only after chronological evaluation). Call `resetMathMacros()` at the start of `renderSessionStream` (or before full-session re-renders in `app.js:664, 776`) and in `resetToBlankState` (`app.js:425`).

---

### F9 — `major` — Resolving a load-time pending approval leaves `#investigation-status` stuck on `"The model is waiting for your approval."` (`crates/desktop-server/assets/investigation.js:82-86, 165-167, 355` & `investigation_state.js:59-60`)

- **What is wrong:** When an Investigation session loads while waiting on an approval (`data.pending_approvals = [{ call_id: ... }]`), `showInvestigationSession` (`investigation.js:343, 355`) stores `session = data` and sets `approvalWaiting = true`. When the user approves or denies that tool call in Conversation view, `app.js:705` dispatches `research:approvals` with `{ sessionId, waiting: false }` (without re-fetching `session`). The listener in `investigation.js:82-86` sets `approvalWaiting = false` and calls `syncStatus()`, which passes `{ ...session, approvalWaiting: false }` to `paperViewState` (`:165-167`). Because `session.pending_approvals` still holds the stale load-time array, `paperViewState` (`investigation_state.js:59-60`) evaluates `Boolean(sess.approvalWaiting) || sess.pending_approvals.length > 0 === true`, keeping `#investigation-status` frozen on `"The model is waiting for your approval. Review in Conversation"` with `timerActive = false` until the next `investigation.*` event refreshes the session.
- **Concrete sequence:** Open an Investigation session with `running: true, pending_approvals: [{ call_id: 'call-1' }]`, then dispatch `research:approvals` `{ sessionId, waiting: false }`.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F9 [major] investigation.js:82-86,165-167 & investigation_state.js:59-60: resolving a load-time pending approval must clear approval status
    AssertionError [ERR_ASSERTION]: Stale session.pending_approvals kept approval banner stuck after research:approvals { waiting: false }
      actual: 'The model is waiting for your approval. Review in Conversation'
  ```
- **Proposed fix:** In `paperViewState` (`investigation_state.js:59-60`), honour explicit `typeof sess.approvalWaiting === 'boolean'` over `sess.pending_approvals`, and in `investigation.js:84` also clear `if (!approvalWaiting && session) session = { ...session, pending_approvals: [] };`.

---

### F10 — `major` — `investigation_map.js` hand-rolled `mathSpans` omits `\begin{env}…\end{env}` and currency guards, splitting equations across `<mark>` (`crates/desktop-server/assets/investigation_map.js:42-89`)

- **What is wrong:** `investigation_map.js` keeps its own regex/loop `mathSpans` (`:42-66`) instead of reusing `math_scan.js`. It handles only `$…$`, `$$…$$`, `\(…\)`, and `\[…\]`, omitting all 30 `\begin{env}…\end{env}` environments supported by `math.js` (as well as currency `$` guards). When an anchor's `quote` matches inside a bare `\begin{equation}…\end{equation}` or `\begin{align}…\end{align}` excerpt, `foldSearchSpan` does not widen the highlight span to the environment boundaries. `renderAnchorExcerptInto` (`:85-88`) slices the excerpt into three separate `math()` calls: `pre` gets `...\begin{equation}`, `<mark>` gets the bare formula body, and `post` gets `\end{equation}...`. None of the three fragments is valid math, so 0 `.katex` elements render and raw `\begin{equation}` / `\end{equation}` text is exposed to the user.
- **Concrete input:** Excerpt containing `\begin{equation}\nH = \sum_{i=1}^{N} J_i \sigma_i^z\n\end{equation}` with `quote = "H = \\sum_{i=1}^{N} J_i \\sigma_i^z"`.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F10 [major] investigation_map.js:42-89: renderAnchorExcerptInto must widen quote span across \begin{equation}...\end{equation}
    AssertionError [ERR_ASSERTION]: \begin{equation} excerpt failed to render KaTeX when quote matched body: We study the chain Hamiltonian
    \begin{equation}
    H = \sum_{i=1}^{N} J_i \sigma_i^z
    \end{equation}
    with open boundaries.
  ```
- **Proposed fix:** Export a span-finding helper from `math_scan.js` (covering `$…$`, `$$…$$`, `\(…\)`, `\[…\]`, currency guards, and `\begin{env}…\end{env}`) and call it from `foldSearchSpan`, removing the duplicate `isEscaped` / `mathSpans` in `investigation_map.js`.

---

### F11 — `major` — Expanding a section-scoped convention catalog hides both running progress and `conventions_error` (`crates/desktop-server/assets/investigation_conventions.js:42-45, 124, 142-144`)

- **What is wrong:** Design 02 (§3.5) adds section-scoped convention catalogs (`catalog.done === true, coverage_scope === 'section'`) with a `"Catalog all conventions"` button that expands the catalog to the whole paper (`inv.conventions_expanding === true`). Three places in `investigation_conventions.js` still assume `catalog.done === true` means no further work can run or fail:
  1. `signature` (`:124`) omits `inv.conventions_expanding`.
  2. `conventionStatus` (`:42-45`) checks `if (!catalog.done)` for running text; once `catalog.done === true` (from the section catalog), clicking `"Catalog all conventions"` keeps the summary static (`"1 entry · section catalog"`) with no `"reading whole paper…"` indicator while `running && inv.conventions_expanding` is true.
  3. Error banner gating (`:142`): `if (inv.conventions_error && !running && (!catalog?.done || inv.conventions_is_current === false))` evaluates to `false` when `catalog.done === true && catalog.coverage_scope === 'section'`, silently dropping `inv.conventions_error` if the paper-wide expansion stops or fails.
- **Concrete input:** `inv` with `conventions: { done: true, coverage_scope: 'section', items: [...] }`, `conventions_is_current: true`, and `conventions_error: 'The conventions review is still incomplete. Its saved entries are available.'`.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F11 [major] investigation_conventions.js:42-45,142-144: section-catalog paper expansion must surface running state and conventions_error
    AssertionError [ERR_ASSERTION]: conventions_error was silently dropped on section-done catalog expansion failure: "Conventions · 1 entry · section catalogCatalog all conventions"
  ```
- **Proposed fix:** Include `inv.conventions_expanding` in `signature` (`:124`); pass `inv.conventions_expanding` to `conventionStatus` so `running && expanding` appends `' · reading whole paper…'`; and allow `:142` to render `inv.conventions_error` whenever `sectionDone` is true (`!catalog?.done || sectionDone || inv.conventions_is_current === false`).

---

### F12 — `minor` — `paper_cites.js` links read spans over 513 lines into clickable `.line-cite` buttons that fail with HTTP 400 (`crates/desktop-server/assets/paper_cites.js:22, 47` & `src/routes_tools.rs:163`)

- **What is wrong:** `investigation_cites.rs` merges adjacent `read_paper_lines` intervals (`cur.1 + 1 >= start`), so two legitimate 300-line reads (`10..=309` and `310..=609`) merge into `[10, 609]`. If the model cites `[L10–550]` (`541` lines), `investigation_cites.rs` marks `read: true`, and `linkLineCitations` (`paper_cites.js:22, 47`) turns `L10–550` into a clickable `<button class="line-cite">`. Clicking it calls `openCite(10, 550)` → `GET /api/sessions/{id}/sources/{source_id}/excerpt?path=...&start_line=10&end_line=550`, which `routes_tools.rs:163` (`if last < first || last - first > 512`) rejects with HTTP 400 `"Select a passage of up to 513 lines."`
- **Concrete input:** `linkLineCitations(root, [{ start: 10, end: 550, read: true }], openCite)` on `<p>Whole section [L10–550] exceeds the 513-line reader limit.</p>`.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F12 [minor] paper_cites.js:22,47: spans over 512 lines must stay plain text so clicking never triggers routes_tools.rs HTTP 400
    AssertionError [ERR_ASSERTION]: [L10–550] (541 lines > 513 limit in routes_tools.rs:163) became a clickable button that returns HTTP 400
    1 !== 0
  ```
- **Proposed fix:** Either cap clickable citations in `paper_cites.js:22` (`end - start <= 512`) or clamp `openCite` in `investigation.js:47` to `Math.min(end, start + 512)` (showing the opening 513 lines of a wide range in the Canvas source drawer).

---

### F13 — `minor` — `#investigation-scope-chip` renders below `#investigation-answer` due to reversed `.after()` order (plus follow-up question drift on `"Find main results"`) (`crates/desktop-server/assets/investigation.js:20, 38` & `investigation_answer.js:86`)

- **What is wrong:**
  1. In `initInvestigation`, line `20` inserts `#investigation-scope-chip` immediately after `#investigation-status` (`byId('investigation-status').after(chip)`), and line `38` then calls `byId('investigation-status').after(answerHost)` a second time. Because `Element.after()` inserts immediately after `#investigation-status`, `#investigation-answer` is spliced **before** `#investigation-scope-chip`, resulting in DOM order `#investigation-status` → `#investigation-answer` → `#investigation-scope-chip` → `#investigation-summary` (placing the `"Reading: Sec. V.D.4"` scope chip beneath the entire Answer card instead of directly under the status line above the Answer card).
  2. Relatedly (proven by code inspection of `investigation_answer.js:86` vs `src/investigation_view.rs:120` and `tests/investigation_browser.py:406-423`): clicking `"Find main results"` on the Answer card after a follow-up question sends `String(newest?.question || '')` (`Q2`) to `findMainResultsFromAnswer`, whereas `inv.question` in `investigation_view.rs:120` reads `paper.and_then(|p| p.get("question"))` from the first `investigation.paper` event (`Q1`). Note that `investigation_browser.py:413` overrides `state["question"] = text` in its mock server, masking that backend `inv.question` stays pinned to `Q1`.
- **Concrete sequence:** Call `initInvestigation(...)` against the `index.html` DOM where `#investigation-answer` is created dynamically.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F13 [minor] investigation.js:20,38: #investigation-scope-chip must appear above #investigation-answer, not below it
    AssertionError [ERR_ASSERTION]: Expected scope chip above answer card, got DOM order: investigation-status -> investigation-answer -> investigation-scope-chip
  ```
- **Proposed fix:** Insert `answerHost` after `#investigation-scope-chip` (or call `scopeChip` insertion after `answerHost` insertion so `#investigation-scope-chip` sits immediately after `#investigation-status` and above `#investigation-answer`).

---

### F14 — `minor` — `stripComments` strips `<!-- … -->` inside blockquote fenced code, `>= 4`-space indented list fenced code, and multi-backtick code spans (`crates/desktop-server/assets/markdown.js:14-19`)

- **What is wrong:** `stripComments` runs on raw Markdown text before container prefixes are stripped, using `^ {0,3}(\`{3,}|~{3,})` for fences and `` (`+)[^`\n]*?\3 `` for inline code:
  1. Fenced code blocks inside blockquotes (`> ```html\n> <!-- keep -->\n> ``` `) start with `>`, so the fence regex never matches and `<!-- keep -->` is deleted.
  2. Fenced code blocks inside numbered list items with `>= 4` spaces of content indent (`10. Step:\n    ```html`) exceed `{0,3}` leading spaces, so `<!-- keep -->` is deleted.
  3. Double-backtick inline code containing single backticks (`` `` `a` <!-- keep --> `` ``) fails `[^`\n]*?` and falls through to comment deletion (`renderInlineMarkdown:46` uses `[^\n]+?`, disagreeing with `stripComments:16`).
- **Concrete input:** `> ```html\n> <!-- keep in blockquote -->\n> ``` `, `10. Step:\n    ```html\n    <!-- keep in list -->\n    ``` `, and `` `` `a` <!-- keep in inline code --> `` ``.
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F14 [minor] markdown.js:14-19: stripComments must preserve HTML comments inside blockquote/list fences and double-backtick code
    AssertionError [ERR_ASSERTION]: Comment stripped inside blockquote fence: <blockquote><pre><code class="lang-html"></code></pre></blockquote>
  ```
- **Proposed fix:** Strip HTML comments inside `renderInlineMarkdown` / `renderMarkdownBlocks` **after** fenced code blocks and inline code tokens (`\u0000C…\u0000`) have been protected, or align `stripComments`'s inline-code pattern with `(`+)(?!`)[^\n]+?(?<!`)\1(?!`)` and allow optional blockquote/list indentation before fences.

---

### F15 — `minor` — `rewriteNewcommands` misclassifies text-mode KaTeX built-ins (`\ae`, `\oe`, `\ss`, `\o`) and causes a 2nd-pass streaming cache miss (`crates/desktop-server/assets/math_katex.js:75-97, 220-235`)

- **What is wrong:**
  1. `isDefined` (`:80`) probes `katexImpl.renderToString('\\' + name, ...)` in math mode only. Text-mode KaTeX built-ins (`\ae`, `\oe`, `\ss`, `\o`, `\AE`, `\OE`, `\O`, `\aa`, `\AA`) exist in KaTeX's macro/function table—so `\newcommand{\ae}{\alpha}` fails with `\newcommand{\ae} attempting to redefine \ae; use \renewcommand`—yet probing `\ae` in math mode throws `ParseError: KaTeX parse error: Undefined control sequence: \ae` (or `Can't use function '\ae' in math mode` depending on command; specifically `\ae` throws `Undefined control sequence: \ae` in math mode while `\newcommand{\ae}` still rejects redefinition!). Because `isDefined` treats `"Undefined control sequence"` as undefined, it emits `\newcommand{\ae}` and triggers a `.katex-error`.
  2. When a user/model writes `$\newcommand{\foo}{x^2}$` on Pass 1, `prepared` is `\newcommand{\foo}{x^2}`, which defines `\foo`, bumps `state.fp` (`0 → 1`), and caches under `0\0\newcommand{\foo}{x^2}\01` (`:232`). On Pass 2 (streaming re-render), `\foo` is now in `state.macros`, so `rewriteNewcommands` rewrites `prepared` to `\renewcommand{\foo}{x^2}`—missing `0\0\newcommand{\foo}{x^2}\01` and invoking KaTeX a second time before stabilizing on Pass 3.
- **Concrete input:** `$\newcommand{\ae}{\alpha} \ae$` and two consecutive `markdownToRenderedHtml('$\newcommand{\foo}{x^2}$ then $\foo$')` calls.
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F15 [minor] math_katex.js:75-97,220-235: \newcommand on text-mode built-in \ae and streaming cache hit on pass 2
    AssertionError [ERR_ASSERTION]: \newcommand{\ae} failed to rewrite to \renewcommand: <p><span class="katex-error" title="ParseError: KaTeX parse error: \newcommand{\ae} attempting to redefine \ae; use \renewcommand" style="color:var(--status-err)">\newcommand{\ae}{\alpha} \ae</span></p>
  ```
- **Proposed fix:** Probe `isDefined` with `\\newcommand{\\${name}}{}` against `{ throwOnError: true, macros: {} }` (which directly asks KaTeX's own `\newcommand` guard whether `\<name>` is already defined in either math or text mode!), and canonicalize `postKey` on Pass 1 with `rewriteNewcommands` against the post-render `state` so Pass 2 hits the cache immediately.

---

### F16 — `minor` — `isMappingStopped` overwrites `inv.mapping_error` when a later conventions, branch, or claim-check turn is stopped (`crates/desktop-server/assets/investigation_state.js:24-32, 101-105`)

- **What is wrong:** `isMappingStopped` checks whether any `session.abort` with `stopped: true` occurred after the latest `investigation.mapping_requested`, unless a newer `investigation.mapping_requested` or `investigation.question_requested` exists. It does **not** bound the search at the `investigation.mapping_finished` event for `req.seq`, nor does it check `investigation.conventions_resumed` or claim-review/branch turns. If a mapping run fails with a real error (`inv.mapping_error = "Provider rate limit 429 during mapping."`) and the user later starts and stops a conventions-resume or claim-check turn, `ev.seq > req.seq` matches that later `session.abort (stopped: true)` and `paperViewState` (`:101`) replaces `inv.mapping_error` with `"Stopped. Anything already saved is kept below."`
- **Concrete input:** Failed mapping (`req.seq = 10`, `mapping_finished` at `seq = 12` with `status: 'failed'`), followed by `investigation.conventions_resumed` at `seq = 15` and `session.abort { stopped: true }` at `seq = 16`.
- **Proof (`audit_ui_investigation_findings.test.mjs`):**
  ```text
  ✖ F16 [minor] investigation_state.js:24-32: stopping a later conventions/branch/check turn must not mask an earlier mapping_error
    AssertionError [ERR_ASSERTION]: Later conventions stop overwrote mapping_error with: "Stopped. Anything already saved is kept below."
  ```
- **Proposed fix:** Bound `isMappingStopped` to `session.abort` events belonging to the mapping turn itself (i.e. before any `investigation.mapping_finished` for `req.seq`, or check `inv.mapping_status === 'stopped'` / no newer `conventions_resumed` or `session.start`/`session.resume` event after `req.seq`).

---

### F17 — `minor` — Hard line breaks inside `.investigation-inner .rich-prose` render as double blank-line breaks (`crates/desktop-server/assets/investigation.css:14` & `markdown.js:100-101`)

- **What is wrong:** `renderParagraphLines` (`markdown.js:100-101`) emits `<br>\n` for trailing two-space and trailing-backslash hard line breaks. In `#message-stream`, `p` has default `white-space: normal`, so `\n` after `<br>` collapses. Inside `#investigation-panel`, however, `investigation.css:14` sets `.investigation-inner p { margin: 0; overflow-wrap: anywhere; white-space: pre-line; }` with no `.rich-prose p` override. In `white-space: pre-line`, `<br>` forces one line break and the immediately following `\n` forces a **second** line break, creating a spurious blank line inside Answer card paragraphs.
- **Concrete input:** Answer card Markdown `'First line  \nSecond line'`.
- **Proof (`audit_ui_latex_findings.test.mjs`):**
  ```text
  ✖ F17 [minor] investigation.css:14 & markdown.js:100-101: Answer card .rich-prose paragraphs must not double <br>\n line breaks
    AssertionError [ERR_ASSERTION]: markdown.js emits "<br>\n" while investigation.css sets ".investigation-inner p { white-space: pre-line }" with no .rich-prose override
  ```
- **Proposed fix:** Add `.investigation-inner .rich-prose p { white-space: normal; }` in `investigation.css` (or emit `<br>` without a trailing `\n` in `markdown.js:100-101`).

---

### F18 — `nit` — Dead `threw` branch in `math_katex.js` and minor helper duplication across math/map modules (`crates/desktop-server/assets/math_katex.js:222, 237, 253-254`, `markdown.js:11`, `investigation_map.js:36-40`, `math.js:45`)

- **What is wrong (proven by static inspection):**
  1. `crates/desktop-server/assets/math_katex.js:222, 237, 253-254`: `let threw = false;` and `threw = true;` feed into two identical return statements:
     ```js
     if (threw) return glue ? `<span class="math-glue">${rendered}${escapeHtml(glue)}</span>` : rendered;
     return glue ? `<span class="math-glue">${rendered}${escapeHtml(glue)}</span>` : rendered;
     ```
  2. `crates/desktop-server/assets/markdown.js:11` redeclares `const CITATION_BODY_RE = /^[\d,\s\u2013\u2014-]+$/;` identically to `crates/desktop-server/assets/math_scan.js:16` instead of importing it.
  3. `crates/desktop-server/assets/investigation_map.js:36-40` (`isEscaped`) duplicates `isEscapedAt` in `crates/desktop-server/assets/math_scan.js:37-41` (superseded if `F10` shares `math_scan.js`).
  4. `crates/desktop-server/assets/math.js:45`: `renderMathInContainer` checks `!/^(CODE|PRE|SCRIPT|STYLE|TEXTAREA)$/.test(p.tagName)` without `!p.closest('.katex, .katex-error, .math-error')`. If called twice on a container containing a `.katex-error` span (which keeps raw `$…$` / `\begin{…}` text), the second pass re-wraps the error text node. Current callers run once on freshly created DOM, so this is not triggered today.
- **Proposed fix:** Remove `threw` and the duplicate `if (threw)` line in `math_katex.js`, export `CITATION_BODY_RE` from `math_scan.js`, share `isEscapedAt`/span scanning with `investigation_map.js`, and skip `.katex, .katex-error, .math-error` ancestors in `renderMathInContainer`.

---

## Checked and found correct

1. **XSS & `innerHTML` safety across all `crates/desktop-server/assets/*.js` files (`58` `innerHTML` sinks audited):**
   - Every string from a model, paper, filename, tool trace, provider, citation, or ledger payload is escaped via `escapeHtml`, `mathTextToHtml`, `renderMathTextInto`, `renderRichInto`, or assigned via `textContent` / DOM constructors (`createElement` + `textContent`).
   - `katexOptions` (`math_katex.js:64-73`) leaves `trust` unset (`undefined`), blocking `\href`, `\url`, `\includegraphics`, `\htmlClass`, `\htmlStyle`, and `\htmlData`.
   - NUL bytes (`\u0000`) in untrusted inputs are stripped (`s.replace(/\u0000/g, '')` in `markdown.js:338`, `tex_preview.js:194`, `math.js:19`) before internal `\u0000M…\u0000` / `\u0000C…\u0000` placeholder substitution, preventing placeholder-spoofing injection.
   - Prose `\eqref{id}` renders `<a class="eqref-link" data-eq-ref="…">` with **no `href` attribute** and `escapeHtml` on both attribute and label; formula `\eqref{id}` validates `SAFE_EQ_ID_RE` and escapes unsafe characters via `escapeTexText`.
   - Markdown links (`markdown.js:51-54`) restrict schemes strictly to `https?://[^\s)<>"'\u0000]+` with `escapeHtml(url)` and `rel="noopener noreferrer"`.
   - New UX & split surfaces (`investigation_answer.js`, `investigation_scope.js`, `paper_cites.js`, `trace_cards.js` `delegate_read_paper`, `subagents.js` `Paper reader` cards) construct DOM via `document.createElement` + `textContent` / `renderRichInto` / `renderMathTextInto`; zero raw `innerHTML` interpolation of model or paper data exists.
2. **Design 03 (`03-latex-kandinsky-parity.md`) WP1–WP5 parity matrix:**
   - Verified all 65 cases in `math_parity.test.mjs` plus 6 multi-assertion tests added in `audit_ui_design03_extra.test.mjs` covering: all 9 seeded Kandinsky macros (`\R, \C, \N, \Z, \ket, \bra, \braket, \Tr, \mbox`), inline `$…$` across soft single newlines, even/odd backslash-run parity before `$` and inside `$…$`, up-to-4-char punctuation glue, Markdown `[label $x$](url)` link boundary protection, absence of `.math-glue` in `markdownToHtml`, `\[…\]` with `\label`, self-tagged `\tag{A.1}` without `\label` as `\eqref{A.1}` target, numbering recovery after unclosed `$$` stopped by ATX heading, `multline`/`gather`/`alignat`/`flalign` numbering, mid-sentence numbered environments, `\@` internal macro cleanup, 500-entry FIFO cache eviction, and TeX preview `\eqref` navigation markup.
   - `window.renderMathInElement` is completely removed from application JS; `auto-render.min.js` remains served in `assets.rs` / `index.html` per Decision 8 (deferred to WP6).
3. **Answer card, scope chip, tabs, Esc, status, and mode/session switching:**
   - **Tabs (`index.html:226-227`, `investigation.js:113-125`):** Labeled `Paper` and `Conversation`, toggling `aria-pressed` (`"true"` / `"false"`), `.is-active`, `data-investigation-view`, and `#investigation-panel.hidden`.
   - **Setup button (`investigation_state.js:10-12`, `investigation.js:99-102, 468-469`):** Dynamically switches between `"Find main results"` (blank question, `scope = 'map'`) and `"Answer my question"` (non-empty question, `scope = 'question'`).
   - **Composer visibility (`investigation.css:2, 7`, `investigation.js:350, 359-360`):** Hidden on setup (`data-investigation-view="map"` with `data-investigation-has-paper="false"`), visible with placeholder `"Ask a follow-up about this paper…"` once a paper is attached (`data-investigation-has-paper="true"`), and reset to default placeholder when leaving Investigation mode.
   - **Scope chip (`investigation_scope.js`):** Correctly renders section scope (`Reading: … · Whole paper instead` when `allow_whole_paper === true`), whole-paper fallback note, `"Reading: whole paper"` after switching, and hides when `scope` is null/empty.
   - **Answer card (`investigation_answer.js`, `paper_cites.js`):** Renders `"Written by the model. Check the cited lines."`, links only `read: true` citations from `row.cites`, leaves unread `[L900–910]` and inline-code/KaTeX spans untouched as plain text, disables `"Find main results"` while `running`, hides `"Find main results"` when `hasMap` is true, and resets `earlierOpen` / `pendingMap` / `signature` on session switch.
   - **Esc key (`composer.js:96-113`, `workbench.js:209-216, 558-574`):** Checked in order: `defaultPrevented`, `isComposing || keyCode === 229`, modal `<dialog open>` (`pdf-preview-modal`, `settings-modal`), open `#effort-popover`, open `#conventions-popover`, narrow-viewport drawers (`rail` `<= 780px`, `canvas` `<= 1120px`), and non-empty focused `<input type="search">` (blurring without stopping the active turn), before calling `onStop()`.
   - **Event listeners:** Verified `initInvestigation`, `initComposer`, `initSubagents`, `initWorkbench`, `installEqrefNavigation` (`Symbol('reSearchEqrefNav')` guard), and `renderStatus` (`dataset.approval` guard): no listener is attached twice across re-renders or session switches.
   - **Session & mode switch cleanup:** `selectSession` (`app.js:542-557`) resets math macros, calls `showInvestigationSession(id, null)` (which saves per-session `setupDrafts`, clears `summaryTimer` and `statusTimer`, resets `paperMap`, `conventionCatalog`, `answerCard`, `scopeChip`, and hides `#investigation-origin`), closes popovers, and resets live turn state. Switching to a Research session (`data.mode !== 'investigation'`, `investigation.js:346-353`) resets `answerCard` and `scopeChip`, clears timers, sets `data-investigation-has-paper="false"`, and restores the composer placeholder.
4. **Architecture, attribution, line-count, asset, and test-preservation rules:**
   - Kandinsky attribution comment present at the top of `math_katex.js`, `math_scan.js`, `math_equations.js`, `markdown.js`, and above the math block in `components.css`.
   - Every new/modified JS/CSS module in `crates/desktop-server/assets/` is `<= 600` lines (`investigation_map.js` is `599` lines; `investigation.js` is `543` lines; `markdown.js` is `384` lines; `math_scan.js` is `325` lines; `math_equations.js` is `271` lines; `math_katex.js` is `256` lines; `tex_preview.js` is `256` lines; `app.js` is exempted by Decision 6 and grew by only `+5` net lines across all briefs; `index.html` grew by `0` net lines).
   - Every new asset file is registered in `crates/desktop-server/src/assets.rs`.
   - Diffing `origin/main...351b1287c9a54a700b0e33111b3891f195afd6d1` across `crates/*/tests/**` confirmed zero existing tests or assertions were deleted, skipped (`#[ignore]`), or weakened.

---

## Decisions taken unattended

1. **Test isolation so `cargo test` stays green while proving findings:** `crates/desktop-server/tests/ui_math.rs` invokes only `math_render.test.mjs` and `math_parity.test.mjs` explicitly by filename. Adding `audit_ui_design03_extra.test.mjs` (passing parity coverage) and `audit_ui_latex_findings.test.mjs` + `audit_ui_investigation_findings.test.mjs` (failing proof tests) under `crates/desktop-server/tests/` satisfies the `audit_ui_*.test.mjs` naming rule in `audit.md`, keeps `cargo test --workspace` green, and reproduces all 17 proven UI bugs directly via `node --test`.
2. **Real asset execution in Investigation proof tests:** Because browser modules in `crates/desktop-server/assets/` use root-anchored `/assets/*.js` ES module specifiers, `audit_ui_investigation_findings.test.mjs` rewrites `/assets/math.js`, `/assets/paper_cites.js`, `/assets/investigation_state.js`, `/assets/investigation_scope.js`, and `/assets/investigation_graph.js` to their real local `file://` URLs and loads the unmodified asset source via `data:text/javascript;base64,…` against a minimal DOM shim and real vendored KaTeX—executing the exact production functions without touching any production file.
3. **Severity classification:** Rated `F1`–`F4` as `critical` because each silently produces **mathematically wrong or inverted output** with no error indicator (`F1` renders prose as math and drops equations; `F2` turns `x^2` into circumflex `xˆ2`; `F3` binds `+` into `\norm` producing `∥+∥_x y`; `F4` strips leading `>` and `-` signs from inequalities/differences after equations). Rated broken features (`F5`–`F11`) as `major`, edge-case/layout mismatches (`F12`–`F17`) as `minor`, and dead/duplicated code (`F18`) as `nit`.

---

## Test commands run and exact counts

| Command | Result |
|---|---|
| `for f in crates/desktop-server/assets/*.js crates/desktop-server/tests/*.mjs; do node --check "$f"; done` | **PASS** (`0` syntax errors across all asset and test modules) |
| `node --test crates/desktop-server/tests/{investigation_map,investigation_ux,math_parity,math_render}.test.mjs` (existing baseline suite) | **PASS:** `109 passed, 0 failed, 0 skipped` |
| `node --test crates/desktop-server/tests/audit_ui_design03_extra.test.mjs` (new Design 03 parity tests) | **PASS:** `6 passed, 0 failed, 0 skipped` |
| `node --test crates/desktop-server/tests/audit_ui_latex_findings.test.mjs` (new failing proof tests `F1`–`F7`, `F14`, `F15`, `F17`) | **FAIL (expected proof):** `0 passed, 10 failed` |
| `node --test crates/desktop-server/tests/audit_ui_investigation_findings.test.mjs` (new failing proof tests `F8`–`F13`, `F16`) | **FAIL (expected proof):** `0 passed, 7 failed` |
| `node --test crates/desktop-server/tests/*.test.mjs` (all 7 `.test.mjs` files combined) | `132 total: 115 passed, 17 failed` (the 17 intentional proof tests) |
| `cargo test -p desktop-server --test ui_math` | **PASS:** `2 passed, 0 failed` |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` + `cargo test -p cli --test desktop_and_ext cli_research_workspace_read_fork_diff_export_and_verify` | **PASS:** `545 passed, 0 failed, 0 ignored` across the entire Linux workspace |
| `cargo clippy --workspace --all-targets -- -D warnings` | **Pre-existing failure** in untouched `crates/desktop-server/src/evidence.rs:737` (`clippy::manual_unwrap_or` under `rustc 1.100.0`; `git diff origin/main..351b1287 -- crates/desktop-server/src/evidence.rs` is `0` lines, last touched on `main` in `743cc5b`) |

### Pre-existing Linux notes (verified on unmodified base `351b1287c9a54a700b0e33111b3891f195afd6d1`)

1. `crates/desktop-server/src/evidence.rs:737` triggers `clippy::manual_unwrap_or` under `rustc 1.100.0-nightly` with `-D warnings` (untouched since `743cc5b` on `main`).
2. `cli_research_workspace_read_fork_diff_export_and_verify` (`crates/cli/tests/desktop_and_ext.rs`) must run without `RE_SEARCH_PYTHON` set on Linux because `re-search run --route api` rejects non-Windows Python configuration before consuming all `ScriptedMock` connections; running it with `RE_SEARCH_PYTHON` unset passes (`1 passed, 0 failed`).

---

## Not compiled or run on Linux

- **`cfg(windows)` code paths:** `crates/desktop` (WebView2 shell), Windows Job Object / AppContainer / restricted-token sandbox code in `crates/tools`, and Windows DPAPI credential storage (`cfg(windows)` is not compiled on Linux).
- **Playwright / browser automation:** `crates/desktop-server/tests/investigation_browser.py` (`python tests/run_playwright.py`) is out of scope on Linux per `RULES.md` and requires a live browser harness on Windows (inspected statically).
- **Visual WebView2 checks (Design 03 §4):** Narrow-column tag wrapping (`:has(> .katex-tag)`), long matrix horizontal scrollbars, dark/light theme red contrast on `.katex-error`, and live 30-equation streaming frame-rate checks require visual inspection in WebView2 on Windows.
