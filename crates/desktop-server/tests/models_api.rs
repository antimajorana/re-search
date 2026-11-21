//! `GET /api/connections` and `GET /api/models`: token required; live lists from loopback
//! mocks of all four API providers (one failing, one oversized); the Gemini key only in a
//! header; the OpenAI chat filter; the cache, `refresh=1` and invalidation on a key change;
//! keys never echoed; the Claude CLI's aliases only when its executable is found (the
//! scripted fake, asked only for `--version`); which credential header each mock host
//! received (only each provider's own key, never the generic one unless
//! RE_SEARCH_PROVIDER names the provider, none under a launch --api-base, none but
//! Ollama's own in the Ollama probe); list errors as a status class only. No real
//! network, CLI or model.

mod common;

use cli_route::testkit::FakeCli;
use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, DesktopConfig, RunningDesktopServer};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const ANTHROPIC_KEY: &str = "sk-ant-test-0001";
const OPENAI_KEY: &str = "sk-openai-test-0002";
const GEMINI_KEY: &str = "gemini-test-key-0003";

#[derive(Clone, Debug)]
struct Seen {
    line: String,
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Loopback mock that answers every request on `path` with `status` + `body` and records
/// each request line and its headers.
struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    fn start(path: &'static str, status: u16, body: Vec<u8>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let mut headers = Vec::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" || h == "\n" {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                    }
                }
                let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(Seen { line: line.trim().to_string(), headers });
                let (st, b): (u16, &[u8]) = if target.split('?').next() == Some(path) { (status, &body) } else { (404, b"{}") };
                let mut s = stream;
                let head = format!(
                    "HTTP/1.1 {st} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    b.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(b);
                let _ = s.flush();
            }
        });
        Mock { base, seen }
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// A loopback port nothing listens on.
fn closed_base() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    format!("http://{addr}")
}

struct Srv {
    _tmp: TempDir,
    srv: RunningDesktopServer,
    port: u16,
    tok: String,
}

impl Srv {
    fn start(tmp: TempDir, edit: impl FnOnce(&mut DesktopConfig)) -> Srv {
        let mut cfg = test_cfg(&tmp.path().join("data"), None, None, None);
        cfg.claude_exe = Some(tmp.path().join("no-such-dir").join("claude.exe"));
        edit(&mut cfg);
        let srv = start_desktop_server(cfg).unwrap();
        let port = srv.addr.port();
        let tok = srv.info.token.clone();
        Srv { _tmp: tmp, srv, port, tok }
    }

    fn get(&self, path: &str) -> (u16, Value, String) {
        let (st, _, b) = raw_http(self.port, "GET", path, &[("X-ReSearch-Token", self.tok.as_str())], b"");
        let text = String::from_utf8_lossy(&b).to_string();
        (st, serde_json::from_slice(&b).unwrap_or(Value::Null), text)
    }

    fn put_settings(&self, body: Value) {
        let hdrs = [("X-ReSearch-Token", self.tok.as_str()), ("Content-Type", "application/json")];
        let (st, _, b) = raw_http(self.port, "PUT", "/api/settings/providers", &hdrs, body.to_string().as_bytes());
        assert_eq!(st, 200, "{}", String::from_utf8_lossy(&b));
    }

    fn group<'a>(models: &'a Value, id: &str) -> Option<&'a Value> {
        models["providers"].as_array().unwrap().iter().find(|g| g["id"] == id)
    }

    fn connection<'a>(conns: &'a Value, id: &str) -> &'a Value {
        conns["connections"].as_array().unwrap().iter().find(|c| c["id"] == id).expect(id)
    }
}

fn ids(group: &Value) -> Vec<String> {
    group["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect()
}

fn defaults(group: &Value) -> Vec<String> {
    group["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["default"] == true)
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn both_routes_need_the_token() {
    let s = Srv::start(TempDir::new().unwrap(), |_| {});
    for path in ["/api/connections", "/api/models", "/api/models?refresh=1"] {
        assert_eq!(raw_http(s.port, "GET", path, &[], b"").0, 401, "{path}");
        assert_eq!(raw_http(s.port, "GET", path, &[("X-ReSearch-Token", "0000")], b"").0, 401, "{path}");
        assert_eq!(s.get(path).0, 200, "{path}");
    }
    s.srv.shutdown();
}

#[test]
fn live_lists_from_all_four_providers_with_failures_isolated() {
    let anthropic = Mock::start(
        "/v1/models",
        500,
        json!({ "error": { "type": "api_error", "message": format!("key {ANTHROPIC_KEY} rejected") } }).to_string().into_bytes(),
    );
    let openai = Mock::start(
        "/v1/models",
        200,
        json!({ "data": [
            { "id": "gpt-old", "created": 100 },
            { "id": "text-embedding-3-small", "created": 400 },
            { "id": "whisper-1", "created": 300 },
            { "id": "gpt-new", "created": 200 },
            { "id": "gpt-4.1-mini", "created": 150 }
        ]})
        .to_string()
        .into_bytes(),
    );
    let gemini = Mock::start(
        "/v1beta/models",
        200,
        json!({ "models": [
            { "name": "models/gemini-a", "displayName": "<b>Gemini A</b>", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/embedding-001", "supportedGenerationMethods": ["embedContent"] }
        ]})
        .to_string()
        .into_bytes(),
    );
    // Larger than the 4 MiB list limit.
    let ollama = Mock::start("/api/tags", 200, vec![b' '; 5 * 1024 * 1024]);

    let s = Srv::start(TempDir::new().unwrap(), |_| {});
    s.put_settings(json!({
        "keys": { "anthropic": ANTHROPIC_KEY, "openai": OPENAI_KEY, "gemini": GEMINI_KEY },
        "base_urls": { "anthropic": anthropic.base, "openai": format!("{}/v1", openai.base),
                       "gemini": gemini.base, "ollama": format!("{}/v1", ollama.base) },
    }));

    let (st, models, text) = s.get("/api/models");
    assert_eq!(st, 200, "{text}");
    for key in [ANTHROPIC_KEY, OPENAI_KEY, GEMINI_KEY] {
        assert!(!text.contains(key), "a key leaked into /api/models: {text}");
    }
    assert_eq!(models["cache_ttl_s"], 600);
    // No Claude CLI here, and Ollama never answered within limits: neither is offered.
    assert!(Srv::group(&models, "claude-cli").is_none(), "{models}");
    assert!(Srv::group(&models, "ollama").is_none(), "{models}");

    // OpenAI: chat models only, newest first, one default (re:Search's default is listed).
    let o = Srv::group(&models, "openai").expect("openai group");
    assert_eq!(ids(o), ["gpt-new", "gpt-4.1-mini", "gpt-old"]);
    assert_eq!(defaults(o), ["gpt-4.1-mini"]);
    assert_eq!(o["stale"], false);
    assert!(o["error"].is_null());
    assert_eq!(o["models"][0]["provider"], "openai");
    let oreq = openai.requests();
    assert_eq!(oreq.len(), 1);
    assert!(oreq[0].line.starts_with("GET /v1/models "), "{:?}", oreq[0]);
    assert_eq!(oreq[0].header("authorization"), Some(format!("Bearer {OPENAI_KEY}").as_str()));

    // Gemini: generateContent models only; the label is passed through as text (the UI
    // escapes it); the key travels in the header, never in the URL.
    let g = Srv::group(&models, "gemini").expect("gemini group");
    assert_eq!(ids(g), ["gemini-a"]);
    assert_eq!(g["models"][0]["label"], "<b>Gemini A</b>");
    assert_eq!(defaults(g), ["gemini-a"]);
    for r in gemini.requests() {
        assert!(!r.line.contains(GEMINI_KEY) && !r.line.contains("key="), "{}", r.line);
        assert!(r.line.starts_with("GET /v1beta/models?pageSize=1000 "), "{}", r.line);
        assert_eq!(r.header("x-goog-api-key"), Some(GEMINI_KEY));
    }

    // Anthropic failed: an error (key scrubbed), a built-in list marked stale.
    let a = Srv::group(&models, "anthropic").expect("anthropic group kept despite the failure");
    assert_eq!(a["stale"], true);
    assert_eq!(a["source"], "fallback");
    let err = a["error"].as_str().unwrap();
    assert!(err.contains("500"), "{err}");
    assert!(!err.contains(ANTHROPIC_KEY), "{err}");
    assert_eq!(ids(a), ["claude-haiku-4-5"]);
    let areq = anthropic.requests();
    assert_eq!(areq[0].header("x-api-key"), Some(ANTHROPIC_KEY));
    assert_eq!(areq[0].header("anthropic-version"), Some("2023-06-01"));

    // Connections: never a key, the source of each, Ollama not answering (oversized).
    let (st, conns, text) = s.get("/api/connections");
    assert_eq!(st, 200, "{text}");
    for key in [ANTHROPIC_KEY, OPENAI_KEY, GEMINI_KEY] {
        assert!(!text.contains(key), "a key leaked into /api/connections: {text}");
    }
    let c_open = Srv::connection(&conns, "openai");
    assert_eq!(c_open["connected"], true);
    assert_eq!(c_open["kind"], "api-key");
    assert_eq!(c_open["key_source"], "saved key");
    assert_eq!(c_open["has_saved_key"], true);
    assert!(c_open["how_to"].is_null());
    let c_anth = Srv::connection(&conns, "anthropic");
    assert_eq!(c_anth["connected"], true);
    assert!(c_anth["status"].as_str().unwrap().contains("model list failed"), "{c_anth}");
    let c_oll = Srv::connection(&conns, "ollama");
    assert_eq!(c_oll["connected"], false, "{c_oll}");
    assert!(c_oll["status"].as_str().unwrap().contains("oversized"), "{c_oll}");
    assert!(c_oll["how_to"].as_str().unwrap().contains("Ollama"));
    let c_cli = Srv::connection(&conns, "claude-cli");
    assert_eq!(c_cli["connected"], false);
    assert_eq!(c_cli["kind"], "subscription");
    assert_eq!(c_cli["how_to"], "Install Claude Code and sign in once by running claude in a terminal.");
    s.srv.shutdown();
}

#[test]
fn lists_are_cached_refreshed_on_request_and_dropped_when_the_key_changes() {
    let openai = Mock::start("/v1/models", 200, json!({ "data": [{ "id": "gpt-x", "created": 1 }] }).to_string().into_bytes());
    let ollama = Mock::start(
        "/api/tags",
        200,
        json!({ "models": [{ "name": "llama3:8b", "modified_at": "2024-05-01T10:00:00Z" }] }).to_string().into_bytes(),
    );
    let s = Srv::start(TempDir::new().unwrap(), |_| {});
    s.put_settings(json!({ "keys": { "openai": OPENAI_KEY }, "base_urls": { "openai": openai.base, "ollama": ollama.base } }));

    let (_, first, _) = s.get("/api/models");
    assert_eq!(ids(Srv::group(&first, "openai").unwrap()), ["gpt-x"]);
    let l = Srv::group(&first, "ollama").expect("ollama answered");
    assert_eq!(ids(l), ["llama3:8b"]);
    assert_eq!(l["kind"], "api-key");
    let (_, second, _) = s.get("/api/models");
    assert_eq!(Srv::group(&second, "openai").unwrap()["source"], "cache");
    assert_eq!(openai.requests().len(), 1, "second request served from the cache");
    assert_eq!(ollama.requests().len(), 1);

    let (_, third, _) = s.get("/api/models?refresh=1");
    assert_eq!(Srv::group(&third, "openai").unwrap()["source"], "live");
    assert_eq!(openai.requests().len(), 2, "refresh=1 bypasses the cache");
    assert_eq!(ollama.requests().len(), 2);

    // A new key invalidates the cache, and the next list uses the new key.
    s.put_settings(json!({ "keys": { "openai": "sk-openai-new-0009" } }));
    s.get("/api/models");
    let reqs = openai.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[2].header("authorization"), Some("Bearer sk-openai-new-0009"));

    // Removing the key disconnects the provider: its group disappears.
    s.put_settings(json!({ "keys": { "openai": null } }));
    let (_, after, _) = s.get("/api/models");
    assert!(Srv::group(&after, "openai").is_none(), "{after}");
    let (_, conns, _) = s.get("/api/connections");
    assert_eq!(Srv::connection(&conns, "openai")["connected"], false);
    assert_eq!(Srv::connection(&conns, "ollama")["connected"], true);
    s.srv.shutdown();
}

#[test]
fn environment_key_is_reported_by_name_and_used_for_the_list() {
    let openai = Mock::start("/v1/models", 200, json!({ "data": [{ "id": "gpt-env", "created": 1 }] }).to_string().into_bytes());
    let closed = closed_base();
    let s = Srv::start(TempDir::new().unwrap(), |cfg| {
        cfg.connection_env = Some(BTreeMap::from([("OPENAI_API_KEY".to_string(), "sk-from-env-0004".to_string())]));
    });
    s.put_settings(json!({ "base_urls": { "openai": openai.base, "ollama": closed } }));
    let (_, conns, text) = s.get("/api/connections");
    assert!(!text.contains("sk-from-env-0004"));
    let c = Srv::connection(&conns, "openai");
    assert_eq!(c["connected"], true);
    assert_eq!(c["key_source"], "environment");
    assert_eq!(c["key_env"], "OPENAI_API_KEY");
    assert_eq!(c["has_saved_key"], false);
    let (_, models, text) = s.get("/api/models");
    assert!(!text.contains("sk-from-env-0004"));
    assert_eq!(ids(Srv::group(&models, "openai").unwrap()), ["gpt-env"]);
    assert_eq!(openai.requests()[0].header("authorization"), Some("Bearer sk-from-env-0004"));
    s.srv.shutdown();
}

#[test]
fn claude_cli_aliases_only_when_the_cli_is_found() {
    let closed = closed_base();

    // Found: connected, version from `--version` (once per launch), the aliases offered.
    let tmp = TempDir::new().unwrap();
    let fake = FakeCli::new(tmp.path(), "fake-cli");
    let exe = fake.exe.clone();
    let s = Srv::start(tmp, |cfg| cfg.claude_exe = Some(exe));
    s.put_settings(json!({ "base_urls": { "ollama": closed } }));
    let (_, conns, _) = s.get("/api/connections");
    let c = Srv::connection(&conns, "claude-cli");
    assert_eq!(c["connected"], true, "{c}");
    assert_eq!(c["version"], "2.1.318");
    assert!(c["status"].as_str().unwrap().contains("2.1.318"), "{c}");
    assert_eq!(c["session_note"], "runs Claude Code (your login)");
    assert_eq!(c["accepts_key"], false);
    fake.write("version.txt", "9.9.9 (Claude Code)");
    let (_, conns, _) = s.get("/api/connections");
    assert_eq!(Srv::connection(&conns, "claude-cli")["version"], "2.1.318", "version is read once per launch");
    let (_, models, _) = s.get("/api/models");
    let g = Srv::group(&models, "claude-cli").expect("claude-cli group");
    assert_eq!(models["providers"][0]["id"], "claude-cli", "listed first");
    assert_eq!(ids(g), ["fable", "opus", "sonnet", "haiku"]);
    assert_eq!(defaults(g), ["sonnet"]);
    assert_eq!(g["kind"], "subscription");
    assert_eq!(g["models"][1]["label"], "Claude Opus (latest)");
    assert_eq!(g["models"][1]["provider"], "claude-cli");
    assert!(fake.argv().is_none(), "the CLI was never started with a prompt");
    s.srv.shutdown();

    // Not found: not connected, a how-to, no aliases (and nothing else is connected).
    let s = Srv::start(TempDir::new().unwrap(), |_| {});
    s.put_settings(json!({ "base_urls": { "ollama": closed } }));
    let (_, models, _) = s.get("/api/models");
    assert_eq!(models["providers"], json!([]), "{models}");
    s.srv.shutdown();

    // Found, but an environment credential would bill instead of the subscription.
    let tmp = TempDir::new().unwrap();
    let fake = FakeCli::new(tmp.path(), "fake-cli");
    let exe = fake.exe.clone();
    let s = Srv::start(tmp, |cfg| {
        cfg.claude_exe = Some(exe);
        cfg.connection_env = Some(BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "dummy-value-0005".to_string())]));
    });
    // The same variable makes the Anthropic API look connected: keep its list call local.
    s.put_settings(json!({ "base_urls": { "anthropic": closed, "ollama": closed } }));
    let (_, conns, text) = s.get("/api/connections");
    assert!(!text.contains("dummy-value-0005"), "the value is never shown");
    let c = Srv::connection(&conns, "claude-cli");
    assert_eq!(c["connected"], false);
    assert!(c["status"].as_str().unwrap().contains("ANTHROPIC_API_KEY"), "{c}");
    assert!(c["how_to"].as_str().unwrap().contains("RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH"), "{c}");
    let (_, models, _) = s.get("/api/models");
    assert!(Srv::group(&models, "claude-cli").is_none(), "{models}");
    s.srv.shutdown();
}

fn list_body(provider: &str, id: &str) -> Vec<u8> {
    match provider {
        "gemini" => json!({ "models": [{ "name": format!("models/{id}"), "supportedGenerationMethods": ["generateContent"] }] }),
        "ollama" => json!({ "models": [{ "name": id }] }),
        _ => json!({ "data": [{ "id": id, "created": 1 }] }),
    }
    .to_string()
    .into_bytes()
}

/// Every credential header a mock received.
fn auth_headers(m: &Mock) -> Vec<String> {
    m.requests()
        .iter()
        .flat_map(|r| ["authorization", "x-api-key", "x-goog-api-key"].map(|h| r.header(h).map(str::to_string)))
        .flatten()
        .collect()
}

#[test]
fn list_calls_use_only_each_providers_own_key() {
    const GENERIC: &str = "sk-generic-0010";
    const OWN_OPENAI: &str = "sk-openai-own-0011";
    let anthropic = Mock::start("/v1/models", 200, list_body("anthropic", "claude-x"));
    let openai = Mock::start("/v1/models", 200, list_body("openai", "gpt-x"));
    let gemini = Mock::start("/v1beta/models", 200, list_body("gemini", "gemini-x"));
    let ollama = Mock::start("/api/tags", 200, list_body("ollama", "llama3:8b"));
    let s = Srv::start(TempDir::new().unwrap(), |cfg| {
        cfg.connection_env = Some(BTreeMap::from([
            ("RE_SEARCH_API_KEY".to_string(), GENERIC.to_string()),
            // Ollama's transport falls back to OPENAI_API_KEY; its list probe must not.
            ("OPENAI_API_KEY".to_string(), OWN_OPENAI.to_string()),
        ]));
    });
    s.put_settings(json!({ "base_urls": { "anthropic": anthropic.base, "openai": openai.base,
                                          "gemini": gemini.base, "ollama": ollama.base } }));
    let (_, models, text) = s.get("/api/models");
    assert!(!text.contains(GENERIC) && !text.contains(OWN_OPENAI));

    // OpenAI: its own variable, not the generic key a turn would read first.
    assert_eq!(auth_headers(&openai), [format!("Bearer {OWN_OPENAI}")]);
    assert_eq!(ids(Srv::group(&models, "openai").unwrap()), ["gpt-x"]);
    // Anthropic and Gemini have only the generic key: no list call, no group.
    assert!(anthropic.requests().is_empty(), "{:?}", anthropic.requests());
    assert!(gemini.requests().is_empty(), "{:?}", gemini.requests());
    assert!(Srv::group(&models, "anthropic").is_none() && Srv::group(&models, "gemini").is_none(), "{models}");
    // Ollama is probed with no key at all.
    assert!(!ollama.requests().is_empty());
    assert!(auth_headers(&ollama).is_empty(), "{:?}", ollama.requests());
    assert_eq!(ids(Srv::group(&models, "ollama").unwrap()), ["llama3:8b"]);

    let (_, conns, text) = s.get("/api/connections");
    assert!(!text.contains(GENERIC) && !text.contains(OWN_OPENAI));
    assert!(anthropic.requests().is_empty() && gemini.requests().is_empty());
    assert!(auth_headers(&ollama).is_empty());
    let a = Srv::connection(&conns, "anthropic");
    assert_eq!(a["connected"], false, "{a}");
    let status = a["status"].as_str().unwrap();
    assert!(status.contains("RE_SEARCH_API_KEY") && status.contains("RE_SEARCH_PROVIDER=anthropic"), "{status}");
    // What a turn would use is still reported truthfully.
    let o = Srv::connection(&conns, "openai");
    assert_eq!((o["connected"].as_bool(), o["key_env"].as_str()), (Some(true), Some("RE_SEARCH_API_KEY")), "{o}");
    s.srv.shutdown();
}

#[test]
fn generic_key_lists_only_the_provider_named_by_re_search_provider() {
    const GENERIC: &str = "sk-generic-0012";
    let anthropic = Mock::start("/v1/models", 200, list_body("anthropic", "claude-x"));
    let openai = Mock::start("/v1/models", 200, list_body("openai", "gpt-x"));
    let gemini = Mock::start("/v1beta/models", 200, list_body("gemini", "gemini-x"));
    let closed = closed_base();
    let s = Srv::start(TempDir::new().unwrap(), |cfg| {
        cfg.connection_env = Some(BTreeMap::from([
            ("RE_SEARCH_API_KEY".to_string(), GENERIC.to_string()),
            ("RE_SEARCH_PROVIDER".to_string(), "anthropic".to_string()),
        ]));
    });
    s.put_settings(json!({ "base_urls": { "anthropic": anthropic.base, "openai": openai.base,
                                          "gemini": gemini.base, "ollama": closed } }));
    let (_, models, _) = s.get("/api/models");
    assert_eq!(auth_headers(&anthropic), [GENERIC.to_string()]);
    assert_eq!(ids(Srv::group(&models, "anthropic").unwrap()), ["claude-x"]);
    assert!(openai.requests().is_empty() && gemini.requests().is_empty());
    let (_, conns, _) = s.get("/api/connections");
    assert_eq!(Srv::connection(&conns, "anthropic")["connected"], true);
    assert_eq!(Srv::connection(&conns, "openai")["connected"], false);
    s.srv.shutdown();
}

#[test]
fn launch_api_base_makes_no_list_call_with_any_key() {
    let host = Mock::start("/v1/models", 200, list_body("openai", "gpt-x"));
    let base = host.base.clone();
    let s = Srv::start(TempDir::new().unwrap(), |cfg| {
        cfg.api_base_override = Some(base);
        cfg.connection_env = Some(BTreeMap::from([("OLLAMA_API_KEY".to_string(), "ollama-key-0013".to_string())]));
    });
    s.put_settings(json!({ "keys": { "anthropic": ANTHROPIC_KEY, "openai": OPENAI_KEY, "gemini": GEMINI_KEY } }));
    let (_, models, _) = s.get("/api/models?refresh=1");
    let (_, conns, _) = s.get("/api/connections?refresh=1");
    assert!(host.requests().is_empty(), "nothing sent to the override host: {:?}", host.requests());
    for id in ["anthropic", "openai", "gemini"] {
        let g = Srv::group(&models, id).expect(id);
        assert_eq!((g["source"].as_str(), g["stale"].as_bool()), (Some("fallback"), Some(true)), "{g}");
        assert!(g["error"].as_str().unwrap().contains("--api-base"), "{g}");
        let c = Srv::connection(&conns, id);
        assert_eq!(c["connected"], true, "{c}");
        assert!(c["status"].as_str().unwrap().contains("model list not fetched"), "{c}");
    }
    assert!(Srv::group(&models, "ollama").is_none());
    let l = Srv::connection(&conns, "ollama");
    assert_eq!(l["connected"], false);
    assert!(l["status"].as_str().unwrap().starts_with("Not checked"), "{l}");
    s.srv.shutdown();
}

#[test]
fn a_rejected_key_is_reported_by_status_class_only() {
    let body = json!({ "error": { "message": "Incorrect API key provided: sk-proj-****abcd. You can find your API key at https://platform.openai.com/account/api-keys.", "code": "invalid_api_key" } });
    let openai = Mock::start("/v1/models", 401, body.to_string().into_bytes());
    let closed = closed_base();
    let s = Srv::start(TempDir::new().unwrap(), |_| {});
    s.put_settings(json!({ "keys": { "openai": OPENAI_KEY }, "base_urls": { "openai": openai.base, "ollama": closed } }));
    let (_, models, mtext) = s.get("/api/models");
    let (_, conns, ctext) = s.get("/api/connections");
    for text in [&mtext, &ctext] {
        for leak in ["sk-proj", "****", "abcd", "Incorrect", "invalid_api_key", OPENAI_KEY] {
            assert!(!text.contains(leak), "{leak} reached the UI: {text}");
        }
    }
    assert_eq!(Srv::group(&models, "openai").unwrap()["error"], "auth (HTTP 401)");
    let status = Srv::connection(&conns, "openai")["status"].as_str().unwrap().to_string();
    assert!(status.ends_with("the model list failed: auth (HTTP 401)"), "{status}");
    s.srv.shutdown();
}
