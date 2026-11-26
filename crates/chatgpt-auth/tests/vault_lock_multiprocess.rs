//! Real two-process tests for `<data_dir>/chatgpt/.lock` (`VaultLock`) and cross-process
//! DPAPI token serialization (`ChatGptAuth::access_token` refresh & fail-closed `logout`).

use chatgpt_auth::vault::{self, VaultLock};
use std::fs;
#[cfg(windows)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ENV_MODE: &str = "CHATGPT_LOCK_CHILD_MODE";
const ENV_DIR: &str = "CHATGPT_LOCK_DATA_DIR";
const ENV_MOCK: &str = "CHATGPT_LOCK_MOCK_BASE";

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn spawn(mode: &str, data_dir: &Path, mock_base: Option<&str>) -> Self {
        let exe = std::env::current_exe().expect("current test binary path");
        let mut cmd = Command::new(exe);
        cmd.args(["--exact", "vault_lock_subprocess_entrypoint", "--nocapture"])
            .env(ENV_MODE, mode)
            .env(ENV_DIR, data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(b) = mock_base {
            cmd.env(ENV_MOCK, b);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        Self(Some(cmd.spawn().expect("spawn lock helper subprocess")))
    }

    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("live child")
    }

    fn kill_and_wait(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn wait_clean(mut self) -> std::process::ExitStatus {
        let mut c = self.0.take().expect("live child");
        c.wait().expect("wait child")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

#[test]
fn vault_lock_subprocess_entrypoint() {
    let Ok(mode) = std::env::var(ENV_MODE) else {
        return;
    };
    let dir = std::env::var_os(ENV_DIR).expect("CHATGPT_LOCK_DATA_DIR");
    let dir_path = Path::new(&dir);
    match mode.as_str() {
        "hold" => {
            let _lock = VaultLock::acquire(dir_path).expect("child acquire VaultLock");
            println!("LOCKED");
            std::io::stdout().flush().unwrap();
            let mut buf = [0u8; 1];
            let _ = std::io::stdin().read(&mut buf);
        }
        #[cfg(windows)]
        "refresh" => {
            let base = std::env::var(ENV_MOCK).expect("CHATGPT_LOCK_MOCK_BASE");
            let opts = chatgpt_auth::Options {
                endpoints: chatgpt_auth::Endpoints::loopback_mock(&base).unwrap(),
                ..chatgpt_auth::Options::default()
            };
            let auth = chatgpt_auth::ChatGptAuth::new(dir_path, opts);
            println!("READY");
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            let _ = BufReader::new(std::io::stdin()).read_line(&mut line);
            let tok = auth.access_token().expect("child access_token");
            println!("TOKEN:{}", tok.as_str());
            std::io::stdout().flush().unwrap();
        }
        other => panic!("unknown child mode `{other}`"),
    }
}

fn wait_for_child_line(child: &mut Child, expected: &str) {
    let stdout = child.stdout.as_mut().expect("child stdout");
    let mut all = String::new();
    loop {
        let mut bytes = Vec::new();
        let mut b = [0u8; 1];
        let mut n;
        while {
            n = stdout.read(&mut b).expect("read byte from child");
            n == 1
        } {
            if b[0] == b'\n' {
                break;
            }
            bytes.push(b[0]);
        }
        let line = String::from_utf8_lossy(&bytes).trim().to_string();
        if line == expected {
            return;
        }
        all.push_str(&line);
        all.push('\n');
        if n == 0 {
            panic!("child exited before printing `{expected}`, stdout:\n{all}");
        }
    }
}

#[test]
fn vault_lock_excludes_real_second_process_and_releases_on_abrupt_kill() {
    if std::env::var_os(ENV_MODE).is_some() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let lock_file = vault::dir(tmp.path()).join(".lock");

    // 1. Abrupt kill (`TerminateProcess` / `SIGKILL`) releases the OS lock handle with zero cleanup.
    let mut holder = ChildGuard::spawn("hold", tmp.path(), None);
    wait_for_child_line(holder.child_mut(), "LOCKED");

    let probe = fs::OpenOptions::new().write(true).open(&lock_file).unwrap();
    assert!(
        matches!(probe.try_lock(), Err(fs::TryLockError::WouldBlock)),
        "lock held by child process must block try_lock in parent process"
    );
    drop(probe);

    let acquired = Arc::new(AtomicBool::new(false));
    let acq_flag = Arc::clone(&acquired);
    let dir_clone = tmp.path().to_path_buf();
    let waiter = std::thread::spawn(move || {
        let _guard = VaultLock::acquire(&dir_clone).expect("acquire after child kill");
        acq_flag.store(true, Ordering::Release);
    });

    std::thread::sleep(Duration::from_millis(120));
    assert!(!acquired.load(Ordering::Acquire), "waiter must remain blocked while child lives");

    let t_kill = Instant::now();
    holder.kill_and_wait();
    waiter.join().expect("waiter thread panicked");
    assert!(acquired.load(Ordering::Acquire));
    assert!(
        t_kill.elapsed() < Duration::from_secs(2),
        "OS should release lock immediately when child dies, took {:?}",
        t_kill.elapsed()
    );
    assert!(lock_file.exists(), ".lock file must stay on disk");

    // 2. Live second process holding past LOCK_WAIT (10 s) causes fail-closed error, never theft.
    let mut live_holder = ChildGuard::spawn("hold", tmp.path(), None);
    wait_for_child_line(live_holder.child_mut(), "LOCKED");

    let err = match VaultLock::acquire(tmp.path()) {
        Ok(_) => panic!("must not steal VaultLock from a live second process"),
        Err(e) => e,
    };
    assert!(err.contains("another ChatGPT sign-in operation is in progress"), "{err}");

    // Closing child stdin lets it exit cleanly and releases the lock.
    drop(live_holder.child_mut().stdin.take());
    assert!(live_holder.wait_clean().success());
    let final_lock = VaultLock::acquire(tmp.path()).expect("acquire after clean child exit");
    drop(final_lock);
}

#[cfg(windows)]
#[test]
fn two_processes_serialize_dpapi_refresh_and_fail_closed_logout_preserves_vault() {
    use chatgpt_auth::testkit::{browser_get, id_claims, jwks, sign_jwt, url_param, MockAuth, Recorded, Reply};
    use chatgpt_auth::{ChatGptAuth, Endpoints, Options};
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    if std::env::var_os(ENV_MODE).is_some() {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let nonce_slot = Arc::new(Mutex::new(String::new()));
    let issuer_slot = Arc::new(Mutex::new(String::new()));
    let refreshes = Arc::new(AtomicUsize::new(0));

    let n2 = Arc::clone(&nonce_slot);
    let i2 = Arc::clone(&issuer_slot);
    let r2 = Arc::clone(&refreshes);
    let mock = MockAuth::start(Arc::new(move |req: &Recorded| match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/.well-known/jwks.json") => Reply::json(200, &jwks()),
        ("POST", "/api/accounts/oauth/token") => match req.form("grant_type").as_deref() {
            Some("authorization_code") => {
                let client = req.form("client_id").unwrap_or_default();
                let exp = chatgpt_auth::codec::now_s() + 3600;
                let claims = id_claims(&i2.lock().unwrap(), &client, &n2.lock().unwrap(), "sub-mp", exp);
                Reply::json(200, &json!({
                    "access_token": "acc-1-SECRETVALUE",
                    "refresh_token": "rt-1-SECRETVALUE",
                    "id_token": sign_jwt(&claims),
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke",
                    "earliest_refresh_at": 0
                }))
            }
            Some("refresh_token") => {
                std::thread::sleep(Duration::from_millis(300));
                let n = r2.fetch_add(1, Ordering::SeqCst) + 2;
                Reply::json(200, &json!({
                    "access_token": format!("acc-{n}-SECRETVALUE"),
                    "refresh_token": format!("rt-{n}-SECRETVALUE"),
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke"
                }))
            }
            _ => Reply::json(400, &json!({ "error": "unsupported_grant_type" })),
        },
        _ => Reply::json(404, &json!({})),
    }));
    *issuer_slot.lock().unwrap() = mock.base.clone();

    let opts = Options {
        endpoints: Endpoints::loopback_mock(&mock.base).unwrap(),
        ..Options::default()
    };
    let auth = Arc::new(ChatGptAuth::new(tmp.path(), opts));
    let start = auth.start_login().unwrap();
    *nonce_slot.lock().unwrap() = url_param(&start.authorize_url, "nonce").unwrap();
    let state = url_param(&start.authorize_url, "state").unwrap();
    let ans = browser_get(
        start.port,
        &format!("/auth/callback?code=c1&scope=chatgpt.tokens.use.direct+openid&state={state}&client_id=oaiapp_mp"),
    );
    assert_eq!(ans.unwrap().0, 200);

    // 1. While another process holds `.lock`, `logout()` fails closed and keeps `credentials.dpapi`.
    let mut holder = ChildGuard::spawn("hold", tmp.path(), None);
    wait_for_child_line(holder.child_mut(), "LOCKED");
    let out = auth.logout();
    assert_eq!(out.revoked, Some(false));
    assert!(out.message.contains("still on this computer"), "{}", out.message);
    assert!(
        vault::load_credentials(tmp.path()).unwrap().is_some(),
        "credentials.dpapi must not be deleted when another process holds the vault lock"
    );
    drop(holder.child_mut().stdin.take());
    assert!(holder.wait_clean().success());

    // 2. Make DPAPI credentials due for refresh and race parent + child processes on `access_token()`.
    let mut creds = vault::load_credentials(tmp.path()).unwrap().unwrap();
    let now = chatgpt_auth::codec::now_s();
    creds.saved_at_s = now - 3540;
    creds.expires_at_s = now + 60;
    vault::save_credentials(tmp.path(), &creds).unwrap();

    let mut racer = ChildGuard::spawn("refresh", tmp.path(), Some(&mock.base));
    wait_for_child_line(racer.child_mut(), "READY");

    racer.child_mut().stdin.as_mut().unwrap().write_all(b"GO\n").unwrap();
    racer.child_mut().stdin.as_mut().unwrap().flush().unwrap();
    let parent_tok = auth.access_token().expect("parent access_token");

    let mut child_out = String::new();
    racer.child_mut().stdout.as_mut().unwrap().read_to_string(&mut child_out).unwrap();
    assert!(racer.wait_clean().success(), "child failed: {child_out}");

    let child_tok = child_out
        .lines()
        .find_map(|l| l.strip_prefix("TOKEN:"))
        .expect("TOKEN line from child");
    assert_eq!(parent_tok.as_str(), "acc-2-SECRETVALUE");
    assert_eq!(child_tok, "acc-2-SECRETVALUE");
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "two racing OS processes must share a single OAuth refresh under VaultLock"
    );
}
