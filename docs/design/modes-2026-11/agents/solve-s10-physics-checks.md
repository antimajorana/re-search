# Brief solve-s10-physics-checks — Solve Physics Checks (S10)

**Status:** Queued right behind **`S9`** (merged inside `solve-s8-s9-s11`). Zero `Cargo.lock` edits! File-disjoint from `solve-s19-export-replay`, `solve-s18a-artifacts`, `solve-s18b-windows-runtime`, `solve-s12-loop`, and `f15-writeup-solve`.
**Push branch:** `gemini/modes/solve-s10-physics-checks`

---

## 1. Package & Governing Design

- **Package:** **S10** — 6 clean-room physics checks (`dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, `matrix_property`) in `python/checks/physics.py`, registry registration in `python/checks/__init__.py`, and Rust `check_run` / `dimensions` auto-check wiring in `crates/research-tools/src/solve_checks.rs`.
- **Governing design:** `00-overview.md` 0.1, 7.3 (R22), 8.1–8.3; `solve.md` 4.4, 5.1, 6 (R3, R4, R8, R19, R22), 7.2–7.3, 9 (`S10`), 11 (licence note: zero code/text copied from "Get Physics Done"), 12 (risk 4); `agents/RULES.md`.

---

## 2. Verified Interfaces

- **Tier contract (`solve_own_tier.rs:16-23`, `solve_defs.rs:47-59`):** `SOLVE_V1_CHECK_NAMES` already freezes all 6 check names (`"dimensions"`, `"limit_case"`, `"symmetry"`, `"conservation"`, `"convergence"`, `"matrix_property"`). `NON_IDENTITY_CHECKS` enforces R8: passing any S10 check adds its name as a sublabel ONLY and NEVER grants `Tier::Verified` or `Tier::CheckedNumerically`; failing flips the step permanently to `Tier::Failed`.
- **Check process & S9 registry (`python/checks/__init__.py`, `python/checks/srepr_safe.py`, `python/checks/algebra.py`):** Reuse S9's check-function signature, setup assumption extractor (`assumed_symbol`), `registry_sha256` hash table, and error mapping (`symbol assumptions differ` -> `inconclusive`, never `fail`).

---

## 3. Checkpoints — Push After Each!

1. **Checkpoint S10a (`python/checks/physics.py` + `python/tests/test_checks_physics.py`):**
   - Clean-room original implementations over canonical handles + setup context (`symbols`, `dimensions`, `assumptions`):
     - `dimensions`: dimensional analysis over `setup.dimensions` (`{symbol: {base: power}}`) with integer and rational powers (`"3/2"` per S2); mismatch -> `fail` with exact computed dimensions; missing symbol dimension -> `inconclusive`.
     - `limit_case`: symbolic/series limit `var -> target` (optional direction) on `lhs` vs `rhs` or named target handle; numeric counterexample -> `fail`; unreduced -> `inconclusive`.
     - `symmetry`: applies substitution map (e.g. `{phi -> -phi}`, parity/scale/exchange) checking invariance or declared sign (`+1`/`-1`) with numeric guard; counterexample -> `fail`.
     - `conservation`: checks vanishing time derivative, commutator (`[H, Q] = 0`), or declared PDE conservation relation under assumptions; counterexample -> `fail`.
     - `convergence`: series/sequence/integral convergence or refinement residual ratio using `mpmath`/`numpy`/`sympy` (no SciPy required); divergence beyond tolerance -> `fail`; ambiguous -> `inconclusive`.
     - `matrix_property`: exact symbolic check (small matrices) + tolerance numeric eigenvalues for `hermitian`, `symmetric`, `unitary`, `orthogonal`, `positive_definite`, `traceless`, `invertible`; violation with counterexample witness -> `fail`.
   - Assumption mismatches ALWAYS return `inconclusive: symbol assumptions differ` (`solve.md` 12 risk 4).
2. **Checkpoint S10b (`python/checks/__init__.py`, `solve_checks.rs`, integration tests):**
   - Register all 6 checks in `python/checks/__init__.py` (updating `registry_sha256`).
   - Wire `dimensions` auto-check trigger on `step_record` when `setup.dimensions` is non-empty (through S9's `host:check` post-call hook) and validate params cleanly in `crates/research-tools/src/solve_checks.rs`.
   - Note in report `WORKER_HELPERS` / `zip.rs` if explicit file lists need `"physics.py"` in S18b/S19.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `python/checks/physics.py`, `python/tests/test_checks_physics.py`, `crates/research-tools/tests/solve_s10_physics.rs`.
- **Small edits:** `python/checks/__init__.py`, `crates/research-tools/src/solve_checks.rs` (split helper module if > 500L).
- **Do NOT edit:** `crates/export/*` (owned by `S19`/`F15`), `crates/desktop-server/src/routes_*`, `tool_profile.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`, `crates/worker/src/{win,stage}.rs`.

---

## 5. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
python3 -m unittest discover -s python -p "test_*.py"
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Write `docs/design/modes-2026-11/reports/solve-s10-physics-checks.md` and push to `gemini/modes/solve-s10-physics-checks`. End with `PUSHED`.
