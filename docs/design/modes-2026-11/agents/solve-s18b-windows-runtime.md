# Brief solve-s18b-windows-runtime — Windows SciPy/Matplotlib Wheels & Job Object Memory Cap (S18 Part B)

**Status:** **LOCAL WINDOWS EXECUTION ONLY** (`#[cfg(windows)]` + AppContainer + NSIS installer smoke cannot compile or run on Linux remote agents). **Approved by Owner Decision (2026-11-26).** Starts after `solve-s18a-artifacts` merges (`PYTHON_VERSION = "3.14.10"` `win_amd64`).
**Push branch:** `gemini/modes/solve-s18b-windows-runtime`

---

## 1. Package & Governing Design

- **Package:** **S18 Part B** — (1) Win32 Job Object memory cap in `crates/worker/src/win.rs` (`252`L) on `create_kill_on_close_job()`, (2) allowlist SciPy, matplotlib, and transitive `cp314` `win_amd64` runtime packages in `crates/worker/src/stage.rs` (`424`L), (3) SHA-256-pinned wheel manifest, `target_member` filter (native DLLs + notices kept, test suites stripped), `THIRD-PARTY.txt` licences, and offline Agg/quadrature smoke in `packaging/windows/stage_runtime.py` (`166`L), and (4) `packaging/windows/smoke-install.ps1` (`256`L) AppContainer + font-cache smoke verification.
- **Governing design:** `00-overview.md` 0.1, 2.10, 8.3 q11 (`"Numerically checked"` NEVER `"Verified"` for SciPy); `solve.md` 4.3, 4.4 (R8), 5.6, 12 risks 10–11, D7; `agents/RULES.md` (zero Rust `Cargo.lock` changes!).

---

## 2. Verified Interfaces & Constraints

- **`crates/worker/src/win.rs` (`252`L, `#[cfg(windows)]`):** Extend `create_kill_on_close_job()` with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY` and `JobMemoryLimit` (configurable via `WorkerConfig`; default `2 * 1024 * 1024 * 1024` = **2 GiB** per worker job so up to 3 Python workers fit safely on the 14.8 GB reference laptop).
- **`crates/worker/src/stage.rs` (`424`L, `#[cfg(windows)]`):** Currently `ALLOWED_PACKAGES = &["sympy", "mpmath", "numpy", "numpy.libs"]` and `ALLOWED_DIST_PREFIXES = &["sympy-", "mpmath-", "numpy-"]`. Add verified `scipy` / `scipy.libs` / `matplotlib` / transitive runtime packages (`contourpy`, `cycler`, `fonttools`, `kiwisolver`, `packaging`, `pillow`, `pyparsing`, `python-dateutil`, `six` — exact wheel names verified on PyPI for CPython `3.14.10` `win_amd64`), plus `"physics.py"` in `WORKER_HELPERS` if present! Keep `stage.rs <= 550`L.
- **`packaging/windows/stage_runtime.py` (`166`L):** Verify genuine PyPI `cp314` (`win_amd64`) / pure-Python (`py3-none-any`) wheel URLs and **observed SHA-256 digests** by downloading and checksum-verifying locally (NEVER invent hashes!). Update `target_member`, `THIRD-PARTY.txt` (all upstream notices), `required` checks, and `SMOKE` (`scipy.integrate.quad` + `matplotlib` Agg PNG render into temp dir).

---

## 3. Checkpoints

1. **Checkpoint S18c (`crates/worker/src/win.rs` + Windows memory-cap test):**
   - Add `job_memory_limit_bytes: Option<usize>` (default `2 GiB`), set `JOB_OBJECT_LIMIT_JOB_MEMORY`, and add a Windows integration test in `crates/worker/tests/` verifying an over-limit Python allocation is terminated cleanly by the Job Object without leaking child processes.
2. **Checkpoint S18d (`stage.rs`, `packaging/windows/stage_runtime.py`, `smoke-install.ps1`):**
   - Stage & SHA-256-pin `scipy`, `matplotlib`, and transitive deps for Python `3.14.10` `win_amd64`, run `py -3 packaging/windows/stage_runtime.py --output ...`, run `cargo test --locked --workspace --no-fail-fast -j 2` on Windows 11 (including `appcontainer_sandbox`, `python_worker`, and new Agg/SciPy AppContainer smoke inside isolated scratch with zero network), and update `smoke-install.ps1` line 220 + AppContainer check.
   - Report exact staged wheel table, SHA-256 digests, licences, and runtime size delta in `docs/design/modes-2026-11/reports/solve-s18b-windows-runtime.md`.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **Owned:** `crates/worker/src/win.rs`, `crates/worker/src/stage.rs`, `crates/worker/src/lib.rs`, `crates/worker/tests/` (memory-cap & SciPy/matplotlib AppContainer test), `packaging/windows/stage_runtime.py`, `packaging/windows/smoke-install.ps1`, `docs/design/modes-2026-11/reports/solve-s18b-windows-runtime.md`.
- **Do NOT edit:** `Cargo.lock`, `Cargo.toml`, `crates/research-tools/src/solve_artifacts.rs` (Part A), `crates/desktop-server/src/solve*.rs`, `tool_defs.txt`, `tool_profiles.txt`.

---

## 5. Mandatory Verification (Windows 11 + WSL2 Cross-Check)

- PowerShell Windows 11 (`RE_SEARCH_PYTHON` unset during full workspace test, `CARGO_TARGET_DIR=D:/cargo-targets/reSearch`, `-j 2`): `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast -j 2`, and `stage_runtime.py` + `appcontainer_sandbox` against the newly staged runtime.
- Linux WSL2 non-root cross-check (`uid != 0`, `RE_SEARCH_PYTHON` unset) ensuring `#[cfg(not(windows))]` builds cleanly. Push to `gemini/modes/solve-s18b-windows-runtime`. End with `PUSHED`.
