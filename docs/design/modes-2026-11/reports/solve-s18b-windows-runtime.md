# Solve S18b: Windows 2 GiB Job Cap + SciPy / Matplotlib Staged Runtime

Branch: `gemini/modes/solve-s18b-windows-runtime`
Base: `a36c11a` (`S18a`) + merges of `origin/gemini/work/fixed` through `43fd8f8` (PRs #51–#64; `59ff311` PR #65 touches docs only with zero code overlap)
Head (code verified): `4ffa33a`

## Summary of Changes

### 1. Win32 Job Object Memory Cap (`S18c`, commit `185b71d` + OpenBLAS fix `78378f0` / `c1fedeb`)
- Added `DEFAULT_JOB_MEMORY_LIMIT_BYTES` (`2 * 1024 * 1024 * 1024` = 2 GiB) and `WorkerConfig.job_memory_limit_bytes: Option<usize>` + `effective_job_memory_limit_bytes()` in `crates/worker/src/lib.rs`, propagated via `WorkerLaunch.job_memory_limit_bytes` (`crates/worker/src/sandbox.rs`) and `spawn_extended` (`crates/worker/src/win/launch.rs`).
- Updated `create_kill_on_close_job(job_memory_limit_bytes: Option<usize>)` in `crates/worker/src/win.rs` to validate `Some(0)` as `InvalidInput`, set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and attach `JOB_OBJECT_LIMIT_JOB_MEMORY` (`JobMemoryLimit`) when `Some(limit)`. Non-worker `spawn_in_job` continues passing `None`.
- Added unit test `kill_on_close_job_sets_optional_memory_limit` (`query_job_extended_limits`) and real Windows integration test `crates/worker/tests/job_memory_limit.rs` (`default_and_tight_job_memory_limits_enforce_cap_without_orphan_children`) proving:
  - Default `None` enforces the 2 GiB cap inside the kernel Job Object.
  - A tight `128 MiB` cap under both `JobObjectSandbox` and `AppContainerSandbox` rejects `256 MiB` allocations (`MemoryError` / nonzero child exit, `PrivateUsage <= 128 MiB`, kernel `PeakJobMemoryUsed >= 128 MiB`) and kills grandchild processes cleanly (`process_is_gone`) on shutdown.
- **OpenBLAS thread-arena fix (`78378f0`, `c1fedeb`):** `numpy 2.5.5` ships OpenBLAS, whose default per-core thread pool committed `410.1 MiB` on `worker.py` startup on this machine (causing intermittent `OpenBLAS error: Memory allocation still failed after 10 retries` inside `128 MiB` Job-capped workers). Setting `OPENBLAS_NUM_THREADS=1` and `OMP_NUM_THREADS=1` in both `Worker::spawn` (`crates/worker/src/lib.rs`) and `AppContainerSandbox::spawn_worker` (`crates/worker/src/appcontainer.rs`) drops startup `PeakJobMemoryUsed` from `410.1 MiB` to `57.0 MiB` (`-86%`).

### 2. SciPy + Matplotlib Windows Runtime Staging & Smoke Tests (`S18d`, commit `60c3758` + merge `2218d67`)
- Added optional `"physics.py"` in `STAGED_HELPER_FILES` (`crates/worker/src/protocol.rs`, `collect_staged_scripts` skips `ErrorKind::NotFound`) and reconciled S9/S10 `STAGED_CHECK_FILES` (`checks/algebra.py`, `checks/algebra_probe.py`, `checks/physics.py`, `checks/physics_helpers.py`, `checks/physics_matrix.py`).
- Extended `ALLOWED_PACKAGES` and `ALLOWED_DIST_PREFIXES` in `crates/worker/src/stage.rs` (`STAGE_VERSION = "stage-v5"`, `530` lines) for `scipy`, `scipy.libs`, `matplotlib`, `mpl_toolkits`, `pylab.py`, `contourpy`, `cycler`, `fonttools`, `kiwisolver`, `packaging`, `pil`, `pillow.libs`, `pyparsing`, `dateutil`, `tzdata`, and `six.py`, plus unit test `allowlist_matches_scipy_matplotlib_and_transitive_runtime_entries`.
- Pinned genuine downloaded PyPI `cp314` (`win_amd64`) / `py3-none-any` wheels, verified SHA-256 digests, full `THIRD-PARTY.txt` notices (including bundled OpenBLAS/LAPACK/GCC/Qhull/DOP/duccfft, DejaVu/STIX fonts, and Pillow imaging libraries), and offline `scipy.integrate.quad` + `matplotlib` Agg PNG render checks in `packaging/windows/stage_runtime.py` and `packaging/windows/smoke-install.ps1`. (SciPy outputs carry `"Numerically checked"` semantics, never `"Verified"`.)
- Added `contained_scipy_quad_and_matplotlib_agg_render_in_scratch` in `crates/worker/tests/appcontainer_sandbox.rs` verifying zero-network AppContainer `scipy.integrate.quad` execution, `matplotlib` Agg headless PNG rendering, and font-cache isolation under `MPLCONFIGDIR`.

## Pinned Windows x64 Runtime Archives (`packaging/windows/stage_runtime.py`)

| Package | Version | Archive | Bytes | Observed SHA-256 | License(s) |
|---|---|---|---:|---|---|
| `python` | `3.14.10` | `python.3.14.10.nupkg` | 15,589,216 | `f54ba67c9951c3d773ea47d2bb095150c037c399f0423a968bcda22809241784` | PSF + MS Distributable Code (`LICENSE.txt`) |
| `numpy` | `2.5.5` | `numpy-2.5.5-cp314-cp314-win_amd64.whl` | 12,710,543 | `20145b11776d2f7570cd61351b156a51d3a341822282095baa819d87272bc710` | BSD-3-Clause / 0BSD / MIT / Zlib / CC0-1.0 |
| `sympy` | `1.14.0` | `sympy-1.14.0-py3-none-any.whl` | 6,299,353 | `e091cc3e99d2141a0ba2847328f5479b05d94a6635cb96148ccb3f34671bd8f5` | BSD-3-Clause |
| `mpmath` | `1.3.0` | `mpmath-1.3.0-py3-none-any.whl` | 536,198 | `a0b2b9fe80bbcd81a6647ff13108738cfb482d481d826cc0e02f5b35e5c88d2c` | BSD-3-Clause |
| `scipy` | `1.18.1` | `scipy-1.18.1-cp314-cp314-win_amd64.whl` | 37,362,755 | `78a0d7c918e74a232394117160e7e3db503377572a45bcef8826e4ab8a35feba` | BSD-3-Clause (+ OpenBLAS/LAPACK/GCC/Qhull/DOP/duccfft) |
| `matplotlib` | `3.11.2` | `matplotlib-3.11.2-cp314-cp314-win_amd64.whl` | 9,505,972 | `ecea603dd2fbf8242fd31a305a8b12a4ece2de28096870c65fdd0d1e35b8d9a6` | PSF-based Matplotlib License (+ DejaVu/STIX fonts) |
| `contourpy` | `1.4.0` | `contourpy-1.4.0-cp314-cp314-win_amd64.whl` | 240,101 | `181bea01bc742734ae672fa00c717d855dd35e1f029536406538c52e7b0cd73d` | BSD-3-Clause |
| `cycler` | `0.12.1` | `cycler-0.12.1-py3-none-any.whl` | 8,321 | `85cef7cff222d8644161529808465972e51340599459b8ac3ccbac5a854e0d30` | BSD-3-Clause |
| `fonttools` | `4.69.1` | `fonttools-4.69.1-cp314-cp314-win_amd64.whl` | 2,974,412 | `84f2d9610b216bd13d2b9a2a26f0e7d31791f4564a7d5a8bc3048919ec2b678a` | MIT |
| `kiwisolver` | `1.5.1` | `kiwisolver-1.5.1-cp314-cp314-win_amd64.whl` | 72,282 | `f4167e87b397f273dc2356fcf1eaf50a6bac51e6105f45103ef7129c8efb0255` | BSD-3-Clause |
| `packaging` | `26.4` | `packaging-26.4-py3-none-any.whl` | 140,049 | `81383611a09c1bc0b08b76e0fd5829d7c6824c65b27a6aee339da133f4233ea2` | Apache-2.0 OR BSD-2-Clause |
| `pillow` | `13.0.0` | `pillow-13.0.0-cp314-cp314-win_amd64.whl` | 7,281,830 | `f7cbb7a64e7ec664775195e4f81923bc8a93d9451b963f844e59118f63147a7a` | MIT-CMU (+ libjpeg-turbo/libpng/zlib/FreeType/libtiff/WebP/Little CMS) |
| `pyparsing` | `3.3.3` | `pyparsing-3.3.3-py3-none-any.whl` | 126,420 | `ece8c00a69cf01b45d0b1dedabb469c90d8caf996d4fda40f147627a122849a4` | MIT |
| `python-dateutil` | `4.0.0` | `python_dateutil-4.0.0-py3-none-any.whl` | 92,853 | `a5e246f79eb80475e691bf93862cead98decd8a498cf53ba0516fcdc0f50dc1c` | Apache-2.0 / BSD-3-Clause |
| `tzdata` | `2026.6` | `tzdata-2026.6-py2.py3-none-any.whl` | 348,210 | `0a10eec6e13ef16b627fabed9a5f2ba61a62b1c67f327305f4e7e412a70b4aec` | Apache-2.0 |

**Staged runtime size delta:**
- Pre-S18d (`rt-s18c`, CPython 3.14.10 + NumPy + SymPy + mpmath): `86,254,409` B (`82.26 MiB`), `2,046` files
- Post-S18d (`rt-s18d`, freshly staged offline): `247,982,547` B (`236.49 MiB`), `5,220` files
- Delta: `+161,728,138` B (`+154.24 MiB`), `+3,174` files (within expected ~150–250 MiB SciPy+Matplotlib window; `.whl` `Tests` paths omitted while retaining `.dist-info` and `.libs` notices; `python-dateutil 4.0.0` requires `tzdata`, not `six`).

## Touched Files & Line Counts (`<= 600` lines; `stage.rs <= 550`)
- `crates/cli/src/runtime.rs`: 203 | `crates/cli/tests/contained_router.rs`: 178
- `crates/desktop-server/src/routers.rs`: 156 | `crates/desktop-server/src/solve_check_routes.rs`: 383 | `crates/desktop-server/src/solve_runtime.rs`: 322
- `crates/mcp-server/src/main.rs`: 332
- `crates/worker/src/appcontainer.rs`: 380 | `crates/worker/src/lib.rs`: 515 | `crates/worker/src/protocol.rs`: 429 | `crates/worker/src/sandbox.rs`: 144 | `crates/worker/src/stage.rs`: 530 | `crates/worker/src/win.rs`: 338 | `crates/worker/src/win/launch.rs`: 423
- `crates/worker/tests/appcontainer_sandbox.rs`: 555 | `crates/worker/tests/job_memory_limit.rs`: 216 | `crates/worker/tests/python_worker.rs`: 85
- `packaging/windows/smoke-install.ps1`: 265 | `packaging/windows/stage_runtime.py`: 291

## Verification (Commit `4ffa33a`)
- **Line-length & frozen-file guard:** `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` passed (`No touched source file is over 600 lines.`); `0` edits to `Cargo.toml`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`, `crates/research-tools/src/solve_artifacts.rs`, `crates/desktop-server/src/solve.rs`.
- **Windows 11 Clippy:** `cargo clippy --workspace --all-targets --locked -j 2 -- -D warnings -A clippy::manual_unwrap_or` — **passed (0 warnings, 0 errors)**.
- **Windows 11 `worker` crate (`RE_SEARCH_PYTHON=rt-s18d\python.exe`):** **24 passed, 0 failed** (`19` unit, `3` `appcontainer_sandbox`, `1` `job_memory_limit`, `1` `python_worker`).
- **Windows 11 full workspace (`RE_SEARCH_PYTHON` unset):** `164` suites: **1,296 passed, 10 failed, 2 ignored** (pre-existing `eval_harness.rs` ignores; all `10` failures are exclusively the real-Python SymPy tests failing due to missing `sympy` in bare system `py -3`).
- **Windows 11 Python-enabled Rust suites (`RE_SEARCH_PYTHON=rt-s18d\python.exe`):** `20` suites: **161 passed, 0 failed, 0 ignored** (`desktop-server --lib --test solve_intake_routes --test solve_loop_routes --test solve_s8_s9_routes --test solve_export_route`: `112/112`; `research-tools` `solve_s7_exprs`/`solve_s8_s9_s11`/`solve_s10_physics`/`solve_set_q_m3`: `11/11`; `cli --test contained_router`: `1/1`; `export`: `37/37`).
- **Staged CPython 3.14.10 Python tests (`rt-s18d\python.exe -I -B -m unittest discover -s python -v`, run at `2218d67`, zero subsequent Python edits):** **52 passed, 0 failed** (`71.544s`).
- **WSL2 Ubuntu non-root (`#[cfg(not(windows))]`, `RE_SEARCH_PYTHON` unset):**
  - `python3 -m unittest discover -s python`: **52 passed, 0 failed** (`36.462s`).
  - `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`: **passed (0 warnings, 0 errors)**.
  - `cargo test --workspace --locked --no-fail-fast -j 2`: `164` suites: **1,256 passed, 0 failed, 2 ignored** (`worker` unit `10/10`, `0` skipped/relaxed).

## Merge Notes
- Six external `WorkerConfig` struct-literal callers received 1–2 line `job_memory_limit_bytes: None` initializers (`crates/cli/src/runtime.rs`, `crates/cli/tests/contained_router.rs`, `crates/desktop-server/src/routers.rs`, `crates/desktop-server/src/solve_check_routes.rs`, `crates/desktop-server/src/solve_runtime.rs`, `crates/mcp-server/src/main.rs`). Any future PR adding another `WorkerConfig { ... }` literal will also need `job_memory_limit_bytes: None`.
- `packaging/windows/re-search.nsi` was left untouched (`RULES.md`/brief ownership); `smoke-install.ps1` expects a runtime staged with the updated `stage_runtime.py`.
- During Windows `--workspace` runs with an external `CARGO_TARGET_DIR`, keep `RE_SEARCH_PYTHON` unset for `cli/tests/desktop_and_ext.rs` (since `find_worker_script` searches relative to the test binary when `RE_SEARCH_PYTHON` is set without `RE_SEARCH_WORKER_SCRIPT`), then verify SymPy-backed suites with `RE_SEARCH_PYTHON` pointing to `rt-s18d\python.exe`.
