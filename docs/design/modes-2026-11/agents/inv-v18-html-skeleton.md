# Brief inv-v18-html-skeleton — Investigate arXiv HTML5/LaTeXML Skeleton & Number Cross-Confirmation (V18)

**Status:** Unblocked on `gemini/work/fixed` (`V1`, `V2`, `V3`, `V4`, `V11`, `F0`, `F2`, `F3`, `F10`, and Owner Decision 12 are merged).
**Package:** `V18` (deterministic HTML5/LaTeXML skeleton builder `SkeletonBuilder::Html1`, pure TeX/HTML number cross-confirmation upgrading `NumberBasis::Computed -> NumberBasis::Confirmed`, and F3-gated arXiv HTML fallback fetch in `investigation_open`).
**Governing design:**
- `00-overview.md`: 0.1, 2.3–2.5 (F2/F3/F4), 2.10, 7.2–7.3, 8.2, 8.3.
- `investigate.md`: 0.1 (`roxmltree` is XML-only; `arxiv.org` `/html/` is allow-listed), 3 (Stage C fallback TeX -> arXiv HTML -> PDF), 4.1 (`SkeletonBuilder::Html1`, `SkConfidence::HtmlClass`, `NumberBasis::Html` / `Confirmed`), rule 6.9 (`locate.rs` already recognizes `"html-1"`, `NumberBasis::Html`, and `NumberBasis::Confirmed`), sections 7 & 9 (`V18`), section 11 (LaTeXML HTML5), section 13 (**Decision 12**: build an **own small HTML tag scanner first** with **zero new crate dependencies**; stop only if fixtures prove an HTML5 crate is unavoidable).
- `agents/RULES.md` (mandatory workspace Clippy & test run, `<= 600`-line limit, no `Cargo.lock` edits, no third-party paper text, push after each checkpoint).

---

## 1. Verified Interfaces on `gemini/work/fixed`

- **`crates/research-tools/src/skeleton/model.rs` (`403` lines):** `SkeletonBuilder::Html1` (`"html-1"`), `SkConfidence::HtmlClass` (`"html_class"`), `NumberBasis::Html` (`"html"`), `NumberBasis::Confirmed` (`"confirmed"`), `MAX_SKELETON_NODES` (`6_000`), `MAX_SKELETON_EDGES` (`20_000`), `MAX_SKELETON_JSON_BYTES` (`2 MiB`).
- **`crates/research-tools/src/skeleton/mod.rs` (`275` lines):** exports TeX + PDF skeleton builders and `locate`.
- **`crates/research-tools/src/skeleton/locate.rs` (`508` lines):** already treats `"tex-1"` and `"html-1"` as non-PDF skeletons eligible for `LocateStatus::Found` when deterministic confirmation (`Confirmed`, `Html`, etc.) holds. Do **not** loosen `locate.rs`.
- **`crates/research-tools/src/net_rate.rs` & `http_client.rs`:** `net_rate::Service::Arxiv` (1 req / 3 s) already classifies `/html/` paths on `arxiv.org` / loopback mocks. `ResearchEnv.network_endpoints.arxiv_eprint_base` supplies the base origin (`https://arxiv.org` in production, `http://127.0.0.1:<port>` in loopback tests).
- **`crates/desktop-server/src/investigation_open.rs` (`540` lines):** near the 600-line ceiling! Put HTML open/fallback helpers in a **new sibling file `investigation_open_html.rs`** and add only a small `<= 15`-line delegation hook in `investigation_open.rs`.

---

## 2. Checkpoints — Push to `gemini/modes/inv-v18-html-skeleton` After Each!

### Checkpoint 1 — Own HTML5/LaTeXML Tag Scanner & `PaperSkeleton` Builder (`crates/research-tools`)
1. Create `crates/research-tools/src/skeleton/html.rs` (split pure tokenizer into `html_lex.rs` if exceeding `420` lines so every file stays comfortably `<= 600` lines):
   - Zero-dependency bounded HTML5 tag/attribute lexer handling void tags (`<br>`, `<hr>`, `<img>`, `<meta>`, `<link>`, `<input>`, etc.), unquoted/single/double-quoted attributes, HTML comments, raw-text `<script>`/`<style>` skipping, standard entity decoding (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#...;`), nested inline math/spans (`<math>`, `<span class="ltx_tag">`), byte/depth caps, and physical 1-based source line tracking in the ingested HTML file.
   - Document any inferred LaTeXML class conventions explicitly as assumptions in comments and in your report (`ltx_section`, `ltx_subsection`, `ltx_appendix`, `ltx_theorem` / `ltx_theorem_<kind>`, `ltx_proof`, `ltx_equation` / `ltx_equationgroup`, `ltx_figure`, `ltx_table`, `ltx_bibitem`, `ltx_bibliography`, `ltx_ref`, `ltx_cite`, `ltx_tag`).
   - Emit `PaperSkeleton` (`builder: SkeletonBuilder::Html1`, `confidence: SkConfidence::HtmlClass`, `number_basis: Some(NumberBasis::Html)`, permanent `n-...` V2 `node_tag`s via `tags::assign_tags`, physical `Span` lines that pass `quote_core::verify_quote` cleanly, and deterministic `locate_in_skeleton` compatibility).
2. Expose `load_ingested_html_source`, `build_html_skeleton`, `build_html_skeleton_from_ingested`, and `build_html_skeleton_timed` alongside `is_html_receipt` and add `pub mod html;` in `crates/research-tools/src/skeleton/mod.rs`.
3. Add comprehensive tests in `crates/research-tools/tests/skeleton_html.rs` + self-authored synthetic HTML5/LaTeXML fixtures under `crates/research-tools/tests/fixtures/skeleton_html/` (0 third-party paper text): determinism, F4 quote verification, `locate_in_skeleton` returning `Found`, malformed HTML5 void/unclosed-tag recovery, `<script>`/`<style>` safety, and size/node caps.
4. Run mandatory checks and push Checkpoint 1 to `gemini/modes/inv-v18-html-skeleton`.

### Checkpoint 2 — TeX/HTML Number Cross-Confirmation & F3 Fallback in `investigation_open`
1. Create `crates/research-tools/src/skeleton/html_confirm.rs`:
   - Pure `confirm_tex_numbers_with_html(tex: &mut PaperSkeleton, html: &PaperSkeleton) -> ConfirmReport`: matches statements conservatively (explicit `\label`/id or normalised statement fingerprint + kind) and upgrades `NumberBasis::Computed -> NumberBasis::Confirmed` **only** when both `SkNodeKind` and trimmed number agree verbatim; records mismatches (`disagreements`) without overwriting TeX numbers.
2. Create `crates/desktop-server/src/investigation_open_html.rs` (plus `crates/desktop-server/tests/investigation_open_html.rs`):
   - Fetches `<arxiv_eprint_base>/html/<target_id>` strictly via F3 `fetch_response_with_gate(..., MAX_EPRINT_BYTES, Some(Service::Arxiv), gate)` when TeX e-print is absent (before falling back to PDF in `investigation_open.rs` — add `OpenStep::HtmlFallback` cleanly in `investigation_open.rs` while keeping `investigation_open.rs <= 600` lines!), validates non-empty LaTeXML HTML bytes (rejecting ArXiv 404 / error / non-HTML pages cleanly so PDF fallback still runs!), and ingests via `ingest_bytes` (`source.ingested`) + builds `SkeletonBuilder::Html1` skeleton (recognized by `is_html_receipt` / `investigation_skeleton.rs` or via `investigation_open_html` wiring documented under Merge notes if `investigation_skeleton.rs` needs >15 lines).
   - Add loopback `RoutedMock` + `FakeRateClock` tests proving: (a) missing TeX -> `/html/<id>` fetched through `Service::Arxiv` 1 req / 3 s gate -> `html-1` skeleton built; (b) 404 `/html/<id>` falls through cleanly to PDF; (c) zero live network calls.
3. Re-run full mandatory checks, write `docs/design/modes-2026-11/reports/inv-v18-html-skeleton.md`, and push to `gemini/modes/inv-v18-html-skeleton`.

---

## 3. File Ownership & Do-Not-Edit List (Zero Collision With Other Running Lanes)

- **New:** `crates/research-tools/src/skeleton/html.rs` (`+ html_lex.rs`, `html_confirm.rs`), `crates/research-tools/tests/skeleton_html.rs`, `crates/research-tools/tests/fixtures/skeleton_html/*`, `crates/desktop-server/src/investigation_open_html.rs`, `crates/desktop-server/tests/investigation_open_html.rs`, `docs/design/modes-2026-11/reports/inv-v18-html-skeleton.md`.
- **Small localized edits (`<= 600` lines):** `crates/research-tools/src/skeleton/mod.rs` (`275`L), `crates/desktop-server/src/lib.rs` (`234`L), `crates/desktop-server/src/investigation_open.rs` (`540`L — keep `<= 580`L!). If `investigation_skeleton.rs` (`531`L) needs a 3-line hook to dispatch `build_html_skeleton_from_receipt`, keep it `<= 5 lines` OR document exact hook in Merge notes.
- **Do NOT edit:** `Cargo.lock`, `crates/research-tools/tests/golden/tool_defs.txt`, `tool_profiles.txt`, `tool_profile.rs`, `skeleton/tex_*.rs`, `skeleton/pdf*.rs`, `net_rate.rs`, `paper_fetch.rs`, `investigation_mapview.rs`, `investigation_tasks.rs`, `investigation_notes.rs`, `investigation_legacy_view.rs`.

---

## 4. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Push **only** via `git push origin HEAD:refs/heads/gemini/modes/inv-v18-html-skeleton`. Never squash, never open/merge PRs, and end with the `PUSHED` block from `RULES.md`.
