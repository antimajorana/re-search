# Report: `inv-v11-cited` (Investigate package V11 — Cited result without a model)

- **Base commit:** `28ac1111f99e49eab151799c629c091b7996b8c2` (`origin/gemini/work/fixed`)
- **Branch:** `gemini/modes/inv-v11-cited`
- **Toolchain on this machine:** `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, `Python 3.12.16` on Linux x86_64
- **Guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` untouched (`tool_def_golden` passes 3/3); zero model calls anywhere in V11; zero occurrences of `"verified"` (case-insensitive) in V11 API responses or JS DOM; `crates/policy`, `crates/ledger`, `net_rate.rs`, `paper_fetch.rs`, `arxiv.rs`, `literature.rs`, `http_srv.rs`, `routes_session.rs`, `desktop-server/src/investigation.rs`, `index.html`, and `app.js` all untouched; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`).

---

## 1. What was implemented (Package V11)

### Checkpoint 1 — Pure citation locator engine (`crates/research-tools/src/skeleton/`, commit `268bbe7`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/research-tools/src/skeleton/locate.rs` | 508 | new | Rule 6.9 strict citation locator (`locate_in_skeleton`, `locate_in_skeleton_with_context`, `LocateContext`, `LocateResult`, `LocateStatus`, `LocateCandidate`, `format_candidate_label`, `format_version_caveat`) and named owner-decision constants (`CITED_STATUS_*`, `NO_TEX_REASON_*`, `PROVENANCE_OUTSIDE_PAPER`, `MAX_LOCATE_CANDIDATES`). Returns `Found` only for non-PDF (`Tex1`/`Html1`) skeletons with a unique `(kind, number)` match, clean quality (`Full` for `Computed`/`Printed`, `Full`/`Partial` for `Html`/`Confirmed`), non-ambiguous parsed locator, and confirmed numbering (`Confirmed`, `Html`, `Printed`, or `Computed` with proven single shared counter and zero counter-drift). Any `Pdf1` skeleton always yields `Candidates` with physical page ranges via F14a (`PageMap::page_of` / `page_of_line`) and `ocr`/`render_math` flags (Rule 6.21), never `Found`. Image-only PDFs with empty text layer yield `NoTex { reason: "empty_text_layer" }`. |
| `crates/research-tools/src/skeleton/locate_parse.rs` | 465 | new | Deterministic citation locator parser (`parse_locator`, `ParsedLocator`) handling `Theorem`/`Thm.`, `Lemma`/`Lem.`, `Proposition`/`Prop.`, `Corollary`/`Cor.`, `Definition`/`Def.`, `Remark`, `Example`, `Assumption`, `Conjecture`, `Claim`, `Equation`/`Eq. (N)`/bare `(N)`, `Section`/`Sec.`/`§`, `Appendix`/`App.`, `Figure`/`Fig.`, `Table`, plus `ambiguous` detection for indirect/prepositional locators (`"Corollary to Theorem 2.1"`, `"Remark after Theorem 2.1"`, `"Theorem in Section 2"`) and multi-statement locators (`"Theorem 2.1, Lemma 3.2"`). Also implements token-level TeX counter-drift scanner (`detect_tex_counter_drift`) detecting `\setcounter`, `\addtocounter`, `\stepcounter`, `\refstepcounter`, `\counterwithin`, `\counterwithout`, `\numberwithin`, `\renewcommand`/`\renewcommand*`/`\def`/`\gdef`/`\edef`/`\xdef` of `\the<counter>` (handling comments, single-line `\iffalse...\fi`, multiple commands per line, and `\newtheorem` unshared counter declarations). |
| `crates/research-tools/src/skeleton/locate_tests.rs` | 432 | new | 7 unit tests covering locator parsing & priority, single shared counter TeX (`Found`), unshared counters & `\setcounter` drift (`Candidates`), `\tag`/printed & HTML confirmed bases vs unlabelled equations, PDF `pdf-1` skeletons with physical pages & scanned OCR (`ocr = true`, `render_math = false`) + compact `page_lines` fallback, kind mismatch / missing locator (`PaperFoundLocatorNotFound`) / empty image-only PDF (`NoTex` + `empty_text_layer`), and Carbon audit regressions (inline `\iffalse...\fi`, `\renewcommand*{\thetheorem}`, prepositional/compound locators, disjoint-section unshared counters). |

### Checkpoint 2 — Desktop server cited-result route & Set P5 integration tests (`crates/desktop-server/`, commit `dbc18d4`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/src/investigation_cited.rs` | 533 | new | Single-route handler `POST /api/sessions/{id}/investigation/cited` (`handle`, `handle_with_gate`) dispatching JSON `"action"`: `"lookup"` (default), `"pick"`, `"raise_budget"`, and `"inspect_node"`. Enforces zero model calls, F10 library fulltext & skeleton cache reuse (zero network on hit), Rule 6.16 fetch budget (`paper_fetch::check_fetch_budget` + RAII `InFlightFetchGuard` concurrency reservation) before any F3 network call, F3 `paper_fetch::fetch_with_gate` + `FetchedSource::ingest_into_session`, `SeenRole::CitedLookup` recording, `investigation.cited_lookup` ledger events, `investigation.note` budget raises under `ledger_write_lock`, `return_ref` (`NodeRef`), and `cited_view` payload for the in-session focused overlay. |
| `crates/desktop-server/src/investigation_cited_support.rs` | 577 | new | Supporting helpers: RAII `InFlightFetchGuard`, `load_pinned_context`, `load_or_build_cached_skeleton` with SHA-256-verified single-file path remapping across origins (`try_remap_single_file_skeleton`), `resolve_bib_to_paper_ref` (including delimiter-checked `extract_arxiv_version_from_raw` because V2 `BibRef::arxiv_id` strips version suffixes), `resolve_library_canonical`, `find_session_source_for_key`, `resolve_external_source_identity`, `build_statement_json`, `enrich_candidates`, `build_cited_view_json`, and `record_cited_in_library`. |
| `crates/desktop-server/src/investigation_cited_tests.rs` | 234 | new | Unit test suite (`cited_lookup_library_hit_unresolved_pick_and_raise_budget`) verifying bare citation -> `unresolved`, F10 pre-populated library hit (zero network, populates skeleton cache, exact version caveat, zero `"verified"` wording), second lookup hitting F10 cached skeleton, missing locator -> `paper_found_locator_not_found` + manual `pick` -> `found`, `raise_budget` writing `investigation.note`, HTTP 400 rejection of invalid `new_limit` values (`-5`, `"bad"`, `0`, `12.5`), and HTTP 400 rejection of picking the pinned primary source. |
| `crates/desktop-server/tests/investigation_cited_route.rs` | 555 | new | End-to-end Eval Set P5 & "Follow the argument" integration tests (`eval_set_p5_tex_pdf_drift_caching_overlay_and_budget`, `cited_http_route_requires_auth_and_dispatches_via_api_dispatch`) using a loopback mock HTTP server, `FakeRateClock`, and self-authored TeX + pure-Rust PDF 1.4 fixtures: confirmed TeX lookup (`Found`, verbatim match, F10 cache population, second lookup = 0 HTTP requests, cross-session F10 skeleton cache hit = 0 HTTP requests), unshared-counter drift (`Candidates`, Set P5 wrong-located count = **0**), PDF-only Paper C with digital + scanned OCR pages (`Candidates` with physical pages 2 & 3, `ocr = true`, `render_math = false`), image-only Paper D (`no_tex`, `empty_text_layer`), paywalled DOI (`no_tex`, `needs_file`), bare citation (`unresolved`), Rule 6.16 fetch budget refusing the 9th external fetch with HTTP 429 until `"raise_budget"`, in-session overlay `inspect_node` + `return_ref` preservation, zero `model.request` events, zero `"verified"` wording, and real HTTP 401/403 auth enforcement through `api_dispatch`. |

### Checkpoint 3 — Context drawer UI & Node tests (`crates/desktop-server/`, commit `30bfbf2`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/assets/investigation_context.js` | 592 | new | Context drawer controller (`createInvestigationContextDrawer`, `openCitedLookup`, `formatFetchBudgetText`, named constants for accepted owner decisions). Renders F6 `PROVENANCE_STYLES.outside_paper` cards (`Outside this paper: <short ref>`, solid coloured left border), `createQuoteChip` with line/page link and `OCR` badge, Rule 6.21 no-KaTeX rendering on OCR statements, all 5 named statuses (`found`, `candidates`, `paper_found_locator_not_found`, `no_tex`, `unresolved`), version caveat, fetch-budget counter & one-click `Allow more (+8)` button, monotonic `lookupToken`/`inspectToken` race guards, non-optimistic error-safe `handleRaiseBudget`, in-session focused Investigate overlay (`Inspect in cited paper`, neighbor traversal via `inspect_node`, `← Return to original claim` restoring exact `return_ref`), guarded `Open full paper in new Investigate session` button, and clean `mountContextSection(id, element)` / `unmountContextSection(id)` V12 seam. |
| `crates/desktop-server/tests/investigation_context.test.mjs` | 586 | new | 5 Node tests verifying constants, budget counter formatting, all 5 cited statuses, F6 `outside_paper` provenance & border styling, Rule 6.21 OCR no-KaTeX enforcement, 429 budget-exceeded & raise flow + candidate picker `onPick` transition, in-session cited overlay neighbor inspection & exact `return_ref` restoration + V12 `mountContextSection` coexistence, XSS escaping (`innerHtmlWrites == 0`), and zero `"verified"` wording. |
| `crates/desktop-server/tests/investigation_context_regressions.test.mjs` | 237 | new | 3 Node regression tests for Carbon UI audit findings: rapid-click `openCitedLookup` race protection (slower earlier response discarded, open overlay closed, stale `returnRef` cleared), `raise_budget` error handling (no unhandled rejection, no optimistic bump on failure, stale 429 card cleared on success), `open-new-session` button omitted when callback is absent, active-KaTeX stub XSS test (prose XSS HTML-escaped, `trust` not enabled), and top-level `ocr: true` fallback suppressing KaTeX. |

### Shared registration & UI wiring seams (commit `0a27579`)

| File | Lines | Diff | Responsibility |
|---|---:|---|---|
| `crates/research-tools/src/skeleton/mod.rs` | 295 | +10 | Registers `pub mod locate; mod locate_parse;` and re-exports locate types and constants. |
| `crates/desktop-server/src/lib.rs` | 224 | +2 | Registers `pub mod investigation_cited; mod investigation_cited_support;`. |
| `crates/desktop-server/src/api_dispatch.rs` | 384 | +3 | Adds exact 1 arm `("POST", ["api", "sessions", id, "investigation", "cited"])` behind `AuthGuard::authorize_api`. |
| `crates/desktop-server/src/assets.rs` | 470 | +4 | Serves `/assets/investigation_context.js` from embedded bytes. |
| `crates/desktop-server/assets/api.js` | 506 | +4 | Exports `investigationCited(id, payload)` calling `POST /api/sessions/{id}/investigation/cited`. |
| `crates/desktop-server/assets/investigation_v3_shell.js` | 581 | +106 / -7 | Persists per-session `contextDrawer`, wires citation clicks (`onSelectCitation` with `cite.locators[0] || cite.locator`), hydrates `fetchBudget`, guards stale `activeSessionId` and `return_ref.session`, removes duplicate `ensureSelectedText` call, and exposes `getContextDrawer(sessionId)` and `mountContextSection(sessionId, id, element)` on the V3 controller for V12. |
| `crates/desktop-server/assets/investigation_shell.css` | 308 | +21 | Adds layout styles for `.inv-context-drawer`, `.inv-context-budget`, `.inv-cited-card--outside` (4px coloured left border), candidate list, bib block, and `.inv-cited-overlay`. |

---

## 2. Independent Gemini 4 Carbon audit findings & resolutions

Two focused adversarial independent audits (`research` subagents on `pro` tier — one for Rust Checkpoint 1–2, one for UI Checkpoint 3 & `RULES.md`) reviewed all V11 files. Every critical, high, and medium finding was fixed and covered by regression tests before committing:

### Rust audit findings resolved
1. **Token-level TeX counter-drift scanning (`locate_parse.rs`):** Replaced line-based `\fi` / single `.find()` scanning with a token-level TeX scanner that handles single-line `\iffalse...\fi` followed by a real `\setcounter`, multiple commands on one line, `\renewcommand*{\the...}`, `\def`/`\gdef`/`\edef`/`\xdef\the...`, and `\counterwithout`.
2. **Prepositional & multi-statement locators (`locate_parse.rs`, `locate.rs`):** Added `ParsedLocator.ambiguous` for indirect locators (`"Corollary to Theorem 2.1"`, `"Remark after Theorem 2.1"`, `"Theorem in Section 2"`) and multi-statement locators (`"Theorem 2.1, Lemma 3.2"`); ambiguous locators always return `Candidates`, never `Found`.
3. **Rule 6.9 strict quality & disjoint-section counter guard (`locate.rs`):** Rejected `SkQuality::Weak` for all bases and `SkQuality::Partial` for `Printed`/`Computed`; added `single_counter_confirmed` on `LocateContext` and tightened `confirm_single_shared_theorem_counter` so two unshared theorem-like kinds in disjoint sections (without `single_counter_confirmed`) cannot return `Found`; checked `seen.insert` before skipping `Printed` nodes; rejected non-numeric computed series except contiguous appendix letters `A..Z` for sections.
4. **Fetch-budget TOCTOU concurrency race (`investigation_cited.rs`, `investigation_cited_support.rs`):** Added RAII `InFlightFetchGuard` per session so concurrent lookup requests cannot race past the session fetch limit before `source.ingested` is appended.
5. **Fail-closed guards & input validation (`investigation_cited.rs`, `investigation_cited_support.rs`):** Updated `paper_ref.key = canonical_key` on F10 resolution; failed closed if `flatten_tex_files` returns `None`; replaced `unreachable!()` with `Err(internal(...))`; rejected `external_source_id == pinned.primary_source_id` in `handle_pick`; added `empty_text_layer` / `is_skeleton_receipt` guards in `handle_pick` and `handle_inspect_node`; rejected invalid non-positive/non-integer `new_limit` with HTTP 400; moved `ledger_write_lock` before read-then-append in `handle_raise_budget`; recomputed remapped skeleton digest; added alphanumeric delimiter boundaries in `extract_arxiv_version_from_raw`.

### UI audit findings resolved
1. **Persistent per-session drawer & V12 seam (`investigation_v3_shell.js`):** Stopped recreating `contextDrawer` on every `renderShell` call (which would have wiped V12 sections and closed open overlays); persisted `st.contextDrawer` per session, re-attached `contextDrawer.element`, exposed `getContextDrawer(sessionId)` and `mountContextSection(sessionId, id, element)` (with pending-section queueing if called before initial render), hydrated `st.fetchBudget` from session/mapview data, guarded `onReturnToClaim` by `ref.session` & no-op check, guarded async callbacks against `activeSessionId !== sessionId`, supported `cite.locator` fallback, and removed the duplicate `ensureSelectedText` call in `selectNode`.
2. **Async race safety, error handling & named constants (`investigation_context.js`):** Added named constants (`FETCH_BUDGET_INCREMENT`, `BUDGET_EXCEEDED_CODE`, `BUDGET_EXCEEDED_TITLE`, `NO_TEX_NEEDS_FILE_REASON`, `NO_TEX_EMPTY_TEXT_REASON`); added monotonic `lookupToken`/`inspectToken` guards; closed any open overlay and cleared stale `activeResult`/`returnRef` at the start of `openLookup`; wrapped `onPick`, `onInspectNode`, and `handleRaiseBudget` in `try/catch` with button disabling; removed optimistic `limit` bump on error/null return; cleared stale `fetch_budget_exceeded` card once raise succeeds; required `typeof opts.onOpenNewSession === 'function'` before rendering the new-session button; honored top-level `result.ocr` / `result.render_math` fallbacks.

---

## 3. Decisions taken alone, deviations, and assumptions

### Decisions taken alone
1. **Added `"inspect_node"` action on the single `POST /api/sessions/{id}/investigation/cited` route:** The in-session focused Investigate overlay lets a reader click prerequisite/neighbor statements inside the cited paper (`"Follow the argument"`, `investigate.md` §3) without adding a second route arm to `api_dispatch.rs`. `"inspect_node"` reads verified node text and PDF page metadata directly from the external `source.ingested` already pinned in the current session, with zero model calls and zero network calls.
2. **Single-file skeleton cache path remapping (`try_remap_single_file_skeleton`):** `paper_fetch` names live arXiv single-file TeX downloads `arxiv_<target_id>.tex` whereas `Library::attach_to_session` names library copies `arxiv_<slug>.tex`. To achieve true cross-session F10 skeleton cache hits without rebuilding, `try_remap_single_file_skeleton` verifies `compute_files_root_sha256({old_path: new_file_sha}) == skel.files_root_sha256` (exact SHA-256 byte identity) before remapping span paths in memory and computing the remapped digest.
3. **External source identity resolution (`resolve_external_source_identity`):** Because `Library::attach_to_session` records `origin: "library:<PaperKey>"` (which `derive_paper_identity` treats as a file-hash key), V11 checks prior `investigation.cited_lookup` events and `library:<PaperKey>` origins before falling back to `derive_paper_identity`, ensuring consistent V2 node tags across `lookup`, `pick`, and `inspect_node`.
4. **Shared fetch-budget event schema:** `handle_raise_budget` writes the exact `investigation.note` (`{ "fetch_limit": <n>, ... }`) event format already read by `investigation_open::effective_fetch_limit` so V4 and V11 share one unified per-session fetch limit (default 8, clamped `<= 256`).
5. **Module splitting for the 600-line limit:** Split `locate.rs` / `locate_parse.rs` / `locate_tests.rs`, `investigation_cited.rs` / `investigation_cited_support.rs` / `investigation_cited_tests.rs`, and `investigation_context.test.mjs` / `investigation_context_regressions.test.mjs` so every touched and added file stays strictly `<= 600` lines (max 592 lines).

### Deviations from the design
- **Named constants for UI & status strings:** All owner-accepted status strings, provenance prefixes, budget defaults, and button labels are centralized in exported constants in `locate.rs`, `investigation_cited.rs`, and `investigation_context.js` per `RULES.md`.
- **No direct edit to `investigation_node.js`:** `investigation_node.js` already provided the `onSelectCitation(cite)` callback hook, so citation-chip clicks were wired cleanly in `investigation_v3_shell.js` without needing to touch `investigation_node.js`.

### Assumptions
- **V18 HTML number confirmation:** `locate.rs` honors `SkeletonBuilder::Html1`, `NumberBasis::Html`, and `NumberBasis::Confirmed`; once V18 populates those in F10 skeletons, `locate_in_skeleton` will automatically upgrade matching citations to `Found` without changes to V11.
- **F13a user-library files:** Local library files registered in F10 with matching `PaperKey` or exact normalized title resolve locally with zero network requests.
- **V4/F7 "Open full paper in new Investigate session":** The UI button renders only when `hooks.openPaperInNewSession` is supplied by the host shell (preventing a dead button until V16/F7 wires that hook into `app.js`).

---

## 4. Merge notes

### Shared existing files edited (all isolated in commit `0a27579`)
- **`crates/research-tools/src/skeleton/mod.rs`** (lines 11–12, 38–45): declares `pub mod locate; mod locate_parse;` and re-exports locate symbols.
- **`crates/desktop-server/src/lib.rs`** (lines 7–8): declares `pub mod investigation_cited; mod investigation_cited_support;`.
- **`crates/desktop-server/src/api_dispatch.rs`** (lines 291–293, `dispatch_api`): adds exact 1 match arm `("POST", ["api", "sessions", id, "investigation", "cited"]) => write_api_res(&mut stream, crate::investigation_cited::handle(state, id, &req.body))` directly above the generic `("POST", ["api", "sessions", id, "investigation", operation])` arm.
- **`crates/desktop-server/src/assets.rs`** (lines 210–213, `lookup_asset`): registers `"/assets/investigation_context.js"`.
- **`crates/desktop-server/assets/api.js`** (lines 221–223): exports `investigationCited(id, payload)`.
- **`crates/desktop-server/assets/investigation_v3_shell.js`** (`createInvestigationV3Controller`, `renderShell`): persists `st.contextDrawer`, wires `onSelectCitation` -> `contextDrawer.openCitedLookup`, and exposes `getContextDrawer(sessionId)` and `mountContextSection(sessionId, id, element)` on the controller.
- **`crates/desktop-server/assets/investigation_shell.css`** (lines 285–305): adds `.inv-context-*` and `.inv-cited-*` styles.

### Integration interfaces for downstream briefs
- **V12 (`inv-v12-context-cards`):** Call `controller.mountContextSection(sessionId, 'v12-context-cards', sectionEl)` on `globalThis.__reSearchInvestigationV3` (or `drawer.mountContextSection(id, el)` via `controller.getContextDrawer(sessionId)`). Sections persist across node clicks and shell re-renders and coexist cleanly below the V11 cited-result card and overlay. Cited external sources ingested by V11 already write standard `source.ingested` receipts into the current session so F4 `verify_quote` works directly on them in V12.
- **V16 (`inv-v16-handoffs`):** Pass `hooks.openPaperInNewSession(citedResult)` into `syncSession(sessionId, sessionData, hooks)` to enable the optional `"Open full paper in new Investigate session"` button inside both the cited card and the in-session cited overlay.
- **V18 (`inv-v18-html-numbers`):** Populate `NumberBasis::Html` / `NumberBasis::Confirmed` on `PaperSkeleton` nodes stored in F10; `locate_in_skeleton` immediately accepts them as confirmed under Rule 6.9.

---

## 5. Verification, test counts, and pre-existing findings

### Commands run and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `cargo test --locked -p research-tools --test tool_def_golden` | **Pass** — **3 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p research-tools --lib skeleton::locate` | **Pass** — **7 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p desktop-server --lib investigation_cited` | **Pass** — **1 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p desktop-server --test investigation_cited_route` | **Pass** — **2 passed, 0 failed, 0 ignored** |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — 0 syntax errors |
| `node --test crates/desktop-server/tests/investigation_context.test.mjs crates/desktop-server/tests/investigation_context_regressions.test.mjs` | **Pass** — **8 passed, 0 failed, 0 ignored** |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **175 passed, 0 failed, 0 ignored** |
| `cargo clippy -p research-tools --lib -- -D warnings` | **Pass** — **0 warnings, 0 errors** |
| `cargo clippy -p desktop-server --test investigation_cited_route -- -D warnings -A clippy::manual-unwrap-or` | **Pass** — **0 warnings, 0 errors** (`-A clippy::manual-unwrap-or` solely to bypass pre-existing `evidence.rs:744`) |
| `cargo clippy --locked --workspace --all-targets --no-deps` | **6 pre-existing warnings only** (0 warnings in any V11 file; proved identical on base commit below) |
| `cargo test --locked --workspace --no-fail-fast` | **Pass** — **921 passed, 0 failed, 2 ignored** across 116 targets (the 2 ignored are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | **Pass** — **13 passed, 0 failed, 0 ignored** across 4 targets |

### Pre-existing clippy warnings (proved on base commit `28ac1111f99e49eab151799c629c091b7996b8c2`)

Running `cargo clippy --locked --workspace --all-targets --no-deps` in a clean detached worktree at base commit `28ac1111f99e49eab151799c629c091b7996b8c2` under `rustc 1.100.0` produced the exact same 6 warnings (and zero additional warnings were introduced by V11):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
5. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — unused imports on Linux (`TEX_AUDIT`, `is_symlink_or_junction`).
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — unused imports on Linux (`BufRead`, `BufReader`).

### What could not be compiled or run on Linux
- `#[cfg(windows)]` code paths and tests (`crates/desktop-app`, `crates/worker` AppContainer/Python worker, `crates/chatgpt-auth` DPAPI, `crates/research-tools/src/sandbox.rs` NTFS junctions, Windows Playwright browser tests). All V11 Rust and JS code is platform-neutral with zero Unix-only paths or shell-outs.
- Live external arXiv/DOI/journal fetches (tests strictly use loopback mock HTTP + `FakeRateClock` + self-authored fixtures per `RULES.md`).

---

## 6. Bugs noticed (not fixed), design notes, and limitations

- **Pre-existing test interaction with `RE_SEARCH_PYTHON` on Linux (`crates/cli/tests/desktop_and_ext.rs:97`):** Running `RE_SEARCH_PYTHON=$(command -v python3) cargo test --workspace` causes `cli_research_workspace_read_fork_diff_export_and_verify` to hang because `re-search --route api` rejects a non-Windows `--python` / `RE_SEARCH_PYTHON` path and exits before connecting to `ScriptedMock`, leaving `ScriptedMock::finish()` blocked in `listener.accept()`. Running `cargo test --locked --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` and running `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` separately passes 100% (921 + 13 passed, 0 failed). Left untouched because `crates/cli/tests/desktop_and_ext.rs` is outside V11 scope.
- **Existing V2 behavior (`crates/research-tools/src/skeleton/bib.rs`):** `BibRef::arxiv_id` strips version suffixes (e.g. `arXiv:2601.00055v2` -> `"2601.00055"`). Rather than editing V2 `bib.rs`, V11 recovers any explicit version suffix from `BibRef::raw` via `extract_arxiv_version_from_raw` so F3 fetches the cited version and the version disclaimer shows the exact `vN`.
- **Design contradictions found:** None beyond the three already documented and resolved in `inv-v11-cited.md` §8 (`no_tex` vs V17 PDF candidates, in-session cited overlay vs child-session hand-off, and V11/V12 ownership of `investigation_context.js`).
- **What does not work yet (out of V11 scope):**
  - Model-generated context cards and `"Hide unchecked"` toggle (V12).
  - arXiv HTML number confirmation (`html-1` / V18 — V11's `locate.rs` already honors `Html1`/`NumberBasis::Html` once V18 writes them).
  - OCR generation for image-only scanned PDFs (F14b — V11 honestly emits `status: "no_tex"` with `reason: "empty_text_layer"` and an attach-file prompt).
  - Live `"Open full paper in new Investigate session"` host callback in `app.js` (V16/F7 — V11 hides the button until `hooks.openPaperInNewSession` is provided).
