# Brief draft-inv-v18-v19-later — Investigate arXiv HTML Skeleton (V18) and Retire Old Recorders (V19)

> **Gated later packages — do NOT launch V18 or V19 until their explicit start & owner gates below are satisfied.** V18 and V19 are independent; execute them as two separate runs/branches (`inv-v18-html-skeleton` and `inv-v19-retire-recorders`) when each gate clears.

## 1. Packages and governing design sections

- **Package V18 (arXiv HTML skeleton):** second deterministic skeleton source (`SkeletonBuilder::Html1`) built from `https://arxiv.org/html/<id>` (LaTeXML HTML5) when TeX source is missing or weak, PLUS cross-confirmation upgrading TeX `NumberBasis::Computed` -> `NumberBasis::Confirmed` when TeX and arXiv HTML theorem numbers agree (directly improving V11 rule 6.9 lookup precision).
- **Package V19 (Retire old recorders):** introduce a new Investigate tool profile (`investigation/3`) through F1 profile data that omits the 4 legacy recorders (`record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`) — without adding any hand-written rule to `resume.rs` — and retire the old map/branch/conventions write routes and prompts while preserving read-only display of historical sessions.
- **Governing design:**
  - `00-overview.md`: 0.1, 2.2 (F1 tool profiles & frozen `research/1` / `investigation/1`), 2.4 (F3 network gate), 3.1 row 1, 7.2–7.3 (Set P, Set P5, Set R), 8.2 (#1 profiles), 8.3 (#8 removing legacy tools waits until earlier Research sessions are retired).
  - `investigate.md`: 0.1 (`roxmltree` is XML-only; `arxiv.org/html` is allow-listed), 0.3 #10 (retiring old recorders gives the reader nothing early; moved to end as owner decision), 3 (stage C fallback TeX -> arXiv HTML -> PDF), 4.1 (`SkeletonBuilder::Html1`, `SkConfidence::HtmlClass`, `NumberBasis::Html` / `Confirmed`), rule 6.9 (HTML number confirmation), rule 6.18 (profile golden), sections 7–9 (rows V18, V19), section 11 (arXiv HTML / LaTeXML), section 13 (**decision 11** for V19; **decision 12** for V18).

## 2. Start conditions & hard owner gates

- **V18 start conditions:**
  1. Required merges: **V2, V3, V4** (`inv-a-v2-v5`), **V11** (`draft-inv-v11-cited`, for number-confirmation wiring; optional if V18 ships confirmation helper first). Already merged on base `832eab5bf`: **F0, F2, F3, F10, V1**.
  2. **Owner decision 12 (`investigate.md` 13 #12):** build an **own small HTML tag scanner first** with **zero new dependencies** (`Cargo.lock` untouched). Only if self-authored + real LaTeXML fixtures demonstrate that an own scanner cannot reliably parse arXiv HTML5 may the agent stop and request explicit owner approval to add an HTML5 parser crate to `Cargo.toml`/`Cargo.lock`.
- **V19 start conditions (HARD GATE — DO NOT START WITHOUT ALL FIVE):**
  1. **Required merges:** **V6, V9, V12** (`draft-inv-v12-context.md` MUST be merged so `investigation/2` already includes `record_context_cards` before `investigation/3 = investigation/2 - 4 legacy recorders` is derived!), plus **F1, F5**.
  2. **One full release** has shipped and been used in production with **V9** (`draft-inv-v9-legacy`, new Investigate view + legacy adapter).
  3. **Explicit owner YES on decision 11** (`investigate.md` 13 #11: `"Retire the old map and branch recorders (V19)? Not now. Decide after one release with the new view."`).
  4. **Coordinator/owner `RULES.md:57-59` & scope gate:** extend `RULES.md:57-59` to explicitly authorize adding `investigation/3` to `tool_profile.rs` and `tests/golden/tool_profiles.txt`, and confirm that `research/1`, `investigation/1`, `investigation/2`, and `tests/golden/tool_defs.txt` stay frozen (unless the owner also explicitly approves retiring old Research/`investigation/1` sessions per `00-overview.md` 8.3 #8).
  5. **Coordinator/owner `RULES.md:62` test-retirement gate:** explicitly authorize retiring/updating the legacy `/investigation/{branches,rechecks,conventions}` POST assertions in `crates/desktop-server/tests/investigation_flow.rs:147,170,544`, `crates/desktop-server/tests/audit_flow_turns.rs:366,438`, `crates/desktop-server/tests/audit_consistency_investigation.rs:312,401`, and `crates/desktop-server/tests/investigation_browser.py:150-152` (otherwise `RULES.md:62` forbids modifying or removing existing test assertions).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **V1 (`crates/research-tools/src/skeleton/model.rs`, 403 lines):** `SkeletonBuilder::Html1` (`"html-1"`), `SkConfidence::HtmlClass` (`"html_class"`), `NumberBasis::Html` (`"html"`), `NumberBasis::Confirmed` (`"confirmed"`), `MAX_SKELETON_NODES` (6,000), `MAX_SKELETON_EDGES` (20,000), `MAX_SKELETON_JSON_BYTES` (2 MiB).
- **Dependencies (`crates/research-tools/Cargo.toml`, 28 lines):** has `roxmltree` (XML-only — **fails on HTML5 void tags/unquoted attributes**), `regex`, `percent-encoding`. Zero HTML5 parser crate.
- **F3 (`crates/research-tools/src/http_client.rs`, 234 lines; `net_rate.rs`, 447 lines; `paper_fetch.rs`, 492 lines):** `"arxiv.org"` is ALREADY in `ALLOWLISTED_HTTPS_HOSTS`; loopback path prefix `"/html/"` ALREADY maps to `Service::Arxiv` (1 req / 3 s gate) in `net_rate.rs` (verified in `reports/f3-net-gate.md` section 3 decision 2!); `env.literature_endpoints().arxiv_eprint_base` provides the base origin overridden in loopback mock tests.
- **F1 (`crates/research-tools/src/tool_profile.rs`, 329 lines; `tests/golden/tool_profiles.txt`, 79 lines):** `ToolProfile`, `PROFILE_RESEARCH_V1`, `PROFILE_INVESTIGATION_V1`, `PROFILE_INVESTIGATION_V2`, `legacy_profile_name_for_mode` (`tool_profile.rs:241`), `register_profile`. Verified: `research/1` and `investigation/1` BOTH currently list all 4 old recorders (`record_paper_branch`, `record_paper_conventions`, `record_paper_map`, `record_paper_summary`) and their `earlier_defs` from `investigation_legacy_defs.rs` (168 lines).
- **Existing old Investigation write paths:** `crates/desktop-server/src/investigation.rs` (597 lines, `<= 600` — deleting retired route handlers shrinks it), `investigation_prompts.rs` (217), `investigation_scope.rs` (470), `investigation_finish.rs` (98), `assets/investigation_map.js` (594), `assets/investigation_graph.js` (235), `assets/investigation_conventions.js` (167).

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **For V18:** V2 `skeleton/tags.rs` (`node_tag` kind-class table), V2 `skeleton/number.rs`, V4 `desktop-server/src/investigation_open.rs` (fallback chain TeX -> HTML -> PDF), V11 `skeleton/locate.rs` (consumes `NumberBasis::Confirmed` / `Html`).
- **For V19:** V6 + V12 `investigation/2` in `tool_profile.rs` (must contain `read_map_node`, `record_map_reading`, `record_context_cards`), V9 `investigation_legacy_view.rs` (must remain intact so historical `investigation.map`/`branch` events remain readable!), F5 `api_dispatch.rs` & `session_mode.rs`.

## 4. Smallest implementation checkpoints

### Track A — V18 (`gemini/modes/inv-v18-html-skeleton`)
1. **Checkpoint 1 — Pure own HTML5/LaTeXML tag scanner (`crates/research-tools/src/skeleton/html.rs`):**
   - Implement a bounded, panic-free, zero-new-dependency HTML5 tag/attribute scanner (handling void elements, raw-text `<script>`/`<style>` skipping, entity decoding, and byte/depth caps). *Document exact LaTeXML class conventions not observed in live HTML as assumptions in the report until verified against real arXiv HTML.*
   - Map LaTeXML structural classes (`ltx_section`/`ltx_subsection`/`ltx_appendix`, `ltx_theorem`/`ltx_theorem_<kind>`, `ltx_proof`, `ltx_equation`/`ltx_equationgroup`, `ltx_figure`, `ltx_table`, `ltx_bibitem`, `ltx_ref`, `ltx_cite`, `ltx_tag`) to `PaperSkeleton` with `builder: SkeletonBuilder::Html1`, `confidence: SkConfidence::HtmlClass`, `number_basis: Some(NumberBasis::Html)`, physical line spans into the pinned ingested HTML file (so F4 `verify_quote` works unchanged), and V2 `node_tag`s.
2. **Checkpoint 2 — Cross-confirmation & F3 fetch fallback:**
   - Pure helper `confirm_tex_numbers_with_html(tex: &mut PaperSkeleton, html: &PaperSkeleton) -> ConfirmReport`: matches nodes conservatively (by label or body fingerprint) and upgrades `NumberBasis::Computed` -> `NumberBasis::Confirmed` ONLY when both kind and printed number agree verbatim; records disagreements without overwriting TeX numbers.
   - Build arXiv HTML URLs via `env.literature_endpoints().arxiv_eprint_base` (mirroring `paper_fetch.rs` so `https://arxiv.org/html/<id>` in production maps cleanly to `http://127.0.0.1:<port>/html/<id>` in loopback mock tests with zero live network calls), and fetch strictly through F3 `http_client::fetch_response_with_gate(..., Some(Service::Arxiv), gate)` (respecting the shared 1/3s arXiv gate and `MAX_EPRINT_BYTES` / HTML size cap), wired into V4 `investigation_open.rs` as fallback between TeX and PDF (and optional confirmation in V11).

### Track B — V19 (`gemini/modes/inv-v19-retire-recorders`, ONLY after all 5 owner/start gates clear)
1. **Checkpoint 1 — New `investigation/3` profile in F1 data (zero `resume.rs` edits!):**
   - Add `PROFILE_INVESTIGATION_V3 = "investigation/3"` to `tool_profile.rs` containing V12's `investigation/2` minus the 4 legacy recorders (`record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`).
   - Add `[profile investigation/3]` to `tests/golden/tool_profiles.txt`.
   - Keep `research/1`, `investigation/1`, `investigation/2`, `investigation_legacy_defs.rs`, and `tests/golden/tool_defs.txt` **100% frozen** so every existing session continues to resume cleanly under `eval_set_r` and `resume_drift_tests`.
   - **CRITICAL (`tool_profile.rs:241`):** `legacy_profile_name_for_mode(Mode::Investigate)` **MUST KEEP returning `PROFILE_INVESTIGATION_V1`** for pre-profile sessions (changing it breaks `eval_set_r`!). Switch **ONLY** the new-session profile selector in F5's `session_mode.rs` / `turn_runner.rs` to `investigation/3`.
2. **Checkpoint 2 — Retire legacy write routes & prompts (keep read-only V9 adapter!):**
   - Remove/disable legacy write endpoints (`/investigation/branches`, `/rechecks`, `/conventions` write triggers) and old mapping/branch prompt builders once owner confirms no active migration path needs them, updating the owner-authorized legacy POST test assertions listed in Gate 5.
   - **Keep** read-only V9 `investigation_legacy_view.rs` and historical event readers (`expanded_map`, `paper_conventions`) so old ledgers remain 100% viewable.

## 5. Owned files (all <= 600 lines)

- **V18 owned files:**
  - `crates/research-tools/src/skeleton/html.rs` (plus sibling `html_lex.rs` / `html_confirm.rs` if > 450 lines)
  - `crates/research-tools/tests/skeleton_html.rs` + `crates/research-tools/tests/fixtures/skeleton_html/*` (self-authored synthetic LaTeXML-like HTML fixtures only!)
  - Small entry-point additions in `skeleton/mod.rs` (210 lines) and V4's `investigation_open.rs`.
- **V19 owned files (only after all 5 owner/start gates clear):**
  - `crates/research-tools/src/tool_profile.rs` (add `investigation/3` data; keep <= 600 lines, split `tool_profile_defs.rs` first if needed)
  - `crates/research-tools/tests/golden/tool_profiles.txt` (add `[profile investigation/3]` section only)
  - `crates/desktop-server/src/api_dispatch.rs`, `session_mode.rs`, `turn_runner.rs` (retire legacy write routes, point new Investigate sessions to `investigation/3`), `investigation.rs` (597 lines `<= 600`; deleting retired route handlers shrinks it)
  - `crates/desktop-server/tests/investigation_v19_retire.rs` (plus owner-authorized updates to legacy POST assertions in Gate 5)
  - **File-length guard:** ensure every touched remaining legacy file (`investigation.rs` <= 597, `assets/investigation_map.js` <= 594, `assets/investigation.js` <= 573) stays <= 600 lines (deleting retired code strictly shrinks them).

## 6. Shared registration seams and "Do not edit" files

- **V18 seams:** `skeleton/mod.rs` (`pub mod html;`), V4 `investigation_open.rs` (TeX -> HTML -> PDF fallback), V11 `investigation_cited.rs` (optional confirmation pass).
- **V19 seams:** `tool_profile.rs`, `tests/golden/tool_profiles.txt`, `session_mode.rs`, `turn_runner.rs`, `api_dispatch.rs`.
- **Do NOT edit:**
  - **In V18:** `Cargo.lock` (until owner approves a dependency under decision 12), `tool_defs.txt`, `tool_profiles.txt`, `skeleton/tex_*.rs` (TeX output must stay byte-identical), `net_rate.rs` (already classifies `/html/` as `Service::Arxiv`!).
  - **In V19:** `crates/agent/src/resume.rs` (profiles are data in F1, never hand-written resume rules!), `tool_profile.rs:241` `legacy_profile_name_for_mode(Mode::Investigate)` (must remain `PROFILE_INVESTIGATION_V1`), `tests/golden/tool_defs.txt`, `research/1` & `investigation/1` & `investigation/2` sections of `tool_profiles.txt`, `eval_set_r.rs`, V9 `investigation_legacy_view.rs` (read-only history must survive!).

## 7. Tests through the entry point

- **V18 tests (zero real network, zero model calls, zero third-party HTML):**
  - Self-authored synthetic LaTeXML-like HTML5 fixtures (sections, amsthm theorems/lemmas/proofs, equations with tags, bibliography, `\ref`/`\cite`, malformed/unclosed tags, `<script>`/`<style>` blocks, 2 MiB JSON / 6,000-node cap): determinism (two runs byte-identical), physical line spans verify cleanly via F4 `verify_quote`, tags match V2 `node_tag` format.
  - Cross-confirmation test: matching TeX + HTML upgrades `Computed` -> `Confirmed` (enabling V11 `found`); mismatched number stays `Computed` (so V11 still returns `candidates`, preserving Set P5's 0-wrong-statement rule!).
  - Loopback mock test with `FakeRateClock`: fetching `/html/<id>` passes `net_rate::Service::Arxiv` 1/3s gate and ingests via `ingest_bytes`.
- **V19 tests (only after owner gate):**
  - `tool_def_golden` passes untouched; `tool_profile_golden` passes with `[profile investigation/3]` omitting all 4 old recorders while `research/1`, `investigation/1`, `investigation/2` remain byte-identical.
  - All four Set R ledgers (`eval_set_r`), `resume_loop`, and `resume_drift_tests` pass untouched (old sessions still resume and still display read-only legacy maps via V9).
  - New Investigate session starts under `investigation/3` and rejects calls to retired old recorders; retired write routes return 404/410 cleanly.

## 8. Document conflicts & resolutions

- **V19 vs `00-overview.md` 2.2 & 8.3 #8 (`research/1` carries the 4 old recorders):** On base `832eab5bf`, `research/1` and `investigation/1` both include `record_paper_{map,summary,branch,conventions}` because pre-F1 Research sessions registered `register_research_tools` (`reports/f1-tool-profiles.md` section 2 decision 1). Therefore, V19 CANNOT delete the 4 recorder implementations or edit `all_research_tool_defs()` / `tool_defs.txt` while earlier Research sessions remain supported (`00-overview.md` 8.3 #8 wins). V19 retires them from **new** Investigate sessions via `investigation/3` (and retires legacy desktop write routes), leaving the Rust recorder handlers registered only for frozen `research/1`, `investigation/1`, and `investigation/2` profiles until earlier Research sessions are separately retired.
- **V18 HTML5 parsing (`investigate.md` 0.1 vs `RULES.md` no-new-dependency rule):** `roxmltree` cannot parse HTML5, and `RULES.md` forbids modifying `Cargo.lock`. `investigate.md` 13 #12 resolves this cleanly: build an own small tag scanner first with zero new dependencies; stop and ask the owner only if fixtures prove an HTML5 crate is unavoidable.

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope:** `ar5iv.org` (not in `http_client.rs` allow-list; `investigate.md` 0.1 confirms it is not needed); removing Lean/physlib or retiring `research/1` (`00-overview.md` 8.3 #8, separate owner decision); V14 prior-work check (deferred).
- **Hand-back (`RULES.md`):**
  - V18: write `docs/design/modes-2026-11/reports/inv-v18-html-skeleton.md` and push to `gemini/modes/inv-v18-html-skeleton`.
  - V19: write `docs/design/modes-2026-11/reports/inv-v19-retire-recorders.md` and push to `gemini/modes/inv-v19-retire-recorders`.
