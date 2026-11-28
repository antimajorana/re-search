# Brief f3-net-gate: network gate and paper fetch (package F3)

Design: `00-overview.md` section 2.4 and row F3 of 2.1; section 2.7 (last line: the minimal
`fulltext` table); `investigate.md` package V4 and section 5.2 for how the fetcher will be called;
`explore.md` packages X2 and X9 for the same.

## Deliver

1. `crates/research-tools/src/net_rate.rs`: one process-wide gate per host with an injectable
   clock. Limits from 2.4: arXiv 1 request per 3 s, INSPIRE-HEP 15 per 5 s, Semantic Scholar 1 per
   s, Crossref 1 per s. Hosts are taken from the request URL; with `LiteratureEndpoints` pointing
   at a loopback mock the gate still applies per logical service (decide how, and say so). The gate
   is shared by every caller in the process and safe under threads. Unit tests use a fake clock and
   never sleep in real time.
2. Behaviour-only edits in `arxiv.rs`, `literature.rs` and `http_client.rs` so that **every**
   outgoing request passes the gate, including the existing `arxiv_search` and `literature_search`
   handlers. Find every place a request leaves (`git grep` for the HTTP client's call sites in all
   crates) and list them in your report with "gated" or "not gated, because". No schema or
   description changes: the golden tool-definition test must pass unchanged.
3. `crates/research-tools/src/paper_fetch.rs`: `fetch(PaperRef, want: metadata | versions |
   source)`. Order: library blob, arXiv e-print (TeX), arXiv PDF. A DOI with no arXiv version
   returns `needs_file`. A fetched result enters a session only through the existing
   `ingest_bytes`, which writes that session's own `source.ingested`; the fetcher itself writes no
   session event. The design says `-> job`: deliver the synchronous core plus a small job type with
   progress states that a later route (V4) can run on a thread; do not add an HTTP route.
4. The session fetch counter: one function `fetched_sources(events)`; the limit is a parameter.
5. The minimal `fulltext` cache (2.7): cache by `PaperKey` and version under
   `<data_dir>/library/`. The `ledger` crate already depends on `rusqlite`; `research-tools` does
   not. Do not add `rusqlite` to `research-tools`. Either put the small table behind a function in
   the `ledger` crate, or use a content-addressed directory with an index file; choose, and give
   the reason. It is a rebuildable cache, never evidence.
6. Tests against a loopback mock server (see `crates/research-tools/tests/` and
   `LiteratureEndpoints::loopback_mock`): gate spacing with the fake clock, two callers sharing one
   arXiv gate (the case the design exists for), fetch order, cache hit makes no request,
   `needs_file`, an oversized or malformed download is refused without panic.

## Not in this brief

Routes, UI, the library beyond the minimal cache (F10), provider query runners (F11), any tool.
The approval rule at the end of 2.4 is a rule for the callers; write it as the doc comment of
`paper_fetch.rs` and do not build an approval path.
