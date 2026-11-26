# Brief lib-f13a-f16a-f15: library lane, packages F13a, F16a and F15 core, in order

Bring your own papers, corrections reused by code, and the write-up core. The library (F10) and
the PDF core (F14a) are in your checkout: read `reports/f10-library.md` and
`reports/f14a-pdf-core.md` first.

Design: `00-overview.md` sections 2.5, 2.11 (F13), 2.13 (F15), 2.14 (F16) in full, the rows F13,
F15, F16 of section 2.1, and evaluation sets B, W and K of section 7.2.

## Order and hand-back

F13a, then F16a, then F15 core. After each: run the checks of RULES.md, commit, and push at once
to `gemini/modes/lib-f13a-f16a-f15`. Between packages `git fetch origin` and
`git merge origin/gemini/work/fixed`.

## F13a: source model, sync, Kandinsky-vault adapter, folder adapter

- Files as the F13 row says: `paper_sources.rs`, `source_sync.rs`, `source_kandinsky.rs`,
  `source_folder.rs`, `source_dedupe.rs`, and `desktop-server/src/routes_sources.rs`. Not
  `source_bibtex.rs`, not `source_zotero.rs` (F13b), nothing that writes into the user's
  library (F13c).
- **Reading only.** No file in a user's library is created, changed, moved or deleted, ever. A
  property test opens each fixture library, runs scan and sync twice, and compares a hash of
  the whole tree before and after.
- **No way out of a listed folder.** Paths are resolved inside the registered root; symbolic
  links and `..` that leave it are refused. Tests for both.
- **Porting.** The overview's table under 2.11 names the Kandinsky files and the seven rules
  for a port. Try `git clone https://github.com/antimajorana/kandinsky` and check out
  `353368b`. If it cannot be cloned, write the code from the design instead, without the
  "Ported" header, and say so in the report. If you port: follow all seven rules, above all
  rule 2 (check each file's history for another author's notice first).
- **No new dependency.** If note properties need a YAML reader, write the small front-matter
  reader that rule 3 names (flat `key: value` pairs and simple lists are enough) and state in
  the report exactly what it does not read.
- Dedupe and moved-file detection register results through the F10 library API; do not add a
  second store.
- The metadata lookup by title over the network is **not built here** (it needs the approval
  card). Leave the hook and say so.
- Fixtures: a self-made vault and folder with the planted cases of evaluation set B (a moved
  file, an edited note, a deleted file, a duplicate by DOI, a duplicate by file hash). All
  fixture text is written by you; no third-party paper text or PDF.
- `routes_sources.rs` follows the pattern of `routes_library.rs`: one dispatcher function, and
  the single registration line in its header comment. Do not edit `http_srv.rs`,
  `routes_session.rs`, `turn_runner.rs`, `index.html` or `app.js`.

## F16a: corrections reused by code

As 2.14 says: `lessons.rs`, `lessons_schema.rs`, `lessons_select.rs`, store in
`<data_dir>/library/lessons.db`, `routes_lessons.rs` in the same one-dispatcher pattern. F16
adds no new way to correct: it reads existing events and library marks. Build the property test
2.14 names (same events with and without the lessons store: nothing a check decided changes;
only rankings and defaults the user can see). No model is involved anywhere in F16a.

## F15 core: write-up document model and writers

- `export/src/latex.rs` is 771 lines, over the limit. If you need to touch it, first move code
  out in a separate commit that changes no behaviour; if you do not need to touch it, leave it.
- `writeup.rs` (document model), `writeup_tex.rs`, `writeup_md.rs`, `writeup_bib.rs`. No mode
  adapter here (`writeup_solve.rs`, `writeup_idea.rs`, `writeup_paper.rs` come with the modes).
- The model carries, for every statement, how it was checked, and both writers print it. The
  wording comes from one table. For Solve statements use the tier labels of
  `solve_badge.rs` (in your checkout) and never a stronger word: no "proved", no "verified"
  for anything below the Verified tier. A test walks every tier and every writer.
- Quotes go through `quote_core` (F4); a quote that does not match its source is not written.
- Golden output files for one hand-built document per writer; the TeX output must be valid
  input for the existing `tex_audit` checks.

## What must hold

- No tool is added and no tool definition changes (both golden tests pass untouched).
- `Cargo.lock` unchanged. No network call and no model call in tests.
