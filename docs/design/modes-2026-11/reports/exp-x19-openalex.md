# Report: `exp-x19-openalex` (X19 Part 1 — OpenAlex Literature Service via F3 Gate; TheoremSearch Skipped)

- **Base commit (`origin/gemini/work/fixed`):** `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7` (merged `--no-ff` onto prior Checkpoint 1 branch tip `3e1b08bcfde39961897bda1549e3e3c3143c691c` in `837e5af`)
- **Work branch:** `work/exp-x19-openalex` -> pushed to `gemini/modes/exp-x19-openalex`
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)` on Linux x86_64 (`uid=1001`, non-root)
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` and `crates/research-tools/tests/golden/tool_profiles.txt` untouched; no tool definitions or profiles added or modified; `crates/desktop-server/src/state.rs` and all Do-Not-Edit files untouched; `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`; zero new `#[ignore]` or skipped tests.

---

## 1. What was implemented (Checkpoint by Checkpoint)

### Checkpoint 1 — F3 OpenAlex Allowlist, `Service::OpenAlex`, and `lit_openalex.rs` (`b1f1b45`, `c313e63`, `3e1b08b`, audited & hardened in `c389308`)
- `crates/research-tools/src/lib.rs` (362 lines): adds `pub mod lit_openalex;`, `pub fn default_openalex_api_base()`, and `#[serde(default = "default_openalex_api_base")] pub openalex_api_base: String` on `LiteratureEndpoints` (preserving deserialization of legacy 5-field JSON configs). Updates `LiteratureEndpoints::loopback_mock` to set `openalex_api_base: format!("{trimmed}/openalex")` so shared-port loopback tests route `/openalex/...` cleanly to `Service::OpenAlex`.
- `crates/research-tools/src/http_client.rs` (347 lines): adds `"api.openalex.org"` to `ALLOWLISTED_HTTPS_HOSTS` (authorized by Owner Decision 2026-11-26 in `RULES.md`), permits `endpoints.openalex_api_base` on loopback mocks, and tightens `validate_extra_headers` to reject non-ASCII (`>= 0x7f`) and control characters (other than HTAB) in header values so bad API keys fail early instead of hitting `ureq`'s header parser. Cross-origin redirects strip `extra_headers` (Rule P8).
- `crates/research-tools/src/net_rate.rs` (464 lines) & `crates/research-tools/src/net_rate_tests.rs` (195 lines): adds `Service::OpenAlex` (`10 req / 1 s`, documented as `ASSUMPTION`), `Service::from_host("api.openalex.org")`, `Service::from_loopback_path` matching `/openalex` **before** Crossref's `/works`, and `resolve_bucket` port classification for `endpoints.openalex_api_base`.
- `crates/research-tools/src/lit_openalex.rs` (542 lines, new): OpenAlex provider runner & parser split into a sibling module so `lit_providers.rs` stays under 600 lines:
  - `apply_auth_header`: attaches optional OpenAlex API key strictly as `Authorization: Bearer <key>` via `HttpRequestOptions.extra_headers` (never in URLs, logs, or ledger payloads; stripped on cross-origin redirects per Rule P8).
  - `build_request_url`: supports free-text `?search=...`, explicit `filter:` queries, `QueryRelation::Cites { key }` (`filter=cited_by:<target>`), and `QueryRelation::CitedBy { key }` (`filter=cites:<target>`) for `openalex:W...` and `doi:...` keys, clamping `per-page` to `1..=200`.
  - `reconstruct_abstract_inverted_index`: deterministic reconstruction using a bounded max-heap of at most `16_384` `(pos, token)` pairs (capped at `262_144` scanned entries, stopping at `MAX_ABSTRACT_CHARS = 3000`) so earliest positions are always preserved regardless of alphabetical key order and sparse huge positions cannot trigger OOM.
  - `parse_openalex_json`: emits F2 `PaperRef`/`PaperKey` records with **`abstract_ref.verbatim = false`** (Rule C3 guarantees OpenAlex rebuilt abstracts can never become `evidence.abstract`), validates `landing_page_url` scheme (`http://`/`https://`), prevents single-work self-loop edges on `Cites`/`CitedBy`, deduplicates `referenced_works`, and includes unit tests for these edge cases.
- `crates/research-tools/src/lit_providers.rs` (591 lines), `crates/research-tools/src/lit_providers_url.rs` (264 lines), `crates/research-tools/src/lit_providers_parse.rs` (435 lines), `crates/research-tools/src/lit_abstracts.rs` (372 lines), `crates/research-tools/src/lit_providers_tests.rs` (151 lines), `crates/research-tools/tests/lit_providers.rs` (575 lines): wires `LitProvider::OpenAlex` into F11 dispatch, `ProviderConfig.openalex_api_key` (redacted in `Debug`), and `extract_abstract_from_blob` `provider_id` fallback while keeping `LitProvider::TheoremSearch` disabled (`net_service() == None`, returns `QueryStatus::Disabled` with 0 HTTP calls).

### Checkpoint 2 — Literature Settings Sidecar, Integration Tests & Threat Model (`375af0a`, `6e25523`, audited & hardened in `17aa839`)
- `crates/desktop-server/src/lit_settings.rs` (498 lines, new): standalone sidecar persisting `<data_dir>/lit_settings.json` without touching `state.rs`:
  - Stores optional `openalex_api_key`, `openalex_polite_pool: bool` (defaults `false`), and `openalex_mailto: Option<String>` (defaults `None`; `effective_polite_mailto()` returns `Some` only when both opt-in and valid email are present).
  - Enforces visible ASCII validation on keys (`<= 512` bytes) and emails (`<= 254` bytes) without ever echoing invalid secrets in error messages.
  - Masks keys on `GET`/`PUT` (`****<last4>` for len >= 8, `"****"` for shorter keys) with provenance (`"saved"`, `"env"`, `"none"`), supports `OPENALEX_API_KEY` env fallback via hermetic `DesktopState::env_var`, redacts `Debug`, scrubs key buffers on drop/serialization, writes atomically with `0600` Unix permissions, refuses symlinks, uses `.bak` backup-and-rollback replacement on Windows rename fallback, serializes reads & writes under `state.ledger_write_lock`, and fails closed (HTTP 500) on corrupt sidecar files without overwriting them.
  - Exposes `GET /api/settings/literature` and `PUT /api/settings/literature` behind `AuthGuard::authorize_api` via a 1-line export in `crates/desktop-server/src/lib.rs` (254 lines) and 1 match arm in `crates/desktop-server/src/api_dispatch.rs` (490 lines).
  - Provides `resolve_provider_config(state: &DesktopState) -> ProviderConfig` for downstream Explore/literature callers.
- `crates/desktop-server/tests/lit_settings.rs` (367 lines, new): 5 HTTP integration tests verifying `AuthGuard::authorize_api` (401 without token / bad token / stream cookie only, 403 on cross-origin `Origin`, 200 with `X-ReSearch-Token` and `Authorization: Bearer`), save/mask/partial-patch/clear persistence, `0600` Unix mode, zero ledger events written, `OPENALEX_API_KEY` hermetic env fallback & override, malformed/non-ASCII/oversized rejection without echoing secrets, and fail-closed HTTP 500 on corrupt `lit_settings.json` without wiping the file.
- `crates/research-tools/tests/optional_services.rs` (565 lines, new): 5 integration tests covering 5-field `LiteratureEndpoints` serde backward compatibility, `api.openalex.org` allowlist (and TheoremSearch exclusion), OpenAlex search/`filter:`/`Cites`/`CitedBy`/`abstract_inverted_index` (`verbatim == false`, Rule C3 rejection in both `locate_abstract_quote` and `locate_abstract_quote_in_blob`), ledger recording + `Library::rebuild()`, cross-origin redirect stripping `Authorization: Bearer <key>`, HTTP 401 redaction, HTTP 429 `Retry-After` parsing, `FakeRateClock` pacing `Service::OpenAlex` (10 req / 1 s) independently from Crossref on a shared loopback port, and `LitProvider::TheoremSearch` staying `Disabled` with `0` HTTP calls.
- `THREAT-MODEL.md` (1133 lines): documents `api.openalex.org` in the literature HTTPS allowlist (and TheoremSearch exclusion) plus `<data_dir>/lit_settings.json` and `GET`/`PUT /api/settings/literature` storage & auth invariants.

---

## 2. Independent Audit Findings & Resolutions

An independent read-only audit (`research` / `pro`) reviewed `origin/gemini/work/fixed..HEAD` and caught the following issues, all resolved in `c389308` and `17aa839`:
1. **Corrupt `lit_settings.json` silently overwritten on `PUT` (HIGH):** `handle_get`/`handle_put` now call `try_load_settings(...).map_err(internal)?` and return HTTP 500 without modifying a corrupt file (regression test added).
2. **Unlocked reads & destructive Windows replace fallback (HIGH):** `handle_get`, `handle_put`, and `resolve_provider_config` now serialize on `state.ledger_write_lock`; Windows replace fallback stages the existing file to `.bak` and restores it if committing `.tmp` fails.
3. **Inverted-index cap dropping earliest tokens on alphabetically early keys (MEDIUM):** replaced the pre-sort `pairs.len() >= 16_384` early break in `reconstruct_abstract_inverted_index` with a bounded max-heap (`BinaryHeap<(u64, &str)>`) so the earliest `16_384` positions are always retained deterministically (unit test added).
4. **Non-ASCII keys accepted by `lit_settings` / `http_client` (MEDIUM):** `validate_api_key` and `validate_mailto` now require visible ASCII (`is_ascii_graphic`), and `http_client::validate_extra_headers` rejects control characters and `>= 0x7f` bytes.
5. **Symlink refusal & `THREAT-MODEL.md` sidecar documentation (MEDIUM):** `try_load_settings` and `save_settings` check `symlink_metadata` and refuse symlinks; `<data_dir>/lit_settings.json` is documented in `THREAT-MODEL.md`.
6. **Single-work relation self-loop, duplicate `referenced_works`, `landing_page_url` scheme, and `per-page=0` (LOW):** hardened in `lit_openalex.rs` with unit tests, and added HTTP 429 `Retry-After` coverage in `optional_services.rs`.

---

## 3. Decisions Taken Alone, Deviations, and Assumptions

### Decisions
1. **Saved-key-over-env precedence in `lit_settings.rs`:** A key saved in `<data_dir>/lit_settings.json` (`source: "saved"`) takes precedence over `OPENALEX_API_KEY` (`source: "env"`), matching the brief's "`OPENALEX_API_KEY` env fallback" specification. Clearing the saved key (`null` or `""`) restores the env fallback.
2. **Safe zero-scrub without touching `Cargo.lock`:** `desktop-server/Cargo.toml` does not list `zeroize`, and adding a workspace dependency edge to `desktop-server/Cargo.toml` would modify `Cargo.lock` (forbidden by `RULES.md`). `lit_settings.rs` therefore uses safe `Vec<u8>::fill(0)` on owned buffers in `Drop` and after serialization (and zero-copy ASCII slicing in `mask_api_key`). Note that LLVM can theoretically optimize away non-volatile fills before drop; if stronger volatile zeroization is desired in `desktop-server`, a follow-up that is allowed to update `Cargo.lock` can add `zeroize = { workspace = true }` to `crates/desktop-server/Cargo.toml`.
3. **Polite-pool opt-in vs Rule P8:** `openalex_polite_pool` (default `false`) and `openalex_mailto` (default `None`) are persisted and exposed via `/api/settings/literature`, with `LitSettings::effective_polite_mailto()` returning `Some` only when both are set. Per `explore.md` Rule P8 (`User-Agent` stays generic `re-search/<version>`, no email in logged URLs by default), `http_client::USER_AGENT` and `lit_openalex::build_request_url` remain generic and carry no contact email in Part 1.
4. **Scaffolding scope of `resolve_provider_config`:** `lit_settings::resolve_provider_config` is exported for upcoming Explore X7/X19 runners; existing `desktop-server/src/idea_search.rs` (first-search on arXiv/INSPIRE/S2/Crossref) and `routes_tools.rs` (`run_literature_search`) were left untouched to stay disjoint from other lanes.

### Deviations from the design / existing tests
- **`crates/research-tools/tests/lit_providers.rs:321,368-374` (done in Checkpoint 1 commit `c313e63`):** The existing F11 test `citation_relations_and_unsupported_and_disabled_openalex` previously asserted `LitProvider::OpenAlex` returned `QueryStatus::Disabled`. Once X19 enabled OpenAlex, Checkpoint 1 updated that assertion to test `LitProvider::TheoremSearch` returning `QueryStatus::Disabled` while leaving the test function name unchanged.

### Assumptions about external services (`api.openalex.org`)
- **Rate limit (`crates/research-tools/src/net_rate.rs:83`):** `Service::OpenAlex` is paced at **10 requests per 1 second** (conservative assumption for OpenAlex's polite / free-key pool; not numerically pinned in `00-overview.md` section 2.4).
- **REST API shape (`crates/research-tools/src/lit_openalex.rs:45-49,204`):** Assumes `/works` accepts `search=<q>`, `filter=<expr>`, `per-page=<n>`, and 1-indexed `page=<p>`; `filter=cited_by:<work>` returns outgoing references (`QueryRelation::Cites`), `filter=cites:<work>` returns incoming citations (`QueryRelation::CitedBy`), both accepting `W<digits>` and `https://doi.org/<doi>`; paginated responses carry `meta.count` and `results[]` with `id`, `doi`, `ids.{openalex,doi,arxiv}`, `title`/`display_name`, `publication_year`, `cited_by_count`, `is_retracted`, `authorships[]`, `primary_location.landing_page_url`, and `abstract_inverted_index`; and optional free API keys are accepted as `Authorization: Bearer <key>`.

---

## 4. Merge Notes

- **Modified existing files:**
  - `THREAT-MODEL.md` (lines 266–277, 361–368)
  - `crates/research-tools/src/lib.rs` (`pub mod lit_openalex`, `default_openalex_api_base`, `LiteratureEndpoints.openalex_api_base`, `loopback_mock`)
  - `crates/research-tools/src/http_client.rs` (`ALLOWLISTED_HTTPS_HOSTS`, loopback bases, ASCII header value validation)
  - `crates/research-tools/src/net_rate.rs` & `net_rate_tests.rs` (`Service::OpenAlex`, `/openalex` path prefix before `/works`)
  - `crates/research-tools/src/lit_providers.rs`, `lit_providers_url.rs`, `lit_providers_parse.rs`, `lit_abstracts.rs`, `lit_providers_tests.rs`, `tests/lit_providers.rs`
  - `crates/desktop-server/src/lib.rs` (+1 line: `pub mod lit_settings;`)
  - `crates/desktop-server/src/api_dispatch.rs` (+3 lines: 1 match arm for `("GET" | "PUT", ["api", "settings", "literature"])`)
- **New files:**
  - `crates/research-tools/src/lit_openalex.rs`
  - `crates/research-tools/tests/optional_services.rs`
  - `crates/desktop-server/src/lit_settings.rs`
  - `crates/desktop-server/tests/lit_settings.rs`
- **Integration point for Explore / UI lanes:** Call `desktop_server::lit_settings::resolve_provider_config(state)` when building `ProviderConfig` for `lit_providers::run_query*`, and call `GET`/`PUT /api/settings/literature` from the Settings UI to manage the masked OpenAlex key and polite-pool opt-in.

---

## 5. Verification, Exact Counts, and Limitations

All verification commands ran on Linux x86_64 as non-root (`uid=1001`) with `RE_SEARCH_PYTHON` unset:

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched Rust file: `lit_providers.rs` at 591 lines) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/research-tools/tests/golden/tool_profiles.txt` | **Pass** — empty diff (all 3 files untouched) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — 0 syntax errors |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **231 passed, 0 failed, 0 skipped** |
| `cargo test --workspace --locked --no-fail-fast -- --quiet` | **Pass** — **1226 passed, 0 failed, 2 ignored** across **158** test/doc-test targets (`lit_openalex` unit 3/3, `optional_services` 5/5, `lit_providers` 5/5, `tool_def_golden` 3/3, `tool_profile_golden` 3/3, `lit_settings` unit 2/2, `lit_settings` integration 5/5) |

### Pre-existing items
- **2 ignored tests:** Pre-existing `#[ignore = "live tier: ..."]` tests in `crates/agent/tests/eval_harness.rs:468,481` (unchanged by this branch).
- **`manual_unwrap_or` lint:** Known pre-existing lint in `crates/desktop-server/src/evidence.rs:744` (suppressed via `-A clippy::manual_unwrap_or` per `RULES.md`).

### Could not compile or run on Linux
- `#[cfg(windows)]` targets and Windows-specific behavior (e.g., Windows file-rename fallback path in `lit_settings::save_settings`, Windows DACL inheritance on `<data_dir>/lit_settings.json`, `re-search-app` WebView2 window, and Windows CI integration tests) were verified by static inspection only and could not be executed on Linux.

### Issues noticed in existing code (left untouched outside scope)
- `crates/research-tools/src/library_rebuild_lit.rs:54,91-143`: `Library::rebuild` does not have an OpenAlex raw-blob parser branch, so it falls back to event-payload `hits[]` (preserving `key`, `title`, `year`, `retracted`, `doi`/`arxiv`/`openalex` ids, while leaving `authors` and `bibtex` empty).
- `crates/research-tools/src/http_client.rs:116-120`: `is_url_permitted` checks only the HTTPS hostname, not the port, so non-443 ports on allowlisted hosts are not rejected (pre-existing behavior).
- `crates/desktop-server/assets/source_links.js` & `crates/desktop-app/src/rules.rs`: external link allowlists do not yet include `openalex.org` (left untouched per Do-Not-Edit / scope boundaries).
- `crates/desktop-server/tests/api_auth.rs:79,111`: route-wide auth matrix does not yet list `/api/settings/literature` (covered directly in `crates/desktop-server/tests/lit_settings.rs`).
- `crates/research-tools/tests/lit_providers.rs:321`: test function is still named `citation_relations_and_unsupported_and_disabled_openalex` even though it now tests `LitProvider::TheoremSearch` as disabled.
