//! Incremental SSE reader and <=40ms cancellable HTTP transport execution across
//! OpenAI Responses (`chatgpt`), Anthropic Messages (`anthropic`), OpenAI/Ollama
//! Chat Completions (`openai` / `ollama`), and non-streaming providers (`gemini`).

use crate::http::{
    map_ureq_error, post_json, scrub_bytes, scrub_text, summarize_error_body, MAX_ERROR_BYTES, MAX_RESPONSE_BYTES,
};
use crate::stream_codecs::{AnthropicSseAcc, OpenAiSseAcc};
use crate::transport::{StreamObserver, TransportError, TransportErrorKind};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SseFlavor {
    Responses,
    Anthropic,
    OpenAiChat,
}

pub(crate) fn inject_stream_flag(body: Vec<u8>) -> Vec<u8> {
    if let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&body) {
        map.insert("stream".into(), json!(true));
        if let Ok(encoded) = serde_json::to_vec(&Value::Object(map)) {
            return encoded;
        }
    }
    body
}

pub(crate) enum StreamMsg {
    Text(String),
    Thinking(String),
    Done(Result<Vec<u8>, TransportError>),
}

struct OwnedHeaders(Vec<(String, String)>);
impl Drop for OwnedHeaders {
    fn drop(&mut self) {
        for (k, v) in &mut self.0 {
            k.zeroize();
            v.zeroize();
        }
    }
}

struct OwnedSecret(String);
impl Drop for OwnedSecret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn drive_channel(
    rx: mpsc::Receiver<StreamMsg>,
    stop: &Arc<AtomicBool>,
    observer: &mut dyn StreamObserver,
) -> Result<Vec<u8>, TransportError> {
    loop {
        if observer.is_cancelled() {
            stop.store(true, Ordering::SeqCst);
            return Err(TransportError::cancelled());
        }
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(StreamMsg::Text(delta)) => {
                if !delta.is_empty() {
                    observer.on_text_delta(&delta);
                }
            }
            Ok(StreamMsg::Thinking(delta)) => {
                if !delta.is_empty() {
                    observer.on_thinking_delta(&delta);
                }
            }
            Ok(StreamMsg::Done(res)) => {
                return if observer.is_cancelled() { Err(TransportError::cancelled()) } else { res };
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if observer.is_cancelled() {
                    return Err(TransportError::cancelled());
                }
                return Err(TransportError {
                    kind: TransportErrorKind::Network,
                    status: None,
                    message: "HTTP stream worker disconnected unexpectedly".into(),
                    raw_body: None,
                });
            }
        }
    }
}

/// Runs a non-streaming `post_json` with <=40ms cooperative `CancelToken` abort.
pub(crate) fn post_json_cancellable(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    secret: &str,
    observer: &mut dyn StreamObserver,
) -> Result<Vec<u8>, TransportError> {
    if observer.is_cancelled() {
        return Err(TransportError::cancelled());
    }
    let agent_cl = agent.clone();
    let url_cl = url.to_string();
    let owned_headers = OwnedHeaders(headers.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect());
    let body_cl = body.to_vec();
    let owned_secret = OwnedSecret(secret.to_string());
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let hdr_refs: Vec<(&str, &str)> = owned_headers.0.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let res = post_json(&agent_cl, &url_cl, &hdr_refs, &body_cl, &owned_secret.0);
        let _ = tx.send(StreamMsg::Done(res));
    });

    drive_channel(rx, &stop, observer)
}

/// Executes an SSE-capable `POST`, emitting live `Text` / `Thinking` deltas while
/// returning either exact SSE bytes (`Responses`) or canonical reassembled JSON
/// (`Anthropic` / `OpenAiChat`) for content-addressed `put_blob` + lossless `resume`.
pub(crate) fn post_sse_streaming(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    secret: &str,
    flavor: SseFlavor,
    observer: &mut dyn StreamObserver,
) -> Result<Vec<u8>, TransportError> {
    if observer.is_cancelled() {
        return Err(TransportError::cancelled());
    }
    let agent_cl = agent.clone();
    let url_cl = url.to_string();
    let owned_headers = OwnedHeaders(headers.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect());
    let body_cl = body.to_vec();
    let owned_secret = OwnedSecret(secret.to_string());
    let stop = Arc::new(AtomicBool::new(false));
    let stop_worker = Arc::clone(&stop);
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let res = run_sse_worker(
            &agent_cl,
            &url_cl,
            &owned_headers.0,
            &body_cl,
            &owned_secret.0,
            flavor,
            &stop_worker,
            &tx,
        );
        let _ = tx.send(StreamMsg::Done(res));
    });

    drive_channel(rx, &stop, observer)
}

#[allow(clippy::too_many_arguments)]
fn run_sse_worker(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(String, String)],
    body: &[u8],
    secret: &str,
    flavor: SseFlavor,
    stop: &AtomicBool,
    tx: &mpsc::Sender<StreamMsg>,
) -> Result<Vec<u8>, TransportError> {
    let mut req = agent.post(url);
    for (name, val) in headers {
        req = req.header(name.as_str(), val.as_str());
    }
    let mut resp = req
        .content_type("application/json")
        .send(body)
        .map_err(|e| map_ureq_error(e, None, secret))?;

    let status = resp.status().as_u16();
    if status != 200 {
        let raw = resp
            .body_mut()
            .with_config()
            .limit(MAX_ERROR_BYTES)
            .read_to_vec()
            .map_err(|e| map_ureq_error(e, Some(status), secret))?;
        let clean_body = scrub_bytes(&raw, secret.as_bytes());
        let kind = match status {
            401 | 403 => TransportErrorKind::Auth,
            429 => TransportErrorKind::RateLimit,
            _ => TransportErrorKind::HttpStatus,
        };
        let message = scrub_text(&summarize_error_body(status, &clean_body), secret);
        return Err(TransportError { kind, status: Some(status), message, raw_body: Some(clean_body) });
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if flavor != SseFlavor::Responses && !content_type.contains("text/event-stream") {
        return resp
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(|e| map_ureq_error(e, Some(status), secret));
    }

    let mut reader = BufReader::new(resp.body_mut().as_reader().take(MAX_RESPONSE_BYTES + 1));
    let mut raw_body = Vec::new();
    let mut line_buf = Vec::new();
    let mut ev_name: Option<String> = None;
    let mut ev_data: Vec<String> = Vec::new();
    let mut first_line = true;
    let mut anthropic_acc = AnthropicSseAcc::default();
    let mut openai_acc = OpenAiSseAcc::default();

    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(TransportError::cancelled());
        }
        line_buf.clear();
        let n = reader.read_until(b'\n', &mut line_buf).map_err(|e| TransportError {
            kind: TransportErrorKind::Network,
            status: Some(200),
            message: scrub_text(&format!("SSE read error: {e}"), secret),
            raw_body: None,
        })?;
        if n == 0 {
            break;
        }
        raw_body.extend_from_slice(&line_buf);
        if raw_body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(TransportError {
                kind: TransportErrorKind::OversizedResponse,
                status: Some(200),
                message: "SSE stream exceeded 16 MiB response limit".into(),
                raw_body: None,
            });
        }

        let Ok(raw_str) = std::str::from_utf8(&line_buf) else {
            continue;
        };
        let stripped_bom = if first_line {
            first_line = false;
            raw_str.strip_prefix('\u{FEFF}').unwrap_or(raw_str)
        } else {
            raw_str
        };
        if !stripped_bom.ends_with('\n') {
            break;
        }
        let line = stripped_bom
            .strip_suffix("\r\n")
            .or_else(|| stripped_bom.strip_suffix('\n'))
            .unwrap_or(stripped_bom);

        if line.is_empty() {
            if !ev_data.is_empty() {
                let joined = ev_data.join("\n");
                handle_sse_frame(
                    flavor,
                    ev_name.as_deref(),
                    &joined,
                    secret,
                    tx,
                    &mut anthropic_acc,
                    &mut openai_acc,
                )?;
            }
            ev_name = None;
            ev_data.clear();
        } else if let Some(rest) = line.strip_prefix("data:") {
            ev_data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if let Some(rest) = line.strip_prefix("event:") {
            ev_name = Some(rest.trim().to_string());
        }
    }

    match flavor {
        SseFlavor::Responses => Ok(raw_body),
        SseFlavor::Anthropic => anthropic_acc.into_json_bytes(&raw_body, secret),
        SseFlavor::OpenAiChat => openai_acc.into_json_bytes(&raw_body, secret),
    }
}

fn handle_sse_frame(
    flavor: SseFlavor,
    event_name: Option<&str>,
    data: &str,
    secret: &str,
    tx: &mpsc::Sender<StreamMsg>,
    anthropic_acc: &mut AnthropicSseAcc,
    openai_acc: &mut OpenAiSseAcc,
) -> Result<(), TransportError> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Ok(());
    }
    match flavor {
        SseFlavor::Responses => {
            if let Ok(ev) = serde_json::from_str::<Value>(trimmed) {
                let ty = ev.get("type").and_then(Value::as_str).or(event_name).unwrap_or("");
                match ty {
                    "response.output_text.delta" => {
                        if let Some(d) = ev.get("delta").and_then(Value::as_str) {
                            let _ = tx.send(StreamMsg::Text(scrub_text(d, secret)));
                        }
                    }
                    "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                        if let Some(d) = ev.get("delta").and_then(Value::as_str) {
                            let _ = tx.send(StreamMsg::Thinking(scrub_text(d, secret)));
                        }
                    }
                    "response.output_item.added" => {
                        let item_ty = ev["item"]["type"].as_str().unwrap_or("");
                        if item_ty == "reasoning" {
                            let _ = tx.send(StreamMsg::Thinking("[Reasoning...]\n".into()));
                        } else if item_ty == "function_call" {
                            let name = ev["item"]["name"].as_str().unwrap_or("tool");
                            let _ = tx.send(StreamMsg::Thinking(format!("[Preparing tool `{name}`...]\n")));
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        SseFlavor::Anthropic => anthropic_acc.ingest(event_name, trimmed, secret, tx),
        SseFlavor::OpenAiChat => openai_acc.ingest(trimmed, secret, tx),
    }
}
