//! Sign in with ChatGPT end to end against a loopback mock of OpenAI's auth server: the
//! authorize step (a browser) is skipped and the callback is called directly. Windows
//! tests exercise DPAPI storage; non-Windows tests prove fail-closed behavior with zero
//! disk writes.

#[cfg(not(windows))]
mod non_windows_flow {
    use chatgpt_auth::{ChatGptAuth, Options, TokenError};
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn non_windows_fails_closed_and_writes_nothing_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(ChatGptAuth::new(dir.path(), Options::default()));

        let st = auth.status();
        assert!(!st.supported && !st.signed_in && !st.plan_enabled);
        assert!(auth.start_login().is_err());
        assert_eq!(auth.access_token().unwrap_err(), TokenError::NotSupported);
        assert_eq!(auth.force_refresh("tok").unwrap_err(), TokenError::NotSupported);
        let gen0 = auth.login_generation();
        let out = auth.logout();
        assert_eq!(out.revoked, None);
        assert_eq!(auth.login_generation(), gen0 + 1);
        assert!(fs::read_dir(dir.path()).unwrap().next().is_none(), "non-Windows must write nothing to disk");
    }
}

#[cfg(windows)]
mod windows_flow {
    use chatgpt_auth::codec::pkce_challenge;
    use chatgpt_auth::testkit::{browser_get, browser_request, id_claims, jwks, sign_jwt, url_param, MockAuth, Recorded, Reply};
    use chatgpt_auth::vault;
    use chatgpt_auth::{ChatGptAuth, Endpoints, LoginState, Options, TokenError};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicUsize, Ordering};
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
        code_error: Mutex<Option<&'static str>>,
        omit_refresh_token: AtomicBool,
        revoke_status: AtomicU16,
        revoke_delay_ms: AtomicUsize,
        /// The vault's data directory, so the revoke handler can look at it.
        data_dir: Mutex<Option<std::path::PathBuf>>,
        /// Set when a revocation arrived while tokens or the cross-process lock still existed.
        revoke_saw_tokens: AtomicBool,
        revoke_saw_lock: AtomicBool,
    }

    fn revoke_reply(s: &Script) -> Reply {
        if let Some(d) = s.data_dir.lock().unwrap().clone() {
            let v = d.join(vault::DIR);
            s.revoke_saw_tokens.fetch_or(v.join("credentials.dpapi").exists(), Ordering::SeqCst);
            // The lock is an OS file lock on `.lock` (the file itself always stays): it is
            // held exactly when another handle cannot lock it.
            let held = std::fs::OpenOptions::new()
                .write(true)
                .open(v.join(".lock"))
                .is_ok_and(|f| matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
            s.revoke_saw_lock.fetch_or(held, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(s.revoke_delay_ms.load(Ordering::SeqCst) as u64));
        Reply { status: s.revoke_status.load(Ordering::SeqCst), content_type: "application/json", body: Vec::new() }
    }

    /// Make the saved access token due for a refresh: an hour-long token with 60 s left.
    /// (A freshly issued short token is no longer refreshed at once; see `refresh_due`.)
    fn make_due(env: &Env) {
        let mut c = vault::load_credentials(env._dir.path()).unwrap().unwrap();
        let now = chatgpt_auth::codec::now_s();
        c.saved_at_s = now - 3540;
        c.expires_at_s = now + 60;
        vault::save_credentials(env._dir.path(), &c).unwrap();
    }

    fn token_reply(s: &Script, r: &Recorded) -> Reply {
        match r.form("grant_type").as_deref() {
            Some("authorization_code") => {
                if let Some(code) = *s.code_error.lock().unwrap() {
                    return Reply::json(400, &json!({ "error": code }));
                }
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
                let mut body = json!({
                    "access_token": "acc-1-SECRETVALUE", "id_token": id,
                    "token_type": "Bearer", "expires_in": s.expires_in.load(Ordering::SeqCst),
                    "scope": s.scope.lock().unwrap().clone(), "earliest_refresh_at": 0
                });
                if !s.omit_refresh_token.load(Ordering::SeqCst) {
                    body["refresh_token"] = json!("rt-1-SECRETVALUE");
                }
                Reply::json(200, &body)
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
        script.revoke_status.store(200, Ordering::SeqCst);
        let s2 = Arc::clone(&script);
        let mock = MockAuth::start(Arc::new(move |r: &Recorded| match (r.method.as_str(), r.path.as_str()) {
            ("POST", "/api/accounts/oauth/token") => token_reply(&s2, r),
            ("GET", "/.well-known/jwks.json") => Reply::json(200, &jwks()),
            ("POST", "/api/accounts/oauth/revoke") => revoke_reply(&s2),
            _ => Reply::json(404, &json!({})),
        }));
        *script.issuer.lock().unwrap() = mock.base.clone();
        let dir = tempfile::tempdir().unwrap();
        *script.data_dir.lock().unwrap() = Some(dir.path().to_path_buf());
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
        assert_eq!(env.auth.login_generation(), 1);

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
            let reg = vault::load_registration(env._dir.path()).unwrap().unwrap();
            assert_eq!(reg.client_id, None, "{want}: failed ID token must leave client_id None");
        }
    }

    #[test]
    fn a_missing_plan_scope_keeps_the_sign_in_and_prompts_consent_only_to_reenable() {
        let env = setup(Duration::from_secs(60));
        *env.script.scope.lock().unwrap() = "email offline_access openid profile resource.invoke".into();
        let (url1, _, answer) = sign_in(&env, Some(CLIENT));
        assert!(url_param(&url1, "prompt").is_none());
        assert_eq!(answer.unwrap().0, 200);
        assert_eq!(env.auth.login_attempt().unwrap().state(), LoginState::Succeeded { plan_enabled: false });
        let st = env.auth.status();
        assert!(st.signed_in && !st.plan_enabled);
        assert_eq!(env.auth.access_token().unwrap_err(), TokenError::PlanNotEnabled);

        // Re-enabling after declined plan scope sends prompt=consent.
        *env.script.scope.lock().unwrap() = "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke".into();
        let (url2, _, answer2) = sign_in(&env, None);
        assert_eq!(url_param(&url2, "prompt").as_deref(), Some("consent"));
        assert_eq!(answer2.unwrap().0, 200);
        assert!(env.auth.status().plan_enabled);

        // Once plan scope is granted, subsequent sign-in omits prompt=consent.
        let next = env.auth.start_login().unwrap();
        assert!(url_param(&next.authorize_url, "prompt").is_none());
        env.auth.cancel_login();
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
    fn reauth_rejects_client_id_or_subject_mismatch() {
        let env = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);

        // Callback naming a different client_id is rejected before token exchange.
        let before_toks = env.mock.count("/api/accounts/oauth/token");
        let (_, _, ans) = sign_in(&env, Some("oaiapp_other"));
        assert_eq!(ans.unwrap().0, 400);
        assert!(matches!(env.auth.login_attempt().unwrap().state(), LoginState::Failed(m) if m.contains("different client")));
        assert_eq!(env.mock.count("/api/accounts/oauth/token"), before_toks);

        // Returning sign-in with a different `sub` is rejected and keeps existing credentials.
        *env.script.claims_patch.lock().unwrap() = Some(("sub".into(), json!("sub-different")));
        assert_eq!(env.mock.count("/api/accounts/oauth/revoke"), 0);
        let (_, _, ans2) = sign_in(&env, None);
        assert_eq!(ans2.unwrap().0, 400);
        assert!(matches!(env.auth.login_attempt().unwrap().state(), LoginState::Failed(m) if m.contains("different ChatGPT account")));
        assert_eq!(env.auth.access_token().unwrap().as_str(), "acc-1-SECRETVALUE");
        // The discarded exchange's refresh token was revoked (best effort), once.
        let rv: Vec<Recorded> = env.mock.requests().into_iter().filter(|r| r.path == "/api/accounts/oauth/revoke").collect();
        assert_eq!(rv.len(), 1);
        assert_eq!(rv[0].form("token_type_hint").as_deref(), Some("refresh_token"));
        assert_eq!(rv[0].form("client_id").as_deref(), Some(CLIENT));
        assert!(env.auth.status().signed_in, "the saved sign-in is kept");
    }

    #[test]
    fn invalid_client_on_refresh_and_code_exchange_clears_client_id_preserving_host_id() {
        // Case 1: invalid_client on refresh.
        let env = setup(Duration::from_secs(60));
        env.script.expires_in.store(60, Ordering::SeqCst);
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        make_due(&env);
        let host_id = vault::load_registration(env._dir.path()).unwrap().unwrap().ext_agent_host_id;
        *env.script.refresh_error.lock().unwrap() = Some("invalid_client");
        assert!(matches!(env.auth.access_token().unwrap_err(), TokenError::Disconnected(m) if m.contains("invalid_client")));
        let reg = vault::load_registration(env._dir.path()).unwrap().unwrap();
        assert_eq!(reg.ext_agent_host_id, host_id);
        assert_eq!(reg.client_id, None);
        assert_eq!(reg.subject, None);

        // Case 2: invalid_client on code exchange during reauth.
        let env2 = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env2, Some(CLIENT)).2.unwrap().0, 200);
        let host_id2 = vault::load_registration(env2._dir.path()).unwrap().unwrap().ext_agent_host_id;
        *env2.script.code_error.lock().unwrap() = Some("invalid_client");
        assert_eq!(sign_in(&env2, None).2.unwrap().0, 400);
        let reg2 = vault::load_registration(env2._dir.path()).unwrap().unwrap();
        assert_eq!(reg2.ext_agent_host_id, host_id2);
        assert_eq!(reg2.client_id, None);
        assert_eq!(reg2.subject, None);
    }

    #[test]
    fn missing_refresh_token_disconnects_and_dropped_plan_scope_disables_plan() {
        // Missing refresh_token on refresh disconnects cleanly.
        let env = setup(Duration::from_secs(60));
        env.script.expires_in.store(60, Ordering::SeqCst);
        env.script.omit_refresh_token.store(true, Ordering::SeqCst);
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        make_due(&env);
        assert!(matches!(env.auth.access_token().unwrap_err(), TokenError::Disconnected(m) if m.contains("no refresh token")));
        assert!(!env.auth.status().signed_in);

        // Plan scope dropped on refresh returns PlanNotEnabled.
        let env2 = setup(Duration::from_secs(60));
        env2.script.expires_in.store(60, Ordering::SeqCst);
        assert_eq!(sign_in(&env2, Some(CLIENT)).2.unwrap().0, 200);
        make_due(&env2);
        *env2.script.scope.lock().unwrap() = "email offline_access openid profile resource.invoke".into();
        assert_eq!(env2.auth.access_token().unwrap_err(), TokenError::PlanNotEnabled);
        let st = env2.auth.status();
        assert!(st.signed_in && !st.plan_enabled);
    }

    #[test]
    fn force_refresh_rotates_and_shares_if_already_rotated() {
        let env = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        assert_eq!(env.auth.access_token().unwrap().as_str(), "acc-1-SECRETVALUE");
        let rotated = env.auth.force_refresh("acc-1-SECRETVALUE").unwrap();
        assert_eq!(rotated.as_str(), "acc-2-SECRETVALUE");
        assert_eq!(env.script.refreshes.load(Ordering::SeqCst), 1);
        // Second caller reporting the same failed token gets the already-rotated token without another refresh.
        let shared = env.auth.force_refresh("acc-1-SECRETVALUE").unwrap();
        assert_eq!(shared.as_str(), "acc-2-SECRETVALUE");
        assert_eq!(env.script.refreshes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn refresh_rotates_tokens_once_for_concurrent_callers() {
        let env = setup(Duration::from_secs(60));
        env.script.expires_in.store(60, Ordering::SeqCst);
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        make_due(&env); // inside the refresh margin
        env.script.refresh_delay_ms.store(300, Ordering::SeqCst);
        let tokens: Vec<String> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..4).map(|_| s.spawn(|| env.auth.access_token().map(|t| t.as_str().to_string()))).collect();
            hs.into_iter().map(|h| h.join().unwrap().unwrap()).collect()
        });
        let refresh_reqs: Vec<Recorded> =
            env.mock.requests().into_iter().filter(|r| r.form("grant_type").as_deref() == Some("refresh_token")).collect();
        // One refresh: the rotated 60 s token is not itself due (it has more than half its
        // lifetime left), so the other three callers share it instead of refreshing again.
        assert_eq!(refresh_reqs.len(), 1);
        for (i, r) in refresh_reqs.iter().enumerate() {
            assert_eq!(r.form("refresh_token"), Some(format!("rt-{}-SECRETVALUE", i + 1)), "refresh {i}");
            assert_eq!(r.form("client_id").as_deref(), Some(CLIENT));
            assert_eq!(r.form("resource").as_deref(), Some("https://api.openai.com/v1"));
            assert!(r.form("scope").is_none());
        }
        assert_eq!(tokens.len(), 4);

        env.script.expires_in.store(3600, Ordering::SeqCst);
        make_due(&env);
        let before = env.script.refreshes.load(Ordering::SeqCst);
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
        make_due(&env);
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
        make_due(&env);
        *env.script.refresh_error.lock().unwrap() = Some("temporarily_unavailable");
        assert!(matches!(env.auth.access_token().unwrap_err(), TokenError::Temporary(_)));
        assert!(env.auth.status().signed_in);
    }

    #[test]
    fn logout_revokes_retries_on_500_deletes_tokens_and_keeps_registration() {
        let env = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        let out = env.auth.logout();
        assert_eq!(out.revoked, Some(true), "{}", out.message);
        let rv = env.mock.requests().into_iter().find(|r| r.path == "/api/accounts/oauth/revoke").unwrap();
        assert_eq!(rv.form("token").as_deref(), Some("rt-1-SECRETVALUE"));
        assert_eq!(rv.form("token_type_hint").as_deref(), Some("refresh_token"));
        assert_eq!(rv.form("client_id").as_deref(), Some(CLIENT));
        // Tokens were deleted first, and the revocation ran outside the cross-process lock.
        assert!(!env.script.revoke_saw_tokens.load(Ordering::SeqCst), "tokens must be deleted before revoking");
        assert!(!env.script.revoke_saw_lock.load(Ordering::SeqCst), "revocation must not hold the vault lock");
        assert!(!env.auth.status().signed_in);
        assert_eq!(env.auth.access_token().unwrap_err(), TokenError::NotSignedIn);
        let next = env.auth.start_login().unwrap();
        assert_eq!(url_param(&next.authorize_url, "client_id").as_deref(), Some(CLIENT));
        assert_no_secret(&[&out.message]);

        // Revocation retries on 500 and reports unconfirmed when all 3 fail.
        let env2 = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env2, Some(CLIENT)).2.unwrap().0, 200);
        env2.script.revoke_status.store(500, Ordering::SeqCst);
        let out2 = env2.auth.logout();
        assert_eq!(out2.revoked, Some(false), "{}", out2.message);
        assert_eq!(env2.mock.count("/api/accounts/oauth/revoke"), 3);
        assert_eq!(env2.mock.count("/.well-known/openid-configuration"), 1, "discovery runs once per sign-out");
        assert!(!env2.auth.status().signed_in);
        assert_no_secret(&[&out2.message]);
    }

    #[test]
    fn a_slow_sign_out_blocks_neither_status_nor_other_callers_and_stays_within_its_budget() {
        let mut env = setup(Duration::from_secs(60));
        assert_eq!(sign_in(&env, Some(CLIENT)).2.unwrap().0, 200);
        env.script.revoke_delay_ms.store(5_000, Ordering::SeqCst);
        // Rebuild the manager on the same vault with a 1.5 s sign-out budget.
        let opts = Options {
            endpoints: Endpoints::loopback_mock(&env.mock.base).unwrap(),
            revoke_backoff: Duration::from_millis(10),
            revoke_budget: Duration::from_millis(1500),
            ..Options::default()
        };
        env.auth = Arc::new(ChatGptAuth::new(env._dir.path(), opts));
        let auth = Arc::clone(&env.auth);
        let t0 = std::time::Instant::now();
        let h = std::thread::spawn(move || auth.logout());
        while env.mock.count("/api/accounts/oauth/revoke") == 0 && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(env.mock.count("/api/accounts/oauth/revoke"), 1, "revocation is in flight");
        let t1 = std::time::Instant::now();
        let st = env.auth.status();
        assert!(t1.elapsed() < Duration::from_millis(500), "status waited {:?}", t1.elapsed());
        assert!(!st.signed_in, "tokens are already deleted while revocation runs");
        assert_eq!(env.auth.access_token().unwrap_err(), TokenError::NotSignedIn);
        assert!(t1.elapsed() < Duration::from_millis(1000), "access_token waited {:?}", t1.elapsed());
        let out = h.join().unwrap();
        assert_eq!(out.revoked, Some(false), "{}", out.message);
        assert!(t0.elapsed() < Duration::from_millis(3500), "sign-out took {:?}", t0.elapsed());
    }
}
