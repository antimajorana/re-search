//! Opt-in PDF equation vision transcription HTTP route and settings sidecar (package F14c).
//!
//! Endpoint: `POST /api/sessions/{id}/sources/{source}/transcribe-equations`
//!
//! Owner Decision 2026-11-26 invariants enforced here:
//! 1. Off by default (`DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false`): refuses with `403`
//!    unless explicitly opted in per request (`opt_in: true`) or in `<data_dir>/pdf_vision_settings.json`.
//! 2. Unconditionally records `pdf.transcription` labelled `"model-transcribed"` and NEVER
//!    writes `evidence.quote`, NEVER mints a `QuoteRef`, and NEVER modifies `source.ingested`.
//! 3. Records `context.supplied` `page_ranges` (zero credit in `SentLines::covers`) and enforces
//!    F9 `run_budget` + fail-closed `(ProviderKind, model)` vision capability checks.

use crate::agent_models::{resolve_in_core_role_model, NEEDS_API_PROVIDER};
use crate::cli_turn::CliRouteKind;
use crate::routes_session::{bad, internal, not_found, ApiResult};
use crate::state::DesktopState;
use crate::turn_control::ApiTurnGuard;
use agent::vision::{
    build_image_block, decode_base64_strict, supports_vision, validate_image_bytes,
    validate_request_images, MAX_IMAGE_BYTES,
};
use agent::{
    CancelToken, ContentBlock, Message, ModelRequest, ProviderKind, Role, StreamObserver,
    TransportErrorKind,
};
use ledger::Event;
use research_tools::lines_sent::{
    ContextSuppliedPayload, SuppliedPageRange, CONTEXT_SUPPLIED_EVENT,
};
use research_tools::pdf_vision::{
    check_text_layer_agreement, is_equation_transcription_opted_in, FormulaTranscription,
    DEFAULT_EQUATION_TRANSCRIPTION_ENABLED, EVENT_PDF_TRANSCRIPTION, MAX_LATEX_DRAFT_BYTES,
    MODEL_TRANSCRIBED_LABEL,
};
use research_tools::run_budget::{count_model_calls, ModelCallBudget, RunScope, RUN_ID_FIELD};
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Sidecar settings file under `DesktopConfig::data_dir` (keeps `state.rs` untouched).
pub const PDF_VISION_SETTINGS_FILE: &str = "pdf_vision_settings.json";

/// Default F9 model-call budget cap for equation transcription requests when omitted.
pub const DEFAULT_VISION_BUDGET_CAP: u64 = 64;

/// Ledger caller identifier recorded on `context.supplied` `page_ranges` events.
pub const VISION_CALLER_ID: &str = "pdf_vision";

const MAX_PROMPT_HINT_CHARS: usize = 500;
const VISION_SYSTEM_PROMPT: &str = "You transcribe mathematical formulas from a cropped PDF page \
image into clean LaTeX. Output ONLY the transcribed LaTeX draft for the visible formula(s). Do not \
invent symbols that are illegible or missing, and do not claim this draft is a verified quote.";

static SETTINGS_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn default_equation_transcription_enabled() -> bool {
    DEFAULT_EQUATION_TRANSCRIPTION_ENABLED
}

/// Persisted opt-in setting for equation-page vision transcription (off by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PdfVisionSettings {
    #[serde(default = "default_equation_transcription_enabled")]
    pub equation_transcription_enabled: bool,
}

impl Default for PdfVisionSettings {
    fn default() -> Self {
        Self {
            equation_transcription_enabled: DEFAULT_EQUATION_TRANSCRIPTION_ENABLED,
        }
    }
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PDF_VISION_SETTINGS_FILE)
}

/// Load [`PdfVisionSettings`] from `<data_dir>/pdf_vision_settings.json` (defaults to disabled).
pub fn load_pdf_vision_settings(data_dir: &Path) -> PdfVisionSettings {
    let Ok(bytes) = fs::read(settings_path(data_dir)) else {
        return PdfVisionSettings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Atomically persist [`PdfVisionSettings`] to `<data_dir>/pdf_vision_settings.json`.
pub fn save_pdf_vision_settings(data_dir: &Path, settings: &PdfVisionSettings) -> Result<(), String> {
    fs::create_dir_all(data_dir)
        .map_err(|e| format!("failed to create data directory `{}`: {e}", data_dir.display()))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|e| format!("failed to serialize pdf vision settings: {e}"))?;
    let target = settings_path(data_dir);
    let seq = SETTINGS_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = data_dir.join(format!(
        "{PDF_VISION_SETTINGS_FILE}.{}.{}.tmp",
        std::process::id(),
        seq
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("failed to create temp pdf vision settings file: {e}"))?;
    if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to write pdf vision settings: {err}"));
    }
    drop(file);
    if let Err(err) = fs::rename(&tmp, &target) {
        if target.exists() {
            let _ = fs::remove_file(&target);
            if fs::rename(&tmp, &target).is_ok() {
                return Ok(());
            }
        }
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to commit pdf vision settings: {err}"));
    }
    Ok(())
}

/// Decoupled page-crop image source so F14c touches zero F14b files (`pdf_render.rs`).
///
/// Merge hook: implement this trait over F14b's `pdf_render::PdfPageRenderer` and pass
/// `Some(&renderer)` into [`handle_transcribe_with_provider`].
pub trait PageCropProvider {
    fn render_crop(
        &self,
        raw_pdf_bytes: &[u8],
        page: usize,
        crop: Option<[u32; 4]>,
    ) -> Result<(String, Vec<u8>), String>;
}

/// JSON request body for `POST /api/sessions/{id}/sources/{source}/transcribe-equations`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscribeEquationsRequest {
    pub page: usize,
    #[serde(default)]
    pub crop: Option<[u32; 4]>,
    #[serde(default)]
    pub opt_in: Option<bool>,
    #[serde(default)]
    pub image_media_type: Option<String>,
    #[serde(default)]
    pub image_base64: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub budget_cap: Option<u64>,
    #[serde(default)]
    pub prompt_hint: Option<String>,
}

struct VisionTurnGuard<'a> {
    state: &'a DesktopState,
    session: &'a str,
}

impl Drop for VisionTurnGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active_turns.lock() {
            active.remove(self.session);
        }
    }
}

struct CancelOnlyObserver {
    cancel: CancelToken,
}

impl StreamObserver for CancelOnlyObserver {
    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

fn matches_scope(ev: &Event, scope: RunScope<'_>) -> bool {
    match scope {
        RunScope::All => true,
        RunScope::AfterSeq(seq) => ev.seq > seq,
        RunScope::SeqRange { start_exclusive, end_inclusive } => {
            ev.seq > start_exclusive && ev.seq <= end_inclusive
        }
        RunScope::RunId(target) => match ev.payload.get(RUN_ID_FIELD) {
            Some(Value::String(s)) => s == target,
            Some(Value::Number(n)) => n.as_u64().is_some_and(|v| v.to_string() == target),
            _ => false,
        },
    }
}

/// Count prior equation-vision model calls in `scope` via `context.supplied` `page_ranges` events.
pub fn count_vision_calls(events: &[Event], scope: RunScope<'_>) -> u64 {
    events
        .iter()
        .filter(|ev| {
            ev.kind == CONTEXT_SUPPLIED_EVENT
                && matches_scope(ev, scope)
                && ev
                    .payload
                    .get("page_ranges")
                    .and_then(Value::as_array)
                    .is_some_and(|arr| !arr.is_empty())
        })
        .count() as u64
}

fn validate_model_override(raw: &str) -> Result<String, (u16, String)> {
    let m = raw.trim();
    if m.is_empty()
        || m.len() > 160
        || m.starts_with('-')
        || m.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(bad(format!("Invalid model identifier `{m}`.")));
    }
    Ok(m.to_string())
}

fn resolve_vision_model(
    state: &DesktopState,
    req_provider: Option<&str>,
    req_model: Option<&str>,
) -> Result<(ProviderKind, String), (u16, String)> {
    let (provider, default_model) = if let Some(p_raw) = req_provider.map(str::trim).filter(|s| !s.is_empty()) {
        if CliRouteKind::from_provider(&p_raw.to_ascii_lowercase()).is_some() {
            return Err(bad(
                "Equation vision transcription requires an in-core vision API provider (anthropic, openai, chatgpt, gemini, or ollama); subscription-CLI routes are not supported.",
            ));
        }
        let pk = ProviderKind::parse(p_raw).map_err(|e| bad(format!("Invalid provider `{p_raw}`: {e}")))?;
        (pk, pk.default_model().to_string())
    } else {
        resolve_in_core_role_model(state, "paper_reader", None, true).map_err(|(st, msg)| {
            if msg == NEEDS_API_PROVIDER {
                bad("Equation transcription needs a connected vision-capable API provider. Choose one in Settings -> Providers.")
            } else {
                (st, msg)
            }
        })?
    };

    let model = match req_model.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let valid = validate_model_override(raw)?;
            provider.resolve_model(&valid)
        }
        None => default_model,
    };

    if !supports_vision(provider, &model) {
        return Err(bad(format!(
            "Model `{model}` on provider `{}` is not vision-capable; refusing to send or drop page images.",
            provider.cli_name()
        )));
    }
    Ok((provider, model))
}

fn resolve_crop_image(
    req: &TranscribeEquationsRequest,
    raw_pdf_bytes: &[u8],
    crop_provider: Option<&dyn PageCropProvider>,
) -> Result<(String, Vec<u8>), (u16, String)> {
    match (&req.image_media_type, &req.image_base64) {
        (Some(mt), Some(b64)) => {
            let decoded = decode_base64_strict(b64, MAX_IMAGE_BYTES)
                .map_err(|e| bad(format!("Invalid crop image base64: {e}")))?;
            validate_image_bytes(mt, &decoded).map_err(|e| bad(e.message))?;
            Ok((mt.clone(), decoded))
        }
        (Some(_), None) | (None, Some(_)) => Err(bad(
            "Provide both `image_media_type` and `image_base64`, or omit both to use a configured page renderer.",
        )),
        (None, None) => {
            let Some(provider) = crop_provider else {
                return Err(bad(
                    "Supply a PNG, JPEG, or WebP crop via `image_media_type` + `image_base64` (no PDF page renderer configured).",
                ));
            };
            let (mt, bytes) = provider
                .render_crop(raw_pdf_bytes, req.page, req.crop)
                .map_err(|e| bad(format!("Failed to render PDF page crop: {e}")))?;
            validate_image_bytes(&mt, &bytes).map_err(|e| bad(e.message))?;
            Ok((mt, bytes))
        }
    }
}

fn valid_source_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// HTTP handler for `POST /api/sessions/{id}/sources/{source}/transcribe-equations`.
pub fn handle_transcribe_equations(
    state: &Arc<DesktopState>,
    session: &str,
    source: &str,
    body: &[u8],
) -> ApiResult {
    handle_transcribe_with_provider(state, session, source, body, None)
}

/// Core handler accepting an optional decoupled [`PageCropProvider`] (F14b merge seam).
pub fn handle_transcribe_with_provider(
    state: &Arc<DesktopState>,
    session: &str,
    source: &str,
    body: &[u8],
    crop_provider: Option<&dyn PageCropProvider>,
) -> ApiResult {
    validate_session_id(session).map_err(|e| bad(e.to_string()))?;
    if !valid_source_id(source) {
        return Err(bad("Invalid source identifier."));
    }
    let req: TranscribeEquationsRequest = serde_json::from_slice(body)
        .map_err(|e| bad(format!("Invalid equation transcription request JSON: {e}")))?;

    // Owner Decision 2026-11-26 Rule 1: off by default; require explicit opt-in.
    let settings = load_pdf_vision_settings(&state.cfg.data_dir);
    if !is_equation_transcription_opted_in(Some(settings.equation_transcription_enabled), req.opt_in) {
        return Err((
            403,
            "Equation vision transcription is off by default. Explicitly opt in (`opt_in: true` or enable equation transcription in settings) before sending page images to your vision model.".into(),
        ));
    }

    if req.page == 0 {
        return Err(bad("`page` must be a 1-based page number (>= 1)."));
    }
    if let Some([_, _, w, h]) = req.crop {
        if w == 0 || h == 0 {
            return Err(bad("`crop` must have positive width and height."));
        }
    }
    let prompt_hint = match req.prompt_hint.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(h) if h.chars().count() > MAX_PROMPT_HINT_CHARS => {
            return Err(bad(format!("`prompt_hint` exceeds {MAX_PROMPT_HINT_CHARS} characters.")));
        }
        other => other,
    };

    if !state.session_exists(session).map_err(internal)? {
        return Err(not_found("Session not found."));
    }
    let _activity = crate::session_lifecycle::reserve_turn(state, session)?;
    {
        let mut active = state
            .active_turns
            .lock()
            .map_err(|_| internal("Session lock unavailable."))?;
        if !active.insert(session.to_string()) {
            return Err((409, "Wait for the current turn or review to finish.".into()));
        }
    }
    let _turn_guard = VisionTurnGuard { state, session };
    let worker = ApiTurnGuard::new(state, session).map_err(internal)?;

    let ledger = state.open_ledger().map_err(internal)?;
    let (_, events) = ledger.read_verified_session(session).map_err(|err| match err {
        ledger::Error::UnknownSession(_) => not_found("This session has no saved ledger."),
        _ => bad("The saved session ledger could not be verified."),
    })?;

    let receipt = events
        .iter()
        .rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source))
        .map(|e| &e.payload)
        .ok_or_else(|| not_found("This session has no record of that source."))?;
    if receipt["format"].as_str() != Some("pdf") {
        return Err(bad("Equation transcription is only available for PDF sources."));
    }
    if let Some(page_count) = receipt["page_count"].as_u64().and_then(|n| usize::try_from(n).ok()) {
        if page_count > 0 && req.page > page_count {
            return Err(bad(format!(
                "Requested page {} exceeds source page_count ({page_count}).",
                req.page
            )));
        }
    }
    let raw_blob_sha256 = receipt["raw_blob"]
        .as_str()
        .filter(|s| is_sha256_hex(s))
        .ok_or_else(|| bad("Source receipt is missing a valid raw_blob SHA-256 digest."))?
        .to_string();
    let raw_pdf_bytes = ledger
        .get_blob(&raw_blob_sha256)
        .map_err(|_| bad("The stored PDF failed its checksum or is missing."))?;

    let (media_type, image_bytes) = resolve_crop_image(&req, &raw_pdf_bytes, crop_provider)?;
    let (provider, model) =
        resolve_vision_model(state, req.provider.as_deref(), req.model.as_deref())?;

    // F9 run_budget enforcement before sending any request or mutating the ledger.
    let run_id_opt = req.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let scope = run_id_opt.map_or(RunScope::All, RunScope::RunId);
    let cap = req.budget_cap.unwrap_or(DEFAULT_VISION_BUDGET_CAP);
    let used = count_model_calls(&events, scope).saturating_add(count_vision_calls(&events, scope));
    let budget = ModelCallBudget::new(cap, used);
    if !budget.can_spend(1) {
        return Err((
            429,
            format!(
                "Model call budget exhausted ({} of {} calls used).",
                budget.used, budget.cap
            ),
        ));
    }

    let img_block = build_image_block(&media_type, &image_bytes).map_err(|e| bad(e.message))?;
    let user_prompt = match prompt_hint {
        Some(hint) => format!(
            "Transcribe the mathematical formula(s) in this crop from page {} into LaTeX. Context hint: {hint}",
            req.page
        ),
        None => format!(
            "Transcribe the mathematical formula(s) in this crop from page {} into LaTeX.",
            req.page
        ),
    };
    let model_req = ModelRequest {
        model: model.clone(),
        system: Some(VISION_SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_prompt }, img_block],
        }],
        tools: Vec::new(),
        max_tokens: 1024,
        reasoning_effort: None,
    };
    validate_request_images(provider, &model_req).map_err(|e| bad(e.message))?;

    let transport = state.build_transport(provider, &model).map_err(bad)?;
    let prepared = transport.prepare_request(&model_req).map_err(|e| bad(e.to_string()))?;

    let run_id_recorded = run_id_opt
        .map(str::to_string)
        .unwrap_or_else(|| format!("pdf-vision-p{}", req.page));
    let supplied_payload = ContextSuppliedPayload::new_with_pages(
        run_id_recorded,
        VISION_CALLER_ID,
        Vec::new(),
        vec![SuppliedPageRange {
            source_id: source.to_string(),
            raw_blob_sha256: raw_blob_sha256.clone(),
            start_page: req.page,
            end_page: req.page,
            crop: req.crop,
        }],
    )
    .map_err(internal)?;

    let context_supplied_seq = {
        let _guard = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("Ledger write lock unavailable."))?;
        let val = serde_json::to_value(&supplied_payload).map_err(|e| internal(e.to_string()))?;
        ledger
            .append_with_hash(session, CONTEXT_SUPPLIED_EVENT, 1, &val)
            .map_err(|e| internal(format!("failed to append context.supplied: {e}")))?
            .seq
    };

    let mut observer = CancelOnlyObserver { cancel: worker.cancel.clone() };
    let resp = transport
        .send_prepared_streaming(&prepared, &mut observer)
        .map_err(|e| match e.kind {
            TransportErrorKind::Cancelled => (409, "Equation transcription cancelled.".into()),
            TransportErrorKind::Config => bad(e.message),
            _ => (502, format!("Vision model request failed: {e}")),
        })?;

    let mut latex_draft = String::new();
    for block in &resp.content {
        if let ContentBlock::Text { text } = block {
            if !latex_draft.is_empty() && !text.is_empty() {
                latex_draft.push('\n');
            }
            latex_draft.push_str(text);
        }
    }
    let latex_draft = latex_draft.trim().to_string();
    if latex_draft.is_empty() {
        return Err((502, "Vision model returned an empty transcription.".into()));
    }
    if latex_draft.len() > MAX_LATEX_DRAFT_BYTES {
        return Err((502, format!("Vision model transcription exceeded {MAX_LATEX_DRAFT_BYTES} bytes.")));
    }

    let env = state.research_env_for_session(session).map_err(bad)?;
    let agreement = check_text_layer_agreement(
        &env.sandbox(),
        session,
        &events,
        source,
        &raw_blob_sha256,
        req.page,
        &latex_draft,
    );

    let transcription = FormulaTranscription::new(
        source,
        &raw_blob_sha256,
        req.page,
        req.crop,
        &model,
        &latex_draft,
        agreement.agreed,
    )
    .map_err(internal)?;

    let event_seq = {
        let _guard = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("Ledger write lock unavailable."))?;
        let val = serde_json::to_value(&transcription).map_err(|e| internal(e.to_string()))?;
        ledger
            .append_with_hash(session, EVENT_PDF_TRANSCRIPTION, 1, &val)
            .map_err(|e| internal(format!("failed to append pdf.transcription: {e}")))?
            .seq
    };

    let updated_budget = ModelCallBudget::new(cap, used.saturating_add(1));
    Ok((
        200,
        json!({
            "event_seq": event_seq,
            "context_supplied_seq": context_supplied_seq,
            "transcription": transcription,
            "label": MODEL_TRANSCRIBED_LABEL,
            "quotable": false,
            "requires_text_layer_agreement": true,
            "budget": updated_budget,
        }),
    ))
}
