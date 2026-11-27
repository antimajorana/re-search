# Investigate V16 (`inv-v16-handoff`) — Implementation & Verification Report

## Base & Branch

- **Base commit**: `0ec99d3` (`Merge pull request #45 from antimajorana/coord/modes-inv-v18`, matching `origin/gemini/work/fixed` at branch creation)
- **Branch**: `gemini/modes/inv-v16-handoff`
- **Checkpoint commits**:
  - **CP1 (`5b272fa`)**: Outgoing packet builders, incoming `target_mode: "investigate"` receiver, and read-only child status projection
  - **CP2 (`22910a6`)**: UI hand-off controls, right half-badge activation, read-only status chips, click-to-open navigation, Rust integration tests, and Node UI tests

## What Was Implemented

### 1. Outgoing Packet Builders (Zero Model Calls, Zero Network Calls)
- Implemented in [`investigation_handoff.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/investigation_handoff.rs) and [`investigation_handoff_extract.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/investigation_handoff_extract.rs):
  - **`"Derive in Solve"` (`HandoffActionKind::DeriveInSolve`)**: Maps theorem-like nodes (`Definition`, `Assumption`, `Theorem`, `Lemma`, `Proposition`, `Corollary`, `Conjecture`, `Claim`, `Example`, `Remark`, `Algorithm`) to `TaskKind::Prove` / `TargetForm::Proof`, `Equation` nodes to `TaskKind::Calculate` / `TargetForm::ClosedForm`, and `Proof` nodes to `TaskKind::CheckDerivation` / `TargetForm::PerStepVerdict`. Extracts verbatim statement text and F4-verified `QuoteRef`s, traverses `SkEdgeKind::Uses` (`SkEdgeRole::Statement`) and `Proof` prerequisites for statement closure into `context`, maps in-scope `SymbolDef` entries to `SymbolSpec`, summarizes pinned + paper conventions (`MAX_CONVENTIONS_CHARS`), carries reader notes in `user_notes`, and places model-authored gists/purposes strictly in `model_text` suffixed with `"(model)"` while never copying badges/tiers/verdicts.
  - **`"Has this been extended?"` (`HandoffActionKind::HasBeenExtended`)**: Builds a `target_mode: Mode::Explore` packet with `ExploreQuestionKind::Extended`, sets `ask.origin_paper` to the origin `PaperRef` (so Explore excludes it from `"found"` verdicts), resolves `skeleton.bib` entries via `Library` into deduplicated `ask.seed_papers` (`origin_paper` excluded, cited-here entries prioritized), and carries `user_notes` + `model_text`.
  - **`"Search wider in Explore"` (`HandoffActionKind::SearchWiderInExplore`)**: Same provenance/seed construction with `ExploreQuestionKind::PriorWork` and `"Is this result earlier?"` framing, connected directly to V7's reserved right half-badge slot (`[data-v16-slot="prior-work-explore"]`) and `.inv-node-handoff-slot`.
  - Added `POST /api/sessions/{id}/investigation/handoff-packet` in [`api_dispatch.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/api_dispatch.rs) returning a ready-to-submit `HandoffRequest` with verbatim `quote_texts`.

### 2. Incoming Receiver (`target_mode: "investigate"`)
- Implemented in [`investigation_handoff_receive.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/investigation_handoff_receive.rs) and wired into [`handoff.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/handoff.rs):
  - Checks copied sources already in `child_session` first, then consults `Library` (`read_fulltext_bytes` + `attach_to_session`, **zero network calls on cache hit**). Returns cleanly with `status: "needs_file"` if offline/uncached rather than making unrequested network calls.
  - Re-verifies incoming `earlier_quotes` / `statement.quotes` / `context[].quotes` via F4 `verify_quote` / `verify_quote_ref` when newly ingested from Library, enforcing `file_sha256` + `quote_sha256` parity and dropping/counting mismatches.
  - Pins source and builds/loads deterministic skeleton (`pin_source_and_build_skeleton`), then locates each surviving quote's **innermost** enclosing skeleton node (`find_innermost_enclosing_node`) and appends an additive `investigation.note` (`EVENT_INVESTIGATION_NOTE`) onto that node.

### 3. Read-Only Child Status Projection & UI
- Implemented backend projection in [`investigation_handoff_status.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/investigation_handoff_status.rs), attached to `GET /api/sessions/{id}/handoffs` (`child_statuses`) and `GET /api/sessions/{id}/investigation/mapview` (`handoff_statuses`) in [`investigation_skeleton.rs`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/src/investigation_skeleton.rs). Never mutates any parent session ledger (Rule 5).
- Enforces the F12 wording invariant (`sanitize_f12_status_label` + JS `sanitizeHandoffBadgeText` using `FORBIDDEN_GENERATED_WORDS` / regex): right half-badge and status chips never emit `"new"`, `"novel"`, `"novelty"`, `"verified"`, `"proved"`, or `"unexplored"`.
- Implemented UI module [`investigation_handoff_ui.js`](file:///C:/Users/antimajorana/Documents/Codex/2026-11-26/files-mentioned-by-the-user-00/work/lane-inv-v16/crates/desktop-server/assets/investigation_handoff_ui.js) mounted cleanly into `.inv-node-handoff-slot` (`investigation_node.js`) and V7's right half-badge slot (`investigation_badges.js`), with click-to-open-child navigation wired through `hooks.openSession` (`investigation.js` `callbacks.open` -> `app.js` `selectSession`) and zero `innerHTML` writes.

## File Inventory & Line Counts

| File | Role | Lines |
|---|---|---:|
| `crates/desktop-server/src/investigation_handoff.rs` | Outgoing packet builders & HTTP endpoint | 291 |
| `crates/desktop-server/src/investigation_handoff_extract.rs` | Closure, symbols, seeds, conventions, quote slicing | 449 |
| `crates/desktop-server/src/investigation_handoff_status.rs` | Read-only child status projection & F12 guard | 198 |
| `crates/desktop-server/src/investigation_handoff_receive.rs` | Incoming `investigate` open/verify/innermost note attach | 562 |
| `crates/desktop-server/src/handoff.rs` | Target receiver dispatch & `child_statuses` view wiring | 104 |
| `crates/desktop-server/src/api_dispatch.rs` | `POST .../investigation/handoff-packet` route | 439 |
| `crates/desktop-server/src/investigation_skeleton.rs` | Attach `handoff_statuses` to `mapview` | 544 |
| `crates/desktop-server/src/lib.rs` | Module declarations | 242 |
| `crates/desktop-server/src/assets.rs` | Register `/assets/investigation_handoff_ui.js` | 550 |
| `crates/desktop-server/assets/investigation_handoff_ui.js` | V16 UI controls, right badge wiring, status chips | 286 |
| `crates/desktop-server/assets/investigation_node.js` | Mount V16 hand-off slot controller | 420 |
| `crates/desktop-server/assets/investigation_badges.js` | Enable right half-badge when `onSearchWider` wired | 274 |
| `crates/desktop-server/assets/investigation_v3_shell.js` | Pass `handoff` opts & forward `derive_solve`/`extend_explore` | 598 |
| `crates/desktop-server/assets/investigation.js` | Forward `openSession: callbacks?.open` in `hooks` | 598 |
| `crates/desktop-server/assets/api.js` | Add `buildInvestigationHandoffPacket` client helper | 526 |
| `crates/desktop-server/assets/investigation_shell.css` | Styles for `.inv-v16-*` actions & status chips | 317 |
| `crates/desktop-server/tests/investigation_handoff.rs` | Set C, `P-to-I`, read-only statuses, receiver, Set R tests | 501 |
| `crates/desktop-server/tests/investigation_handoff_ui.test.mjs` | UI, fake-DOM shell integration & F12 wording tests | 382 |

## Deviations, Decisions & Assumptions

- **Submodule split (`investigation_handoff_{extract,status}.rs`)**: Split helper extraction and status projection out of `investigation_handoff.rs` via `#[path]` submodules so every source file remains comfortably below the 600-line ceiling (max `562` lines in `investigation_handoff_receive.rs`).
- **Additive `POST /api/sessions/{id}/investigation/handoff-packet` endpoint**: Added alongside F7's `POST /api/sessions/{id}/handoff` rather than modifying `crates/export/src/handoff*.rs` (not owned by this lane), keeping `HandoffRequest`'s strict `#[serde(deny_unknown_fields)]` schema intact while letting the frontend construct and immediately preview/submit verified packets cleanly.
- **`handoff_statuses` + `child_statuses` coexistence**: Preserved F7's `read_session_handoffs` `outgoing` fields untouched (keeping F7 tests' `"Opened in Solve: Not run yet"` / `"Opened in Explore: Not run yet"` assertions green) while adding `child_statuses` / `mapview.handoff_statuses` with V16 action keying (`derive_solve`, `extend_explore`, `prior_work_explore`), F12 verdict phrasing (`PriorWorkStatus` mapping + forbidden-word sanitization), and `"Opened in Explore: not finished"` / `"Opened in Solve: in progress"` summaries.
- **Right half-badge backward compatibility**: `renderInvestigationBadges` retains `aria-disabled="true"` when called standalone without `opts.onSearchWider` / `opts.enableRightSlot` (preserving standalone V7 unit tests in `investigation_orient_open_notes.test.mjs`), and enables clicking + F12-safe status text whenever V16 mounts via `renderInvestigationNodeCard` / `wirePriorWorkHalfBadge`.
- **Cache-miss incoming `investigate` behavior**: When `ask.paper` is absent from both the parent session and the local `Library`, `receive_investigate_handoff` makes zero network calls and records `investigate.status = "needs_file"` cleanly on the HTTP 201 response so the user can drop TeX/PDF locally.

## Merge Notes

- **`crates/desktop-server/assets/investigation_v3_shell.js`** and **`crates/desktop-server/assets/investigation.js`** remain at `598` lines (`2` lines below the `600`-line cap). Future additions should extract helpers into sibling modules rather than growing either file.
- **`crates/desktop-server/src/assets.rs`** is at `550` lines (`30` lines below the `580`-line threshold for extracting `assets_fonts.rs`).
- Remote **`origin/gemini/work/fixed`** moved forward during development (`0ec99d3` -> `374744eb`, removing `investigation_legacy*` in `inv-v9`), with zero overlap against any file modified or created by V16.
- Zero frozen files (`Cargo.lock`, `paper_map*`, `writeup*`, `fork.rs`, `latex.rs`, `investigation_share*`, `turn_runner.rs`, `agent_models.rs`, `http_srv.rs`, `routes_session.rs`, `routes_tools.rs`, `index.html`, `app.js`, `tests/golden/*`, `.github/*`) were modified.

## Verification Results (Pre-Report Checkpoints `5b272fa` / `22910a6`)

*(Final verification on the final report commit follows in the final commit step.)*

- **`py -3 scripts/check_file_length.py --base origin/gemini/work/fixed`**: passed (`No touched source file is over 600 lines.`)
- **Asset JS syntax (`node --check crates/desktop-server/assets/*.js`)**: all passed (`exit 0`)
- **`node --test crates/desktop-server/tests/*.test.mjs`**: `211 passed, 0 failed` (including 3 tests in `investigation_handoff_ui.test.mjs` and all V3/V7/V8/V10 suites)
- **`/home/antimajorana/.venvs/bootloops/bin/python -m unittest discover -v` (in `python/`)**: `33 passed, 0 failed`
- **`/var/tmp/research-cargo.sh inv-v16 cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`**: `0 warnings / exit 0`
- **`/var/tmp/research-cargo.sh inv-v16 cargo test --locked -p desktop-server --test investigation_handoff --test handoff_routes -j 2`**: `6 passed, 0 failed` (`4` in `investigation_handoff`, `2` in `handoff_routes`)
- **`/var/tmp/research-cargo.sh inv-v16 cargo test --workspace --locked --no-fail-fast -j 2` (baseline WSL `PATH` before adding nvm/venv `PATH`)**: `1127 passed, 5 failed, 2 #[ignore]` (`EVAL_SHEET_JSON` / `EVAL_LIVE_MANIFEST_DIR`). The 5 baseline failures were environment-specific (`ui_math` 2 failures due to Linux `node` in `~/.nvm` absent from non-login `PATH`; `solve_s7_exprs` 3 failures due to `RE_SEARCH_PYTHON` unset per `RULES.md` and default `python3` lacking `sympy`). Targeted re-runs passed completely:
  - `cargo test --locked -p desktop-server --test ui_math -j 2` with nvm `node` on `PATH`: `2 passed, 0 failed`
  - `env RE_SEARCH_PYTHON=/home/antimajorana/.venvs/bootloops/bin/python cargo test --locked -p research-tools --test solve_s7_exprs -j 2`: `5 passed, 0 failed`

## Not Run on Linux & Observed Caveats

- **Windows-only (`cfg(windows)`) & Playwright browser E2E**: AppContainer/Job-object sandboxing tests compile/run only on Windows, and live Playwright browser `.py` suites were not run in this headless verification pass; all backend HTTP/ledger tests and Node fake-DOM UI integration tests ran cleanly.
- **No pre-existing unfixed bugs introduced**: Incoming `investigate` handoffs from sessions that do not copy source files rely on `Library` cache; if neither source nor Library entry exists offline, `receive_investigate_handoff` returns `needs_file` cleanly with zero network calls.
