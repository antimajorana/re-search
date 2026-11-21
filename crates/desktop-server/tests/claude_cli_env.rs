//! (d) Billing safety on the desktop route: with an environment credential set, a
//! `claude-cli` turn is refused before anything starts, naming the variable; the explicit
//! opt-in lets it proceed. A separate test binary because it changes the process
//! environment. No real CLI and no model: the "credential" is a dummy string.
#![cfg(windows)]

mod cli_support;
mod common;

use cli_support::*;
use ledger::Ledger;
use serde_json::json;
use std::time::Duration;

#[test]
fn env_credentials_refuse_the_turn_unless_explicitly_allowed() {
    clear_env_auth();
    std::env::set_var("ANTHROPIC_API_KEY", "dummy-not-a-key-0000");
    let h = CliHarness::start();
    let sid = h.create_cli_session();
    h.fake.script(&[
        init_line(&expected_tools(&h.data), core_only()),
        assistant(json!([{ "type": "text", "text": "ok" }])),
        result("ok"),
    ]);
    let mut sse = h.sse(&sid);
    h.send(&sid, "hello");
    let err = sse.wait_turn_end(Duration::from_secs(30)).expect_err("refused");
    let text = err["error"].as_str().unwrap();
    assert!(text.contains("ANTHROPIC_API_KEY") && text.contains("subscription"), "{text}");
    assert!(text.contains("RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH"), "{text}");
    assert!(!text.contains("dummy-not-a-key-0000"), "the value is never shown");
    assert!(h.fake.argv().is_none(), "the CLI must not have been started");
    assert!(Ledger::open(&h.data).unwrap().read_session(&sid).unwrap().is_empty());

    std::env::set_var("RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH", "1");
    h.send(&sid, "hello again");
    let done = sse.wait_turn_end(Duration::from_secs(60)).expect("allowed turn");
    assert_eq!(done["text"], "ok");
    let events = Ledger::open(&h.data).unwrap().read_session(&sid).unwrap();
    assert_eq!(events[0].payload["env_auth_vars_allowed"], json!(["ANTHROPIC_API_KEY"]));
    assert!(!events[0].payload.to_string().contains("dummy-not-a-key-0000"));

    // The same opt-in covers the CLI's own report of a non-subscription credential.
    std::env::remove_var("ANTHROPIC_API_KEY");
    let mut init: serde_json::Value = serde_json::from_str(&init_line(&expected_tools(&h.data), core_only())).unwrap();
    init["apiKeySource"] = json!("ANTHROPIC_API_KEY");
    h.fake.script(&[init.to_string(), assistant(json!([{ "type": "text", "text": "ok2" }])), result("ok2")]);
    h.send(&sid, "with an api key source");
    let done = sse.wait_turn_end(Duration::from_secs(60)).expect("allowed by the opt-in");
    assert_eq!(done["text"], "ok2");
    clear_env_auth();
    h.srv.shutdown();
}
