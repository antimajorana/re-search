//! Sign in with ChatGPT end to end against a loopback mock of OpenAI's auth server: the
//! authorize step (a browser) is skipped and the callback is called directly. Windows
//! only: tokens are stored with DPAPI, and other systems refuse to store them.
#![cfg(windows)]

use chatgpt_auth::codec::pkce_challenge;
use chatgpt_auth::testkit::{browser_get, browser_request, id_claims, jwks, sign_jwt, url_param, MockAuth, Recorded, Reply};
use chatgpt_auth::{ChatGptAuth, Endpoints, LoginState, Options, TokenError};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CLIENT: &str = "oaiapp_test_client";

#[derive(Default)]
struct Script {
    issuer: Mutex<String>,
    nonce: Mutex<String>,
    scope: Mutex<String>,
    expires_in: AtomicI64,
    claims_patch: Mutex<Option<(String, Value)>>,
    tamper_sig: AtomicBool,
    refreshes: AtomicUsize,
    refresh_delay_ms: AtomicUsize,
    refresh_error: Mutex<Option<&'static str>>,
}

fn token_reply(s: &Script, r: &Recorded) -> Reply {
    match r.form("grant_type").as_deref() {
        Some("authorization_code") => {
            let client = r.form("client_id").unwrap_or_default();
            let exp = chatgpt_auth::codec::now_s() + 3600;
            let mut claims = id_claims(&s.issuer.lock().unwrap(), &client, &s.nonce.lock().unwrap(), "sub-123", exp);
            if let Some((k, v)) = s.claims_patch.lock().unwrap().clone() {
                claims[k.as_str()] = v;
            }
            let mut id = sign_jwt(&claims);
            if s.tamper_sig.load(Ordering::SeqCst) {
                let (head, sig) = id.rsplit_once('.').unwrap();
                let flipped = if sig.starts_with('A') { "B" } else { "A" };
                id = format!("{head}.{flipped}{}", &sig[1..]);
            }
            Reply::json(200, &json!({
                "access_token": "acc-1-SECRETVALUE", "refresh_token": "rt-1-SECRETVALUE", "id_token": id,
                "token_type": "Bearer", "expires_in": s.expires_in.load(Ordering::SeqCst),
                "scope": s.scope.lock().unwrap().clone(), "earliest_refresh_at": 0
            }))
        }
        Some("refresh_token") => {
            std::thread::sleep(Duration::from_millis(s.refresh_delay_ms.load(Ordering::SeqCst) as u64));
            if let Some(code) = *s.refresh_error.lock().unwrap() {
                return Reply::json(400, &json!({ "error": code, "error_description": "rt-1-SECRETVALUE is bad" }));
            }
            let n = s.refreshes.fetch_add(1, Ordering::SeqCst) + 2;
            Reply::json(200, &json!({
                "access_token": format!("acc-{n}-SECRETVALUE"), "refresh_token": format!("rt-{n}-SECRETVALUE"),
                "token_type": "Bearer", "expires_in": s.expires_in.load(Ordering::SeqCst),
                "scope": s.scope.lock().unwrap().clone()
            }))
        }
        _ => Reply::json(400, &json!({ "error": "unsupported_grant_type" })),
    }
}

struct Env {
    _dir: tempfile::TempDir,
    auth: Arc<ChatGptAuth>,
    mock: MockAuth,
    script: Arc<Script>,
}

fn setup(login_timeout: Duration) -> Env {
    let script = Arc::new(Script::default());
    *script.scope.lock().unwrap() = "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke".into();
    script.expires_in.store(3600, Ordering::SeqCst);
    let s2 = Arc::clone(&script);
    let mock = MockAuth::start(Arc::new(move |r: &Recorded| match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/accounts/oauth/token") => token_reply(&s2, r),
        ("GET", "/.well-known/jwks.json") => Reply::json(200, &jwks()),
        ("POST", "/api/accounts/oauth/revoke") => Reply { status: 200, content_type: "application/json", body: Vec::new() },
        _ => Reply::json(404, &json!({})),
    }));
    *script.issuer.lock().unwrap() = mock.base.clone();
    let dir = tempfile::tempdir().unwrap();
    let opts = Options {
        endpoints: Endpoints::loopback_mock(&mock.base).unwrap(),
        login_timeout,
        revoke_backoff: Duration::from_millis(10),
        ..Options::default()
    };
    let auth = Arc::new(ChatGptAuth::new(dir.path(), opts));
    Env { _dir: dir, auth, mock, script }
}

/// Start a sign-in and send the browser's callback; returns the authorize URL, the
/// listener port and the callback's answer.
fn sign_in(env: &Env, client_in_callback: Option<&str>) -> (String, u16, Option<(u16, String)>) {
    let start = env.auth.start_login().unwrap();
    let url = start.authorize_url.clone();
    *env.script.nonce.lock().unwrap() = url_param(&url, "nonce").unwrap();
    let state = url_param(&url, "state").unwrap();
    let mut q = format!("/auth/callback?code=code-abc&scope=chatgpt.tokens.use.direct+openid&state={state}");
    if let Some(c) = client_in_callback {
        q.push_str(&format!("&client_id={c}"));
    }
    let answer = browser_get(start.port, &q);
    (url, start.port, answer)
}

fn assert_no_secret(texts: &[&str]) {
    for t in texts {
        assert!(!t.contains("SECRETVALUE") && !t.contains("code-abc"), "secret material in {t:?}");
    }
}

#[test]
fn first_sign_in_registers_exchanges_validates_and_stores() {
    let env = setup(Duration::from_secs(60));
    let (url, port, answer) = sign_in(&env, Some(CLIENT));
    let (status, body) = answer.expect("callback answered");
    assert_eq!(status, 200, "{body}");
    assert_no_secret(&[&body]);
    assert_eq!(env.auth.login_attempt().unwrap().state(), LoginState::Succeeded { plan_enabled: true });

    // Authorize URL: documented names, dynamic registration with the app name.
    assert_eq!(url_param(&url, "client_id").as_deref(), Some("dynamic_agent_client"));
    assert_eq!(url_param(&url, "agent_name_hint").as_deref(), Some("re:Search"));
    assert_eq!(url_param(&url, "redirect_uri"), Some(format!("http://127.0.0.1:{port}/auth/callback")));
    assert_eq!(url_param(&url, "resource").as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(url_param(&url, "code_challenge_method").as_deref(), Some("S256"));
    let host_id = url_param(&url, "ext_agent_host_id").unwrap();
    assert!(host_id.starts_with("urn:uuid:"));

    // Code exchange: issued client ID, PKCE verifier matching the challenge, no secret.
    let ex = env.mock.requests().into_iter().find(|r| r.path == "/api/accounts/oauth/token").unwrap();
    assert_eq!(ex.header("content-type"), Some("application/x-www-form-urlencoded"));
    assert_eq!(ex.form("client_id").as_deref(), Some(CLIENT));
    assert_eq!(ex.form("code").as_deref(), Some("code-abc"));
    assert_eq!(ex.form("redirect_uri"), url_param(&url, "redirect_uri"));
    assert_eq!(ex.form("resource").as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(pkce_challenge(&ex.form("code_verifier").unwrap()), url_param(&url, "code_challenge").unwrap());
    assert!(ex.form("client_secret").is_none());

    let st = env.auth.status();
    assert!(st.signed_in && st.plan_enabled, "{st:?}");
    assert_eq!(st.account.as_deref(), Some("person@example.test"));
    assert_eq!(env.auth.access_token().unwrap().as_str(), "acc-1-SECRETVALUE");
    assert_no_secret(&[&st.message, &env.auth.login_view().1]);

    // The listener is single-use: a replayed callback finds nothing listening.
    let state = url_param(&url, "state").unwrap();
    assert!(browser_get(port, &format!("/auth/callback?code=code-abc&state={state}")).is_none(), "replay must fail");
    assert_eq!(env.mock.count("/api/accounts/oauth/token"), 1);

    // The next sign-in reuses the issued client ID and the same host ID, without the hint.
    let (url2, _, answer2) = sign_in(&env, None);
    assert_eq!(answer2.unwrap().0, 200);
    assert_eq!(url_param(&url2, "client_id").as_deref(), Some(CLIENT));
    assert_eq!(url_param(&url2, "ext_agent_host_id"), Some(host_id));
    assert!(url_param(&url2, "agent_name_hint").is_none());
}

#[test]
fn wrong_state_path_method_or_host_is_refused_and_the_attempt_stays_open() {
    let env = setup(Duration::from_secs(60));
    let start = env.auth.start_login().unwrap();
    let p = start.port;
    assert_eq!(browser_get(p, "/auth/callback?code=x&state=forged").unwrap().0, 400);
    assert_eq!(browser_get(p, "/callback?code=x&state=forged").unwrap().0, 404);
    assert_eq!(browser_request(p, "POST", "/auth/callback?code=x", &format!("127.0.0.1:{p}")).unwrap().0, 404);
    assert_eq!(browser_request(p, "GET", "/auth/callback?code=x", "evil.example").unwrap().0, 404);
    assert!(env.auth.login_attempt().unwrap().is_pending());
    assert_eq!(env.mock.count("/api/accounts/oauth/token"), 0, "nothing exchanged");
    assert!(!env.auth.status().signed_in);
}

#[test]
fn an_expired_listener_closes() {
    let env = setup(Duration::from_millis(300));
    let start = env.auth.start_login().unwrap();
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(env.auth.login_attempt().unwrap().state(), LoginState::Expired);
    let state = url_param(&start.authorize_url, "state").unwrap();
    assert!(browser_get(start.port, &format!("/auth/callback?code=c&state={state}")).is_none());
    assert!(env.auth.pending_authorize_url().is_none());
}

#[test]
fn id_token_failures_store_nothing() {
    type Arm = Box<dyn Fn(&Script)>;
    let cases: Vec<(&str, Arm)> = vec![
        ("signature", Box::new(|s: &Script| s.tamper_sig.store(true, Ordering::SeqCst))),
        ("client ID", Box::new(|s: &Script| *s.claims_patch.lock().unwrap() = Some(("aud".into(), json!(["oaiapp_someone_else"]))))),
        ("nonce", Box::new(|s: &Script| *s.claims_patch.lock().unwrap() = Some(("nonce".into(), json!("not-the-nonce"))))),
        ("expired", Box::new(|s: &Script| *s.claims_patch.lock().unwrap() = Some(("exp".into(), json!(1_000_000))))),
    ];
    for (want, arm) in cases {
        let env = setup(Duration::from_secs(60));
        arm(&env.script);
        let (_, _, answer) = sign_in(&env, Some(CLIENT));
        assert_eq!(answer.unwrap().0, 400, "{want}");
        match env.auth.login_attempt().unwrap().state() {
            LoginState::Failed(m) => {
                assert!(m.contains(want), "{want}: {m}");
                assert_no_secret(&[&m]);
            }
            other => panic!("{want}: {other:?}"),
        }
        assert!(!env.auth.status().signed_in, "{want}");
        assert_eq!(env.auth.access_token().unwrap_err(), TokenError::NotSignedIn);
    }
}

#[test]
fn a_missing_plan_scope_keeps_the_sign_in_but_never_yields_a_token() {
    let env = setup(Duration::from_secs(60));
    *env.script.scope.lock().unwrap() = "email offline_access openid profile resource.invoke".into();
    let (_, _, answer) = sign_in(&env, Some(CLIENT));
    assert_eq!(answer.unwrap().0, 200);
    assert_eq!(env.auth.login_attempt().unwrap().state(), LoginState::Succeeded { plan_enabled: false });
    let st = env.auth.status();
    assert!(st.signed_in && !st.plan_enabled);
    assert_eq!(env.auth.access_token().unwrap_err(), TokenError::PlanNotEnabled);
}

#[test]
fn a_declined_consent_or_missing_client_id_fails_without_exchange() {
    let env = setup(Duration::from_secs(60));
    let start = env.auth.start_login().unwrap();
    let state = url_param(&start.authorize_url, "state").unwrap();
    assert_eq!(browser_get(start.port, &format!("/auth/callback?error=access_denied&state={state}")).unwrap().0, 400);
    assert!(matches!(env.auth.login_attempt().unwrap().state(), LoginState::Failed(m) if m.contains("access_denied")));
    let (_, _, answer) = sign_in(&env, None);
    assert_eq!(answer.unwrap().0, 400);
    assert!(matches!(env.auth.login_attempt().unwrap().state(), LoginState::Failed(m) if m.contains("issued client ID")));
    assert_eq!(env.mock.count("/api/accounts/oauth/token"), 0);
}

#[test]
fn refresh_rotates_tokens_once_for_concurrent_callers() {
    let env = setup(Duration::from_secs(60));
    env.script.expires_in.store(60, Ordering::SeqCst); // inside the refresh margin
    assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
    env.script.refresh_delay_ms.store(300, Ordering::SeqCst);
    let tokens: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| env.auth.access_token().map(|t| t.as_str().to_string()))).collect();
        hs.into_iter().map(|h| h.join().unwrap().unwrap()).collect()
    });
    // One refresh; every caller after it got the new token... which again expires within the
    // margin, so callers that came later refreshed again in turn, never in parallel.
    let refresh_reqs: Vec<Recorded> =
        env.mock.requests().into_iter().filter(|r| r.form("grant_type").as_deref() == Some("refresh_token")).collect();
    assert!(!refresh_reqs.is_empty());
    for (i, r) in refresh_reqs.iter().enumerate() {
        // Each refresh used the refresh token the previous one returned (rotation).
        assert_eq!(r.form("refresh_token"), Some(format!("rt-{}-SECRETVALUE", i + 1)), "refresh {i}");
        assert_eq!(r.form("client_id").as_deref(), Some(CLIENT));
        assert_eq!(r.form("resource").as_deref(), Some("https://api.openai.com/v1"));
        assert!(r.form("scope").is_none());
    }
    assert_eq!(tokens.len(), 4);

    // With a long-lived answer, concurrent callers share exactly one refresh.
    env.script.expires_in.store(3600, Ordering::SeqCst);
    let before = env.script.refreshes.load(Ordering::SeqCst);
    // Force the next call to refresh: the stored token still expires within the margin.
    let tokens: Vec<String> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| env.auth.access_token().unwrap().as_str().to_string())).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(env.script.refreshes.load(Ordering::SeqCst), before + 1, "one refresh for four callers");
    let expected = format!("acc-{}-SECRETVALUE", before + 2);
    assert!(tokens.iter().all(|t| *t == expected), "{tokens:?}");
}

#[test]
fn a_terminal_refresh_error_disconnects() {
    let env = setup(Duration::from_secs(60));
    env.script.expires_in.store(60, Ordering::SeqCst);
    assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
    *env.script.refresh_error.lock().unwrap() = Some("invalid_grant");
    match env.auth.access_token().unwrap_err() {
        TokenError::Disconnected(m) => {
            assert!(m.contains("invalid_grant"), "{m}");
            assert_no_secret(&[&m]);
        }
        other => panic!("{other:?}"),
    }
    let st = env.auth.status();
    assert!(!st.signed_in);
    assert!(st.message.contains("Disconnected"), "{}", st.message);
    assert_eq!(env.auth.access_token().unwrap_err(), TokenError::NotSignedIn);

    // A temporary failure keeps the sign-in.
    let env = setup(Duration::from_secs(60));
    env.script.expires_in.store(60, Ordering::SeqCst);
    assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
    *env.script.refresh_error.lock().unwrap() = Some("temporarily_unavailable");
    assert!(matches!(env.auth.access_token().unwrap_err(), TokenError::Temporary(_)));
    assert!(env.auth.status().signed_in);
}

#[test]
fn logout_revokes_deletes_tokens_and_keeps_the_registration() {
    let env = setup(Duration::from_secs(60));
    assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
    let out = env.auth.logout();
    assert_eq!(out.revoked, Some(true), "{}", out.message);
    let rv = env.mock.requests().into_iter().find(|r| r.path == "/api/accounts/oauth/revoke").unwrap();
    assert_eq!(rv.form("token").as_deref(), Some("rt-1-SECRETVALUE"));
    assert_eq!(rv.form("token_type_hint").as_deref(), Some("refresh_token"));
    assert_eq!(rv.form("client_id").as_deref(), Some(CLIENT));
    assert!(!env.auth.status().signed_in);
    assert_eq!(env.auth.access_token().unwrap_err(), TokenError::NotSignedIn);
    let next = env.auth.start_login().unwrap();
    assert_eq!(url_param(&next.authorize_url, "client_id").as_deref(), Some(CLIENT));
    assert_no_secret(&[&out.message]);
}
