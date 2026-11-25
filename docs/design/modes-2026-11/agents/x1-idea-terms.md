# Brief x1-idea-terms: terms, queries, rank (package X1 of Explore)

Pure code, no I/O. Design: `explore.md`, row X1 of the package table and the sections it draws
on (term and identifier extraction from TeX-flavoured text, the first-query builder per service,
reciprocal-rank fusion, dedupe on `PaperKey` with alias merge). `00-overview.md` 2.3.

## Deliver

`crates/research-tools/src/idea_terms.rs`, `idea_queries.rs`, `idea_rank.rs`, with fixture tests.

1. TeX-aware term and id extraction from an idea text: arXiv ids and DOIs through F2's
   constructors, quoted phrases, capitalised names, TeX commands and math kept as tokens rather
   than split into noise. Deterministic and order-stable.
2. The first-query builder per service, producing exactly the query strings the user will be
   shown before anything is sent (overview 2.4, the approval rule). Query syntax of each service
   as the existing `arxiv.rs` and `literature.rs` build it; anything beyond that is marked as an
   assumption.
3. Reciprocal-rank fusion and dedupe on `PaperKey`, with alias merge: two hits for one paper,
   one keyed `arxiv:` and one keyed `doi:`, merge when either record carries the other's
   identifier. State the fusion constant and where it comes from in the design; if the design
   gives none, choose, mark it as your choice, and make it a named constant.
4. Property-style tests with a small deterministic generator: fusion is invariant under
   permutation of the service lists; merging is idempotent and order-independent; no hit is lost.

## Not in this brief

Any network call, any model call, coverage and verdicts, the Explore session and screen.
