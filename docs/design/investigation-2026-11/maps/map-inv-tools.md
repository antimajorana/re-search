# inv-tools: Investigation record tools, anchor verification, reader pointers

Scope read in full: `crates/research-tools/src/investigation.rs` (771 lines), `investigation_branch.rs`, `investigation_conventions.rs`, `investigation_summary.rs`, `lib.rs`, `fs_tools.rs`, `paper_passages.rs`, `pdf.rs`, `crates/research-tools/tests/investigation.rs`. Read in part: `crates/tools/src/registry.rs` (check_args, def_hash), `crates/tools/src/router.rs` (dispatch tail), `crates/research-tools/src/ingest.rs` (receipt writing), `crates/agent/src/resume.rs:191-234`, `crates/desktop-server/src/routes_tools.rs:143-193`, grep hits only in `crates/desktop-server`.

Not done / not verified: no cargo build or test was run (rule). Edge cases marked "port" were checked with a Python re-implementation of the Rust logic written by reading the code (`<local scratch file>`), not with the Rust code itself. I did not open the live session's ledger, so the actual cause of the three rejected record calls is NOT established; statements about likely causes are labelled inference. `crates/desktop-server/tests/investigation_browser.py` and the UI assets were not read beyond grep hits.

---

## 1. Exact schemas

All four tools: `effect = ReadWorkspace`, `approval_scope = Once` (`investigation.rs:86-87`, `investigation_summary.rs:23`, `investigation_conventions.rs:42`; branch clones the map def, `investigation_branch.rs:25`). Registered in `investigation.rs:91-123`, listed in `lib.rs:233-236`, wired in `lib.rs:255`.

Important context: the router's `check_args` validates only TOP-LEVEL properties (required, type, maxLength, closed set) (`crates/tools/src/registry.rs:194-233`, called at `crates/tools/src/router.rs:148`). Every nested constraint below is enforced by serde `deny_unknown_fields` plus hand-written checks, not by the schema.

### Shared anchor (`anchor_schema`, `investigation.rs:59-65`)
```
{ source_id: string 1..64, path: string 1..1024,
  start_line: integer >=1, end_line: integer >=1,
  excerpt: string 1..3200,
  kind: "passage"|"equation"|"figure"|"table"|"reference",
  label?: string 1..160 }
required: source_id, path, start_line, end_line, excerpt, kind;  additionalProperties: false
```
Runtime extras (`investigation.rs:471-482`): `source_id` is ASCII alnum/`-`/`_`; `path` must start with `<source_id>/`; `end_line >= start_line`; at most 64 lines; excerpt non-blank and <= 3200 chars; label non-blank <= 160. The Rust `Anchor` struct additionally accepts `file_sha256` and `source_seq` (`investigation.rs:169-172`), which are not in the schema.

### record_paper_summary (`investigation_summary.rs:9-25`)
```
{ source_id: string 1..64,
  title?: string 1..300,
  what: { text: string 1..700, anchors: [anchor] (1..3) }   required text, anchors
  why:  { text?, anchors?, unstated?: boolean }              (flat, no "required")
  how:  { text?, anchors?, unstated?: boolean } }
required: source_id, what, why, how;  additionalProperties: false
```
Runtime (`investigation_summary.rs:45-82`): each part is a strict union, either `{text, anchors}` or exactly `{unstated:true}`; `what` may not be unstated; every anchor's `source_id` must equal the summary's `source_id`. Maximum 9 anchors per call.

### record_paper_map (`investigation.rs:66-89`)
```
{ source_id: string 1..64, title: string 1..300, coverage: "paper"|"partial",
  answer_node_ids?: [string 1..48] (max 96),
  limitations: [string 1..500] (max 12),
  nodes: [node] (1..96), edges: [edge] (max 256) }
required: source_id, title, coverage, limitations, nodes, edges
node: { id: string 1..48, sentence: string 1..1400,
        kind: assumption|definition|method|equation|prediction|measurement|evidence|result|conclusion|reference,
        main: boolean, questions?: [string 1..360] (max 4), anchors: [anchor] (1..4) }
      required: id, sentence, kind, main, anchors
edge: { from, to: string 1..48, kind: depends_on|supports|cites, basis: explicit|inferred,
        reason: string 1..1000, anchor: anchor (exactly one) }   all six required
```
Runtime (`validate_map`, `investigation.rs:186-294`): `partial` needs >=1 limitation; at least one `main` non-reference node; unique node ids; each node needs an anchor in the map's source and only `reference` nodes may cite another source; edge anchor must be in the map's source; unique (from,to) pairs; `cites` only from a reference node to a non-reference node, and every reference node needs one; acyclic; `answer_node_ids` unique and existing. If the request has `overview_only:true`, every node must be `main` and non-reference and every edge `explicit` (`investigation.rs:689-697`). If the request has `summary_requested:true`, the summary must already be saved (`investigation.rs:657-662`).

### record_paper_branch (`investigation_branch.rs:24-39`)
```
{ request_seq: integer >=1, source_id, nodes: [node] (0..96), edges: [edge] (max 256), limitations (max 12 x 500) }
all five required; additionalProperties: false
```
Runtime (`investigation_branch.rs:110-194`): `request_seq` must be the active branch request; new nodes `main:false`; not all three lists empty; every new node/edge needs a directed path to the selected node; union with the saved graph validated at 1024 nodes / 4096 edges; only the NEW anchors are verified (`:183-190`).

### record_paper_conventions (`investigation_conventions.rs:27-44`)
```
{ request_seq: integer >=1, source_id: string 1..64, batch_index: integer >=1, done: boolean,
  coverage: "paper"|"partial", limitations: [string 1..500] (max 12), items: [item] (max 32) }
all seven required
item: { id: string 1..48,
        category: notation|terminology|units|normalization|sign|coordinates|indexing|ordering|orientation|algebra|geometry|reporting|other,
        label: string 1..160, meaning: string 1..1000, scope: string 1..450,
        basis: explicit|inferred, rationale?: string 1..600, anchors: [anchor] (1..4) }
      required: id, category, label, meaning, scope, basis, anchors
```
Runtime (`investigation_conventions.rs:95-193`): batches are sequential and 1-based; a not-done batch needs >=1 item and `coverage:"partial"`; a done batch that is partial or wholly empty needs limitations; ids unique across batches; (label, scope) unique across batches; `inferred` needs `rationale`; anchors must be in the selected paper. Worst case one batch = 32 items x 4 anchors x 3200 chars.

Result on success (all four): `{status:"recorded", source_id, request_seq, anchors_verified:true, <map_seq|summary_seq|branch_seq|conventions_seq>, ...counts}` (`investigation.rs:755-770`). The stored excerpts are NOT echoed back.

---

## 2. Anchor verification, step by step

Single choke point: `SourceIndex::verify(&mut self, anchor: &mut Anchor)` (`investigation.rs:467-523`), called from four loops.

Before any anchor is looked at (`record_document`, `investigation.rs:570-683`): the session ledger is hash-verified; the session must be in an active Investigation with a selected paper; the newest phase marker must be the expected request (`:600-619`); `source_id` must be the selected paper; a `SelectedPages` filter is built if the paper has `page_start`/`page_end` (`:675`).

Per anchor, in this order:
1. **Page filter** (`:468-470`, `SelectedPages::check` `:371-387`): for a primary-paper anchor, `path` must be the PDF extraction and BOTH `start_line` and `end_line` must lie inside a selected physical page's line range. Error: "The entire primary-paper anchor must fall within selected PDF pages A–B."
2. **Shape** (`:471-482`): one combined error for eight different conditions: "Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt."
3. **Load the file** (`load`, `:391-465`), once per path per call: the source must have an ingest receipt in this session with status `ok`/`pdf_only`; the assembled TeX view is refused; the path must be listed in the receipt; the file is read from the session `sources` sandbox (<=16 MiB, 32 MiB per call) and its SHA-256 must equal the receipt's `files_sha256[path]` ("Source file changed since ingestion: ..."). Only then is it decoded as UTF-8 and `\r\n` replaced by `\n` (`:442-444`).
4. **Locate lines** (`:445-453`, `:484-490`): `line_starts` = 0 plus the byte after each `\n` (a final trailing `\n` does not open a new line). The passage is exactly the bytes of lines `start_line..=end_line`. If `end_line` exceeds the line count: "Line N is outside <path>." Nothing outside the cited lines is ever searched.
5. **Label** (`:494-503`): if given, the label must be a literal substring of the passage. Error: "Copy the printed label exactly from <path> lines A–B or omit it."
6. **Excerpt** (`:491-514`): in the excerpt, `\r\n` becomes `\n` and ONE trailing `\n` is removed. Then two conditions must both hold:
   - the excerpt contains exactly `end_line - start_line` newline characters, i.e. it must touch every cited line (`:504`);
   - the excerpt is a byte-exact substring of the passage (with the passage's one trailing `\n` removed) (`:505-508`).
   Otherwise, verbatim error text: **`Excerpt does not match {path} lines {start}–{end} exactly. Read the lines and retry.`** (en dash; `:510-513`).
7. **Revision pin** (`:515-519`): if the caller supplied `file_sha256` or `source_seq`, they must equal the verified file's. Then both are written into the anchor (`:520-521`).

Mental model: the excerpt acts as a checksum that the model really looked at those lines. It may start mid-way through the first line and stop mid-way through the last line, but everything in between must be byte-identical, including every line break.

Normalisation applied: CRLF to LF on both sides; one trailing newline on both sides. Nothing else. Not applied: whitespace collapsing, trimming, tabs, NBSP, Unicode NFC/NFKC, ligatures, soft hyphens, de-hyphenation across line breaks, case, dash/quote variants. Port results: one sentence quoted for a 3-line range, reflowing two lines with a space, off-by-one line number, `ﬁ` vs `fi`, NBSP vs space, double vs single space, `renor-\nmalization` vs `renormalization` are all rejected with the same message; a 1-character excerpt is accepted.

**What happens to the whole call when one anchor fails.** Every call site uses `?`: map `investigation.rs:698-705`, summary `investigation_summary.rs:70`, branch `investigation_branch.rs:183-190`, conventions `investigation_conventions.rs:187`. The first failing anchor aborts the handler; the ledger append is only reached after all anchors pass (`investigation.rs:739-753`), so nothing is saved. The router records `tool.result {ok:false, error}` and returns the bare string (`crates/tools/src/router.rs:482-486`). Consequences:
- The model gets one error for one anchor; later anchors are never examined, so k bad anchors can cost k full round trips.
- The message names path and lines only, not the node/item/part or anchor index, and does not include the true text of the cited lines.
- No patch path exists: the model must resend the entire payload. After a success the record is frozen for that request: an identical resend returns the same seq, anything different gets "This record was already saved for this request. Start another mapping request to revise it." (`:739-741`).

---

## 3. What a reader already gets for free

### fs_read (`fs_tools.rs:53-76`, `218-302`)
Inputs: `path` (required), `root` (`workspace` default; the paper needs `sources`), `offset` (0-based Unicode scalar CHAR offset), `max_chars` (default 8000, hard cap 12000), `query` (<=512 chars), `regex`. There is NO line-addressed input.

Offset mode returns (`:288-301`): `root, path, file_sha256, content, offset, end_offset, returned_chars, total_chars, total_lines, start_line, end_line, truncated` (+ `tool_call_ref` added by the router, `router.rs:400-409`). `start_line`/`end_line` are the 1-based lines of the first and last returned character, for the chunk as a whole. Lines inside `content` are not numbered; chunks may start and end mid-line. To cite a passage in the middle of a 12,000-char chunk the model must count newlines itself.

Query mode (`:356-417`): per-line substring (case-sensitive) or bounded regex; returns `matches: [{line, text}]` (max 50) and `content` as `"<line>: <text>"`; `text` is the whole line if <=500 chars, otherwise a window with `...` added (`:419-437`), so not verbatim for long lines. `offset` is ignored in query mode (`:270-283`), so only the first 50 matching lines of the file are reachable. This is the only place fs_read gives exact per-line numbers.

fs_read does not check the file against the ingest receipt; it returns the SHA-256 of what it read. Whole file re-read and hashed per call (16 MiB cap).

### search_paper_passages (`paper_passages.rs:30-49`, `99-158`)
Inputs: `query` (<=512 chars, <=32 distinct terms), optional `source_id`, `limit` (default 5, max 10).

Each hit (`:61-82`): `source_id, title, path, source_sha256, file_sha256, source_seq, start_line, end_line, start_byte, end_byte, excerpt, matched_terms, score` (+ `tool_call_ref`). The file is hash-checked against the receipt before searching (`:289-293`). `excerpt` is a raw verbatim slice with original line endings; `start_byte/end_byte` are BYTE offsets in the on-disk file; lines are 1-based.

How passages are cut (`next_excerpt`, `:381-406`): the file is partitioned into consecutive non-overlapping windows of at most 8 lines and 1800 bytes, ending at the first blank line after content (the blank line is included); an over-long physical line is cut at whitespace. Ranking: 100 x distinct matched terms + per-term counts capped at 3 (`:332-339`). Terms are lower-cased words split on every non-alphanumeric character with a stopword list; exact word match only, no phrases, no stemming (`:408-459`).

Coverage block: `files_searched, bytes_searched, matching_passages, omissions{reason:count}, truncated`. Limits: 64 files, 2 MiB per file, 8 MiB total, 2 s scan (`:23-26`); only `.txt/.tex/.ltx/.md/.markdown`; the assembled TeX view is skipped. `truncated` is true whenever more passages matched than were returned (`:376`).

Bottom line: a search hit is already a record-ready anchor (`source_id, path, start_line, end_line, excerpt, file_sha256, source_seq`), missing only `kind`. The compact pointer `{source_id, path, file_sha256, start_line, end_line}` is already an established shape in the evidence API ("reader" object, asserted in `crates/desktop-server/tests/paper_passages.rs:139-146`). fs_read offset mode, which is what the orchestrator used ~30 times, gives the weakest pointer: chunk-level line numbers only.

A UI-only HTTP endpoint already reads sources by line range with 8 lines of context (`source_line_window`, `crates/desktop-server/src/routes_tools.rs:159-193`); it is not exposed as a model tool.

---

## 4. Caller passes only {path, start_line, end_line}; app fills the excerpt

Verdict: small change inside research-tools (one function), with two non-obvious costs outside it.

Why it is small: all four tools funnel through `SourceIndex::verify`, which already holds the hash-verified text of exactly the cited lines in `passage` (`investigation.rs:489-490`). Filling `anchor.excerpt` from `passage` when none was supplied is a few lines, and the stored payload keeps the same shape (still has `excerpt`, `file_sha256`, `source_seq`), so readers of saved records are unaffected: UI `assets/investigation_map.js:122,128`, `assets/investigation.js:197`, prompt compaction `crates/desktop-server/src/investigation.rs:357-363`. The idempotent-retry comparison (`investigation.rs:739-741`) still works because the derived text is deterministic.

Honesty: the guarantee "the stored excerpt exists at the cited lines of the hash-pinned file" becomes true by construction. It is arguably stronger than today, because today's substring rule accepts a fragment that drops context (a 1-char excerpt passes; quoting "conserved in this limit" from a line reading "not conserved in this limit" passes). What is LOST is the checksum role of the excerpt: a wrong line number would silently store real but irrelevant text under `anchors_verified:true`. Mitigations that fit the code: keep `excerpt` optional and verify it when present; accept the caller's `file_sha256` (already supported, `:515-519`); echo the derived text (bounded) in the tool result (`:755-770` currently returns counts only); store a marker such as `excerpt_basis`.

Touch list:
- `anchor_schema()` `investigation.rs:59-65`: drop `excerpt` from `required`. This one schema feeds all four tools (`:68`, `investigation_summary.rs:11`, `investigation_conventions.rs:31`, branch via `def()`).
- `Anchor` struct `investigation.rs:158-173`: `#[serde(default)]` on `excerpt` (and on any new marker field, so saved maps still deserialize in `investigation_branch.rs:156-159`).
- `verify` `investigation.rs:467-523`: allow empty excerpt at `:479`; fill after `:490`; decide the cap. 64 short PDF lines can exceed 3200 chars, and one TeX source line can be a whole paragraph, so line-only pointers need either a higher cap, an error asking for a narrower range, or the optional substring excerpt to narrow.
- Wording: `paper_map_instructions` `investigation.rs:50`; descriptions at `investigation.rs:79`, `investigation_summary.rs:20`, `investigation_conventions.rs:35`, `investigation_branch.rs:17,33`; combined error `investigation.rs:481`.
- **Tool-definition drift (outside research-tools).** `def_hash` covers description and input_schema (`crates/tools/src/registry.rs:26-35`). On every resume the recorded hashes must match (`crates/agent/src/resume.rs:191-216`, used by the desktop turn path at `crates/desktop-server/src/turn_runner.rs:399`). The only tolerated difference today is the map tool's `answer_node_ids` (`resume.rs:207-212`, pinned by the test at `resume.rs:224-234`). Changing the anchor schema or any description of the four tools therefore makes existing sessions fail with "tool definition drift" unless a compat rule is added for each. A brand-new tool id is only tolerated if added to the hard-coded list at `resume.rs:202`. I did not trace whether every follow-up desktop turn goes through this check; verify in `turn_runner.rs`.
- **File-length invariant.** `investigation.rs` is already 771 lines (limit stated as 600). The natural move is to extract `VerifiedFile`/`SourceIndex`/`SelectedPages`/`verify` (`:296-524`) into its own file using the existing `#[path]` module pattern (`:13-19`).
- Tests: `crates/research-tools/tests/investigation.rs` helper `anchor()` `:164-166` can stay. The fabricated-quote rejections stay valid only if a supplied excerpt is still checked strictly: `:70`, `:294`, `:423-425`, `:618-621` (asserts `contains("does not match")`). New tests needed: excerpt omitted and filled from the file, multi-line, CRLF, over-cap, selected pages, identical retry. Other fixtures that build anchors: `crates/research-tools/tests/ingest_archive_pdf.rs:173`, `crates/desktop-server/tests/investigation_flow.rs:126`.

Prerequisite for this to be reliable (inference from section 3): line-only pointers are only as good as the reader's line numbers, and fs_read offset mode does not number lines. A reader needs either search hits, fs_read query hits, or a line-addressed read (a model-facing version of `source_line_window`). Changing fs_read's own schema would trigger the same drift check for a tool present in every session.

---

## 5. Partial acceptance

Does not fit the code as written; it fits conventions well, the other three poorly.

Current structure working against it:
- Fail-fast `?` in all four loops; handler result is `Result<Value, String>`, so an error is one string (`router.rs:482-486`).
- One ledger event per record, appended only after full success (`investigation.rs:739-753`), and frozen per request (`:741`).
- Map and branch items are interdependent: dropping a node with a bad anchor can invalidate edges, `answer_node_ids`, the "one main claim" rule, reference/cites pairing and the branch path check (`validate_map` `:186-294`, `check_paths` `investigation_branch.rs:82-108`), all of which run BEFORE anchors are verified. Summary has three fixed parts and `what` is mandatory.

Where it fits: `record_paper_conventions`. Items are independent and batches are already append-only. Needed changes in `investigation_conventions.rs:157-189`: collect per-item failures instead of returning; store only accepted items; return `Ok` with something like `rejected: [{id, anchor_index, reason, cited_text}]` and the next `batch_index`; force `done=false` (or refuse to finalize) when anything was rejected; keep the "unfinished batch needs at least one convention" rule (`:146-150`) when everything was rejected. The identical-retry check still works if filtering is deterministic. Caller-side: anything that treats an `Ok` result as "all saved" must look at the new field (not traced). Test impact: `tests/investigation.rs:422-425` currently expects `is_err()` for a two-item batch with one bad excerpt; under partial acceptance that call would save batch 1 and break the assertions at `:429-430`.

For summary, map and branch, cheaper options that keep all-or-nothing commit and the honesty guarantee:
- Verify ALL anchors and return every failure in one error, each tagged with its owner (node id / item id / part name, anchor index) and the true text of the cited lines, bounded. `verify` only needs to return a richer error; the loops collect.
- Repair instead of reject: when a supplied excerpt fails the exact test but matches after whitespace/line-break collapsing (and optionally compatibility folding) inside the cited lines or a small window around them, store the app's raw slice and corrected line numbers with a marker. No Unicode-normalisation crate is a direct dependency of research-tools (`crates/research-tools/Cargo.toml`); `unicode-normalization` is already in `Cargo.lock` transitively.
- Anchor-level tolerance: nodes/items/parts allow several anchors; drop an invalid anchor when at least one valid one remains and report it. Structure is untouched. Limited value if the model sends one anchor each.
- Patch-retry (design inference): the router already stores every call's arguments in the ledger (`router.rs:141`, `tool.call` with `args`), so a retry could reference the rejected call and send only corrected pointers. This needs new plumbing and a new or changed tool schema (drift check applies).

---

## 6. Proven bugs and inconsistencies

Each is established by reading the cited lines; "port" means additionally reproduced in the Python port.

1. **Undocumented line-count rule.** The excerpt must contain exactly `end_line - start_line` newlines (`investigation.rs:504`). Neither the schema, the tool descriptions (`:79`) nor the instructions (`:50`) say so, and the error message does not distinguish it from a text mismatch (`:510-513`). Quoting one sentence out of a cited 3-line range is rejected although the text is there (port).
2. **"Read the lines and retry" has no matching tool.** The error (`:510-513`) tells the model to read lines, but fs_read has no line-addressed input (`fs_tools.rs:60-72`), offset mode returns only chunk-level `start_line`/`end_line` (`:288-301`), and query mode ignores `offset` and stops at 50 matches (`:270-283`, `:380`). A line-range reader exists only for the UI (`crates/desktop-server/src/routes_tools.rs:159-193`).
3. **Rejection gives no owner and no true text, one failure per call** (`investigation.rs:510-513`; `?` at `:700`, `:704`, `investigation_summary.rs:70`, `investigation_branch.rs:185,189`, `investigation_conventions.rs:187`). In conventions and summary, structural checks and anchor checks are interleaved per item, so a structural problem in a late item surfaces only after all earlier anchors pass (`investigation_conventions.rs:157-189`).
4. **Search hits end on the blank line and carry a trailing blank line.** A passage ending at a paragraph break includes the blank line and reports it as `end_line` (`paper_passages.rs:386-392`, `:351`). Copied verbatim it verifies; with the excerpt's trailing whitespace trimmed and the same line numbers it fails, because only one trailing newline is stripped (`investigation.rs:493`, `:504`) (port).
5. **Search hits at a page end fail when a page range is selected.** PDF page ranges exclude the blank separator line between pages (`pdf.rs:162-167`), but the search hit for the last paragraph of a page has `end_line` on that separator (item 4), and both ends must be inside a selected page (`investigation.rs:375-384`) (port). Search also has no notion of the selected page range and returns no page numbers (nothing page-related in `paper_passages.rs`).
6. **Incompatible offset units between the two reading tools.** Search returns byte offsets (`paper_passages.rs:352-353`); fs_read takes Unicode scalar character offsets (`fs_tools.rs:65`, `:317-318`). They diverge on any non-ASCII text, so a search hit cannot be turned into an fs_read window except by guessing.
7. **Search tokenisation cannot find section labels.** Splitting on every non-alphanumeric character (`paper_passages.rs:408-411`) turns "V.D.4" into the words `v`, `d`, `4`; there is no phrase match. (Usefulness for the user's actual question is inference; the tokenisation is fact.)
8. **One error for eight different shape faults**, and "short verbatim excerpt" while the limit is 3200 chars (`investigation.rs:471-482`).
9. **Summary swallows the parse error.** Any malformed field, including a bad anchor shape, yields "Each summary part needs text and anchors, or only unstated=true for why or how." (`investigation_summary.rs:46-48`); map and conventions pass the serde message through (`investigation.rs:687`, `investigation_conventions.rs:102`).
10. **Schema and struct disagree.** The anchor schema is closed (`investigation.rs:64`) but the struct accepts `file_sha256` and `source_seq` from the caller (`:169-172`); nested schema rules are not checked by the router at all (`crates/tools/src/registry.rs:194-233`). Not harmful (the values are checked at `:515-519`) and useful for revision-pinned pointers, but the published schema does not describe it.
11. **Size limits disagree.** Search skips files over 2 MiB (`paper_passages.rs:24`, `:274`); the recorder and fs_read accept 16 MiB (`investigation.rs:38`, `fs_tools.rs:22`); PDF extraction allows 8 MiB (`pdf.rs:9`). A large extraction is recordable but unsearchable.
12. **No LaTeX under-escape repair on anchors.** `repair_under_escaped_latex` is applied only in `proof_dag.rs:176-178,257`; record tools compare bytes as received. (That models under-escape backslashes in JSON is inference; it matters for TeX sources, not for the PDF run.)
13. **Test gap.** Every anchor in `crates/research-tools/tests/investigation.rs` is single-line (helper `:164-166`; also `ingest_archive_pdf.rs:173`, `desktop-server/tests/investigation_flow.rs:126`). The line-count rule, the trailing-newline strip, CRLF handling and partial first/last line matching have no direct test, and no test feeds a multi-line search hit into a record tool.
14. **File-length invariant already broken.** `crates/research-tools/src/investigation.rs` has 771 lines; `paper_passages.rs` has 605 including its tests. I found no script or doc in the repo that enforces or documents the 600-line rule (searched `docs`, `scripts`, `README.md`).
15. **Minor:** fs_read reports `total_lines: 0` for an empty file while the recorder treats it as one line (`fs_tools.rs:266` vs `investigation.rs:445`); an `offset` past the end of a file ending in a newline reports `start_line = total_lines + 1` (`fs_tools.rs:338-341`).

Likely contributors to the three live rejections (inference, not verified against the session): items 1, 2 and 4 together with the absence of any normalisation (section 2), since the orchestrator read by 12,000-char offset chunks and had to both count lines and reproduce PDF line breaks byte for byte.