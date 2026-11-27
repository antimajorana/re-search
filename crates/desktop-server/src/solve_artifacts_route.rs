//! Safe authenticated Solve artifact serving route (`solve_artifacts_route.rs`, package S18 Part A).
//!
//! Serves `GET /api/sessions/{id}/solve/artifacts/{artifact_id}` with full session validation,
//! Solve-mode verification, ledger chain verification, blob SHA-256 verification, magic-byte
//! re-validation, `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox`, and
//! forced `Content-Disposition: attachment` for every type except raster PNG.

use crate::http_srv::{write_api_res, write_bytes};
use crate::routes_session::{bad, internal, not_found};
use crate::session_mode::session_mode;
use crate::state::DesktopState;
use ledger::sha256_hex;
use research_tools::paper_key::Mode;
use research_tools::solve_artifacts::{
    sniff_artifact_mime, MIME_CSV, MIME_JSON, MIME_PDF, MIME_PLAIN, MIME_PNG, MIME_SVG,
};
use research_tools::solve_state::SolveState;
use research_tools::validate_session_id;
use std::net::TcpStream;

/// Verified artifact bytes and HTTP response headers ready to write to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedSolveArtifact {
    pub content_type: String,
    pub content_disposition: String,
    pub bytes: Vec<u8>,
}

fn validate_artifact_id(artifact_id: &str) -> Result<(), (u16, String)> {
    if artifact_id.is_empty()
        || artifact_id.len() > 128
        || matches!(artifact_id, "." | "..")
        || !artifact_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(bad(format!("invalid artifact_id `{artifact_id}`")));
    }
    Ok(())
}

fn is_allowed_artifact_mime(mime: &str) -> bool {
    matches!(
        mime,
        MIME_PNG | MIME_PDF | MIME_SVG | MIME_JSON | MIME_CSV | MIME_PLAIN
    )
}

fn safe_download_filename(raw_name: &str) -> String {
    let base = raw_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("artifact")
        .trim_start_matches('.');
    let cleaned: String = base
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    if cleaned.is_empty() || matches!(cleaned.as_str(), "." | "..") {
        "artifact".to_string()
    } else {
        cleaned
    }
}

/// Load, authenticate-context check, and integrity-verify a Solve artifact for HTTP serving.
pub fn read_solve_artifact(
    state: &DesktopState,
    session_id: &str,
    artifact_id: &str,
) -> Result<ServedSolveArtifact, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    validate_artifact_id(artifact_id)?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    let ledger = state.open_ledger().map_err(internal)?;
    let events = match ledger.read_verified_session(session_id) {
        Ok((_, ev)) => ev,
        Err(ledger::Error::UnknownSession(_)) => Vec::new(),
        Err(_) => return Err(bad("The session ledger could not be verified.")),
    };
    if session_mode(&events) != Mode::Solve {
        return Err(bad("This endpoint is only available for Solve sessions."));
    }

    let solve_state = SolveState::from_events(&events);
    let artifact = solve_state
        .artifacts
        .iter()
        .rev()
        .find(|a| a.artifact_id == artifact_id)
        .ok_or_else(|| not_found(format!("artifact `{artifact_id}` not found")))?;

    if !is_allowed_artifact_mime(&artifact.mime) {
        return Err(bad("The stored artifact has an unsupported MIME type."));
    }
    if artifact.blob.is_empty() || artifact.blob != artifact.sha256 {
        return Err(bad("The artifact record has mismatched digest metadata."));
    }

    let bytes = ledger
        .get_blob(&artifact.blob)
        .map_err(|_| bad("The artifact blob is missing or failed its checksum."))?;
    if bytes.len() as u64 != artifact.bytes {
        return Err(bad("The artifact blob size does not match its ledger record."));
    }
    if sha256_hex(&bytes) != artifact.sha256 {
        return Err(bad("The artifact blob failed SHA-256 verification."));
    }

    let sniffed = sniff_artifact_mime(&bytes)
        .map_err(|_| bad("The artifact bytes failed magic-byte safety verification."))?;
    let magic_matches = match artifact.mime.as_str() {
        MIME_PNG => sniffed == MIME_PNG,
        MIME_PDF => sniffed == MIME_PDF,
        MIME_SVG => sniffed == MIME_SVG,
        MIME_JSON => sniffed == MIME_JSON,
        MIME_CSV | MIME_PLAIN => !matches!(sniffed, MIME_PNG | MIME_PDF | MIME_SVG),
        _ => false,
    };
    if !magic_matches {
        return Err(bad("The artifact bytes do not match the recorded MIME type."));
    }

    let filename = safe_download_filename(&artifact.name);
    // Inline display is strictly restricted to raster PNG; SVG/CSV/JSON/PDF/text force attachment.
    let disposition = if artifact.mime == MIME_PNG {
        format!("inline; filename=\"{filename}\"")
    } else {
        format!("attachment; filename=\"{filename}\"")
    };

    Ok(ServedSolveArtifact {
        content_type: artifact.mime.clone(),
        content_disposition: disposition,
        bytes,
    })
}

/// Write HTTP response for `GET /api/sessions/{id}/solve/artifacts/{artifact_id}`.
pub fn serve_solve_artifact(
    stream: &mut TcpStream,
    state: &DesktopState,
    session_id: &str,
    artifact_id: &str,
) {
    match read_solve_artifact(state, session_id, artifact_id) {
        Ok(served) => {
            write_bytes(
                stream,
                200,
                &served.content_type,
                &served.bytes,
                &[
                    ("Content-Disposition", served.content_disposition.as_str()),
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", "sandbox"),
                ],
            );
        }
        Err((status, msg)) => write_api_res(stream, Err((status, msg))),
    }
}
