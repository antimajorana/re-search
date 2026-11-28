use super::*;

#[test]
fn sse_parser_handles_bom_crlf_names_multiline_and_drops_unterminated() {
    let evs = sse_events("\u{FEFF}event: a\r\ndata: {\"x\":\r\ndata: 1}\r\n\r\n: comment\n\ndata: [DONE]\n\ndata: cut");
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0].0.as_deref(), Some("a"));
    assert_eq!(evs[0].1, "{\"x\":\n1}");
    assert_eq!(evs[1].1, "[DONE]");
}

#[test]
fn only_official_host_or_loopback_is_allowed() {
    let ok = |u: &str| chatgpt_base_allowed(&validate_base_url(u).unwrap());
    for u in [
        "https://api.openai.com", "https://api.openai.com/v1", "https://api.openai.com/v1/responses",
        "http://127.0.0.1:9", "http://127.0.0.1:9/v1", "http://127.0.0.1:9/v1/responses",
    ] {
        assert!(ok(u), "{u}");
    }
    for u in [
        "https://proxy.example", "https://api.openai.com.evil.example", "https://api.openai.com/evil/prefix",
        "http://localhost:9", "http://[::1]:9", "http://127.0.0.1:9/evil",
    ] {
        assert!(!ok(u), "{u}");
    }
}

#[test]
fn http_errors_carry_code_param_and_hint_but_no_secret_or_raw_text() {
    let mk = |kind, status, msg: &str, body: &[u8]| describe_http_error(
        TransportError { kind, status: Some(status), message: msg.into(), raw_body: Some(body.to_vec()) },
        "tok-123",
    );
    let e = mk(TransportErrorKind::RateLimit, 429, "x", br#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","param":"model","message":"tok-123 partial-leak"}}"#);
    assert!(e.message.contains("subscription_sharing_usage_limit_exceeded") && e.message.contains("(param model)") && e.message.contains("Usage"));
    assert!(!e.message.contains("tok-123") && !e.message.contains("partial-leak"));
    let e = mk(TransportErrorKind::HttpStatus, 400, "HTTP 400: raw-leak", br#"{"error":{"code":"invalid_request_error","param":"tools","message":"partial-tok-leak"}}"#);
    assert_eq!(e.message, "HTTP 400 invalid_request_error (param tools)");
    let e = mk(TransportErrorKind::Auth, 401, "x", br#"{"detail":"raw-detail-leak"}"#);
    assert!(e.message.contains("sign in again") && !e.message.contains("raw-detail-leak"));
    let e = mk(TransportErrorKind::HttpStatus, 502, "HTTP 502: raw-html-leak", b"<html>raw-html-leak</html>");
    assert_eq!(e.message, "HTTP 502");
}
