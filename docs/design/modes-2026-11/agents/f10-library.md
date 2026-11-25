# Brief f10-library: the library (package F10)

One global, rebuildable index of papers for all modes. Investigate's "recent papers", the
fetch cache and the own-papers import (F13) sit on it.

Design: `00-overview.md` section 2.7 (all of it), row F10 of 2.1, 2.11 for what F13 will need
from you (read it so that your schema does not have to change next week; do not build F13),
2.14 for the same reason (F16), 8.2 row 5. `explore.md`: package X5 and the library sections it
points to (schema, tables, what is and is not evidence).

## Deliver

`crates/research-tools/src/library.rs`, `library_schema.rs`;
`crates/desktop-server/src/routes_library.rs` with routes under `/api/library/..`.

1. F3 already created `<data_dir>/library/library.db` with the table `fulltext` through
   `ledger::FulltextCache` (read `crates/ledger/src/fulltext_cache.rs` and
   `reports/f3-net-gate.md`). `research-tools` must not depend on `rusqlite`: put SQL behind
   functions in the `ledger` crate next to `fulltext_cache.rs`, in new files, and keep
   `research-tools/src/library.rs` as the typed API. Extend the same database; a database
   created by F3's code opens and upgrades in place (test it), and one created by you is still
   usable by `FulltextCache`.
2. The schema of X5 with the changes of 2.7: neutral names; `seen.role` gains `investigated`,
   `cited_lookup`, `solve_source`; the rebuildable table `skeleton(key, version, builder,
   blob_sha256)`. Keys are F2 `PaperKey`s. A schema version and a forward-only migration step.
3. It is an index over ledger blobs: rebuildable from the ledgers and the blobs, never evidence.
   Deliver `rebuild` and a test that deletes the database, rebuilds it and compares. User marks
   are the one thing that cannot be rebuilt: say where they live and how they are backed up
   (8.2 row 5), and build that.
4. Routes: read-only list, search by title, author, year and key, one paper's record, and the
   user-mark writes. Follow the style, authentication and error shape of the existing route
   files (`routes_tools.rs`, `routes_subagents.rs`); register them where the existing routes are
   registered. If that requires editing `http_srv.rs` (702 lines, over the limit, and not yours),
   do not: export one `handle_library(...)` function and name the one line a later change must
   add, under "Merge notes".
5. No network, no model call, no tool.

## Not in this brief

Sources and sync (F13), lessons (F16), the library screen, write-up (F15).
