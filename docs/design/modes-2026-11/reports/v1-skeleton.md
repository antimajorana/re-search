# Report: `v1-skeleton` (Investigate package V1 — Skeleton core for TeX)

- **Base commit:** `49edc18dd66b85044046dacac186af3593a44de3` (`origin/claude/project-thread-caszr2`)
- **Branch:** `work/v1-skeleton` -> pushed to `gemini/modes/v1-skeleton`
- **Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)` on Linux x86_64
- **Dependencies / lockfile:** Zero new dependencies; `Cargo.lock` untouched; `crates/research-tools/tests/golden/tool_defs.txt` untouched.

---

## 1. What was implemented (Package V1)

All new library code lives under `crates/research-tools/src/skeleton/`, exposed by one line (`pub mod skeleton;`) in `crates/research-tools/src/lib.rs`. Every source file is at or under 600 lines (verified by `scripts/check_file_length.py --base origin/claude/project-thread-caszr2`).

### Library files (`crates/research-tools/src/skeleton/`)

| File | Lines | Responsibility |
|---|---:|---|
| `model.rs` | 403 | Section 4.1 serde data model (`PaperSkeleton`, `SkNode`, `SkNodeKind`, `SkConfidence`, `NumberBasis`, `SkeletonBuilder`, `SkeletonQuality`, `Span`, `SkEdge`, `SkEdgeKind`, `SkEdgeRole`, `SkEdgeRule`, `EdgeConfidence`, `BibRef`, `BibLocator`, `SymbolDef`, `SymbolDefHow`, `MacroDef`, `SectionStats`, `Carry`, `CarryMatch`, `CarryBasis`), limits (`6_000` nodes, `20_000` edges, `2 MiB` JSON), `compute_files_root_sha256`, and `compute_body_fp` (12 lower-case hex chars of SHA-256 over comment-stripped, whitespace-normalised body). `paper_key` is `paper_key::PaperKey`. Tags (`tag`, `parent`, `proof_of`) are left `None` in V1; `paper_key::node_tag` is never called. |
| `flatten.rs` | 564 | Verified `source.ingested` loader (`load_ingested_tex_source`) checking `files_sha256` digests under `SandboxRoot::Sources` and skipping synthetic `assembled_path`; primary `.tex` selection (`select_primary_tex`); in-memory `\input`/`\include` flattener (`flatten_tex_files`) with exact `(path, 1-based line)` origin map (`FlattenedTex`, `FlattenedLine`, `OriginEntry`), `spans_for_range` with guaranteed child-inside-parent span containment, cycle detection, depth limit (`MAX_INCLUDE_DEPTH = 8`), output byte/line caps, and dormant masking so `\input` inside `\verb`, verbatim-like environments, or `\iffalse...\fi` is not expanded. |
| `tex_lex.rs` | 380 | Shared comment- and escape-safe lexical helpers: `skip_ws_and_comments`, bounded `read_balanced_group` (respecting nested `{}` inside `[]`, `%` comments, paragraph-break guard on non-`\long` headings), `clean_tex_title`, `skip_inline_verb`, `extract_unique_cites`, `classify_proof_hint` (`ProofHint`), `has_active_begin_document`, and `try_parse_macro_cmd` (`\newcommand`, `\renewcommand`, `\providecommand`, `\DeclareMathOperator`). |
| `tex_theorems.rs` | 534 | `TheoremDef`, `TheoremLookup`, `TheoremTable`, `map_theorem_kind`, and parsers for `\theoremstyle`, `\newtheorem[*]`, `\spnewtheorem[*]`, `\newmdtheoremenv`, `\declaretheorem[*]` (with `name`/`title`, `sibling`/`sharenumber`/`usecounter`, `numberwithin`/`parent`, `numbered=no`, `style`, comma-separated names), and simple `\newenvironment`/`\renewenvironment` wrappers around theorem or proof environments. |
| `tex_math.rs` | 251 | Single-equation (`equation[*]`, `multline[*]`, `displaymath`) and multi-row (`align[*]`, `gather[*]`, `eqnarray[*]`, `alignat[*]`, `flalign[*]`, `IEEEeqnarray[*]`) math body inspector (`inspect_math_body`), top-level `\\` row splitter respecting nested math environments and braces, `\label` and `\tag[*]` extraction, and comment-safe `\[...\]` / `$$...$$` matchers. |
| `tex_env.rs` | 530 | Core comment-safe structural `Scanner` with section stack and environment stack (`EnvFrame::{Document, Node, Math, Wrapper}`), verbatim environment skipping, nested `\iffalse ... \else ... \fi` skipping (preserving math `\iff`), preamble/body separation, and `handle_begin`. |
| `tex_env_cmds.rs` | 501 | `Scanner` command handlers (`open_proof_env`, `read_optional_heading`, `handle_end`, `handle_section`, `handle_label`, `handle_caption`, `handle_bib_cmd`, `handle_display_brackets`, `handle_display_dollars`, `emit_math_flush`) and stack-closing / error-recovery routines (`close_non_container_envs`, `close_sections_at_level`, `close_sections_above_env_depth`, `close_unclosed_frame`, `close_all_open`, `finish_all`). |
| `mod.rs` | 210 | Public re-exports, `TexSkeletonInput`, `TexSkeletonOutput`, `BuildTiming`, `build_tex_skeleton`, `build_tex_skeleton_timed` (enforcing the 2 MiB JSON cap), and `build_tex_skeleton_from_ingested`. |

### Tests and fixtures (`crates/research-tools/tests/`)

| File | Lines | Purpose |
|---|---:|---|
| `tests/skeleton_tex.rs` | 506 | Runs all 8 self-written fixtures against their hand-written `expected.json`, checks byte-identical determinism, serde round-trip, CRLF vs LF invariance, comment-only fingerprint stability across versions, verified `source.ingested` `.zip` loading + traversal refusal + tamper failure, malformed TeX recovery, include depth limit, and the 5,000-line timing test. |
| `tests/skeleton_props.rs` | 285 | Deterministic property-style tests (200 random seeds with xorshift64, zero new dependencies) over random interleavings of `\begin`, `\end`, `%`, braces, brackets, verbatim, `\iffalse`, sections, and multi-file `\input` checking termination, panic freedom, span nesting (`child.span ⊆ parent.span`), and origin-map round-trip; plus same-line multi-file nesting, single-line dormant include masking, and `MAX_SKELETON_NODES` (6,000) truncation. |
| `tests/fixtures/skeleton/*` | 24 files | 8 self-written math/physics TeX fixtures (`01_amsthm_shared_counters`, `02_custom_theorem_names`, `03_multi_file`, `04_revtex_physics`, `05_detached_proof`, `06_unlabelled_theorems`, `07_two_versions/{v1,v2}`, `08_hard_edge_cases`) and their hand-written `expected.json` files. |

---

## 2. Reuse of existing `research-tools` modules

- **`tex_scan.rs` — reused directly:**
  - `strip_latex_comments_line` and `strip_latex_comments` (handles `\%`, `\\%`, `\\\%`, and `\verb|...|`) in `model.rs`, `flatten.rs`, `tex_lex.rs`, and `tex_math.rs`.
  - `extract_tex_labels` in `tex_math.rs`.
  - `extract_tex_citations` and `extract_tex_refs` in `tex_lex.rs` for `head_cites` and `ProofHint::HeaderRef`.
- **`tex_bundle.rs` — partially reused:**
  - Reused `score_tex_entrypoint`, `find_tex_includes_in_line_block`, and `normalize_rel_segments`.
  - Could **not** reuse `assemble_tex_bundle` because it strips comments, drops comment-only lines, inserts synthetic `% --- begin inlined: ... ---` banner lines (destroying physical line correspondence), expands `\input` inside verbatim/`\iffalse`, swallows cycles/depth limits into comments instead of structured quality notes, and does not return a line-origin map.
  - Could **not** reuse `resolve_tex_include` directly because it is private (`fn`) in `tex_bundle.rs` and does not check primary-directory-relative resolution first (standard LaTeX `\input` semantics); implemented `flatten::resolve_include` on top of `normalize_rel_segments` without editing `tex_bundle.rs`.
- **`ingest.rs` and `investigation_anchor.rs` — protocol reused, files untouched:**
  - `load_ingested_tex_source` reads the exact `source.ingested` ledger payload schema (`source_id`, `primary_path`, `assembled_path`, `files`, `files_sha256`) produced by `ingest.rs`, uses `WorkspaceSandbox::resolve_existing(SandboxRoot::Sources, ...)` and `ledger::sha256_hex`, skips `assembled_path` when distinct from `primary_path`, and fails closed on hash mismatch, matching `SourceIndex::load` in `investigation_anchor.rs`.
- **`investigation_outline_tex.rs` — could not reuse directly:**
  - Line-by-line regex scanner targeting legacy `OutlineSection`/`OutlineItem` with no environment stack, no multi-line headings, no `\newtheorem`/`\declaretheorem` table, no verbatim/`\iffalse` skipping, and no physical-file origin map. Its multi-row math-environment list (`align`, `gather`, `eqnarray`, `alignat`, `flalign`) and top-level `\\` / `\tag` rules informed `tex_math.rs`.

---

## 3. How the 8 fixtures and expected JSON files were created

Every `.tex` fixture was written from scratch for this brief with short, self-authored mathematical and physical prose (no copied or paraphrased third-party paper text). Every `expected.json` was authored **by hand from reading the `.tex` files line-by-line before running the scanner** (not by capturing scanner output):

1. **`01_amsthm_shared_counters`**: discrete harmonic functions on finite weighted graphs (`\newtheorem{theorem}{Theorem}[section]`, shared `[theorem]` counters for lemma/proposition/corollary/definition, starred `\newtheorem*{remark}`, `\newcommand`/`\DeclareMathOperator`, heading citation, labelled + unlabelled equations). Expected nodes, spans, `theorem_defs`, `macros`, and `proof_hint` were counted directly from lines 1–55 of `main.tex`.
2. **`02_custom_theorem_names`**: weighted sequence space contractions using `thmtools` `\declaretheorem` (`name=Main Result`, `sibling=mainres`, `numbered=no`, `style=remark` fallback) plus `\newenvironment` wrappers (`boxedres`, `myproof`). Expected JSON counted from lines 1–34 of `main.tex`.
3. **`03_multi_file`**: 1D tight-binding chain split across `main.tex`, `preamble.tex`, `sections/setup.tex`, `sections/sub_lemma.tex` (nested `\input` inside `setup.tex`), and `sections/results.tex` (`\include`), plus `figure` and `thebibliography`. Multi-file section spans (`setup.tex:1..8`, `sub_lemma.tex:1..6`, `setup.tex:10..10`) were derived by tracing the include expansion by hand.
4. **`04_revtex_physics`**: RevTeX (`revtex4-2`) coupled quartic oscillator physics note with zero theorem environments, `abstract`, `section`/`subsection`, `equation`, multi-row `align` (2 labelled rows + 1 `\nonumber` unlabelled row counted in `eq_count`), `table`, `figure`, and `\bibliography`. Expected JSON counted from lines 1–38 of `main.tex`.
5. **`05_detached_proof`**: positive stochastic matrices with statements in Section 1, three detached proofs in Section 2 (`Proof of Lemma~\ref{lem:pos}`, `Proof of Theorem 1.2`, `Proof of Corollary~\ref{cor:mix} \cite{seneta2006}`), and one attached `\begin{proof}` in Section 3. Expected JSON and `proof_hint` variants written by hand from lines 1–35.
6. **`06_unlabelled_theorems`**: convex cones and bipolar identity with unlabelled definition, lemma, theorem, proof, two identical-body remarks (asserting equal `fp`), one labelled equation, and unlabelled `\[...\]` / `$$...$$` display math. Expected JSON counted from lines 1–36.
7. **`07_two_versions/{v1,v2}`**: discrete heat kernel on $\mathbb{Z}_N$ in two versions where `v2` adds a comment-only line to `thm:decay` (same `fp`), inserts `lem:fourier` shifting `thm:sup` (same `fp`), edits `prop:gap` (different `fp`), removes `remark`, and adds `cor:uniform`. Both `v1/expected.json` and `v2/expected.json` were written by hand from lines 1–22 (`v1`) and 1–26 (`v2`).
8. **`08_hard_edge_cases`**: symplectic integrator note combining multi-line `\section` with commented brace `% ignore this brace }` and nested math `$O(h^{2p})$`, inline `\verb`, `\begin{verbatim}` and `\begin{lstlisting}` containing fake sections/theorems, nested `\iffalse...\iftrue...\fi` with math `$A \iff B$`, `\iffalse...\else...\fi` active branch, nested brackets `\cite[Theorem 3.2]{hairer2006}` inside a theorem heading, `95\%` + comment, `\tag{S1}` printed equation number, and a two-file `\input` cycle (`cycle_a.tex <-> cycle_b.tex`). Expected JSON written by hand from all three `.tex` files.

All 8 fixtures matched their hand-written `expected.json` on the first test execution.

---

## 4. Honest assessment: where the scanner fails or has known limits

Per `00-overview.md` section 6 ("What can move") and the brief, here are the concrete cases where this own scanner (`tex-1`) is known to miss or misclassify structure:

1. **Macros wrapping `\begin`/`\end` or `\section`:** `\newcommand{\bt}{\begin{theorem}}` / `\bt ... \et`, `\def\thm#1{...}`, and custom section macros (`\mysec{...}`) are **not** expanded (by design, `macros` are never expanded into text). Only `\newenvironment{foo}{\begin{bar}}{\end{bar}}` wrappers around a single known theorem/proof environment are recognized.
2. **Dynamic theorem declarations inside `.sty`/`.cls` files absent from the arXiv bundle:** If a paper uses a custom theorem environment name defined only in an external journal class (e.g., `\begin{myfact}` from a publisher `.cls` not shipped in the e-print), `TheoremTable::lookup` falls back only to standard built-in names (`theorem`, `thm`, `lemma`, `lem`, `proposition`, `prop`, `corollary`, `cor`, `conjecture`, `conj`, `definition`, `defn`, `def`, `assumption`, `assump`, `hypothesis`, `hyp`, `axiom`, `claim`, `observation`, `example`, `exmp`, `remark`, `rem`, `rmk`, `algorithm`, `alg`). Unlisted custom names are treated as plain wrapper environments.
3. **TeX conditionals other than `\iffalse`:** `\iftrue` is scanned normally, and nested `\if...` inside `\iffalse` are skipped, but `\ifdraft ... \else ... \fi` or `\newif\iflong \longfalse \iflong ... \fi` are not evaluated; both branches of arbitrary user `\if...` conditionals are scanned.
4. **Plain TeX / non-environment theorem headings:** Plain TeX `\proclaim Theorem 1. ... \endproclaim` or bold-paragraph theorems (`\noindent\textbf{Theorem 1.} ...`) are not detected by `tex-1`.
5. **`\verb` inside `\caption` or `\section` arguments:** `read_balanced_group` is comment- and backslash-escape-safe, not `\verb`-delimiter-safe inside `{...}` arguments (LaTeX itself forbids `\verb` inside moving arguments like `\section`/`\caption`, though `\cprotect` allows it).
6. **Multiple `\label` commands on a single node or single `align` row:** Only the first `\label{...}` directly inside a node (or row) is stored in `SkNode::label`, because `SkNode::label` is `Option<String>`.
7. **Unbalanced environments inside `\newcommand` bodies:** If a `\newcommand` body in the preamble contains unbalanced `{` or spans > 4,096 bytes (`MAX_GROUP_SCAN_BYTES`), `read_balanced_group` marks it unclosed and resumes on the next line.
8. **Nested `subequations` with a parent label:** `subequations` is treated as a transparent wrapper so inner `align`/`equation` rows become individual `Equation` nodes; a standalone `\label` placed directly on `\begin{subequations}\label{eq:group}` attaches to the enclosing node (if unlabelled) rather than creating a separate parent equation node.
9. **`\paragraph` / `\subparagraph`:** Intentionally not emitted as `Section` nodes (matching `investigation_outline_tex.rs`), so physics papers using `\paragraph{...}` as their only sectioning level (e.g., PRL letters with no `\section`) yield only front matter, labelled equations, floats, and bibliography unless an orient/fallback pass handles them.

None of these limits broke any of the 8 V1 fixtures; whether any of them reduces recall below 0.9 on real arXiv papers must be measured on Evaluation Set P before M1.

---

## 5. Deviations, decisions taken alone, and assumptions

### Decisions & deviations from `investigate.md` 4.1

1. **Optional fields in V1 JSON:**
   - `SkNode::tag`, `SkNode::parent`, `SkNode::proof_of`, and `SkNode::number_basis` are `Option<...>` (`skip_serializing_if = "Option::is_none"`), left `None` in V1 (except `number` + `number_basis: Some(Printed)` when an equation carries an explicit `\tag{...}`). This satisfies the brief ("Leave tags unset or absent as the design allows for V1; do not call `node_tag`") while keeping the exact section 4.1 field names ready for V2.
   - `PaperSkeleton::quality_reason`, `SkNode::env_name`, and `SkNode::title` are `Option<String>` (omitted when absent).
2. **Named structs for `MacroDef` and `CarryMatch`:** Section 4.1 sketches `macros: Vec<(name, arity, body)>` and `Carry.matched: [(tag, basis, changed)]`. Implemented as named structs `MacroDef { name, arity, body }` and `CarryMatch { tag, basis, changed }` for field-order clarity and consistency with `BibLocator` and `SectionStats`.
3. **`SectionStats` representation:** Section 4.1 sketches `stats: per section { words, read_minutes, counts by kind }`. Implemented as `Vec<SectionStats>` where `SectionStats { section_tag: Option<String>, words: u32, read_minutes: f64, counts: BTreeMap<SkNodeKind, u32> }` (empty in V1; populated in V2 once tags and word counts exist). `read_minutes` is `f64` (0.0 in V1) so fractional reading minutes in V2 deserialize cleanly.
4. **V1-to-V2 sidecar metadata in `TexSkeletonOutput`:** Kept `PaperSkeleton` and `SkNode` strictly faithful to section 4.1 (no extra fields on the serialized skeleton), and returned `parent_orders: Vec<Option<usize>>`, `proof_hints: BTreeMap<usize, ProofHint>`, `theorem_table: TheoremTable`, `flattened: FlattenedTex`, and `quality_notes: Vec<String>` alongside `skeleton` in `TexSkeletonOutput` for V2 and tests.
5. **Module splitting for the 600-line rule:** Split the scanner cleanly across `tex_lex.rs`, `tex_math.rs`, and `tex_env_cmds.rs` alongside the brief's `mod.rs`, `model.rs`, `flatten.rs`, `tex_env.rs`, and `tex_theorems.rs` so every file stays well under 600 lines.
6. **Constants / limits chosen:** `MAX_INCLUDE_DEPTH = 8`, `MAX_FLATTENED_LINES = 250_000`, `MAX_FLATTENED_BYTES = 16 MiB`, `MAX_MACROS = 500`, `MAX_MACRO_BODY_CHARS = 512`, `MAX_GROUP_SCAN_BYTES = 4_096`.
7. **Section end line at `\end{document}`:** Open sections close at the flattened line immediately preceding `\end{document}` (clamped to `>= start_flat` and `>= max_child_end`), matching how a section closes at the line before the next `\section`.
8. **Unresolved non-TeX `\input` targets:** Unresolved `\input` targets with a non-TeX extension (e.g., `.pdf_tex`, `.tikz`, `.eps`) do not emit a quality warning, whereas targets with no extension or `.tex`/`.ltx` do.

### Assumptions

- Physical line numbering matches `VerifiedFile` in `investigation_anchor.rs` (1-based inclusive, `\r\n` normalised to `\n`).
- `files_root_sha256` is computed as SHA-256 over `"re-search.files-root.v1\0"` followed by length-prefixed sorted `(path, digest)` pairs from `files_sha256`. V2/V3 should reuse `skeleton::compute_files_root_sha256`.

---

## 6. Merge notes

### Edits to existing files
- **`crates/research-tools/src/lib.rs`** (1 line added, isolated in its own commit `18deac0`): added `pub mod skeleton;`. No other existing file was modified.

### Interfaces for downstream packages (V2, V3, V5, V10)
- **V2 (`skeleton/{edges, number, tags, stats, bib}.rs`)** consumes `TexSkeletonOutput` returned by `skeleton::build_tex_skeleton` / `build_tex_skeleton_from_ingested`:
  - `output.skeleton`: mutate `nodes[i].tag` (via `paper_key::node_tag`), populate `nodes[i].parent` from `output.parent_orders[i]`, populate `nodes[i].proof_of` from `output.proof_hints`, populate `nodes[i].number` / `number_basis` using `output.theorem_table`, populate `words`/`read_minutes`/`stats`/`edges`/`bib`.
  - `output.flattened`: map `\ref`/`\eqref`/`\cref`/`\cite` byte/line positions back to physical `Span`s via `FlattenedTex::origin_of` and `spans_for_range`.
- **V3 (`desktop-server/src/investigation_skeleton.rs`)**: calls `build_tex_skeleton_from_ingested` (after V2 post-processing is wired in) and stores serialized `PaperSkeleton` JSON via `Ledger::put_blob`.
- **No conflicts with other concurrent briefs (`f3-net-gate`, `f4-f9-quote-budget`, `s2-solve-model`)** except the one-line module declaration in `crates/research-tools/src/lib.rs`.

---

## 7. Verification & test results

### Commands run and exact outcomes

1. **V1 skeleton & golden tool-definition tests (with `--locked`):**
   ```bash
   cargo test --locked -p research-tools --test skeleton_tex --test skeleton_props --test tool_def_golden -- --nocapture
   ```
   - `skeleton_props`: **4 passed, 0 failed, 0 ignored** (200 random fuzz seeds + same-line multi-file + dormant single-line includes + 6,000-node cap)
   - `skeleton_tex`: **12 passed, 0 failed, 0 ignored** (all 8 fixtures + ingested zip + malformed TeX + depth limit + 5,000-line timing)
   - `tool_def_golden`: **3 passed, 0 failed, 0 ignored** (`golden/tool_defs.txt` unchanged)
   - **5,000-line measured timing (unoptimized debug build):** `lines=5000, nodes=1720, json_bytes=438148, flatten=35.1 ms, scan=127.3 ms, total=212.2 ms` (well under the 10 s TeX target).

2. **Clippy on touched crate targets (`-D warnings`):**
   ```bash
   cargo clippy -p research-tools --lib --test skeleton_tex --test skeleton_props -- -D warnings
   ```
   - Result: **passed (0 warnings, 0 errors)**.

3. **Full workspace clippy:**
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```
   - Failed only on **pre-existing** warnings (none in our code). Proved pre-existing by running `cargo clippy --workspace --all-targets --no-deps -- -W clippy::all` on detached base commit `49edc18dd66b85044046dacac186af3593a44de3` with `rustc 1.100.0`, which emitted the exact same 6 warnings across 5 untouched files:
     1. `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`, documented in `RULES.md`)
     2. `crates/research-tools/tests/sandbox_and_fs.rs:6` (unused imports `TEX_AUDIT`, `is_symlink_or_junction` on Linux where Windows junction tests are `cfg(windows)`)
     3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (unused imports `BufRead`, `BufReader` on Linux)
     4. `crates/agent/tests/eval_harness.rs` (2 clippy lints on Rust 1.100.0: `match` looks like `matches!`, `map_or` simplification)
     5. `crates/agent/src/lib.rs` (test module ordering lint `items after a test module` on Rust 1.100.0)

4. **Full workspace test suite:**
   ```bash
   cargo test --workspace --no-fail-fast
   ```
   - Result: **655 passed, 0 failed, 2 ignored** across all workspace crates.
   - Note on `RE_SEARCH_PYTHON`: when `RE_SEARCH_PYTHON=/usr/bin/python3` is exported on Linux, `cli::desktop_and_ext::cli_research_workspace_read_fork_diff_export_and_verify` hangs indefinitely (see Section 9 below); without `RE_SEARCH_PYTHON`, `cli --test desktop_and_ext` passes 2/2 in 0.57 s.

5. **File-length check:**
   ```bash
   python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2
   ```
   - Result: **`No touched source file is over 600 lines.`** (exit 0).

---

## 8. What could not be compiled or run on Linux

- `#[cfg(windows)]` code paths across the workspace (AppContainer sandbox, `re-search-app` WebView2 desktop window, Windows DPAPI vault, NTFS symlink/junction sandbox tests, Windows Python worker tests).
- `research-tools::skeleton` itself has no platform-specific code (`cfg(windows)` or Unix-only paths/shells) and uses forward-slash normalized relative paths matching `WorkspaceSandbox` and `ingest.rs`.

---

## 9. Bugs noticed in existing code (not fixed)

- **`crates/cli/tests/desktop_and_ext.rs:149-168` (`cli_research_workspace_read_fork_diff_export_and_verify`):** When `RE_SEARCH_PYTHON` is set in the environment on Linux, `re-search run --route api` rejects `--python` on non-Windows before contacting the local HTTP mock, exiting immediately with an error; the test then unconditionally calls `mock.finish()`, which joins the mock server thread blocked forever in `listener.accept()`. Checking `out.status` before `mock.finish()` (or clearing `RE_SEARCH_PYTHON` in the test) would prevent the deadlock.
- **`crates/research-tools/tests/sandbox_and_fs.rs:6`** and **`crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`:** Unused imports under non-Windows compilation cause `cargo clippy --workspace --all-targets -- -D warnings` to fail on Linux.
- **`crates/desktop-server/src/evidence.rs:744`**, **`crates/agent/tests/eval_harness.rs`**, **`crates/agent/src/lib.rs`**: Pre-existing clippy warnings under Rust 1.100.0 (unchanged from base commit `49edc18`).

---

## 10. Design contradictions or impossibilities found

None in V1. The boundary between V1 and V2 in `investigate.md` section 9 is clean, with one minor clarification resolved in code: because `SkNode::parent` and `SkNode::proof_of` hold node tags (`Option<tag>`) that do not exist until V2, V1 leaves those fields `None` on `SkNode` and exposes `parent_orders` and `proof_hints` on `TexSkeletonOutput` so V1's span-nesting property test and V2's tag/edge wiring both have the exact tree and proof-attachment structure without fabricating placeholder tags.
