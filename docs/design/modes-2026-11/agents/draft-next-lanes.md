# Brief draft-next-lanes — prepare later implementation briefs

Read the handoff, `00-overview.md`, the mode documents and the repository's
status table. The accepted owner decisions and the overview govern scope.
This is design and planning work. Implement no mode package before its
dependencies merge. Keep Investigate first.

Use separate Neon subagents on disjoint files for these five groups:

1. Investigate V7–V9 after V6 and the remaining included V packages. Keep the
   already-written V6 and V17 briefs unchanged.
2. Solve S4/S5, then S7–S15 and S17–S21 in dependency order.
3. Explore F12, X3/X4, then X6–X19 in dependency order.
4. F13b/c, F14b/c and F16b. Keep the OCR measurement decision explicit and
   leave dependency approval with the owner.
5. F15 mode adapters, waiting for the relevant mode records and F15 core.

For each ready-to-review draft, name its package and governing design sections,
the conditions for starting, smallest implementation checkpoints, owned files,
shared registration seams, and tests through the entry point. Keep adjacent
packages from claiming the same file at the same time. Distinguish an interface
that must be verified against a future merge from an implementation already in
the base. When the documents disagree, report the conflict and the exact source
for the resolution. Split groups to allow independent work after each gate.

The only exception to the RULES.md document edit restriction for this brief is
new `agents/draft-*.md` files for these five groups. Do not edit the overview,
existing briefs, README, RULES.md, production code, tests, or tool goldens.
Do not launch implementation agents. Use your Neon subagents only to prepare
and cross-review draft text.

Write `reports/draft-next-lanes.md`, including the dependency order and any
items that require the owner or a package merge before dispatch. Push only
to `gemini/modes/draft-next-lanes` as directed in `RULES.md`.
