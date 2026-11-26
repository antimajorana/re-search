//! Process-wide per-host / per-service outgoing network rate gate (package F3).
//!
//! Limits (from `00-overview.md` section 2.4):
//! - arXiv (`export.arxiv.org`, `arxiv.org`): 1 request per 3 s (shared bucket)
//! - INSPIRE-HEP (`inspirehep.net`): 15 requests per 5 s
//! - Semantic Scholar (`api.semanticscholar.org`): 1 request per 1 s
//! - Crossref (`api.crossref.org`): 1 request per 1 s
//! - OpenAlex (`api.openalex.org`): 10 requests per 1 s (ASSUMPTION, X19 Part 1)
//!
//! Every outgoing literature HTTP request in `research-tools` passes through
//! [`global_gate`] (or an injected [`NetRateGate`] in tests). For loopback mocks
//! (`LiteratureEndpoints::loopback_mock`), requests are still bucketed per logical
//! service via explicit [`Service`] hints, unique endpoint origins, or known service
//! URL path prefixes.

use crate::LiteratureEndpoints;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ureq::http::Uri;

/// Rate limit policy: at most `max_requests` grants within any sliding `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimit {
    pub max_requests: usize,
    pub window: Duration,
}

/// Conservative fallback limit for any allowlisted HTTPS host not in [`Service`].
pub const DEFAULT_HOST_LIMIT: RateLimit = RateLimit {
    max_requests: 1,
    window: Duration::from_secs(1),
};

/// Logical scholarly services governed by the process-wide network gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Service {
    Arxiv,
    Inspire,
    SemanticScholar,
    Crossref,
    OpenAlex,
}

impl Service {
    pub const ALL: [Service; 5] = [
        Self::Arxiv,
        Self::Inspire,
        Self::SemanticScholar,
        Self::Crossref,
        Self::OpenAlex,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arxiv => "arxiv",
            Self::Inspire => "inspire",
            Self::SemanticScholar => "semantic_scholar",
            Self::Crossref => "crossref",
            Self::OpenAlex => "openalex",
        }
    }

    /// Configured rate limit from `00-overview.md` section 2.4 (plus X19 Part 1 OpenAlex).
    pub const fn rate_limit(self) -> RateLimit {
        match self {
            Self::Arxiv => RateLimit {
                max_requests: 1,
                window: Duration::from_secs(3),
            },
            Self::Inspire => RateLimit {
                max_requests: 15,
                window: Duration::from_secs(5),
            },
            Self::SemanticScholar => RateLimit {
                max_requests: 1,
                window: Duration::from_secs(1),
            },
            Self::Crossref => RateLimit {
                max_requests: 1,
                window: Duration::from_secs(1),
            },
            // ASSUMPTION (X19 Part 1): OpenAlex polite/free-key pool ceiling is 10 req/s;
            // not pinned numerically in `00-overview.md` 2.4.
            Self::OpenAlex => RateLimit {
                max_requests: 10,
                window: Duration::from_secs(1),
            },
        }
    }

    /// Map a real allowlisted hostname to its logical service.
    ///
    /// Both `export.arxiv.org` and `arxiv.org` map to [`Service::Arxiv`] so Atom API
    /// metadata queries and e-print/PDF downloads share a single 1-per-3s gate.
    pub fn from_host(host: &str) -> Option<Self> {
        match host.to_ascii_lowercase().as_str() {
            "export.arxiv.org" | "arxiv.org" => Some(Self::Arxiv),
            "inspirehep.net" => Some(Self::Inspire),
            "api.semanticscholar.org" => Some(Self::SemanticScholar),
            "api.crossref.org" => Some(Self::Crossref),
            "api.openalex.org" => Some(Self::OpenAlex),
            _ => None,
        }
    }

    /// Infer the logical service from a request path when multiple services share one
    /// loopback mock server (`LiteratureEndpoints::loopback_mock`).
    pub fn from_loopback_path(path: &str) -> Option<Self> {
        if path == "/api/query"
            || path.starts_with("/api/query/")
            || path.starts_with("/e-print/")
            || path.starts_with("/pdf/")
            || path.starts_with("/abs/")
            || path.starts_with("/html/")
        {
            return Some(Self::Arxiv);
        }
        if path == "/api/literature"
            || path.starts_with("/api/literature/")
            || path.starts_with("/api/doi/")
        {
            return Some(Self::Inspire);
        }
        // Match `/openalex` before Crossref's `/works` because OpenAlex loopback paths are
        // `/openalex/works...`.
        if path == "/openalex" || path.starts_with("/openalex/") {
            return Some(Self::OpenAlex);
        }
        if path == "/works" || path.starts_with("/works/") {
            return Some(Self::Crossref);
        }
        if path.starts_with("/graph/v1/") {
            return Some(Self::SemanticScholar);
        }
        None
    }
}

/// Gate key for a request: a known logical service, an arbitrary remote host, or an
/// unmapped loopback mock path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateBucket {
    Service(Service),
    Host(String),
    LoopbackGeneric(u16),
}

impl RateBucket {
    pub fn rate_limit(&self) -> Option<RateLimit> {
        match self {
            Self::Service(svc) => Some(svc.rate_limit()),
            Self::Host(_) => Some(DEFAULT_HOST_LIMIT),
            Self::LoopbackGeneric(_) => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Service(svc) => svc.as_str().to_string(),
            Self::Host(host) => host.clone(),
            Self::LoopbackGeneric(port) => format!("127.0.0.1:{port}"),
        }
    }
}

fn parse_loopback_port(base: &str) -> Option<u16> {
    let uri: Uri = base.trim().parse().ok()?;
    if !uri.scheme_str()?.eq_ignore_ascii_case("http") {
        return None;
    }
    let auth = uri.authority()?;
    if auth.host() != "127.0.0.1" {
        return None;
    }
    auth.port_u16()
}

/// Resolve the rate-limit bucket for `uri` under `endpoints` and an optional caller-known
/// [`Service`] hint.
///
/// Resolution order:
/// 1. Real remote hosts always map by hostname (`export.arxiv.org` and `arxiv.org` share
///    `Service::Arxiv`; unknown remote hosts get `RateBucket::Host`).
/// 2. Loopback URLs (`127.0.0.1:<port>`) try, in order: an explicit `service_hint`
///    (used by `arxiv.rs`, `literature.rs`, `paper_fetch.rs`); a unique logical service in
///    `endpoints` bound to that loopback port; a known service URL path prefix
///    (`Service::from_loopback_path`), which separates services even when
///    `LiteratureEndpoints::loopback_mock` points all five endpoints at one port; and
///    finally `RateBucket::LoopbackGeneric(port)` for generic HTTP-allowlist test paths.
pub fn resolve_bucket(
    uri: &Uri,
    endpoints: &LiteratureEndpoints,
    service_hint: Option<Service>,
) -> RateBucket {
    let host = uri
        .authority()
        .map(|a| a.host().to_ascii_lowercase())
        .unwrap_or_default();
    if host != "127.0.0.1" && host != "localhost" {
        return match Service::from_host(&host) {
            Some(svc) => RateBucket::Service(svc),
            None => RateBucket::Host(host),
        };
    }

    if let Some(svc) = service_hint {
        return RateBucket::Service(svc);
    }

    let port = uri.authority().and_then(|a| a.port_u16()).unwrap_or(0);
    if port != 0 {
        let mut matched: Vec<Service> = Vec::new();
        let candidates = [
            (endpoints.arxiv_api_base.as_str(), Service::Arxiv),
            (endpoints.arxiv_eprint_base.as_str(), Service::Arxiv),
            (endpoints.inspire_api_base.as_str(), Service::Inspire),
            (endpoints.crossref_api_base.as_str(), Service::Crossref),
            (endpoints.semantic_scholar_api_base.as_str(), Service::SemanticScholar),
            (endpoints.openalex_api_base.as_str(), Service::OpenAlex),
        ];
        for (base, svc) in candidates {
            if parse_loopback_port(base) == Some(port) && !matched.contains(&svc) {
                matched.push(svc);
            }
        }
        if matched.len() == 1 {
            return RateBucket::Service(matched[0]);
        }
    }

    if let Some(svc) = Service::from_loopback_path(uri.path()) {
        return RateBucket::Service(svc);
    }
    RateBucket::LoopbackGeneric(port)
}

/// Injectable monotonic clock for [`NetRateGate`].
pub trait RateClock: Send + Sync {
    /// Elapsed monotonic duration since this clock's origin.
    fn now(&self) -> Duration;
    /// Pause for `dur`. Fake clocks advance virtual time without real sleeping.
    fn sleep(&self, dur: Duration);
}

/// Production clock backed by [`Instant`] and [`std::thread::sleep`].
pub struct SystemRateClock {
    origin: Instant,
}

impl SystemRateClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemRateClock {
    fn default() -> Self {
        Self::new()
    }
}

impl RateClock for SystemRateClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, dur: Duration) {
        if !dur.is_zero() {
            std::thread::sleep(dur);
        }
    }
}

#[derive(Default)]
struct FakeClockInner {
    now: Duration,
    sleeps: Vec<Duration>,
}

/// Deterministic virtual clock for unit and integration tests. Never sleeps in real time.
#[derive(Default)]
pub struct FakeRateClock {
    inner: Mutex<FakeClockInner>,
}

impl FakeRateClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Manually advance virtual time (for example to simulate elapsed wall time between calls).
    pub fn advance(&self, dur: Duration) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.now = g.now.saturating_add(dur);
    }

    /// Current virtual timestamp.
    pub fn current(&self) -> Duration {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).now
    }

    /// Every non-zero sleep duration requested from this clock, in order.
    pub fn recorded_sleeps(&self) -> Vec<Duration> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sleeps
            .clone()
    }

    /// Sum of all recorded sleep durations.
    pub fn total_slept(&self) -> Duration {
        self.recorded_sleeps()
            .into_iter()
            .fold(Duration::ZERO, |acc, d| acc.saturating_add(d))
    }
}

impl RateClock for FakeRateClock {
    fn now(&self) -> Duration {
        self.current()
    }

    fn sleep(&self, dur: Duration) {
        if !dur.is_zero() {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.now = g.now.saturating_add(dur);
            g.sleeps.push(dur);
        }
    }
}

/// Details of one acquired rate-gate slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateGrant {
    pub bucket: RateBucket,
    pub granted_at: Duration,
    pub waited: Duration,
}

#[derive(Default)]
struct GateState {
    slots: HashMap<RateBucket, VecDeque<Duration>>,
    acquired: HashMap<RateBucket, u64>,
}

/// Thread-safe per-host / per-service sliding-window rate gate.
///
/// Slot reservation happens under a short mutex lock; waiting (`clock.sleep`) happens
/// strictly outside the lock so calls to other hosts or threads waiting on staggered
/// slots never serialize on each other.
pub struct NetRateGate {
    clock: Arc<dyn RateClock>,
    state: Mutex<GateState>,
    /// Whether a request to a loopback mock waits like the service it stands for.
    wait_on_loopback: bool,
}

impl NetRateGate {
    pub fn new(clock: Arc<dyn RateClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(GateState::default()),
            wait_on_loopback: true,
        }
    }

    /// The gate of a running process: real hosts wait on the real clock; a loopback mock is
    /// counted but never waited for. The limits protect the services, and a mock on
    /// `127.0.0.1` needs no protection, so tests that reach the process-wide gate through a
    /// mock do not sleep. Spacing is tested with [`NetRateGate::new`] and a fake clock, which
    /// does apply the limits to loopback URLs.
    pub fn process_wide() -> Self {
        Self {
            wait_on_loopback: false,
            ..Self::system()
        }
    }

    pub fn system() -> Self {
        Self::new(Arc::new(SystemRateClock::new()))
    }

    /// Reserve a slot for `bucket` and wait until it is eligible.
    pub fn acquire_grant(&self, bucket: &RateBucket) -> RateGrant {
        let (granted_at, waited) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *state.acquired.entry(bucket.clone()).or_insert(0) += 1;
            let now = self.clock.now();
            match bucket.rate_limit() {
                None => (now, Duration::ZERO),
                Some(limit) => {
                    let max = limit.max_requests.max(1);
                    let q = state.slots.entry(bucket.clone()).or_default();
                    let grant_at = if q.len() < max {
                        now
                    } else {
                        let earliest = q.pop_front().unwrap_or(Duration::ZERO);
                        let eligible = earliest.saturating_add(limit.window);
                        now.max(eligible)
                    };
                    q.push_back(grant_at);
                    let wait = grant_at.saturating_sub(now);
                    (grant_at, wait)
                }
            }
        };
        if !waited.is_zero() {
            self.clock.sleep(waited);
        }
        RateGrant {
            bucket: bucket.clone(),
            granted_at,
            waited,
        }
    }

    /// Reserve a slot for `bucket`, wait if needed, and return how long this call waited.
    pub fn acquire(&self, bucket: &RateBucket) -> Duration {
        self.acquire_grant(bucket).waited
    }

    /// Convenience wrapper for acquiring a known [`Service`] bucket.
    pub fn acquire_service(&self, service: Service) -> Duration {
        self.acquire(&RateBucket::Service(service))
    }

    /// Resolve the bucket for `uri` and wait for a slot.
    pub fn acquire_for_url(
        &self,
        uri: &Uri,
        endpoints: &LiteratureEndpoints,
        service_hint: Option<Service>,
    ) -> RateGrant {
        let bucket = resolve_bucket(uri, endpoints, service_hint);
        let host = uri.authority().map(|a| a.host().to_ascii_lowercase()).unwrap_or_default();
        if !self.wait_on_loopback && (host == "127.0.0.1" || host == "localhost") {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            *state.acquired.entry(bucket.clone()).or_insert(0) += 1;
            return RateGrant { bucket, granted_at: self.clock.now(), waited: Duration::ZERO };
        }
        self.acquire_grant(&bucket)
    }

    /// Total number of requests that have passed this gate for `bucket`.
    pub fn acquired_count(&self, bucket: &RateBucket) -> u64 {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.acquired.get(bucket).copied().unwrap_or(0)
    }
}

static GLOBAL_GATE: OnceLock<NetRateGate> = OnceLock::new();

/// Process-wide network rate gate shared by every caller and mode in the process.
pub fn global_gate() -> &'static NetRateGate {
    GLOBAL_GATE.get_or_init(NetRateGate::process_wide)
}

#[cfg(test)]
#[path = "net_rate_tests.rs"]
mod tests;
