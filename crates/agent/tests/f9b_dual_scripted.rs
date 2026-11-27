//! Offline dual scripted-transport acceptance tests for F9b Core
//! (`RunBudgetPool`, strict `role_routing`, honest `usage`, and partial-worker safety).
//!
//! Uses loopback `127.0.0.1` HTTP mocks only — zero live network calls, zero `eval_support` edits.

use agent::role_routing::{
    build_role_transport, resolve_role, resolve_run_roles, ResolvedRole, RoleModelInput,
    RoleRoutingInputs, RoleRoutingPolicy, RoutingReason, ROLE_COMPUTE_BLIND, ROLE_ORCHESTRATOR,
    ROLE_PAPER_READER,
};
use agent::transport::{
    ContentBlock, Message, ModelRequest, ModelResponse, NoopStreamObserver,
    ProviderKind, Role, StreamObserver, Transport,
};
use agent::usage::{extract_usage, inject_openai_stream_usage, RoleUsageRollup};
use agent::ProviderKeyStore;
use ledger::Ledger;
use research_tools::budget_pool::{
    ExhaustedScope, PoolCaller, RunBudgetPool, DEFAULT_FRONTIER_RESERVE,
};
use research_tools::run_budget::{count_model_calls, RunScope};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
struct CapturedHttpReq {
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

type ReplyHandler = Arc<dyn Fn(usize, &CapturedHttpReq) -> (u16, Vec<u8>) + Send + Sync>;

struct LoopbackProvider {
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedHttpReq>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LoopbackProvider {
    fn start(handler: ReplyHandler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (cap_t, stop_t) = (Arc::clone(&captured), Arc::clone(&stop));
        let seq = Arc::new(AtomicUsize::new(0));

        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stop_t.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let (cap_c, h_c, seq_c) =
                            (Arc::clone(&cap_t), Arc::clone(&handler), Arc::clone(&seq));
                        workers.push(thread::spawn(move || {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                            let mut reader = BufReader::new(stream.try_clone().unwrap());
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                                return;
                            }
                            let mut hdrs = BTreeMap::new();
                            let mut len = 0usize;
                            loop {
                                line.clear();
                                if reader.read_line(&mut line).unwrap_or(0) == 0
                                    || line == "\r\n"
                                    || line == "\n"
                                {
                                    break;
                                }
                                if let Some((k, v)) = line.trim_end().split_once(':') {
                                    let kl = k.trim().to_ascii_lowercase();
                                    let vl = v.trim().to_string();
                                    if kl == "content-length" {
                                        len = vl.parse().unwrap_or(0);
                                    }
                                    hdrs.insert(kl, vl);
                                }
                            }
                            let mut body = vec![0u8; len];
                            if len > 0 {
                                let _ = reader.read_exact(&mut body);
                            }
                            let req = CapturedHttpReq { headers: hdrs, body };
                            let idx = seq_c.fetch_add(1, Ordering::SeqCst);
                            cap_c.lock().unwrap().push(req.clone());
                            let (status, resp_body) = h_c(idx, &req);
                            let head = format!(
                                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                resp_body.len()
                            );
                            let _ = stream.write_all(head.as_bytes());
                            let _ = stream.write_all(&resp_body);
                            let _ = stream.flush();
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            captured,
            stop,
            handle: Some(handle),
        }
    }

    fn requests(&self) -> Vec<CapturedHttpReq> {
        self.captured.lock().unwrap().clone()
    }
}

impl Drop for LoopbackProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

struct BudgetedTransport {
    inner: Box<dyn Transport>,
    pool: RunBudgetPool,
    caller: PoolCaller,
}

impl BudgetedTransport {
    fn send_with_budget(
        &self,
        req: &ModelRequest,
        observer: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, String> {
        let prepared = self.inner.prepare_request(req).map_err(|e| e.to_string())?;
        let lease = self
            .pool
            .try_acquire(self.caller.clone())
            .map_err(|e| e.to_string())?;
        let _committed = lease.mark_sent();
        self.inner
            .send_prepared_streaming(&prepared, observer)
            .map_err(|e| e.to_string())
    }
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("f9b-dual-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn simple_req(model: &str, text: &str) -> ModelRequest {
    ModelRequest {
        model: model.to_string(),
        system: Some("You are a test model.".to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.to_string() }],
        }],
        tools: Vec::new(),
        max_tokens: 512,
        reasoning_effort: None,
    }
}

fn ant_reply(model: &str, stop_reason: &str, text: &str, usage: Value) -> (u16, Vec<u8>) {
    let body = json!({
        "id": "msg_f9b",
        "type": "message",
        "role": "assistant",
        "model": model,
        "stop_reason": stop_reason,
        "content": [{ "type": "text", "text": text }],
        "usage": usage
    });
    (200, serde_json::to_vec(&body).unwrap())
}

fn oai_reply(model: &str, text: &str, usage: Value) -> (u16, Vec<u8>) {
    let body = json!({
        "id": "chat_f9b",
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": usage
    });
    (200, serde_json::to_vec(&body).unwrap())
}

#[test]
fn dual_scripted_barrier_parallel_budget_scoped_keys_and_honest_usage() {
    // 1. Start two distinct loopback transports: frontier (Anthropic) vs worker (OpenAI).
    let frontier_srv = LoopbackProvider::start(Arc::new(|idx, _| {
        if idx == 0 {
            ant_reply(
                "claude-sonnet-4-6",
                "end_turn",
                "Plan: dispatch readers.",
                json!({ "input_tokens": 300, "output_tokens": 40 }),
            )
        } else {
            ant_reply(
                "claude-sonnet-4-6",
                "end_turn",
                "Final synthesis from bounded evidence.",
                json!({ "input_tokens": 220, "output_tokens": 60 }),
            )
        }
    }));

    // Barrier forces the 3 admitted worker threads to overlap in flight concurrently.
    let worker_barrier = Arc::new(Barrier::new(3));
    let worker_peak = Arc::new(AtomicUsize::new(0));
    let worker_in_flight = Arc::new(AtomicUsize::new(0));
    let wb = Arc::clone(&worker_barrier);
    let wp = Arc::clone(&worker_peak);
    let wi = Arc::clone(&worker_in_flight);
    let worker_srv = LoopbackProvider::start(Arc::new(move |idx, _| {
        let cur = wi.fetch_add(1, Ordering::SeqCst) + 1;
        wp.fetch_max(cur, Ordering::SeqCst);
        if idx < 3 {
            wb.wait();
        }
        wi.fetch_sub(1, Ordering::SeqCst);
        match idx {
            // Call 0: purely missing usage (null) -> must report null, never 0.
            0 => oai_reply("gpt-4.1-mini", "Found lemma 1 (lines 12-20).", Value::Null),
            // Call 1: Value::Null input_tokens with prompt_tokens fallback -> recovers 42, output null.
            1 => oai_reply(
                "gpt-4.1-mini",
                "Found definition 2 (lines 30-45).",
                json!({ "input_tokens": null, "prompt_tokens": 42, "output_tokens": null }),
            ),
            // Call 2: HTTP 500 failure after HTTP send -> still counted once in budget and rollup!
            _ => (500, br#"{"error":"upstream overloaded"}"#.to_vec()),
        }
    }));

    // 2. Configure provider-scoped keys and custom base_urls (F-9 isolation test).
    let mut store = ProviderKeyStore::default();
    store.set_key(ProviderKind::OpenAi, Some("sk-oai-worker-secret".into())).unwrap();
    store.set_base_url(ProviderKind::Anthropic, Some(frontier_srv.base_url.clone())).unwrap();
    store.set_base_url(ProviderKind::OpenAi, Some(worker_srv.base_url.clone())).unwrap();

    let mut env = BTreeMap::new();
    env.insert("RE_SEARCH_PROVIDER".into(), "anthropic".into());
    env.insert("RE_SEARCH_API_KEY".into(), "sk-ant-frontier-secret".into());

    let mut inputs = RoleRoutingInputs {
        store,
        env: Some(env),
        ..Default::default()
    };
    inputs.agent_models.insert(
        ROLE_PAPER_READER.into(),
        RoleModelInput::new("openai", "gpt-4.1-mini"),
    );

    let policy = RoleRoutingPolicy::new(ProviderKind::Anthropic, "claude-sonnet-4-6");
    let resolved = resolve_run_roles(&[ROLE_ORCHESTRATOR, ROLE_PAPER_READER], &policy, &inputs);
    assert!(resolved[ROLE_ORCHESTRATOR].is_pinned());
    assert!(resolved[ROLE_PAPER_READER].is_pinned());

    let (frontier_tx, frontier_model) =
        build_role_transport(&resolved[ROLE_ORCHESTRATOR], &inputs, None).unwrap();
    let (worker_tx, worker_model) =
        build_role_transport(&resolved[ROLE_PAPER_READER], &inputs, None).unwrap();
    assert_eq!(frontier_model, "claude-sonnet-4-6");
    assert_eq!(worker_model, "gpt-4.1-mini");

    // 3. Setup ledger + RunBudgetPool (cap = 6, frontier_reserve = 2).
    let tmp = TempDir::new("main");
    let ledger = Ledger::open(&tmp.0).unwrap();
    let sid = "s-f9b-dual";
    ledger
        .append_with_hash(
            sid,
            "session.start",
            1,
            &json!({ "provider": "anthropic", "model": "claude-sonnet-4-6" }),
        )
        .unwrap();

    let pool = RunBudgetPool::from_events(&[], RunScope::All, 6, DEFAULT_FRONTIER_RESERVE, 0);
    let frontier_budgeted = BudgetedTransport {
        inner: frontier_tx,
        pool: pool.clone(),
        caller: PoolCaller::frontier(),
    };

    // Frontier initial call (call 1 of 6) -> leaves 3 worker permits + 2 frontier reserve.
    let f1_req = simple_req(&frontier_model, "Plan reading");
    let f1_lease = pool.try_acquire(PoolCaller::frontier()).unwrap();
    f1_lease.mark_sent();
    let f1_prep = frontier_budgeted.inner.prepare_request(&f1_req).unwrap();
    let f1_resp = frontier_budgeted
        .inner
        .send_prepared_streaming(&f1_prep, &mut NoopStreamObserver)
        .unwrap();
    ledger.append_with_hash(sid, "model.request", 1, &json!({ "turn": 1, "model": f1_resp.model })).unwrap();
    ledger
        .append_with_hash(
            sid,
            "model.response",
            1,
            &json!({ "turn": 1, "model": f1_resp.model, "usage": f1_resp.usage }),
        )
        .unwrap();

    // Local prepare_request error before send must drop lease and refund slot!
    {
        let bad_req = simple_req("", "empty model fails locally");
        assert!(worker_tx.prepare_request(&bad_req).is_err());
        let check_lease = pool
            .try_acquire(PoolCaller::worker("paper_reader", Some("r-local".into())))
            .unwrap();
        drop(check_lease);
    }
    assert_eq!(pool.snapshot().available_for_workers, 3);

    // 4. Launch 6 parallel worker threads competing for 3 remaining worker permits.
    let worker_shared: Arc<dyn Transport> = Arc::from(worker_tx);
    let start_barrier = Arc::new(Barrier::new(6));
    let worker_results: Mutex<Vec<(usize, Result<ModelResponse, String>)>> =
        Mutex::new(Vec::new());
    let worker_rejected = AtomicUsize::new(0);

    thread::scope(|s| {
        for idx in 0..6 {
            let wtx = Arc::clone(&worker_shared);
            let p = pool.clone();
            let sb = Arc::clone(&start_barrier);
            let res_box = &worker_results;
            let rej_cnt = &worker_rejected;
            let wm = worker_model.clone();
            s.spawn(move || {
                sb.wait();
                let req = simple_req(&wm, &format!("Read slice {idx}"));
                let prep = wtx.prepare_request(&req).unwrap();
                let lease = match p.try_acquire(PoolCaller::worker("paper_reader", Some(format!("s{idx}")))) {
                    Ok(l) => l,
                    Err(err) => {
                        assert_eq!(err.scope, ExhaustedScope::ReservedHeadroom);
                        rej_cnt.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                };
                lease.mark_sent();
                let outcome = wtx
                    .send_prepared_streaming(&prep, &mut NoopStreamObserver)
                    .map_err(|e| e.to_string());
                res_box.lock().unwrap().push((idx, outcome));
            });
        }
    });

    assert_eq!(worker_rejected.load(Ordering::SeqCst), 3);
    assert_eq!(worker_peak.load(Ordering::SeqCst), 3, "3 worker calls must run concurrently");
    let worker_reqs = worker_srv.requests();
    assert_eq!(worker_reqs.len(), 3, "rejected threads must send 0 HTTP bytes");
    for req in &worker_reqs {
        assert_eq!(
            req.headers.get("authorization").map(String::as_str),
            Some("Bearer sk-oai-worker-secret"),
            "worker must use OpenAI key, never Anthropic generic env key"
        );
        let wire: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(wire["model"], "gpt-4.1-mini");
        assert!(wire.get("stream_options").is_none(), "non-streaming HTTP mock must not get stream_options");
    }

    // Record partial worker outcomes in ledger (2 succeeded with evidence, 1 failed HTTP 500).
    let outcomes = worker_results.into_inner().unwrap();
    let ok_count = outcomes.iter().filter(|(_, r)| r.is_ok()).count();
    let err_count = outcomes.iter().filter(|(_, r)| r.is_err()).count();
    assert_eq!((ok_count, err_count), (2, 1));

    ledger
        .append_with_hash(
            sid,
            "delegate.start",
            1,
            &json!({
                "call_id": "del-pr-1", "role": "paper_reader",
                "provider": "openai", "model": "gpt-4.1-mini"
            }),
        )
        .unwrap();
    for (turn_idx, (_, res)) in outcomes.iter().enumerate() {
        match res {
            Ok(resp) => {
                let blob = ledger.put_blob(&resp.raw_body).unwrap();
                ledger
                    .append_with_hash(
                        sid,
                        "delegate.turn",
                        1,
                        &json!({
                            "call_id": "del-pr-1", "role": "paper_reader",
                            "turn": turn_idx + 1, "model": resp.model,
                            "usage": resp.usage, "raw_blob": blob
                        }),
                    )
                    .unwrap();
            }
            Err(err) => {
                // Post-send failed attempt still counts once!
                ledger
                    .append_with_hash(
                        sid,
                        "delegate.turn",
                        1,
                        &json!({
                            "call_id": "del-pr-1", "role": "paper_reader",
                            "turn": turn_idx + 1, "model": "gpt-4.1-mini",
                            "usage": null, "error": err
                        }),
                    )
                    .unwrap();
            }
        }
    }
    let out_blob = ledger.put_blob(b"partial surviving evidence from 2 slices").unwrap();
    ledger
        .append_with_hash(
            sid,
            "delegate.end",
            1,
            &json!({
                "call_id": "del-pr-1", "worker": "paper_reader",
                "provider": "openai", "model": "gpt-4.1-mini",
                "ok": false, "turns": 3, "raw_blob": out_blob
            }),
        )
        .unwrap();

    // 5. Frontier closing pass still completes using protected frontier_reserve = 2!
    let f2 = frontier_budgeted
        .send_with_budget(&simple_req(&frontier_model, "Synthesize partial"), &mut NoopStreamObserver)
        .unwrap();
    ledger.append_with_hash(sid, "model.request", 1, &json!({ "turn": 2, "model": f2.model })).unwrap();
    ledger
        .append_with_hash(
            sid,
            "model.response",
            1,
            &json!({ "turn": 2, "model": f2.model, "usage": f2.usage }),
        )
        .unwrap();

    let f_reqs = frontier_srv.requests();
    assert_eq!(f_reqs.len(), 2);
    assert_eq!(
        f_reqs[0].headers.get("x-api-key").map(String::as_str),
        Some("sk-ant-frontier-secret")
    );

    // 6. Verify ledger call count and honest usage rollup.
    let events = ledger.read_session(sid).unwrap();
    assert_eq!(count_model_calls(&events, RunScope::All), 5); // 2 frontier + 3 worker
    let rollup = RoleUsageRollup::from_events(&events).to_json();
    let roles = rollup["roles"].as_array().unwrap();
    let reader_row = roles.iter().find(|r| r["role"] == "paper_reader").unwrap();
    assert_eq!(reader_row["calls"], 3);
    assert_eq!(reader_row["input_tokens"], 42);
    assert!(reader_row["output_tokens"].is_null(), "missing output tokens stay null: {reader_row}");
    assert_eq!(reader_row["unknown_usage_calls"], 3);

    ledger.verify_session(sid).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn unavailable_and_disabled_roles_fail_closed_and_never_hit_frontier() {
    let frontier_srv = LoopbackProvider::start(Arc::new(|_, _| {
        panic!("frontier server must never receive worker traffic on unavailable/disabled roles")
    }));
    let mut store = ProviderKeyStore::default();
    store.set_key(ProviderKind::Anthropic, Some("sk-ant".into())).unwrap();
    store.set_base_url(ProviderKind::Anthropic, Some(frontier_srv.base_url.clone())).unwrap();

    let mut inputs = RoleRoutingInputs {
        store,
        env: Some(BTreeMap::new()),
        ..Default::default()
    };
    inputs.agent_models.insert(
        ROLE_COMPUTE_BLIND.into(),
        RoleModelInput::new("openai", "gpt-4.1-mini"),
    );

    // Missing OpenAI credentials -> Unavailable, build_role_transport refuses.
    let policy = RoleRoutingPolicy::new(ProviderKind::Anthropic, "claude-haiku-4-5");
    let blind = resolve_role(ROLE_COMPUTE_BLIND, &policy, &inputs);
    assert!(blind.is_unavailable());
    let build_err = build_role_transport(&blind, &inputs, None).err().unwrap();
    assert!(build_err.contains("unavailable"));

    // Unassigned worker on smallest-tier primary without opt-in -> Disabled.
    let reader = resolve_role(ROLE_PAPER_READER, &policy, &inputs);
    assert_eq!(reader, ResolvedRole::Disabled { reason: "no_smaller_model_configured" });
    assert!(build_role_transport(&reader, &inputs, None).is_err());

    // Explicit opt-in yields ExplicitFrontierOptIn (still never silent!).
    let opt_in = policy.with_frontier_fallback(true);
    assert_eq!(
        resolve_role(ROLE_PAPER_READER, &opt_in, &inputs),
        ResolvedRole::Pinned {
            provider: ProviderKind::Anthropic,
            model: "claude-haiku-4-5".into(),
            base_url: None,
            routing_reason: RoutingReason::ExplicitFrontierOptIn,
        }
    );
    assert!(frontier_srv.requests().is_empty());
}

#[test]
fn openai_stream_usage_injection_and_null_usage_extraction_end_to_end() {
    // Check OpenAI streaming flag injection stays strictly scoped to OpenAI + stream=true.
    let base_body = serde_json::to_vec(&json!({ "model": "gpt-4.1-mini", "stream": true })).unwrap();
    let with_usage = inject_openai_stream_usage(base_body.clone(), ProviderKind::OpenAi, true);
    let parsed: Value = serde_json::from_slice(&with_usage).unwrap();
    assert_eq!(parsed["stream_options"]["include_usage"], json!(true));
    assert_eq!(inject_openai_stream_usage(base_body.clone(), ProviderKind::Anthropic, true), base_body);

    // Purely unknown usage across all calls serializes null in RoleUsageRollup.
    let u_null = extract_usage(&json!({ "input_tokens": null, "output_tokens": null }));
    assert!(u_null.is_unknown());
    let mut rollup = RoleUsageRollup::new();
    rollup.record_call("paper_reader", "ollama", "qwen2.5:7b", u_null);
    let out = rollup.to_json();
    assert!(out["roles"][0]["input_tokens"].is_null());
    assert!(out["roles"][0]["output_tokens"].is_null());
    assert_eq!(out["roles"][0]["unknown_usage_calls"], 1);
    assert!(out["totals"]["input_tokens"].is_null());
}
