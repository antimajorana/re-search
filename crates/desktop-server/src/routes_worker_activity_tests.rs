use super::fold::WORKER_SUMMARY_UNVERIFIED_LABEL;
use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::ChatGptHub;
use crate::routes_session::handle_create_session;
use crate::sse::{SseFrame, SseHub};
use crate::state::DesktopState;
use crate::DesktopConfig;
use research_tools::LiteratureEndpoints;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const SECRET_ANTHROPIC: &str = "sk-ant-supersecret-leak-check-9999";
const SECRET_OPENAI: &str = "sk-openai-supersecret-leak-check-8888";

fn make_state(tmp: &TempDir) -> Arc<DesktopState> {
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let mut store = ProviderKeyStore::default();
    store.set_key(ProviderKind::Anthropic, Some(SECRET_ANTHROPIC.into())).unwrap();
    store.set_key(ProviderKind::OpenAi, Some(SECRET_OPENAI.into())).unwrap();
    store.save_to_data_dir(&data_dir).unwrap();

    let cfg = DesktopConfig {
        port: 0,
        data_dir,
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(BTreeMap::new()),
        chatgpt_test: None,
    };
    let auth = AuthGuard::new(0).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    Arc::new(DesktopState::new(cfg, auth, sse, chatgpt))
}

fn new_session(state: &Arc<DesktopState>) -> String {
    let (st, created) = handle_create_session(state, b"{}").unwrap();
    assert_eq!(st, 200);
    created["id"].as_str().unwrap().to_string()
}

fn contains_secret(val: &Value) -> bool {
    let s = serde_json::to_string(val).unwrap();
    s.contains(SECRET_ANTHROPIC) || s.contains(SECRET_OPENAI)
}

#[test]
fn list_and_detail_fold_workers_usage_missing_blobs_and_scrub_secrets() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    let sid = new_session(&state);
    let ledger = state.open_ledger().unwrap();

    let resp_blob_lit = ledger
        .put_blob(
            format!("Worker summary leaks {SECRET_ANTHROPIC} in raw body and cites Sec. 2")
                .as_bytes(),
        )
        .unwrap();
    let req_blob_x = ledger
        .put_blob(format!("Exchange prompt header {SECRET_OPENAI}").as_bytes())
        .unwrap();
    let tool_blob = ledger
        .put_blob(b"{\"paper\":\"2401.00001\",\"note\":\"ok\"}")
        .unwrap();
    let missing_blob = "deadbeef".repeat(8);

    ledger
        .append(
            &sid,
            "session.start",
            1,
            &json!({ "provider": "anthropic", "model": "claude-opus-4-6" }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "investigation.task_requested",
            1,
            &json!({ "task": "literature" }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.start",
            1,
            &json!({
                "call_id": "job-lit-1",
                "worker": "literature_worker",
                "role": "literature_worker",
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "routing_reason": "auto_smaller_tier",
                "caller": "worker:literature_worker:job-lit-1"
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "context.supplied",
            1,
            &json!({
                "caller": "worker:literature_worker:job-lit-1",
                "ranges": [{ "path": "paper.tex", "start_line": 10, "end_line": 42 }]
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "tool.request",
            1,
            &json!({
                "call_id": "tc-1",
                "tool": "arxiv_search",
                "caller": "worker:literature_worker:job-lit-1"
            }),
        )
        .unwrap();
    let ev_seq = ledger
        .append(
            &sid,
            "investigation.evidence",
            1,
            &json!({
                "caller": "worker:literature_worker:job-lit-1",
                "source_id": "paper-1",
                "quote": "Lemma 3 establishes exponential decay."
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "tool.result",
            1,
            &json!({
                "call_id": "tc-1",
                "ok": true,
                "result_blob": tool_blob,
                "exprs_out": ["decay_bound@1"]
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.turn",
            1,
            &json!({
                "call_id": "job-lit-1",
                "role": "literature_worker",
                "provider": "openai",
                "used_model": "gpt-4.1-mini-2025-04-14",
                "usage": { "input_tokens": 120, "output_tokens": 0 },
                "raw_blob": resp_blob_lit
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.end",
            1,
            &json!({
                "call_id": "job-lit-1",
                "worker": "literature_worker",
                "ok": true,
                "turns": 1,
                "raw_blob": resp_blob_lit
            }),
        )
        .unwrap();

    // Failed worker with partial surviving item and null usage + missing artifact blob.
    ledger
        .append(
            &sid,
            "delegate.start",
            1,
            &json!({
                "call_id": "job-partial-2",
                "worker": "compute_worker",
                "role": "compute_worker",
                "provider": "openai",
                "model": "gpt-4.1-mini",
                "routing_reason": "explicit_role",
                "caller": "worker:compute_worker:job-partial-2"
            }),
        )
        .unwrap();
    let xe_seq = ledger
        .append(
            &sid,
            "evidence.quote",
            1,
            &json!({
                "caller": "worker:compute_worker:job-partial-2",
                "source_id": "paper-1",
                "quote": "Partial surviving equation bound"
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.turn",
            1,
            &json!({
                "call_id": "job-partial-2",
                "role": "compute_worker",
                "provider": "openai",
                "used_model": "gpt-4.1-mini",
                "usage": { "input_tokens": null, "output_tokens": null },
                "raw_blob": missing_blob
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "delegate.end",
            1,
            &json!({
                "call_id": "job-partial-2",
                "worker": "compute_worker",
                "ok": false,
                "turns": 1,
                "error": "worker timed out"
            }),
        )
        .unwrap();

    // Idea exchange + Verifier job.
    ledger
        .append(
            &sid,
            "idea.exchange",
            1,
            &json!({
                "job_id": "ex-1",
                "stage": "critique",
                "role": "strategist",
                "provider": "anthropic",
                "model": "claude-sonnet-4-6",
                "routing_reason": "preset_role",
                "req_blob": req_blob_x,
                "returned_items": ["xe-claim-1"],
                "usage": { "input_tokens": 0, "output_tokens": 35 }
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "verifier.request",
            1,
            &json!({
                "call_id": "ver-1",
                "kind": "full_verifier",
                "role": "verifier",
                "provider": "openai",
                "verifier_model": "gpt-4.1-mini",
                "routing_reason": "auto_smaller_tier"
            }),
        )
        .unwrap();
    ledger
        .append(
            &sid,
            "proof.verification",
            1,
            &json!({
                "kind": "full_verifier",
                "verifier_model": "gpt-4.1-mini",
                "target_entry_id": "entry-7",
                "summary": "Verified step chain",
                "usage": { "input_tokens": 50, "output_tokens": 12 }
            }),
        )
        .unwrap();

    let (code, list) = handle_list_workers(&state, &sid).unwrap();
    assert_eq!(code, 200);
    assert!(!contains_secret(&list), "secret leaked in list response");
    let jobs = list["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 4);

    let lit = jobs.iter().find(|j| j["job_id"] == "job-lit-1").unwrap();
    assert_eq!(lit["status"], "done");
    assert_eq!(lit["disposition"], "accepted");
    assert_eq!(lit["usage"]["input_tokens"], 120);
    assert_eq!(lit["usage"]["output_tokens"], 0);
    assert_eq!(lit["returned_item_ids"], json!([format!("ev-{ev_seq}")]));

    let partial = jobs.iter().find(|j| j["job_id"] == "job-partial-2").unwrap();
    assert_eq!(partial["status"], "partial");
    assert_eq!(partial["disposition"], "partial");
    assert!(partial["usage"]["input_tokens"].is_null());
    assert!(partial["usage"]["output_tokens"].is_null());
    assert_eq!(partial["usage"]["unknown_usage_calls"], 1);
    assert_eq!(partial["returned_item_ids"], json!([format!("xe-{xe_seq}")]));

    // Lazy detail check on completed job: ranges, tool receipts, redacted summary + label.
    let (code, lit_detail) = handle_get_worker_detail(&state, &sid, "job-lit-1").unwrap();
    assert_eq!(code, 200);
    assert!(!contains_secret(&lit_detail), "secret leaked in detail response");
    assert_eq!(lit_detail["artifact_missing"], false);
    assert_eq!(lit_detail["worker_summary"]["label"], WORKER_SUMMARY_UNVERIFIED_LABEL);
    let summary_text = lit_detail["worker_summary"]["text"].as_str().unwrap();
    assert!(summary_text.contains("[REDACTED]"));
    assert_eq!(lit_detail["context_supplied"].as_array().unwrap().len(), 1);
    assert_eq!(lit_detail["tool_receipts"].as_array().unwrap().len(), 1);
    assert_eq!(lit_detail["tool_receipts"][0]["tool"], "arxiv_search");
    assert_eq!(lit_detail["returned_items"][0]["kind"], "investigation.evidence");

    // Lazy detail check on partial job: missing blob explicitly flagged.
    let (code, partial_detail) = handle_get_worker_detail(&state, &sid, "job-partial-2").unwrap();
    assert_eq!(code, 200);
    assert_eq!(partial_detail["artifact_missing"], true);
    assert_eq!(partial_detail["response_blobs"][0]["missing"], true);

    // Lazy detail check on exchange job scrubs OpenAI secret from request blob.
    let (code, ex_detail) = handle_get_worker_detail(&state, &sid, "ex-1").unwrap();
    assert_eq!(code, 200);
    assert!(!contains_secret(&ex_detail));
    assert!(ex_detail["request_blobs"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("[REDACTED]"));

    // Unknown or invalid job_id / session_id errors.
    assert_eq!(handle_get_worker_detail(&state, &sid, "does-not-exist").unwrap_err().0, 404);
    assert_eq!(handle_get_worker_detail(&state, &sid, "../escape").unwrap_err().0, 400);
    assert_eq!(handle_list_workers(&state, "no-such-session").unwrap_err().0, 404);
}

#[test]
fn run_control_actions_emit_sse_fold_state_and_validate_inputs() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);
    let sid = new_session(&state);
    let ledger = state.open_ledger().unwrap();
    ledger
        .append(
            &sid,
            "delegate.start",
            1,
            &json!({
                "call_id": "job-1",
                "role": "literature_worker",
                "provider": "openai",
                "model": "gpt-4.1-mini"
            }),
        )
        .unwrap();
    let ev_seq = ledger
        .append(
            &sid,
            "investigation.evidence",
            1,
            &json!({ "source_id": "s1", "quote": "q1" }),
        )
        .unwrap();
    let ev_id = format!("ev-{ev_seq}");
    ledger
        .append(
            &sid,
            "delegate.end",
            1,
            &json!({
                "call_id": "job-1",
                "ok": true,
                "turns": 1,
                "usage": { "input_tokens": 11, "output_tokens": 7 }
            }),
        )
        .unwrap();

    let (sub_id, rx) = state.sse.subscribe(&sid);

    for body in [
        json!({ "action": "focus_note", "note": "Check boundary conditions" }),
        json!({ "action": "dismiss_worker_item", "item_id": ev_id, "reason": "wrong sign" }),
        json!({ "action": "pin_worker_item", "item_id": ev_id, "job_id": "job-1" }),
        json!({ "action": "pause_after_stage", "enabled": true }),
        json!({ "action": "budget_delta", "delta": 4 }),
        json!({
            "action": "next_run_roles",
            "allow_frontier_fallback": true,
            "roles": {
                "verifier": { "provider": "openai", "model": "gpt-4.1" }
            }
        }),
    ] {
        let bytes = serde_json::to_vec(&body).unwrap();
        let (code, res) = handle_post_run_control(&state, &sid, &bytes).unwrap();
        assert_eq!(code, 200);
        assert_eq!(res["ok"], true);
    }

    // Verify 6 SSE session_event frames arrived with run.control kind.
    let mut sse_count = 0;
    while let Ok(frame) = rx.recv_timeout(Duration::from_millis(50)) {
        if let SseFrame::Event { kind, data, .. } = frame {
            if kind == "session_event" && data.contains("run.control") {
                sse_count += 1;
            }
        }
    }
    state.sse.unsubscribe(&sid, sub_id);
    assert_eq!(sse_count, 6);

    // Pin after dismiss promotes item/job to accepted without altering usage numbers.
    let (_, listed) = handle_list_workers(&state, &sid).unwrap();
    let job = &listed["jobs"][0];
    assert_eq!(job["pinned"], true);
    assert_eq!(job["dismissed"], false);
    assert_eq!(job["disposition"], "accepted");
    assert_eq!(job["usage"]["input_tokens"], 11);
    assert_eq!(job["usage"]["output_tokens"], 7);
    assert_eq!(listed["controls"]["pause_after_stage"], true);
    assert_eq!(listed["controls"]["budget_delta"], 4);
    assert_eq!(listed["roles"]["verifier"]["provider"], "openai");
    assert_eq!(listed["roles"]["verifier"]["model"], "gpt-4.1");

    // Bad payloads return 400.
    for bad_body in [
        json!({ "action": "focus_note", "note": "   " }),
        json!({ "action": "budget_delta", "delta": 0 }),
        json!({ "action": "budget_delta", "delta": 200 }),
        json!({ "action": "next_run_roles", "roles": { "unknown_role": null } }),
        json!({
            "action": "next_run_roles",
            "roles": { "verifier": { "provider": "chatgpt", "model": "gpt-5", "base_url": "https://evil.test" } }
        }),
        json!({
            "action": "next_run_roles",
            "roles": { "verifier": { "provider": "openai", "model": "gpt-4.1-mini", "base_url": "ftp://bad" } }
        }),
        json!({ "action": "nonsense_action" }),
    ] {
        let bytes = serde_json::to_vec(&bad_body).unwrap();
        let err = handle_post_run_control(&state, &sid, &bytes).unwrap_err();
        assert_eq!(err.0, 400, "expected 400 for {bad_body}");
    }
}
