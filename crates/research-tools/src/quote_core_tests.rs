use super::*;
use crate::lines_sent::{ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT};
use crate::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;

const SID: &str = "s-quote-core";
const WRONG_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";

struct Fixture {
    tmp: tempfile::TempDir,
    env: ResearchEnv,
    ledger: Ledger,
    source_id: String,
    path: String,
    file_sha256: String,
    source_seq: u64,
    events: Vec<Event>,
}

impl Fixture {
    fn new(text: &str) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(tmp.path());
        let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
        let ingested = ingest_bytes(
            &env.sandbox(),
            &db,
            &IngestRequest {
                session_id: SID,
                file_name: "synthetic.txt",
                origin: "unit fixture",
                raw_bytes: text.as_bytes(),
                source_id_hint: Some("src1"),
                section_filter: None,
                title: Some("Synthetic Test Document"),
            },
        )
        .unwrap();
        let ledger = db.into_inner().unwrap();
        let events = ledger.read_session(SID).unwrap();
        let path = ingested.primary_path.unwrap();
        let file_sha256 = ledger::sha256_hex(text.as_bytes());
        Self { tmp, env, ledger, source_id: ingested.source_id, path, file_sha256, source_seq: ingested.event_seq, events }
    }

    fn source_disk_path(&self) -> PathBuf {
        let base = self.tmp.path().join("sessions").join(SID).join("sources");
        self.path.split('/').fold(base, |acc, seg| acc.join(seg))
    }

    fn append_context_supplied(&mut self, run_id: &str, caller: &str, file_sha256: &str, start: usize, end: usize) {
        let payload = ContextSuppliedPayload::new(
            run_id,
            caller,
            vec![SuppliedRange {
                source_id: self.source_id.clone(),
                path: self.path.clone(),
                file_sha256: file_sha256.to_string(),
                start_line: start,
                end_line: end,
            }],
        )
        .unwrap();
        self.ledger
            .append(SID, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(&payload).unwrap())
            .unwrap();
        self.events = self.ledger.read_session(SID).unwrap();
    }
}

fn synthetic_thirty_lines() -> String {
    (1..=30)
        .map(|line| {
            if line == 20 {
                "Synthetic Lemma 20: every bounded lattice admits a unique fixed point.\n".to_string()
            } else {
                format!("Synthetic filler sentence number {line} with distinct index {line:02}.\n")
            }
        })
        .collect()
}

#[test]
fn verify_quote_exact_relocated_and_out_of_radius_without_investigation_marker() {
    let fx = Fixture::new(&synthetic_thirty_lines());
    // Confirm there is NO Investigation marker in this session, only source.ingested.
    assert!(fx.events.iter().all(|e| e.kind == "source.ingested"));

    let quote = "every bounded lattice admits a unique fixed point";
    let sb = fx.env.sandbox();

    // Exact match (no relocation).
    let exact = verify_quote(&sb, SID, &fx.events, &fx.source_id, &fx.path, 20, 20, quote).unwrap();
    assert_eq!(exact.quote_ref.start_line, 20);
    assert_eq!(exact.quote_ref.end_line, 20);
    assert_eq!(exact.quote_ref.file_sha256, fx.file_sha256);
    assert_eq!(exact.quote_ref.quote_sha256, quote_sha256(quote));
    assert_eq!(exact.quote_ref.evidence_seq, None);
    assert_eq!(exact.cited_range, None);
    assert!(exact.range_basis.is_empty());
    assert_eq!(exact.source_seq, fx.source_seq);
    assert!(exact.excerpt.contains(quote));

    // Relocated within RELOCATE_RADIUS (= 12): citing line 10 finds line 20 (distance 10).
    let relocated = verify_quote(&sb, SID, &fx.events, &fx.source_id, &fx.path, 10, 10, quote).unwrap();
    assert_eq!(relocated.quote_ref.start_line, 20);
    assert_eq!(relocated.quote_ref.end_line, 20);
    assert_eq!(relocated.cited_range, Some([10, 10]));
    assert_eq!(relocated.range_basis, vec!["relocated".to_string()]);

    // Outside RELOCATE_RADIUS (distance 18 > 12): fails with quote_not_found and true_text.
    let err = verify_quote(&sb, SID, &fx.events, &fx.source_id, &fx.path, 2, 2, quote).unwrap_err();
    assert_eq!(err.code, "quote_not_found");
    assert_eq!(err.start_line, Some(2));
    assert_eq!(err.end_line, Some(2));
    assert!(err.true_text.as_deref().unwrap().contains("2| Synthetic filler sentence number 2"));
    assert!(!err.to_string().is_empty());
}

#[test]
fn wrong_file_hash_on_disk_and_in_verify_quote_ref() {
    let fx = Fixture::new(&synthetic_thirty_lines());
    let quote = "every bounded lattice admits a unique fixed point";
    let sb = fx.env.sandbox();

    let matched = verify_quote(&sb, SID, &fx.events, &fx.source_id, &fx.path, 20, 20, quote).unwrap();

    // Valid verify_quote_ref preserves evidence_seq.
    let mut expected = matched.quote_ref.clone();
    expected.evidence_seq = Some(77);
    let rechecked = verify_quote_ref(&sb, SID, &fx.events, &expected, quote).unwrap();
    assert_eq!(rechecked.quote_ref.evidence_seq, Some(77));

    // Wrong quote_sha256 returns quote_hash_mismatch.
    let mut bad_qhash = expected.clone();
    bad_qhash.quote_sha256 = WRONG_HASH.into();
    assert_eq!(verify_quote_ref(&sb, SID, &fx.events, &bad_qhash, quote).unwrap_err().code, "quote_hash_mismatch");

    // Wrong expected file_sha256 returns source_revision_mismatch.
    let mut bad_fhash = expected.clone();
    bad_fhash.file_sha256 = WRONG_HASH.into();
    assert_eq!(
        verify_quote_ref(&sb, SID, &fx.events, &bad_fhash, quote).unwrap_err().code,
        "source_revision_mismatch"
    );

    // Tampering the file on disk causes verify_quote to fail hash verification.
    std::fs::write(fx.source_disk_path(), "Tampered replacement text on disk.\n").unwrap();
    let disk_err = verify_quote(&sb, SID, &fx.events, &fx.source_id, &fx.path, 20, 20, quote).unwrap_err();
    assert_eq!(disk_err.code, "source_unavailable");
    assert!(disk_err.message.contains("changed since ingestion"));
}

#[test]
fn accept_quote_enforces_verification_caller_and_hash_pinned_sent_lines() {
    let mut fx = Fixture::new(&synthetic_thirty_lines());
    let quote = "every bounded lattice admits a unique fixed point";
    let sb = fx.env.sandbox();

    // Unverified quote fails with VerificationFailed before lines_sent check.
    let unverified = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-a", &fx.source_id, &fx.path, 2, 2, quote).unwrap_err();
    assert!(matches!(unverified, QuoteRejection::VerificationFailed(ref f) if f.code == "quote_not_found"));

    // Verified quote whose lines were never sent fails with LinesNotSent.
    let not_sent = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-a", &fx.source_id, &fx.path, 20, 20, quote).unwrap_err();
    assert!(matches!(not_sent, QuoteRejection::LinesNotSent { start_line: 20, end_line: 20, .. }));

    // Supply lines 18..=22 to caller-a with the WRONG file_sha256: still rejected.
    fx.append_context_supplied("run-1", "caller-a", WRONG_HASH, 18, 22);
    let wrong_hash = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-a", &fx.source_id, &fx.path, 20, 20, quote).unwrap_err();
    assert!(matches!(wrong_hash, QuoteRejection::LinesNotSent { .. }));

    // Supply lines 18..=22 to caller-a with the REAL file_sha256: accepted for caller-a,
    // still rejected for caller-b!
    let real_hash = fx.file_sha256.clone();
    fx.append_context_supplied("run-1", "caller-a", &real_hash, 18, 22);

    let ok = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-a", &fx.source_id, &fx.path, 20, 20, quote).unwrap();
    assert_eq!(ok.quote_ref.start_line, 20);

    let other = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-b", &fx.source_id, &fx.path, 20, 20, quote).unwrap_err();
    assert!(matches!(other, QuoteRejection::LinesNotSent { ref caller, .. } if caller == "caller-b"));

    let precomputed = lines_sent_with_ledger(&fx.events, Some(&fx.ledger), Some("run-1"));
    assert!(accept_quote_with_sent(&sb, SID, &fx.events, &precomputed, "caller-a", &fx.source_id, &fx.path, 20, 20, quote).is_ok());

    // Quote cited at line 10 relocates to line 20: supplying only lines 8..=12 to caller-c
    // must NOT accept it because the matched line (20) was never sent to caller-c.
    fx.append_context_supplied("run-1", "caller-c", &real_hash, 8, 12);
    let relocated_unsent = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-c", &fx.source_id, &fx.path, 10, 10, quote).unwrap_err();
    assert!(matches!(relocated_unsent, QuoteRejection::LinesNotSent { start_line: 20, end_line: 20, ref caller, .. } if caller == "caller-c"));

    // Crediting via read_paper_lines tool.request/tool.result accepts for caller-d, not caller-e.
    fx.ledger.append(SID, "tool.request", 1, &json!({
        "call_id": "rpl-1", "caller": "caller-d", "tool": "read_paper_lines",
        "args": { "inline": json!({"source_id": fx.source_id, "start_line": 19, "end_line": 21}).to_string() }
    })).unwrap();
    fx.ledger.append(SID, "tool.result", 1, &json!({
        "call_id": "rpl-1", "ok": true,
        "result": { "inline": json!({
            "a_meta": { "path": fx.path, "file_sha256": real_hash, "start_line": 19, "end_line": 21 },
            "text": "19\t...\n20\t...\n21\t...\n"
        }).to_string() }
    })).unwrap();
    fx.events = fx.ledger.read_session(SID).unwrap();
    assert!(accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-d", &fx.source_id, &fx.path, 20, 20, quote).is_ok());
    let not_d = accept_quote(&sb, SID, &fx.events, Some(&fx.ledger), Some("run-1"), "caller-e", &fx.source_id, &fx.path, 20, 20, quote).unwrap_err();
    assert!(matches!(not_d, QuoteRejection::LinesNotSent { ref caller, .. } if caller == "caller-e"));
}

#[test]
fn evidence_payload_constructors_round_trip_validation_and_ids() {
    assert_eq!(QUOTE_MATCHED_WORD, "matched");
    assert_eq!(EVIDENCE_QUOTE_EVENT, "evidence.quote");
    assert_eq!(EVIDENCE_ABSTRACT_EVENT, "evidence.abstract");

    // ID formatting and parsing.
    assert_eq!(quote_id(1).unwrap(), "xe-1");
    assert_eq!(quote_id(42).unwrap(), "xe-42");
    assert!(quote_id(0).is_err());
    assert_eq!(parse_quote_id("xe-42"), Some(42));
    for bad in ["xe-0", "xe-01", "xa-1", "xe-"] {
        assert_eq!(parse_quote_id(bad), None);
    }

    assert_eq!(abstract_id(9).unwrap(), "xa-9");
    assert!(abstract_id(0).is_err());
    assert_eq!(parse_abstract_id("xa-9"), Some(9));
    for bad in ["xa-0", "xa-09", "xe-9", "xa-"] {
        assert_eq!(parse_abstract_id(bad), None);
    }

    assert_eq!(quote_sha256("  alpha\n\t beta   gamma  "), quote_sha256("alpha beta gamma"));

    let fx = Fixture::new(&synthetic_thirty_lines());
    let quote = "every bounded lattice admits a unique fixed point";
    let verified = verify_quote(&fx.env.sandbox(), SID, &fx.events, &fx.source_id, &fx.path, 20, 20, quote).unwrap();
    let paper_key = PaperKey::from_arxiv("2401.00001").unwrap();

    let eq = EvidenceQuotePayload::from_verified(
        &verified,
        quote,
        "worker:reader:1",
        EvidenceModeMeta {
            paper_key: Some(paper_key.clone()),
            role: Some(EvidenceRole::StatesResult),
            subclaim_ids: vec!["sc-1".into(), "sc-12".into()],
            gist: Some("  Fixed point existence on bounded lattices.  ".into()),
        },
    )
    .unwrap();
    assert_eq!(eq.gist.as_deref(), Some("Fixed point existence on bounded lattices."));
    let decoded: EvidenceQuotePayload = serde_json::from_str(&serde_json::to_string(&eq).unwrap()).unwrap();
    assert_eq!(decoded, eq);
    assert!(decoded.validate().is_ok());

    let qref = eq.to_quote_ref(15);
    assert_eq!((qref.evidence_seq, qref.start_line, qref.end_line), (Some(15), 20, 20));

    // Validation failures on EvidenceQuotePayload.
    assert!(EvidenceQuotePayload::from_verified(&verified, "wrong text", "caller", EvidenceModeMeta::default()).is_err());
    assert!(EvidenceQuotePayload::from_verified(&verified, quote, "caller", EvidenceModeMeta { subclaim_ids: vec!["sc-0".into()], ..Default::default() }).is_err());
    assert!(EvidenceQuotePayload::from_verified(&verified, quote, "caller", EvidenceModeMeta { gist: Some("   ".into()), ..Default::default() }).is_err());
    assert!(EvidenceQuotePayload::from_verified(&verified, quote, "caller", EvidenceModeMeta { gist: Some("g".repeat(MAX_GIST_CHARS + 1)), ..Default::default() }).is_err());

    let mut broken = eq.clone();
    broken.source_seq = 0;
    assert!(broken.validate().is_err());
    let mut broken_hash = eq.clone();
    broken_hash.file_sha256 = "short".into();
    assert!(broken_hash.validate().is_err());
    let mut broken_excerpt = eq.clone();
    broken_excerpt.excerpt = "e".repeat(MAX_EXCERPT_BYTES + 1);
    assert!(broken_excerpt.validate().is_err());

    // Abstract payload constructor, round-trip, and validation failures.
    let abs = EvidenceAbstractPayload::new(
        paper_key.clone(),
        "We prove a synthetic convergence bound for iterative maps.",
        0,
        58,
        WRONG_HASH,
        "arxiv",
        Some(EvidenceRole::Method),
        vec!["sc-2".into()],
        Some("Synthetic convergence method".into()),
    )
    .unwrap();
    let abs_dec: EvidenceAbstractPayload = serde_json::from_str(&serde_json::to_string(&abs).unwrap()).unwrap();
    assert_eq!(abs_dec, abs);
    assert!(abs_dec.validate().is_ok());

    assert!(EvidenceAbstractPayload::new(paper_key.clone(), "Valid quote", 10, 10, WRONG_HASH, "arxiv", None, vec![], None).is_err());
    assert!(EvidenceAbstractPayload::new(paper_key.clone(), "Valid quote", 0, 5, "not-hex", "arxiv", None, vec![], None).is_err());
    assert!(EvidenceAbstractPayload::new(paper_key, "q".repeat(MAX_DISPLAY_QUOTE_CHARS + 1), 0, 10, WRONG_HASH, "arxiv", None, vec![], None).is_err());

    // Deny unknown fields on both payloads.
    let mut extra_eq = serde_json::to_value(&eq).unwrap();
    extra_eq.as_object_mut().unwrap().insert("extra".into(), json!(1));
    assert!(serde_json::from_value::<EvidenceQuotePayload>(extra_eq).is_err());

    let mut extra_abs = serde_json::to_value(&abs).unwrap();
    extra_abs.as_object_mut().unwrap().insert("extra".into(), json!(1));
    assert!(serde_json::from_value::<EvidenceAbstractPayload>(extra_abs).is_err());
}

#[test]
fn clip_quote_respects_300_char_limit_and_multibyte_utf8_boundaries() {
    let ascii = "a".repeat(350);
    let clipped_ascii = clip_quote(&ascii);
    assert_eq!(clipped_ascii.chars().count(), MAX_DISPLAY_QUOTE_CHARS);
    assert_eq!(clipped_ascii.len(), MAX_DISPLAY_QUOTE_CHARS);

    let short = "short synthetic quote";
    assert_eq!(clip_quote(short), short);
    assert_eq!(clip_to_chars(short, 0), "");
    assert_eq!(clip_to_chars("", 10), "");

    // Mix Greek math (2-3 bytes), CJK (3 bytes), and emoji (4 bytes).
    let long_utf8 = "αβ∑∫量子🧪🔬".repeat(50); // 400 Unicode scalar values
    let clipped = clip_quote(&long_utf8);
    assert_eq!(clipped.chars().count(), MAX_DISPLAY_QUOTE_CHARS);
    assert!(long_utf8.starts_with(clipped));
    assert!(clipped.len() > MAX_DISPLAY_QUOTE_CHARS);
    assert!(std::str::from_utf8(clipped.as_bytes()).is_ok());

    let sample = "λ=∑量子🧪";
    for max in 0..=sample.chars().count() + 2 {
        let part = clip_to_chars(sample, max);
        assert!(part.chars().count() <= max);
        assert!(sample.starts_with(part));
    }
}
