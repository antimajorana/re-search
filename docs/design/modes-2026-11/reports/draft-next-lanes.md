# Report: `draft-next-lanes` — Later Implementation Briefs (Groups 1–5)

- **Brief:** `docs/design/modes-2026-11/agents/draft-next-lanes.md`
- **Base commit:** `832eab5bf856cec90b3a4a2f08f95827ec9d1acd` (`origin/gemini/work/fixed`)
- **Work branch:** `work/draft-next-lanes` (pushed to `gemini/modes/draft-next-lanes`)
- **Scope & guard rails:** Planning and design only. Created **30** new `docs/design/modes-2026-11/agents/draft-*.md` files across the 5 required groups plus this report (`docs/design/modes-2026-11/reports/draft-next-lanes.md`). **Zero** edits to `00-overview.md`, mode design documents, `README.md`, `HANDOFF.md`, `RULES.md`, existing briefs (including `inv-v6-tasks.md` and `inv-v17-pdf-skeleton.md`), production code, tests, `Cargo.toml`, `Cargo.lock`, or tool goldens (`tool_defs.txt`, `tool_profiles.txt`). No implementation agents were launched.

---

## 1. Deliverables by Group (30 Draft Briefs, 3,725 Lines Total)

All packages cut or deferred by the 2026-11-25 scope revision (`00-overview.md` 0.1, 8.1) remain strictly excluded: **S16** (Lean formal tier — cut), **S22** (Cadabra2/FORM/`solve/2` — deferred), **V14** (Investigate prior-work check — deferred; `"Search wider in Explore"` moved to V16), and **version tracking** (half of V10, eval set P2 — deferred; V10 keeps notes only). Packages already merged on base (`F0`, `F1`, `F2`, `F3`, `F4`, `F6`, `F8`, `F9`, `F10`, `F14a`, `V1`, `S2`) or already briefed (`V2–V5`, `V6`, `V17`, `F5`, `F7`, `F11`, `X1`, `S3`, `S6`, `F13a`, `F16a`, `F15` core, `eval-p-cases`, `eval-q-cases`, `eval-solve-badges`) are untouched and referenced only as dependencies.

### Group 1 — Investigate (`V7–V13`, `V15–V16`, `V18–V19`; 7 files, 742 lines)

| Draft file | Lines | Packages | Hard start gate(s) | Push branch |
|---|---:|---|---|---|
| `agents/draft-inv-v11-cited.md` | 113 | **V11** (Cited result without a model) | `V4` + `F5` (`F10`, `F3`, `F14a` on base); CP3 PDF path needs `V17` | `gemini/modes/inv-v11-cited` |
| `agents/draft-inv-v13-wholemap.md` | 91 | **V13** (Focused & whole-paper graph views) | CP1: `V5` (`F6` on base); CP2 filters: `V7` | `gemini/modes/inv-v13-wholemap` |
| `agents/draft-inv-v7-v8-v10.md` | 116 | **V7** (Orient) → **V8** (Open node) → **V10** (Notes only) | `V5` + `V6` + `F5` | `gemini/modes/inv-v7-v8-v10` |
| `agents/draft-inv-v9-legacy.md` | 99 | **V9** (Old sessions / legacy adapter) | `V3` + `V6` + `F5`; `investigation_skeleton.rs` embed serialized after `V7` | `gemini/modes/inv-v9-legacy` |
| `agents/draft-inv-v12-context.md` | 108 | **V12** (Context cards + `record_context_cards`) | `V6` + `V8` + `V11` + **`RULES.md:57-59` gate** | `gemini/modes/inv-v12-context` |
| `agents/draft-inv-v15-v16-share-handoff.md` | 115 | **V15** (`.rsmap`/HTML share) & **V16** (Hand-off + `"Search wider in Explore"`) | V15: `V8` + `V10` + `F5`; V16: `V5` + `V7` + `V8` + `V10` + `F7` | `gemini/modes/inv-v15-v16-share-handoff` |
| `agents/draft-inv-v18-v19-later.md` | 100 | **V18** (arXiv HTML skeleton) & **V19** (Retire old recorders) | V18: `V4` + owner decision 12; V19: `V9` + `V12` + ≥1 shipped release + **owner/RULES gates** | `gemini/modes/inv-v18-v19-later` |

### Group 2 — Solve (`S4–S5`, `S7–S15`, `S17–S21`; 8 files, 860 lines)

| Draft file | Lines | Packages | Hard start gate(s) | Push branch |
|---|---:|---|---|---|
| `agents/draft-solve-s4-s5-setup.md` | 98 | **S4** (Marker, setup, `solve/view`) → **S5** (UI setup card, sole `approval_card.js` owner) | `F5` (`S2`, `F4`, `F9`, `F6` on base) + pre-turn `UiApprover` verification | `gemini/modes/solve-s4-s5-setup` |
| `agents/draft-solve-s7-s8-s9-s11.md` | 132 | **S7** (`expr_show` + frozen 16-tool `solve/1`) → **S8** (`step_record`, `solve/ack`) → **S9** (algebra checks, `solve/check`) → **S11** (`SolveGate`, `solve_finish`) | `S3` + `S6` + **S6 parser stop-gate** + **`RULES.md:57-59` gate**; S8 needs `S4`; M3 needs `eval-q-cases` + `eval-solve-badges` | `gemini/modes/solve-s7-s8-s9-s11` |
| `agents/draft-solve-s18-scipy-artifacts.md` | 100 | **S18** (SciPy, matplotlib, artifacts, job memory cap, installer smoke) | Part A (Linux): `S7` (+ `S4` view); Part B (Windows): **owner wheel/cap approval** + Windows CI | `gemini/modes/solve-s18-scipy-artifacts` |
| `agents/draft-solve-s10-s19-checks-export.md` | 110 | **S10** (6 physics checks) & **S19** (Notebook export + ZIP replay, parallel-safe) | `S9` (`S19a` splits `routes_tools.rs` 610→`<560` first) | `gemini/modes/solve-s10-s19-checks-export` |
| `agents/draft-solve-s12-loop.md` | 121 | **S12** (Solve loop, mode-neutral `Paused`, resume brief, `solve/continue`) | `S11` + `F5` (`S12a` splits `runner.rs` 626→`<540` first) | `gemini/modes/solve-s12-loop` |
| `agents/draft-solve-s15-cli-parity.md` | 92 | **S15** (CLI/MCP parity for `solve/1`) | CP1: `S11` + `S12` (serialized with `X18` on `cli_turn.rs`); CP2 review: `S14` | `gemini/modes/solve-s15-cli-parity` |
| `agents/draft-solve-s13-s17-ui-intake.md` | 100 | **S13** (Derivation cards, badges, Why popover, F6 `graph_view.js`) → **S17** (TeX intake) | S13: `S5` + `S8` + `S9` + `S12` (`F6`, `F10` on base); S17: `S9` + `S13` | `gemini/modes/solve-s13-s17-ui-intake` |
| `agents/draft-solve-s14-s20-s21-review-blind-migrate.md` | 107 | **S14** (Verifier loop + host `independence`) → **S20** (Blind compute + G3); **S21** (Open Research in Solve, parallel-safe) | S14: `S9` + `S12` (splits `verifiers.rs` 878 & `agent_models.rs` tests first); S20: strictly after `S14`; S21: `S4` + `S11` + `S12` + `F5` | `gemini/modes/solve-s14-s20-s21-review-blind-migrate` |

### Group 3 — Explore & F12 (`F12`, `X3–X4`, `X6–X19`; 9 files, 894 lines)

| Draft file | Lines | Packages | Hard start gate(s) | Push branch |
|---|---:|---|---|---|
| `agents/draft-exp-f12-coverage.md` | 84 | **F12** (Coverage, verdict, wording; `independence` excluded → S14) | `F11` (`F3`, `F10` on base) | `gemini/modes/exp-f12-coverage` |
| `agents/draft-exp-x3-x4-shell.md` | 101 | **X3** (Session, stage machine, `idea_view`, `routes_idea.rs`) → **X4** (Model-free first screen) | `X1` + `F11` + `F5` (`F1`, `F6`, `F9`, `F10` on base) | `gemini/modes/exp-x3-x4-shell` |
| `agents/draft-exp-x6-x7-x8-search-triage.md` | 106 | **X6** (`agent::exchange`, Sharpen) → **X7** (Plan, frozen 4-tool `explore/1`, F12 coverage) → **X8** (Triage, `evidence.abstract`) | X6: `X3` + `X4`; X7: `X6` + `F12` + `S5` (`approval_card.js` + pre-turn `UiApprover`) + **`RULES.md:57-59` gate**; X8: `X7` | `gemini/modes/exp-x6-x7-x8-search-triage` |
| `agents/draft-exp-x9-x10-x11-read-verdict.md` | 108 | **X9** (Reading queue) → **X10** (Passage reader, C8 OCR) → **X11** (Map, 9-kind edge direction, verdict, V9 exclude-origin) | Full `X6` + `X7` + `X8` merged (`F3`, `F4`, `F10`, `F14a` on base) | `gemini/modes/exp-x9-x10-x11-read-verdict` |
| `agents/draft-exp-x12-x13-map-handoff.md` | 106 | **X12** (Map & verdict UI, `idea/note`) → **X13** (Directions & F7 hand-off, sequential) | X12: `X11` + `X4` (`F6` on base); X13: `X12` + `F7` | `gemini/modes/exp-x12-x13-map-handoff` |
| `agents/draft-exp-x14-eval.md` | 85 | **X14** (Set I, Explore Set C, Set P6 reader) | CP1: `X7` + F8 harness expansion; CP2a: `X11` + `X13` + `F7` + `F13a` + **owner ideas & non-builder keys**; CP2b (`I→Q`): + Solve `S11`/`S12` | `gemini/modes/exp-x14-eval` |
| `agents/draft-exp-x15-x16-chat-cite.md` | 104 | **X15** (Conversation turns) & **X16** (Citation expansion, `routes_idea_net.rs`, parallel-safe) | X15: `X11` + `F5` + `S12` (`turn_runner.rs` serialization); X16: `X11` + `F11` | `gemini/modes/exp-x15-x16-chat-cite` |
| `agents/draft-exp-x17-x18-lib-cli.md` | 109 | **X17** (Library pane & JSON/BibTeX export in `routes_idea_export.rs`) & **X18** (CLI-subscription routes, parallel-safe) | X17: `X11` + `F5` + `F13a`; X18: `X6` + `X10` (serialized with Solve `S15` on `cli_turn.rs`); CP2b needs `X15` | `gemini/modes/exp-x17-x18-lib-cli` |
| `agents/draft-exp-x19-optional-services.md` | 91 | **X19** (OpenAlex & TheoremSearch) | `F11` (`F3` on base) + **owner allowlist / `THREAT-MODEL.md` / consent approvals** | `gemini/modes/exp-x19-optional-services` |

### Group 4 — Library, PDF OCR/Vision, and Lessons Extensions (`F13b/c`, `F14b/c`, `F16b`; 3 files, 515 lines)

| Draft file | Lines | Packages | Hard start gate(s) | Push branch |
|---|---:|---|---|---|
| `agents/draft-f13bc-sources-ext.md` | 154 | **F13b** (BibTeX & Zotero read-only adapters) & **F13c** (Opt-in Export BibTeX & Kandinsky capture) | `F13a` + `F5` (`F2`, `F10`, `F14a` on base) | `gemini/modes/f13bc-sources-ext` |
| `agents/draft-f14bc-ocr-vision.md` | 197 | **F14b** (Sandboxed Tesseract OCR) & **F14c** (Page images to vision models for formulas) | F14b: **Owner Live-P6 Measurement Gate (8.3 q4)** + **Owner dependency approvals (page renderer + Tesseract helper/installer)** + `F5` (+ `V17`); F14c: **Owner page-renderer dependency approval** + `F5` | `gemini/modes/f14bc-ocr-vision` |
| `agents/draft-f16b-lessons.md` | 164 | **F16b** (User-approved lessons shown to models, `lessons.supplied`, "What I have learnt" UI, R24/Set K) | `F16a` + `F5` + Solve `S12` (`solve_loop.rs`) + `S13` (`solve_why.js` `onStepCorrected`) | `gemini/modes/f16b-lessons` |

### Group 5 — F15 Write-Up Mode Adapters (`writeup_solve`, `writeup_idea`, `writeup_paper`; 3 files, 714 lines)

| Draft file | Lines | Packages | Hard start gate(s) | Push branch |
|---|---:|---|---|---|
| `agents/draft-f15-writeup-solve.md` | 246 | **F15 Solve adapter** (1st: `writeup_solve.rs`, shared `routes_writeup.rs`, `cmd_writeup.rs`, `assets/writeup_btn.js`) | `F15` core + `F13a` + `F5` + `S8` + `S9` + `S11` | `gemini/modes/f15-writeup-solve` |
| `agents/draft-f15-writeup-explore.md` | 221 | **F15 Explore adapter** (2nd: `writeup_idea.rs`, `writeup_explore_routes.rs`) | `F15` core + `F13a` + `f15-writeup-solve` + `F12` + `X10` + `X11` (`X13` optional) | `gemini/modes/f15-writeup-explore` |
| `agents/draft-f15-writeup-investigate.md` | 247 | **F15 Investigate adapter** (3rd: `writeup_paper.rs`, `writeup_investigate_routes.rs`) | `F15` core + `F13a` + `f15-writeup-solve` (+ `f15-writeup-explore` serialized on `export/src/lib.rs`) + `V2–V3` + `V6–V8` + `V10` + `V17` (`F16a` optional) | `gemini/modes/f15-writeup-investigate` |

---

## 2. Master Dependency Order (Investigate First)

Following owner priority (`00-overview.md` 0, 6; `README.md` line 5): **Investigate stays first**, with Solve headless and Library/PDF advancing alongside, Explore following F11/X1/F5, and F15/F16b/F13b–c/F14b–c attaching as each upstream gate merges.

1. **Wave 0 — Currently in-flight or ready briefs (merge before later lanes):**
   - `inv-a-v2-v5` (`V2 → V3 → V4 → V5`) — **highest priority**
   - `f5-f7-mode-shell` (`F5 → F7`)
   - `inv-v17-pdf-skeleton` (`V17`, after `V2`) and `inv-v6-tasks` (`V6`, after `V3`)
   - `q-f11-x1-s3-s6` (`F11`, `X1`, `S3`, `S6`)
   - `lib-f13a-f16a-f15` (`F13a → F16a → F15 core`)
   - `eval-p-cases`, `eval-q-cases`, `eval-solve-badges`, `audit-mode-foundations`
2. **Wave 1 — Immediate fan-out as Wave 0 packages merge (all mutually file-disjoint):**
   - **Investigate (first):** `draft-inv-v13-wholemap` (CP1 after `V5`), `draft-inv-v11-cited` (CP1–2 after `V4+F5`; CP3 after `V17`), `draft-inv-v18-v19-later` Track A `V18` (after `V4` + owner decision 12)
   - **Solve:** `draft-solve-s4-s5-setup` (after `F5` + pre-turn `UiApprover` verification), `draft-solve-s7-s8-s9-s11` (S7 after `S3+S6` + S6 parser stop-gate + `RULES.md` gate; S8 after `S4`)
   - **Explore & F12:** `draft-exp-f12-coverage` (after `F11`), `draft-exp-x3-x4-shell` (after `X1+F11+F5`), `draft-exp-x19-optional-services` (after `F11` + owner allowlist/consent gates)
   - **Library:** `draft-f13bc-sources-ext` (after `F13a+F5`)
3. **Wave 2 — Post-V6 (Investigate), Post-S7/S9/S11 (Solve), Post-X4/F12/S5 (Explore):**
   - **Investigate:** `draft-inv-v7-v8-v10` (`V7 → V8 → V10` after `V5+V6+F5`), then `draft-inv-v9-legacy` (`V9`, `investigation_skeleton.rs` embed serialized after `V7`) and `V13` CP2 (after `V7`)
   - **Solve:** `draft-solve-s18-scipy-artifacts` Part A (after `S7`), `draft-solve-s10-s19-checks-export` (`S10` and `S19` in parallel after `S9`), `draft-solve-s12-loop` (after `S11+F5`), `draft-f15-writeup-solve` (1st F15 adapter, after `F15 core + F13a + S8 + S9 + S11`)
   - **Explore:** `draft-exp-x6-x7-x8-search-triage` (`X6 → X7 → X8` after `X3+X4+F12+S5` + `RULES.md` gate), then `draft-exp-x14-eval` CP1 (after `X7` + F8 expansion)
4. **Wave 3 — Post-V8 (Investigate), Post-S12 (Solve), Post-X8 (Explore):**
   - **Investigate:** `draft-inv-v12-context` (after `V6+V8+V11` + `RULES.md` gate) and `draft-inv-v15-v16-share-handoff` (after `V5+V7+V8+V10+F7`) — disjoint and parallel-safe
   - **Solve:** `draft-solve-s13-s17-ui-intake` (`S13 → S17` after `S5+S8+S9+S12`), `draft-solve-s15-cli-parity` (CP1 after `S11+S12`; serialized with `X18`), `draft-solve-s14-s20-s21-review-blind-migrate` (`S14 → S20` serialized; `S21` parallel after `S4+S11+S12+F5`)
   - **Explore:** `draft-exp-x9-x10-x11-read-verdict` (`X9 → X10 → X11` strictly after full `X6+X7+X8` merge)
5. **Wave 4 — Post-X11 (Explore), Post-S13 (F16b), Owner-Gated F14b/c & V19, Remaining F15 Adapters:**
   - **Explore (5 mutually file-disjoint tracks after `X11`):** `draft-exp-x12-x13-map-handoff` (`X12 → X13` sequential), `draft-exp-x15-x16-chat-cite` (`X15` after `F5+S12`, `X16` via `routes_idea_net.rs`), `draft-exp-x17-x18-lib-cli` (`X17` via `routes_idea_export.rs`; `X18` serialized with `S15`), `draft-exp-x14-eval` (CP2a after `X11+X13+F13a` + owner keys; CP2b `I→Q` after Solve `S11+S12`)
   - **F16b:** `draft-f16b-lessons` (after `F16a + F5 + S12 + S13`)
   - **F15 adapters 2 & 3:** `draft-f15-writeup-explore` (after `f15-writeup-solve + F12 + X10 + X11`), then `draft-f15-writeup-investigate` (after `f15-writeup-explore + V2–V3 + V6–V8 + V10 + V17`), followed by a single future brief **`f15-writeup-polish`** for F15 Rule 5 (`"Polish the prose (model)"`, `export/src/writeup_polish.rs` + `desktop-server/src/writeup_polish.rs`)
   - **Owner-gated late lanes:** `draft-f14bc-ocr-vision` (F14b after **live P6 OCR measurement + explicit owner GO + dependency approvals**; F14c after **owner page-renderer dependency approval**), `draft-solve-s18-scipy-artifacts` Part B (after **owner wheel/cap approval + Windows CI**), and `draft-inv-v18-v19-later` Track B `V19` (after `V9 + V12` + ≥1 shipped release + **explicit owner & `RULES.md` approvals**).

---

## 3. Items Requiring Owner Action, `RULES.md` Authorization, or Pre-Dispatch Gates

| # | Gate / Blocker | Blocks | Required Action Before Dispatch |
|---|---|---|---|
| 1 | **Live Set P6 OCR measurement decision (`00-overview.md` 8.3 q4)** | `F14b` (`draft-f14bc-ocr-vision.md`) | Owner supplies live Set P6 (8 real PDFs incl. 2 pre-1990 publisher-text scans and 2 no-text scans); run F14a measurement sheet in `docs/design/eval-2026-11/`; owner gives explicit GO if `PageBasis::None` share justifies F14b. |
| 2 | **Dependency & packaging approvals (`RULES.md:66`, `Cargo.lock`)** | `F14b`, `F14c`, `S18` Part B, (`V18` fallback) | Owner approves: (a) PDF page renderer crate/helper + Windows packaging (shared by `F14b` & `F14c`), (b) Tesseract helper build/AppContainer staging + SHA-256 `tessdata` manifest (`F14b`), (c) optional `pdf_oxide` only if live P6 fails `pdf-extract-layout/0.12.1`, (d) cp314 `win_amd64` SciPy + matplotlib wheel manifest, licences, installer size delta, and `win.rs` Job Object memory cap (`S18` Part B), (e) HTML5 parser crate for `V18` only if own scanner fails fixtures. |
| 3 | **`RULES.md:57-59` tool-definition & `tool_profiles.txt` authorization** | `V12`, `V19`, `S7` (`solve/1`), `X7` (`explore/1`) | `RULES.md:57` currently authorizes ONLY `inv-v6-tasks`. Coordinator/owner must extend `RULES.md:57-59` per brief before dispatch to authorize scoped tool defs and `[profile ...]` entries in `tests/golden/tool_profiles.txt` while keeping `research/1` and `tool_defs.txt` frozen. |
| 4 | **S6 `srepr_safe.py` parser stop-gate (`00-overview.md` 6)** | `S7–S11` (`draft-solve-s7-s8-s9-s11.md`) | Read `reports/q-f11-x1-s3-s6.md` once S6 merges; if `srepr_safe.py` rejects too much of the physics/math corpus, stop and widen the allowlist before launching S7–S9. |
| 5 | **Pre-turn `UiApprover` verification & `approval_card.js` owner (`00-overview.md` 3.1 row 13 & §9)** | `S5`, `X7`, `F13b` metadata lookup | `f5-f7-mode-shell.md` does NOT currently include verifying `UiApprover` outside a running turn. Verify/settle pre-turn `UiApprover` in a small F5 follow-up or at the start of `S4/S5`; confirm Solve `S5` as sole owner of `assets/approval_card.js` (consumed read-only by `X7` and `F13b`). |
| 6 | **HTTP allowlist & consent approvals (`THREAT-MODEL.md:268`)** | `X19` (`draft-exp-x19-optional-services.md`) | Explicit owner approval for CP1 (`api.openalex.org` allowlist + `THREAT-MODEL.md`) and CP2 (TheoremSearch verified hostname, terms of use, allowlist, off-by-default consent notice). |
| 7 | **V19 retirement & `RULES.md:62` legacy-test gate (`investigate.md` 13 #11)** | `V19` (`draft-inv-v18-v19-later.md`) | Requires ≥1 shipped release with V9, `V12` merged, explicit owner YES on decision 11, confirmation of `research/1` retention (`00-overview.md` 8.3 #8), and explicit authorization to retire/update legacy `/investigation/{branches,rechecks,conventions}` POST assertions in `investigation_flow.rs`, `audit_flow_turns.rs`, `audit_consistency_investigation.rs`, and `investigation_browser.py`. |
| 8 | **Live evaluation sets & non-builder answer keys (`00-overview.md` 8.2 #12)** | `X14`, `F14b`, Milestones M1–M3 | Owner supplies live Set P, P3, P5, P6, I (10 ideas, 2 library-only), C, and real library B; a non-builder reviewer writes answer keys. |
| 9 | **F8 harness expansion (`eval_support/`, `eval_harness.rs:35`)** | `X14` (and `Set P6/B/W/K` live sheets) | Base `eval_harness.rs:35` asserts `[Set::P, Set::Q]`, `manifest.rs` lacks `Set::P6/B/W/K`, and `measures.rs` lacks P6 and library-split recall ids (and still references `Set::P4`). Schedule a focused F8 harness-expansion step before `X14` and merge `Set C` into the single `tests/fixtures/eval/manifest-c.json`. |
| 10 | **Cross-group serialization & F15 Rule 5 follow-up** | `S15 ↔ X18`, `S12 ↔ X15`, `F15 polish` | Never run `S15` and `X18` concurrently on `cli_turn.rs`/`core_tools.rs`/`mcp-server/src/main.rs`; never run `S12` and `X15` concurrently on `turn_runner.rs`; schedule one shared `f15-writeup-polish` brief (`export/src/writeup_polish.rs` + `desktop-server/src/writeup_polish.rs`) after all 3 pure F15 adapters merge. |

---

## 4. Base-Commit Verified Interfaces (`832eab5bf`) vs Future-Merge Interfaces

### Verified present on base `832eab5bf856cec90b3a4a2f08f95827ec9d1acd`
- **F1 (`tool_profile.rs` 329, `resume_profile.rs` 310, `tool_profiles.txt` 79):** `ToolProfile`, `ProfileHostTools` (6 struct-literal callers across 4 crates), `allows_gain` (`ReadWorkspace` only), `session_mode_from_events`, `legacy_profile_name_for_mode` (`tool_profile.rs:241`), `register_profile`, `mcp_defs()`.
- **F2 (`paper_key.rs`):** `PaperKey`, `PaperRef`, `PaperIds`, `NodeRef`, `Mode`, `ID_PREFIXES`, `node_tag`.
- **F3 (`net_rate.rs` 426, `paper_fetch*.rs`, `http_client.rs` 234, `ledger/src/fulltext_cache.rs` 312):** `NetRateGate`, `global_gate`, `Service::{Arxiv, Inspire, SemanticScholar, Crossref}`, `fetch_response_with_gate`, `fetch_bytes_for_service`, `paper_fetch::{fetch, fetch_with_gate, FetchJob, fetched_sources, check_fetch_budget, STATUS_NEEDS_FILE}`, `FulltextCache`.
- **F4 & F9 (`quote_core.rs` 472, `lines_sent.rs` 465, `run_budget.rs` 464):** `QuoteRef`, `verify_quote`, `accept_quote`, `accept_quote_with_sent`, `clip_quote`, `EvidenceRole`, `EvidenceQuotePayload`, `EvidenceAbstractPayload`, `lines_sent_with_ledger`, `ContextSuppliedPayload` (`#[serde(deny_unknown_fields)]`), `count_model_calls`, `RunScope`, `ModelCallBudget`, `RunState`, `InvestigateTaskState`, `ExploreStageState`, `SolveRunState`.
- **F6 (`assets/{graph_layout,graph_view,ui_badge,run_bar,quote_chip}.js`, `assets.rs` 365):** `createGraphView`, `createBadge`, `createSplitBadge`, `createNodeCardShell`, `createRunBar`, `createQuoteChip` (already supports `"OCR"` tag and `openPageImage`). `canvas_dag.js` (688) was NOT slimmed (blocked by 868-line `hitl_ui.test.mjs`).
- **F10 (`ledger/src/library_*.rs`, `research-tools/src/library*.rs`, `desktop-server/src/routes_library.rs` 287):** Full v1 SQLite schema (`source`, `source_item`, `paper_text`, `skeleton`, `search_cache`, `user_mark`, `paper_fts` FTS5, `fulltext` with `origin = 'user_library'`), `Library` API, `library_ext_types::PageBasis` (4 variants incl. `Ocr`), `handle_library` dispatcher (not yet registered in `http_srv.rs`).
- **F14a (`pdf_pages*.rs`, `pdf_firstpage*.rs`, `pdf_match.rs` 589, `ingest.rs` 590):** `PdfExtractLayout` (`"pdf-extract-layout/0.12.1"`), 3-variant `pdf_pages::PageBasis::{TextLayer, ScanTextLayer, None}` (**no `Ocr` variant yet**), `PageMap::page_of` / `page_of_line`, `PdfExtractJob`, `PdfExtractOptions::user_supplied()` (`MAX_USER_PDF_BYTES = 64 MiB`), `pdf_match::match_pdf_quote`, `ingest_pdf_pages`.
- **V1 (`skeleton/{mod,model,flatten,tex_lex,tex_theorems,tex_math,tex_env,tex_env_cmds}.rs`):** `PaperSkeleton`, `SkNode`, `TexSkeletonOutput`, `build_tex_skeleton_from_ingested`.
- **S2 (`solve_model*.rs`, `solve_state.rs` 517, `solve_status.rs`, `solve_badge.rs` 398, `solve_own_tier.rs` 415, `proof_closure.rs` 247):** `SolveEntry`, `SolveState`, `compute_all_tiers`, `badge_for_step`, `Tier::{Verified, CheckedNumerically, ...}`, `event_kinds::LESSONS_SUPPLIED`, `DepNode`, `topological_order`. (`proof_dag.rs` untouched; `solve_model.rs:161-170` has a local `QuoteRef` needing alignment with F4 `quote_core::QuoteRef`.)
- **Verified core helpers:** `Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`, NOT `verify_chain`), `export/src/latex.rs` (771), `desktop-server/src/auth.rs` (340, `is_cookie_route` at line 208), `desktop-server/src/routes_tools.rs` (610, `handle_export` at line 563), `desktop-server/src/original_pdf.rs` (43, 16 MiB cap at line 32), `worker/src/{sandbox.rs (142), appcontainer.rs (377), stage.rs (475)}`, `agent/src/{transport.rs (no ContentBlock::Image), responses.rs (587), runner.rs (626), verifiers.rs (878)}`, `desktop-server/src/{state.rs (598), investigation.rs (597), agent_models.rs (546), cli_turn.rs (626)}`.

### Future-merge interfaces that MUST be verified after prerequisite merges before coding
- **`F5` & `F7` (`f5-f7-mode-shell`):** `api_dispatch.rs`, post-split `routes_session.rs` (`<560`) and `index.html` (`<560`), `session_mode.rs`, `new_menu.js`, `export/src/handoff.rs`, `desktop-server/src/handoff.rs`, whether F7 created `investigation_handoff.rs`, and whether F5 guards `turn_runner.rs`'s `investigation::*` calls by mode.
- **`V2–V5` (`inv-a-v2-v5`):** `skeleton/{edges,number,tags,stats,bib,skip,symbols}.rs`, `investigation_skeleton.rs`, `investigation_open.rs`, `investigation_{outline,results,node,entry,skip}.js`, where V4 persists paper `licence`/`license`.
- **`V6` (`inv-v6-tasks`) & `V17` (`inv-v17-pdf-skeleton`):** `investigation_markers.rs`, `map_node_read.rs`, `map_reading.rs`, `map_defs.rs`, `map_fold.rs`, `investigation_tasks.rs`, `investigation_task_context.rs`, exact V6 `caller` string for `context.supplied`/`Router::try_call`, `investigation/2` exact `defs` (incl. `search_formal_library` status), `skeleton/pdf.rs` (`pdf-1`).
- **`S3` & `S6` (`q-f11-x1-s3-s6`):** `tools::Gate` & `Router::with_gate`, protocol 2 (`exprs_in`, outbox, `expr()`), `srepr_safe.py` allowlist & measured rejection report, check-process invocation and canonical `srepr`/`latex` output.
- **`F11` & `X1` (`q-f11-x1-s3-s6`):** `lit_providers.rs`, `lit_abstracts.rs`, `literature.fetched` `qid`/`run_id`, `idea_terms.rs`, `idea_queries.rs`, `idea_rank.rs`.
- **`F13a`, `F16a`, `F15 core` (`lib-f13a-f16a-f15`):** `paper_sources.rs` trait, `source_sync.rs`, `source_dedupe.rs`, `routes_sources.rs`, `assets/library_sources.js`, `lessons*.rs`, `routes_lessons.rs`, `export/src/writeup{,_tex,_md,_bib}.rs` exact struct/writer signatures.

---

## 5. Document Conflicts & Base-Code Gaps Found and Resolved (`00-overview.md` Wins)

1. **`RULES.md:57-59` vs new mode profiles (`V12`, `V19`, `S7`, `X7`):** `RULES.md:57` authorizes only `inv-v6-tasks`. Resolved by adding explicit hard coordinator/owner gates in all four drafts to amend `RULES.md:57-59` before dispatch while keeping `research/1` and `tool_defs.txt` frozen.
2. **Progressive tool additions vs F1 `may_gain` & `policy_version` (`tool_profile.rs:109-111`, `resume_profile.rs:163-199`):** F1 restricts `may_gain` to `ReadWorkspace`. Solve's `expr_show`/`step_record`/`solve_finish` (`ObserveLocal`) and `check_run` (`ExecuteCode`), and Explore's `idea_fetch_papers` (`NetworkRead`), cannot use `may_gain`, and switching `research_policy()` → `solve_policy()` mid-mode breaks resume. Resolved by freezing the full 16-tool `solve/1` + `solve_policy()` once in `S7` and the full 4-tool `explore/1` once in `X7` (with fail-closed unbuilt stub handlers until later checkpoints wire them without altering `def_hash` or `policy_version`). For `V12`, `record_context_cards` IS `ReadWorkspace`, so adding it to `investigation/2.defs` + `investigation/{1,2}.may_gain` preserves V6 resume cleanly (never removing any V6 tool).
3. **`solve.md:354-356` vs `all_research_tool_defs()` (`doc_ingest`):** Explicitly excluded `doc_ingest` from `solve/1` (Solve ingests only via host `"Attach a source"` and S17).
4. **Crate dependency boundaries vs `solve.md` & `00-overview.md` (`Cargo.lock` frozen):**
   - `crates/agent/Cargo.toml` does NOT depend on `worker` → `S14` (`verifier_loop.rs`) and `S20` (`delegate_blind.rs`) inject an `IsolatedWorkerFactory` trait implemented in `crates/desktop-server/src/solve_workers.rs`.
   - `crates/mcp-server/Cargo.toml` does NOT depend on `agent` → `S11` places `SolveGate` in `crates/research-tools/src/solve_gates.rs` (used by both `agent` and `mcp-server`, and extended by `S20` G3).
   - `crates/research-tools/Cargo.toml` stays free of `rusqlite` → `F13b` puts SQLite Zotero reading in `crates/ledger/src/zotero_reader.rs` and `F16b` puts `lessons.db` extensions in `crates/ledger/src/lessons_b_store.rs`.
   - `crates/export/Cargo.toml` has no `agent` dep → all 3 F15 adapters stay pure in `export`; future `f15-writeup-polish` splits pure placeholder validation (`export/src/writeup_polish.rs`) from the model exchange (`desktop-server/src/writeup_polish.rs`).
5. **`agent::honesty::lint_honesty_with_ledger` vs Solve G7 (`solve.md:434`, `honesty.rs:203-233`):** `honesty.rs` inspects only `ProofDag` + `sympy_eval`/`lean_check` (`LEAN_PHRASES`), which falsely rejects `solve.check`-backed `Verified` steps and misses `CheckedNumerically`. Resolved by building a Solve-tier-aware G7 summary checker directly over `SolveState` + `tiers` + R22 in `solve_done.rs` + `solve_gates.rs`.
6. **Over-600-line base files vs design package tables (`check_file_length.py`):**
   - `crates/desktop-server/src/routes_tools.rs` (**610** lines, holds `handle_export` with exhaustive `match ExportFormat`) → `S19a` splits `handle_export` into `routes_export.rs` first; `F15` uses `routes_writeup.rs`; `X17` uses `routes_idea_export.rs`; `F14b` routes 64 MiB scan upload through `routes_ocr.rs` + `ingest_ocr.rs` (avoiding 590-line `ingest.rs` and 610-line `routes_tools.rs`).
   - `crates/agent/src/runner.rs` (**626**) → `S12a` splits into `dispatch.rs` first; `S12b` updates exhaustive `SessionOutcome::Paused` matches in `cli/src/api_session.rs:163` and `turn_runner.rs:459`.
   - `crates/agent/src/verifiers.rs` (**878**) → `S14a` splits into 3 files first; `agent_models.rs` (**546**) extracts inline tests (`450–546`) to `agent_models_tests.rs` first.
   - `crates/desktop-server/src/cli_turn.rs` (**626**) → `S15` and `X18` serialize strictly and share the same split target (`cli_turn_setup.rs` + `cli_turn_tests.rs`).
   - `crates/agent/src/responses.rs` (**587**) → `F14c` extracts inline tests (`541–587`) into `responses_tests.rs` first.
   - `crates/desktop-server/src/state.rs` (**598**) → `V7` uses `investigation_settings.rs` and `X19` uses `lit_settings.rs` instead of growing `state.rs`.
   - `crates/desktop-server/assets/canvas_dag.js` (**688**, blocked by 868-line `hitl_ui.test.mjs`) → `S13` mounts F6's `graph_view.js` directly without touching `canvas_dag.js`.
   - `crates/desktop-server/src/session_view.rs` (**753**), `projects.rs` (**806**), `export/src/latex.rs` (**771**), `research-tools/src/paper_passages.rs` (**605**), `assets/app.js` (**932**) → all left strictly untouched.
7. **`pdf_pages::PageBasis` (3 variants) vs `library_ext_types::PageBasis` (4 variants incl. `Ocr`):** F15 adapters match `pdf_pages::PageBasis::ScanTextLayer` + raw `"ocr"` strings on base and treat `pdf_pages::PageBasis::Ocr` as an F14b future-merge extension; F14b adds `PageBasis::Ocr` to `pdf_pages.rs` and coordinates with `V17`.
8. **`crates/agent/src/transport.rs` lacks `ContentBlock::Image` & `lines_sent.rs` has `#[serde(deny_unknown_fields)]`:** Resolved in `F14c` by adding `ContentBlock::Image` + `agent/src/vision.rs` + optional `page_ranges` on `ContextSuppliedPayload` while keeping `SentLines::covers` strictly line-only so page images never satisfy `quote_core::accept_quote`.
9. **`00-overview.md` 2.14 single-axis lesson scope vs 8.3 #7 `"Default scope: this mode and this field"`:** Resolved in `F16b` with structured scope (`scope_kind` + optional `mode`, `field`, `paper_key`).
10. **Investigate licence spelling (`paper_fetch_meta.rs:36` `licence` vs `library_schema.rs:227` `license`; absent in `ingest.rs:280`):** Resolved in `draft-f15-writeup-investigate.md` by checking both spellings and failing closed to `<= 300` chars (`quote_core::clip_quote`) unless an explicit permissive licence is recorded.
11. **Explore `explore.md` legacy names vs `00-overview.md` 0.1/2.5/2.9/3.2/4.1:** Reconciled across all Explore drafts (`exchange`, `"Sharpened idea"`, `"matched"` never `"verified"`, shared `evidence.quote`/`evidence.abstract`/`context.supplied`/`handoff.*`, 6-value `PriorWorkStatus`, `independence` moved to S14, 4 `P-to-I` + 2 `I-to-Q` chains in Set C, Solve's `task_kind` and structured `target`, 9-kind `src → dst` edge direction table, X17 JSON/BibTeX only while F15 owns `writeup_idea.rs`).
12. **`solve.md` `roles.verifier` vs base `agent_models.rs:18-31, 266-268`:** Base has only `step_verifier`/`full_verifier` and suppresses same-model transport (`return None`). Resolved in `S14` by adding Solve verifier role resolution that still runs same-model for `solve.md` D2 (`"Model agreed (same model)"`).
13. **`investigate.md` V19 vs `research/1` & `00-overview.md` 8.3 #8:** Because `research/1` and `investigation/1` carry all 4 `record_paper_*` handlers, `V19` cannot delete those handlers while earlier Research sessions remain supported; `V19` retires them from new Investigate sessions via `investigation/3` while keeping `legacy_profile_name_for_mode(Mode::Investigate) == PROFILE_INVESTIGATION_V1`.

---

## 6. Merge Notes & Shared Seam Registry

Zero existing files were modified in this branch. For future implementation merges, these shared seams are coordinated across the 30 drafts so no two parallel briefs claim the same file:

- **`crates/desktop-server/src/api_dispatch.rs` (post-F5):** Single-line delegating arms only — each mode/extension owns its own new route file (`routes_idea.rs`, `routes_idea_net.rs`, `routes_idea_export.rs`, `routes_export.rs`, `routes_writeup.rs`, `routes_sources_ext.rs`, `routes_ocr.rs`, `routes_pdf_vision.rs`, `routes_lessons_ui.rs`, `solve.rs`, `solve_turn_routes.rs`, `solve_artifacts_route.rs`, `investigation_cited.rs`, `investigation_notes.rs`, `investigation_share.rs`).
- **`crates/desktop-server/src/routes_session.rs` (shrunk `<560` by F5):** 2-line create/view seams only in `S4` (Solve) and `X3` (Explore).
- **`crates/desktop-server/src/turn_runner.rs` (463 lines):** Strictly serialized `F5 → S12 → X15`.
- **`crates/desktop-server/src/cli_turn.rs` (626) + `cli-route/src/core_tools.rs` + `mcp-server/src/main.rs`:** Strictly serialized `S15 ↔ X18` (first mover splits `cli_turn.rs` into `cli_turn_setup.rs` + `cli_turn_tests.rs`, fixes `main.rs:184` `use_research || CORE_TOOL_PROFILE`, and strips hardcoded Research prompt instructions at `cli_turn.rs:407-419`).
- **`crates/research-tools/src/tool_profile.rs` + `tests/golden/tool_profiles.txt`:** Strictly serialized by profile — `V6` (`investigation/2`) → `V12` (+`record_context_cards`) → `V19` (`investigation/3`); `S7` (frozen 16-tool `solve/1` once); `X7` (frozen 4-tool `explore/1` once). `tool_defs.txt` and `research/1` stay frozen forever.
- **`crates/desktop-server/src/investigation_skeleton.rs` (from V3):** `V9`'s `mapview` embed serialized after `V7`.
- **`crates/desktop-server/assets/approval_card.js`:** Sole owner `S5`; read-only consumers `X7` and `F13b`.
- **`crates/desktop-server/assets/writeup_btn.js`:** Sole owner `f15-writeup-solve`; mounted by Solve `solve_steps.js`, Explore `idea_map.js`, and Investigate `investigation_handoff_ui.js`.
- **`crates/export/src/lib.rs` (132 lines):** 1-line `pub mod` + match arm per package (`S19`, `V15`, `X17`, `f15-writeup-solve → explore → investigate`); route tests live in separate files (`writeup_{solve,explore,investigate}_routes.rs`).

---

## 7. Decisions Taken Alone, Subagent Methodology, and Independent Audits

1. **Subagent model mapping:** Per the user prompt ("Use Gemini 4 Neon for implementation subagents and Gemini 4 Carbon for hard independent audits") and the remote tool schema (`inherit | flash_lite | flash | pro`), the 5 parallel drafting subagents ran with `Model: "inherit"` (Neon) on strictly disjoint file sets, and 3 read-only hard independent audit passes ran with `Model: "pro"` (Carbon):
   - **Carbon Auditor A** audited all 15 Investigate + Solve drafts against the design docs and base code, finding **15 Critical, 16 Moderate, 6 Minor** defects.
   - **Carbon Auditor B** audited all 15 Explore + F13b/c–F16b + F15 drafts plus cross-group seams across all 30 drafts, finding **6 Critical, 4 Moderate, 4 Minor** defects.
   - All 5 Neon subagents applied every single Critical, Moderate, and actionable Minor fix strictly to their disjoint `agents/draft-*.md` files.
   - **Carbon Final Verification Auditor** re-verified all 7 audit axes across all 30 modified drafts and returned **PASS (7/7)** with zero remaining blocking defects (only 3 harmless 1-line base-count nits on well-under-600 files).
2. **Splitting into 30 gate-aligned briefs:** Rather than 5 large monolithic group files, each group was split at natural dependency gates and file-ownership boundaries so independent lanes can launch immediately after each merge without overlapping file edits.
3. **Architectural seam decisions recorded for coordinator review:**
   - Moving `SolveGate` to `crates/research-tools/src/solve_gates.rs` (eliminates `mcp-server → agent` crate cycle without touching `Cargo.lock`).
   - Injecting `IsolatedWorkerFactory` from `desktop-server/src/solve_workers.rs` into `crates/agent` for `S14` and `S20` (preserves `agent` free of `worker`).
   - Freezing full `solve/1` (16 tools + `solve_policy()`) in `S7` and full `explore/1` (4 tools) in `X7` with fail-closed stub handlers (respects F1 `may_gain` `ReadWorkspace`-only clamp and `policy_version` check).
   - Keeping `/api/sessions/{id}/writeup` strictly header-token authenticated (`api.js` Blob download) instead of expanding `auth.rs:208` stream-cookie routes.
   - Assigning `assets/approval_card.js` solely to Solve `S5` and splitting `X16` (`routes_idea_net.rs`), `X17` (`routes_idea_export.rs`), `S12` (`solve_turn_routes.rs`), `S18` (`solve_artifacts_route.rs`), and `S19` (`routes_export.rs` extracted from 610-line `routes_tools.rs`) into disjoint route modules.
   - Tracking F15 Rule 5 (`"Polish the prose (model)"`) as one future shared brief (`f15-writeup-polish`) rather than duplicating it across 3 adapters.
4. **Assumptions:** All future-merge symbol/payload names for unmerged Wave 0 briefs (`F5`, `F7`, `V2–V6`, `V17`, `S3`, `S6`, `F11`, `X1`, `F13a`, `F16a`, `F15 core`) are explicitly marked *"Future-merge interfaces (must be verified after merge)"* in every draft brief.

---

## 8. Verification Commands Run and Exact Results

Because this brief creates documentation files only (`docs/design/modes-2026-11/agents/draft-*.md` and `docs/design/modes-2026-11/reports/draft-next-lanes.md`) and modifies zero Rust, Python, JS, HTML, CSS, `Cargo.toml`, `Cargo.lock`, or golden files, full build/test checks confirm an untouched base while file-length and diff checks confirm strict scope compliance:

| Command | Exit | Exact Result |
|---|---:|---|
| `git rev-parse origin/gemini/work/fixed` | 0 | `832eab5bf856cec90b3a4a2f08f95827ec9d1acd` |
| `git diff --stat origin/gemini/work/fixed` | 0 | Empty (zero existing tracked files modified) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | 0 | `No touched source file is over 600 lines.` |
| `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden` | 0 | `tool_def_golden`: **3 passed, 0 failed, 0 ignored**; `tool_profile_golden`: **3 passed, 0 failed, 0 ignored** |

---

## 9. What Was Not Checked / Known Limitations & Pre-Existing Notes

- **Planning only:** No production Rust, Python, or JS code was written, compiled, or executed; Windows `cfg(windows)` paths were not compiled on Linux.
- **Unverified external materials:** Live Set P6 journal/scanned PDFs, live Set I ideas, non-builder answer keys, Kandinsky `zotero_import.rs` git history copyright audit (`353368b`), and live Tesseract Windows installer packaging await owner supply/verification at their respective gates.
- **Pre-existing base observations (not modified, outside brief scope):**
  - `crates/desktop-server/src/routes_tools.rs` (610 lines), `crates/agent/src/runner.rs` (626), `crates/desktop-server/src/cli_turn.rs` (626), `crates/agent/src/verifiers.rs` (878), `crates/desktop-server/assets/canvas_dag.js` (688), `crates/desktop-server/tests/hitl_ui.test.mjs` (868), `crates/export/src/latex.rs` (771), `crates/desktop-server/src/session_view.rs` (753), `crates/desktop-server/src/projects.rs` (806), `crates/research-tools/src/paper_passages.rs` (605), `crates/desktop-server/src/http_srv.rs` (702), `crates/desktop-server/src/routes_session.rs` (616), `crates/desktop-server/assets/index.html` (768), and `crates/desktop-server/assets/app.js` (932) are all over 600 lines on `832eab5bf`.
  - `crates/desktop-server/src/state.rs` (598), `crates/desktop-server/src/investigation.rs` (597), `crates/research-tools/src/library_schema.rs` (594), `crates/research-tools/src/investigation.rs` (591), `crates/research-tools/src/ingest.rs` (590), `crates/research-tools/src/pdf_match.rs` (589), and `crates/agent/src/responses.rs` (587) are within 2–13 lines of the 600-line limit.
  - `crates/desktop-server/src/original_pdf.rs:32` still enforces `MAX_PDF_BYTES` (16 MiB) while F14a raised `PdfExtractOptions::user_supplied()` to 64 MiB, `ingest.rs:24` enforces 32 MiB, and `routes_tools.rs:21` enforces 10 MiB for HTTP uploads.
  - `crates/mcp-server/src/main.rs:184` ignores `CORE_TOOL_PROFILE` unless `use_research` is true (`CORE_ENABLE_RESEARCH_TOOLS=1`, workspace, or existing `sources/`).
  - `crates/agent/tests/eval_harness.rs:35` hardcodes `assert_eq!(sets, [Set::P, Set::Q])`.
