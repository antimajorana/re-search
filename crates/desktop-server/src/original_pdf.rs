use crate::routes_session::{bad, internal, not_found};
use crate::state::DesktopState;
use research_tools::pdf::{verified_page_lines, MAX_PDF_BYTES};
use research_tools::validate_session_id;

pub(crate) struct OriginalPdf {
    pub bytes: Vec<u8>,
    pub page: Option<usize>,
}

pub(crate) fn read(
    state: &DesktopState, session: &str, source: &str, path: Option<&str>, line: Option<usize>,
) -> Result<OriginalPdf, (u16, String)> {
    validate_session_id(session).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session).map_err(internal)? {
        return Err(not_found("Session not found."));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let (_, events) = ledger.read_verified_session(session)
        .map_err(|_| bad("The source record could not be verified."))?;
    let receipt = events.iter().rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source))
        .map(|e| &e.payload)
        .ok_or_else(|| not_found("This session has no record of that paper."))?;
    if receipt["format"].as_str() != Some("pdf") {
        return Err(not_found("An original PDF was not stored for this source."));
    }
    let raw = receipt["raw_blob"].as_str().ok_or_else(|| not_found("Original PDF is unavailable."))?;
    let bytes = ledger.get_blob(raw).map_err(|_| bad("The original PDF failed its checksum."))?;
    if bytes.len() > MAX_PDF_BYTES || !bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        return Err(bad("The stored document is not a supported PDF."));
    }
    let mut page = None;
    if let (Some(path), Some(line)) = (path, line) {
        if line > 0 && receipt["primary_path"].as_str() == Some(path) {
            let ranges = verified_page_lines(receipt, || Ok(bytes.clone())).unwrap_or_default();
            page = ranges.into_iter().find(|&(_, first, last)| (first..=last).contains(&line)).map(|r| r.0);
        }
    }
    Ok(OriginalPdf { bytes, page })
}
