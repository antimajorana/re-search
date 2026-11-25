# Brief s3-s6-gate-worker: router gate (S3) and check process (S6) of Solve

Two independent packages; use one subagent for each. Lower priority than the Investigate work,
so take no shortcut to finish early: nobody is waiting on these today.

Design: `solve.md` rows S3 and S6 of the package table and the sections they draw on: the gate
and post-call hook with rule R11 (cannot widen); the worker protocol 2 (`exprs_in`, outbox,
`expr()`), the second process role, `srepr_safe.py`, canonical `srepr` and LaTeX from the check
process, rules R5 and R6; 4.3 and 4.4 for the tiers a check can give (Checked numerically and
Verified; there is no tier called Computed and no Lean). `00-overview.md` section 6, "What can
move": if the restricted parser rejects most real expressions, the lane stops to widen the
allowlist, so a measured list of what it rejects is a deliverable.

## S3

`crates/tools/src/gate.rs`, edits in `router.rs` and `router/tests.rs`. `tools::Gate`; a refusal
becomes a tool error plus a `guardrail.blocked` event; tests R11. A router built without a gate
behaves exactly as today: every existing router test passes untouched, the approval flow and the
policy order (deny > ask > allow) are not weakened, and a gate can only refuse, never allow
something the policy would not.

## S6

`python/worker.py`, `solver_calc.py`, `python/checks/__init__.py`, `srepr_safe.py`, their
tests; `crates/worker/src/lib.rs`, `stage.rs`.

1. The safe parser reads `srepr` text into SymPy objects from an allowlist of constructors with
   no `eval`, no `exec`, no attribute access and no import driven by the input. Test it with
   hostile inputs (dunder names, `__import__`, nested lambdas, huge integers and exponents, deep
   nesting) and with a corpus of ordinary expressions you write: integrals, sums, matrices,
   special functions, physics-flavoured expressions with indexed symbols. Report what share of
   the corpus is accepted and list every rejected constructor.
2. Protocol 2 is additive: a worker speaking protocol 1 to today's host keeps working, and
   today's `sympy_eval` tool definition does not change (golden test). The `cfg(windows)`
   sandbox code is not compiled on your machine: make no change there that you cannot compile,
   and list what a Windows build must check.
3. The check process is a second role of the same script with its own start-up, as the design
   says; it never sees solver state other than what the protocol hands it. Tests R5 and R6.
4. Python tests run with `python -m unittest discover -s python` and need SymPy: install it on
   your machine.
