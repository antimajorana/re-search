//! Integration tests for `GET /api/sessions/{id}/writeup` (package F15 Solve adapter).
//!
//! Verifies strict header-token authentication (unauth & cookie-only rejected with 401;
//! `X-ReSearch-Token` and `Authorization: Bearer` accepted), `format=tex|md|bib`,
//! `inline_marks=0|1`, Gate G7 refusal (409), unimplemented Explore/Investigate (501),
//! unknown session (404), invalid query params (400), and safe download headers.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use export::writeup::writeup_wording;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::{PaperIds, PaperRef};
use research_tools::quote_core::{verify_quote, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, Claim, ClaimForm,
    ClaimRelation, HandleRef, SetupAssumption, SolveDoneStatus, StepEntryType, StepOrigin,
    SymbolSpec,
};
use research_tools::solve_status::Tier;
use research_tools::tool_profile::{MARKER_IDEA_STARTED, MARKER_INVESTIGATION_STARTED};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

const SYNTH_PAPER: &[u8] = br"\documentclass{article}
\begin{document}
The finite sum equals 7/12 on even chains.
\end{document}
";

fn header_val<'a>(hdrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hdrs.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn seed_verified_solve_session(data_dir: &Path, sid: &str, fail_check: bool) {
    let paper = PaperRef::from_ids(
        PaperIds { arxiv: Some("2402.01234".into()), ..Default::default() },
        Some("v1".into()),
    )
    .unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper.key.clone(),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        title: "Synthetic Chain Sums".into(),
        authors: vec!["Voss, Elena".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, venue: Some("Synth J.".into()), ..Default::default() },
        first_seen_ms: 100,
        last_seen_ms: 100,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "vault-1".into(), kind: SourceKind::KandinskyVault, root: "/tmp/vault".into(),
        settings: json!({}), added_ms: 100, last_sync_ms: Some(100), state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "vault-1".into(), external_id: "voss2024route".into(),
        key: Some(paper.key.clone()), citekey: Some("voss2024route".into()),
        ids: LibraryIds::from_paper_ids(&paper.ids), version: Some("v1".into()),
        title: "Synthetic Chain Sums".into(), authors: vec!["Voss, Elena".into()],
        year: Some(2024), item_kind: "journalArticle".into(), collections: vec![],
        tags: vec![], files: vec![], state: SourceItemState::Present, updated_ms: 100,
    })
    .unwrap();

    let ledger = Ledger::open(data_dir).unwrap();
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let dims = BTreeMap::new();
    let assumps = vec![SetupAssumption { id: "a-1".into(), text: "x real".into() }];
    let math_sha = compute_math_sha256(&symbols, &dims, &assumps, "hbar = 1");
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &json!({
        "revision": 1, "statement": "Route test identity.", "task_kind": "calculate",
        "target": { "description": "Route Identity", "form": "closed_form" },
        "done": [], "conventions": "hbar = 1", "symbols": symbols, "dimensions": dims,
        "assumptions": assumps, "sources": [],
        "roles": {
            "solver": { "provider": "anthropic", "model": "s" },
            "verifier": { "provider": "openai", "model": "v" }
        },
        "tool_profile": event_kinds::SOLVE_TOOL_PROFILE_V1,
        "math_sha256": math_sha,
    })).unwrap();

    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let mtx = Mutex::new(Ledger::open(data_dir).unwrap());
    let ing = ingest_bytes(&sandbox, &mtx, &IngestRequest {
        session_id: sid, file_name: "chain.tex", origin: &format!("library:{}", paper.key),
        raw_bytes: SYNTH_PAPER, source_id_hint: Some("chain.tex"),
        section_filter: None, title: Some("Synthetic Chain Sums"),
    }).unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let q = "The finite sum equals 7/12 on even chains.";
    let vq = verify_quote(&sandbox, sid, &evs, &ing.source_id, &ing.files[0], 3, 3, q).unwrap();
    let eq_seq = ledger.append_with_hash(sid, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": vq.quote_ref.source_id, "path": vq.quote_ref.path,
        "start_line": vq.quote_ref.start_line, "end_line": vq.quote_ref.end_line,
        "file_sha256": vq.quote_ref.file_sha256, "quote_sha256": vq.quote_ref.quote_sha256,
        "quote": q, "excerpt": vq.excerpt, "source_seq": vq.source_seq,
        "caller": "user", "paper_key": paper.key,
    })).unwrap().seq;
    let qref = QuoteRef { evidence_seq: Some(eq_seq), ..vq.quote_ref };
    let e1_sha = compute_entry_sha256(None, "Cited chain lemma.", "");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-1", "entry_type": StepEntryType::ReferenceLookup.as_str(),
        "summary": "Cited chain lemma.", "details": "", "depends_on": [],
        "origin": StepOrigin::Solver, "entry_sha256": e1_sha, "source_ref": qref,
    })).unwrap();

    let srepr = "Add(x, 1)";
    let ssha = ledger::sha256_hex(srepr.as_bytes());
    let blob = ledger.put_blob(srepr.as_bytes()).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &json!({
        "name": "ans", "version": 1, "srepr_blob": blob, "srepr_sha256": ssha,
        "latex": "x + 1", "str_preview": srepr, "free_symbols": [],
        "produced_by": { "call_id": "c-1", "result_seq": 1 }, "owner_entry": "e-2", "portable": true,
    })).unwrap();
    let claim = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("ans", 1).unwrap(),
        rhs: HandleRef::new("ans", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(ssha.as_str())).unwrap();
    let e2_sha = compute_entry_sha256(Some(&csha), "Final route identity.", "");
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-2", "entry_type": StepEntryType::FinalAnswer.as_str(),
        "summary": "Final route identity.", "details": "", "depends_on": ["e-1"],
        "claims": [claim], "claim_sha256": csha, "entry_sha256": e2_sha,
        "origin": StepOrigin::Solver,
    })).unwrap();
    let outcome = if fail_check { "fail" } else { "pass" };
    let ev = json!({ "residual": if fail_check { "1" } else { "0" } });
    let ev_sha = ledger::sha256_hex(ev.to_string().as_bytes());
    ledger.append_with_hash(sid, event_kinds::SOLVE_CHECK, 1, &json!({
        "check_id": "chk-r", "claim_sha256": csha, "claim_index": 0, "math_sha256": math_sha,
        "name": "identity_symbolic", "registry_sha256": "b".repeat(64), "params": {},
        "outcome": outcome, "asked_by": "host", "receipt": { "call_id": "chk-r", "result_seq": 1 },
        "evidence": ev, "evidence_sha256": ev_sha,
    })).unwrap();
    if !fail_check {
        ledger.append_with_hash(sid, event_kinds::SOLVE_DONE, 1, &json!({
            "final_entry_id": "e-2", "own_tier": Tier::Verified, "effective_tier": Tier::Verified,
            "criteria": [], "rests_on": {}, "status": SolveDoneStatus::Finished,
            "model_calls": { "solver": 1, "verifier": 0, "workers": 0 },
        })).unwrap();
    }
}

#[test]
fn writeup_route_enforces_header_auth_formats_inline_marks_g7_and_unbuilt_modes() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    seed_verified_solve_session(&data_dir, "s-wu-ok", false);
    seed_verified_solve_session(&data_dir, "s-wu-fail", true);

    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append_with_hash("s-wu-exp", MARKER_IDEA_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash("s-wu-inv", MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // 1. Strict header-token auth: unauthenticated and cookie-only MUST be rejected (401).
    let url = "/api/sessions/s-wu-ok/writeup?format=tex&inline_marks=1";
    assert_eq!(raw_http(port, "GET", url, &[], b"").0, 401);
    assert_eq!(raw_http(port, "GET", url, &[("Cookie", cookie.as_str())], b"").0, 401);

    // 2. Bearer token and X-ReSearch-Token both work; check TeX headers & content.
    let bearer = format!("Bearer {tok}");
    let (st, hdrs, body) = raw_http(port, "GET", url, &[("Authorization", bearer.as_str())], b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    let tex = String::from_utf8(body).unwrap();
    assert!(tex.contains("x + 1 = x + 1"));
    assert!(tex.contains(r"\cite{voss2024route}"));
    assert!(tex.contains("[Verified]"));
    assert!(tex.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert_eq!(header_val(&hdrs, "content-type"), Some("application/x-tex; charset=utf-8"));
    assert_eq!(header_val(&hdrs, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header_val(&hdrs, "content-security-policy"), Some("sandbox"));
    assert_eq!(header_val(&hdrs, "cache-control"), Some("no-store"));
    assert_eq!(
        header_val(&hdrs, "content-disposition"),
        Some("attachment; filename=\"s-wu-ok-writeup.tex\"")
    );

    // 3. `inline_marks=0` drops inline `[Verified]` while keeping status + check table.
    let (st, hdrs, body) = raw_http(
        port, "GET", "/api/sessions/s-wu-ok/writeup?format=md&inline_marks=0", &auth, b"",
    );
    assert_eq!(st, 200);
    let md = String::from_utf8(body).unwrap();
    assert!(!md.contains("[Verified]"));
    assert!(md.contains("Done (final answer: Verified)"));
    assert!(md.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert_eq!(header_val(&hdrs, "content-type"), Some("text/markdown; charset=utf-8"));
    assert_eq!(
        header_val(&hdrs, "content-disposition"),
        Some("attachment; filename=\"s-wu-ok-writeup.md\"")
    );

    // 4. `format=bib` returns BibTeX with the user's `source_item.citekey`.
    let (st, hdrs, body) = raw_http(
        port, "GET", "/api/sessions/s-wu-ok/writeup?format=bib", &auth, b"",
    );
    assert_eq!(st, 200);
    let bib = String::from_utf8(body).unwrap();
    assert!(bib.contains("@article{voss2024route,"));
    assert_eq!(header_val(&hdrs, "content-type"), Some("application/x-bibtex; charset=utf-8"));

    // 5. Gate G7 refusal returns 409 Conflict.
    let (st, _, raw) = raw_http(port, "GET", "/api/sessions/s-wu-fail/writeup?format=tex", &auth, b"");
    assert_eq!(st, 409, "{}", String::from_utf8_lossy(&raw));
    let val: Value = serde_json::from_slice(&raw).unwrap();
    assert!(val["error"].as_str().unwrap().contains("Gate G7"));

    // 6. Bare Explore (no idea.stated) and skeletonless Investigate both fail closed (400).
    let (st, _, raw) = raw_http(port, "GET", "/api/sessions/s-wu-exp/writeup?format=tex", &auth, b"");
    assert_eq!(st, 400, "expected 400 for bare Explore session s-wu-exp");
    let val: Value = serde_json::from_slice(&raw).unwrap();
    assert!(val["error"].as_str().unwrap().contains("idea.stated"));

    let (st, _, raw) = raw_http(port, "GET", "/api/sessions/s-wu-inv/writeup?format=tex", &auth, b"");
    assert_eq!(st, 400, "expected 400 for skeletonless Investigate session s-wu-inv");
    let val: Value = serde_json::from_slice(&raw).unwrap();
    assert!(val["error"].as_str().unwrap().contains("investigation.skeleton"));

    // 7. Unknown session -> 404; invalid format or inline_marks -> 400.
    assert_eq!(raw_http(port, "GET", "/api/sessions/no-such-session/writeup", &auth, b"").0, 404);
    assert_eq!(raw_http(port, "GET", "/api/sessions/s-wu-ok/writeup?format=pdf", &auth, b"").0, 400);
    assert_eq!(raw_http(port, "GET", "/api/sessions/s-wu-ok/writeup?inline_marks=maybe", &auth, b"").0, 400);
}

#[test]
fn writeup_route_404_is_strictly_readonly_before_ledger_exists() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("fresh-data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];

    let (st, _, _) = raw_http(port, "GET", "/api/sessions/missing-s1/writeup", &auth, b"");
    assert_eq!(st, 404);
    assert!(!data_dir.join("ledger.db").exists(), "GET /writeup must not create ledger.db");
    assert!(!data_dir.join("blobs").exists(), "GET /writeup must not create blobs/");
    srv.shutdown();
}
