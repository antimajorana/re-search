//! Integration tests for WP7: ledger-derived usage aggregation (`GET /api/usage`)
//! and compact per-session usage summaries in `GET /api/sessions/{id}`.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use desktop_server::state::{now_ms, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use ledger::Ledger;
use serde_json::{json, Value};
use std::path::Path;
use tempfile::TempDir;

const MS_PER_DAY: i64 = 86_400_000;

fn save_meta(data_dir: &Path, meta: &SessionMeta) {
    let dir = data_dir.join("sessions").join(&meta.id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(meta).unwrap()).unwrap();
}

fn utc_date_for_ts(ts_ms: i64) -> String {
    let day = ts_ms.div_euclid(MS_PER_DAY);
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

#[test]
fn usage_endpoint_aggregates_multi_day_providers_and_cli_routes_from_ledger() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();

    // Anchor to noon UTC today so +/- N days never straddle midnight during test runtime.
    let today_noon = (now_ms().div_euclid(MS_PER_DAY)) * MS_PER_DAY + 12 * 3_600_000;
    let day_0 = today_noon;
    let day_3 = today_noon - 3 * MS_PER_DAY;
    let day_6 = today_noon - 6 * MS_PER_DAY;
    let day_10 = today_noon - 10 * MS_PER_DAY;

    // Session 1: Anthropic + OpenAI calls using real agent wire_id() names across today and 3 days ago (research mode).
    save_meta(&data_dir, &SessionMeta {
        id: "s-api-mix".into(),
        title: "Spin Chain Perturbation".into(),
        created_ms: day_3,
        ..Default::default()
    });
    ledger.append_with_ts("s-api-mix", "session.start", 1, &json!({
        "provider": "anthropic-messages", "model": "claude-sonnet-4-6", "prompt": "Check spin chain"
    }), day_3).unwrap();
    // Aborted request with a different model must not leak into the next turn's response.
    ledger.append_with_ts("s-api-mix", "model.request", 1, &json!({
        "turn": 1, "model": "stale-aborted-model"
    }), day_3 + 5).unwrap();
    ledger.append_with_ts("s-api-mix", "model.error", 1, &json!({ "turn": 1, "error": "timeout" }), day_3 + 8).unwrap();
    ledger.append_with_ts("s-api-mix", "model.request", 1, &json!({
        "turn": 1, "model": "claude-sonnet-4-6"
    }), day_3 + 10).unwrap();
    ledger.append_with_ts("s-api-mix", "model.response", 1, &json!({
        "turn": 1,
        "model": "claude-sonnet-4-6",
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1000, "output_tokens": 250, "cache_read_input_tokens": 300, "cache_creation_input_tokens": 100 }
    }), day_3 + 20).unwrap();
    // Same session resumed today on OpenAI Chat Completions wire format.
    ledger.append_with_ts("s-api-mix", "session.resume", 1, &json!({
        "provider": "openai-chat", "model": "gpt-4.1", "prompt": "Cross check"
    }), day_0).unwrap();
    ledger.append_with_ts("s-api-mix", "model.response", 1, &json!({
        "turn": 2,
        "model": "gpt-4.1",
        "stop_reason": "stop",
        "usage": {
            "prompt_tokens": 600,
            "completion_tokens": 150,
            "prompt_tokens_details": { "cached_tokens": 100 }
        }
    }), day_0 + 20).unwrap();

    // Session 2: Gemini (6 days ago, still in last_7_days) + older Responses API call (10 days ago, all_time only) in Solve mode.
    save_meta(&data_dir, &SessionMeta {
        id: "s-solve-old".into(),
        title: "Solve Integral".into(),
        created_ms: day_10,
        ..Default::default()
    });
    ledger.append_with_ts("s-solve-old", "solve.started", 1, &json!({}), day_10).unwrap();
    ledger.append_with_ts("s-solve-old", "session.start", 1, &json!({
        "provider": "chatgpt-responses", "model": "gpt-5.4", "prompt": "Solve I"
    }), day_10 + 1).unwrap();
    ledger.append_with_ts("s-solve-old", "model.response", 1, &json!({
        "turn": 1,
        "model": "gpt-5.4",
        "usage": {
            "input_tokens": 2000,
            "output_tokens": 500,
            "input_tokens_details": { "cached_tokens": 300 }
        }
    }), day_10 + 10).unwrap();
    ledger.append_with_ts("s-solve-old", "session.resume", 1, &json!({
        "provider": "gemini-generate-content", "model": "gemini-2.5-pro", "prompt": "Verify I"
    }), day_6).unwrap();
    ledger.append_with_ts("s-solve-old", "model.response", 1, &json!({
        "turn": 2,
        "model": "gemini-2.5-pro",
        "usage": {
            "promptTokenCount": 800,
            "candidatesTokenCount": 200,
            "cachedContentTokenCount": 50
        }
    }), day_6 + 10).unwrap();

    // Session 3: Subscription CLI route (claude-cli) with model.response carrying null / empty usage (today).
    save_meta(&data_dir, &SessionMeta {
        id: "s-claude-cli".into(),
        title: "Claude Subscription Run".into(),
        route: Some(CLAUDE_CLI_ROUTE.into()),
        created_ms: day_0 + 100,
        ..Default::default()
    });
    ledger.append_with_ts("s-claude-cli", "session.start", 1, &json!({
        "provider": CLAUDE_CLI_ROUTE, "route": "claude-code-cli", "model": "sonnet", "prompt": "Run check"
    }), day_0 + 100).unwrap();
    ledger.append_with_ts("s-claude-cli", "model.response", 1, &json!({
        "turn": 1, "model": "sonnet", "usage": null
    }), day_0 + 110).unwrap();
    ledger.append_with_ts("s-claude-cli", "model.response", 1, &json!({
        "turn": 2, "model": "sonnet", "usage": {}
    }), day_0 + 120).unwrap();

    // Session 4: Antigravity CLI route with zero-with-no-real-tokens model.response (3 days ago).
    save_meta(&data_dir, &SessionMeta {
        id: "s-agy-cli".into(),
        title: "Antigravity Check".into(),
        route: Some(ANTIGRAVITY_ROUTE.into()),
        created_ms: day_3 + 200,
        ..Default::default()
    });
    ledger.append_with_ts("s-agy-cli", "session.start", 1, &json!({
        "provider": ANTIGRAVITY_ROUTE, "route": ANTIGRAVITY_ROUTE, "model": "agy-default"
    }), day_3 + 200).unwrap();
    ledger.append_with_ts("s-agy-cli", "model.response", 1, &json!({
        "turn": 1, "usage": { "input_tokens": 0, "output_tokens": 0 }
    }), day_3 + 210).unwrap();

    // Session 5: Deleted session tombstone must be excluded from global usage and return 404 on ?session_id.
    save_meta(&data_dir, &SessionMeta {
        id: "s-deleted".into(),
        title: String::new(),
        created_ms: day_0,
        deleted: true,
        ..Default::default()
    });
    ledger.append_with_ts("s-deleted", "session.start", 1, &json!({
        "provider": "anthropic-messages", "model": "claude-sonnet-4-6", "prompt": "Secret deleted prompt"
    }), day_0).unwrap();
    ledger.append_with_ts("s-deleted", "model.response", 1, &json!({
        "turn": 1, "model": "claude-sonnet-4-6", "usage": { "input_tokens": 99999, "output_tokens": 9999 }
    }), day_0 + 10).unwrap();

    // Confirm seeded ledger is 100% hash-chain valid.
    let verify = ledger.verify_all().unwrap();
    assert_eq!(verify.sessions.len(), 5, "expected 5 verified sessions: {verify:?}");
    drop(ledger);

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let hdr = [("X-ReSearch-Token", token.as_str())];

    let (st, _, raw) = raw_http(port, "GET", "/api/usage", &hdr, b"");
    assert_eq!(st, 200, "GET /api/usage failed: {}", String::from_utf8_lossy(&raw));
    let body_text = String::from_utf8(raw).unwrap();
    assert!(
        !body_text.to_ascii_lowercase().contains("cost") && !body_text.contains("usd") && !body_text.contains('$'),
        "usage payload must contain zero cost estimates: {body_text}"
    );
    let report: Value = serde_json::from_str(&body_text).unwrap();

    // Today: 1 OpenAI (600/150) + 2 claude-cli unreported = 3 calls, 600 in, 150 out, 2 unreported.
    assert_eq!(report["totals"]["today"]["calls"], 3);
    assert_eq!(report["totals"]["today"]["input_tokens"], 600);
    assert_eq!(report["totals"]["today"]["output_tokens"], 150);
    assert_eq!(report["totals"]["today"]["unreported_calls"], 2);

    // Last 7 days: today (3 calls, 600/150, 2 unrep) + day_3 (1 anthropic 1000/250 + 1 agy unrep)
    //            + day_6 (1 gemini 800/200) = 6 calls, 2400 in, 600 out, 3 unreported.
    assert_eq!(report["totals"]["last_7_days"]["calls"], 6);
    assert_eq!(report["totals"]["last_7_days"]["input_tokens"], 2400);
    assert_eq!(report["totals"]["last_7_days"]["output_tokens"], 600);
    assert_eq!(report["totals"]["last_7_days"]["unreported_calls"], 3);

    // All time: last_7_days + day_10 (1 chatgpt 2000/500) = 7 calls, 4400 in, 1100 out, 3 unreported.
    assert_eq!(report["totals"]["all_time"]["calls"], 7);
    assert_eq!(report["totals"]["all_time"]["input_tokens"], 4400);
    assert_eq!(report["totals"]["all_time"]["output_tokens"], 1100);
    assert_eq!(report["totals"]["all_time"]["cached_tokens"], 850);
    assert_eq!(report["totals"]["all_time"]["unreported_calls"], 3);

    // by_provider_model checks.
    let pm = report["by_provider_model"].as_array().unwrap();
    assert_eq!(pm.len(), 6);
    let find_pm = |prov: &str, model: &str| {
        pm.iter().find(|r| r["provider"] == prov && r["model"] == model).unwrap_or_else(|| {
            panic!("missing ({prov}, {model}) in {pm:?}")
        })
    };
    let ant = find_pm("anthropic", "claude-sonnet-4-6");
    assert_eq!(ant["calls"], 1);
    assert_eq!(ant["input_tokens"], 1000);
    assert_eq!(ant["output_tokens"], 250);
    assert_eq!(ant["tokens_reported"], true);
    assert_eq!(ant["unreported_calls"], 0);

    let cli = find_pm("claude-cli", "sonnet");
    assert_eq!(cli["calls"], 2);
    assert_eq!(cli["input_tokens"], 0);
    assert_eq!(cli["output_tokens"], 0);
    assert_eq!(cli["tokens_reported"], false);
    assert_eq!(cli["unreported_calls"], 2);

    let agy = find_pm("antigravity", "agy-default");
    assert_eq!(agy["calls"], 1);
    assert_eq!(agy["tokens_reported"], false);
    assert_eq!(agy["unreported_calls"], 1);

    // by_day checks (4 distinct UTC dates, descending).
    let days = report["by_day"].as_array().unwrap();
    let expected_dates = vec![
        utc_date_for_ts(day_0),
        utc_date_for_ts(day_3),
        utc_date_for_ts(day_6),
        utc_date_for_ts(day_10),
    ];
    let got_dates: Vec<String> = days.iter().map(|d| d["date"].as_str().unwrap().to_string()).collect();
    assert_eq!(got_dates, expected_dates);
    assert_eq!(days[0]["calls"], 3);
    assert_eq!(days[0]["unreported_calls"], 2);
    assert_eq!(days[1]["calls"], 2);
    assert_eq!(days[1]["input_tokens"], 1000);
    assert_eq!(days[1]["unreported_calls"], 1);

    // by_session checks.
    let sessions = report["by_session"].as_array().unwrap();
    assert_eq!(sessions.len(), 4);
    let s_api = sessions.iter().find(|s| s["session_id"] == "s-api-mix").unwrap();
    assert_eq!(s_api["title"], "Spin Chain Perturbation");
    assert_eq!(s_api["mode"], "research");
    assert_eq!(s_api["calls"], 2);
    assert_eq!(s_api["input_tokens"], 1600);
    assert_eq!(s_api["output_tokens"], 400);
    assert_eq!(s_api["tokens_reported"], true);

    let s_solve = sessions.iter().find(|s| s["session_id"] == "s-solve-old").unwrap();
    assert_eq!(s_solve["mode"], "solve");
    assert_eq!(s_solve["calls"], 2);
    assert_eq!(s_solve["input_tokens"], 2800);
    assert_eq!(s_solve["output_tokens"], 700);

    let s_cli = sessions.iter().find(|s| s["session_id"] == "s-claude-cli").unwrap();
    assert_eq!(s_cli["calls"], 2);
    assert_eq!(s_cli["tokens_reported"], false);
    assert_eq!(s_cli["unreported_calls"], 2);

    // ?session_id=<id> filter & GET /api/sessions/{id} compact usage summary.
    let (st_f, _, raw_f) = raw_http(port, "GET", "/api/usage?session_id=s-claude-cli", &hdr, b"");
    assert_eq!(st_f, 200);
    let filt: Value = serde_json::from_slice(&raw_f).unwrap();
    assert_eq!(filt["session"]["session_id"], "s-claude-cli");
    assert_eq!(filt["session"]["calls"], 2);
    assert_eq!(filt["session"]["tokens_reported"], false);
    assert_eq!(filt["totals"]["all_time"]["calls"], 2);

    let (st_sess, _, raw_sess) = raw_http(port, "GET", "/api/sessions/s-api-mix", &hdr, b"");
    assert_eq!(st_sess, 200);
    let sess_json: Value = serde_json::from_slice(&raw_sess).unwrap();
    assert_eq!(sess_json["usage"]["calls"], 2);
    assert_eq!(sess_json["usage"]["input_tokens"], 1600);
    assert_eq!(sess_json["usage"]["output_tokens"], 400);
    assert_eq!(sess_json["usage"]["tokens_reported"], true);

    // Error handling for bad / nonexistent / deleted session_id.
    assert_eq!(raw_http(port, "GET", "/api/usage?session_id=../bad", &hdr, b"").0, 400);
    assert_eq!(raw_http(port, "GET", "/api/usage?session_id=s-does-not-exist", &hdr, b"").0, 404);
    assert_eq!(raw_http(port, "GET", "/api/usage?session_id=s-deleted", &hdr, b"").0, 404);

    // Single source of truth: no second usage database/cache file created in data_dir.
    let entries: Vec<String> = std::fs::read_dir(&data_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !entries.iter().any(|name| name.to_ascii_lowercase().contains("usage")),
        "must not create a separate usage DB/cache file in data_dir: {entries:?}"
    );

    srv.shutdown();
}

#[test]
fn usage_endpoint_enforces_authentication_and_cli_result_fallback() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append("s-cli-fallback", "session.start", 1, &json!({
        "provider": CLAUDE_CLI_ROUTE, "route": "claude-code-cli", "model": "sonnet"
    })).unwrap();
    ledger.append("s-cli-fallback", "cli.result", 1, &json!({
        "num_turns": 3, "usage": null, "total_cost_usd": 0.42
    })).unwrap();
    // Resumed turn with explicit model.response must not erase turn 1's cli.result count.
    ledger.append("s-cli-fallback", "session.resume", 1, &json!({
        "provider": "anthropic-messages", "model": "claude-sonnet-4-6"
    })).unwrap();
    ledger.append("s-cli-fallback", "model.response", 1, &json!({
        "turn": 1, "model": "claude-sonnet-4-6", "usage": { "input_tokens": 500, "output_tokens": 100 }
    })).unwrap();
    drop(ledger);

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let token = srv.info.token.clone();
    let cookie = launch_cookie(port, &token);

    // Unauthenticated, wrong token, cookie-only, and foreign-Host requests are rejected.
    let (st_no_auth, _, _) = raw_http(port, "GET", "/api/usage", &[], b"");
    assert!(matches!(st_no_auth, 401 | 403), "expected 401/403 without token, got {st_no_auth}");

    let (st_bad, _, _) = raw_http(port, "GET", "/api/usage", &[("X-ReSearch-Token", "wrong-token")], b"");
    assert!(matches!(st_bad, 401 | 403), "expected 401/403 with bad token, got {st_bad}");

    let (st_cookie, _, _) = raw_http(port, "GET", "/api/usage", &[("Cookie", cookie.as_str())], b"");
    assert!(matches!(st_cookie, 401 | 403), "cookie-only must be rejected on /api/usage, got {st_cookie}");

    let (st_host, _, _) = raw_http(
        port,
        "GET",
        "/api/usage",
        &[("X-ReSearch-Token", token.as_str()), ("Host", "evil.example")],
        b"",
    );
    assert_eq!(st_host, 403);

    // Valid token succeeds and counts both CLI cli.result turns and API model.response without cost fields.
    let (st_ok, _, raw_ok) = raw_http(port, "GET", "/api/usage", &[("X-ReSearch-Token", token.as_str())], b"");
    assert_eq!(st_ok, 200);
    let text = String::from_utf8(raw_ok).unwrap();
    assert!(!text.to_ascii_lowercase().contains("cost") && !text.contains("0.42"));
    let val: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(val["totals"]["all_time"]["calls"], 4);
    assert_eq!(val["totals"]["all_time"]["input_tokens"], 500);
    assert_eq!(val["totals"]["all_time"]["output_tokens"], 100);
    assert_eq!(val["totals"]["all_time"]["unreported_calls"], 3);
    let pm = val["by_provider_model"].as_array().unwrap();
    let cli_row = pm.iter().find(|r| r["provider"] == "claude-cli").unwrap();
    assert_eq!(cli_row["calls"], 3);
    assert_eq!(cli_row["tokens_reported"], false);
    let ant_row = pm.iter().find(|r| r["provider"] == "anthropic").unwrap();
    assert_eq!(ant_row["calls"], 1);
    assert_eq!(ant_row["tokens_reported"], true);

    srv.shutdown();
}
