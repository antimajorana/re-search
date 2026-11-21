//! Test helpers (feature `testkit`): a loopback mock of OpenAI's auth and API hosts, a
//! throwaway RSA key (generated for these tests only; it protects nothing) with its JWKS,
//! RS256 JWT signing, and a raw HTTP client for the sign-in callback. No real network.

use crate::codec::b64url_encode;
use ring::rand::SystemRandom;
use ring::signature::{KeyPair, RsaKeyPair, RSA_PKCS1_SHA256};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Throwaway 2048-bit RSA key (DER `RSAPrivateKey`), made with `openssl genpkey` for these tests.
const TEST_KEY: &[u8] = include_bytes!("test_rsa_key.der");
pub const TEST_KID: &str = "re-search-test-key";

fn key() -> RsaKeyPair {
    RsaKeyPair::from_der(TEST_KEY).expect("test key")
}

/// The JWKS that holds the test key's public half.
pub fn jwks() -> Value {
    let k = key();
    let (n, e) = rsa_public_components(k.public_key().as_ref());
    json!({ "keys": [{ "kty": "RSA", "use": "sig", "alg": "RS256", "kid": TEST_KID, "n": b64url_encode(&n), "e": b64url_encode(&e) }] })
}

/// `n` and `e` from a DER `RSAPublicKey` (SEQUENCE { INTEGER n, INTEGER e }).
fn rsa_public_components(der: &[u8]) -> (Vec<u8>, Vec<u8>) {
    fn tlv(b: &[u8], tag: u8) -> (&[u8], &[u8]) {
        assert_eq!(b[0], tag, "unexpected DER tag");
        let (len, hdr) = match b[1] {
            l if l < 0x80 => (l as usize, 2),
            0x81 => (b[2] as usize, 3),
            0x82 => (((b[2] as usize) << 8) | b[3] as usize, 4),
            _ => panic!("unsupported DER length"),
        };
        (&b[hdr..hdr + len], &b[hdr + len..])
    }
    let (seq, _) = tlv(der, 0x30);
    let (n, rest) = tlv(seq, 0x02);
    let (e, _) = tlv(rest, 0x02);
    let strip = |x: &[u8]| x.iter().skip_while(|b| **b == 0).copied().collect::<Vec<u8>>();
    (strip(n), strip(e))
}

pub fn sign_jwt_with_header(header: &Value, claims: &Value) -> String {
    let input = format!("{}.{}", b64url_encode(header.to_string().as_bytes()), b64url_encode(claims.to_string().as_bytes()));
    let k = key();
    let mut sig = vec![0u8; k.public().modulus_len()];
    k.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), input.as_bytes(), &mut sig).expect("sign");
    format!("{input}.{}", b64url_encode(&sig))
}

pub fn sign_jwt(claims: &Value) -> String {
    sign_jwt_with_header(&json!({ "alg": "RS256", "typ": "JWT", "kid": TEST_KID }), claims)
}

/// Standard ID-token claims for tests.
pub fn id_claims(issuer: &str, client_id: &str, nonce: &str, sub: &str, exp: i64) -> Value {
    json!({
        "iss": issuer, "sub": sub, "aud": [client_id], "nonce": nonce,
        "iat": exp - 3600, "exp": exp, "email": "person@example.test", "email_verified": true
    })
}

/// One request the mock received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// A form field of the body.
    pub fn form(&self, name: &str) -> Option<String> {
        crate::codec::parse_query(&self.body).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, v: &Value) -> Self {
        Reply { status, content_type: "application/json", body: v.to_string().into_bytes() }
    }
}

pub type Handler = Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>;

/// A loopback HTTP server answering every request with `handler` (one thread per
/// connection), recording what it received.
pub struct MockAuth {
    pub base: String,
    log: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockAuth {
    pub fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (l2, s2) = (Arc::clone(&log), Arc::clone(&stop));
        let handle = std::thread::spawn(move || {
            while !s2.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (h, l) = (Arc::clone(&handler), Arc::clone(&l2));
                        std::thread::spawn(move || serve(stream, &h, &l));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self { base, log, stop, handle: Some(handle) }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }

    pub fn count(&self, path: &str) -> usize {
        self.requests().iter().filter(|r| r.path == path).count()
    }
}

impl Drop for MockAuth {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn serve(mut stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Recorded>>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target.clone(), String::new()));
    let mut headers = Vec::new();
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.trim_end().split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().unwrap_or(0);
            }
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    let rec = Recorded { method, path, query, headers, body: String::from_utf8_lossy(&body).into_owned() };
    log.lock().unwrap().push(rec.clone());
    let reply = handler(&rec);
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}

/// A value of the authorize URL's query.
pub fn url_param(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    crate::codec::parse_query(q).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// `GET <path_and_query>` on the callback listener the way a browser sends it. Returns the
/// status and body, or `None` when nothing is listening.
pub fn browser_get(port: u16, path_and_query: &str) -> Option<(u16, String)> {
    browser_request(port, "GET", path_and_query, &format!("127.0.0.1:{port}"))
}

pub fn browser_request(port: u16, method: &str, path_and_query: &str, host: &str) -> Option<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    let req = format!("{method} {path_and_query} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split_whitespace().nth(1)?.parse().ok()?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Some((status, body))
}
