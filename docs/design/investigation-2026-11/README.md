# Investigation mode and LaTeX: designs and decisions (2026-11)

This folder is the single authority for the work on branch `gemini/runner`. It was written by a
local Claude session that coordinates Gemini agents; every agent brief in `agents/` points here.

| File | What it is |
|---|---|
| `01-reader-recorder-split.md` | Pointer citations, partial acceptance, `read_paper_lines`, reader subagents, outline/scope |
| `02-investigation-ux.md` | Question-first Investigation turns, own system prompt, answer card, status, Esc, title |
| `03-latex-kandinsky-parity.md` | Make re:Search render math like Kandinsky's reading view |
| `maps/` | The code and data maps the designs were built from (background, may be stale) |
| `agents/` | Rules and one brief per Gemini agent |

The designs were made read-only against `7c36a0e` (= `main`). Line numbers in them refer to that
commit and drift as packages land; re-locate by symbol, not by line.

## Decisions (made by the user on 2026-11-25)

The user accepted every recommended default in the three designs, with one change: **code may be
ported from Kandinsky** (see below). In short:

**Reader/recorder split (01, section 5)**
1. Correct citations automatically (relocate within ±12 lines, narrow to quote ±2) and show "lines corrected by the app": yes.
2. Save valid conventions and mark the catalog unfinished when some citations fail: yes.
3. Keep the legacy verbatim-`excerpt` anchor form for one release; stop asking for it in the instructions.
4. Reader model when none is assigned: the primary provider; another provider only if assigned in settings.
5. Reader parallelism: 2 on ChatGPT-plan routes, 4 on API keys, configurable.
6. Reader reasoning effort: `low`.
7. Readers on by default only when a smaller reader model is set or the question is section-scoped; otherwise an opt-in setting.
8. Question-scoped first pass with the full conventions catalog on click: yes, shipped before readers are on by default.
9. Licence: the repository is `GPL-3.0-only` (`Cargo.toml`, `LICENSE`). The "MIT OR Apache-2.0" in older notes is out of date.

**Investigation UX (02, section 5)**
1. Conventions catalog off in the first pass; the existing button builds it.
2. A typed question gets an answer only, with "Find main results" offered on the answer card.
3. New Investigation sessions use their own system prompt (no proof-graph duty, `[L…]` line citations).
4. Tab names "Paper / Conversation".
5. Keep `DEFAULT_MAX_TURNS` for question turns until measured.
6. Clickable citations (phase 3): only for lines the model actually received.
7. Leave the word "branch" unchanged for now.

**LaTeX (03, section 5)**
1. **Changed by the user: porting Kandinsky code is allowed.** Kandinsky (`https://github.com/antimajorana/kandinsky`,
   commit `833a95f`, branch `fix/graph-zoom`) is the user's own project. Agents may port its math/Markdown rendering code
   (mainly `ui/src/lib/math.ts`, `math-align.ts`, `markdown.ts`, `equations.ts`, `preview.ts` and `ui/src/styles/markdown.css`)
   into re:Search's plain ES modules. Put a one-line header in each file that contains ported code:
   `// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 833a95f), the same author's project.`
   Do not copy third-party code that lives inside Kandinsky (vendored libraries, `node_modules`, anything under
   `plugins/obsidian/`). The deviations listed below still win over Kandinsky's behaviour.
2. `\(…\)` stays math.
3. A failed mid-line `\[` / `\(` opener drops the backslash (`\[1\]` shows `[1]`).
4. HTML comments outside code are stripped.
5. Equation numbering follows the design's single rule, including per-row numbers (deviation from Kandinsky).
6. Macros reset on session switch, with the small `app.js` addition (exception to the 600-line rule for `app.js` only).
7. Bidi isolation on math, scoped to `.rich-prose`.
8. Remove `auto-render.min.js` later (WP6), together with `index.html`, `assets.rs` and the browser test.
9. Optional extras (align shorthand, hover card, callouts, copy-as-TeX): deferred.
10. Standalone HTML export parity: out of scope.

## Order of work

| Wave | Agent brief | Packages | Runs alongside |
|---|---|---|---|
| 1 | `agents/latex.md` | LaTeX WP1–WP5 | `ux-rust` |
| 1 | `agents/ux-rust.md` | UX P0, P1a, P1b, P2, P3 | `latex` |
| 2 | `agents/ux-ui.md` | UX P4, P5 | `split-1` |
| 2 | `agents/split-1.md` | Split WP1a, WP1b, WP2, WP3, WP4, WP5 | `ux-ui` |
| 3 | `agents/split-2.md` | Split WP6–WP9 (readers) | `split-3` |
| 3 | `agents/split-3.md` | Split WP10–WP12 (outline, scope) | `split-2` |
| 4 | `agents/audit.md` | Read-only bug and consistency audit | several lenses in parallel |

A wave starts from the tip of `gemini/runner` after the previous wave's patches were reviewed,
gated on Windows and pushed. Briefs for waves 2–4 are added when their wave starts.
