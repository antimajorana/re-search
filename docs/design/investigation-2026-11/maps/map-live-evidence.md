# Live evidence from real Investigation runs (ledger copy, read-only)

Scripts and outputs are in `<local scratch file>` (`rejects.py`, `rules.py`, `turns.py`, `phases.py`, `reads.py`, `ctxgrow.py`, `payload.py`, `final2.py`, `mathsurvey2.py`, `deleg2.py`, `cli.py`, `ingest.py`, `prompt.py`). Nothing in either repo was touched; no cargo, browser or network.

Sessions:
- `s-086e8a2bc04ad89c` ("slow"): gpt-6.2-sol via ChatGPT sign-in, question "Help me understand section V.D.4".
- `s-ddc0ea95f6aec6a9` ("ddc"): same PDF, routed through the Claude Code CLI (`claude-sonnet-5-5`), question "Identify the paper's main results."
- `s-ff4675c9635d152d`: a single `investigation.started` event and nothing else.

The ddc sources folder has been deleted, so its anchors were re-checked against the slow session's extracted file. That file's sha256 (`8336a32e…`) matches both sessions' `source.ingested` records.

## 1. Rejected `record_paper_*` calls

No rejection was a paraphrase, a whitespace difference, a ligature or a Unicode look-alike. Fourteen of the 15 failing anchors were byte-exact quotes that occur exactly once in the file; the model's line range was wrong or loose. The fifteenth was a one-character slip.

The verifier (`crates/research-tools/src/investigation.rs:467-523`) applies three rules:
- The excerpt must have exactly as many lines as the cited range (`:504`).
- The excerpt must be a substring of those lines (`:505-508`).
- Any `label` must occur inside the cited lines (`:494-503`).

It stops at the first failing anchor and rejects the whole call. I re-implemented it and checked every anchor of every rejected call.

| # | Session / seq (turn) | Tool | Failing anchors | Range the app reported |
|---|---|---|---|---|
| 1 | slow / 41 (t3) | `record_paper_summary` | 4 of 4 | 32–40 |
| 2 | slow / 162 (t9) | `record_paper_map` | 1 of 13 | 3090–3097 |
| 3 | slow / 287 (t16) | `record_paper_conventions` b1 | 2 of 19 | 3454–3469 |
| 4 | ddc / 74 | `record_paper_summary` | n/a | CLI `InputValidationError` |
| 5 | ddc / 77 | `record_paper_summary` | 3 of 3 | label error, 36–43 |
| 6 | ddc / 83 | `record_paper_summary` | 3 of 3 | 36–43 |
| 7 | ddc / 194 | `record_paper_conventions` | 2 of 5 | 1137–1144 |

Character-level diagnosis (`⏎` = newline):

- **#1 `what/0`, cited 32–40.** The excerpt is a byte-exact copy of lines 28–32 ("We consider two-dimensional (2d) quantum…" to "…order parameters in such systems."). 388 of its 445 characters lie before cited line 32. Class: shifted range (+4 lines).
- **#1 `how/0`, cited 36–43.** Byte-exact copy of lines 32–35, starting mid-line at "We show that the universal properties". Zero overlap with the cited lines. Class: shifted range.
- **#1 `how/1`, cited 2876–2883.** Byte-exact lines 2878–2880, starting mid-line at "Below, we demonstrate the fusion rule (52)". The range was copied from an 8-line search passage (seq 33); the quote is a sub-span. Class: range wider than quote.
- **#1 `why/0`, cited 186–200.** Byte-exact lines 195–200, from mid-line "The lack of a general theory". Class: range wider than quote.
- **#2 `dislocation-fusion/1`, cited 3090–3097.** Range and text came from a search passage. True text: `…(or plaquette), Therefore after mov-⏎ing the 4 dislocations into…`. Model: `…Therefore after mov-⏎ning the 4 dislocations into…`. One inserted `n` at offset 60, right after a hyphenated line break (462 vs 461 chars).
- **#3 `abelian-fonts/0`, cited 3454–3469.** Byte-exact lines 3454–3468 (15 lines, 710 chars). The last cited line, the fragment `Z2 = Z h` of a formula garbled across lines 3469–3470, was left out. Class: trailing line dropped.
- **#3 `cocycle-table-axes/0`, cited 4012–4013, `kind=table`, `label="TABLE A2."`.** The excerpt is identical to the cited lines. The label is printed on line 4011, one line above. The app never reported this one; it would have caused a second rejection.
- **#4.** Not an anchor problem. The CLI model emitted `"what": The paper builds…` with unquoted string values.
- **#5.** Labels "Abstract" and "Introduction" are section names, not printed labels. "Abstract" appears nowhere in the extracted text; "Introduction" only on line 47. The same anchors also had the #6 problem.
- **#6.** Ranges 36–43, 260–267 and 194–201 are 8-line search passages. The excerpts are byte-exact sub-quotes at 36–38, 264–266 and 195–199. Class: range wider than quote.
- **#7 `cs-action/0` (1137–1144) and `cs-skyrmion-U1/0` (3215–3222).** Both ranges are search passages whose last line is blank. The search excerpt ends `…exact sequence⏎⏎`; the model's copy ends `…exact sequence`. The whole difference is one trailing `⏎`.

Where the bad line numbers came from: `fs_read` in offset mode returns up to 12,000 characters with only the chunk's `start_line`/`end_line` (for example "lines 1–204", seq 16) and no per-line numbers, so the model counts lines itself. Only `fs_read` query mode (`N: text`) and `search_paper_passages` give usable line numbers. The model spent four turns (13, 14, 17, 19; about 49 s) on regex greps, including one that tried to match line-number prefixes (`^(34[5-9][0-9]):…`, seq 253).

Classes by primary cause (15 failing anchor submissions):

| Class | Count | Where |
|---|---|---|
| Range wider than an exact quote | 5, plus 3 hidden behind label errors | #1 ×2, #6 ×3, #5 ×3 |
| Trailing blank or garbled last line dropped | 3 | #3, #7 ×2 |
| Shifted line numbers, exact quote elsewhere | 2 | #1 ×2 |
| One-character slip at a hyphen break | 1 | #2 |
| Label not inside cited lines, excerpt fine | 1 | #3 |
| Whitespace-only, hyphenation-only, ligature, Unicode look-alike, paraphrase | 0 | none |

What each candidate rule would have accepted (`rules.py`):

| Rule | Accepts | Caveat |
|---|---|---|
| Whitespace / NFKC / de-hyphenation normalisation alone | 0 of 15 | "movning" is still not "moving" |
| Exact quote may be any substring of the cited lines (drop line-count equality) | 8 | 12 if the label rule also looks a few lines above and ignores section names |
| App locates the unique exact quote and rewrites the range | 14 | all but #2; the 4 label failures still need the label change |
| Model sends only a range; app fills the excerpt | 15 syntactically | in the 2 shifted cases it stores different text than the model read (0 and 1 lines of overlap) |
| Bounded edit distance, then store the app's own text | the #2 slip | only safe if the stored excerpt is the source text |

App-filled excerpts alone are therefore honest (the text exists at the cited lines) but can silently cite the wrong passage. A short verbatim probe that the app must find in, or use to re-locate, the cited lines covers 14 of 15. Reporting all failing anchors at once, and accepting the valid items, would also have removed the hidden second rejection in #3.

Side effects of rejection, seen in the retries:
- The model made evidence thinner instead of fixing it. In #2 the 8-line anchor became the single line 3092.
- In #3 cited lines fell from 67 to 42, every item was regenerated and several ids were renamed.
- In ddc #7 the item `cs-action` was dropped (5 items became 4).
- Rejected arguments stay in context for all later turns (30,492 chars in the slow session), because masking only touches tool outputs (`crates/agent/src/guardrails.rs:108-172`).

## 2. Time, tokens and reading

### Slow session (wall clock from `investigation.started`)

| Phase | Window (s) | Wall | Breakdown (model seconds) |
|---|---|---|---|
| PDF ingest | 0–137.7 | 137.7 | no model call |
| Summary (turns 1–5) | 138.2–195.2 | 57.0 | read 17.9; rejected record 17.2; accepted 21.5 |
| Map (turns 6–11) | 195.2–353.5 | 158.3 | read/notes 38.1; rejected 60.8; accepted 58.6 |
| Conventions (turns 12–20) | 353.5–671.4 | 317.9 | read/grep 68.2; rejected b1 83.8; accepted b1 79.5; b2 85.4 |
| Answer (turns 21–41) | 671.4–852.5 | 181.1 | 11 re-read turns 59.5; 9 `ledger_append` turns 98.6; final text 21.3 |

- Tool execution totalled 2.1 s and app overhead between turns 3.6 s. `session.end` reports 714.3 s.
- Record calls: 406.8 s (57% of model time) and 19,525 of 30,247 output tokens. They run at a steady 44–49 output tokens/s, so their duration is proportional to output size. Reasoning tokens were only 2,054 in total.
- Rejections: 161.8 s of generation plus 24.1 s of recovery turns, 26% of the session, and 7,769 wasted output tokens.
- Record payload composition (75,721 argument chars over 7 calls): 28% verbatim excerpts, 22% the 178-character source path repeated in every anchor, 14% other anchor JSON, 34% actual prose and ids.
- Input: 2,092,538 tokens, of which 89,600 were cached (4.3%); 29 of 41 turns had zero cached tokens. The request body carries no cache key (top-level keys are `input, instructions, model, reasoning, store, stream, tools`), `prompt_cache_key` appears nowhere under `crates/`, and each response reports a different key. Inference: the provider assigns a fresh key per request, so the prefix rarely hits.
- Every turn re-sends a 27,768-character schema for 27 tools (including `latex_patch`, `latex_compile`, `sympy_eval`, `fs_write` and the `delegate_*` tools) plus 5,383 characters of instructions.

Per-turn tokens:

```
turn start_s dur_s  in_tok cached out_tok  calls
  1  138.2   5.5    6775     0    149  title, fs_read, search
  2  143.7   6.1   10349     0    205  fs_read x2, search
  3  149.9  17.2   18910     0    754  record_paper_summary  REJECTED
  4  167.2   6.3   19734     0    222  search x3, fs_read
  5  173.6  21.5   27244     0    977  record_paper_summary  ok
  6  195.2  10.2   28269     0    389  search x4, ledger_append
  7  205.6   6.2   32752     0    229  search x4, fs_read
  8  212.0  12.9   39698     0    504  fs_read x3, search, ledger_append
  9  225.0  60.8   51816     0   2952  record_paper_map  REJECTED
 10  286.0   8.8   54843     0    337  fs_read x4
 11  294.8  58.6   68681     0   2814  record_paper_map  ok
 12  353.5   9.5   71555     0    360  fs_read x5
 13  363.1  19.2   91852     0    752  fs_read x3, ledger_append
 14  382.5  10.3   97442     0    280  fs_read (grep)
 15  392.8   9.5   59178     0    360  fs_read x5   (old outputs masked)
 16  402.4  83.8   78265  4608   4063  record_paper_conventions b1  REJECTED
 17  486.3   9.0   82332     0    300  fs_read (grep)
 18  495.3  79.5   83796     0   3811  record_paper_conventions b1  ok
 19  575.0  10.8   87646     0    282  fs_read (grep)
 20  585.8  85.4   89683  4608   4154  record_paper_conventions b2  ok
 21  671.4  12.1   93902     0    485  ledger_append
 22  683.6   5.4   94229     0    157  fs_read
 23  689.0   3.5   37050  4608     72  fs_read      (masked)
 24  692.6   4.3   39381     0     72  fs_read
 25  696.9   6.1   41242  4608    179  fs_read, search
 26  703.2  16.1   43172  4608    697  ledger_append x3
 27  719.4   9.6   43931     0    337  ledger_append
 28  729.0   5.6   44219  4608    176  search
 29  734.7   9.1   44996     0    346  ledger_append
 30  743.9   8.5   45264     0    307  ledger_append
 31  752.5   4.7   37438  4608    114  fs_read      (masked)
 32  757.3   4.4   38411  4608    109  fs_read
 33  761.7   6.2   39853     0    153  fs_read
 34  767.9   9.6   41324     0    343  ledger_append
 35  777.5   4.5   41561  4608    101  fs_read
 36  782.0   8.8   42882     0    305  fs_read, search
 37  791.0   9.2   44448     0    316  ledger_append
 38  800.3   9.0   44694     0    318  ledger_append
 39  809.4   6.0   40360  4608    190  ledger_query, fs_read, search  (masked)
 40  815.7  15.4   46364  4608    643  ledger_append (final_answer entry)
 41  831.1  21.3   46997 38912    933  final text (3,190 chars)
```

Reading in the slow session (file is 176,921 chars, 4,922 lines):
- 31 offset-mode `fs_read` calls returned 254,171 chars. Unique coverage is 171,021 chars (96.7% of the paper); 83,150 chars (32.7%) were re-reads.
- 7 grep-mode calls returned 11,091 chars (185 matched lines).
- 18 searches returned 54 passages: 57,340 JSON chars, 20,929 excerpt chars, 15,786 unique.
- Section V.D.4 is lines 2687–3105 (9,863 chars, 5.6% of the file). It was read 12 times, 42,202 in-region chars, about 4.3 times over: once before the summary, three times before conventions and eight times in the answer phase.
- Old tool outputs are masked in strides at turns 15, 23, 31 and 39. At turn 23 tool-output context falls from 188,774 to 19,288 chars. Inference: this is why turn 15 re-read offsets 105,700–153,000 (47,900 chars) and why the whole answer phase (35,250 chars) is re-reading.
- Function-call arguments are never masked. They grow to 101,675 chars by turn 41, of which 75.7k are record payloads.

### ddc session
- Ingest took 34.1 s, CLI start 5.9 s, and the model phase 148.0 s over 32 turns. Total wall time was 188.6 s.
- Per-turn tokens are not recorded; the CLI reports totals only: 40 input tokens, 77,447 cache-creation, 920,451 cache-read, 21,934 output (7,075 thinking), cost $0.62. Caching worked on this route.
- Phase timeline (seconds): reading 42.0–59.3; three failed summary attempts at 59.3, 65.2 and 71.0, accepted at 84.8; conventions rejected at 146.0 after a 40.8 s generation gap, accepted at 162.8; map accepted first time at 180.4; final text at 188.1.
- About 42 s of 148 s went on rejections.
- Reading: 7 offset reads returned 34,100 chars (33,200 unique, 18.8% of the file, 900 re-read), plus 2 greps and 14 searches (45 passages). Combined coverage was 23.7%.
- Output: 4 convention items and 5 nodes, against 29 and 6 in the slow session.

### s-ff4675c9635d152d
No ingest, no model call.

## 3. How much recorded output served the question

The question was appended as the last line of a 4,059-character user message that opens with "Identify the paper's central results…" and demands a "comprehensive" conventions catalog across sections, appendices, tables and footnotes (`prompt.py`). `investigation.mapping_requested` has `overview_only: true`, yet that instruction drove a 96.7% read.

- **Summary:** `what` and `why` are whole-paper statements. `how` has one sentence on V.D.4. One of 4 anchors lies in the section.
- **Map:** 6 nodes; `answer_node_ids = ["dislocation-fusion"]`. That node is 2,102 of 12,309 stored chars (17%) and holds the 3 of 13 anchors that lie in the section. The other five nodes are generic results (residual symmetry, SPT/DQCP, anyon permutation, defect fractionalization, skyrmion data).
- **Conventions:** 29 items; 6 are anchored inside V.D.4 (`burgers`, `link-paulis`, `fusion-order`, `wilson`, `lattice-moves`, `merge`), 6,891 of 33,654 stored chars (20.5%). The other 23 include Q8 cocycle-table axes, spectral-sequence pages, the spinor-BEC table, WZW parameters and skyrmion flux normalisation. Three or four general items (coset, group fonts, action notation) are arguably useful background; that is my judgement.
- **Cost:** 533 s of the 714 s session and all 19,525 record output tokens preceded any answer; roughly one fifth of that output concerns the section.
- **Use by the answer:** the final answer cites only ledger entries (`[e-5]`…`[e-14]`). It names the map node once in a closing status paragraph and never references a convention id. It was built from answer-phase re-reads plus 14 proof entries (10,457 chars of details, written under the Research-mode "proof DAG" discipline).

In ddc the question was the default "main results" one, so all 5 nodes are answer nodes and the whole-paper work was on topic.

## 4. Final answer text of the slow session

It is ledger event seq 526 (`model.text`) and is byte-identical to the `message` item in the last `model.response` blob (turn 41).

- **Size:** 3,190 chars, 478 words, 8 blocks. About 2,070 chars explain V.D.4; a 515-char paragraph places it in the paper; a 494-char paragraph reports what was saved.
- **Markdown:** two `###` headings, 6 bold spans (one contains inline math: `**background \(m\) per unit cell**`), 3 `*italic*` spans, 1 inline code span, 13 `[e-N]` citation tags. No lists, tables, links or code fences.
- **Math:** 24 inline `\( … \)` spans and nothing else. There is no `\[ \]`, `$`, `$$` or `\begin{…}`.
  - Bodies are 1–35 chars with no newlines; 14 are single symbols (`m` ×5, `e` ×2, `1`, `x`, `y`, `a`, `Z`, `+1`, `-1`).
  - The longest are `A_p=\prod_{\ell\in\partial p}Z_\ell` and `a=\omega(T_x,T_y)/\omega(T_y,T_x)`.
  - Control words: `\Delta` ×6, `\hat` ×4 (unbraced, `\hat x`), `\ell` ×2, `\omega` ×2, `\prod`, `\in`, `\partial`. There are 14 unbraced subscripts and one braced.
  - Closing delimiters are followed by a space (17), `.` (4), `,` (2) or `’` (1). One opening delimiter follows a hyphen (`background-\(m\)`).
- **Unicode math outside LaTeX:** the answer has none, but the recorded map, conventions and proof entries write math as plain Unicode (`Δ`, `π`, `₂`, `−`, `ω`, `∂`, `x̂` with a combining hat) and contain no LaTeX delimiters.

The renderer must also cope with what the same model emitted in Research sessions (7 answer texts, `mathsurvey2.py`):

| Form | Count | Notes |
|---|---|---|
| `\( \)` inline | 477 | never contains a newline |
| `\[ \]` display | 79 | delimiters always alone on their own lines; 35 bodies multi-line |
| `$$ $$` display | 17 | 11 multi-line; all in one text |
| `$ $` inline | 87 | same text, which uses dollars exclusively |
| Environments, inside display math only | 3 | `aligned` ×2, `cases` ×1 |
| Bodies with `\tag` | 8 | |
| Math inside table rows | 4 | |
| Math inside list items / a heading | 2 / 1 | |

A compute worker's returned summary was a Markdown table with `\(\tfrac{21}{4}\)` in its cells. Common control words include `\mathcal`, `\mathbb`, `\langle`/`\rangle`, `\widehat`, `\frac`/`\tfrac`, `\qquad`, `\rm`, `\operatorname`, `\boxed`, `\bigl`/`\bigr`, `\left`/`\right`, `\text`, `\lvert`/`\rvert`.

## 5. Research sessions that delegated

Every worker ran on the same model (`routing_reason: isolated_primary_model`).

| Session | Worker | Turns | Duration | Tokens in / out | Returned to orchestrator |
|---|---|---|---|---|---|
| s-182034b655e9335e | literature | 8 | 58.6 s | 157,420 / 1,395 | failed: "worker reached max_turns (8)", 538 chars of JSON, no findings |
| s-e9826031a37391eb | literature | 8 | 53.5 s | 81,666 / 558 | same failure, 473 chars |
| s-ea1c994eebdc142e | literature | 8 | 126.8 s | 59,220 / 3,513 | same failure, 526 chars |
| s-675c3d0b0f175c00 | compute | 3 | 47.3 s | 4,586 / 999 | ok: 727-char summary, one proof entry (`e-5`) |

- All three literature workers were still calling tools on turn 8. `crates/agent/src/delegate.rs:449` ends the run as an error with only that turn's text, so everything they read was discarded. There is no forced wrap-up turn.
- Worker turns were also uncached (cached tokens 0, 7,936 and 0).
- `delegate_explore` ran 11 times: 4.8–128.9 s, 2–11 turns, up to 9,563 output tokens per run. The orchestrator received only 1.9–3.5k chars each (9,108 output tokens became 3,124 chars in one case).
- The summary clip constant is `DELEGATE_RETURN_SUMMARY_CHARS = 2_800` (`crates/agent/src/delegate.rs:27`).
- No Investigation session called any `delegate_*` tool.

## 6. Errors and oddities in the Investigation streams

- **No recorded failures.** Both `investigation.mapping_finished` events have `error: null`, `conventions_error: null`, `status: "mapped"`. There are no aborted turns, no interrupted conventions review events, and all 78 policy decisions in the slow session are `allow`.
- **Late completion marker.** In the slow session `mapping_finished` is emitted only at 852.6 s, although conventions finished at 671.4 s.
- **Orphan session.** `s-ff4675c9635d152d` started 0.6 s before the slow session and has no terminal event. Its `meta.json`, like ddc's, is a `deleted: true` tombstone with `created_ms` reset to the deletion time (18:32:41 and 18:32:48). Why it was abandoned is not recorded.
- **Ingest variance.** The identical PDF (same `raw_blob`, 1,471,623 bytes, 37 pages) took 34.1 s in ddc and 137.7 s six minutes later, and was extracted from scratch both times. The orphan session starting at the same moment may have contended for the extractor; that is unverified.
- **No visible progress.** All 26 `model.thinking` events have `chars: 0`, so the 60–85 s record turns produced no visible stream.
- **ddc extras.** One call to an unprefixed `fs_read` returned "No such tool available"; one tool input was malformed JSON; the CLI route exposed unrelated tools (`fs_write`, `arxiv_search`, `doc_ingest`).
- **Duplicate anchor.** The accepted slow summary cites the same 8 lines (28–35, 775 chars) for both `what` and `how`.
- **Extraction quality.** Footnotes are interleaved into body text (lines 1137–1141, 3215–3221), superscripts are split across lines (3458–3460, 3469–3470) and table captions are separated from their bodies. These are exactly the places where anchors failed.
- **Coverage wording.** The map's limitations say appendices A–C "are not mapped" while the conventions catalog covers them, and `coverage: "partial"` is `model_reported` although 96.7% was read.

## Not read or not verified
- The cause of the provider cache misses and of the 137.7 s ingest (both inferences above).
- Reasoning content inside response blobs and the workers' raw blobs.
- The step-mask keep/stride configuration values; the turn 15/23/31/39 pattern is from observed requests.
- Kandinsky was not opened for this task.
- No verifier change was tested in Rust; the acceptance figures come from a Python re-implementation of `investigation.rs:467-523`.