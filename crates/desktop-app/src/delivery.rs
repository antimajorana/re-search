//! How the window gets the per-launch token without it ever appearing in a URL or on a
//! process command line.
//!
//! 1. **Header path.** The first navigation is `GET <base_url>/` with the token in the
//!    `X-ReSearch-Token` request header. The server then writes the token into the page's
//!    `<meta name="re-search-csrf">` (and sets the stream cookie), matching the
//!    single-use `?launch=` nonce flow of `re-search ui`.
//! 2. **Probe.** WebView2 can drop the header silently (the navigation still "succeeds"
//!    and shows the server's 403 body). After the first load finishes (or a timeout) a
//!    probe script reports only `ok` / `no-token` / `elsewhere`, never the token itself.
//! 3. **Fallback.** If the token did not arrive, a script is run in the current top-level
//!    document, and only if its origin is exactly `base_url`: it stores the token under
//!    the `sessionStorage` key the UI already reads (`re-search-token`), fetches `/` with
//!    the header (which sets the `HttpOnly` stream cookie) and reloads `/`. The reloaded
//!    page carries only the cookie, so the server serves it without the token, and the UI
//!    picks the token up from `sessionStorage`. If the window is not on the server at all,
//!    it is first sent to `<base_url>/` (no token).
//!
//! The state machine is pure so it is unit-tested; `app.rs` performs the actions.

use crate::rules::is_allowed_navigation;

pub const TOKEN_STORAGE_KEY: &str = "re-search-token";
const MAX_PROBE_TIMEOUTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPath {
    Header,
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input<'a> {
    /// A top-level navigation finished at this URL.
    PageFinished(&'a str),
    /// The probe script answered (`ok`, `no-token`, `elsewhere`, anything else = error).
    ProbeResult(&'a str),
    /// Nothing happened before the deadline.
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Run `probe_script` and feed its answer back as `ProbeResult`.
    Probe,
    /// Navigate to `<base_url>/` without any credential.
    LoadBase,
    /// Run `fallback_script` in the current document.
    RunFallback,
    Authenticated(TokenPath),
    Failed(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    WaitingForLoad,
    Probing,
    Done,
}

#[derive(Debug)]
pub struct Delivery {
    base_url: String,
    stage: Stage,
    reloaded: bool,
    fallback_run: bool,
    probe_timeouts: u8,
}

impl Delivery {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            stage: Stage::WaitingForLoad,
            reloaded: false,
            fallback_run: false,
            probe_timeouts: 0,
        }
    }

    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    fn on_server(&self, url: &str) -> bool {
        url != "about:blank" && is_allowed_navigation(url, &self.base_url)
    }

    fn finish(&mut self, action: Action) -> Action {
        self.stage = Stage::Done;
        action
    }

    fn not_on_server(&mut self) -> Action {
        if self.reloaded {
            return self.finish(Action::Failed("the window did not stay on the re:Search server"));
        }
        self.reloaded = true;
        self.stage = Stage::WaitingForLoad;
        Action::LoadBase
    }

    pub fn step(&mut self, input: Input<'_>) -> Action {
        match (self.stage, input) {
            (Stage::Done, _) => Action::None,
            (_, Input::PageFinished(url)) if self.on_server(url) => {
                self.stage = Stage::Probing;
                Action::Probe
            }
            (Stage::WaitingForLoad, Input::PageFinished(_)) => self.not_on_server(),
            (Stage::Probing, Input::PageFinished(_)) => Action::None,
            (Stage::WaitingForLoad, Input::Timeout) => {
                self.stage = Stage::Probing;
                Action::Probe
            }
            (Stage::Probing, Input::Timeout) => {
                self.probe_timeouts += 1;
                if self.probe_timeouts >= MAX_PROBE_TIMEOUTS {
                    return self.finish(Action::Failed("the window did not answer the token probe"));
                }
                Action::Probe
            }
            (Stage::WaitingForLoad, Input::ProbeResult(_)) => Action::None,
            (Stage::Probing, Input::ProbeResult(answer)) => match answer {
                "ok" => {
                    let path = if self.fallback_run { TokenPath::Fallback } else { TokenPath::Header };
                    self.finish(Action::Authenticated(path))
                }
                "no-token" if !self.fallback_run => {
                    self.fallback_run = true;
                    self.stage = Stage::WaitingForLoad;
                    Action::RunFallback
                }
                "no-token" => self.finish(Action::Failed("the token reached neither the page nor sessionStorage")),
                "elsewhere" => self.not_on_server(),
                _ => self.finish(Action::Failed("the token probe failed")),
            },
        }
    }
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Reports whether the page has the token. Returns a status word, never the token.
pub fn probe_script(base_url: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    if (window.top !== window.self || window.location.origin !== {base}) return 'elsewhere';
    var m = document.querySelector('meta[name="re-search-csrf"]');
    if (!m) return 'no-token';
    if (typeof m.content === 'string' && m.content.trim()) return 'ok';
    var s = '';
    try {{ s = window.sessionStorage.getItem({key}) || ''; }} catch (e) {{}}
    return s.trim() ? 'ok' : 'no-token';
  }} catch (e) {{ return 'error'; }}
}})()"#,
        base = js_string(base_url),
        key = js_string(TOKEN_STORAGE_KEY),
    )
}

/// Hands the token to the page when the header navigation did not. Acts only in the
/// top-level document and only when its origin is exactly `base_url`.
pub fn fallback_script(base_url: &str, token: &str) -> String {
    format!(
        r#"(function () {{
  var base = {base};
  if (window.top !== window.self || window.location.origin !== base) return 'elsewhere';
  var tok = {tok};
  fetch(base + '/', {{ method: 'GET', headers: {{ 'X-ReSearch-Token': tok }}, credentials: 'same-origin', cache: 'no-store', redirect: 'error' }})
    .then(function (r) {{
      if (!r.ok) return;
      try {{ window.sessionStorage.setItem({key}, tok); }} catch (e) {{ return; }}
      window.location.replace(base + '/');
    }})
    .catch(function () {{}});
  return 'started';
}})()"#,
        base = js_string(base_url),
        tok = js_string(token),
        key = js_string(TOKEN_STORAGE_KEY),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:43123";

    #[test]
    fn header_path_is_recognised_after_the_first_load() {
        let mut d = Delivery::new(BASE);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123/")), Action::Probe);
        assert_eq!(d.step(Input::ProbeResult("ok")), Action::Authenticated(TokenPath::Header));
        assert!(d.is_done());
        assert_eq!(d.step(Input::Timeout), Action::None);
    }

    #[test]
    fn dropped_header_falls_back_once_then_succeeds() {
        let mut d = Delivery::new(BASE);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123/")), Action::Probe);
        assert_eq!(d.step(Input::ProbeResult("no-token")), Action::RunFallback);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123/")), Action::Probe);
        assert_eq!(d.step(Input::ProbeResult("ok")), Action::Authenticated(TokenPath::Fallback));
    }

    #[test]
    fn fallback_is_not_repeated_forever() {
        let mut d = Delivery::new(BASE);
        d.step(Input::PageFinished("http://127.0.0.1:43123/"));
        assert_eq!(d.step(Input::ProbeResult("no-token")), Action::RunFallback);
        assert_eq!(d.step(Input::Timeout), Action::Probe);
        assert!(matches!(d.step(Input::ProbeResult("no-token")), Action::Failed(_)));
        assert!(d.is_done());
    }

    #[test]
    fn a_window_that_is_not_on_the_server_is_sent_there_once() {
        let mut d = Delivery::new(BASE);
        assert_eq!(d.step(Input::PageFinished("about:blank")), Action::LoadBase);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123/")), Action::Probe);
        assert_eq!(d.step(Input::ProbeResult("no-token")), Action::RunFallback);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123/")), Action::Probe);
        assert_eq!(d.step(Input::ProbeResult("ok")), Action::Authenticated(TokenPath::Fallback));

        let mut d = Delivery::new(BASE);
        assert_eq!(d.step(Input::PageFinished("http://127.0.0.1:43123.evil.example/")), Action::LoadBase);
        assert!(matches!(d.step(Input::PageFinished("about:blank")), Action::Failed(_)));
    }

    #[test]
    fn timeouts_probe_and_eventually_give_up() {
        let mut d = Delivery::new(BASE);
        assert_eq!(d.step(Input::Timeout), Action::Probe);
        assert_eq!(d.step(Input::Timeout), Action::Probe);
        assert_eq!(d.step(Input::Timeout), Action::Probe);
        assert!(matches!(d.step(Input::Timeout), Action::Failed(_)));
    }

    #[test]
    fn scripts_check_the_exact_origin_and_never_build_a_token_url() {
        let token = "ab".repeat(32);
        let probe = probe_script(BASE);
        assert!(probe.contains(r#"window.location.origin !== "http://127.0.0.1:43123""#));
        assert!(probe.contains("window.top !== window.self"));
        assert!(!probe.contains("return s;") && !probe.contains("return m"), "the probe must not return the token");

        let fb = fallback_script(BASE, &token);
        assert!(fb.contains(r#"var base = "http://127.0.0.1:43123";"#));
        assert!(fb.contains("window.location.origin !== base"));
        assert!(fb.contains("'X-ReSearch-Token': tok"));
        assert!(!fb.contains("token="), "the token is never put into a URL");
        // Values are JSON-escaped, so a hostile base URL cannot break out of the string.
        assert!(fallback_script("http://x\"+alert(1)+\"", &token).contains(r#""http://x\"+alert(1)+\"""#));
    }
}
