# Brief solve-s18a-artifacts — Solve Numerical & Plot Artifact Harvest, Safe Serving Route, Lazy Agg Config & UI (S18 Part A)

**Status:** Unblocked on `gemini/work/fixed` (`F0`, `F5`, `S2`, `S3`, `S4`, `S5`, `S6`, `S7` are merged). Windows-only Part B (`cp314` SciPy/matplotlib wheel manifest in `packaging/windows/stage_runtime.py` + `crates/worker/src/{stage,win}.rs` Job Object memory cap) stays **gated on explicit owner approval** and is **out of scope** for this brief.
**Package:** `S18 Part A` (platform-neutral scratch artifact harvesting -> `solve.artifact`, read-only input file staging, authenticated safe artifact serving route `solve_artifacts_route.rs`, lazy `MPLBACKEND=Agg`/`MPLCONFIGDIR` in `python/worker.py`, and `solve_artifacts.js` UI).
**Governing design:**
- `00-overview.md`: 0.1, 2.10, 6, 8.1, 8.3 q11 (`"Numerically checked"` never `"Verified"` for SciPy/numeric results).
- `solve.md`: 3 (Artifacts tab), 4.1 (`solve.artifact` v1), 4.4 (rule R8: numeric results are at most `CheckedNumerically` / `"Numerically checked"`), 5.6 (16 files / 20 MiB per run caps, magic-byte sniffing, read-only user input staging).
- `agents/RULES.md` (mandatory workspace Clippy & test run, `<= 600`-line limit, no `Cargo.lock` edits, push after each checkpoint).

---

## 1. Verified Interfaces on `gemini/work/fixed`

- **S2 (`crates/research-tools/src/solve_model_records.rs` & `solve_state.rs`):** `event_kinds::SOLVE_ARTIFACT` (`"solve.artifact"`), `SolveArtifact { artifact_id, name, mime, blob, sha256, bytes, call_id, entry_id }`, already folded into `SolveState.artifacts`.
- **S4 (`crates/desktop-server/src/solve_view.rs` line 231 & `solve.rs` `328` lines):** `build_solve_view` ALREADY emits `"artifacts": state.artifacts`. `solve.rs` explicitly reserves `GET /api/sessions/{id}/solve/artifacts/{artifact_id}` in standalone `solve_artifacts_route.rs` so **`solve.rs` does NOT need any edit!**
- **S5 UI (`crates/desktop-server/assets/solve_setup.js` `596` lines; `mode_stages.js` `238` lines):** `solve_setup.js` has only `4` lines of headroom under `600`, so **do NOT edit `solve_setup.js`**; mount/hook `solve_artifacts.js` cleanly via `mode_stages.js` (or self-contained export + `mode_stages.js` tab/panel mount) and register `/assets/solve_artifacts.js` in `crates/desktop-server/src/assets.rs`.
- **S6 Worker (`python/worker.py` `462` lines):** protocol 2 JSON-lines server (`solver` vs `check` roles). Do **not** import `scipy` or `matplotlib` eagerly at startup.

---

## 2. Checkpoints — Push to `gemini/modes/solve-s18a-artifacts` After Each!

### Checkpoint 1 — Core Artifact Harvesting & Read-Only Input Staging (`crates/research-tools`)
1. Create `crates/research-tools/src/solve_artifacts.rs` (and `crates/research-tools/src/solve_artifacts_tests.rs` to stay well `<= 600` lines):
   - `harvest_scratch_artifacts(ledger, session_id, scratch_dir, call_id, entry_id) -> Result<HarvestReport, String>`:
     - Scans `scratch_dir/artifacts/` non-recursively (or safely bounded relative paths), refusing `..`, absolute paths, symlinks, and Windows junctions (reusing `crate::sandbox` helpers).
     - Enforces hard caps: **at most 16 files** and **20 MiB total** per run (`solve.md` 5.6); oversized/excess files are rejected into diagnostic warnings without crashing the run.
     - Detects MIME strictly by **magic bytes** (ignoring spoofed filename extensions!): allows PNG (`\x89PNG\r\n\x1a\n`), PDF (`%PDF-`), valid UTF-8 JSON/CSV/plain text, and download-only SVG (`image/svg+xml`); explicitly rejects executables (`MZ`, `\x7fELF`, Mach-O), ZIP/archives, HTML (`<!doctype html`, `<html`, `<script`), and binary garbage.
     - Writes verified bytes via `ledger.put_blob` and appends host-only `solve.artifact` v1 (`artifact_id`, `name`, `mime`, `blob`, `sha256`, `bytes`, `call_id`, `entry_id`).
   - `stage_readonly_inputs(scratch_dir, inputs: &[StagedInputSpec]) -> Result<Vec<StagedInputFile>, SandboxError>`:
     - Validates filenames cleanly, copies attached CSV/JSON/NPZ user data into `scratch_dir/inputs/` with read-only permissions (where supported), and leaves user originals untouched.
   - Add 1-line `pub mod solve_artifacts;` in `crates/research-tools/src/lib.rs` (`339`L) — do **not** edit `solve_exprs.rs` (list the 3-line post-run call site in Merge notes so there is zero conflict with `solve-s8-s9-s11`).
2. Run mandatory checks and push Checkpoint 1 to `gemini/modes/solve-s18a-artifacts`.

### Checkpoint 2 — Authenticated Route, Lazy Python Agg Setup, and `solve_artifacts.js` UI
1. Create `crates/desktop-server/src/solve_artifacts_route.rs` (+ `crates/desktop-server/tests/solve_artifacts_route.rs`):
   - Serves `GET /api/sessions/{id}/solve/artifacts/{artifact_id}` (session-validated, Solve-mode checked, blob integrity verified via `sha256_hex`), returning `Content-Type` from the ledger's sniffed `mime`, `X-Content-Type-Options: nosniff`, and forcing `Content-Disposition: attachment; filename="..."` on SVG / CSV / JSON / PDF (inline allowed ONLY for raster PNG; zero stored XSS!).
   - Register in a separate small commit: `crates/desktop-server/src/lib.rs` (`234`L) and 1 match arm in `crates/desktop-server/src/api_dispatch.rs` (`424`L) **before** the catch-all `["api", "sessions", id, "solve", rest @ ..]` arm (following `api_dispatch.rs`'s `write_bytes` pattern used by `/export` so headers and binary bytes write directly).
2. Update `python/worker.py` (`462`L, keeping `<= 600`L) + add `python/tests/test_worker_artifacts.py`:
   - Configure `MPLBACKEND=Agg` and `MPLCONFIGDIR=<scratch>/mplconfig` lazily inside `evaluate` only when needed (using `sys.modules` test proving neither `scipy` nor `matplotlib` is imported during worker startup or pure `sympy` eval!).
3. Create `crates/desktop-server/assets/solve_artifacts.js` (+ `crates/desktop-server/tests/solve_artifacts.test.mjs`):
   - Renders `State.artifacts` cards (escaped filename, MIME, byte size, `call_id`/`entry_id` badge, raster PNG preview via authenticated fetch/blob URL, safe download button for SVG/CSV/JSON/PDF — **zero `innerHTML`, zero inline `<svg>`**). Also exports `registerSolveTab` metadata AND mounts cleanly inside `crates/desktop-server/assets/mode_stages.js` (`238`L) below the Solve setup stage whenever `viewData.artifacts` is rendered (update/add test in `solve_artifacts.test.mjs` proving live mount + `node --test crates/desktop-server/tests/*.test.mjs` passes!).
   - Register `/assets/solve_artifacts.js` in `crates/desktop-server/src/assets.rs` (`530`L).
4. Re-run full mandatory checks (`check_file_length.py`, `node --check`, `node --test`, non-root Linux workspace Clippy & `cargo test --workspace --locked`, and `py -3 -m unittest discover -s python/tests`), write `docs/design/modes-2026-11/reports/solve-s18a-artifacts.md`, and push to `gemini/modes/solve-s18a-artifacts`.

---

## 3. Do-Not-Edit List (Strict Parallel Safety)

- **Do NOT edit:** `crates/worker/src/win.rs`, `crates/worker/src/stage.rs`, `packaging/windows/stage_runtime.py`, `packaging/windows/smoke-install.ps1` (Part B owner-gated!), `crates/desktop-server/src/solve.rs` (owned by `solve-s8-s9-s11`), `crates/research-tools/src/solve_exprs.rs`, `crates/desktop-server/assets/solve_setup.js` (`596`L), `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.

---

## 4. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
node --test crates/desktop-server/tests/*.test.mjs
python3 -m unittest discover -s python/tests
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Push **only** via `git push origin HEAD:refs/heads/gemini/modes/solve-s18a-artifacts`. Never squash, never open/merge PRs, and end with the `PUSHED` block from `RULES.md`.
