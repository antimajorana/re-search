# Brief draft-solve-s18-scipy-artifacts — SciPy, matplotlib, artifacts & worker memory cap (S18)

**Draft status:** Planning brief — **BLOCKED ON OWNER DEPENDENCY/WHEEL APPROVAL & WINDOWS VERIFICATION** for its Windows installer/runtime half (Part B); the platform-neutral artifacts & UI half (Part A) may proceed after S7 merges once the split is confirmed.
**Package:** S18 (SciPy & matplotlib bundled wheels, `stage.rs` allowlists, Agg backend in scratch, artifact harvesting & `solve.artifact`, standalone `solve_artifacts_route.rs`, Artifacts tab via S13 tab hook, read-only user data file staging, Windows Job Object memory cap, Windows installer smoke test).
**Governing design:** `00-overview.md` 0.1 ("Skip: numerics in Solve ahead of Lean" — S18 stays as planned; SciPy results are `"Checked numerically"`, never `"Verified"`), 8.1, 8.3 (q11); `solve.md` 3 (Artifacts tab), 4.1 (`solve.artifact` v1), 4.3 (job memory cap), 4.4 (rule 3: every SciPy result is at most `Checked numerically`), 5.6, 6 (R8, R18, R20, R22), 7.2–7.3, 9 (row S18), 10 (free core offline), 12 (risks 10, 11), 13 (D1, D7); `agents/RULES.md`.
**Read reports first:** `reports/s2-solve-model.md`, and (once merged) `reports/q-f11-x1-s3-s6.md`, `reports/f5-f7-mode-shell.md`, `reports/solve-s4-s5-setup.md`, `reports/solve-s7-s8-s9-s11.md`.

## 1. Start conditions, owner approval & Windows gates

- **Required merges on `origin/gemini/work/fixed` before S18 Part A starts:**
  - Already merged on base `832eab5bf`: F0, S2 (`SolveArtifact`, `event_kinds::SOLVE_ARTIFACT` already in `solve_model_records.rs` & folded into `SolveState.artifacts`).
  - **Hard merge gates:** `q-f11-x1-s3-s6` (S6 worker protocol 2), `f5-f7-mode-shell` (F5 `api_dispatch.rs`), and `solve-s7-s8-s9-s11` up through **S7** (`solve_exprs.rs` harvesting pattern & session scratch conventions).
  - **For live UI mounting:** `solve-s4-s5-setup` (S4 `solve_view.rs` already exposes `SolveState.artifacts`) and `solve-s13-s17-ui-intake` (S13 `registerSolveTab` hook; `solve_artifacts.js` can still be built and Node-tested standalone right after S7).
- **MANDATORY OWNER APPROVAL GATES (before Part B stages any new wheels):**
  1. **Bundled Python wheels & transitive dependencies (`RULES.md` "No new dependency" + `solve.md` 12 risk 11, D7):** Adding SciPy and matplotlib to `packaging/windows/stage_runtime.py` pulls in native + pure-Python wheels for CPython `3.14.10` (`win_amd64`), including matplotlib's runtime deps (`contourpy`, `cycler`, `fonttools`, `kiwisolver`, `packaging`, `pillow`, `pyparsing`, `python-dateutil`, `six` — exact cp314 wheel availability and licences MUST be verified on Windows first) and increases the NSIS installer by tens of MB. Owner MUST approve the exact SHA-256-pinned wheel manifest, installer size delta, and `THIRD-PARTY.txt` licence additions before merging Part B!
  2. **Worker Job Object memory cap value (`crates/worker/src/win.rs`):** Confirm the per-job memory limit (e.g. 1.5–2 GiB per worker job so up to 3 Python processes stay safe on the 14.8 GB reference laptop; `solve.md` 4.3, 12 risk 10).
- **LINUX CI LIMITATION (EXPLICIT):** `crates/worker/src/win.rs`, `crates/worker/src/stage.rs`, `packaging/windows/stage_runtime.py` (nt smoke branch), and `packaging/windows/smoke-install.ps1` are **Windows-only** (`#[cfg(windows)]` / PowerShell / AppContainer). A Linux agent **cannot** compile `win.rs` / `stage.rs` or test SciPy/matplotlib DLL loading and font-cache writes inside Windows AppContainer! Split execution cleanly into Linux-verifiable Part A and Windows-verified Part B.

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- S2 (`solve_model_records.rs` & `solve_state.rs`): `event_kinds::SOLVE_ARTIFACT` (`"solve.artifact"`), `SolveArtifact { artifact_id, name, mime, blob, sha256, bytes, call_id, entry_id }`, already folded into `SolveState.artifacts`.
- `crates/worker/src/win.rs` (276 lines, `#[cfg(windows)]`): `create_kill_on_close_job()` at lines 142–163 currently sets ONLY `info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;` (line 151) with **no memory cap**.
- `crates/worker/src/stage.rs` (**475 lines on base**, `#[cfg(windows)]` — **check post-S6 line count before editing; split into a helper module FIRST if `> 560` lines!**):
  - Line 17: `ALLOWED_PACKAGES = &["sympy", "mpmath", "numpy", "numpy.libs"];`
  - Line 18: `ALLOWED_DIST_PREFIXES = &["sympy-", "mpmath-", "numpy-"];`
  - Line 19: `WORKER_HELPERS = &["solver_calc.py", "symbolic_validation.py", "numerical_validation.py"];`
- `crates/worker/src/lib.rs` (336 lines): `WorkerConfig.scratch` doc comment (line 37) already notes `"Working directory of the worker; TEMP and the matplotlib config are redirected here."`
- `packaging/windows/stage_runtime.py` (166 lines): pins `PYTHON_VERSION = "3.14.10"`, `PACKAGES`, `ARCHIVES` with SHA-256 digests, `target_member`, `THIRD-PARTY.txt`, `required`, and `SMOKE`.
- `packaging/windows/smoke-install.ps1` (256 lines): line 220 runs `& $python -c "import sys, sympy, mpmath, numpy; assert sys.maxsize > 2**32"` followed by `appcontainer_sandbox`.
- `crates/research-tools/src/sandbox.rs`: existing sandbox path/symlink/junction checks.

**Future-merge interfaces to verify before coding:**
- From **S4:** `solve_view.rs` already exposes `SolveState.artifacts` in `GET /api/sessions/{id}/solve/view` (if not yet present when Part A runs before S4, record the 1-line `artifacts` field in Merge notes).
- From **S6 / S7:** `python/worker.py` protocol 2 environment setup, `solve_exprs.rs` post-run harvesting lifecycle, and exact post-S6 line count of `crates/worker/src/stage.rs` (split first if `> 560` lines!).
- From **S13:** `registerSolveTab` hook in `solve_state.js`/`solve_steps.js` so `solve_artifacts.js` registers its `Artifacts` tab cleanly without editing S13 files!

## 3. Smallest implementation checkpoints

### Part A — Platform-neutral (compilable & testable on Linux right after S7; zero collision with `solve.rs`!)
1. **Checkpoint S18a (`crates/research-tools/src/solve_artifacts.rs` & `crates/desktop-server/src/solve_artifacts_route.rs`):**
   - Harvest files created under `scratch/artifacts/` after a `sympy_eval` run:
     - Hard caps: at most **16 files** and **20 MiB total** per run (`solve.md` 5.6); oversized/excess files refused with clear diagnostic in receipt.
     - Path safety: refuse `..`, absolute paths, symlinks, and Windows junctions (reusing `sandbox.rs` helpers).
     - MIME detection strictly by **magic bytes** (never trusting filename extensions!): allowlist PNG (`\x89PNG`), PDF (`%PDF-`), valid UTF-8 JSON/CSV/plain text, and download-only SVG; reject executables (`MZ`, `\x7fELF`), archives, and HTML/JS!
     - Store bytes as ledger blobs (`put_blob`) and append host-only `solve.artifact` v1 `{ artifact_id, name, mime, blob, sha256, bytes, call_id, entry_id? }`.
   - Read-only staging helper copying user-attached data files (CSV/JSON/NPZ) into `scratch/inputs/` read-only so solver code can read data without mutating the user's originals.
   - **Standalone route file `crates/desktop-server/src/solve_artifacts_route.rs`** (NEVER editing `solve.rs`, eliminating any conflict with S8/S9/S17!): implements `GET /api/sessions/{id}/solve/artifacts/{artifact_id}` (registered via 1-line arm in F5 `api_dispatch.rs` behind `AuthGuard::authorize_api`), serving blobs with `Content-Type` from stored `mime`, `X-Content-Type-Options: nosniff`, and `Content-Disposition: attachment` for SVG/HTML-capable types (zero stored XSS!).
2. **Checkpoint S18b (`python/worker.py` lazy Agg/SciPy config & `assets/solve_artifacts.js`):**
   - In `python/worker.py`: set `MPLBACKEND=Agg` and `MPLCONFIGDIR=<scratch>/mplconfig` lazily (never importing SciPy or matplotlib at startup so basic SymPy runs pay zero import/memory overhead!).
   - `crates/desktop-server/assets/solve_artifacts.js` (+ `tests/solve_artifacts.test.mjs`): Artifacts tab rendering artifact cards (name, MIME, size, linked step/call id, PNG preview via authenticated blob URL, safe download button for CSV/JSON/PDF/SVG — NEVER inline `innerHTML` or inline `<svg>`!), mounted via S13's `registerSolveTab` hook once S13 merges. Register `/assets/solve_artifacts.js` in `crates/desktop-server/src/assets.rs`.

### Part B — Windows-only (requires owner wheel approval + Windows build & CI verification)
3. **Checkpoint S18c (`crates/worker/src/win.rs` — Job Object memory cap):**
   - Extend `create_kill_on_close_job()` (`win.rs:151`) to set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY` and `info.JobMemoryLimit = cap_bytes` (configurable via `WorkerConfig`, default confirmed with owner for 14.8 GB reference machine).
   - Add Windows-only integration test in `crates/worker/tests/` proving a Python worker allocating past the job memory cap is terminated cleanly by Windows without crashing the host or leaking processes.
4. **Checkpoint S18d (`stage.rs`, `packaging/windows/stage_runtime.py`, `smoke-install.ps1` — SciPy & matplotlib runtime):**
   - Verify cp314 `win_amd64` wheel availability, DLL loading (`scipy.libs`, OpenBLAS/GFortran DLLs, Pillow/contourpy/kiwisolver), and licences on Windows; obtain owner sign-off on the exact SHA-256-pinned `ARCHIVES` / `PACKAGES` / `THIRD-PARTY.txt` diff.
   - Check post-S6 line count of `crates/worker/src/stage.rs` (475 on base); **split into a helper module in a zero-behaviour-change commit first if `> 560` lines!** Then update `ALLOWED_PACKAGES` & `ALLOWED_DIST_PREFIXES`.
   - Update `packaging/windows/stage_runtime.py`: pin wheels + SHA-256, update `target_member` (keeping DLLs + upstream licence notices, stripping test suites), `THIRD-PARTY.txt`, `required`, and `SMOKE` (`scipy.integrate.quad`, `matplotlib.pyplot` Agg PNG output to temp dir).
   - Update `packaging/windows/smoke-install.ps1` (line 220 + AppContainer test): verify `import scipy, matplotlib`, `scipy.integrate.quad`, and matplotlib Agg PNG rendering + font-cache write inside AppContainer scratch with zero network!

## 4. Owned files

- **Platform-neutral (Part A — Linux-verifiable):**
  - New: `crates/research-tools/src/solve_artifacts.rs`, `solve_artifacts_tests.rs`, `crates/desktop-server/src/solve_artifacts_route.rs`, `crates/desktop-server/assets/solve_artifacts.js`, `crates/desktop-server/tests/solve_artifacts.test.mjs`
  - Small edits (`<= 600` lines): `crates/research-tools/src/lib.rs` (300), `crates/desktop-server/src/lib.rs`, F5 `api_dispatch.rs` (+1 `/solve/artifacts/..` arm), `crates/desktop-server/src/assets.rs` (365), `python/worker.py` (lazy `MPLBACKEND`/`MPLCONFIGDIR`)
- **Windows-only (Part B — Windows-verifiable):**
  - Small edits (`<= 600` lines; split `stage.rs` first if `> 560` post-S6): `crates/worker/src/win.rs` (276), `crates/worker/src/stage.rs` (475 on base), `packaging/windows/stage_runtime.py` (166), `packaging/windows/smoke-install.ps1` (256), plus Windows test in `crates/worker/tests/`

## 5. Shared registration seams & Do-not-edit files

- **Seams:**
  - `solve_artifacts_route.rs` registers via 1 line in F5 `api_dispatch.rs` — zero edits to `solve.rs`!
  - `solve_artifacts.js` registers via `assets.rs` and mounts via S13's `registerSolveTab` hook — zero edits to S13 files!
- **Do NOT edit:** `Cargo.lock` (zero new Rust crate deps!), `crates/desktop-server/src/solve.rs` (owned by S4/S8/S9/S17), `crates/research-tools/src/tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `crates/desktop-server/src/http_srv.rs` (702), `crates/desktop-server/assets/app.js` (932), `crates/desktop-server/assets/canvas_dag.js` (688).

## 6. Tests through the entry point

- **Linux-runnable tests (`cargo test`, `python3 -m unittest`, `node --test`):**
  - Artifact harvesting: valid PNG/CSV/JSON harvested into blobs + `solve.artifact`; magic-byte mismatch (`.png` containing `<html>` or `MZ`) refused; `> 16` files or `> 20 MiB` refused; symlink / `..` escape refused; read-only user input copy never modifies original file.
  - Artifact HTTP route (`solve_artifacts_route.rs`): requires `AuthGuard::authorize_api`, returns stored `mime`, sets `X-Content-Type-Options: nosniff`, forces `Content-Disposition: attachment` on SVG/non-raster types.
  - Python worker test: `MPLBACKEND == "Agg"`, `MPLCONFIGDIR` inside `scratch`, SciPy/matplotlib NOT imported until user code imports them.
  - Node UI test (`solve_artifacts.test.mjs`): renders PNG safely, forces download for SVG/CSV/PDF, zero `innerHTML` writes, XSS payloads in artifact names escaped.
  - **Rule R8 reminder test:** a SciPy quadrature / ODE numerical value check produces at most `CheckedNumerically` (two-run agreement), NEVER `Verified` (`00-overview.md` 0.1, `solve.md` 4.4).
- **Windows-only tests (MUST run on Windows CI before declaring Part B done):**
  - `cargo test --locked -p worker` including Job Object memory-cap enforcement test and `appcontainer_sandbox` (SciPy DLL load + matplotlib Agg PNG write inside AppContainer).
  - `python packaging/windows/stage_runtime.py --output ...` (with owner-approved wheel hashes) + `packaging/windows/smoke-install.ps1`.
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero network calls in unit tests.

## 7. Document conflicts & resolutions

1. **`00-overview.md` 0.1 ("Skip: numerics in Solve ahead of Lean" + `"Verified"` vs `"Checked numerically"`) vs `solve.md` D7 & 4.4:** Nothing extra is added ahead of Lean (Lean is cut), and S18 stays as originally scheduled. Every SciPy result gives at most **`Checked numerically`**, NEVER **`Verified`** (`00-overview.md` 8.3 q11, `solve.md` 4.4).
2. **`RULES.md` ("No new dependency") vs `solve.md` D7 & 12 risk 11 (bundling SciPy & matplotlib):** Zero new Rust dependencies (`Cargo.lock` untouched). Bundled Python wheels in `stage_runtime.py` require explicit owner approval of the verified cp314 `win_amd64` wheel manifest + licences + Windows AppContainer smoke pass before merge.
3. **File-lock collision avoidance (`solve.rs`):** Putting S18's artifact GET endpoint in standalone `solve_artifacts_route.rs` prevents S18 Part A from colliding with S8/S9/S17 on `solve.rs`.

## 8. Out of scope & hand-back

- **Out of scope:** Cadabra2 / FORM / `solve/2` (deferred, S22), Lean (cut, S16), adding new tools to `solve/1`, running SciPy checks in the symbolic check process as `"Verified"`.
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s18-scipy-artifacts.md` — explicitly separating Linux-verified Part A results from Windows-only Part B status (never claiming Windows AppContainer or cp314 wheels work until measured on Windows!), listing the exact pinned wheel table for owner sign-off, and pushing only to `gemini/modes/solve-s18-scipy-artifacts`.
