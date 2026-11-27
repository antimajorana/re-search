# Solve Package S10 (`solve-s10-physics-checks`) Report

- **Branch**: `gemini/modes/solve-s10-physics-checks`
- **Initial base commit**: `acf7cb0` (PR #52 candidate integrating S8+S9+S11 onto `26c2dc6`)
- **Merged remote fixed tip**: `eb0eb63` (`origin/gemini/work/fixed`, PR #55)
- **Implementation commits**:
  - **S10a (Python physics checks + unit tests)**: `28f98bb`
  - **S10-staging (`STAGED_CHECK_FILES` + NSIS)**: `3c6616c`
  - **S10b (Rust host params + `AutoCheckGate` + cross-process E2E)**: `59f40fd`
  - **Merged `origin/gemini/work/fixed` (`24be58e` then `eb0eb63`)**: `a71d5b6` and fast-forward/merge of `eb0eb63`
  - **S10 final report**: committed last on this branch

---

## 1. Scope Implemented

### Checkpoint S10a — Clean-Room Python Physics Checks (`python/checks/`, `python/tests/`)
Implemented six clean-room checks in `python/checks/physics.py`, `python/checks/physics_helpers.py`, and `python/checks/physics_matrix.py`, registered in `python/checks/__init__.py` (updating `checks.registry_sha256()`), with unit coverage in `python/tests/test_checks_physics.py`:
1. `dimensions`: rational dimensional algebra over host-injected `setup.dimensions` (`{symbol: {base: power}}`) handling powers, derivatives, integrals, sums (auto-binding undeclared dummy index as dimensionless), `DiracDelta`, `Abs`/`re`/`im`/`Heaviside`/`sign`, piecewise, transcendental dimensionless-argument validation, matrix entry-wise checks, and zero-scalar compatibility. Returns `inconclusive` when setup omits dimensions or an expression symbol lacks declared dimensions, and `fail` on mismatch with formatted dimension evidence.
2. `limit_case`: symbolic `sp.limit` with fallback `mpmath` directional numeric counterexample detection (`var -> point`, `+/-`), supporting both `lhs` vs `rhs` and verified `target_handle` (`handles["target"]`).
3. `symmetry`: simultaneous substitution invariance/covariance (`subs`/`transform`/`map` or shorthand `kind` in `{even, odd, exchange}` with optional `sign`/`factor`) checked symbolically and via `mpmath` residual guards for scalars and matrices.
4. `conservation`: exact + numeric verification for `commutator` (`[A, B] = 0`), `derivative`/`time_derivative` (`d/dt = 0`), `continuity`/`divergence` (`d rho/dt + div J = 0`), `poisson_bracket` (`{f, g}_{q,p} = 0`), and `residual`, supporting optional constraint `subs`.
5. `convergence`: clean-room convergence checks reusing `numerical_validation.check_convergence` alongside SymPy `Sum.is_convergent()`, `Integral.doit()` / `mp.quad` tail-cutoff refinement, vector refinement lists, and parametrized sequences (`n->oo`, `h->0`); zero SciPy dependencies.
6. `matrix_property`: exact + NumPy/mpmath checks across `hermitian`, `symmetric`, `unitary`, `orthogonal`, `positive_definite` (leading minors + Hermitian `eigvalsh` eigenvalues), `traceless`, and `invertible` (symbolic `det` + `solveset` domain roots + `svd` minimum singular value).

All expressions arrive solely via `srepr_safe.parse_srepr_safe` (zero `eval`, `exec`, string `sympify`, or code/text from GPLv3 external packages). Matrix handles are converted with `.as_immutable()` before `algebra_probe.normalize_symbols`, so any conflicting symbol assumptions return `inconclusive` (`"inconclusive: symbol assumptions differ"`).

### Worker / NSIS Staging (`crates/worker/src/protocol.rs`, `packaging/windows/re-search.nsi`)
- Added `checks/physics.py`, `checks/physics_helpers.py`, and `checks/physics_matrix.py` to `STAGED_CHECK_FILES` (plus `collect_and_write_staged_scripts_includes_checks_package` test assertions) and to the Windows NSIS installer manifest (`packaging/windows/re-search.nsi`).

### Checkpoint S10b — Host `check_run` Validation, `AutoCheckGate`, & Cross-Process E2E (`crates/research-tools/`)
- **`crates/research-tools/src/solve_checks_params.rs` (`572`L)** + **`solve_checks.rs` (`399`L)** + **`solve_checks_tests.rs` (`347`L)**:
  - Split argument/parameter parsing and R3 guards out of `solve_checks.rs` into `solve_checks_params.rs` so every file remains `<= 600` lines.
  - Enforced R3 size (`16 KiB`), depth (`8`), recursive forbidden keys (`code`, `outcome`, `expected`, `expected_value`), and top-level host-reserved/forged key rejection (`handles`, `symbols`, `dimensions`, `assumptions`, `target`, `evidence`, hashes), plus per-check parameter validation for S10 checks.
  - Host-injected `setup.assumptions` alongside `setup.symbols` and `setup.dimensions`, and resolved `limit_case` `params.target_handle` (`HandleRef`) against `state.handles` into `handles["target"]`.
  - Updated `CheckProcessRunner::supports_check` to include all `SOLVE_V1_CHECK_NAMES`, enabling automatic `dimensions` execution inside `AutoCheckGate` whenever `setup.dimensions` is non-empty.
  - Retained existing S9 `auto_check_gate_runs_and_handles_denied_and_timeout` assertions verbatim by adding `support_dimensions: bool` to `MockCheckRunner`, and added `auto_check_gate_executes_dimensions_and_validates_s10_params`.
- **`crates/research-tools/tests/solve_s10_physics.rs` (`535`L)**:
  - Cross-process E2E tests spawning real Python `worker.py --stdio --role solver` and `--role check` through `solve/1` `Router`, `GateChain`, and `solve_check_host_v1`:
    1. `s10_dimensions_autocheck_r8_and_finish_criterion`: auto-check `dimensions` + `identity_symbolic`, `NamedCheckPassed("dimensions")` satisfaction on `solve_finish`, and R8 `Failed` latch on dimensional mismatch.
    2. `s10_all_six_checks_r8_sublabel_only_failure_and_r19`: exercises all 6 checks passing on `Unchecked` inequality steps (sublabel-only per R8, never `Verified`/`Numerically checked`), failing `symmetry` on a symbolically `Verified` step flipping it to `Failed` + propagating `Broken` + `GATE G7` refusal, and R19 `evaluate_recheck_diff` reproducibility (`matched == true`).
    3. `s10_assumption_mismatch_and_r3_param_refusals`: verifies all 6 checks return `inconclusive` on contradictory setup vs handle assumptions or cross-handle assumption disagreement, and confirms host R3 rejection before worker execution.

---

## 2. File Inventory & Line Counts (All <= 600 Lines)

| File | Lines | Scope |
|---|---|---|
| `python/checks/__init__.py` | 127 | Registers 6 physics checks in `CHECKS_REGISTRY` |
| `python/checks/physics.py` | 411 | `dimensions`, `limit_case`, `symmetry`, `conservation`, `PHYSICS_CHECKS` |
| `python/checks/physics_helpers.py` | 538 | Safe AST param parser, rational dimensional algebra, handle normalization |
| `python/checks/physics_matrix.py` | 443 | `convergence`, `matrix_property`, matrix/scalar zero guards |
| `python/tests/test_checks_physics.py` | 445 | Unit + stdio `--role check` worker tests (8 tests) |
| `crates/worker/src/protocol.rs` | 428 | Added 3 `checks/physics*.py` entries to `STAGED_CHECK_FILES` + test |
| `packaging/windows/re-search.nsi` | 247 | Added 3 `checks/physics*.py` installer entries |
| `crates/research-tools/src/solve_checks.rs` | 399 | Enabled `supports_check` for all `SOLVE_V1_CHECK_NAMES`; extracted `params` |
| `crates/research-tools/src/solve_checks_params.rs` | 572 | R3 parameter validation, `target_handle` resolution, `assumptions` injection |
| `crates/research-tools/src/solve_checks_tests.rs` | 347 | Added `support_dimensions` toggle + S10 auto-check/param unit test |
| `crates/research-tools/tests/solve_s10_physics.rs` | 535 | Cross-process E2E tests for all 6 S10 checks, R3, R8, R19 |

Zero frozen or out-of-lane files were modified (`Cargo.lock`, `crates/export/*`, `tool_profile.rs`, `solve_defs.rs`, `tests/golden/*`, `crates/worker/src/{win,stage}.rs`, `desktop-server/src/routes_*`).

---

## 3. Deviations, Decisions, & Assumptions

- **Module splits (`<= 600` line cap)**: Split Python physics checks into `physics.py` + `physics_helpers.py` + `physics_matrix.py`, and split Rust parameter validation/prep into `solve_checks_params.rs` instead of growing `solve_checks.rs` (`568`L) or `physics.py` past `600` lines.
- **Standalone E2E harness**: Kept `crates/research-tools/tests/solve_rig/mod.rs` untouched and placed a local `PhysicsRig` helper inside `crates/research-tools/tests/solve_s10_physics.rs` to supply non-empty `setup.dimensions` without altering shared S8/S9/S11 test helpers.
- **Preserved S9 mock assertions**: Added `support_dimensions: bool` to `MockCheckRunner` in `solve_checks_tests.rs` so the existing S9 test keeps exercising the `not_run` fallback branch unmodified while a dedicated S10 test exercises `support_dimensions: true`.
- **SymPy matrix normalization quirk**: SymPy 1.14 `sp.Matrix` returns `MutableDenseMatrix`, which is not an `sp.Basic` instance; `physics_helpers.coerce_handle_obj` calls `.as_immutable()` before `algebra_probe.normalize_symbols` so free-symbol assumption checks run uniformly on matrices.
- **`normalize_symbols` compatibility behavior**: Observed in `python/checks/algebra_probe.py:109-123` that weaker handle assumptions (`real=True`) are treated as compatible with stronger setup assumptions (`real=True, positive=True`) and promoted cleanly; contradictory handle assumptions (`negative=True` vs `positive=True`) or cross-handle mismatches (`lhs` vs `rhs`) produce `"inconclusive: symbol assumptions differ"`.

---

## 4. Merge Notes

1. **Worker sandbox / runtime staging (`crates/worker/src/protocol.rs`, `packaging/windows/re-search.nsi`)**: Commit `3c6616c` added `checks/physics.py`, `checks/physics_helpers.py`, and `checks/physics_matrix.py` to `STAGED_CHECK_FILES` and `re-search.nsi` (per S6 handoff instructions in `docs/design/modes-2026-11/reports/q-f11-x1-s3-s6.md`).
2. **S18b (`solve-s18b-windows-runtime`) & S19 (`solve-s19-export-replay`)**: `crates/export/src/zip.rs` and Windows packaging/sandbox flows should read `worker::protocol::STAGED_CHECK_FILES` dynamically (already populated with all three `checks/physics*.py` modules) rather than hardcoding `checks/algebra*.py`.
3. **`desktop-server` / `mode_markers_r2.rs`**: `solve_checks_params.rs` performs zero ledger `append*` writes and is a private submodule of `solve_checks.rs`; existing HTTP check/recheck routes inherit S10 support through `check_run_handler` without modification.

---

## 5. Verification Results (Post-Final-Code-Edit, After Merging `eb0eb63`)

| Command | Result |
|---|---|
| `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Pass (`No touched source file is over 600 lines.` across all 11 touched files) |
| `python3 -m unittest discover -s python -v` (WSL2 Python 3.12.3, SymPy 1.14.0, mpmath 1.3.0, NumPy 2.5.4) | **52 passed, 0 failed** (`38.479s`) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` (WSL2 non-root, `RE_SEARCH_PYTHON` unset) | Pass (`0` errors, `0` warnings beyond documented pre-existing `evidence.rs:744` suppression) |
| `cargo test --workspace --locked --no-fail-fast -j 2` (WSL2 non-root, `RE_SEARCH_PYTHON` unset) | **1,202 passed, 0 failed, 2 ignored** across 153 suites (`agent/tests/eval_harness.rs`: `file_sheet_from_json` and `live_manifests_validate` require live env vars, pre-existing) |
| `RE_SEARCH_PYTHON="/home/antimajorana/.venvs/bootloops/bin/python3" cargo test --locked -p worker -p research-tools` | **535 passed, 0 failed, 0 ignored** across 46 suites (includes `solve_s10_physics`: `3 passed`, `solve_checks`: `2 passed`, `worker::protocol::tests::collect_and_write_staged_scripts_includes_checks_package`: `1 passed`) |

### Not Run on Linux WSL2
- Windows-only targets: `crates/worker/tests/appcontainer_sandbox.rs` (`0 tests` compiled/run on Linux), Windows Job Object containment enforcement (`job_object_sandbox_is_not_contained` unit test passed), and Windows NSIS installer packaging / `smoke-install.ps1`.
