# Brief solve-s21-migrate — Fork Earlier Research Session Into Solve (S21)

**Status:** Queued right behind **`S11`** (`solve-s8-s9-s11`, alongside `S4` already merged). File-disjoint from `solve-s12-loop`, `solve-s14-s20-review-blind`, `solve-s13-s17-ui-intake`, and `solve-s15-cli-parity` — can run in parallel with all of them! Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/solve-s21-migrate`

---

## 1. Package & Governing Design

- **Package:** **S21** — `POST /api/sessions/{id}/open-in-solve` in new standalone `crates/desktop-server/src/solve_migrate.rs`: forks an earlier `Mode::Research` session via `export::fork_session`, appends host-written `solve.started` + prefilled `solve.setup` (with `origin: NodeRef` pointing back to the Research session, pinned conventions, existing `assumption` entries, ingested sources), and verifies D19 legacy step tier mapping (`compute_all_tiers`) while leaving the original Research session completely untouched.
- **Governing design:** `00-overview.md` 0.1, 2.3 (`NodeRef`), 4.3, 5.3; `solve.md` 6 (R2, R4), 8.4, 9 (`S21`), 13 (D19); `agents/RULES.md`.

---

## 2. Verified Interfaces on `gemini/work/fixed`

- **`crates/export/src/lib.rs` (`128`L):** `fork_session(data_dir, &ForkOptions)` creates an isolated forked ledger without mutating the source session.
- **`crates/research-tools/src/{solve_setup.rs, solve_own_tier.rs, solve_status.rs}`:** `compute_all_tiers` already maps legacy Research steps per D19 (accepted old reviews -> `ModelAgreed` / `ModelAgreedSameModel`, old SymPy validations -> receipts only, old `lean_check` -> `"Lean check recorded in the earlier session"` receipt with no computed tier!).
- **`crates/desktop-server/src/api_dispatch.rs` (`430`L):** Register `("POST", ["api", "sessions", id, "open-in-solve"])` behind `AuthGuard::authorize_api` without editing `solve.rs` or `http_srv.rs`.

---

## 3. Checkpoints

1. **Checkpoint S21a (`crates/desktop-server/src/solve_migrate.rs` + `tests/solve_migrate.rs`):**
   - Implement `handle_open_in_solve(state, session_id, body)` behind `AuthGuard::authorize_api` and `Origin` check:
     - Validates source session is `Mode::Research` (refuses non-Research or missing sessions cleanly).
     - Calls `export::fork_session`, appends `solve.started` (`research_tools::tool_profile::MARKER_SOLVE_STARTED`) and prefilled `solve.setup` carrying `origin: NodeRef`, first user prompt, pinned conventions, copied `assumption` steps, and ingested sources.
     - Leaves the original Research ledger untouched so it keeps passing `eval_set_r` resume under `research/1`.
   - Integration tests in `crates/desktop-server/tests/solve_migrate.rs`: real legacy Research ledger containing `proof.entry`, `sympy_eval` receipt, `lean_check` receipt, and `proof.verification` forks into a valid Solve session with exact D19 tiers, `AuthGuard` 401/403 enforcement, and intact parent Research session (`eval_set_r`).

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/desktop-server/src/solve_migrate.rs`, `crates/desktop-server/tests/solve_migrate.rs`.
- **Small edits:** `crates/desktop-server/src/lib.rs`, `crates/desktop-server/src/api_dispatch.rs` (+1 arm).
- **Do NOT edit:** `crates/export/src/*`, `crates/agent/src/*`, `crates/desktop-server/src/{http_srv,routes_session,solve}.rs`, `tests/eval_set_r.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`.

---

## 5. Mandatory Verification Before Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Write `docs/design/modes-2026-11/reports/solve-s21-migrate.md` and push to `gemini/modes/solve-s21-migrate`. End with `PUSHED`.
