# Brief exp-x19-openalex — OpenAlex Literature Service via F3 Gate (X19 Part 1; TheoremSearch Skipped)

**Status:** Unblocked on `gemini/work/fixed` (`F3` and `F11` are merged, and **Owner Decision 2026-11-26** explicitly approved OpenAlex with a free per-user key through the F3 gate while keeping TheoremSearch skipped).
**Package:** `X19 Part 1` (OpenAlex search & citation relations, `api.openalex.org` allowlisted in `http_client.rs` and `THREAT-MODEL.md`, `Service::OpenAlex` in `net_rate.rs`, free per-user API key in `lit_settings.rs`, `abstract_ref.verbatim = false` so OpenAlex inverted-index reconstructions never count as `evidence.abstract`). **TheoremSearch stays skipped (`QueryStatus::Disabled`, NOT allowlisted).**
**Governing design:**
- `00-overview.md`: 2.3 (`openalex:W...`), 2.4 (F3 per-host gate), 2.8 (F11), 8.2/8.3.
- `explore.md`: 4.1 (`verbatim = false`), 5.2, 6 (C3, P6, P8), 8 (`#[serde(default)]` on `LiteratureEndpoints`), 9 (`X19`), 13 (decisions 8 & 14).
- `agents/RULES.md` (explicitly authorizes `exp-x19-openalex` to widen `ALLOWLISTED_HTTPS_HOSTS` for `"api.openalex.org"` and update `THREAT-MODEL.md`).

---

## 1. Verified Interfaces & Disjointness on `gemini/work/fixed`

- **`crates/research-tools/src/http_client.rs` (`330`L):** `ALLOWLISTED_HTTPS_HOSTS` lists the 5 hosts; `fetch_response_with_options_and_gate` already strips `HttpRequestOptions.extra_headers` on any cross-origin redirect (Rule P8).
- **`crates/research-tools/src/net_rate.rs` (`447`L):** `Service` has `Arxiv`, `Inspire`, `SemanticScholar`, `Crossref`. Add `Service::OpenAlex` (document rate limit e.g. `10 req/s` polite or conservative default explicitly as an assumption if not pinned by owner) with `from_host("api.openalex.org")` and `from_loopback_path` matching `/openalex/` BEFORE Crossref's `/works`.
- **`crates/research-tools/src/lit_providers.rs` (`585`L — only `15`L headroom!):** Split OpenAlex query execution & response parsing into a **new sibling `crates/research-tools/src/lit_openalex.rs`** so `lit_providers.rs` stays `<= 600` lines! Note `LitProvider::{OpenAlex, TheoremSearch}` already exist in the enum, so zero `idea_search` schema or `def_hash` change!
- **`crates/desktop-server/src/state.rs` (`598`L):** **DO NOT EDIT `state.rs`!** Put OpenAlex optional key/settings in standalone `crates/desktop-server/src/lit_settings.rs` persisted to `<data_dir>/lit_settings.json` (plus `OPENALEX_API_KEY` env fallback; API key stored securely / scrubbed, never written to ledger events or URLs!).

---

## 2. Checkpoints — Push to `gemini/modes/exp-x19-openalex` After Each!

### Checkpoint 1 — F3 OpenAlex Allowlist, `Service::OpenAlex`, and `lit_openalex.rs` (`crates/research-tools`)
1. `crates/research-tools/src/lib.rs`: add `#[serde(default = "default_openalex_api_base")] pub openalex_api_base: String` (`"https://api.openalex.org"`) to `LiteratureEndpoints`, keeping old 5-field JSON configs deserializable (`#[serde(default)]`); update `LiteratureEndpoints::loopback_mock` to use `format!("{trimmed}/openalex")` so shared-port loopback tests route `/openalex/...` cleanly to `Service::OpenAlex`.
2. `crates/research-tools/src/http_client.rs` (`ALLOWLISTED_HTTPS_HOSTS` += `"api.openalex.org"`, `endpoints.openalex_api_base` permitted on loopback) & `crates/research-tools/src/net_rate.rs` (`Service::OpenAlex`, `/openalex/` prefix before `/works`).
3. `THREAT-MODEL.md` line ~`269`: document `api.openalex.org` in the literature HTTPS allowlist (and state TheoremSearch remains excluded/off).
4. `crates/research-tools/src/lit_openalex.rs` + tiny dispatch hooks in `lit_providers.rs` (`<= 585`L -> `<= 600`L!), `lit_providers_url.rs`, `lit_providers_parse.rs`:
   - Support search (`?search=...` / `?filter=...`), `QueryRelation::Cites { key }`, and `QueryRelation::CitedBy { key }` via F2 `PaperKey` (`openalex:W...`, `doi:...`).
   - Send user's optional free OpenAlex API key strictly via `HttpRequestOptions.extra_headers` (`Authorization: Bearer <key>`) rather than query strings so secrets never enter logged URLs and cross-origin redirects strip the header (Rule P8). Also keep `LitProvider::TheoremSearch` returning `QueryStatus::Disabled`.
   - Reconstruct `abstract_inverted_index` deterministically with **`abstract_ref.verbatim = false`** so Rule C3 guarantees an OpenAlex rebuilt abstract can NEVER become `evidence.abstract`.
5. Run mandatory checks and push Checkpoint 1 to `gemini/modes/exp-x19-openalex`.

### Checkpoint 2 — Settings Sidecar (`lit_settings.rs`) & Full Tests
1. Create `crates/desktop-server/src/lit_settings.rs` (+ `crates/desktop-server/tests/lit_settings.rs`):
   - Persists `<data_dir>/lit_settings.json` (`openalex_api_key` masked on GET, clearable, never leaked in debug logs or ledger) + optional polite-pool opt-in, exposed via `GET /api/settings/literature` and `PUT /api/settings/literature` behind `AuthGuard::authorize_api` (1-line export in `desktop-server/src/lib.rs` and 1 match arm in `api_dispatch.rs`).
2. Add comprehensive loopback mock + `FakeRateClock` tests in `crates/research-tools/tests/optional_services.rs`:
   - Serde backward compatibility of 5-field `LiteratureEndpoints` JSON; `api.openalex.org` allowlisted; cross-origin redirect strips `Authorization: Bearer <key>`; `FakeRateClock` paces `Service::OpenAlex`; `abstract_inverted_index` produces `verbatim == false`; `LitProvider::TheoremSearch` stays `Disabled` with `0` HTTP calls; `tool_def_golden` and `tool_profile_golden` pass unchanged.
3. Run mandatory checks, write `docs/design/modes-2026-11/reports/exp-x19-openalex.md`, and push to `gemini/modes/exp-x19-openalex`.

---

## 3. Do-Not-Edit List & Verification (`RULES.md`)

- **Do NOT edit:** `crates/desktop-server/src/{state,idea_view,idea_exchanges*,routes_idea,routers,session_mode}.rs`, `crates/research-tools/src/{tool_profile,idea_brief,idea_tools,idea_triage,pdf_*,lines_sent,solve_*}.rs`, `tests/golden/{tool_defs,tool_profiles}.txt`, `Cargo.lock`.
- Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
  ```bash
  python3 scripts/check_file_length.py --base origin/gemini/work/fixed
  unset RE_SEARCH_PYTHON
  cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
  cargo test --workspace --locked --no-fail-fast
  ```
- Push **only** via `git push origin HEAD:refs/heads/gemini/modes/exp-x19-openalex`. Never squash, never open/merge PRs, and end with `PUSHED`.
