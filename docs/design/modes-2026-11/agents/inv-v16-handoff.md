# Brief inv-v16-handoff — Investigate Cross-Mode Hand-Off to Explore & Solve + `"Search wider in Explore"` (V16)

**Status:** Ready / Unblocked (`V2–V8`, `V10`, `F5`, `F7` merged on `gemini/work/fixed`). File-disjoint from `inv-v15-share` and `inv-v12-context`! Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/inv-v16-handoff`

---

## 1. Package & Governing Design Sections

- **Package:** **V16** — Investigate's three outgoing hand-off builders built on F7 (`"Derive in Solve"`, `"Has this been extended?"`, and `"Search wider in Explore"` moved from deferred V14 into V7's right half-badge slot), full node packet filler (verbatim statement + `QuoteRef`s, statement closure, `SymbolDef` symbols, paper conventions, reader notes in `user_notes`, model gists in `model_text` tagged `"(model)"`, origin paper + resolved bibliography seeds), Investigate incoming hand-off receiver (`target_mode: "investigate"`), read-only child-session status chips (`solve.done` / `idea.verdict`), and `assets/investigation_handoff_ui.js`.
- **Governing design:**
  - `00-overview.md`: 0.1 (`"Search wider in Explore"` moved from deferred V14 to V16), 2.3 (F2 `PaperRef`, `NodeRef`), 2.5 (F4 `verify_quote`, 300-char quote cap), 2.9 (F12 wording: right side never contains `"new"` or `"novel"`), 3.1 rows 8–9 (F7 `handoff.packet`, Solve task kinds `prove` / `calculate` / `check_derivation`), 3.2 row 20 (zero `"verified"` in Investigate), 4.1–4.3 (hand-off rules & table), 7.2–7.3 (Set C chains: 100% quote survival on unchanged bytes; Set R untouched), 8.2 (#8 hand-overs re-match every quote and never carry a badge).
  - `investigate.md`: 2 (worked example 4:00, 5:30, 9:00), 3 (stage I Earlier work, novelty row right half-badge), 4.4 (`handoff.sent`; legacy `investigation.handoff` stays unchanged), sections 7, 9, 13 (decisions 3 & 7).

---

## 2. Verified F7 & V7–V10 Interfaces on `gemini/work/fixed`

- **`crates/export/src/handoff.rs` & `crates/desktop-server/src/handoff.rs` (`79`L):** F7 `execute_handoff`, `HandoffPacket` v1, `HandoffRequest`, `read_session_handoffs`, and HTTP routes `POST /api/sessions/{id}/handoff` + `GET /api/sessions/{id}/handoffs` ALREADY exist!
- **V7/V8/V10 slots ALREADY mounted on node cards:**
  - `assets/investigation_node.js`: already creates `.inv-node-handoff-slot` (`data-node-tag`) right below `.inv-node-actions` and returns `{ ..., handoffSlot }`, plus action buttons `derive_solve` (`"Derive in Solve"`) and `extend_explore` (`"Has this been extended?"`).
  - `assets/investigation_badges.js`: already reserves the right half-badge slot (`RIGHT_SLOT_V16_LABEL` / `[data-half="right"]`)!
- **Skeleton symbols (`V5`):** `SymbolDef` in `skeleton/symbols.rs`; folded notes & claims in `map_fold.rs` (`FoldedMap`); child statuses read-only (`Ledger::read_session`, checking `solve.done` if present or fallback `"Opened in Solve: in progress"`, and `idea.verdict` if present or fallback `"Opened in Explore: not finished"`).

---

## 3. Smallest Implementation Checkpoints (Push to `gemini/modes/inv-v16-handoff` After Each!)

1. **Checkpoint 1 — Outgoing packet builders, incoming receiver, & child-status projection (`crates/desktop-server/src/investigation_handoff.rs`, split `investigation_handoff_receive.rs` if `> 450`L):**
   - Zero model calls & zero network calls on F10/session cache hit:
     - `"Derive in Solve"` (`target_mode: "solve"`): maps `theorem`-like nodes -> `prove`, equations -> `calculate`, proofs -> `check_derivation` (`00-overview.md` 3.1 row 9); includes verbatim statement + `QuoteRef`s, statement closure (used defs/assumptions/equations) in `context`, `SymbolDef` symbols, paper conventions, reader notes in `user_notes`, and model gists ONLY in `model_text` (tagged `"(model)"`). Never copies badges/tiers.
     - `"Has this been extended?"` (`target_mode: "explore"`): statement text + origin `PaperRef` (marked so Explore excludes it from `"found"` verdicts) + resolved bibliography `PaperRef`s as seeds + `user_notes` + `model_text`.
     - `"Search wider in Explore"` (`target_mode: "explore"`, right half-badge action, moved from deferred V14): same packet with `ask.kind = "prior_work"` / question `"is this earlier"`, origin paper + resolved bibliography seeds.
   - **Receiving side (`target_mode: "investigate"`):** opens/ingests `PaperRef` (library first, zero network on cache hit), builds/loads skeleton, verifies every incoming `QuoteRef` against the new session's own `source.ingested` via F4 (dropping & counting failures; badges never travel), and attaches each surviving quote as a note on the **innermost** enclosing skeleton node.
   - Wire both outgoing builder convenience / validation AND `target_mode: "investigate"` receiver cleanly into `crates/desktop-server/src/handoff.rs` (`79`L).
   - **Read-only child-status projection (`child_handoff_statuses(ledger, parent_events)`):** scans `handoff.sent` in parent session, reads each child session's latest `solve.done` or `idea.verdict` strictly read-only (**never** writes back into parent ledger), and enforces F12 wording invariant (right half-badge NEVER contains `"new"`, `"novel"`, or `"verified"`).
2. **Checkpoint 2 — Hand-off UI & tests (`crates/desktop-server/assets/investigation_handoff_ui.js`):**
   - Renders `"Derive in Solve"`, `"Has this been extended?"`, `"Search wider in Explore"` in V7's reserved right half-badge slot (`investigation_badges.js`) + `.inv-node-handoff-slot` (`investigation_node.js`), and read-only child status chips with click-to-open-child navigation. Strictly XSS-safe (zero `innerHTML`).
   - Add `crates/desktop-server/tests/investigation_handoff.rs` (Set C `P-to-Q` 100% quote survival on unchanged bytes, `P-to-I` + `"Search wider in Explore"`, simulated child `solve.done` & `idea.verdict` status chips with zero parent writes, incoming receiver dropping tampered quotes, Set R `research_from_paper` untouched) and `crates/desktop-server/tests/investigation_handoff_ui.test.mjs`.

---

## 4. Verification & Do-Not-Edit (`RULES.md`)

- **Shared 1-line registrations ONLY:** `crates/desktop-server/src/lib.rs` (`mod investigation_handoff;`), `crates/desktop-server/src/handoff.rs`, `crates/desktop-server/src/assets.rs` (`/assets/investigation_handoff_ui.js`), `crates/desktop-server/src/api_dispatch.rs` (only if adding a build-packet helper endpoint).
- **Do NOT edit:** `crates/export/src/{fork.rs, latex.rs, paper_map*.rs, writeup*.rs}`, `crates/desktop-server/src/{investigation_share.rs, turn_runner.rs, agent_models.rs, http_srv.rs, routes_session.rs, routes_tools.rs}`, `crates/desktop-server/assets/{investigation_share_ui.js, index.html, app.js}`, `crates/research-tools/tests/golden/*`, `Cargo.lock`.
- Run non-root (`id -u != 0`, `unset RE_SEARCH_PYTHON`) Linux checks (`check_file_length.py`, `node --check`, `node --test`, `clippy --workspace --all-targets --locked`, `cargo test --workspace --locked`), write `docs/design/modes-2026-11/reports/inv-v16-handoff.md`, and push to `gemini/modes/inv-v16-handoff`. End with `PUSHED`.
