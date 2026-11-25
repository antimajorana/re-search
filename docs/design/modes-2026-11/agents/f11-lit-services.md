# Brief f11-lit-services: literature services (package F11)

Design: `00-overview.md` section 2.8, row F11 of 2.1, 2.4 (the gate you must go through).
`explore.md`: package X2 (now F3 and F11) and the provider sections it points to (services,
fields, abstracts, citation relations, raw responses as blobs, `literature.fetched`).

## Deliver

`crates/research-tools/src/lit_providers.rs`, `lit_abstracts.rs` (more files where one would
pass 600 lines).

1. One query runner per service (arXiv, INSPIRE-HEP, Semantic Scholar, Crossref; OpenAlex only if
   the design names it and its host is already allowlisted in `http_client.rs`: do not widen the
   allowlist, say so instead) with abstracts and citation relations, results keyed by F2
   `PaperKey` / `PaperRef`.
2. **Every request goes through F3's gate** (`http_client::fetch_response_with_gate` or
   `fetch_bytes_for_service`). No second HTTP path, no second limiter.
3. The raw response is stored as a blob; `literature.fetched` gains optional `qid` and `run_id`.
   Existing readers of `literature.fetched` keep working (find them with `git grep` and test).
4. The existing `arxiv_search` and `literature_search` tools keep their definitions byte for byte
   (golden test) and their behaviour.
5. Tests against a loopback mock with responses **you write by hand** in the shape of each
   service (short, invented papers; no real titles or abstracts): parsing, missing fields,
   malformed JSON and XML, paging, a 429 with and without `Retry-After`, a 5xx, a timeout. Every
   field you rely on that you did not see parsed in this repository is listed as an assumption.

## Not in this brief

Terms, queries and ranking (X1), coverage and verdicts (F12), the library (F10), any tool or
route.
