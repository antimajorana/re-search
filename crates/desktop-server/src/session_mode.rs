//! Single source of truth for desktop session mode detection and mode-to-profile lookup (F5).
//!
//! Reuses [`research_tools::tool_profile::session_mode_from_events`] directly so there is never
//! a second mode-detection rule anywhere in the desktop server (`00-overview.md` 3.1 row 3).

use ledger::Event;
use research_tools::paper_key::Mode;
use research_tools::tool_profile::{
    active_profile_for_session_events, legacy_profile_name_for_mode, lookup as lookup_profile,
    ProfileError, ToolProfile, MARKER_IDEA_STARTED, MARKER_INVESTIGATION_HANDOFF,
    MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED, PROFILE_INVESTIGATION_V2,
};

/// Wire mode string for earlier Research sessions on `/api/sessions` (`00-overview.md` 5.2).
pub const MODE_API_RESEARCH: &str = "research";
/// Wire mode string for Investigate sessions on `/api/sessions` (kept as `"investigation"`
/// for backwards compatibility with existing desktop UI and API callers; `"investigate"` is
/// also accepted on session creation).
pub const MODE_API_INVESTIGATE: &str = "investigation";
/// Accepted alias for Investigate mode on `POST /api/sessions` (`00-overview.md` 5.2).
pub const MODE_API_INVESTIGATE_ALIAS: &str = "investigate";
/// Wire mode string for Explore sessions (`00-overview.md` 5.2).
pub const MODE_API_EXPLORE: &str = "explore";
/// Wire mode string for Solve sessions (`00-overview.md` 5.2).
pub const MODE_API_SOLVE: &str = "solve";

/// On-screen mode labels (`00-overview.md` 5.2, 5.3, 8.2 decision 7).
pub const LABEL_RESEARCH: &str = "Research (earlier)";
pub const LABEL_INVESTIGATE: &str = "Investigate";
pub const LABEL_EXPLORE: &str = "Explore";
pub const LABEL_SOLVE: &str = "Solve";

/// Host-written mode marker event kinds (`00-overview.md` 3.1 row 3, rule R2).
/// No tool handler may ever emit any of these kinds.
pub const HOST_ONLY_MODE_MARKERS: &[&str] = &[
    MARKER_INVESTIGATION_STARTED,
    MARKER_INVESTIGATION_HANDOFF,
    MARKER_IDEA_STARTED,
    MARKER_SOLVE_STARTED,
];

/// Determine the mode of a session from its ledger events.
///
/// Always delegates to [`research_tools::tool_profile::session_mode_from_events`]
/// (latest of `investigation.started`, `investigation.handoff`, `idea.started`, `solve.started`;
/// `investigation.handoff` and no marker resolve to [`Mode::Research`]).
pub fn session_mode(events: &[Event]) -> Mode {
    research_tools::tool_profile::session_mode_from_events(events)
}

/// Wire string returned in `"mode"` on `/api/sessions` and `/api/sessions/{id}`.
pub fn api_mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Research => MODE_API_RESEARCH,
        Mode::Investigate => MODE_API_INVESTIGATE,
        Mode::Explore => MODE_API_EXPLORE,
        Mode::Solve => MODE_API_SOLVE,
    }
}

/// Human-readable on-screen mode label (`00-overview.md` 5.2).
pub fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Research => LABEL_RESEARCH,
        Mode::Investigate => LABEL_INVESTIGATE,
        Mode::Explore => LABEL_EXPLORE,
        Mode::Solve => LABEL_SOLVE,
    }
}

/// Host-written starting marker event kind for a newly created session of `mode`, if any.
pub fn start_marker_for_mode(mode: Mode) -> Option<&'static str> {
    match mode {
        Mode::Research => None,
        Mode::Investigate => Some(MARKER_INVESTIGATION_STARTED),
        Mode::Explore => Some(MARKER_IDEA_STARTED),
        Mode::Solve => Some(MARKER_SOLVE_STARTED),
    }
}

/// Parse a mode string from `POST /api/sessions`.
pub fn parse_requested_mode(raw: &str) -> Result<Mode, String> {
    match raw.trim() {
        MODE_API_RESEARCH => Ok(Mode::Research),
        MODE_API_INVESTIGATE | MODE_API_INVESTIGATE_ALIAS => Ok(Mode::Investigate),
        MODE_API_EXPLORE => Ok(Mode::Explore),
        MODE_API_SOLVE => Ok(Mode::Solve),
        _ => Err("Choose Research, Investigate, Explore, or Solve.".to_string()),
    }
}

/// Desktop-server gate for Solve session creation and turns (`solve/1` enabled in S12e).
const DESKTOP_MODE_SOLVE_ENABLED: bool = true;

/// Desktop-server gate for bare Explore session creation and Conversation turns (`explore/1`
/// tool profile is frozen in package X7 and used directly by `build_idea_router`, while bare
/// Explore sessions and `/messages` Conversation turns land in X15).
pub(crate) const DESKTOP_MODE_EXPLORE_ENABLED: bool = false;

/// Resolve the built [`ToolProfile`] for a mode, returning a plain user-facing error when
/// that mode's desktop session flow or tool profile has not been built yet (never faking
/// or falling back to Research).
pub fn profile_for_mode(mode: Mode) -> Result<ToolProfile, String> {
    if mode == Mode::Solve && !DESKTOP_MODE_SOLVE_ENABLED {
        return Err(format!(
            "{} mode is not built yet (session setup and loop pending S4/S12).",
            mode_label(mode)
        ));
    }
    if mode == Mode::Explore && !DESKTOP_MODE_EXPLORE_ENABLED {
        return Err(format!(
            "{} mode is not built yet (conversation turns pending X15).",
            mode_label(mode)
        ));
    }
    let profile_name = match mode {
        Mode::Investigate => PROFILE_INVESTIGATION_V2,
        other => legacy_profile_name_for_mode(other),
    };
    lookup_profile(profile_name).map_err(|err| format_profile_error(mode, &err))
}

/// Resolve the [`ToolProfile`] for an existing session from its ledger events.
///
/// Respects an explicit `session.start["tool_profile"]` if present (upgrading older
/// `investigation/1` sessions onto `investigation/2` per `investigate.md` §8.2), otherwise maps
/// [`session_mode`] to its active profile. Returns a plain error when the mode or profile is not
/// built yet in the desktop server.
pub fn profile_for_session(events: &[Event]) -> Result<ToolProfile, String> {
    let mode = session_mode(events);
    if mode == Mode::Solve && !DESKTOP_MODE_SOLVE_ENABLED {
        return Err(format!(
            "{} mode is not built yet (session setup and loop pending S4/S12).",
            mode_label(mode)
        ));
    }
    if mode == Mode::Explore && !DESKTOP_MODE_EXPLORE_ENABLED {
        return Err(format!(
            "{} mode is not built yet (conversation turns pending X15).",
            mode_label(mode)
        ));
    }
    active_profile_for_session_events(events).map_err(|msg| {
        if msg.contains("profile not built yet") {
            format!("{} mode is not built yet ({msg}).", mode_label(mode))
        } else {
            msg
        }
    })
}

/// True iff `mode` is built and enabled in the desktop server.
pub fn is_mode_built(mode: Mode) -> bool {
    profile_for_mode(mode).is_ok()
}

fn format_profile_error(mode: Mode, err: &ProfileError) -> String {
    match err {
        ProfileError::NotBuiltYet { name } => {
            format!("{} mode is not built yet (tool profile `{name}`).", mode_label(mode))
        }
        ProfileError::Unknown { name } => {
            format!("{} mode uses an unknown tool profile `{name}`.", mode_label(mode))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_tools::tool_profile::{
        PROFILE_INVESTIGATION_V1, PROFILE_INVESTIGATION_V2, PROFILE_RESEARCH_V1,
    };
    use serde_json::json;

    fn ev(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
        Event {
            session: "s-test".to_string(),
            seq,
            ts_ms: 1000 + seq as i64,
            kind: kind.to_string(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "a".repeat(64),
        }
    }

    #[test]
    fn session_mode_follows_latest_marker_rule() {
        assert_eq!(session_mode(&[]), Mode::Research);
        assert_eq!(session_mode(&[ev(1, MARKER_INVESTIGATION_STARTED, json!({}))]), Mode::Investigate);
        assert_eq!(
            session_mode(&[
                ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
                ev(2, MARKER_INVESTIGATION_HANDOFF, json!({})),
            ]),
            Mode::Research
        );
        assert_eq!(
            session_mode(&[
                ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
                ev(2, MARKER_IDEA_STARTED, json!({})),
            ]),
            Mode::Explore
        );
        assert_eq!(
            session_mode(&[
                ev(1, MARKER_IDEA_STARTED, json!({})),
                ev(2, MARKER_SOLVE_STARTED, json!({})),
            ]),
            Mode::Solve
        );
    }

    #[test]
    fn built_and_unbuilt_modes_resolve_or_error_plainly() {
        assert_eq!(profile_for_mode(Mode::Research).unwrap().name, PROFILE_RESEARCH_V1);
        assert_eq!(profile_for_mode(Mode::Investigate).unwrap().name, PROFILE_INVESTIGATION_V2);
        assert!(is_mode_built(Mode::Research));
        assert!(is_mode_built(Mode::Investigate));
        assert!(!is_mode_built(Mode::Explore));
        assert!(is_mode_built(Mode::Solve));

        // Older investigation/1 sessions (marker-inferred or explicit) upgrade to investigation/2.
        assert_eq!(
            profile_for_session(&[
                ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
                ev(2, "session.start", json!({})),
            ])
            .unwrap()
            .name,
            PROFILE_INVESTIGATION_V2
        );
        assert_eq!(
            profile_for_session(&[
                ev(1, MARKER_INVESTIGATION_STARTED, json!({})),
                ev(2, "session.start", json!({ "tool_profile": PROFILE_INVESTIGATION_V1 })),
            ])
            .unwrap()
            .name,
            PROFILE_INVESTIGATION_V2
        );
        // Research sessions stay research/1.
        assert_eq!(
            profile_for_session(&[ev(1, "session.start", json!({}))]).unwrap().name,
            PROFILE_RESEARCH_V1
        );

        let explore_err = profile_for_mode(Mode::Explore).unwrap_err();
        assert!(explore_err.contains("Explore mode is not built yet"), "{explore_err}");
        assert_eq!(
            profile_for_mode(Mode::Solve).unwrap().name,
            research_tools::tool_profile::PROFILE_SOLVE_V1
        );

        assert_eq!(
            profile_for_session(&[ev(1, MARKER_SOLVE_STARTED, json!({}))])
                .unwrap()
                .name,
            research_tools::tool_profile::PROFILE_SOLVE_V1
        );
    }

    #[test]
    fn parse_requested_mode_accepts_all_four_and_alias() {
        assert_eq!(parse_requested_mode("research").unwrap(), Mode::Research);
        assert_eq!(parse_requested_mode("investigation").unwrap(), Mode::Investigate);
        assert_eq!(parse_requested_mode("investigate").unwrap(), Mode::Investigate);
        assert_eq!(parse_requested_mode("explore").unwrap(), Mode::Explore);
        assert_eq!(parse_requested_mode("solve").unwrap(), Mode::Solve);
        assert!(parse_requested_mode("unknown").is_err());
    }
}
