//! Profile-driven session resume verification (shared package F1, overview section 2.2).
//!
//! Resolves a session's [`ToolProfile`] from the recorded `tool_profile` field (or, for
//! pre-F1 sessions without that field, from the latest host-written mode marker), compares
//! against the profile's policy version (overview 3.1 row 12), and runs the data-driven
//! tool-definition drift check (`may_gain` and `earlier_defs`).

use crate::resume::{
    rehydrate_history, verify_and_read_with_profile, ResumeInspection, ResumeReject, ResumeRejectKind,
};
use ledger::{Event, Ledger};
use research_tools::tool_profile::{
    defs_match_exact, investigation_v2, legacy_profile_name_for_events, lookup,
    profile_for_session_events, ToolProfile, PROFILE_INVESTIGATION_V1, PROFILE_INVESTIGATION_V2,
    TOOL_PROFILE_FIELD,
};
use serde_json::{json, Value};
use tools::{Router, ToolDef};

/// Profile resolved for a session during verified resume.
#[derive(Debug, Clone)]
pub struct ResolvedSessionProfile {
    /// Built profile governing this session's expected tools and policy.
    pub profile: ToolProfile,
    /// `true` when `session.start` carried an explicit `tool_profile` field; `false` for
    /// pre-F1 sessions (or minimal `v0.2` builtin sessions) resolved from mode markers.
    pub explicit: bool,
}

fn reject(kind: ResumeRejectKind, message: impl Into<String>) -> ResumeReject {
    ResumeReject { kind, message: message.into() }
}

fn drift(message: impl Into<String>) -> ResumeReject {
    reject(ResumeRejectKind::Drift, message)
}

/// Resolve a session's [`ToolProfile`] from its verified ledger events.
///
/// - If `session.start` carries `tool_profile`, it must be a string naming a built profile.
/// - If `session.start` has no `tool_profile`, falls back to the latest mode marker
///   (`research/1`, or `investigation/1` when the latest marker is `investigation.started`).
/// - Enforces rule R2: any `session.resume` recording a different `tool_profile` is `Drift`.
///   (Future Solve package S12 may relax this solely on a host-written resume-brief boundary.)
pub fn resolve_session_profile(
    session: &str,
    events: &[Event],
) -> Result<ResolvedSessionProfile, ResumeReject> {
    resolve_session_profile_with_lookup(session, events, |name| {
        lookup(name).map_err(|e| e.to_string())
    })
    .map(|(resolved, _)| resolved)
}

fn is_solve_profile(name: &str) -> bool {
    name.starts_with("solve/")
}

fn valid_brief_resume_checkpoint(events: &[Event], resume_ev: &Event) -> bool {
    if resume_ev.payload.get("resume_mode").and_then(Value::as_str) != Some("brief") {
        return false;
    }
    let Some(sha) = resume_ev.payload.get("brief_sha256").and_then(Value::as_str) else {
        return false;
    };
    if sha.is_empty() {
        return false;
    }
    events.iter().any(|cp| {
        cp.kind == "solve.checkpoint"
            && cp.seq < resume_ev.seq
            && cp.payload.get("brief_sha256").and_then(Value::as_str) == Some(sha)
    })
}

pub(crate) fn resolve_session_profile_with_lookup<F>(
    session: &str,
    events: &[Event],
    mut lookup_fn: F,
) -> Result<(ResolvedSessionProfile, bool), ResumeReject>
where
    F: FnMut(&str) -> Result<ToolProfile, String>,
{
    let start_ev = events
        .iter()
        .find(|e| e.kind == "session.start")
        .ok_or_else(|| {
            reject(
                ResumeRejectKind::CorruptHistory,
                format!("session `{session}` has no `session.start` event"),
            )
        })?;

    let (mut profile, explicit) = match start_ev.payload.get(TOOL_PROFILE_FIELD) {
        Some(Value::String(name)) => {
            let p = lookup_fn(name)
                .map_err(|e| drift(format!("cannot resume session `{session}`: {e}")))?;
            (p, true)
        }
        Some(_) => {
            return Err(drift(format!(
                "cannot resume session `{session}`: `{TOOL_PROFILE_FIELD}` in `session.start` must be a string"
            )));
        }
        None => {
            let inferred = legacy_profile_name_for_events(events);
            let p = lookup_fn(inferred)
                .map_err(|e| drift(format!("cannot resume session `{session}`: {e}")))?;
            (p, false)
        }
    };

    // Rule R2: replay resume cannot switch tool profiles mid-session, except the single
    // forward compatibility upgrade `investigation/1 -> investigation/2`, OR a Solve-only
    // brief-resume boundary (`resume_mode: "brief"` with `brief_sha256` matching a prior
    // `solve.checkpoint`). Cross-mode changes (`research/1 -> solve/1`) remain `Drift`.
    let mut brief_switched = false;
    for ev in events.iter().filter(|e| e.kind == "session.resume") {
        if ev.payload.get("resume_mode").and_then(Value::as_str) == Some("brief")
            && !valid_brief_resume_checkpoint(events, ev)
        {
            return Err(drift(format!(
                "cannot resume session `{session}`: brief resume at seq {} missing matching prior `solve.checkpoint`",
                ev.seq
            )));
        }
        if let Some(val) = ev.payload.get(TOOL_PROFILE_FIELD) {
            let Some(res_name) = val.as_str() else {
                return Err(drift(format!(
                    "cannot resume session `{session}`: `{TOOL_PROFILE_FIELD}` at seq {} must be a string",
                    ev.seq
                )));
            };
            if profile.name == PROFILE_INVESTIGATION_V1 && res_name == PROFILE_INVESTIGATION_V2 {
                profile = lookup_fn(PROFILE_INVESTIGATION_V2)
                    .map_err(|e| drift(format!("cannot resume session `{session}`: {e}")))?;
            } else if res_name != profile.name {
                if is_solve_profile(profile.name)
                    && is_solve_profile(res_name)
                    && valid_brief_resume_checkpoint(events, ev)
                {
                    profile = lookup_fn(res_name)
                        .map_err(|e| drift(format!("cannot resume session `{session}`: {e}")))?;
                    brief_switched = true;
                } else {
                    return Err(drift(format!(
                        "cannot resume session `{session}`: `{TOOL_PROFILE_FIELD}` changed from `{}` to `{res_name}` at seq {}",
                        profile.name, ev.seq
                    )));
                }
            }
        }
    }

    Ok((ResolvedSessionProfile { profile, explicit }, brief_switched))
}

/// Data-driven tool-definition drift check using `profile.may_gain` and `profile.earlier_defs`.
///
/// Every tool recorded in `payload["tool_defs"]` must match `expected_defs` in order by
/// `def_hash()`, or (when the expected tool equals `profile`'s canonical definition for its id)
/// match one of `profile.earlier_defs_for(&exp.id)`. An expected tool absent from `payload` is
/// tolerated only when `profile.allows_gain(exp)` is true (listed in `may_gain` and `ReadWorkspace`).
pub fn check_tool_defs_drift_for_profile(
    session: &str,
    payload: &Value,
    expected_defs: &[ToolDef],
    profile: &ToolProfile,
) -> Result<(), ResumeReject> {
    let recorded = payload
        .get("tool_defs")
        .and_then(Value::as_array)
        .ok_or_else(|| drift(format!("session `{session}` missing `tool_defs`")))?;
    let mut prior = recorded.iter().peekable();
    for exp in expected_defs {
        let same_id = prior.peek().and_then(|rec| rec["id"].as_str()) == Some(exp.id.as_str());
        if !same_id && profile.allows_gain(exp) {
            continue;
        }
        let hash = prior.next().and_then(|rec| rec["def_hash"].as_str());
        let compatible = same_id
            && (hash == Some(exp.def_hash().as_str())
                || (profile.is_canonical_def(exp)
                    && profile
                        .earlier_defs_for(&exp.id)
                        .iter()
                        .any(|earlier| hash == Some(earlier.def_hash().as_str()))));
        if !compatible {
            return Err(drift(format!(
                "cannot resume session `{session}`: tool definition drift for `{}`",
                exp.id
            )));
        }
    }
    if prior.next().is_some() {
        return Err(drift(format!(
            "cannot resume session `{session}`: tools were removed or reordered"
        )));
    }
    Ok(())
}

/// Verify profile identity, policy version (belonging to the profile, overview 3.1 row 12),
/// active router compatibility, and recorded tool definitions.
pub(crate) fn verify_profile_policy_and_defs(
    session: &str,
    events: &[Event],
    start_ev: &Event,
    expected_policy_version: &str,
    expected_defs: &[ToolDef],
    active_router_profile: Option<&str>,
) -> Result<ResolvedSessionProfile, ResumeReject> {
    let (mut resolved, brief_switched) = resolve_session_profile_with_lookup(session, events, |name| {
        lookup(name).map_err(|e| e.to_string())
    })?;
    let upgrading_to_v2 = resolved.profile.name == PROFILE_INVESTIGATION_V1
        && active_router_profile == Some(PROFILE_INVESTIGATION_V2);

    if let Some(active) = active_router_profile {
        if active != resolved.profile.name && !upgrading_to_v2 {
            return Err(drift(format!(
                "cannot resume session `{session}`: active router profile `{active}` differs from session profile `{}`",
                resolved.profile.name
            )));
        }
    }

    if !brief_switched {
        let recorded_policy = start_ev.payload.get("policy_version").and_then(Value::as_str);
        if resolved.explicit {
            let profile_policy = resolved.profile.policy.version.as_str();
            if recorded_policy != Some(profile_policy) {
                return Err(drift(format!(
                    "cannot resume session `{session}`: policy_version `{}` differs from profile `{}` policy `{profile_policy}`",
                    recorded_policy.unwrap_or("<missing>"),
                    resolved.profile.name
                )));
            }
            if expected_policy_version != profile_policy {
                return Err(drift(format!(
                    "cannot resume session `{session}`: active policy_version `{expected_policy_version}` differs from profile `{}` policy `{profile_policy}`",
                    resolved.profile.name
                )));
            }
        }
        if recorded_policy != Some(expected_policy_version) {
            return Err(drift(format!(
                "cannot resume session `{session}`: policy_version `{}` differs from expected `{expected_policy_version}`",
                recorded_policy.unwrap_or("<missing>")
            )));
        }
    }
    if let Some(resume_ev) = events
        .iter()
        .rev()
        .find(|e| e.kind == "session.resume" && e.payload.get("policy_version").is_some())
    {
        let res_policy = resume_ev.payload.get("policy_version").and_then(Value::as_str);
        if res_policy != Some(expected_policy_version) {
            return Err(drift(format!(
                "cannot resume session `{session}`: policy_version `{}` at seq {} differs from expected `{expected_policy_version}`",
                res_policy.unwrap_or("<missing>"),
                resume_ev.seq
            )));
        }
    }

    let v2_profile;
    let check_defs: &[ToolDef] = if upgrading_to_v2 {
        v2_profile = investigation_v2();
        let actual_refs: Vec<&ToolDef> = expected_defs.iter().collect();
        if !defs_match_exact(&actual_refs, &v2_profile.defs) {
            return Err(drift(format!(
                "cannot resume session `{session}`: active router tools do not match profile `{}`",
                v2_profile.name
            )));
        }
        expected_defs
    } else if resolved.explicit {
        let actual_refs: Vec<&ToolDef> = expected_defs.iter().collect();
        if !defs_match_exact(&actual_refs, &resolved.profile.defs) {
            return Err(drift(format!(
                "cannot resume session `{session}`: active router tools do not match profile `{}`",
                resolved.profile.name
            )));
        }
        &resolved.profile.defs
    } else {
        expected_defs
    };

    if !brief_switched {
        check_tool_defs_drift_for_profile(session, &start_ev.payload, check_defs, &resolved.profile)?;
    }
    if let Some(resume) = events
        .iter()
        .rev()
        .find(|event| event.kind == "session.resume" && event.payload["tool_defs"].is_array())
    {
        check_tool_defs_drift_for_profile(session, &resume.payload, check_defs, &resolved.profile)?;
    }

    if upgrading_to_v2 {
        resolved.profile = investigation_v2();
    }

    Ok(resolved)
}

/// Inspect a profiled session for resume by building expected definitions and policy version
/// directly from its recorded (or mode-inferred) [`ToolProfile`].
pub fn inspect_session_with_profile(
    ledger: &Ledger,
    session: &str,
) -> Result<(ResumeInspection, ResolvedSessionProfile), ResumeReject> {
    let (_, events) = ledger.read_verified_session(session).map_err(|err| {
        let kind = match err {
            ledger::Error::LegacyUnchainedSchema => ResumeRejectKind::LegacySchema,
            _ => ResumeRejectKind::TamperOrMissing,
        };
        reject(kind, format!("cannot resume session `{session}`: {err}"))
    })?;
    if events.is_empty() {
        return Err(reject(
            ResumeRejectKind::CorruptHistory,
            format!("session `{session}` has no events"),
        ));
    }
    let resolved = resolve_session_profile(session, &events)?;
    let (insp, verified_events) = verify_and_read_with_profile(
        ledger,
        session,
        resolved.profile.policy.version.as_str(),
        &resolved.profile.defs,
        Some(resolved.profile.name),
    )?;
    rehydrate_history(ledger, &verified_events, insp.last_provider)?;
    Ok((insp, resolved))
}

/// Verify a session for resume against an active [`Router`] (its policy, defs, and profile).
pub(crate) fn verify_router_for_resume(
    ledger: &Ledger,
    router: &Router,
) -> Result<(ResumeInspection, Vec<Event>), ResumeReject> {
    let defs = router.registry().defs();
    let expected_defs: Vec<ToolDef> = defs.iter().map(|d| (*d).clone()).collect();
    verify_and_read_with_profile(
        ledger,
        router.session(),
        router.policy().version.as_str(),
        &expected_defs,
        router.tool_profile(),
    )
}

/// Determine which `tool_profile` name a `session.start` or `session.resume` event should record.
///
/// Uses `router.tool_profile()` when set; otherwise checks whether `router`'s policy and tool
/// definitions match the session's resolved built profile (leaving minimal `v0.2` builtin routers
/// and ad-hoc partial test routers unlabelled so they keep resuming as before).
pub(crate) fn effective_router_profile(router: &Router, events: &[Event]) -> Option<String> {
    if let Some(name) = router.tool_profile() {
        return Some(name.to_string());
    }
    let profile = profile_for_session_events(events).ok()?;
    if router.policy().version == profile.policy.version
        && defs_match_exact(&router.registry().defs(), &profile.defs)
    {
        return Some(profile.name.to_string());
    }
    if profile.name == PROFILE_INVESTIGATION_V1 {
        let v2 = investigation_v2();
        if router.policy().version == v2.policy.version
            && defs_match_exact(&router.registry().defs(), &v2.defs)
        {
            return Some(v2.name.to_string());
        }
    }
    None
}

/// Insert `"tool_profile": "<name>"` into a `session.start` / `session.resume` JSON payload
/// when the active router belongs to a named tool profile.
pub(crate) fn insert_tool_profile(payload: &mut Value, router: &Router, events: &[Event]) {
    if let Some(profile) = effective_router_profile(router, events) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(TOOL_PROFILE_FIELD.to_string(), json!(profile));
        }
    }
}

#[cfg(test)]
#[path = "resume_profile_tests.rs"]
mod tests;
