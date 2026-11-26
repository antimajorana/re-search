# Report: `inv-a-v2-v5` (Investigate Lane A — Packages V2, V3, V4, V5)

- **Brief:** `docs/design/modes-2026-11/agents/inv-a-v2-v5.md`
- **Base commit:** `4e0fbaaf4ef51cf83e7fa3d65b484cb4709a3431` (`origin/gemini/work/fixed`)
- **Intermediate base merges:** `c7ba452` (after V2), `cf2544d` (after V3, includes integrator merge of V2 + `275fbcb`), `7f3fb65` (after V4, includes F5 `api_dispatch.rs` / `session_mode.rs` / `new_menu.js`, F11, X1, S3, S6)
- **Branch pushed:** `gemini/modes/inv-a-v2-v5` (pushed incrementally after V2, V3, V4, V5, and final report)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `node v22.23.5`, `Python 3.12.16` on Linux x86_64
- **Guard rails preserved:**
  - Zero new dependencies; `Cargo.lock` untouched (`git diff` empty).
  - Zero tool definitions added or changed; `crates/research-tools/tests/golden/tool_defs.txt` untouched; `tool_def_golden` and `tool_profile_golden` pass.
  - Zero model calls (`model.request` events) anywhere across V2–V5.
  - `crates/policy` and `crates/ledger` untouched (free of `tokio`, `rmcp`, `windows-sys`).
  - `routes_session.rs`, `http_srv.rs`, `turn_runner.rs`, `index.html`, and `app.js` untouched.
  - Every touched source file is at or under 600 lines (`scripts/check_file_length.py --base origin/gemini/work/fixed` passes; max touched file is 540 lines).
  - User-facing Investigate output never contains `"verified"` / `"Verified"` (quotes are `"matched"`, dashed means `"not checked by code"`).

---

## 1. What was implemented, package by package

### Package V2 — Edges, tags, reading stats (`crates/research-tools`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/research-tools/src/skeleton/tags.rs` | 284 | new | Single permanent `KIND_CLASS_TABLE` covering all 18 `SkNodeKind` variants (`theorem`/`lemma`/`proposition`/`corollary`/`conjecture`/`claim` -> `"statement"`, `definition`/`assumption` -> `"definition"`, `example`/`remark` -> `"remark"`, etc.), `kind_class`, `assign_tags` calling `paper_key::node_tag` unchanged (first-build tags only; `#2`/`#3` collision suffixing for identical unlabelled bodies), parent tag wiring, and a golden test pinning both `KIND_CLASS_TABLE` and SHA-256/base32 outputs independently verified via Python. |
| `crates/research-tools/src/skeleton/number.rs` | 481 | new | LaTeX counter simulation (`assign_numbers`) in document order: sections (`part`..`subsubsection`, starred unnumbered), `\appendix` alphabetic lettering (`A`, `A.1`, ...), `\numberwithin`/`\counterwithin`, theorem-like shared & parent counters (including `\newenvironment`/`\renewenvironment` wrapper counter aliasing so `boxedres` shares `mainres`), equations preserving `NumberBasis::Printed` `\tag{...}` without incrementing, figures, and tables. |
| `crates/research-tools/src/skeleton/bib.rs` | 472 | new | Deterministic bibliography extraction (`extract_bibliography`) from inline `thebibliography`/`\bibitem`, `.bib` files (nested-brace titles), and non-inlined `.bbl` files; arXiv ID and DOI canonicalization via `paper_key`; comment/verbatim/`\iffalse`-safe active `\cite*` scanning with 1- and 2-optional `[pre][post]` locators (`BibLocator`), deduplicated `cited_at`, and cited-only key preservation. |
| `crates/research-tools/src/skeleton/edges.rs` | 518 | new | Positional dependency edges adapted from KnowTeX (Apache-2.0, credited in file header): proof linking (`proof_of` + `Proves` via `HeaderRef`, unique `HeaderMention`, `Adjacent`; rule 6.2 at most one target per proof), `\ref`/`\eqref`/`\cref`-family `Uses` edges split into `Statement`/`Proof`/`Prose` roles, `Lifted` edges moving equation references to enclosing nodes, heuristic `TextMention` edges, `HeadCite`/`Cite` edges, `intro_refs` flag, deduplication, and `MAX_SKELETON_EDGES` (20,000) truncation. (Merged integrator fix `275fbcb`: duplicate labels left unresolved.) |
| `crates/research-tools/src/skeleton/stats.rs` | 273 | new | Comment- and command-safe TeX word counting (`count_tex_words`), `estimate_read_minutes` (`MATH_READ_WORDS_PER_MIN = 180.0`, `EQ_READ_MINUTES = 0.25`), per-node `words`/`read_minutes`, and per-section `SectionStats` (with `section_tag: None` bucket for top-level non-section nodes). |
| `crates/research-tools/src/skeleton/mod.rs` | 270 | edited | Wired V2 passes (`assign_tags`, `assign_numbers`, `extract_bibliography`, `build_edges`, `compute_stats`) and V5 (`extract_symbols`) into `build_tex_skeleton_timed`, maintaining referential integrity when the 2 MiB JSON cap truncates nodes. |
| `crates/research-tools/tests/skeleton_v2.rs` | 493 | new | Hand-written expected edges, proof links, bib entries/locators, stats, and cross-version tag stability across all 8 V1 fixtures (9 dirs), a self-authored cross-reference test paper (KnowTeX roles, lifted equations, text mentions, `\appendix`, `\numberwithin`, `.bib`, `intro_refs`), duplicate-label ambiguity check (`275fbcb`), and an 80-page paper timing benchmark. |
| `crates/research-tools/tests/fixtures/skeleton/*/expected.json` | 9 files | edited | Extended every V1 fixture's `expected.json` with hand-derived `number` and `number_basis` values read directly from the fixture `.tex` files. |
| `crates/research-tools/tests/skeleton_tex.rs` & `skeleton_props.rs` | 519, 288 | edited | Replaced V1-only placeholder `assert!(tag.is_none())` checks with stronger V2 invariants (`n-` prefix, 12-char length, uniqueness, parent tag match, CRLF edge invariance across all fixtures and 200 fuzz seeds). |

### Package V3 — Skeleton on screen (`crates/desktop-server`)

F6's shared UI modules (`graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js`) were present in the checkout, so both the Rust half and the UI half were delivered.

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/src/investigation_mapview.rs` | 464 | new | Pure `PaperSkeleton` -> shared `GraphView` JSON projection (`00-overview.md` §2.6, zero node body text per `investigate.md` §5.2): nodes (`id`, `kind`, `label`, `group`, `parent`, `grade: "code"`, `badges`, `"cites [...] in its heading"` flags, `proof_tag`, `cites`), node-to-node edges (`src`, `dst`, `basis: "code"` for explicit / `"model"` for heuristic), `legend`, `quality_line`, `symbols`, `skip_plan` (+ `build_mapview_json_with_plan`), and hash-checked physical span reader `read_verified_node_text`. Unit-tested for `GraphView` shape, zero body text, and zero `"verified"` wording. |
| `crates/desktop-server/src/investigation_skeleton.rs` | 469 | new | Host-only `investigation.skeleton` event + blob persistence (`Ledger::put_blob`), `derive_paper_identity` (using F2 `PaperKey`), `build_and_record_skeleton` (Rule 6.13 stored-skeleton preservation, Rule 6.2 missing/corrupt blob verification & rebuild-digest check), `pin_source_and_build_skeleton`, `ensure_skeleton_on_open` (in-place upgrade of old TeX sessions on open, graceful fallback on missing/changed files or non-TeX sources), and `handle`/`handle_with_etag` for `GET/POST skeleton`, `GET mapview`, `GET node-text` with `ETag`/`If-None-Match` (`304`). |
| `crates/desktop-server/assets/investigation_outline.js` | 252 | new | Left outline tree (`buildOutlineTree`, `createQualityBanner`, `renderInvestigationOutline`) built on F6 `ui_badge.js`: quality line banner, reading minutes, result counts, collapsible section subtrees, zero `innerHTML`. |
| `crates/desktop-server/assets/investigation_results.js` | 293 | new | "Results in this paper" list (`filterResultNodes`, `renderInvestigationResults`) with kind filters, heading-cite flags, dependency counts, safe TeX statement rendering via `renderMathTextInto`, and theorem-less physics paper fallback showing sections & labelled equations. |
| `crates/desktop-server/assets/investigation_node.js` | 329 | new | Node Card Block A (`collectNodeRelations`, `renderInvestigationNodeCard`) built on F6 `createNodeCardShell`: verbatim statement, inline proof toggle, heavy solid Statement uses / light solid Proof uses / dashed heuristic chips, Used by chips, citations with locators, action bar (`Source lines`, `PDF page`, `Proof`, `Note`, `Ask`, `Derive in Solve`, `Has this been extended?`), and dashed Block B placeholder. |
| `crates/desktop-server/assets/investigation_shell.css` | 287 | new | Responsive 3-column Investigate layout (collapsing to single column under `900px` per `investigate.md` §3) using `tokens.css` variables, plus styles for `.inv-*` and F6 `.ui-*` components. |
| `crates/desktop-server/tests/investigation_v3_ui.test.mjs` | 333 | new | Node UI tests for outline, results, node card Block A, F6 `createGraphView` mounting of V3 mapview JSON, physics fallback, XSS payloads, zero `innerHTML` writes, and `"verified"`-free check. |

### Package V4 — Open by arXiv id, TeX first (`crates/desktop-server`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/src/investigation_open.rs` | 540 | new | Host-only user-initiated `open` / `open-status` routes built on F3 `paper_fetch` and F10 `Library`: accepts `arxiv_id`, `doi`, `url`, `input`, `library_key` (zero-network open from F10 library), or `source_id` (zero-network open of session source); enforces session fetch budget (`DEFAULT_INVESTIGATE_FETCH_LIMIT = 8`, raisable via `investigation.note.fetch_limit`, HTTP `429` on overflow); returns immediately with `job_id` and tracks visible steps (`queued` -> `metadata` -> `library_lookup`/`eprint_tex`/`pdf_fallback` -> `ingest` -> `skeleton` -> `done`/`needs_file`/`failed`) with session-scoped polling and zero model calls. |
| `crates/desktop-server/assets/investigation_entry.js` | 297 | new | Investigate entry card (`detectPaperInput`, `renderMetadataCard`, `renderOpenProgress`, `renderInvestigationEntry`) built on F6 `ui_badge.js`: single input + `"Open"` button, paste detection restricted strictly to paper identifiers/links, Stage B metadata card, visible step progress + fetch budget counter, `needs_file` drop prompt, recent library list, zero `innerHTML`. |
| `crates/desktop-server/tests/investigation_open.rs` | 391 | new | Loopback-mock Rust integration tests (using `FakeRateClock`, zero real network/sleeps/model calls) covering arXiv TeX open + zero `model.request`, paywalled DOI `needs_file`, zero-network `library_key` and `source_id` open, cross-session job isolation, and 9th-fetch `429` budget refusal + raise via `investigation.note`. |
| `crates/desktop-server/tests/investigation_v4_entry.test.mjs` | 173 | new | Node UI tests for `detectPaperInput`, entry form submit, library click, `needs_file`, metadata card, XSS escaping, zero `innerHTML`, and `"verified"`-free check. |

### Package V5 — Skip plan, symbols (`crates/research-tools`, `crates/desktop-server`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/research-tools/src/skeleton/skip.rs` | 393 | new | Pure `compute_skip_plan(skeleton, main_result_tags, extra_edges)` enforcing Rule 6.14: empty/non-dimming state with no main results, statement & proof closures over explicit `Uses` edges, heuristic `TextMention` excluded unless `NumberBasis::Confirmed`, `SkipBasis` switching to `IncludesModelInferred` strictly when a model-inferred edge changes a section's status, non-double-counted top-level section reading times, and honest `"by cross-references"` wording (never `"not needed"` or `"verified"`). |
| `crates/research-tools/src/skeleton/symbols.rs` | 436 | new | `normalize_symbol`, `symbol_lookup_keys`, `extract_lhs_symbol`, `extract_symbols` (preamble `\newcommand`/`\DeclareMathOperator` + body `DefinePhrase` patterns, comment/verbatim/`\iffalse`-safe), and `lookup_nearest_symbol` returning nearest earlier definition (`"found by pattern"`) or `"No definition found above this line"`. Wired into `build_tex_skeleton_timed`. |
| `crates/desktop-server/assets/investigation_skip.js` | 335 | new | Reading-plan strip (`renderReadingPlanStrip` with empty state, explicit solid vs model-inferred dashed basis badge, two reading times, dim toggle, section jump pills) and instant client-side symbol lookup + popover (`normalizeSymbol`, `lookupNearestSymbol`, `renderSymbolPopover`, measured 0.8 ms on 5,000 symbols, << 100 ms target). |
| `crates/desktop-server/tests/investigation_v5_skip.test.mjs` | 234 | new | Node UI tests for symbol normalization/scoping/macro-stem matching, reading-plan strip states, symbol popover found/not-found, XSS payloads, zero `innerHTML`, and `"verified"`/`"not needed"` ban. |

### Minimal existing-file wiring edits

| File | Lines | Change |
|---|---:|---|
| `crates/desktop-server/src/lib.rs` | 221 | Added 3 module declarations: `pub mod investigation_mapview;`, `pub mod investigation_open;`, `pub mod investigation_skeleton;`. |
| `crates/desktop-server/src/assets.rs` | 462 | Added 6 static asset arms in `lookup_asset`: `/assets/investigation_outline.js`, `/assets/investigation_results.js`, `/assets/investigation_node.js`, `/assets/investigation_shell.css`, `/assets/investigation_entry.js`, `/assets/investigation_skip.js`. |

---

## 2. Measured skeleton build timings

All timings below were measured on this Linux x86_64 machine in **unoptimized debug builds** (`cargo test --locked -p research-tools --test skeleton_v2 --test skeleton_tex -- --nocapture`) with the full V1 + V2 + V5 pipeline active (flattening, TeX scanning, permanent tags, counter simulation, bibliography/locator extraction, KnowTeX edges + lifted edges + text mentions, reading stats, symbol extraction, and 2 MiB JSON sizing):

| Paper / fixture | Flattened lines | Nodes | Edges | Bib | JSON bytes | Flatten | Scan | Total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `01_amsthm_shared_counters` | 55 | 12 | 3 | 1 | 6,350 | 0.66 ms | 2.11 ms | 16.61 ms* |
| `02_custom_theorem_names` | 34 | 8 | 1 | 0 | 3,370 | 0.41 ms | 1.06 ms | 5.67 ms |
| `03_multi_file` | 34 | 9 | 1 | 1 | 4,329 | 0.47 ms | 0.94 ms | 13.67 ms* |
| `04_revtex_physics` | 38 | 11 | 0 | 0 | 4,500 | 0.46 ms | 1.14 ms | 4.99 ms |
| `05_detached_proof` | 35 | 11 | 5 | 1 | 5,482 | 0.43 ms | 12.29 ms* | 17.06 ms* |
| `06_unlabelled_theorems` | 36 | 8 | 1 | 0 | 3,184 | 0.34 ms | 0.84 ms | 3.77 ms |
| `07_two_versions/v1` | 22 | 5 | 0 | 0 | 2,117 | 0.24 ms | 0.57 ms | 2.63 ms |
| `07_two_versions/v2` | 26 | 6 | 0 | 0 | 2,608 | 0.29 ms | 0.74 ms | 3.13 ms |
| `08_hard_edge_cases` | 48 | 7 | 1 | 1 | 3,534 | 0.45 ms | 0.93 ms | 4.11 ms |
| **Generated 80-page paper** (20 sections, defs/lemmas/thms/proofs/eqs/refs/cites/20-entry bib) | 3,893 | 302 | 901 | 20 | 282,423 | 52.78 ms | 92.68 ms | **580.22 ms** |
| **Generated 5,000-line paper** (`skeleton_tex.rs`) | 5,000 | 1,720 | — | 0 | 762,577 | 34.95 ms | 130.48 ms | **529.49 ms** |

\*Includes one-time `OnceLock<Regex>` compilation on the first fixture hitting each regex path in the test process. Even unoptimized debug builds complete an 80-page cross-referenced paper in **0.58 s** (well under the 10 s target).

---

## 3. Deviations, decisions taken alone, and assumptions

### Deviations from design
1. **No `contains` edges emitted in `SkEdge`:** `investigate.md` §4.1 lists `contains` in `SkEdgeKind`, while `SkEdgeRule` has no `contains` variant and every `SkNode` already carries `parent: Option<tag>`. Emitting only `Uses`, `Proves`, and `Cites` avoids doubling edge counts against the 20,000-edge cap while preserving exact containment on `SkNode::parent`.
2. **Additional `GET .../investigation/node-text?tag=` helper endpoint:** `investigate.md` §5.2 requires `GET .../investigation/mapview` to omit node prose text, while V3 UI displays verbatim statements in the results list and node card Block A. Adding a read-only `node-text` handler inside `investigation_skeleton.rs` keeps `mapview` prose-free while serving hash-checked physical spans on demand.
3. **V3 Rust module split into two files (`investigation_mapview.rs` + `investigation_skeleton.rs`):** Split to keep both files comfortably below the 600-line rule (464 and 469 lines).
4. **Route registration deferred to Merge notes:** Per `RULES.md` and the brief, `http_srv.rs` (and F5's extracted `api_dispatch.rs`), `routes_session.rs`, `turn_runner.rs`, `index.html`, and `app.js` were not edited.

### Decisions taken alone
1. **Permanent kind-class grouping (`tags.rs::KIND_CLASS_TABLE`):** Grouped `Theorem | Lemma | Proposition | Corollary | Conjecture | Claim` -> `"statement"`, `Definition | Assumption` -> `"definition"`, and `Example | Remark` -> `"remark"`, keeping all other 8 kinds distinct (`front_matter`, `section`, `proof`, `equation`, `figure`, `table`, `algorithm`, `bibliography`). This preserves tags across lemma-to-theorem promotion while preventing collisions between proofs, equations, and statements.
2. **Whole-skeleton collision suffixing (`#2`, `#3`) for identical unlabelled bodies:** Tracking `(kind_class, base_key)` occurrences across the full skeleton (rather than per parent alone) guarantees unique tags even when identical unlabelled remarks appear in different sections.
3. **Wrapper counter aliasing in `number.rs`:** Worked around V1's `tex_theorems.rs:468` (which clones `TheoremDef` with `shared_counter: None` for `\newenvironment` wrappers around declared theorems) by scanning `\newenvironment` aliases in `number.rs`, keeping V1's `expected.json` `theorem_defs` untouched while giving `boxedres` in fixture 02 the true LaTeX number `1.2`.
4. **Heuristic edges mapped to `basis: "model"` in `GraphView`:** In `investigation_mapview.rs`, `EdgeConfidence::Heuristic` skeleton edges (`TextMention`, heuristic `ProofHeader`) map to `basis: "model"` so F6's `graph_view.js` automatically draws them dashed ("not checked by code", overview §2.6 & §3.2).
5. **Reading-speed constants (`stats.rs`):** Chose `MATH_READ_WORDS_PER_MIN = 180.0` and `EQ_READ_MINUTES = 0.25` (15 s per display equation), rounded to 2 decimal places.
6. **Fetch-budget raise storage (`investigation_open.rs`):** `effective_fetch_limit` reads `payload["fetch_limit"]` from `investigation.note` events (clamped to `<= 256`), implementing Rule 6.16 without introducing a new event kind.

### Assumptions
- `investigate.md` §5.2 specifies `ETag = blob hash` for `GET .../skeleton` and `ETag = ledger head` for `GET .../mapview`; `handle_with_etag` returns `(status, Option<String>, Value)` so `api_dispatch.rs` can attach the `ETag` HTTP header and pass `If-None-Match` when wired.
- When `library_key` opens a PDF-only paper or arXiv falls back to PDF in V4, `investigation_open.rs` ingests and pins `investigation.paper` and returns `status: "done"` with `skeleton_seq: None` and an explicit note that PDF skeleton generation awaits V17.
- `SymbolDef` phrase patterns (`let $x$ be`, `where $x$ is`, `$x := ...$`, etc.) are heuristic; all UI surfaces mark them `"found by pattern"`.

---

## 4. Merge notes

### Required route registrations (owned by the foundation lane)
During our work, F5 (`7f3fb65`) moved API routing out of `crates/desktop-server/src/http_srv.rs` into **`crates/desktop-server/src/api_dispatch.rs`** (around line 253, right above `("POST", ["api", "sessions", id, "investigation", operation])`). Add two arms:

```rust
// V3 skeleton, mapview, and hash-checked node text:
(method, ["api", "sessions", id, "investigation", op @ ("skeleton" | "mapview" | "node-text")]) => {
    write_api_res(&mut stream, crate::investigation_skeleton::handle(state, method, id, op, &req.query, &req.body));
}
// V4 user-initiated open and job-progress poll:
(method, ["api", "sessions", id, "investigation", op @ ("open" | "open-status")]) => {
    write_api_res(&mut stream, crate::investigation_open::handle(state, method, id, op, &req.query, &req.body));
}
```
*(Optional ETag enhancement: pass `req.header("if-none-match")` to `investigation_skeleton::handle_with_etag` and emit `ETag` / `304 Not Modified`.)*

### Required UI mount wiring (owned by `index.html` / `app.js` lane)
All 6 static assets are already registered in `crates/desktop-server/src/assets.rs`:
- Link `<link rel="stylesheet" href="/assets/investigation_shell.css">` in `crates/desktop-server/assets/index.html`.
- Mount `renderInvestigationEntry` (`/assets/investigation_entry.js`) in the Investigate entry card, and mount `renderInvestigationOutline` (`/assets/investigation_outline.js`), `renderInvestigationResults` (`/assets/investigation_results.js`), `renderInvestigationNodeCard` (`/assets/investigation_node.js`), and `renderReadingPlanStrip` / `renderSymbolPopover` (`/assets/investigation_skip.js`) in the Investigate session shell.

### Small edits made to existing files (isolated commits)
- `crates/research-tools/src/skeleton/mod.rs` (wired V2 + V5 passes into `build_tex_skeleton_timed`).
- `crates/research-tools/tests/skeleton_tex.rs`, `skeleton_props.rs`, `tests/fixtures/skeleton/*/expected.json` (upgraded V1 unset-tag checks to V2 invariants + hand-derived numbers).
- `crates/desktop-server/src/lib.rs` (+3 `pub mod` declarations).
- `crates/desktop-server/src/assets.rs` (+6 static asset match arms).

---

## 5. Verification, test counts, and pre-existing findings

| Command | Outcome |
|---|---|
| `cargo test --workspace --no-fail-fast` | **Pass** — **886 passed, 0 failed, 2 ignored** across 111 test targets (the 2 ignored tests are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`). |
| `cargo test --locked -p research-tools --lib -- skeleton` | **Pass** — **23 passed, 0 failed** (`tags`, `number`, `bib`, `edges`, `stats`, `skip`, `symbols`). |
| `cargo test --locked -p research-tools --test skeleton_v2 --test skeleton_tex --test skeleton_props --test tool_def_golden --test tool_profile_golden` | **Pass** — **26 passed, 0 failed** (4 + 12 + 4 + 3 + 3). |
| `cargo test --locked -p desktop-server --lib` | **Pass** — **81 passed, 0 failed** (includes all existing `investigation*` tests + 5 new V3 tests). |
| `cargo test --locked -p desktop-server --test investigation_flow --test investigation_open --test eval_set_r` | **Pass** — **13 passed, 0 failed** (4 + 4 + 5). |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — 0 syntax errors across all JS assets. |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **167 passed, 0 failed, 0 skipped** (includes 6 new tests across `investigation_v3_ui.test.mjs`, `investigation_v4_entry.test.mjs`, `investigation_v5_skip.test.mjs`). |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `cargo clippy -p research-tools --lib --test skeleton_v2 --test skeleton_tex --test skeleton_props -- -D warnings` | **Pass** — 0 warnings, 0 errors. |
| `cargo clippy -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors. |
| `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or -A unused_imports -A clippy::items_after_test_module` | **Pass** — 0 warnings, 0 errors across the entire workspace. |

### Pre-existing clippy warnings (untouched files, documented on base commit in `RULES.md`, `v1-skeleton.md`, `f3-net-gate.md`, `f6-graph-ui.md`)
- `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`, documented in `RULES.md`)
- `crates/research-tools/tests/sandbox_and_fs.rs:6` (`unused_imports` on Linux where Windows junction tests are `cfg(windows)`)
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (`unused_imports` on Linux)
- `crates/agent/src/guardrails.rs:335` (`clippy::items_after_test_module` on Rust 1.100.0)
- `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`) and `sheet.rs:102` (`clippy::unnecessary_map_or`)

None of these files were touched by this brief.

---

## 6. What could not be compiled or run on Linux / What does not work yet

- **Not compiled/run on Linux:** `#[cfg(windows)]` targets (`crates/desktop-app` WebView2 shell, `AppContainerSandbox` / `python_worker` tests in `crates/worker`, NTFS junction tests in `crates/research-tools`, Windows DPAPI vault).
- **Not run in a real browser:** Headless browser scripts (`*_browser.py`) were not run; JS UI components were verified under Node's fake DOM in `node --test`.
- **Not yet live end-to-end in the desktop UI until Merge notes are applied:** Because `api_dispatch.rs` / `http_srv.rs`, `index.html`, and `app.js` are outside this lane's write scope, the new `/investigation/{skeleton,mapview,node-text,open,open-status}` handlers and the 6 new UI modules are compiled, asset-registered, and unit/integration tested, but not yet mounted into the running desktop page until the foundation lane adds the two dispatch arms and HTML/JS mount hooks.
- **PDF-only papers:** V4 ingests and pins PDF papers cleanly, but PDF skeleton construction awaits V17 (`skeleton/pdf.rs`).

---

## 7. Bugs noticed in existing code (not fixed, out of scope)

- **`crates/research-tools/src/skeleton/tex_theorems.rs:468`:** When `\newenvironment{boxedres}{\begin{mainres}}{\end{mainres}}` wraps an already-declared theorem environment in `table.defs`, line 468 clones `existing` (`TheoremDef { env_name, ..existing }`), leaving `shared_counter: None` instead of `existing.shared_counter.or(Some(inner))` (unlike line 474 for built-in environments). Worked around cleanly in `skeleton/number.rs` via wrapper alias scanning without altering V1's `expected.json` `theorem_defs`.
- **`crates/cli/tests/desktop_and_ext.rs:149-168`:** Pre-existing deadlock on Linux if `RE_SEARCH_PYTHON` is exported globally (also recorded in `v1-skeleton.md` and `f3-net-gate.md`).

---

## 8. Contradictions or impossibilities found in the design

None blocking V2–V5. One minor evolution noted: `investigate.md` §9 row V3 names `http_srv.rs` for the route registration line, whereas F5 (`7f3fb65`) extracted the API match table into `crates/desktop-server/src/api_dispatch.rs`; the Merge notes above reflect the current `api_dispatch.rs` location.
