# Brief solve-s19-export-replay — Solve Notebook Export & ZIP Replay (S19)

**Status:** Queued right behind **`S9`** (merged inside `solve-s8-s9-s11`). Zero `Cargo.lock` edits! File-disjoint from `solve-s10-physics-checks`, `solve-s18a-artifacts`, `solve-s18b-windows-runtime`, `solve-s12-loop`, and `f15-writeup-solve`.
**Push branch:** `gemini/modes/solve-s19-export-replay`

---

## 1. Package & Governing Design

- **Package:** **S19** — split `handle_export` out of `crates/desktop-server/src/routes_tools.rs` (`578`L) into `routes_export.rs` first, build deterministic nbformat 4.5 JSON exporter (`ExportFormat::Notebook`, `.ipynb`) in `crates/export/src/notebook.rs`, serve `GET /api/sessions/{id}/export?format=notebook`, and extend `crates/export/src/zip.rs` + `replay_checks.py` for Solve bundles and R19 check replay.
- **Governing design:** `00-overview.md` 0.1, 2.13 (boundary with F15 write-up), 7.3 (R22), 8.1–8.3; `solve.md` 4.4, 5.1, 5.6, 6 (R4, R5, R8, R19, R22), 7.2–7.3, 8.3, 9 (`S19`), 13 (D1, D18); `agents/RULES.md`.

---

## 2. Verified Interfaces on `gemini/work/fixed`

- **`crates/desktop-server/src/routes_tools.rs` (`578` lines):** Contains `handle_export` with exhaustive `match format`. Extract `handle_export` + helpers into standalone `crates/desktop-server/src/routes_export.rs` FIRST so `routes_tools.rs` drops well below 550 lines and never grows past 600L!
- **`crates/export/src/lib.rs` (`128` lines):** `ExportFormat { Tex, Html, Zip }`, `export_session_to_path`, `export_session_bytes`. Dispatch `ExportFormat::Notebook` and Solve ZIP via `Ledger::read_verified_session` + `SolveState::from_events` directly — NEVER invoke Research `latex::load_session_export_data` (`latex.rs` is `771` lines; do NOT edit it!).
- **Auth contract (`GET`):** Existing export endpoint is `GET /api/sessions/{id}/export?format=<fmt>`. Serve `.ipynb` with `Content-Type: application/x-ipynb+json; charset=utf-8` under existing `AuthGuard::authorize_api` / stream-cookie rules without editing `http_srv.rs` or `auth.rs`.

---

## 3. Checkpoints — Push After Each!

1. **Checkpoint S19a — MANDATORY ZERO-BEHAVIOUR-CHANGE SPLIT (`routes_tools.rs` 578L -> `< 530`L):**
   - Extract `handle_export` and private helpers out of `routes_tools.rs` into new `crates/desktop-server/src/routes_export.rs`, exported via `lib.rs` and wired via `api_dispatch.rs`. Run tests & `check_file_length.py` and push CP1 alone!
2. **Checkpoint S19b (`crates/export/src/notebook.rs` + `notebook_tests.rs`):**
   - nbformat 4.5 JSON (`nbformat: 4, nbformat_minor: 5`), default = closure of `done.final_entry_id` (`include_all_runs: bool` opt-in, default `false` per D18):
     - Cell 1 (Markdown): problem setup (statement, task kind, target, symbols + assumptions, conventions, dimensions).
     - Cell 2 (Code): self-contained SymPy prelude defining `expr(name_or_handle)` and `save_expr(name, obj)` with inlined `solve.expr` handle `srepr` table so the notebook runs in plain Python/SymPy outside re:Search.
     - Per step in `proof_closure::topological_order`: Markdown cell (step id, `badge_for_step`, `"own check: verified"` ONLY when `own == Verified && effective != Verified` per R22, flags, summary, LaTeX strictly from `solve.expr.latex` per R5), plus code cells for receipts and `solve.check` items.
     - Final Markdown cell: `solve.done.criteria`, status line, and `"rests on"` breakdown.
     - Deterministic cell IDs: first 8 lowercase hex chars of `sha256` (deterministic dedup on collision).
3. **Checkpoint S19c (`export/src/{lib,zip}.rs`, `replay_checks.py`, `routes_export.rs`, tests):**
   - Add `ExportFormat::Notebook` (`"ipynb" | "notebook"`) to `export/src/lib.rs`, wire `GET /api/sessions/{id}/export?format=notebook` in `routes_export.rs`.
   - Extend `zip.rs` to export Solve sessions (including `solve.*` ledger events, `_re_search_runtime/python/checks/*` present on repo, and optional `notebook.ipynb`) without breaking Research ZIP tests.
   - Extend `replay_checks.py` to replay Solve handle receipts and re-run `solve.check` items checking `evidence_sha256` parity (R19).
   - Add integration tests executing exported `.ipynb` cells in plain `python3` + SymPy, R22 wording sweep across all Markdown cells, `AuthGuard` test on `routes_export`, and Solve + Research ZIP replay tests (`RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export`).

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/desktop-server/src/routes_export.rs`, `crates/export/src/notebook.rs` (+ `notebook_tests.rs`), `crates/export/tests/solve_notebook_zip.rs`.
- **Small edits:** `crates/desktop-server/src/{lib,api_dispatch,routes_tools}.rs`, `crates/export/src/{lib,zip}.rs`, `crates/export/src/replay_checks.py`.
- **Do NOT edit:** `crates/export/src/latex.rs` (`771`L), `crates/export/src/writeup*.rs`, `crates/desktop-server/src/{http_srv,auth}.rs`, `python/checks/physics.py` (owned by `S10` — note 1-line ZIP inclusion in report if `S10` merges later), `tool_profile.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`.

---

## 5. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`)**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Write `docs/design/modes-2026-11/reports/solve-s19-export-replay.md` and push to `gemini/modes/solve-s19-export-replay`. End with `PUSHED`.
