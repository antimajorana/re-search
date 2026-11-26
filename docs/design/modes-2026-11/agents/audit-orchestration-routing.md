# Brief audit-orchestration-routing — review the F9b execution design

Use Carbon. Review the new Orchestrator design in `00-overview.md` 3.4 against
the actual application. Start from `gemini/work/fixed` and record the commit.
Read the relevant mode tasks, `crates/agent/src/delegate.rs`, reader/orchestration
code, the desktop role resolver and settings, and the F4/F9 reports. Scope is
design and verification. The concurrent packages still own their code.

- Trace an Investigate reader call, an Explore literature call and a Solve
  compute call from role settings to the transport, tool policy, receipt,
  token reporting and caller-provenance check. Identify where an existing
  same-model fallback would spend frontier tokens without an explicit choice.
- Propose the minimum compatible change to route configured smaller workers
  and pin assignments for a run. Include the normal entry points, in-flight
  budget reservation, transport accounting and test seams. Separate current
  reusable behavior from hooks that depend on the in-flight mode packages.
- Stress-test the design against unavailable transports, missing usage,
  concurrent dispatch, cancellation after a send, capped partial work,
  duplicate task requests, and an Orchestrator citing a worker's finding.
  Preserve the mode's existing tool profiles, containment and evidence checks.
- Review the proposal independently across modes. Use Carbon or Neon cloud
  subagents for disjoint reviews if useful. Give concrete proposed file and
  test ownership and any compatibility or unresolved design decision.

Change only `reports/audit-orchestration-routing.md`. Run existing focused
tests when needed for a report claim. Use scripted or loopback data, no real
provider calls, private sources, personal settings or credentials. Push to
`gemini/modes/audit-orchestration-routing` under `RULES.md`.
