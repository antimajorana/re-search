# Brief v1-skeleton: skeleton core for TeX (package V1 of Investigate)

Design: `investigate.md` section 4.1 (layer 1, the skeleton; read all of it), 4.2 for what later
packages will hash (tags are V2, not yours), the row V1 in section 9, section 6 for the rules that
name V1, section 12 for the risks, and section 10 of the evaluation text (`00-overview.md` 7.2 set P
and 7.3) for what will be measured. `00-overview.md` section 6, table "What can move": if the
fixtures show your own scanner failing, the project stops to decide on another parser. So an honest
account of where the scanner fails is a deliverable, not a defect of your report.

## Deliver

Files as in the design: `crates/research-tools/src/skeleton/{mod, model, flatten, tex_env,
tex_theorems}.rs`, `crates/research-tools/tests/fixtures/skeleton/*`,
`crates/research-tools/tests/skeleton_tex.rs`. More files are fine where one would pass 600 lines.

1. Types of the skeleton exactly as section 4.1 defines them (serde, stable field order). The
   `paper_key` field is `paper_key::PaperKey`. Leave tags unset or absent as the design allows for
   V1; do not call `node_tag`.
2. In-memory `\input` / `\include` flattening with an origin map (flattened line to file and line),
   reading from the already ingested files of a source. Read `tex_bundle.rs`, `tex_scan.rs`,
   `ingest.rs` and `investigation_outline_tex.rs` first and reuse what exists; say in the report
   what you reused and what you could not. No file access outside the ingested source, no cycle
   hangs, a depth limit.
3. A comment-safe scanner with an environment stack: sectioning commands; a `\newtheorem` /
   `\declaretheorem` table mapped to kinds (including shared counters and starred forms);
   theorem-like environments with optional title and heading citations; proofs (attached and
   detached, `proof of Theorem X` forms); labelled equations; figures; tables. Verbatim-like
   environments and `\iffalse ... \fi` are not scanned. Malformed TeX (unclosed environment,
   unbalanced braces) never panics and never loops: it ends with a recorded quality note.
4. Eight self-written TeX fixtures with expected JSON, as the row lists: amsthm shared counters,
   custom theorem names, multi-file, revtex with no theorem environments (a physics paper: sections
   and labelled equations only), detached proof, unlabelled theorems, two versions of one paper,
   plus one of your choice that you expect to be hard. **Write the mathematics and physics
   yourself**; short, plausible, not copied. The expected JSON is written from the fixture by
   reading it, not by running your scanner and saving the output: say in the report how each
   expected file was made.
5. A timing helper and one test that builds the skeleton of a generated 5000-line input and
   reports the time (assert only a generous bound, to stay stable in CI).
6. Property-style tests without a new dependency (a small deterministic generator is fine): the
   scanner terminates and does not panic on random interleavings of `\begin`, `\end`, `%`, braces
   and `\input`; every node span lies inside its parent's span; spans map back through the origin
   map to the right file and line.

## Not in this brief

Edges, tags, counters and reading statistics (V2); the mapview, routes and UI (V3); arXiv fetch
(V4); any ledger event, any tool, any model call.
