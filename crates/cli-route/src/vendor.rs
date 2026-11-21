//! The vendor-specific half of the route, as a trait. [`crate::claude::ClaudeCode`] is the
//! only implementation today.

use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Version of the vendor CLI: raw `--version` output, the leading token, and the version
/// this route was tested against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionInfo {
    pub raw: String,
    pub detected: String,
    pub tested: String,
}

impl VersionInfo {
    pub fn matches_tested(&self) -> bool {
        self.detected == self.tested
    }
}

/// How the vendor CLI is told to start `core.exe`: its path and the environment it gets
/// (`CORE_SESSION`, `CORE_SESSION_TOKEN`, `CORE_APPROVAL_ADDR`, ...).
#[derive(Debug, Clone)]
pub struct CoreLaunch {
    pub exe: PathBuf,
    pub env: BTreeMap<String, String>,
}

/// Everything about one vendor CLI that the generic turn loop needs.
pub trait VendorCli: Send + Sync {
    /// Route label recorded in the ledger (for example `claude-code-cli`).
    fn route(&self) -> &'static str;
    /// Caller label `core.exe` uses for this route's tool calls.
    fn caller(&self, session: &str) -> String;
    /// Find the executable: `explicit` when given, otherwise from `env` and well-known places.
    fn locate(&self, explicit: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String>;
    /// Run the CLI's version command (sends nothing to a model).
    fn version(&self, exe: &Path) -> Result<VersionInfo, String>;
    /// Environment variables that would make the CLI authenticate with something other
    /// than the user's subscription login.
    fn env_auth_vars(&self) -> &'static [&'static str];
    /// Name under which the model sees core tool `tool_id`.
    fn tool_name(&self, tool_id: &str) -> String;
    /// Write the per-turn tool config (it carries the session token).
    fn write_tool_config(&self, path: &Path, core: &CoreLaunch) -> Result<(), String>;
    /// Command-line arguments. The prompt is never among them: it goes on stdin.
    fn args(&self, model: &str, tool_config: &Path) -> Vec<String>;
    /// Last adjustments to the command (for example removing nesting markers).
    fn prepare(&self, cmd: &mut Command);
    /// Whether this output line is the handshake that lists the offered tools.
    fn is_handshake(&self, line: &Value) -> bool;
    /// Check the handshake: exactly `expected_tools`, only our MCP server, connected.
    fn check_handshake(&self, line: &Value, expected_tools: &[String]) -> Result<(), String>;
    /// Billing check on a verified handshake: refuse when the CLI reports it would
    /// authenticate with something other than the subscription login, unless the person
    /// opted in (`allow_env_auth`).
    fn check_auth(&self, line: &Value, allow_env_auth: bool) -> Result<(), String>;
    /// Version the handshake reports, if any.
    fn handshake_version(&self, line: &Value) -> Option<String>;
    /// Whether a line shows model activity (which must not come before a verified handshake).
    fn is_model_activity(&self, line: &Value) -> bool;
    /// Map one output line to zero or more `(kind, data)` ledger events.
    fn map_line(&self, line: &Value) -> Vec<(String, Value)>;
}
