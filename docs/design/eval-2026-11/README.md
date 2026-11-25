# Evaluation set and harness (package F8)

This directory holds the evaluation sheets, one per run. The harness that
produces them is package F8 of the overview (`00-overview.md`, section 7).
What is here today is the skeleton: the formats, the gate and the
fixed-tier plumbing, with self-tests. It contains no evaluation case yet;
see "Open slots" below.

| What | Where |
|---|---|
| Harness code (test support, no production code) | `crates/agent/tests/eval_support/` |
| Harness self-tests | `crates/agent/tests/eval_harness.rs` |
| Fixed tier: manifests and fixtures | `tests/fixtures/eval/` |
| Sheets, one per run | this directory |
| Blank sheet | `sheet-template.md` |

## The two tiers

| Tier | Where | Content | Runs |
|---|---|---|---|
| Fixed | In the repository, `tests/fixtures/eval/` | Self-written TeX, recorded service responses served by a loopback mock, a scripted model, ledgers of old sessions. No third-party paper text | In CI, on every change |
| Live | Outside the repository | Manifests of arXiv ids, ideas and problems with hand-made answer keys. Papers are fetched, never stored in the repository | By hand before each milestone; one sheet per run, filed here |

The owner chooses the real papers and ideas. A reviewer who is not the
builder of a feature writes its answer keys.

## Manifest format

One format for all modes. One file per set, named `manifest-<set>.json`
with the set letter in lower case (`manifest-p.json`, `manifest-p2.json`,
`manifest-q.json`, ...).

```json
{
  "format_version": 1,
  "set": "Q",
  "tier": "fixed",
  "cases": [
    { "id": "Q-...", "mode": "solve", "input": { "kind": "problem", "...": "..." }, "answer_key": { "...": "..." }, "notes": "..." }
  ]
}
```

A case is exactly `{id, mode, input, answer_key, notes}`. Set and tier are
stated once per file, in the envelope.

Why one file per set and not one manifest: the sets belong to different
lanes (set Q is written by the Solve lane, set I by Explore) and their
keys to different reviewers, so separate files keep their edits apart; and
the validator can state set-wide rules (which mode, which input) once per
file instead of once per case. The loader reads every `manifest*.json` of
a directory and checks them together.

| Set | Mode | Input kinds | The answer key must have |
|---|---|---|---|
| P | `investigate` | `arxiv`, `doi`, `fixture` (fixed tier only) | `statements`, `sections`, `uses_edges` |
| P2 | `investigate` | `arxiv` with two `versions`, or two `fixture` files | `statements` |
| P3 | `investigate` | as P, with an optional `locator` | `results` |
| P4 | `investigate` | as P, with an optional `locator` | `earlier_source` |
| P5 | `investigate` | as P, with an optional `locator` | `statement` |
| I | `explore` | `idea` | `parts` |
| Q | `solve` | `problem` (`task`: `calculate`, `check_derivation`, `prove`) | `final_answer`, `steps`, `planted_step` |
| C | `handover` | `chain` (`from` and `to` are case ids: P to Q, P to I, I to Q) | `target_answer` |
| R | `resume` | `ledger` (fixed tier only) | `must_resume`, `same_content` |

What the validator enforces (`eval_support/manifest.rs`):

- Unknown fields are rejected everywhere: envelope, case, input.
- Ids are unique across all manifests loaded together and start with the
  set letter (`P-`, `P2-`, `Q-`, ...).
- The mode is the one of the set.
- No kind of input can hold a paper's text. A paper is named by arXiv id
  or DOI (checked by shape, without network), or, in the fixed tier only,
  by the path of a self-written file under `tests/fixtures/eval/`. A
  locator ("Theorem 2.1") is one line of at most 80 characters.
- A fixture path is relative, uses `/`, cannot leave the fixtures
  directory, and the file must exist.
- A chain names two existing cases in an allowed direction.
- The answer key is an object with the members listed above. Their inner
  shape is left to the key writers and the mode that reads them.

What it cannot enforce: that an answer key of the live tier quotes no
paper text (set P3 asks for "the sentence that says so"). That is why live
manifests stay outside the repository.

## Sheet format

A sheet is data (`eval_support/sheet.rs`, JSON) rendered to Markdown:
date, build (git commit and crate version), models, model calls, wall
time, cases run, and the measures of section 7.3. `sheet-template.md` is
the rendering of a blank sheet and is checked by a test.

The measures are a table in `eval_support/measures.rs`, transcribed from
7.3. Each has a rule:

| Rule | Meaning |
|---|---|
| `recorded` | The number is written down. It never passes or fails |
| `must_equal` | Passes only at exactly the target (100 percent means every case) |
| `must_be_zero` | Passes only at 0 |
| `threshold` | Passes below, or at or above, the bound that 7.3 gives |

Outcome per measure: `pass`, `FAIL`, `recorded`, or `not measured`. A
measure without a value is `not measured`; it is never a pass. A
percentage over zero cases (no planted-error case run) has no value.

Gate over the whole sheet:

| Verdict | When |
|---|---|
| `FAIL` | At least one measure with a pass rule failed |
| `INCOMPLETE` | Nothing failed, but at least one measure with a pass rule has no value |
| `PASS` | Every measure with a pass rule was measured and passed |

The sheet lists what failed and what is still open by id.

## How to run

Fixed tier (what CI runs as part of `cargo test --locked --workspace`):

```
cargo test --locked -p agent --test eval_harness
```

After changing the measures or the renderer, regenerate the template by
running the same command with the environment variable `EVAL_BLESS=1`.

Live tier, by hand, before a milestone:

1. Check the manifests kept outside the repository:
   `EVAL_LIVE_MANIFEST_DIR=<dir> cargo test --locked -p agent --test eval_harness -- --ignored live_manifests_validate`
2. Run the cases. There is no live runner yet (see "Open slots"); each
   mode adds its own as it lands, using the loader, the gate and the sheet
   from `eval_support`.
3. Fill a sheet as JSON (the fields of `Sheet`; leave out what was not
   measured) and file it:
   `EVAL_SHEET_JSON=<file> cargo test --locked -p agent --test eval_harness -- --ignored file_sheet_from_json`
   An empty date or build is filled in from today and the checked-out
   commit. The sheet is written here as
   `sheet-<date>-<tier>-<commit>.md`; an existing sheet is never
   overwritten.

On Windows PowerShell set the variable first, for example
`$env:EVAL_SHEET_JSON = "C:\path\sheet.json"`.

## Fixed-tier plumbing

| Part | File | Notes |
|---|---|---|
| Manifest loader | `eval_support/mod.rs` | `load_manifests(dir, tier)`; the repository root is found from the crate's manifest directory |
| Recorded-service mock | `eval_support/routed_mock.rs` | Routes by method, path and query (order of query parts ignored) to a recorded body. Anything else gets 404 and is listed by `unexpected()`. Recordings are `tests/fixtures/eval/recorded/<name>.json`. Point the literature tools at it with `LiteratureEndpoints::loopback_mock` |
| Scripted model | `eval_support/scripted.rs` | A queue of canned responses, `tests/fixtures/eval/scripts/<case id>.json`, served through the existing `support::MockServer` and Anthropic transport. One request to the provider is one model call. A run that does not use its whole script is an error |

## What is in `tests/fixtures/eval/` today

Only harness self-test cases, marked as such in their `notes`. They are
not the evaluation set and no figure may be quoted from them.

- `manifest-p.json`, `p/*.tex`: two self-written TeX notes (one with
  theorem environments, one physics-style without) with keys written by
  the builder of the harness.
- `manifest-q.json`: a Gaussian integral and a two-step derivation with
  one planted error, both checked by hand.
- `scripts/Q-selftest-gaussian.json`: the scripted model responses of the
  one case the self-test runs through the agent loop.
- `recorded/selftest.json`: a hand-written recording that exercises the
  mock. It is not a response of any real service.

## Open slots

| Slot | What is needed | From whom |
|---|---|---|
| Sets P, P2 to P5 (live) | 10 arXiv ids with the mix of 7.2; 3 papers in two versions; 5 papers with 6 to 10 results each; 5 restated and 5 not known restated results; 10 citations with a locator | Owner picks; a reviewer who is not the Investigate builder writes the keys; kept outside the repository |
| Sets P to P5 (fixed) | Self-written TeX covering the same shapes (multi-file, custom theorem macros, two versions, a citation with a locator) | Investigate lane, with V1 to V4 |
| Set I | 10 ideas (3 done, 2 done under another name, 3 done partly, 2 open as far as known), key papers and expected status per part, frozen as recorded responses | Owner picks; a reviewer who is not the Explore builder writes the keys; recording by the Explore lane (X14) |
| Set Q | 10 calculations with known closed forms including the effective-potential example, 3 derivations with one planted error each, 2 prose-heavy proofs; final answer, expected tier per step, planted step; scripted-model fixtures | Solve lane (former S0); keys reviewed by someone who is not the Solve builder |
| Set C | 3 chains P to Q, 2 P to I, 2 I to Q, with the answer of the target case | Solve and Explore lanes supply the targets, once P, I and Q exist |
| Set R | 4 ledgers: research, investigation before pointer citations, investigation current, hand-off child. They must be real ledgers written by those builds, not hand-made | Whoever has such sessions (owner or integrator); the hand-off child needs F7 |
| Recorded service responses | Recordings of the literature services for the fixed tier, with the date and the request list in `notes` | Lane C with F11; see the question on third-party text below |
| Live runner per mode | Code that runs a manifest against real services and models and fills a `Sheet` | Each mode's lane, with the mode |
| Caps | The model-call and token caps that `runs_above_cap` is counted against | F9 and the three mode designs; write the cap in force into the measure's note |
| Forbidden-word property tests | The tests behind `forbidden_word_property_failures` | Each mode's lane |

## Questions section 7 leaves open

They are recorded here so that the first real sheet does not settle them
silently.

1. The 0.9 recall bar ("below 0.9 on TeX papers with theorem environments
   blocks M1") does not say whether it applies to statement recall,
   section recall or both, nor whether it is per paper or pooled. The
   table has one measure for each recall on those papers, both with the
   bar; whoever fills the sheet writes the aggregation used into the note.
2. The three time targets are "targets". The gate treats a miss as a
   failure. If they are meant as goals that do not block, their rule
   should become `recorded`.
3. "Recorded; every such case reviewed" (judged same where the key says
   different) is modelled as a recorded count plus a count of unreviewed
   cases that must be 0.
4. "None in scope" for a result with a known earlier source "decides
   whether the feature is announced", with no number given. It is
   recorded only.
5. The fixed tier holds "recorded service responses" and "no third-party
   paper text". A recorded search response of a real service carries
   titles and abstracts of real papers. Set I is to be frozen as such
   recordings in `tests/fixtures/eval/`. The owner should decide whether
   titles and abstracts count as paper text, or whether the recordings
   are reduced to identifiers and metadata.
6. Set P is "10 arXiv ids" and includes "1 PDF-only" paper; the 10 s
   target is for TeX. No time target is given for PDF.
7. "Old ledgers resume; views equal" is "Must pass; in CI", so it belongs
   to the fixed tier only; "Property tests in CI" likewise is not a number
   a live run produces. Both appear on every sheet and stay "not measured"
   on a live sheet unless the CI result is copied in, which keeps a live
   gate at `INCOMPLETE`.
8. The gate is one verdict over all measures. Section 6 ties bars to
   milestones (recall blocks M1; planted errors are a release blocker)
   but does not say which measures a given milestone needs. Until that is
   decided, a milestone sheet is read by its lists of failed and open
   measures, not by the single word.
