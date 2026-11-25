# Report: `f3-net-gate` (package F3 — network gate and paper fetch)

- **Base commit:** `49edc18dd66b85044046dacac186af3593a44de3` (`origin/claude/project-thread-caszr2`)
- **Work branch:** `work/f3-net-gate` (pushed to `gemini/modes/f3-net-gate`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` untouched; no tool definitions added or modified; `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`.

---

## 1. What was implemented (package F3)

### Minimal `fulltext` cache (`crates/ledger`)
- `crates/ledger/src/fulltext_cache.rs` (306 lines, new): `FulltextCache` opening `<data_dir>/library/library.db` (SQLite WAL, `busy_timeout(10s)`, `DEFENSIVE`, `trusted_schema = OFF`) with the strict `fulltext(paper_key, version, raw_blob, format, bytes, origin, fetched_ms)` table. Reads `<data_dir>/blobs/` first and falls back to `<data_dir>/library/blobs/`, verifying SHA-256 on every read and treating missing or hash-mismatched files as a cache miss. Provisional names `LIBRARY_DIR_NAME`, `LIBRARY_DB_NAME`, and `FULLTEXT_TABLE` are single named constants referencing `00-overview.md` section 8, decision 5.
- `crates/ledger/src/fulltext_cache_tests.rs` (158 lines, new): unit tests for `(paper_key, version)` round-trips, TeX-over-PDF preference on unversioned lookups, zero unreferenced blobs in `<data_dir>/blobs/`, corruption-as-miss handling, and entry removal.
- `crates/ledger/src/lib.rs` (506 lines, +4 lines): exports `pub mod fulltext_cache` and re-exports `FulltextCache`, `FulltextEntry`, `FULLTEXT_TABLE`, `LIBRARY_DB_NAME`, `LIBRARY_DIR_NAME`.

### Process-wide per-service network gate (`crates/research-tools`)
- `crates/research-tools/src/net_rate.rs` (426 lines, new): `NetRateGate`, `global_gate()` (`OnceLock`), `Service` (`Arxiv` 1/3s, `Inspire` 15/5s, `SemanticScholar` 1/1s, `Crossref` 1/1s), `RateBucket`, `resolve_bucket`, `RateClock` trait, `SystemRateClock`, and `FakeRateClock` (never sleeps in real time). Slot reservation runs under a short mutex lock; waiting (`clock.sleep`) happens outside the lock so calls to other hosts or staggered threads do not serialize.
- `crates/research-tools/src/net_rate_tests.rs` (165 lines, new): deterministic fake-clock tests for all four service limits, shared `export.arxiv.org` + `arxiv.org` bucket, loopback logical-service classification (both shared port and distinct ports), and 6 concurrent threads sharing one arXiv gate.

### Behaviour-only HTTP client and literature tool gating (`crates/research-tools`)
- `crates/research-tools/src/http_client.rs` (234 lines, +69 net): adds `HttpResponse`, `fetch_response_with_gate`, `fetch_bytes_with_gate`, and `fetch_bytes_for_service`; routes `fetch_bytes_allowlisted` through `global_gate()` on every outgoing hop (after allowlist validation succeeds, so rejected URLs consume no rate-limit slot).
- `crates/research-tools/src/arxiv.rs` (559 lines, +30 net): routes `fetch_atom_feed`, `ingest_eprints_direct`, and `ingest_eprints_locked` through `fetch_bytes_with_gate(..., Some(Service::Arxiv), gate)`, adds `run_arxiv_search_with_gate` for fake-clock tests, and makes `parse_atom_feed` `pub(crate)`. Tool definition `arxiv_search_def()` is untouched.
- `crates/research-tools/src/literature.rs` (452 lines, +19 net): routes `fetch_source` through `fetch_bytes_with_gate(..., Some(svc), gate)` (`Inspire`, `Crossref`, `SemanticScholar`), adds `run_literature_search_with_gate`, and makes `parse_hits` `pub(crate)`. Tool definition `literature_search_def()` is untouched.

### Shared paper fetcher, job progress wrapper, and session fetch counter (`crates/research-tools`)
- `crates/research-tools/src/paper_fetch.rs` (492 lines, new): documents the Overview 2.4 caller approval rule in the module doc comment (no approval path built); implements `fetch`, `fetch_with_gate`, `fetch_with_progress`, `FetchWant::{Metadata, Versions, Source}`, `FetchOutcome::{Metadata, Versions, Source, NeedsFile}`, `SourceOrigin::{Library, ArxivEprint, ArxivPdf}`, `FetchedSource::ingest_into_session` (delegating to `ingest::ingest_bytes`), `fetched_sources(events)`, and parameterized `check_fetch_budget(events, limit)`. Writes zero session ledger events itself.
- `crates/research-tools/src/paper_fetch_meta.rs` (508 lines, new): `PaperMetadata`, `PaperVersion`, `parse_paper_input` (using F2's `PaperKey`, `PaperRef`, `PaperIds`, `split_arxiv_id`, `canonical_doi` exclusively), DOI/INSPIRE/S2-to-arXiv resolution, arXiv Atom metadata & license extraction, single-request arXiv version list synthesis, and Crossref DOI metadata lookup.
- `crates/research-tools/src/paper_fetch_validate.rs` (173 lines, new): refuses empty bodies, oversized downloads (`> MAX_EPRINT_BYTES` / `> MAX_PDF_BYTES`), HTML error pages, corrupt gzip/tar, symlink/bomb archives (via `archive::extract_gzip_or_tar_gz`), and corrupt/encrypted/>400-page PDFs (via `catch_unwind` around `pdf_extract::Document::load_mem`) without panicking.
- `crates/research-tools/src/paper_fetch_job.rs` (199 lines, new): `FetchStage`, `FetchJobSnapshot`, and `FetchJob` (`start`, `start_with_gate`, `run_sync`, `snapshot`, `is_finished`, `join`) so later V4 routes can spawn and poll fetches on a thread.
- `crates/research-tools/src/paper_fetch_tests.rs` (140 lines, new): unit tests for `fetched_sources`, `check_fetch_budget`, `parse_paper_input`, version synthesis, and malformed download validation.
- `crates/research-tools/src/lib.rs` (275 lines, +5 lines): exposes `pub mod net_rate`, `pub mod paper_fetch`, and private submodules `paper_fetch_job`, `paper_fetch_meta`, `paper_fetch_validate`.

### Integration tests (`crates/research-tools`)
- `crates/research-tools/tests/net_gate_and_fetch.rs` (590 lines, new): 6 loopback mock tests covering fake-clock gate spacing across all 4 services, two callers (sequential and concurrent threads) sharing one arXiv gate, fetch order (`library -> e-print -> PDF`, plus raw-PDF e-print), zero-request cache hits with `ingest_bytes` session entry and `verify_all().unreferenced_blobs == 0`, DOI `needs_file` + DOI-to-arXiv resolution + metadata + versions, oversized/malformed/symlink download refusal without panic, and threaded `FetchJob` stage history.

---

## 2. Complete audit of outgoing HTTP call sites across all crates

| Location | Call / Purpose | Status |
|---|---|---|
| `crates/research-tools/src/http_client.rs:145` | `fetch_response_with_gate` (`ureq::Agent::get`) | **gated** — single choke point for all literature GETs (`fetch_bytes_with_gate`, `fetch_bytes_for_service`, `fetch_bytes_allowlisted`), gates every hop after allowlist check |
| `crates/research-tools/src/arxiv.rs:205` | `fetch_atom_feed` (`arxiv_search`, `run_arxiv_search`) | **gated** (`Service::Arxiv`) |
| `crates/research-tools/src/arxiv.rs:451` | `ingest_eprints_direct` (`run_arxiv_search` with `include_source`) | **gated** (`Service::Arxiv`) |
| `crates/research-tools/src/arxiv.rs:494` | `ingest_eprints_locked` (`arxiv_search` tool handler) | **gated** (`Service::Arxiv`) |
| `crates/research-tools/src/literature.rs:140` | `fetch_source` (`literature_search`, `run_literature_search`) | **gated** (`Service::Inspire`, `Service::Crossref`, `Service::SemanticScholar`) |
| `crates/research-tools/src/paper_fetch.rs:346,376` | `fetch_source_flow` (arXiv `/e-print/` and `/pdf/`) | **gated** (`Service::Arxiv`) |
| `crates/research-tools/src/paper_fetch_meta.rs:189,216,255,283,338,450` | DOI/INSPIRE/S2 resolution, arXiv metadata/versions, Crossref metadata | **gated** (`Service::Inspire`, `Service::SemanticScholar`, `Service::Arxiv`, `Service::Crossref`) |
| `crates/desktop-server/src/routes_tools.rs:393,419` | Desktop UI literature drawer (`handle_literature`) | **gated** — delegates to `run_arxiv_search` and `run_literature_search` |
| `crates/agent/src/http.rs:218` | `post_json` (Anthropic, OpenAI, Ollama, Gemini, ChatGPT Responses) | **not gated, because** model inference APIs use per-user provider accounts/quotas, not the scholarly literature hosts governed by 2.4 |
| `crates/agent/src/http.rs:266,270` | `get_json` (`model_list.rs` live model list refresh, 10-min cache) | **not gated, because** model provider / local Ollama endpoint, not a scholarly literature host |
| `crates/agent/src/stream_http.rs:178` | `post_sse_streaming` (streaming LLM inference) | **not gated, because** model inference endpoint, not a scholarly literature host |
| `crates/chatgpt-auth/src/oauth.rs:211,342,384` | OpenAI OAuth token/discovery/JWKS calls | **not gated, because** sign-in/auth endpoints, not scholarly literature hosts |
| `crates/mcp-server/src/approver.rs:26`, `crates/cli-route/src/relay.rs:64` | Raw `TcpStream::connect` loopback approval relay | **not gated, because** local loopback IPC between supervisor and `core.exe`, not external HTTP |

---

## 3. Decisions taken alone, design deviations, and assumptions

### Decisions
1. **Cache backend choice (Deliverable 5): SQLite in `crates/ledger/src/fulltext_cache.rs` over `<data_dir>/library/library.db`.**
   - *Reason:* `ledger` already depends on bundled `rusqlite` (zero new dependencies, zero `Cargo.lock` change); SQLite WAL + `busy_timeout` gives crash-safe and multi-thread/multi-process safe upserts on Windows without atomic-rename races on a flat JSON index; and `00-overview.md` 2.7 / `explore.md` 4.3 already designate `<data_dir>/library/library.db` with table `fulltext` as the database F10 will extend.
   - *Blob storage split:* `FulltextCache::get_bytes` checks `<data_dir>/blobs/` first (zero duplication once a paper has been ingested into any session) and falls back to `<data_dir>/library/blobs/`. `FulltextCache::put_bytes` writes only to `<data_dir>/library/blobs/` when the hash is not already in `<data_dir>/blobs/`, so fetching without immediately ingesting never leaves unreferenced blobs in the ledger's evidence directory (`Ledger::verify_all().unreferenced_blobs` stays 0).
2. **Logical service classification on loopback mocks (Deliverable 1).**
   - Real remote hosts always map by hostname (`export.arxiv.org` and `arxiv.org` share `Service::Arxiv` so Atom metadata and e-print/PDF downloads share the single 1-per-3s gate).
   - For loopback URLs (`127.0.0.1:<port>`): (a) explicit `Option<Service>` hint wins (passed by `arxiv.rs`, `literature.rs`, `paper_fetch.rs`, `paper_fetch_meta.rs`); (b) if only one logical service in `LiteratureEndpoints` maps to that port, that service is chosen; (c) if multiple services share one port (`LiteratureEndpoints::loopback_mock`), the URL path prefix classifies the service (`/api/query`, `/e-print/`, `/pdf/`, `/abs/`, `/html/` -> `Arxiv`; `/api/literature`, `/api/doi/` -> `Inspire`; `/works` -> `Crossref`; `/graph/v1/` -> `SemanticScholar`); (d) unrecognized loopback test paths (such as `/ok0`, `/hop0`, `/selftest/...` in allowlist/harness tests) map to `RateBucket::LoopbackGeneric(port)` (counted in the gate with no artificial wait).
3. **DOI-to-arXiv resolution order:** INSPIRE-HEP (`/api/literature?q=doi:<doi>&size=1`) first, then Semantic Scholar (`/graph/v1/paper/DOI:<doi>?fields=...`), using only existing allowlisted hosts. Clean 404/empty responses on both produce `FetchOutcome::NeedsFile`; if every resolver fails with a transport/5xx error, `Err` is returned rather than falsely reporting `needs_file`. When a DOI resolves to an arXiv paper, the downloaded source is indexed under both the resolved `arxiv:` key and the queried `doi:` key so repeated DOI lookups hit the cache with zero network requests.
4. **Versions lookup (`FetchWant::Versions`):** Uses a single arXiv Atom query (`id_list=<base_id>&max_results=1`) and synthesizes `v1..=vN` (capped at 200) from the latest version `vN`, filling `v1.date = published` and `vN.date = updated` (intermediate version dates are `None`).
   - *Reason:* Matches `investigate.md` Stage M's "one metadata request" budget, avoids fragile HTML scraping of `/abs/`, and works with any Atom mock.
   - *Assumption:* arXiv version numbers are contiguous `v1..=vN` without gaps.
5. **Fast PDF validation in `paper_fetch`:** Validates `%PDF-`, size `<= MAX_PDF_BYTES`, non-HTML, and `pdf_extract::Document::load_mem` (encrypted check + `1..=MAX_PDF_PAGES` page count) inside `catch_unwind` without running full per-page text extraction (`extract_pdf_bytes`), because `ingest_bytes` runs full extraction and doing it twice would double the 34–138 s wait on large PDFs.
6. **`fetched_sources(events)` definition:** Counts distinct `source.ingested` events whose `origin` starts with `https://`, `http://`, or `library:` (deduplicated by `source_id`, falling back to `raw_blob`). Local uploads (`origin: "upload"`, `workspace:...`, plain filenames) are excluded.
7. **Impact on existing loopback tests:** Because `arxiv_search` and `run_arxiv_search` now pass through the real process-wide 3-second arXiv gate even on `LiteratureEndpoints::loopback_mock`, `crates/research-tools/tests/literature_and_proof_dag.rs` now takes ~21 s (8 arXiv calls across 2 parallel tests = 7 * 3 s) instead of < 1 s. All existing tests still pass untouched.

### Deviations from the design
- `paper_fetch.rs` was split into four sibling files (`paper_fetch.rs`, `paper_fetch_meta.rs`, `paper_fetch_validate.rs`, `paper_fetch_job.rs`) plus `paper_fetch_tests.rs` so every file stays under the 600-line ceiling; all public items are re-exported from `research_tools::paper_fetch`.
- `FetchWant::Versions` does not fetch `arxiv.org/abs/<id>` HTML for intermediate version timestamps (see Decision 4 above). If V10 needs per-version dates for `v2..v(N-1)`, it can add an optional `/abs/` HTML parser in `paper_fetch_meta.rs`.

### Assumptions about external services
- arXiv Atom API responds to `GET /api/query?id_list=<id>&max_results=1` with Atom 1.0 XML carrying `<id>http(s)://arxiv.org/abs/<base>v<N></id>`, `<published>`, `<updated>`, and optionally `<arxiv:license>` or `<link rel="license" href="...">` (verified against existing `arxiv.rs` tests; `<link rel="license">` / `<arxiv:license>` handling is an assumption based on arXiv Atom/OAI conventions).
- arXiv `/e-print/<id>` may return `.tar.gz`, single-file `.gz`, uncompressed `.tex`, a raw `%PDF-` payload (when no TeX exists), or HTTP 404/403/410 (in which case `/pdf/<id>.pdf` is tried).
- INSPIRE-HEP `/api/literature?q=doi:<doi>&size=1` and `/api/literature?q=recid:<id>&size=1` return the same `hits.hits[].metadata` JSON shape already parsed by `crates/research-tools/src/literature.rs`; Semantic Scholar `/graph/v1/paper/DOI:<doi>` and `/graph/v1/paper/<paperId>` return single-paper JSON objects with `paperId` and `externalIds.ArXiv` (already handled by `parse_s2` in `literature.rs:342`).
- Crossref `GET /works/<doi>` returns `{ "status": "ok", "message": { ... } }` (already handled by `parse_crossref` in `literature.rs:249`).

---

## 4. Merge notes

### Edits to existing files
- `crates/ledger/src/lib.rs` (lines 17–23): added `pub mod fulltext_cache;` and `pub use fulltext_cache::{...};`.
- `crates/research-tools/src/lib.rs` (lines 16–20): added `pub mod net_rate;`, `pub mod paper_fetch;`, and private `mod paper_fetch_job;`, `mod paper_fetch_meta;`, `mod paper_fetch_validate;`.
- `crates/research-tools/src/http_client.rs`: added `HttpResponse`, `fetch_response_with_gate`, `fetch_bytes_with_gate`, `fetch_bytes_for_service`; `fetch_bytes_allowlisted` signature unchanged, now routes through `net_rate::global_gate()`.
- `crates/research-tools/src/arxiv.rs`: gated `fetch_atom_feed`, `ingest_eprints_direct`, `ingest_eprints_locked`; added `run_arxiv_search_with_gate`; made `parse_atom_feed` `pub(crate)`.
- `crates/research-tools/src/literature.rs`: gated `fetch_source`; added `run_literature_search_with_gate`; made `parse_hits` `pub(crate)`.

### Integration points for other briefs / later packages
- **V4 (`investigation_open.rs`, `POST /api/sessions/{id}/investigation/open`):** call `paper_fetch::parse_paper_input`, check `paper_fetch::check_fetch_budget(&events, DEFAULT_INVESTIGATE_FETCH_LIMIT)`, start `paper_fetch::FetchJob::start(job_id, env, paper, FetchWant::Source)`, poll `job.snapshot()`, and on `FetchOutcome::Source(s)` call `s.ingest_into_session(&env.sandbox(), &ledger, session_id, None)` (or `ingest::ingest_bytes`) on a worker thread.
- **X9 (`idea_tools.rs`, `idea_fetch_papers`) / X2 / F11 (`lit_providers.rs`):** call `http_client::fetch_response_with_gate` or `fetch_bytes_for_service` with `Service::{Arxiv, Inspire, Crossref, SemanticScholar}` so all F11 queries share `net_rate::global_gate()`, and call `paper_fetch::fetch` + `paper_fetch::check_fetch_budget(&events, preset.max_papers)` in X9.
- **F10 (`library.rs`, `library_schema.rs`):** opens the same `<data_dir>/library/library.db` (via `ledger::LIBRARY_DIR_NAME` / `ledger::LIBRARY_DB_NAME`) and extends it alongside the existing `fulltext` table created by `ledger::FulltextCache::open`.

---

## 5. Verification, test counts, and pre-existing findings

### Test and check commands run

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` | **Pass** — `No touched source file is over 600 lines.` |
| `git diff --stat origin/claude/project-thread-caszr2 -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt` | **Pass** — empty diff (both files unchanged) |
| `cargo test -p ledger` | **Pass** — 22 passed, 0 failed, 0 ignored (5 new `fulltext_cache` unit tests + 8 `hash_chain` + 9 `schema_rejects`) |
| `cargo test -p research-tools --test tool_def_golden` | **Pass** — 3 passed, 0 failed, 0 ignored |
| `cargo test -p research-tools --test net_gate_and_fetch` | **Pass** — 6 passed, 0 failed, 0 ignored (0.89 s, zero real sleeps) |
| `cargo test -p research-tools --test literature_and_proof_dag` | **Pass** — 7 passed, 0 failed, 0 ignored (21.27 s with process-wide 3 s arXiv gate active) |
| `cargo test -p research-tools --lib` | **Pass** — 61 passed, 0 failed, 0 ignored (includes 7 new `net_rate` + 4 new `paper_fetch` unit tests) |
| `cargo test --workspace --no-fail-fast` | **Pass** — **661 passed, 0 failed, 2 ignored** across 97 test/doc-test targets (the 2 ignored tests are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **Pass** — **8 passed, 0 failed, 0 ignored** (Python SymPy/NumPy/mpmath replay checks executed and passed) |
| `cargo clippy -p ledger --all-targets -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy -p research-tools --lib --test net_gate_and_fetch --test tool_def_golden -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy --workspace --all-targets -- -D warnings` | **Failed only on pre-existing warnings** (see proof below; 0 warnings in any added or modified file) |

### Pre-existing clippy warnings (proven on base commit `49edc18dd66b85044046dacac186af3593a44de3`)

Running `cargo clippy --workspace --all-targets -- --no-deps` on a clean detached worktree of base commit `49edc18dd66b85044046dacac186af3593a44de3` under `rustc 1.100.0 (0ef1bc386 2026-11-09)` produced the exact same 6 warnings (and zero others on our branch):

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also noted in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`, used only in `#[cfg(windows)]` tests).
3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
5. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
6. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.

### What could not be compiled or run on Linux
- `crates/desktop-app` Windows target dependencies (`wry`, `tao`, `windows` crate, WebView2 windowing, single-instance mutex) are `cfg(windows)`-gated and were not compiled on Linux.
- `cfg(windows)` code paths and tests in `crates/worker` (AppContainer/JobObject; `appcontainer_sandbox.rs` and `python_worker.rs` ran 0 tests on Linux), `crates/cli/tests/contained_router.rs` (0 tests on Linux), `crates/research-tools/src/sandbox.rs` (NTFS junction checks), `crates/research-tools/tests/sandbox_and_fs.rs` (Windows junction/symlink tests), `crates/chatgpt-auth`, and `crates/cli` were not compiled or run on Linux.
- No live external network requests were made to arXiv, INSPIRE-HEP, Semantic Scholar, or Crossref (prohibited by rules); all HTTP tests ran against loopback mock servers.

### Bugs / oddities noticed but not fixed
- `crates/cli/tests/desktop_and_ext.rs:149-168`: When `RE_SEARCH_PYTHON` is set globally in the environment on Linux, `cli_research_workspace_read_fork_diff_export_and_verify` spawns `re-search run --route api` without clearing `RE_SEARCH_PYTHON`. On Linux `re-search run --route api` refuses `--python` / `RE_SEARCH_PYTHON` and exits immediately before contacting `ScriptedMock`, after which line 168 calls `mock.finish()` before checking `out.status`, causing `listener.accept().unwrap()` in `ScriptedMock` to block indefinitely. Workaround used here: ran `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` and separately ran `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast`.
- `crates/research-tools/src/arxiv.rs:118-122` and `444-447`: `run_arxiv_search` creates an unused `LedgerDummy` mutex and `ingest_eprints_direct` discards its `&Ledger` parameter (`let _ = ledger;`) to open a fresh `Ledger::open(&env.data_dir)` connection inside the e-print loop. Left untouched to keep `arxiv.rs` edits strictly minimal and behaviour-only.
- `crates/desktop-server/src/evidence.rs:744` and the other 5 pre-existing Linux clippy warnings listed above (untouched files).

### Contradictions or gaps noticed in the design
- None that block F3. One minor note: `explore.md` 4.3 states "Full-text bytes are not copied: they are the ledger's content-addressed blobs," while `f3-net-gate.md` deliverable 3 requires that `paper_fetch` itself write no session ledger event (bytes enter a session only through `ingest_bytes`). Storing pre-ingest cache blobs in `<data_dir>/library/blobs/` while preferring `<data_dir>/blobs/` on read reconciles both requirements without creating unreferenced blobs in `Ledger::verify_all()`.
