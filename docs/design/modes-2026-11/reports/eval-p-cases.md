# Report: `eval-p-cases` — Investigate fixed evaluation cases (F8)

## Base commit

- Base branch: `gemini/work/fixed`
- Base commit: `c5c89e64806c2430440ef4cd05b1b7fa86274078`
- Working branch: `work/eval-p-cases` (target push branch: `gemini/modes/eval-p-cases`)
- Scope: Package F8 fixed-tier Set P (Investigate structure evaluation cases). No production code, no tool definition changes, no `Cargo.lock` changes, no third-party paper text, no version tracking (P2), no author novelty labels (P3), no PDF case set (P6).

---

## What was implemented

### 1. Fixed-tier Set P fixtures and manifest (F8)

Preserved the two existing harness self-test fixture files (`selftest-theorems.tex`, `selftest-oscillator.tex`) byte-for-byte unchanged and kept both self-test case entries (`P-selftest-theorems`, `P-selftest-oscillator`) semantically/JSON-identical in `manifest-p.json` (reformatted with 2-space pretty-printing when adding the new cases). Added **10 original fixed-tier Set P evaluation cases** (`P-eval-01` through `P-eval-10`) covering **62 section-tree nodes**, **70 theorem-like statements** (with **47 proofs**), **29 labelled equations**, **3 citations with locators**, **1 fixed-schema `input.locator`**, and **exactly 30 sampled `uses_edges`** (`12` `statement_use`, `11` `proof_use` including `4` lifted inner-equation proof uses, and `7` `heuristic_mention`).

Every `.tex` file was authored from scratch for this evaluation (LF-only, verified clean with two-pass `pdflatex` in a temporary directory outside the repository to confirm every printed counter independently of any Rust TeX scanner).

| File | Lines | Role / Coverage |
|---|---:|---|
| `tests/fixtures/eval/manifest-p.json` | 2,980 | 2 preserved self-tests + 10 `P-eval-*` cases; 30 sampled `uses_edges` (`from` = prerequisite -> `to` = dependent) |
| `tests/fixtures/eval/p/eval-01-damped-wave.tex` | 73 | Physics note (no theorem envs): damped Klein-Gordon wave, 2-level sections, explicit `\eqref`, heuristic `Eq. (1)` |
| `tests/fixtures/eval/p/eval-02-ising-meanfield.tex` | 89 | Physics note (no theorem envs): Weiss mean-field Ising, `\appendix` (`A`), unnumbered `\section*{Acknowledgments}`, heuristic `Eq. (2)`, `\cite[Section~3.1]{kerr2019ising}` |
| `tests/fixtures/eval/p/eval-03-berry-chern.tex` | 84 | Physics note (no theorem envs): Berry curvature & Chern number, multi-label `align` (`(2)` & `(3)`), heuristic `Eq. (3)` |
| `tests/fixtures/eval/p/eval-04-rg-flow.tex` | 84 | Physics note (no theorem envs): $\phi^4$ RG flow in $4-\epsilon$, 3-level section tree (`\section`/`\subsection`/`\subsubsection`), math in heading, heuristic `Eq. (2)` |
| `tests/fixtures/eval/p/eval-05-pos-refs.tex` | 121 | Position-sensitive cross-references (statement vs adjacent proof vs deferred proof vs section prose between environments) & inner equation `eq:energy-id` lifted into deferred proof of `thm:decay` |
| `tests/fixtures/eval/p/eval-06-ambiguous-nums.tex` | 123 | Independent colliding counters (`Lemma 1`, `Theorem 1`, `Eq. (1)`), manual `\addtocounter{proposition}{-1}` creating duplicate `Proposition 2.1`, unnumbered unlabelled `claim*` coordinate, appendix `\renewcommand{\thelemma}{A.\arabic{lemma}}` |
| `tests/fixtures/eval/p/eval-07-multifile/main.tex` | 23 | Multi-file entry point (`\input{macros}`, `sec1_defs`, `sec2_results`, `sec3_proofs`) |
| `tests/fixtures/eval/p/eval-07-multifile/macros.tex` | 13 | Custom operator macros & custom theorem environments (`mythm`, `mylem`, `myprop`, `mycor`, `mydef`, `myassump`) |
| `tests/fixtures/eval/p/eval-07-multifile/sec1_defs.tex` | 39 | Section 1: `assump:domain`, `def:resolvent`, `lem:neumann` with inner equation `eq:neumann-series` |
| `tests/fixtures/eval/p/eval-07-multifile/sec2_results.tex` | 35 | Section 2: `prop:stability`, `thm:perturbation`, `cor:gap-preserve` with cross-file statement `\ref`s |
| `tests/fixtures/eval/p/eval-07-multifile/sec3_proofs.tex` | 31 | Section 3: cross-file deferred proofs & lifted cross-file `\eqref{eq:neumann-series}` |
| `tests/fixtures/eval/p/eval-08-cited-locator.tex` | 99 | `Input::Fixture.locator = "Theorem 2.1"` -> `thm:spectral-gap`, `head_cite` `\cite[Theorem~2.1]{nord2024operator}`, proof `cite` `\cite[Proposition~4.3]{hale2023bounds}`, synthetic bibliography |
| `tests/fixtures/eval/p/eval-09-comments-cref.tex` | 110 | Comment-only traps (fake theorem & commented refs in statement/proof), multi-label `\cref{assump:integrable,lem:subadd}`, lifted inner equation `eq:kingman-step`, diverse kinds (`assumption`, `example`, `remark`, `conjecture`) |
| `tests/fixtures/eval/p/eval-10-long-monograph.tex` | 3,996 | Long generated monograph (34,687 words, 266,213 bytes, **86 pages** in 11pt `article` `pdflatex`), 30 sections/subsections, 40 theorem-like statements, 30 proofs, 10 inner lemma equations, 4 long-range sampled uses |

### 2. Test harness and structural validation (`crates/agent/tests/`)

| File | Lines | Change |
|---|---:|---|
| `crates/agent/tests/eval_harness.rs` | 456 | Minimal local update (+14/-3 lines in its own commit) to `fixed_tier_manifests_load_and_validate` so it preserves all self-test assertions for P and Q while asserting the 10 new `P-eval-*` cases |
| `crates/agent/tests/eval_p_cases.rs` | 326 | New independent validation test binary verifying LF-only raw TeX, required case mix, exact section/statement/proof/equation/citation/bibliography line coordinates, exact total of 30 directed `uses_edges`, lifted inner-equation containment, and `non_edges` trap sites — without invoking V1 |

---

## Harder cases (exact fixture file and line citations)

1. **Position-sensitive cross-references (`p/eval-05-pos-refs.tex`):**
   - Statement use: `def:coercive -> prop:contraction` at `p/eval-05-pos-refs.tex:61` (inside `prop:contraction` L59–64) and `def:coercive -> thm:decay` at `p/eval-05-pos-refs.tex:74` (inside `thm:decay` L72–80).
   - Adjacent proof use: `lem:energy -> prop:contraction` at `p/eval-05-pos-refs.tex:66` (inside adjacent proof L65–70).
   - Plain section prose between two environments: `Lemma~\ref{lem:energy}` at `p/eval-05-pos-refs.tex:82` sits strictly between `\end{theorem}` (L80) and `\begin{remark}` (L84). Recorded under `prose_refs` (`to: sec:main`) and `non_edges` (must NOT attach to `thm:decay` or `rem:sharpness`).
   - Deferred proof binding & proof use: `\begin{proof}[Proof of Theorem~\ref{thm:decay}]` at `p/eval-05-pos-refs.tex:107` binds `proves` (`rule: "proof_header"`, listed in `non_edges` so it does not emit a self-loop `uses` edge); `Proposition~\ref{prop:contraction}` at `p/eval-05-pos-refs.tex:108` is a deferred `proof_use` targeting `thm:decay`.
2. **Inner equations used in proofs (lifted `uses` edges):**
   - `p/eval-05-pos-refs.tex:41-44` (`eq:energy-id` inside `lem:energy` L35–45) referenced by `\eqref{eq:energy-id}` at `p/eval-05-pos-refs.tex:111` inside deferred proof of `thm:decay` (L107–119) -> lifted `proof_use` `lem:energy -> thm:decay`.
   - `p/eval-07-multifile/sec1_defs.tex:25-30` (`eq:neumann-series` inside `lem:neumann` L21–31) referenced cross-file at `p/eval-07-multifile/sec3_proofs.tex:22` (and `alt_sites` L28) inside deferred proof of `thm:perturbation` (L19–31) -> lifted `proof_use` `lem:neumann -> thm:perturbation`.
   - `p/eval-09-comments-cref.tex:34-37` (`eq:kingman-step` inside `lem:subadd` L29–38) referenced at `p/eval-09-comments-cref.tex:65` (and `alt_sites` L69) inside proof of `thm:lyapunov` (L64–73) -> lifted `proof_use` `lem:subadd -> thm:lyapunov`.
   - `p/eval-10-long-monograph.tex:927-930` (`eq:s03-cluster-est` inside `lem:s03-cluster` L919–936, Section 3) referenced at `p/eval-10-long-monograph.tex:3127` inside proof of `thm:s08-massgap` (L3102–3135, Section 8) -> long-range lifted `proof_use` `lem:s03-cluster -> thm:s08-massgap`.
3. **Ambiguous printed numbers & coordinate-identified unlabelled statement (`p/eval-06-ambiguous-nums.tex`):**
   - Bare numeral `1` collides across `eq:rayleigh-quot` (L22–25, `(1)`), `lem:rayleigh` (L31–39, `Lemma 1`), and `thm:spectral-lower` (L47–51, `Theorem 1`). Heuristic mentions `"Lemma 1"` (L94) and `"Theorem 1"` (L96) inside `thm:cheeger`'s proof (L90–97) disambiguate by kind+number.
   - `\addtocounter{proposition}{-1}` at `p/eval-06-ambiguous-nums.tex:73` causes both `prop:cut-upper` (L64–70) and `prop:cut-refined` (L75–81) to print as `Proposition 2.1`. Plain-text mention `"Proposition 2.1"` at `p/eval-06-ambiguous-nums.tex:99` is genuinely ambiguous and recorded symmetrically in `non_edges` for both propositions.
   - Unnumbered, unlabelled `\begin{claim*}` at `p/eval-06-ambiguous-nums.tex:101-104` uses stable coordinate ID `coord:p/eval-06-ambiguous-nums.tex:L101-104` as the dependent of `thm:spectral-lower` via `\ref{thm:spectral-lower}` at L102.
   - Appendix counter reset `\setcounter{lemma}{0}` and `\renewcommand{\thelemma}{A.\arabic{lemma}}` at `p/eval-06-ambiguous-nums.tex:110-111` give `lem:app-sum` (L113–121) printed number `A.1` (`number_status: "ambiguous"`).
4. **Multi-file bundle with custom theorem macros (`p/eval-07-multifile/`):**
   - Custom environments declared in `macros.tex:7-13` (`mythm`, `mylem`, `myprop`, `mycor`, `mydef`, `myassump`), imported by `main.tex:5`, used across `sec1_defs.tex`, `sec2_results.tex`, `sec3_proofs.tex` with cross-file statement uses (`sec2_results.tex:8,16`) and cross-file deferred proofs (`sec3_proofs.tex:6-17,19-31`).
5. **Citations with locators & fixed-schema `input.locator` (`p/eval-02-ising-meanfield.tex`, `p/eval-08-cited-locator.tex`):**
   - `P-eval-08-cited-locator` sets `input.locator = "Theorem 2.1"` (validated by `eval_support/manifest.rs`), resolving to `thm:spectral-gap` at `p/eval-08-cited-locator.tex:32-44`.
   - Heading citation locator: `\begin{proposition}[{\cite[Theorem~2.1]{nord2024operator}}]` at `p/eval-08-cited-locator.tex:59` (`rule: "head_cite"`, `role: "statement"`).
   - Proof citation locator: `\cite[Proposition~4.3]{hale2023bounds}` at `p/eval-08-cited-locator.tex:91` (`rule: "cite"`, `role: "proof"`).
   - Prose citation locator: `\cite[Section~3.1]{kerr2019ising}` at `p/eval-02-ising-meanfield.tex:49` (`rule: "cite"`, `role: "prose"`).
6. **Comment traps & multi-label `\cref` (`p/eval-09-comments-cref.tex`):**
   - Multi-label `\cref{assump:integrable,lem:subadd}` at `p/eval-09-comments-cref.tex:56` produces two `statement_use` edges targeting `thm:lyapunov`.
   - Commented-out fake theorem at `p/eval-09-comments-cref.tex:46-49`, commented ref inside `thm:lyapunov` proof at L68, and commented ref inside `cor:rate` statement at L80 are all recorded in `non_edges`.
7. **Long generated monograph (`p/eval-10-long-monograph.tex`):**
   - Cross-section statement use `def:s01-lattice -> thm:s05-transfer` at L1887 (Sec 1 -> Sec 5), long-range lifted proof use `lem:s03-cluster (eq:s03-cluster-est) -> thm:s08-massgap` at L3127 (Sec 3 -> Sec 8), cross-section proof use `prop:s06-spectral -> thm:s10-main` at L3952 (Sec 6 -> Sec 10), and long-range heuristic mention `"Theorem 5.4"` (`thm:s05-transfer -> thm:s10-main`) at L3947 (Sec 5 -> Sec 10).

---

## Independent Second Neon Reviewer audit: disagreements and resolutions

An independent read-only Neon reviewer (`research` subagent) read all 14 new raw TeX files and `manifest-p.json` without consulting V1. Every section tree (62/62), statement span (70/70), proof span (47/47), equation span (29/29), and sampled edge direction/site (30/30) verified clean. The reviewer raised **five concrete disagreements** and several judgment calls, all resolved before committing:

| # | Reviewer disagreement | Resolution |
|---|---|---|
| B.1 | **Inconsistent `number_status` between `P-eval-06` and `P-eval-07`:** `thm:spectral-lower` (`Theorem 1`, `eval-06:47-51`) was marked `"ambiguous"` while `thm:cheeger` (`Theorem 2`, `eval-06:83-89`) and `assump:domain` (`Assumption 1`, `eval-07/sec1_defs.tex:6-11`) were marked `"confirmed"`, even though the `theorem` counter is never mutated. | **Accepted.** Adopted one uniform rule across all cases: `confirmed` = unique `(kind, number)` on an unmutated counter; `ambiguous` = duplicate printed `(kind, number)` (`prop:cut-upper`, `prop:cut-refined`) or counter mutated by `\setcounter`/`\renewcommand` (`lem:rayleigh`, `lem:app-sum`); `unnumbered` = starred env (`claim*`). Updated `thm:spectral-lower` to `"confirmed"`. |
| B.2 | **Missing `role` on `citations_with_locators`:** `hale2023bounds` at `eval-08:91` sits inside `cor:mixing`'s proof (L87–92), not its statement (L75–86), yet `in_node: "cor:mixing"` lacked a `role` field. | **Accepted.** Added `"role"` (`"statement"`, `"proof"`, or `"prose"`) to all three `citations_with_locators` entries (`eval-02:49` -> `prose`, `eval-08:59` -> `statement`, `eval-08:91` -> `proof`). |
| B.3 | **Multi-site lifted references listed only one line:** `\eqref{eq:neumann-series}` appears at both `eval-07/sec3_proofs.tex:22` and `28`; `\eqref{eq:kingman-step}` appears at both `eval-09:65` and `69`. | **Accepted.** Added `"alt_sites"` listing the second valid line (`sec3_proofs.tex:28` and `eval-09:69`) so a deduplicating V2 runner matching either site passes cleanly. |
| B.4 | **Asymmetric `non_edges` in `P-eval-06` L99:** Only `prop:cut-upper -> sec:cuts` was listed for ambiguous `"Proposition 2.1"`, omitting the symmetric `prop:cut-refined -> sec:cuts`. | **Accepted.** Added the symmetric `prop:cut-refined -> sec:cuts` entry, and also added `non_edges` for deferred proof headers (`eval-05:107`, `eval-07/sec3_proofs.tex:6,19`) so V2 tests that `proof_header` refs do not emit self-loop `uses` edges. |
| B.5 | **Schema completeness gaps:** `P-eval-08` omitted `"equations": []`; `P-eval-10` omitted two prose `\ref`s at `eval-10:2316-2317`; `P-eval-02` and `P-eval-08` omitted `"bibliography"` entries. | **Accepted.** Added `"equations": []` to `P-eval-08`, added both `prose_refs` (`prop:s06-spectral`, `thm:s06-infrared` -> `subsec:s06b` at `eval-10:2316-2317`), and added `"bibliography"` entries to `P-eval-02` and `P-eval-08`. |

---

## Independent Carbon Audit

A separate hard independent audit (`research` subagent on `pro` / Carbon tier) reviewed all three code/fixture commits (`c303a99`, `1b57a6a`, `206c7ec`), all 14 new TeX fixtures (independently compiling all 10 cases with two-pass `pdflatex` and cross-checking `.aux` printed counters against `manifest-p.json`), `eval_harness.rs`, `eval_p_cases.rs`, `RULES.md` compliance, workspace test counts (`781 passed; 0 failed; 2 ignored`), and every line count and citation in this report.

The Carbon auditor verified all code, fixtures, manifest keys, git hygiene, and test results clean, and flagged two report accuracy defects plus two minor polish items — all fixed before committing this report:
1. Updated the `P-selftest-theorems` reversed-edge citation under "Bugs noticed" from base-commit lines `18-21` to `HEAD` lines `34-59` (noting `18-21` on base `c5c89e6`).
2. Qualified the self-test preservation statement: the two self-test `.tex` files are byte-for-byte unchanged while their two JSON case objects in `manifest-p.json` are semantically/JSON-identical after 2-space pretty-printing.
3. Corrected the unsampled `\ref`/`\eqref` count in `eval-10-long-monograph.tex` below to exact audited figures (`49` in statements/proofs, `51` including the `2` `prose_refs`).

---

## Decisions taken alone, deviations, and assumptions

1. **10-case fixed-tier Set P structure:** Mirrored `00-overview.md` §7.2's 10-paper mix (4 physics notes without theorem environments, 1 multi-file bundle with custom theorem macros, 1 long generated document >80 pages, plus 4 targeted math fixtures covering position-sensitive refs, ambiguous numbers, citation locators, and comment/`\cref` traps).
2. **All 30 `uses_edges` carry `"right": true`, with traps in `"non_edges"`:** Because `eval-p-cases.md` specifies "a total of 30 sampled uses... Record whether each connection is a statement use, proof use or a heuristic mention", every one of the 30 sampled uses is a verified positive use (`12` `statement_use`, `11` `proof_use`, `7` `heuristic_mention`), while comment traps, prose-between-environments traps, deferred proof-header traps, and ambiguous-number traps live in `"non_edges"`.
3. **Physics-note heuristic mentions attach to enclosing section (`role: "prose"`):** At `eval-01:66`, `eval-02:47`, `eval-03:71`, `eval-04:71`, each `Eq. (k)` mention lies in prose outside any equation environment; attaching `to` to the enclosing section (`role: "prose"`) follows `investigate.md` §4.1 ("decided by position only"). Same-section `prose_refs` (`eval-01:61`, `eval-02:54`) are recorded unlifted.
4. **Sampled subset vs complete graph:** `uses_edges` is a 30-edge sample across the 10 cases, not an exhaustive edge list (for example, `eval-05:100`, `eval-06:38`, `eval-07/sec3_proofs.tex:7,16`, and `49` unsampled `\ref`/`\eqref` occurrences inside statements/proofs of `eval-10` — `51` including its `2` `prose_refs` — are valid unsampled edges).
5. **Minimal `eval_harness.rs` edit instead of leaving it failing:** `eval-p-cases.md` stated: *"If the harness already validates the new cases, leave its code alone."* Running the unmodified harness failed at `crates/agent/tests/eval_harness.rs:36` (`assert_eq!(set.cases_of(Set::P).len(), 2)`) and line 40 (requiring every case's `notes` to say `"Harness self-test case, not part of the evaluation set"`). We made a 14-line local update in its own commit (`1b57a6a`), preserving every self-test assertion for P and Q, and placed deep fixture validation in a new file `crates/agent/tests/eval_p_cases.rs` (`206c7ec`).
6. **Multi-file input path:** Because `crates/agent/tests/eval_support/manifest.rs:333-336` enforces `files.len() == 1` for `Set::P`, `P-eval-07-multifile-macros` lists only the entry file `["p/eval-07-multifile/main.tex"]` in `input.files` and lists the four `\input`-included sibling files in `answer_key.included_files`.
7. **Locator whitespace normalization:** TeX non-breaking space `~` in `\cite[Section~3.1]`, `\cite[Theorem~2.1]`, `\cite[Proposition~4.3]` is normalized to ASCII space in `manifest-p.json`.
8. **Assumptions:** `eval-10-long-monograph.tex` was compiled with `pdflatex` (TeX Live on Linux, `article` 11pt) to confirm **86 pages** (>80 pages). Exact page count can vary slightly by TeX distribution/paper size (`letterpaper` vs `a4paper`), while staying comfortably above 80 pages.

---

## Milestone checks that still need V2 or V3

None of the deterministic Investigate quality/timing measures in `00-overview.md` §7.3 can be scored on Set P until V2 and V3 land:

- **Needs V2 (`inv-a-v2-v5`, packages `edges.rs`, `number.rs`, `tags.rs`, `bib.rs`, plus a fixed-tier Set P runner):**
  - `statement_recall_tex_theorem_envs` (M1 gate: `>= 0.9` on `P-eval-05`..`P-eval-10`)
  - `section_recall_tex_theorem_envs` (M1 gate: `>= 0.9` on `P-eval-05`..`P-eval-10`)
  - `statement_recall` and `section_recall` across all 10 `P-eval-*` cases
  - `uses_edge_precision_statement` and `uses_edge_precision_proof` (plus reporting `prose`-role `heuristic_mention` resolution on `P-eval-01`..`04`)
  - `time_to_skeleton_tex_s` (`< 10.0 s`, especially measured on `P-eval-10-long-monograph`)
  - Deterministic `BibRef.locators` and single-file `input.locator` matching on `P-eval-02` and `P-eval-08`
- **Needs V3 (`inv-a-v2-v5`, ledger `investigation.skeleton` event + mapview route + UI):**
  - End-to-end M1 check: skeleton built from ingested receipt with zero `model.request` events and rendered on screen (`GraphView`).
- **Out of scope for this brief (deferred or other sets):** P2 (version carry-over, deferred), P3 (author novelty labels, needs V7 Orient), P5 cross-paper cited-result lookup (needs V4 + V2), P6 (PDF case set, needs F14a + V17).

---

## Merge notes

- **`crates/agent/tests/eval_harness.rs:35-52` (`fixed_tier_manifests_load_and_validate`, commit `1b57a6a`):**
  - Updated `Set::P` count check from `2` to `2` self-tests + `10` `P-eval-*` cases (`12` total) and scoped the `"Harness self-test case, not part of the evaluation set"` note check to `-selftest-` IDs.
  - **Expected merge point with `eval-q-cases`:** `eval-q-cases` will update `assert_eq!(set.cases_of(Set::Q).len(), 2);` on line 42 and the Set Q branch of the notes check. Our `else if s == Set::P { ... } else { assert!(!case.notes.trim().is_empty(), ...); }` structure keeps the Set Q merge trivial.
- **`tests/fixtures/eval/manifest-p.json` (commit `c303a99`):**
  - Consumed by the future V2 evaluation runner (`inv-a-v2-v5`). The runner should filter `id.starts_with("P-eval-")` (skipping `P-selftest-*`, whose legacy `P-selftest-theorems` edges point backwards), ingest all files under `p/eval-07-multifile/` when loading `P-eval-07-multifile-macros`, resolve statements by `label` or fallback physical `(file, start_line, end_line)` coordinate for unlabelled nodes (`coord:p/eval-06-ambiguous-nums.tex:L101-104`, `coord:p/eval-02-ising-meanfield.tex:L80`), and accept either `site` or `alt_sites` for multi-occurrence edges.

---

## Verification commands and exact results

Toolchain on this Linux host: `rustc 1.100.0`, `cargo 1.100.0`, `Python 3.12.16`.

| Command | Result |
|---|---|
| `cargo test --locked -p agent --test eval_harness` | **9 passed; 0 failed; 2 ignored** (`live_manifests_validate`, `file_sheet_from_json` — live tier) |
| `cargo test --locked -p agent --test eval_p_cases` | **3 passed; 0 failed; 0 ignored** |
| `cargo test -p research-tools --test tool_def_golden` | **3 passed; 0 failed; 0 ignored** (`tool_defs.txt` unchanged) |
| `cargo test --workspace --no-fail-fast` | **781 passed; 0 failed; 2 ignored** (exit code `0`) |
| `cargo clippy -p agent --test eval_harness --test eval_p_cases -- -D warnings -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or` | **0 warnings, 0 errors** in our touched/added files |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Passed** (`No touched source file is over 600 lines.`) |
| `git diff origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/ .github/` | **Empty** |

### Pre-existing clippy lints on Linux (`rustc 1.100.0`), proven on base commit `c5c89e6`

All five lints below were reproduced directly on base commit `c5c89e64806c2430440ef4cd05b1b7fa86274078` (`gemini/work/fixed`) and left untouched per `RULES.md`:

1. `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`) — known in `RULES.md`.
2. `crates/agent/src/guardrails.rs:335` (`clippy::items_after_test_module`).
3. `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`).
4. `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`).
5. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` (`unused_imports`: `TEX_AUDIT`, `is_symlink_or_junction`).

### What could not be compiled or run on Linux

- `cfg(windows)` code paths (AppContainer sandbox, Windows DPAPI vault, JobObject limits, `desktop-app` WebView2 runtime).
- Live-tier ignored tests (`live_manifests_validate`, `file_sheet_from_json`), which require external live manifests/sheets.

---

## Bugs noticed but not fixed

1. **`tests/fixtures/eval/manifest-p.json:34-59` (`18-21` on base `c5c89e6`, `P-selftest-theorems`):** The pre-existing harness self-test records `uses_edges` backwards (`from: dependent` -> `to: prerequisite`, e.g., `from: "lem:sum", to: "def:even"`), contradicting `investigate.md` §4.1 (`from` = prerequisite, `to` = dependent). Left untouched per brief ("Keep the two existing harness self-tests").
2. **`crates/cli/tests/desktop_and_ext.rs:97,149-168` (`cli_research_workspace_read_fork_diff_export_and_verify`):** Running `RE_SEARCH_PYTHON=$(command -v python3) cargo test --workspace` on Linux causes this test to hang indefinitely at `mock.finish()` because `re-search run --route api` rejects non-Windows `--python`/`RE_SEARCH_PYTHON` and exits before sending the 3 expected HTTP requests to `ScriptedMock`. Running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` passes all 781 tests (including this test in 0.47s).

---

## Design inconsistencies / friction noted

1. **Single-file fixture constraint for Set P (`crates/agent/tests/eval_support/manifest.rs:333-336`):** `validate_input` requires `files.len() == 1` for `Set::P`, even though `00-overview.md` §7.2 and `eval-p-cases.md` require a multi-file fixture. Handled cleanly by pointing `input.files` to the entry point `p/eval-07-multifile/main.tex` and listing `\input` children in `answer_key.included_files`.
2. **Prose-role edges on physics notes vs `measures.rs`:** `00-overview.md` §7.3 and `crates/agent/tests/eval_support/measures.rs:97-98` define `uses_edge_precision_statement` and `uses_edge_precision_proof`, whereas physics notes without theorem environments (`P-eval-01`..`04`) only produce `prose`-role equation/section references. A V2 runner should either report `prose` separately in the sheet notes or pool heuristic mentions accordingly.
3. **Stale Solve tier word in `measures.rs` (not ours, noted for completeness):** `crates/agent/tests/eval_support/measures.rs:121-125` still uses `"Computed"` in measure labels/IDs (`planted_error_ends_computed`, `steps_computed_share`), whereas `00-overview.md` §8.1 / S2 adopted `"Verified"` and `"Checked numerically"`.
