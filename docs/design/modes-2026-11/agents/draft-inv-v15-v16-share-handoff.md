# Brief draft-inv-v15-v16-share-handoff — Investigate Share/Import (V15) and Hand-off to Explore & Solve (V16)

## 1. Packages and governing design sections

- **Packages:**
  - **V15 (Export and import):** machine-exchangeable `.rsmap` export, standalone read-only HTML export with copyright/licence guards, and fail-closed `.rsmap` import that re-matches every quote against the local source (rule 6.11).
  - **V16 (Hand-off to Explore and Solve + `"Search wider in Explore"` right half-badge):** three outgoing hand-off buttons built on F7 (`"Derive in Solve"`, `"Has this been extended?"`, and `"Search wider in Explore"` moved here from deferred V14), full node packet filler (statement, statement closure, symbols, conventions, reader notes, gists as `model_text`, origin paper + resolved bibliography seeds), Investigate receiving side (re-matching quotes via F4 and attaching as notes on innermost containing nodes), and read-only child-session status chips (`solve.done` / `idea.verdict`).
- **Governing design:**
  - `00-overview.md`: 0.1 (V14 deferred; `"Search wider in Explore"` moved to V16; F15 write-up is separate), 2.3 (F2 `PaperRef`, `NodeRef`), 2.5 (F4 `verify_quote`, 300-char quote cap), 2.9 (F12 wording: right side never contains `"new"` or `"novel"`), 2.13 (F15 write-up is separate from `.rsmap`/HTML!), 3.1 row 8 & 9 (F7 `handoff.packet`, Solve task kinds), 3.2 row 20 (zero `"verified"` in Investigate), 4.1–4.3 (hand-off packet rules & table), 7.2–7.3 (Evaluation Set C chains: 100% quote survival on unchanged bytes; Set R untouched), 8.1–8.2 (#8 hand-overs re-match every quote and never carry a badge).
  - `investigate.md`: 0.2 #8 (HTML export copyright rule), 2 (worked example 4:00, 5:30, 9:00), 3 (stage I Earlier work, stage L Export/import, novelty row right half-badge), 4.4 (`handoff.sent`, `investigation.exported`, `investigation.imported`; old `investigation.handoff` unchanged), 5.2 (`GET .../investigation/export`, `POST .../investigation/import`; hand-overs use F7 routes), rule 6.11, sections 7 & 9 (rows V15, V16), section 12 (copyright in exports), section 13 (decision 3, 7, 9).
- **Owner decisions (2026-11-25) applied:**
  - **Reading notes (Markdown/TeX) are NOT in V15:** Investigate's reading-notes write-up (`crates/export/src/writeup_paper.rs`) belongs strictly to Group 5's `draft-f15-writeup-investigate.md` (F15). V15 delivers `.rsmap` and standalone `.html` only.
  - **V14 is deferred:** `"Search wider in Explore"` on the right half-badge of each result is Investigate's only prior-work path and belongs to V16. Nothing is written back into the parent session except `handoff.sent`; Explore's status is read from the child session's latest `idea.verdict`.

## 2. Start conditions

- **Required merges on `gemini/work/fixed` before coding:**
  - Already merged on base `832eab5bf`: **F0, F1, F2, F3, F4, F6, F8, F9, F10, F14a, V1**.
  - Must be merged before starting V15: **V2–V5** (`inv-a-v2-v5`), **V6** (`inv-v6-tasks`), **V7, V8, V10** (`draft-inv-v7-v8-v10`, so notes, pins, authors' labels, and `m-` nodes all serialize/import), and **F5** (`f5-f7-mode-shell`, for `api_dispatch.rs`). (If V12 is merged, include context cards in `.rsmap`/import; otherwise structure `.rsmap` schema with optional `context_cards: []` so V12 plugs in seamlessly.)
  - Must be merged before starting V16: **V5** (`inv-a-v2-v5`, `skeleton/symbols.rs` for `SymbolDef`), **V7** (`draft-inv-v7-v8-v10`, reserved right-badge slot in `investigation_badges.js`), **V8, V10** (`draft-inv-v7-v8-v10`), AND **F7** (`f5-f7-mode-shell`, delivering `export/src/handoff.rs` and `desktop-server/src/handoff.rs`).
- **Owner gates:** None (decisions 3, 7, 9 and overview 8.2 #8 accepted).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **Export crate (`crates/export/src/`):** `lib.rs` (132 lines), `fork.rs` (280 lines — `research_from_paper`, must remain byte-for-byte compatible for Set R!), `html.rs` (110 lines — pattern for self-contained HTML escaping), `zip.rs` (341 lines), `diff.rs` (204 lines), `latex.rs` (**771 lines — over limit, do NOT edit**). Dependencies already in `crates/export/Cargo.toml`: `ledger`, `tools`, `research-tools`, `serde`, `serde_json`, `sha2`, `getrandom`, `zip`, `similar`.
- **F2 (`paper_key.rs`):** `PaperKey`, `PaperRef`, `NodeRef`, `Mode::{Investigate, Explore, Solve, Research}`.
- **F4 (`quote_core.rs`):** `QuoteRef`, `verify_quote`, `clip_quote` (300-char cap), `QUOTE_MATCHED_WORD`.
- **F6 (`assets/ui_badge.js`):** `createSplitBadge`, `createBadge`, `MODEL_TEXT_SUFFIX`.
- **F9 (`run_budget.rs`):** `RunState`, `RUN_STATE_LABEL_*` (unified run-state semantics for V16 child-status chips).
- **F10 (`library.rs`):** `Library::open`, `read_fulltext_bytes`, `get_skeleton` (used by V15 import & V16 receiving side).
- **F14a (`pdf_pages.rs`, `pdf_match.rs`):** `PageMap::page_of`, `match_pdf_quote` (for PDF quote re-matching and page links on imported/handed-over PDF papers).

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **F5 & F7 (`f5-f7-mode-shell`):** `export/src/handoff.rs` (`HandoffPacket` v1, `handoff.packet`, `handoff.sent`), `desktop-server/src/handoff.rs` (shared hand-off routes), `api_dispatch.rs`. **Verify specifically** whether F7 already created a minimal `desktop-server/src/investigation_handoff.rs` for the week-8 milestone (`investigate.md` 9 V16 order note); extend it if present, or create it if absent.
- **V3/V5/V6/V7/V8/V10:** `investigation_skeleton.rs`, `skeleton/symbols.rs` (`SymbolDef`), `map_fold.rs`, `map_reading.rs` validators (reused by V15 import!), `investigation_notes.rs`, `investigation_node.js`, `investigation_badges.js`.
- **Explore / Solve child status events (optional at V16 time):** `solve.done` (Solve S11) and `idea.verdict` (Explore X11 / F12). Read these defensively from child session ledger events (`Ledger::read_session`) with clean fallback (`"Opened in Explore: not finished"`, `"Opened in Solve: in progress"`) so V16 works both before and after Explore/Solve land!

## 4. Smallest implementation checkpoints (push after each)

1. **Checkpoint 1 — V15 `.rsmap` & standalone HTML export/import:**
   - `crates/export/src/paper_map.rs` (`.rsmap` v1 JSON schema: `paper_key`, `version`, `builder`, `files_root_sha256`, `licence`, skeleton summary, gists, authors' claims, model nodes/edges, optional `context_cards`, quotes clamped to `<= 300` chars via `quote_core::clip_quote`, full statements included **only** when paper licence is permitting e.g. CC-BY/CC0/public-domain OR caller passes `include_full_statements: true`, private notes included **only** when `include_notes: true`).
   - `crates/export/src/paper_map_html.rs`: pure standalone HTML renderer (zero external scripts/fonts/CDN, strict HTML escaping on every untrusted string, same copyright & private-note gates, explicit `"(model)"` tags, provenance styles, zero `"verified"` anywhere).
   - `crates/export/src/paper_map_import.rs` (rule 6.11):
     - Given an `.rsmap` payload and a session: locate matching local source (same `arxiv` id + version via F10/F3, or file matching `files_root_sha256` / raw hash), write host-only `source.ingested` + rebuilt `investigation.skeleton` blob cleanly (guaranteeing `Ledger::verify_all().unreferenced_blobs == 0`), and pass **every** imported item/edge/claim/card through the exact same recorder & F4 `verify_quote` checks.
     - Drop invalid/tampered items and count `items_matched` / `items_dropped`; write surviving readings/notes tagged `"imported"` (never `"verified"`).
     - If no matching source is available locally, open read-only with `source_match: false` and explicit banner `"quotes not matched on this machine"` (NEVER `"not verified"`).
   - `crates/desktop-server/src/investigation_share.rs`:
     - `GET /api/sessions/{id}/investigation/export?format=rsmap|html&include_statements=0|1&include_notes=0|1` -> streams bytes with `Content-Disposition`, appends `investigation.exported { format, sha256 }`. Rejects `format=md` with a clear message pointing to F15 write-up.
     - `POST /api/sessions/{id}/investigation/import` -> runs rule 6.11 import, appends `investigation.imported { format, sha256, source_match, items_matched, items_dropped }`, returns summary for UI banner (`"Imported, N quotes could not be matched"`).
2. **Checkpoint 2 — V16 Hand-off filler, receiving side, child-status reader, and UI (`investigation_handoff.rs` + `investigation_handoff_ui.js`):**
   - Extend (or create, depending on F7) `crates/desktop-server/src/investigation_handoff.rs`:
     - **Outgoing packet builders** over F7's `HandoffPacket` v1 (zero model calls):
       1. `"Derive in Solve"` (`target_mode: "solve"`): task kind uses Solve's exact names (`prove` for theorem-like nodes, `calculate` for equations, `check_derivation` for proofs; `00-overview.md` 3.1 row 9); includes verbatim statement + `QuoteRef`s, statement closure (definitions, assumptions, equations it uses) as `context`, symbols from `SymbolDef`, paper conventions, reader notes in `user_notes`, and gists ONLY in `model_text` (tagged `"(model)"`). Never carries badges/tiers.
       2. `"Has this been extended?"` (`target_mode: "explore"`): editable statement as idea text, this paper as `origin` (flagged so Explore excludes it from `"found"` verdicts and marks its references `"cited by the origin"`), resolved bibliography `PaperRef`s as seeds, notes in `user_notes`, gists in `model_text`.
       3. `"Search wider in Explore"` (`target_mode: "explore"`, right half-badge of a result, moved from deferred V14): same packet with `ask.kind = "prior_work"` / question `"is this earlier"`, this paper as `origin`, resolved bibliography as seeds.
     - **Receiving side (`target_mode: "investigate"`):** opens/ingests the `PaperRef` (library first, zero network on cache hit), builds/loads skeleton, runs **every** incoming `QuoteRef` in `packet.statement` / `packet.context` through F4 `verify_quote` against the new session's own `source.ingested` receipt (dropping & counting failures; badges never travel), and attaches each surviving quote as a note on the **innermost** skeleton node whose physical spans contain it.
     - **Read-only child-status projection (`child_handoff_statuses(ledger, parent_events)`):** scans `handoff.sent` events in the parent session, reads each child session's latest `solve.done` or `idea.verdict` (read-only; **never** writes back into the parent ledger), and exposes chips (`"Opened in Solve: Done with open items"`, `"Opened in Explore: <Explore's status sentence>"`, or `"Opened in Explore: not finished"`). Enforce F12 wording invariant: right half-badge NEVER contains `"new"`, `"novel"`, or `"verified"`.
   - Create `crates/desktop-server/assets/investigation_handoff_ui.js`:
     - Renders the `"Derive in Solve"` and `"Has this been extended?"` buttons, the `"Search wider in Explore"` right half-badge (populating V7's reserved right-badge slot in `investigation_badges.js`), child-status chips with click-to-open-child navigation, and the V15 export/import dialog (with explicit opt-in checkboxes for full statements and private notes + read-only `"quotes not matched on this machine"` banner).

## 5. Owned files (all <= 600 lines)

- **New Rust files:**
  - `crates/export/src/paper_map.rs`
  - `crates/export/src/paper_map_html.rs`
  - `crates/export/src/paper_map_import.rs`
  - `crates/export/tests/paper_map_share.rs`
  - `crates/desktop-server/src/investigation_share.rs`
  - `crates/desktop-server/src/investigation_handoff.rs` (extend if F7 created minimal stub; keep <= 600 lines, split `investigation_handoff_receive.rs` if needed)
  - `crates/desktop-server/tests/investigation_share_handoff.rs`
- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_handoff_ui.js` (keeping `investigation_node.js` conflict-free with V3 and V10!)
  - `crates/desktop-server/tests/investigation_share_handoff.test.mjs`

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/export/src/lib.rs` (132 lines): add `pub mod paper_map;` (re-exporting HTML/import helpers).
  - `crates/desktop-server/src/lib.rs` (215 lines): add `mod investigation_share;` (and `mod investigation_handoff;` if not already declared by F7).
  - `crates/desktop-server/src/handoff.rs` (from F7): wire V16's three outgoing Investigate packet fillers (`"Derive in Solve"`, `"Has this been extended?"`, `"Search wider in Explore"`) and V16's `target_mode: "investigate"` receiver directly into F7's shared hand-off route dispatch so none of V16's code is left unreachable.
  - `crates/desktop-server/src/api_dispatch.rs` (from F5): add 2 arms behind `AuthGuard::authorize_api` for `GET /api/sessions/{id}/investigation/export` and `POST /api/sessions/{id}/investigation/import`.
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_handoff_ui.js`.
  - **Conflict-free UI mount seam:** V3 owns `investigation_node.js` (Block A) and V10 owns `investigation_notes_ui.js`. V16 mounts `investigation_handoff_ui.js` ONLY into the dedicated `.inv-node-handoff-slot` action bar and V7's `investigation_badges.js` right half-badge slot — never modifying Block A/Block B or notes logic.
- **Do NOT edit:**
  - `crates/export/src/latex.rs` (**771 lines**, over limit!) and `crates/export/src/writeup*.rs` (owned by F15 / Group 5's `draft-f15-writeup-investigate.md`).
  - `crates/export/src/fork.rs` (`research_from_paper` and legacy `investigation.handoff` stay byte-for-byte unchanged for Set R!).
  - `crates/research-tools/tests/golden/tool_defs.txt`, `tool_profiles.txt`, `tool_profile.rs`, `Cargo.lock`.
  - `http_srv.rs` (702), `routes_session.rs` (616), `desktop-server/src/investigation.rs` (597), `index.html` (768), `app.js` (932).

## 7. Tests through the entry point

- **V15 tests (`crates/export/tests/paper_map_share.rs` + `investigation_share_handoff.rs`, zero network, zero model calls):**
  - Round-trip export -> import on a self-authored TeX paper: 100% of valid items re-match, carry `"imported"` (never `"verified"`), write host-only `source.ingested` + `investigation.skeleton` + readings/notes, and pass `Ledger::verify_all()` with `unreferenced_blobs == 0`.
  - Tampered quote in `.rsmap`: dropped during import, counted in `items_dropped`, valid siblings survive (rule 6.10/6.11).
  - Import with missing local source: opens read-only (`source_match: false`) with `"quotes not matched on this machine"` (zero `"verified"`).
  - Copyright & privacy gates (`investigate.md` 13 #9): default arXiv licence omits full statements unless `include_statements=1`; CC-BY licence allows them; quotes never exceed 300 chars; private notes omitted unless `include_notes=1`.
  - Standalone HTML: zero `<script>`/external `http(s)://` URLs, HTML-escaped XSS payloads (`<img onerror>`, `</style><script>`), zero `"verified"`.
- **V16 & Eval Set C alignment (`investigation_share_handoff.rs`):**
  - **P-to-Q ("Derive in Solve"):** theorem -> `prove`, equation -> `calculate`, proof -> `check_derivation`; statement closure, symbols, conventions, notes, and `model_text` travel through `desktop-server/src/handoff.rs`; 100% of quotes survive re-matching on unchanged bytes (`00-overview.md` 7.3 Set C); badges/tiers never travel; child `solve.done` appears as read-only chip on parent with zero writes to parent ledger.
  - **P-to-I ("Has this been extended?" and `"Search wider in Explore"`):** origin paper + bibliography seeds travel; V16's own merge tests simulated child `idea.verdict` rendering on the right half-badge (`"Opened in Explore: ..."`) and asserts zero `"new"`, `"novel"`, or `"verified"` on the right half-badge; full end-to-end Set C P-to-I origin-exclusion tests execute once Explore X11 merges.
  - **Receiving side (`target_mode: "investigate"`):** routed through `desktop-server/src/handoff.rs`; valid incoming quotes re-match via F4 and attach as notes on the innermost enclosing skeleton node; a tampered incoming quote is dropped and counted.
  - **Set R regression:** `eval_set_r` and legacy `research_from_paper` pass untouched.
- **Node UI tests (`node --test`):** right half-badge before/after Explore hand-off, Solve/Explore hand-off buttons, V15 export checkboxes & read-only banner, XSS escaping.

## 8. Document conflicts & resolutions

- **`investigate.md` 5.2 (`?format=rsmap / md / html`) vs `00-overview.md` 2.13 & `investigate.md` 9 V15:** Section 5.2 still lists `md` on the V15 export route, whereas `00-overview.md` 2.13 and `investigate.md` 9 row V15 explicitly move Markdown/TeX reading notes to shared package **F15** (`export/src/writeup_paper.rs`). Resolution (`00-overview.md` wins): V15 implements ONLY `rsmap` and `html`; reading notes stay in Group 5's `draft-f15-writeup-investigate.md`.
- **`f5-f7-mode-shell.md` vs `investigate.md` 9 V16 order note (`investigation_handoff.rs`):** `investigate.md` 9 says F7 writes the minimal `"Derive in Solve"` filler in `investigation_handoff.rs` in week 8, while `f5-f7-mode-shell.md` lists only `export/src/handoff.rs` and `desktop-server/src/handoff.rs`. Resolution: V16 checks `desktop-server/src/` after F7 merges — extending `investigation_handoff.rs` if F7 created it, or creating it cleanly if not.
- **`investigate.md` 4.4 (`investigation.handoff`) vs `00-overview.md` 3.1 row 8 / 4.3:** Old `investigation.handoff` is NOT extended; all V16 hand-overs use F7's `handoff.packet` + `handoff.sent`.

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope / deferred:** Investigate reading-notes write-up (`export/src/writeup_paper.rs` -> Group 5 `draft-f15-writeup-investigate.md`); V14 prior-work task inside Investigate (deferred); Solve-to-Explore `"Has this been computed before?"` (later); hosted map sharing library (optional add-on).
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v15-v16-share-handoff.md` and push to `gemini/modes/inv-v15-v16-share-handoff`.
