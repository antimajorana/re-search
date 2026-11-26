# Brief exp-x19-optional-services — Optional literature services: OpenAlex and TheoremSearch (package X19)

## 1. Package and governing design sections

- **Package:** `X19` (Optional services — OpenAlex and TheoremSearch, split into two independent S-sized checkpoints so OpenAlex is never blocked on TheoremSearch terms review).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.3 (F2 `openalex:<W id>` already supported), 2.4 (F3 per-host network gate), 2.8 (F11 literature services), and 8.2/8.3.
  - `docs/design/modes-2026-11/explore.md`: sections 4.1 (`Candidate.abstract_ref.verbatim = false` for OpenAlex rebuilt abstracts), 5.2 (`idea_search` provider enum already carries `openalex` and `theoremsearch` from X7 so NO `def_hash` change), 6 (rules C3, P6, P8, S1, S2), 8 (item 3: `LiteratureEndpoints` new fields carry `#[serde(default)]`), 9 (row X19), 10 (free core vs optional services), 11 (TheoremSearch: no licence, optional public service only, never its code, off by default), 12 (risk 7), and 13 (decisions 8 & 14).
  - `THREAT-MODEL.md` (lines 268–270: literature HTTPS allowlist documentation).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and MANDATORY OWNER APPROVAL GATES

- **Required merges on `gemini/work/fixed`:**
  - `F3` (merged in base: `http_client.rs`, `net_rate.rs`), `F11` (`f11-lit-services`: `lit_providers.rs`, `lit_abstracts.rs`), and `X7` (`exp-x6-x7-x8-search-triage`: `idea_search` provider enum already includes `openalex` and `theoremsearch`).
- **MANDATORY OWNER APPROVAL GATES BEFORE DISPATCH (DO NOT DISPATCH WITHOUT EXPLICIT YES):**
  1. **Checkpoint 1 (OpenAlex) owner gate:** explicit owner approval to widen `ALLOWLISTED_HTTPS_HOSTS` in `crates/research-tools/src/http_client.rs` to add `api.openalex.org`, update `THREAT-MODEL.md:268`, and confirm OpenAlex API key / optional polite-pool settings handling (`explore.md` 13 decisions 8 & 14).
  2. **Checkpoint 2 (TheoremSearch) owner gate:** explicit owner sign-off on (a) TheoremSearch's exact verified HTTPS hostname and acceptable terms of use (`explore.md` 13 decision 8: `"TheoremSearch off until its terms are known"`), (b) widening `ALLOWLISTED_HTTPS_HOSTS` + `THREAT-MODEL.md` for that host, and (c) the exact versioned consent notice text (following `crates/desktop-server/src/antigravity.rs`).
  - **Independent dispatch:** Checkpoint 1 (OpenAlex) may be dispatched as soon as Gate 1 is signed off, leaving TheoremSearch disabled (`status: "disabled"`) until Gate 2 is signed off.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/research-tools/src/http_client.rs` (234 lines): `ALLOWLISTED_HTTPS_HOSTS` has strictly 5 hosts (`export.arxiv.org`, `arxiv.org`, `inspirehep.net`, `api.crossref.org`, `api.semanticscholar.org`), `is_url_permitted` checks `endpoints.*_api_base` for loopback ports, `USER_AGENT` is `re-search/<version>`.
- `crates/research-tools/src/net_rate.rs` (447 lines): `Service::{Arxiv, Inspire, SemanticScholar, Crossref}`, `Service::ALL`, `as_str`, `rate_limit`, `from_host`, `from_loopback_path`.
- `crates/research-tools/src/lib.rs` (300 lines): `LiteratureEndpoints` (5 fields, `Default`, `loopback_mock`).
- `crates/research-tools/src/paper_key.rs`: `PaperKey::from_openalex("W...")` (`openalex:W...`) is ALREADY implemented and tested!
- `crates/research-tools/src/library_schema.rs` & `library_rebuild_lit.rs`: `ids.openalex` is ALREADY supported!
- `crates/desktop-server/src/antigravity.rs` (285 lines): proven versioned-consent pattern (`NOTICE_VERSION`, `acknowledged`, `require_acknowledged`, `handle_acknowledge`, `handle_withdraw`).
- **`crates/desktop-server/src/state.rs` is 598 lines (2 lines below the 600-line ceiling!):** DO NOT add optional-service settings/consent structs inline in `state.rs`! Place them in a new `crates/desktop-server/src/lit_settings.rs` (or split `state.rs` first in a zero-behaviour-change commit).
- `THREAT-MODEL.md:268-269`: documents the 5 allowlisted literature hosts.

### Future-merge interfaces to verify before coding
- **`F11` (`lit_providers.rs`, `lit_abstracts.rs`):** exact provider dispatch, `literature.fetched` blob writer, and `abstract_ref { raw_blob, provider, verbatim }` shape.
- **`X7` (`idea_tools.rs`):** verify `idea_search` already dispatches `"openalex"` and `"theoremsearch"` through `lit_providers` (so X19 changes ZERO tool schemas and `tool_profiles.txt` stays byte-identical!).
- **`F5` (`api_dispatch.rs`):** verify where settings/connections routes register.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — OpenAlex (requires Owner Gate 1):**
   - `crates/research-tools/src/lib.rs`: add `#[serde(default = "default_openalex_api_base")] pub openalex_api_base: String` to `LiteratureEndpoints` (`"https://api.openalex.org"`), keeping existing serialized configs deserializable (`explore.md` 8 item 3); update `loopback_mock` to pin a distinct loopback path prefix (e.g., `format!("{trimmed}/openalex")` so OpenAlex requests hit `/openalex/works` rather than colliding with Crossref's `/works` on a shared-port mock — or bind a separate loopback origin in multi-service tests).
   - `crates/research-tools/src/http_client.rs`: add `"api.openalex.org"` to `ALLOWLISTED_HTTPS_HOSTS`, add `endpoints.openalex_api_base` to loopback allowed bases, and support per-request optional auth headers sent ONLY to the matching host (Rule P8 — never leak an API key across hosts or redirects!).
   - `crates/research-tools/src/net_rate.rs`: add `Service::OpenAlex` with an explicit conservative rate limit (mark rate limit as an assumption if not pinned by owner, e.g., 10 req/s or 1 req/s polite default), `from_host("api.openalex.org")`, and `from_loopback_path` matching `/openalex/` (ahead of `/works`) so shared-port loopback tests NEVER mis-bucket `Service::OpenAlex` into `Service::Crossref` even when called without an explicit `Service` hint!
   - `crates/research-tools/src/lit_providers.rs` (and `lit_openalex.rs` sibling if `lit_providers.rs` approaches 540 lines): implement OpenAlex search & citation-relations runner keyed by F2 `PaperKey` (`arxiv:` -> `doi:` -> `openalex:W...`); reconstruct `abstract_inverted_index` strictly with **`abstract_ref.verbatim = false`** so Rule C3 guarantees an OpenAlex abstract can NEVER become `evidence.abstract`!
   - `crates/desktop-server/src/lit_settings.rs` (new, keeping `state.rs` `<= 600` lines): optional OpenAlex API key (stored in settings/env, never logged or put in ledger) and optional opt-in Crossref polite-pool contact email (off by default, `explore.md` 13 decision 14); default `User-Agent` stays `re-search/<version>` (Rule P8).
   - Update `THREAT-MODEL.md:268` with the OpenAlex allowlist entry and rationale.
   - Commit and push Checkpoint 1.
2. **Checkpoint 2 — TheoremSearch (requires Owner Gate 2):**
   - Add `#[serde(default = ...)] pub theoremsearch_api_base: String` to `LiteratureEndpoints`, add the owner-verified host to `ALLOWLISTED_HTTPS_HOSTS` and `THREAT-MODEL.md`, add `Service::TheoremSearch` to `net_rate.rs`, and add `lit_theoremsearch.rs` (optional public service client only — NEVER vendor or copy TheoremSearch code, `explore.md` 11).
   - In `crates/desktop-server/src/lit_settings.rs`: strictly **off by default**; requires explicit versioned consent acknowledgement (same pattern as `antigravity.rs`) before `idea_search` sends any request; returns `status: "disabled"` whenever off or unacknowledged (never fails F12 coverage gate, Rule V4).
   - Commit and push Checkpoint 2.

## 5. Owned files

- `crates/research-tools/src/http_client.rs` (234 lines — allowlist & host-scoped auth header)
- `crates/research-tools/src/net_rate.rs` (447 lines — `Service::OpenAlex` / `Service::TheoremSearch`)
- `crates/research-tools/src/lib.rs` (300 lines — `LiteratureEndpoints` serde-defaulted fields)
- `crates/research-tools/src/lit_providers.rs` (from F11; split `lit_openalex.rs` / `lit_theoremsearch.rs` to keep every file `<= 600` lines)
- `crates/desktop-server/src/lit_settings.rs` (new, `<= 600` lines)
- `THREAT-MODEL.md` (allowlist note only, ~5–15 lines)
- Tests: `crates/research-tools/tests/optional_services.rs` (new, `<= 600` lines), `crates/desktop-server/tests/lit_settings.rs` (new, `<= 600` lines).

## 6. Shared registration seams and "Do not edit" list

- **Registration seams (1–3 lines each):**
  - `crates/desktop-server/src/lib.rs`: `pub mod lit_settings;`.
  - `crates/desktop-server/src/api_dispatch.rs` (from F5): settings/consent routes for optional literature services behind `AuthGuard::authorize_api`.
  - `crates/desktop-server/src/state.rs` (598 lines!): if any field is needed in `UiSettings`, first split `UiSettings` out of `state.rs` in a zero-behaviour-change commit, OR store `lit_settings.json` beside `ui_settings` from `lit_settings.rs` so `state.rs` is NOT touched!
- **Do not edit:** `tool_profile.rs`, `idea_tools.rs` schema (`def_hash` MUST NOT change!), `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`, or any F12 coverage gate logic (`disabled`/`not_configured` already handled by Rule V4).

## 7. Tests through the entry point

Run `cargo test -p research-tools --test optional_services --test tool_def_golden --test tool_profile_golden --test net_gate_and_fetch`, `cargo test -p desktop-server --test lit_settings`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (loopback mock + `FakeRateClock` only, zero live network, zero credentials in code/logs/ledger, zero third-party paper text):
- **Serde backward compatibility:** pre-X19 5-field `LiteratureEndpoints` JSON deserializes cleanly with default OpenAlex/TheoremSearch bases (`explore.md` 8 item 3).
- **Allowlist & redirect confinement:** newly allowlisted host succeeds; non-allowlisted host and cross-host redirects to non-allowlisted hosts are still refused; API key header is NEVER sent to another host (including on redirect, Rule P8); default `User-Agent` stays `re-search/<version>` with zero personal email unless opted in (Rule P8).
- **Rate gate:** `FakeRateClock` spaces `Service::OpenAlex` (and `Service::TheoremSearch`) through `NetRateGate` (Rule P6).
- **OpenAlex rebuilt-abstract honesty (Rule C3):** `abstract_inverted_index` reconstructs readable text with `abstract_ref.verbatim == false`, and `record_idea_triage` (X8) refuses to emit `evidence.abstract` from it!
- **TheoremSearch consent & off-by-default:** disabled by default (`status: "disabled"`, zero HTTP requests, F12 gate unaffected); enabling without consent acknowledgement is rejected; withdrawing consent immediately disables requests.
- **Frozen tool hashes (Rules S1, S2):** `tool_def_golden` and `tool_profile_golden` pass with both golden files byte-for-byte unchanged.

## 8. Document conflicts and resolutions

1. **OpenAlex in F11 vs X19:** `f11-lit-services.md` says "OpenAlex only if its host is already allowlisted in `http_client.rs`: do not widen the allowlist". Base `http_client.rs:14-20` does NOT allowlist `api.openalex.org`. **Resolution:** `explore.md` 9 row X19 and 13 decision 8 govern — OpenAlex stays out of F11 and lands only in X19 after explicit owner sign-off on the allowlist and `THREAT-MODEL.md`.
2. **`state.rs` line ceiling:** `explore.md` 9 row X19 says "key in settings, own consent text", which would push `state.rs` (598 lines) over 600 lines if placed inline. **Resolution:** follow `RULES.md` 600-line ceiling by storing optional-service settings and consent in new `lit_settings.rs` (and a sidecar settings file or pre-split `UiSettings`).

## 9. Out of scope and hand-back

- **Out of scope:** Copying/vendoring TheoremSearch code (`explore.md` 11), publisher full-text host allowlisting (stays closed, `explore.md` 13 decision 9), Docling/GROBID, changing `idea_search` schema or any `def_hash`.
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x19-optional-services.md` (recording exact owner approval reference, rate-limit assumptions, and `THREAT-MODEL.md` diff), commit with `Co-Authored-By`, and push to `gemini/modes/exp-x19-optional-services`.
