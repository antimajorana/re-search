//! Scripted model transport (fixed tier, section 7.1): a queue of canned
//! model responses for one run, with the model calls counted.
//!
//! Built on the existing test machinery, not beside it: the server is
//! `support::MockServer`, the transport is `support::make_transport`, and
//! each response is shaped by `support::ok_msg`. One request to the
//! provider is one model call.

use crate::support::{make_transport, ok_msg, MockServer};
use agent::AnthropicTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// One canned model response. A script file is a JSON array of these:
/// `tests/fixtures/eval/scripts/<case id>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptStep {
    pub id: String,
    /// `tool_use` or `end_turn`, as the provider reports it.
    pub stop_reason: String,
    /// The content blocks of the response.
    pub content: Value,
}

pub fn parse_script(text: &str) -> Result<Vec<ScriptStep>, String> {
    let steps: Vec<ScriptStep> = serde_json::from_str(text).map_err(|e| format!("script does not parse: {e}"))?;
    if steps.is_empty() {
        return Err("a script needs at least one response".into());
    }
    if let Some(bad) = steps.iter().find(|s| !s.content.is_array()) {
        return Err(format!("response `{}`: content must be an array of blocks", bad.id));
    }
    Ok(steps)
}

pub fn load_script(path: &Path) -> Result<Vec<ScriptStep>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_script(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub struct ScriptedModel {
    server: MockServer,
    pub transport: AnthropicTransport,
    scripted: usize,
}

impl ScriptedModel {
    /// Starts the mock provider with the script queued. The key is a
    /// placeholder; the mock never checks it.
    pub fn start(script: &[ScriptStep]) -> Self {
        let responses = script.iter().map(|s| ok_msg(&s.id, &s.stop_reason, s.content.clone())).collect();
        let server = MockServer::start(responses);
        let transport = make_transport(server.base_url.clone(), "sk-eval-scripted-model");
        Self { server, transport, scripted: script.len() }
    }

    /// Model calls made so far: requests the provider mock has received.
    pub fn model_calls(&self) -> u64 {
        self.server.requests_received().len() as u64
    }

    /// Raw request bodies, in order, for a test that inspects what was sent.
    pub fn requests(&self) -> Vec<Vec<u8>> {
        self.server.requests_received()
    }

    /// Ends the run and returns the number of model calls. It is an error
    /// if the run did not use the whole script: a case whose script is
    /// longer than its run is testing something else than it says.
    pub fn finish(self) -> Result<u64, String> {
        let calls = self.model_calls();
        if calls as usize != self.scripted {
            // The server thread is still waiting for the next request;
            // joining it would hang, so it is left to end with the process.
            return Err(format!("the run made {calls} model call(s); the script has {} response(s)", self.scripted));
        }
        self.server.finish();
        Ok(calls)
    }
}
