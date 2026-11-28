# Report: `inv-v9-legacy` (Investigate V9 — Old Sessions & Read-Side Legacy Adapter)

- **Base commit:** `0ec99d316c70025a2acc73c2f9ea12414ea59a45` (`origin/gemini/work/fixed`)
- **Branch:** `gemini/modes/inv-v9-legacy`
- **Commits before report:**
  - `35cabf8` — `investigate(v9): add read-side legacy adapter, GET legacy route, and mapview embedding`
  - `a6442ab` — `investigate(v9): add investigation_legacy.js UI module, styles, and static asset route`
  - `a4764b5` — `investigate(v9): wire direct UI integration into node card, api.js, and V3 shell`
- **Toolchain on Linux WSL2 (`antimajorana`, non-root):** `rustc 1.100.0`, `cargo 1.100.0`, `node v24.22.2`, `python` (`sympy 1.14.0`, `mpmath 1.3.0`, `numpy 2.5.4` in `$HOME/.venvs/bootloops`)
- **Guard rails:**
  - `Cargo.lock` untouched (no new dependencies; `--locked` builds).
  - `tool_defs.txt`, `tool_profiles.txt`, `eval_set_r.rs`, `investigation_mapview.rs`, `investigation_map.js`, `assets/investigation.js`, `desktop-server/src/investigation.rs`, `research-tools/src/investigation.rs`, `resume.rs`, `http_srv.rs`, `routes_session.rs`, `index.html`, `app.js` all untouched.
  - All touched/created `.rs`, `.js`, `.mjs`, `.css` files stay `<= 600` lines (`py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits `0`).
  - Zero `"verified"` / `"Verified"` in V9 adapter/UI output or strings; zero `"proof"` guessed for legacy edges; zero `innerHTML` writes; zero third-party paper text.

---

## 1. What was implemented

| File | Lines | Role |
|---|---:|---|
| `crates/desktop-server/src/investigation_legacy_model.rs` | 242 | V9 data model (`LegacyMapProjection`, `LegacyNode`, `LegacyEdge`, `LegacyNodeEdge`, `LegacyCorrectionNote`, `LegacyAnchor`), wording/prefix constants (`LEGACY_NODE_PREFIX = "legacy:"`, `LEGACY_EDGE_ROLE = "unknown (older map)"`, `LEGACY_SECTION_LABEL = "Earlier map"`, `LEGACY_UNPLACED_LABEL = "Unplaced (older map)"`, `LEGACY_REBUILD_LABEL = "Rebuild earlier map"`), innermost span-containment matcher, and disallowed-key sanitizer |
| `crates/desktop-server/src/investigation_legacy_view.rs` | 479 | Pure read-side adapter `adapt_legacy_investigation(&[Event], Option<&PaperSkeleton>)` (zero ledger mutations, zero model calls), `embed_legacy_into_mapview`, `handle_get_legacy` calling `ensure_skeleton_on_open`, `expanded_map` + standalone branch merging, correction folding, `paper_conventions`, `summary`, `activities`, and `answers` preservation |
| `crates/desktop-server/src/investigation_legacy_view_tests.rs` | 316 | Unit tests covering pure V2 `has_legacy: false` / `mapview.legacy = null`, innermost nested span placement with deterministic `SkNode.order` tie-break, `unplaced` fallback, standalone branches, summary/conventions/activities/answers preservation, determinism, and zero `"verified"` / guessed `"proof"` |
| `crates/desktop-server/src/lib.rs` | 242 | Registered `mod investigation_legacy_model;` and `pub mod investigation_legacy_view;` |
| `crates/desktop-server/src/api_dispatch.rs` | 436 | Registered `GET /api/sessions/{id}/investigation/legacy` behind `AuthGuard::authorize_api` before the catch-all `investigation/{operation}` arm |
| `crates/desktop-server/src/investigation_skeleton.rs` | 546 | Made `open_and_read` and `fallback_note` `pub(crate)`, and wired `embed_legacy_into_mapview` into both the skeleton-present and fallback (`skeleton_seq: null`) branches of `GET /investigation/mapview` |
| `crates/desktop-server/tests/investigation_legacy_view.rs` | 462 | HTTP integration tests: auth (`401` missing token & stream-cookie-only, `403` foreign `Host`, `400` bad session id), all 4 Set R session shapes (`Research`, pre-pointer `7c36a0e`, pointer-cited `Investigation`, `research_from_paper` handoff child), nested TeX innermost placement (`Lemma`/`Theorem`/`Proof`), `investigation.branch` + `correction` + `summary` + `conventions` + `answers` + `activities`, tampered source `fallback_old_view: true`, pure V2 `null`, and live `/mapview` embedding |
| `crates/desktop-server/assets/investigation_legacy.js` | 339 | Pure DOM builder (`createElement` + `textContent`; zero `innerHTML`) exporting `mountLegacyEarlierMap`, `mountLegacyPanel`, `relabelOldSessionButtons`, and wording constants |
| `crates/desktop-server/assets/investigation_shell.css` | 341 | Added `.inv-legacy-*` styles matching dashed model styling and warning fallback banners |
| `crates/desktop-server/src/assets.rs` | 550 | Registered `/assets/investigation_legacy.js` |
| `crates/desktop-server/assets/api.js` | 526 | Exported `getInvestigationLegacy(id)` |
| `crates/desktop-server/assets/investigation_node.js` | 423 | Wired `mountLegacyEarlierMap` in Block A when `node.earlier_map` (or `opts.legacyNodes`) is non-empty |
| `crates/desktop-server/assets/investigation_v3_shell.js` | 599 | Wired `mountLegacyPanel` inside `renderShell` and on the `mv.skeleton_seq == null` fallback path (plus label reset on non-legacy sessions) |
| `crates/desktop-server/tests/investigation_legacy.test.mjs` | 310 | Unit + direct integration JS tests covering `mountLegacyEarlierMap`, `mountLegacyPanel`, XSS escaping (`innerHtmlWrites === 0`), `node.js` `earlier_map` mounting, `v3_shell` panel & fallback mounting, and `"Find main results"` -> `"Rebuild earlier map"` relabelling + reset |

---

## 2. Deviations, decisions taken alone, and assumptions

### Deviations from `inv-v9-legacy.md`
1. **Direct integration wired directly into `investigation_skeleton.rs`, `investigation_node.js`, `investigation_v3_shell.js`, and `api.js`:** `inv-v9-legacy.md` §2/§5 initially paused edits to those files while `inv-v7-v8-v10` ran in parallel. Because V7, V8, and V10 are already merged on `origin/gemini/work/fixed` (`0ec99d3`), the coordinator explicitly instructed direct integration wiring on this branch.
2. **Helper split for 600-line ceiling:** Split pure data structures, constants, and span-matching helpers into `investigation_legacy_model.rs` (`242`L) so `investigation_legacy_view.rs` (`479`L) and `investigation_legacy_view_tests.rs` (`316`L) stay well under `600` lines.
3. **Minimal styles in `investigation_shell.css`:** Appended 30 lines of `.inv-legacy-*` classes to `investigation_shell.css` (`311` -> `341` lines) instead of adding an unlinked stylesheet or editing `index.html`.

### Decisions taken alone & assumptions
- **Per-node `LegacyNode.edges` pre-joined:** Attached `edges: Vec<LegacyNodeEdge>` directly on each `LegacyNode` so standalone callers of `mountLegacyEarlierMap(nodeHost, node.earlier_map)` render `"unknown (older map)"` dashed edge badges and jump buttons without requiring a separate `legacyPayload` argument.
- **Innermost span tie-breaking:** `match_innermost_skeleton_node` inspects each node's first anchor `{path, start_line, end_line}`, skips empty paths, `start_line == 0`, or mismatched non-empty `source_id`, and picks the tagged `SkNode` minimizing `(enclosing_span_line_count, SkNode.order, tag)`.
- **Edge & wording safety:** `strip_disallowed_keys` strips any object key containing `"verified"` (case-insensitive) in projected anchor/convention/activity JSON without altering ledger storage, and normalizes any legacy `"proof"` edge kind to `"depends_on"` while keeping `role: "unknown (older map)"`, `basis: "model"`, `dashed: true`.
- **Reversible `"Rebuild earlier map"` relabelling:** `relabelOldSessionButtons` updates matching `"Find main results"` buttons in old sessions and restores `"Find main results"` when `has_legacy` is false so switching between old and new sessions does not leak labels.
- **Scope assumption on `has_legacy`:** Assumes `investigation.question_requested`/`answer` and `investigation.requested`/`result` without V2 markers belong to legacy sessions; see Merge Notes below regarding future V14 Q&A.

---

## 3. Merge notes

- **Direct hooks already wired on this branch:**
  - `crates/desktop-server/src/lib.rs:10-12`
  - `crates/desktop-server/src/api_dispatch.rs:291-293`
  - `crates/desktop-server/src/assets.rs:286-289`
  - `crates/desktop-server/src/investigation_skeleton.rs:249-256,394,436`
  - `crates/desktop-server/assets/api.js:206-208`
  - `crates/desktop-server/assets/investigation_node.js:7,356-363,422`
  - `crates/desktop-server/assets/investigation_v3_shell.js:15,172,572-575`
- **`investigation_v3_shell.js` headroom:** Now at `599` lines (`1` line below the `600`-line CI limit). Subsequent lanes adding non-trivial logic to `investigation_v3_shell.js` should extract a helper module first.
- **Future V14 Q&A:** If V14 stores new Investigate questions under `investigation.question_requested` / `investigation.answer`, narrow `has_legacy` in `crates/desktop-server/src/investigation_legacy_view.rs:78-85` (e.g., check `map` / `branch` / `summary` / `conventions` / `correction` or distinguish V2 Q&A by `skeleton_seq`).

---

## 4. Verification on code commit `a4764b5` (pre-report snapshot)

*(Full post-report final suite ran immediately after committing this file.)*

| Check | Outcome | Notes |
|---|---:|---|
| `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0 | `No touched source file is over 600 lines.` (`max 599` on `investigation_v3_shell.js`) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 | All JS files syntax-clean |
| `node --test crates/desktop-server/tests/*.test.mjs` | Exit 0 | **211 passed; 0 failed; 0 skipped** |
| `$HOME/.venvs/bootloops/bin/python -m unittest discover -s python` | Exit 0 | **33 passed; 0 failed** |
| `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 | Zero warnings / errors across entire workspace |
| `cargo test --locked -p desktop-server --lib investigation_legacy_view` | Exit 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo test --locked -p desktop-server --test investigation_legacy_view --test investigation_v3_routes --test investigation_v7_v8_v10 --test eval_set_r --test mode_markers_r2` | Exit 0 | **14 passed; 0 failed; 0 ignored** (`2 + 2 + 2 + 5 + 3`) |
| `cargo test --workspace --locked --no-fail-fast -j 2` (`unset RE_SEARCH_PYTHON`, system `/usr/bin/python3`) | Exit 1 | 139 test targets: **1130 passed; 3 failed; 2 ignored** (sole 3 failures in untouched `research-tools --test solve_s7_exprs` because `/usr/bin/python3` lacked `sympy`) |
| `cargo test --locked -p research-tools --test solve_s7_exprs` (with `$HOME/.venvs/bootloops/bin` first on `PATH`, `unset RE_SEARCH_PYTHON`) | Exit 0 | **5 passed; 0 failed; 0 ignored** |

---

## 5. Pre-existing environment notes & Linux exclusions

- `crates/desktop-server/src/evidence.rs:744` (`1,057`L, untouched) triggers `clippy::manual_unwrap_or` on Linux Rust `1.100.0` (suppressed via `-A clippy::manual_unwrap_or` per `RULES.md`).
- 2 pre-existing ignored tests in `crates/agent/tests/eval_harness.rs` (live provider tier).
- Windows-only targets/tests (`desktop-app`, `worker::tests::{appcontainer_sandbox, python_worker}`, `cli` Windows containment tests, `chatgpt-auth` DPAPI tests) do not compile/run on Linux (`0` tests executed in those harnesses on Linux).

---

## 6. Bugs noticed (not fixed, outside V9 scope)

- **`crates/ledger/src/verify.rs:280-306` vs `crates/desktop-server/src/investigation_skeleton.rs:344`:** `investigation.skeleton` records its ledger blob SHA-256 at top-level `payload["blob"]`, while `Ledger::verify_event_blobs` only indexes top-level `"raw_blob"` and nested `tool.{request,result}` slots (`args`/`result`/`error`). Consequently `Ledger::verify_all()` counts distinct `investigation.skeleton` blobs in `unreferenced_blobs`, even though `Ledger::verify_session()` verifies every session chain cleanly. Tested explicitly in `tests/investigation_legacy_view.rs:233-248`.
- **Untouched Python test discovery on system Python:** `/usr/bin/python3` and `miniforge3` base on this Linux host lack `sympy`/`mpmath`, causing cross-process `solve_s7_exprs` tests to fail unless the SymPy venv (`$HOME/.venvs/bootloops/bin`) is on `PATH` when `RE_SEARCH_PYTHON` is unset.
