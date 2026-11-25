use super::*;
use crate::paper_fetch_meta::synthesize_versions;
use crate::paper_key::PaperKey;
use serde_json::json;

fn fake_event(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    Event {
        session: "s-fetch".into(),
        seq,
        ts_ms: 1_700_000_000_000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: ledger::GENESIS_HASH.into(),
        event_hash: ledger::GENESIS_HASH.into(),
    }
}

#[test]
fn fetched_sources_counts_only_distinct_network_and_library_origins() {
    let events = vec![
        fake_event(
            1,
            "source.ingested",
            json!({"source_id": "upload-1", "origin": "upload", "raw_blob": "a".repeat(64)}),
        ),
        fake_event(
            2,
            "source.ingested",
            json!({"source_id": "ws-1", "origin": "workspace:notes.tex", "raw_blob": "b".repeat(64)}),
        ),
        fake_event(
            3,
            "source.ingested",
            json!({"source_id": "ax-1", "origin": "https://arxiv.org/e-print/2103.01234v1", "raw_blob": "c".repeat(64)}),
        ),
        // Duplicate ingestion of the same source_id does not double-count.
        fake_event(
            4,
            "source.ingested",
            json!({"source_id": "ax-1", "origin": "https://arxiv.org/e-print/2103.01234v1", "raw_blob": "c".repeat(64)}),
        ),
        fake_event(
            5,
            "source.ingested",
            json!({"source_id": "lib-1", "origin": format!("library:{}", "d".repeat(64)), "raw_blob": "d".repeat(64)}),
        ),
        fake_event(
            6,
            "source.ingested",
            json!({"source_id": "mock-1", "origin": "http://127.0.0.1:18080/pdf/2103.01234.pdf", "raw_blob": "e".repeat(64)}),
        ),
        fake_event(7, "literature.fetched", json!({"provider": "arxiv"})),
    ];

    assert_eq!(fetched_sources(&events), 3);

    // Limit is a parameter: 4 has room, 3 is full, raising to 8 re-opens room.
    let ok = check_fetch_budget(&events, 4).unwrap();
    assert_eq!(ok, FetchBudgetStatus { used: 3, limit: 4 });
    assert_eq!(ok.remaining(), 1);
    assert!(ok.can_fetch());

    let err = check_fetch_budget(&events, 3).unwrap_err();
    assert!(err.contains("3 of 3"));

    let raised = check_fetch_budget(&events, DEFAULT_INVESTIGATE_FETCH_LIMIT).unwrap();
    assert_eq!(raised.remaining(), 5);
}

#[test]
fn parse_paper_input_normalizes_arxiv_doi_and_canonical_keys() {
    let ax = parse_paper_input("https://arxiv.org/abs/2103.01234v2").unwrap();
    assert_eq!(ax.key, PaperKey::parse("arxiv:2103.01234").unwrap());
    assert_eq!(ax.version.as_deref(), Some("v2"));

    let doi = parse_paper_input("https://doi.org/10.1000/Example.1").unwrap();
    assert_eq!(doi.key, PaperKey::parse("doi:10.1000/example.1").unwrap());
    assert_eq!(doi.ids.doi.as_deref(), Some("10.1000/example.1"));

    let inspire = parse_paper_input("inspire:451647").unwrap();
    assert_eq!(inspire.key, PaperKey::parse("inspire:451647").unwrap());
    assert_eq!(inspire.ids.inspire.as_deref(), Some("451647"));

    assert!(parse_paper_input("").is_err());
    assert!(parse_paper_input("not a paper id").is_err());
}

#[test]
fn synthesize_versions_builds_contiguous_range_capped() {
    let vs = synthesize_versions(Some("v3"), Some("2021-03-01"), Some("2022-05-09"));
    assert_eq!(vs.len(), 3);
    assert_eq!(vs[0].version, "v1");
    assert_eq!(vs[0].date.as_deref(), Some("2021-03-01"));
    assert!(!vs[0].latest);
    assert_eq!(vs[1].version, "v2");
    assert_eq!(vs[1].date, None);
    assert!(!vs[1].latest);
    assert_eq!(vs[2].version, "v3");
    assert_eq!(vs[2].date.as_deref(), Some("2022-05-09"));
    assert!(vs[2].latest);

    let capped = synthesize_versions(Some("v99999"), None, None);
    assert_eq!(capped.len(), 200);
    assert!(capped.last().unwrap().latest);
}

#[test]
fn validation_refuses_empty_html_and_corrupt_downloads_without_panic() {
    assert!(validate_eprint_download("2103.01234", "http://127.0.0.1:1/e", None, vec![]).is_err());
    assert!(
        validate_eprint_download(
            "2103.01234",
            "http://127.0.0.1:1/e",
            Some("text/html; charset=utf-8"),
            b"<html><body>Rate limit</body></html>".to_vec(),
        )
        .is_err()
    );
    assert!(
        validate_eprint_download(
            "2103.01234",
            "http://127.0.0.1:1/e",
            None,
            b"\x1f\x8b\x08\x00corrupt-gzip-payload".to_vec(),
        )
        .is_err()
    );
    assert!(
        validate_eprint_download(
            "2103.01234",
            "http://127.0.0.1:1/e",
            None,
            b"random binary \x00\xff\xfe garbage".to_vec(),
        )
        .is_err()
    );
    assert!(validate_pdf_download(None, b"%PDF-1.4\nnot actually a valid pdf xref").is_err());
    assert!(validate_pdf_download(None, b"not a pdf").is_err());
}
