//! Title fallback logic for Investigation sessions.
use crate::investigation::{is_investigation, latest};
use ledger::Event;
use serde_json::Value;

fn clip_title(s: &str, max_chars: usize) -> String {
    let trimmed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn strip_suffix_ignore_ascii_case<'a>(s: &'a str, suffix: &str) -> &'a str {
    if s.len() >= suffix.len() {
        let split = s.len() - suffix.len();
        if s.is_char_boundary(split) && s[split..].eq_ignore_ascii_case(suffix) {
            return &s[..split];
        }
    }
    s
}

fn clean_origin_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("upload") {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return None;
    }
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    let stripped = strip_suffix_ignore_ascii_case(base, ".pdf.extracted.txt");
    let stripped = strip_suffix_ignore_ascii_case(stripped, ".pdf").trim();
    (!stripped.is_empty()).then(|| clip_title(stripped, 80))
}

fn source_label(payload: &Value) -> Option<String> {
    if let Some(title) = payload["title"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(clip_title(title, 80));
    }
    if let Some(origin_label) = payload["origin"].as_str().and_then(clean_origin_label) {
        return Some(origin_label);
    }
    let path = payload["primary_path"].as_str().or_else(|| {
        payload["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(Value::as_str)
    })?;
    let name = path.rsplit('/').next().unwrap_or(path).trim();
    (!name.is_empty()).then(|| clip_title(name, 80))
}

pub(crate) fn reading_title(events: &[Event]) -> Option<String> {
    if !is_investigation(events) {
        return None;
    }
    events
        .iter()
        .find_map(|event| {
            matches!(
                event.kind.as_str(),
                "investigation.summary" | "investigation.map"
            )
            .then(|| event.payload["title"].as_str())
            .flatten()
            .filter(|s| !s.trim().is_empty())
        })
        .map(|title| clip_title(title, 80))
}

pub(crate) fn paper_title(events: &[Event]) -> Option<String> {
    if !is_investigation(events) {
        return None;
    }
    if let Some(paper) = latest(events, "investigation.paper") {
        if let Some(source) = events.iter().find(|e| {
            e.kind == "source.ingested" && e.payload["source_id"] == paper.payload["source_id"]
        }) {
            if let Some(label) = source_label(&source.payload) {
                return Some(label);
            }
        }
    } else {
        let mut sources = events.iter().filter(|e| e.kind == "source.ingested");
        if let (Some(only), None) = (sources.next(), sources.next()) {
            if let Some(label) = source_label(&only.payload) {
                return Some(label);
            }
        }
    }
    Some("New investigation".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes_session::session_title;
    use crate::state::{SessionMeta, DEFAULT_SESSION_TITLE};
    use ledger::Ledger;
    use serde_json::json;

    fn default_meta() -> SessionMeta {
        SessionMeta {
            id: "s-title".into(),
            title: DEFAULT_SESSION_TITLE.into(),
            allow_model_title: Some(true),
            ..SessionMeta::default()
        }
    }

    #[test]
    fn title_order_reading_then_model_then_paper_then_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let sid = "s-inv-title";
        let meta = default_meta();

        ledger
            .append(sid, "investigation.started", 1, &json!({}))
            .unwrap();
        // Single unpinned source with file name only.
        ledger
            .append(
                sid,
                "source.ingested",
                1,
                &json!({"source_id":"src-1","title":null,"primary_path":"src-1/scaling.pdf"}),
            )
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(session_title(Some(&meta), &events, sid), "scaling.pdf");

        // Pinned source with explicit document title overrides file name.
        ledger
            .append(
                sid,
                "source.ingested",
                1,
                &json!({"source_id":"src-2","title":"Quantum Hall Scaling","primary_path":"src-2/qh.pdf"}),
            )
            .unwrap();
        ledger
            .append(sid, "investigation.paper", 1, &json!({"source_id":"src-2"}))
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(
            session_title(Some(&meta), &events, sid),
            "Quantum Hall Scaling"
        );

        // Model title beats paper title.
        ledger
            .append(
                sid,
                agent::runner::SESSION_TITLE_EVENT,
                1,
                &json!({"title":"Hall Conductivity Question"}),
            )
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(
            session_title(Some(&meta), &events, sid),
            "Hall Conductivity Question"
        );

        // Reading title (summary/map) beats model title.
        ledger
            .append(
                sid,
                "investigation.map",
                1,
                &json!({"title":"Mapped Hall Result"}),
            )
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(
            session_title(Some(&meta), &events, sid),
            "Mapped Hall Result"
        );

        // Explicit user-chosen meta title beats everything without being overwritten.
        let custom = SessionMeta {
            title: "My Custom Title".into(),
            allow_model_title: Some(false),
            ..default_meta()
        };
        assert_eq!(session_title(Some(&custom), &events, sid), "My Custom Title");
        assert_eq!(meta.title, DEFAULT_SESSION_TITLE);
    }

    #[test]
    fn title_before_upload_is_new_investigation() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let sid = "s-inv-empty";
        let meta = default_meta();

        ledger
            .append(sid, "investigation.started", 1, &json!({}))
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(
            session_title(Some(&meta), &events, sid),
            "New investigation"
        );
        assert_eq!(paper_title(&events).as_deref(), Some("New investigation"));
        assert!(reading_title(&events).is_none());
    }

    #[test]
    fn research_session_title_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let sid = "s-res";
        let meta = default_meta();

        assert!(reading_title(&[]).is_none());
        assert!(paper_title(&[]).is_none());
        assert_eq!(session_title(Some(&meta), &[], sid), DEFAULT_SESSION_TITLE);

        ledger
            .append(
                sid,
                agent::runner::SESSION_TITLE_EVENT,
                1,
                &json!({"title":"Renormalization Check"}),
            )
            .unwrap();
        let events = ledger.read_session(sid).unwrap();
        assert_eq!(
            session_title(Some(&meta), &events, sid),
            "Renormalization Check"
        );

        // Handoff Research session is not treated as an Investigation.
        ledger
            .append("s-ho", "investigation.handoff", 1, &json!({}))
            .unwrap();
        let ho = ledger.read_session("s-ho").unwrap();
        assert!(paper_title(&ho).is_none());
        assert_eq!(session_title(Some(&meta), &ho, "s-ho"), DEFAULT_SESSION_TITLE);
    }

    #[test]
    fn source_label_cleans_origin_when_title_missing() {
        assert_eq!(
            source_label(&json!({
                "title": null,
                "origin": "spectral_bounds.pdf",
                "primary_path": "src-1/document.pdf.extracted.txt"
            }))
            .as_deref(),
            Some("spectral_bounds")
        );
        assert_eq!(
            source_label(&json!({
                "title": "",
                "origin": "/home/u/papers/Decay_Bound.PDF.EXTRACTED.TXT",
                "primary_path": "src-1/raw.txt"
            }))
            .as_deref(),
            Some("Decay_Bound")
        );
        assert_eq!(
            source_label(&json!({
                "title": null,
                "origin": "C:\\Users\\alice\\Downloads\\Quantum_Hall.PDF",
                "primary_path": "src-1/raw.txt"
            }))
            .as_deref(),
            Some("Quantum_Hall")
        );
        // Generic `upload` and bare HTTP(S) URLs fall back to primary_path / files[0].
        assert_eq!(
            source_label(&json!({
                "title": null,
                "origin": "upload",
                "primary_path": "src-1/fallback.pdf"
            }))
            .as_deref(),
            Some("fallback.pdf")
        );
        assert_eq!(
            source_label(&json!({
                "title": null,
                "origin": "https://arxiv.org/pdf/2501.00042",
                "files": ["src-1/arxiv_paper.pdf"]
            }))
            .as_deref(),
            Some("arxiv_paper.pdf")
        );
    }
}
