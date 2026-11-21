//! The vendor-CLI route ("external loop, partial provenance").
//!
//! An unmodified agent CLI (today: the Claude Code CLI the user signed into) runs the
//! model's agent loop. Its only tools are re:Search's, served over MCP by `core.exe`, so
//! every tool call goes through the router in `core.exe` (policy, approval, ledger). This
//! crate holds the parts that do not depend on who shows the approval prompt or where the
//! events go, so both the console supervisor (`crates/cli`) and the desktop server
//! (`crates/desktop-server`) use the same code:
//!
//! - [`recorder`]: ledger recorder, event sinks, session and tool-config guards;
//! - [`vendor`]: the [`VendorCli`] trait for the vendor-specific parts, [`claude`] its
//!   Claude Code implementation;
//! - [`turn`]: one turn (spawn in a kill-on-close job object, prompt on stdin, line loop,
//!   handshake check, abort/stop, cleanup);
//! - [`relay`]: the loopback approval listener's wire handling, with the decision taken
//!   from a [`relay::RelayDecider`] (console prompt or desktop approval card);
//! - [`core_tools`]: the tool set `core.exe` serves, shared with `core.exe` itself;
//! - [`env_auth`]: refusal to start when environment credentials would bypass the
//!   subscription login; [`history`]: bounded transcript replay for later turns.

pub mod claude;
pub mod core_tools;
pub mod env_auth;
pub mod history;
pub mod recorder;
pub mod relay;
pub mod turn;
pub mod vendor;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use recorder::{
    ndjson_line, remove_stale_tool_config, sweep_stale_tool_configs, EventSink, Recorder, SessionGuard, ToolConfigGuard,
    TOOL_CONFIG_FILE,
};
pub use turn::{run_turn, StopHandle, TurnEnd, TurnReport, TurnSpec};
pub use vendor::{CoreLaunch, VendorCli, VersionInfo};

/// Version of the NDJSON / ledger events this route produces.
pub const EVENT_VERSION: u32 = 1;

/// Provenance marker recorded in every start event of this route.
pub const PROVENANCE: &str = "external loop, partial provenance";
