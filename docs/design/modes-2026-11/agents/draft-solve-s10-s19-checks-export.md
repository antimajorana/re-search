# Brief draft-solve-s10-s19-checks-export — Physics checks (S10) & Notebook/ZIP export (S19)

**Draft status:** Planning brief — **two disjoint sub-lanes runnable in parallel** (zero shared source files; dispatch either as two independent agents or as one lead with two subagents) once S9 merges.
**Packages:**
- **Sub-lane A — S10:** Physics checks (`dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, `matrix_property`).
- **Sub-lane B — S19:** Split `handle_export` out of 610-line `routes_tools.rs`, then Notebook export (`nbformat 4.5`) and Solve ZIP replay (`zip.rs`, `replay_checks.py`).
**Governing design:** `00-overview.md` 0.1, 2.13 (boundary with F15), 7.3 (R22), 8.1–8.3; `solve.md` 4.4, 5.1, 5.6, 6 (R4, R5, R8, R19, R22), 7.2–7.3, 8.3, 9 (rows S10, S19), 11 (licence note on "Get Physics Done"), 12 (risk 4), 13 (D1, D18); `agents/RULES.md`.
**Read reports first:** `reports/s2-solve-model.md`, and (once merged) `reports/q-f11-x1-s3-s6.md`, `reports/f5-f7-mode-shell.md`, and `reports/solve-s7-s8-s9-s11.md`.

## 1. Start conditions & owner gates

- **Required merges on `origin/gemini/work/fixed` before either sub-lane starts:**
  - Already merged on base `832eab5bf`: F0, F1, F4, F9, S2.
  - **Hard merge gates:** `q-f11-x1-s3-s6` (S6 check process & `srepr_safe.py`) and `solve-s7-s8-s9-s11` up through **S9** (`python/checks/__init__.py`, `algebra.py`, `solve_checks.rs`).
  - **For S19 desktop HTTP wiring:** `f5-f7-mode-shell` (F5 `api_dispatch.rs`) so `GET /api/sessions/{id}/export?format=notebook` routes cleanly without touching 702-line `http_srv.rs`.
- **Owner gates:** None beyond the accepted 2026-11-25 decisions. **Zero new Rust or Python dependencies** (`Cargo.lock` untouched; nbformat 4.5 JSON written directly with `serde_json`).
- **Parallelism rule:** S10 touches only `python/checks/` + `crates/research-tools/src/solve_checks.rs`; S19 touches only `crates/export/src/` + `crates/desktop-server/src/{routes_tools,routes_export}.rs` (+ `api_dispatch.rs`). They share **zero** files and may run concurrently after S9 merges.

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- S2 (`solve_own_tier.rs:16-23`): `NON_IDENTITY_CHECKS = &["dimensions", "limit_case", "symmetry", "conservation", "convergence", "matrix_property"]`. Passing any of these adds its name as a sublabel ONLY and NEVER grants `Verified` or `CheckedNumerically`; failing turns the step `Failed` (R8).
- S2 (`solve_badge.rs`): `wording::*` single wording table (`BADGE_VERIFIED`, `OWN_CHECK_VERIFIED`, `BADGE_CHECKED_NUMERICALLY`, `NON_VERIFIED_CONSTANTS`). `proof_closure.rs`: `closure_ids`, `topological_order`.
- **Export crate (`crates/export/src/`):**
  - `lib.rs` (132 lines): `ExportFormat { Tex, Html, Zip }`, `export_session_to_path`, `export_session_bytes`. **Critical:** `lib.rs:86,117` currently calls Research `latex::load_session_export_data` unconditionally; S19 MUST dispatch `ExportFormat::Notebook` and Solve ZIP via `SolveState` / verified ledger directly without forcing Research `latex.rs` (771 lines, Do NOT edit!) assumptions or breaking Research exports.
  - `zip.rs` (341 lines): `build_reproducible_zip`, `runtime` embedded files array (lines 84–102), `replay_manifest.json`.
  - `replay_checks.py` (130 lines): deterministic hash check `--verify-only` and per-worker replay.
- **CRITICAL BASE-COMMIT DETAIL — `routes_tools.rs` IS 610 LINES & EXPORT IS `GET`!**
  - `crates/desktop-server/src/routes_tools.rs` is **610 lines** (over 600!) and contains `handle_export` with an exhaustive `match format` at `routes_tools.rs:579` — adding `ExportFormat::Notebook` breaks compilation unless `handle_export` is extracted FIRST!
  - Existing export route is **`GET /api/sessions/{id}/export?format=<fmt>`** (`http_srv.rs:433`, `auth.rs:208`, `desktop-app/src/rules.rs:57-80` — NOT `POST`!), with no `routes_export.rs` file on base!
- Worker staging (`crates/worker/src/stage.rs:19`, 475 lines, Windows-only): `WORKER_HELPERS = &["solver_calc.py", "symbolic_validation.py", "numerical_validation.py"]` — verify after S6 how `python/checks/` is staged on Windows.

**Future-merge interfaces to verify after S6 and S9 merge:**
- From **S6 / S9:** exact check-function signature and registry registration in `python/checks/__init__.py` (`registry_sha256`), `assumed_symbol` helper for setup assumptions, and whether S6 stages `python/checks/` recursively or by explicit file list in `crates/worker/src/stage.rs:19` (if explicit, note the Windows-only `stage.rs` `physics.py` line in S10's report without breaking Linux builds).
- From **S9:** whether `crates/research-tools/src/solve_checks.rs` validates check names against a Rust allowlist and how `dimensions` auto-check invokes S10's `dimensions` check when `setup.dimensions` is non-empty.
- From **F5:** `crates/desktop-server/src/api_dispatch.rs` export arm (`GET /api/sessions/{id}/export`).

## 3. Smallest implementation checkpoints

### Sub-lane A — S10 (Physics checks)
1. **Checkpoint S10a (`python/checks/physics.py`):** Implement all 6 pure check functions over handles + setup math context (`symbols`, `dimensions`, `assumptions`), clean-room (zero code/text from "Get Physics Done" per `solve.md` 11):
   - `dimensions`: dimensional analysis of `lhs` and `rhs` using `setup.dimensions` (`{symbol: {base: power}}`); supports integer and rational powers (`"3/2"` per S2); mismatched dimensions -> `fail` with exact computed dimensions; missing symbol dimension -> `inconclusive`.
   - `limit_case`: symbolic/series limit `var -> target` (direction optional) on `lhs` and `rhs` (or against a named target handle); numeric disagreement -> `fail`, unreduced -> `inconclusive`.
   - `symmetry`: applies substitution map (e.g. `{phi -> -phi}`, parity/scale/exchange) and checks invariance or stated sign (`+1`/`-1`) with numeric guard; counterexample -> `fail`.
   - `conservation`: checks vanishing derivative/commutator/divergence (`d/dt Q = 0`, `[H, Q] = 0`, or explicit conservation relation) under setup assumptions; counterexample -> `fail`.
   - `convergence`: tests series/sequence/integral convergence or numerical mesh refinement residual ratio using `mpmath`/`numpy`/`sympy`; divergence beyond tolerance -> `fail`, ambiguous -> `inconclusive`.
   - `matrix_property`: checks declared matrix properties (`hermitian`, `symmetric`, `unitary`, `orthogonal`, `positive_definite`, `traceless`, `invertible`) via exact SymPy where small and numeric eigenvalues with stated tolerance otherwise; violation with witness -> `fail`.
   - Symbol assumption mismatch always returns `inconclusive: symbol assumptions differ`, never `fail` (`solve.md` 12 risk 4).
2. **Checkpoint S10b (registration & Rust dispatch):** Register all 6 checks in `python/checks/__init__.py` (updating `registry_sha256`), wire any check-name allowlist / auto-check `dimensions` hook in `crates/research-tools/src/solve_checks.rs`, note any Windows-only `stage.rs:19` `WORKER_HELPERS` entry if S6 listed check files individually, and add end-to-end tests proving R8.

### Sub-lane B — S19 (Split-first `routes_tools.rs`, Notebook export & ZIP replay)
1. **Checkpoint S19a — MANDATORY ZERO-BEHAVIOUR-CHANGE SPLIT OF `routes_tools.rs` (610 -> `< 560` lines):**
   - Extract `handle_export` (and its private helpers around lines 555–610) out of `crates/desktop-server/src/routes_tools.rs` (610 lines) into a new `crates/desktop-server/src/routes_export.rs`, re-exporting or pointing F5's `api_dispatch.rs` (`GET /api/sessions/{id}/export`) at `routes_export::handle_export`.
   - Zero behaviour change; run `cargo test -p desktop-server` and `check_file_length.py` BEFORE adding `ExportFormat::Notebook`! Commit alone.
2. **Checkpoint S19b (`crates/export/src/notebook.rs`):** Pure function from verified ledger + blobs + `SolveState` to nbformat 4.5 JSON (`ExportFormat::Notebook`, `.ipynb`), default = closure of final answer only (`include_all_runs: bool` option, default `false` per D18):
   - Cell 1 (Markdown): problem setup (statement, task kind, target, symbols + assumptions, conventions, dimensions).
   - Cell 2 (Code): self-contained Python prelude defining `expr(name_or_handle)` and `save_expr(name, obj)` with the session's `solve.expr` handle table inlined (canonical `srepr` loaded via safe SymPy constructors so the notebook runs with plain SymPy outside re:Search, zero app imports required).
   - Per step in topological dependency order (`proof_closure::topological_order`):
     - Markdown cell: step id, effective badge (from S2 `badge_for_step`), `"own check: verified"` ONLY when `own == Verified && effective != Verified` (R22), flags, summary, LaTeX rendered from `solve.expr.latex` (R5).
     - One code cell per cited receipt (`tool_call_refs`) with stored stdout/value outputs.
     - One code cell per matching `solve.check` with its recorded outcome and evidence summary.
   - Final Markdown cell: done checklist (`solve.done.criteria`), status line, and `"rests on"` breakdown.
   - Deterministic cell IDs: first 8 lowercase hex characters of `sha256` of canonical cell content (deduplicated deterministically on collision).
3. **Checkpoint S19c (`lib.rs`, `routes_export.rs`, `zip.rs`, `replay_checks.py`):**
   - Add `ExportFormat::Notebook` (`"ipynb" | "notebook"`) to `crates/export/src/lib.rs` (132 lines), dispatching `ExportFormat::Notebook` directly without invoking Research `latex::load_session_export_data` (`lib.rs:86,117`).
   - Update `crates/desktop-server/src/routes_export.rs` (`GET /api/sessions/{id}/export?format=notebook`) to serve `.ipynb` with `Content-Type: application/x-ipynb+json; charset=utf-8` (preserving existing `auth.rs:208` and `desktop-app/src/rules.rs:57-80` `GET` contract!).
   - Update `crates/export/src/zip.rs` (341 lines) to handle Solve sessions cleanly alongside Research (without editing 771-line `latex.rs`!): include `solve.*` events in `ledger.json`, bundle `python/checks/` files present at S9 in `_re_search_runtime/` for Solve bundles, and include optional `notebook.ipynb` / check manifest entries.
   - Update `crates/export/src/replay_checks.py` (130 lines): replay Solve handle-based receipts and re-run recorded `solve.check` items comparing `evidence_sha256` (R19), while keeping every existing Research ZIP replay test untouched and green.

## 4. Owned files

- **Sub-lane A (S10):**
  - New: `python/checks/physics.py`, `python/tests/test_checks_physics.py`
  - Small edits: `python/checks/__init__.py`, `crates/research-tools/src/solve_checks.rs`
- **Sub-lane B (S19):**
  - New: `crates/desktop-server/src/routes_export.rs` (split from `routes_tools.rs`), `crates/export/src/notebook.rs` (+ `notebook_tests.rs` if needed to stay `<= 600` lines)
  - Small edits (`<= 600` lines after S19a): `crates/desktop-server/src/routes_tools.rs` (610 -> `< 560`), `crates/desktop-server/src/lib.rs`, F5 `api_dispatch.rs` (if re-pointing `handle_export`), `crates/export/src/lib.rs` (132), `crates/export/src/zip.rs` (341), `crates/export/src/replay_checks.py` (130)

## 5. Shared registration seams & Do-not-edit files

- **Seams:**
  - `GET /api/sessions/{id}/export?format=notebook`: handled cleanly by extracted `routes_export.rs` under existing `GET` auth/CSP rules (`auth.rs:208`, `desktop-app/src/rules.rs:57-80`).
  - S10 `physics.py` inclusion in `zip.rs`: if S10 and S19 run in parallel, S19 embeds the `python/checks/` files present at S9 (`__init__.py`, `srepr_safe.py`, `algebra.py`) and adds a 1-line Merge note to include `physics.py` once S10 merges.
- **Do NOT edit:** `crates/export/src/latex.rs` (771 lines, over limit), `crates/export/src/writeup*.rs` (owned by F15 core / Group 5 `writeup_solve.rs`), `crates/desktop-server/src/http_srv.rs` (702 lines), `crates/research-tools/src/tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 6. Tests through the entry point

- **S10 (`python3 -m unittest discover -s python` + `cargo test -p research-tools`):**
  - Unit tests for all 6 physics checks (known physical formulas passing, failing with witnesses, assumption-mismatch -> `inconclusive`, malformed params -> `inconclusive`).
  - Rust integration test through `check_run` + `compute_all_tiers`: passing all 6 physics checks leaves tier `Unchecked` with sublabels (R8); failing `dimensions` on a symbolically `Verified` step flips it to `Failed` permanently (R8).
- **S19 (`RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` + `cargo test -p desktop-server`):**
  - Valid nbformat 4.5 schema (`nbformat == 4`, `nbformat_minor == 5`, deterministic 8-hex `id`s, reproducible bytes).
  - Execute exported notebook's prelude + code cells in plain Python/SymPy (no re:Search imports) and assert zero errors and matching outputs.
  - Closure-only default (D18) vs `include_all_runs: true`.
  - `GET /api/sessions/{id}/export?format=notebook` returns 200 with `application/x-ipynb+json; charset=utf-8` behind `AuthGuard::authorize_api`.
  - Rule R22 sweep across every exported Markdown cell: `"verified"` ONLY tied to `own == Verified` (badge `"Verified"` only when `effective == Verified`), never `"Computed"`, never `"proved"` on a computed step.
  - ZIP export + `run_all_checks.py --verify-only` and live replay pass for BOTH a synthetic Solve session and all existing Research ZIP tests.
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero network/model calls, zero third-party paper text.

## 7. Document conflicts & resolutions

1. **`solve.md` 5.6 vs `00-overview.md` 2.13 (F15):** Readable TeX/Markdown derivation belongs to **F15** (`export/src/writeup_solve.rs`, Group 5), NOT S19. S19 delivers only machine-readable Notebook export (`nbformat 4.5`) and ZIP replay updates.
2. **Base `routes_tools.rs` (610 lines) & `ExportFormat` match (`routes_tools.rs:579`) + `export/src/lib.rs:86,117` (`latex.rs`, 771 lines):** Adding `ExportFormat::Notebook` fails to compile without updating `handle_export` inside 610-line `routes_tools.rs`, and `lib.rs` currently routes all formats through 771-line Research `latex.rs`. **Resolution:** S19a splits `handle_export` into `routes_export.rs` first (`GET /api/sessions/{id}/export?format=notebook` with `application/x-ipynb+json; charset=utf-8`), and S19c dispatches Solve Notebook/ZIP without touching `latex.rs`.
3. **`solve.md` 11 ("Get Physics Done" licence unconfirmed):** Zero code or text may be copied; all S10 checks and fixtures are clean-room original code.
4. **S2 `NON_IDENTITY_CHECKS` (`solve_own_tier.rs:16`) vs `solve.md` 4.4:** Verified exact match for all 6 S10 check names.

## 8. Out of scope & hand-back

- **Out of scope:** `export/src/writeup_solve.rs` (Group 5 / F15), `export/src/latex.rs` refactoring, SciPy/matplotlib wheels (S18 — S10 uses only SymPy/NumPy/mpmath), UI buttons (S13), Lean/S16 (cut), Cadabra2/FORM/S22 (deferred).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s10-s19-checks-export.md` (with separate sections for S10 and S19, `routes_tools.rs` before/after line counts, any Windows-only `stage.rs:19` note for `physics.py`, and the 1-line `physics.py` ZIP hook if run in parallel) and push only to `gemini/modes/solve-s10-s19-checks-export` (or per-sublane branches if dispatched separately).
