# Report: `solve-s18a-artifacts` — Solve S18 Part A (Scratch Artifacts, Route, Lazy Plotting, UI)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s18a-artifacts.md`
- **Base commit:** `374744ebf426f2081891faeaabb19a91e34a9158` (`work/lane-solve-s18a`, PR #46)
- **Work branch:** `gemini/modes/solve-s18a-artifacts` -> pushed to `origin/gemini/modes/solve-s18a-artifacts`
- **Toolchains verified (WSL2 Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Node `v24.22.2`, `Python 3.12.3`
- **Unchanged guardrails:**
  - `Cargo.lock` untouched (0 new dependencies).
  - Windows Part B files (`packaging/windows/stage_runtime.py`, `packaging/windows/smoke-install.ps1`, `crates/worker/src/{win,stage}.rs`) untouched.
  - Frozen / out-of-scope files untouched: `tool_profile.rs`, `tool_profiles.txt`, `tool_defs.txt`, `runner.rs`, `resume.rs`, `solve.rs`, `solve_setup.js`, `solve_exprs.rs`, `solve_{receipts,steps,checks,done,gates}.rs`, `solve_routes_ext.rs`, `investigation_*`, `routers.rs`, `cli/src/runtime.rs`.
  - Every modified/created source file stays strictly `<= 600` lines (`py -3 scripts/check_file_length.py --base 374744e` passes; max touched file is `assets.rs` at `554` lines and `solve_artifacts.rs` at `549` lines).

---

## 1. What was implemented (S18 Part A)

### Checkpoint 1 (`crates/research-tools`, commit `1aa2318`)

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/research-tools/src/solve_artifacts_sniff.rs` | 198 | `1aa2318` | Pure magic-byte MIME sniffer (`sniff_artifact_mime`) + constants `MIME_PNG`, `MIME_PDF`, `MIME_SVG`, `MIME_JSON`, `MIME_CSV`, `MIME_PLAIN`. Rejects binaries, HTML/XML non-SVG, scripts, PE/ELF/ZIP/archives, empty payloads, and spoofed extensions. |
| `crates/research-tools/src/solve_artifacts.rs` | 549 | `1aa2318` | `validate_artifact_rel_path`, `harvest_scratch_artifacts` scanning `scratch_dir/artifacts/` (`16` files / `20 MiB` per-run caps, symlink/junction & traversal guards via `crate::sandbox`, ledger `put_blob` + `solve.artifact` v1 via head-pinned `append_at_head`, idempotent SHA-256 skip), and `stage_readonly_inputs` copying user inputs read-only into `scratch_dir/inputs/`. |
| `crates/research-tools/src/solve_artifacts_tests.rs` | 239 | `1aa2318` | 5 unit tests covering magic-byte allow/reject rules, spoofed extensions, ledger blob roundtrip, idempotent re-harvest, 16-file and 20 MiB caps, symlink/traversal rejection, and read-only input staging/re-staging. |
| `crates/research-tools/src/lib.rs` | 344 | `1aa2318` | Declared `pub mod solve_artifacts;` and `mod solve_artifacts_sniff;`. |

### Checkpoint 2 (`desktop-server`, `python`, `assets`, commits `c3aee09`, `ff97b2d`, `6c814c5`, `ad67dcd`)

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/desktop-server/src/solve_artifacts_route.rs` | 172 | `c3aee09` | Standalone `read_solve_artifact` and `serve_solve_artifact` for `GET /api/sessions/{id}/solve/artifacts/{artifact_id}`: validates `session_id` + `artifact_id`, verifies Solve ledger (`Mode::Solve`; sessions without ledger events fail closed with `400`), verifies `blob == sha256`, byte length, `sha256_hex`, and magic-byte compatibility, forces `Content-Disposition: attachment` for SVG/PDF/JSON/CSV/text and `inline` strictly for PNG, sets `Cache-Control: no-store`, `Content-Security-Policy: sandbox`, and `X-Content-Type-Options: nosniff` (via `write_bytes`). |
| `crates/desktop-server/tests/solve_artifacts_route.rs` | 205 | `c3aee09` | Integration test verifying 401 on unauthenticated and cookie-only requests, 200 Bearer/Header auth across PNG/SVG/CSV/JSON/PDF/text with exact MIME, `nosniff`, `sandbox` CSP, and `inline` vs `attachment` headers, 400 on non-Solve session or `..`, 404 on missing artifact, 400 on spoofed HTML-as-PNG, and 400 on corrupted blob. |
| `crates/desktop-server/src/lib.rs` | 243 | `ff97b2d` | Declared `pub mod solve_artifacts_route;`. |
| `crates/desktop-server/src/api_dispatch.rs` | 439 | `ff97b2d` | Added `("GET", ["api", "sessions", id, "solve", "artifacts", artifact_id])` arm before catch-all `solve` dispatch. |
| `python/worker.py` | 498 | `6c814c5` | Added `configure_matplotlib_env` and scoped one-shot `_LazyMatplotlibEnvFinder` on `sys.meta_path` inside `evaluate(..., scratch_dir=None)` so `MPLBACKEND=Agg` and `MPLCONFIGDIR=<scratch>/mplconfig` are configured lazily only if `matplotlib` or `pylab` is imported; zero eager `scipy` or `matplotlib` imports on startup or pure SymPy runs. |
| `python/tests/__init__.py` | 1 | `6c814c5` | Package marker enabling `python3 -m unittest discover -s python` and `discover -s python/tests`. |
| `python/tests/test_worker_artifacts.py` | 207 | `6c814c5` | 5 tests verifying clean worker startup (`solver` and `check` roles), zero `scipy`/`matplotlib` imports or `mplconfig` dir creation on pure SymPy `evaluate`, lazy `Agg` + `mplconfig` setup on `--stdio` `matplotlib.pyplot.savefig` (valid PNG magic), direct `importlib.import_module("matplotlib")`, and lazy `scipy.integrate.quad` execution. |
| `crates/desktop-server/assets/solve_artifacts.js` | 368 | `ad67dcd` | Zero-HTML-injection, zero-`<svg>` module exporting `SOLVE_ARTIFACTS_TAB`, `registerSolveTab`, `artifactDownloadUrl`, `formatArtifactBytes`, `isPreviewableImageMime`, `mimeShortLabel`, `safeDownloadName`, `renderSolveArtifacts`, and `mountSolveArtifactsSection`. PNG previews use authenticated `fetch` (`X-ReSearch-Token`) into tracked blob URLs; SVG/PDF/CSV/JSON/text are download-only; all object URLs are revoked on re-render/destroy. |
| `crates/desktop-server/assets/mode_stages.js` | 273 | `ad67dcd` | Registered `SOLVE_ARTIFACTS_TAB`, mounted `mountSolveArtifactsSection` inside `#solve-session-stage` below `solve_setup.js` using `viewData.artifacts`, updated on setup submit if view updates, and cleaned up (`destroy()`) on stage switch/reset. |
| `crates/desktop-server/assets/solve.css` | 429 | `ad67dcd` | Added `.solve-artifacts-*` grid, card, badge, preview, and download action styles. |
| `crates/desktop-server/src/assets.rs` | 554 | `ad67dcd` | Registered `"/assets/solve_artifacts.js"` (`application/javascript; charset=utf-8`). |
| `crates/desktop-server/tests/solve_artifacts.test.mjs` | 433 | `ad67dcd` | 5 Node tests verifying static XSS/no-`<svg>`/no-`"Verified"` checks, `assets.rs` registration, `registerSolveTab`, authenticated PNG blob preview vs download-only SVG/CSV/JSON/PDF, XSS-payload escaping in filenames, blob URL revocation, and `mode_stages.js` mount/teardown lifecycle. |

---

## 2. Architecture decisions & deviations

1. **Private `solve_artifacts_sniff.rs` module split:**
   - Kept `solve_artifacts.rs` (`549` lines) comfortably below the 600-line ceiling while re-exporting `sniff_artifact_mime` and `MIME_*` constants at `research_tools::solve_artifacts::*`.
2. **Standalone `solve_artifacts_route.rs` + `mode_stages.js` mount (zero edits to `solve.rs` and `solve_setup.js`):**
   - `solve_setup.js` (`596` lines) and `solve.rs` were kept completely untouched per lane guardrails. Artifact serving lives in `solve_artifacts_route.rs` with a single dispatch arm in `api_dispatch.rs`, and the UI gallery mounts cleanly via `mode_stages.js` alongside a generic `registerSolveTab` hook ready for future Solve tabs (S12).
3. **Existing sessions without ledger events fail closed on artifact reads:**
   - Newly created `research` sessions persist `meta.json` (`session_exists == true`) without writing a ledger file yet (`UnknownSession` from `read_verified_session`). `read_solve_artifact` treats `UnknownSession` after `session_exists == true` as empty events -> `Mode::Research` != `Mode::Solve`, returning `400` ("This endpoint is only available for Solve sessions.") while nonexistent sessions still return `404`.
4. **Scoped `sys.meta_path` hook instead of patching global `builtins.__import__`:**
   - `_LazyMatplotlibEnvFinder` stays installed only inside `evaluate()`'s `try/finally` block, triggers `configure_matplotlib_env` before `matplotlib`/`pylab` executes module top-level code, respects an explicit `scratch_dir` or existing `Worker`-provided `MPLCONFIGDIR` (falling back to `Path.cwd() / "mplconfig"` in `--stdio`), and keeps `scipy`/`matplotlib` completely unloaded otherwise.
5. **Plain HTML MIME badges (`<span>`) instead of `ui_badge.js` `<svg>` icons:**
   - Ensures `solve_artifacts.js` and `mode_stages.js` introduce zero `<svg>` elements into the DOM.

---

## 3. Merge notes & cross-lane integration points

| Target location | Owner / Package | Integration action |
|---|---|---|
| `crates/desktop-server/src/routers.rs:105-119`, `crates/cli/src/runtime.rs:140-151` | Solve S12 / merge integration | Wire `research_tools::solve_artifacts::harvest_scratch_artifacts` after protocol-2 `sympy_eval` (and `stage_readonly_inputs` when staging user input files into worker `scratch/inputs/`). Left untouched here to avoid conflicting with parallel Solve lanes. |
| `crates/desktop-server/assets/solve_artifacts.js` (`SOLVE_ARTIFACTS_TAB`, `registerSolveTab`, `mountSolveArtifactsSection`) | Solve S12 | When S12 builds a tabbed Solve stage shell, pass its tab registry to `registerSolveTab` and mount `SOLVE_ARTIFACTS_TAB` inside the Artifacts tab. |
| `packaging/windows/stage_runtime.py`, `packaging/windows/smoke-install.ps1`, `crates/worker/src/{win,stage}.rs` | Solve S18 Part B (`solve-s18b-windows-runtime`) | Windows runtime wheel staging (`scipy`/`matplotlib`/font bundle) and Windows Job memory cap are out of scope in S18a and remain untouched. |
| `crates/ledger/src/verify.rs:288-308` (`verify_event_blobs`) | Future ledger update | `verify_event_blobs` currently scans `raw_blob`, `tool.request.args.blob`, and `tool.result.{result,error}.blob`, not `solve.artifact.blob`, so `Ledger::verify_all()` counts harvested artifact blobs under `unreferenced_blobs` until `"solve.artifact"` is added (`read_solve_artifact` still verifies `blob == sha256`, byte count, SHA-256, and magic bytes on every read). |

---

## 4. Verification commands and exact results

All final verification commands ran cleanly in `lane-solve-s18a` after all code edits (`ad67dcd`):

| Check | Command | Outcome | Exact counts / notes |
|---|---|---|---|
| Line length guard | `py -3 scripts/check_file_length.py --base 374744e` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (all 16 touched code/test/asset files `<= 554` lines) |
| JS syntax check | `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** (exit 0) | `NODE_CHECK_OK` across all asset JS files |
| JS unit + DOM + XSS tests | `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** (exit 0) | **216 passed, 0 failed, 0 skipped** (includes 5 new tests in `solve_artifacts.test.mjs`) |
| Python full suite | `unset RE_SEARCH_PYTHON && python3 -m unittest discover -s python -v` | **PASS** (exit 0) | **38 passed, 0 failed** (`27.820s`) |
| Python S18a artifact tests | `unset RE_SEARCH_PYTHON && python3 -m unittest discover -s python/tests -v` | **PASS** (exit 0) | **5 passed, 0 failed** (`10.143s`) |
| Targeted route test | `/var/tmp/research-cargo.sh solve-s18a cargo test --locked -p desktop-server --test solve_artifacts_route -j 2` | **PASS** (exit 0) | **1 passed, 0 failed, 0 ignored** |
| Workspace Clippy | `/var/tmp/research-cargo.sh solve-s18a cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit 0) | 0 warnings / 0 errors |
| Full workspace Cargo tests | `/var/tmp/research-cargo.sh solve-s18a cargo test --workspace --locked --no-fail-fast -j 2` | **PASS** (exit 0) | **1139 passed, 0 failed, 2 ignored** (2 pre-existing ignored live-tier tests in `agent/tests/eval_harness.rs`; includes 316 `research-tools` unit tests w/ 5 `solve_artifacts_tests` + `desktop-server` `solve_artifacts_route` 1/1) |

---

## 5. What could not be compiled or run on Linux

- All `#[cfg(windows)]` Windows-only code paths across the workspace (`AppContainerSandbox`, `JobObjectSandbox`, `crates/worker/src/{win,appcontainer,stage}.rs`, Windows pipe launches in `desktop-server/src/routers.rs` and `cli/src/runtime.rs`, and Windows-only integration tests) and Windows packaging/smoke checks (`solve-s18b-windows-runtime`).
- Note on symlink/junction test guard: `solve_artifacts_tests.rs` runs symlink traversal tests on Unix; Windows NTFS junction reparse behavior requires verification in the Windows Part B run.
