//! Evaluation plan Set B (00-overview.md §7.2, §7.3; packages F10, F13a, F13b):
//!
//! Exercises the fixed-tier synthetic library fixtures (`tests/fixtures/eval/b/` and
//! `manifest-b.json`): 30-entry Kandinsky vault + `refs/refs.bib`, plain folder of PDFs/TeX,
//! and standalone `.bib` file across all 7 planted cases, verifying idempotency (`sync_second_changes`),
//! planted states (`sync_planted_case_mismatches`), read-only byte identity (`sync_source_folder_diffs`),
//! and session quote independence (`sync_removed_source_quote_breaks`).

mod common;
mod eval_set_b_support;

use common::test_cfg;
use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::routes_sources::handle_sources;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use eval_set_b_support::manifest::{parse_manifest, validate_manifest, Case, Input, LibrarySourceKind, Set, Tier};
use eval_set_b_support::measures::{evaluate, Outcome};
use eval_set_b_support::{copy_dir_recursive, eval_fixtures_dir, hash_tree};
use ledger::Ledger;
use research_tools::library::{Library, SourceItemState, SourceKind};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::{attach_source_paper_to_session, FILE_ROLE_PDF, FILE_ROLE_TEX};
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::source_dedupe::DuplicateConfidence;
use research_tools::source_sync::{preview_source, register_source, remove_source, sync_source, SyncOptions};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn load_set_b_cases() -> Vec<Case> {
    let text = fs::read_to_string(eval_fixtures_dir().join("manifest-b.json")).unwrap();
    let m = parse_manifest(&text).unwrap();
    assert_eq!(m.set, Set::B);
    assert_eq!(m.tier, Tier::Fixed);
    assert_eq!(validate_manifest(&m), Vec::<String>::new());
    m.cases
}

fn find_case<'a>(cases: &'a [Case], id: &str) -> &'a Case {
    cases.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("missing case `{id}`"))
}

fn assert_items_match_key(lib: &Library, items: &[research_tools::library::SourceItemRecord], key: &Value) {
    let expected = key["items"].as_array().unwrap();
    assert_eq!(items.len(), expected.len(), "synced item count mismatch");
    for exp in expected {
        let ext_id = exp["external_id"].as_str().unwrap();
        let got = items.iter().find(|i| i.external_id == ext_id).unwrap_or_else(|| panic!("missing `{ext_id}`"));
        if let Some(ck) = exp.get("citekey").and_then(Value::as_str) {
            assert_eq!(got.citekey.as_deref(), Some(ck), "{ext_id}: citekey mismatch");
        }
        match exp["expected_key"].as_str() {
            Some(ek) => {
                let want = PaperKey::parse(ek).unwrap();
                let got_key = got.key.as_ref().expect("expected paper key");
                let resolved = lib.resolve_key(&want).unwrap().unwrap_or_else(|| want.clone());
                assert!(got_key == &want || got_key == &resolved, "{ext_id}: got {got_key}, want {want}");
            }
            None => assert!(got.key.is_none(), "{ext_id}: expected no key, got {:?}", got.key),
        }
        let want_state = match exp["expected_initial_state"].as_str().unwrap() {
            "present" => SourceItemState::Present,
            "possible_duplicate" => SourceItemState::PossibleDuplicate,
            "missing" => SourceItemState::Missing,
            other => panic!("unexpected state `{other}`"),
        };
        assert_eq!(got.state, want_state, "{ext_id}: initial state mismatch");
    }
}

fn verify_quote_survives_source_removal(
    lib: Library,
    data_dir: &Path,
    source_id: &str,
    source_dir_to_delete: &Path,
    session_id: &str,
    paper_key_str: &str,
    needle: &str,
) {
    let pk = PaperKey::parse(paper_key_str).unwrap();
    let ledger = Mutex::new(Ledger::open(data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let ing = attach_source_paper_to_session(&lib, &sandbox, &ledger, session_id, &pk, None, None).unwrap();
    let rel_path = ing.files.iter().find(|f| f.ends_with(".txt")).unwrap_or(&ing.files[0]).clone();
    let full = data_dir.join("sessions").join(session_id).join("sources").join(&rel_path);
    let body = fs::read_to_string(&full).unwrap();
    let (line_no, line_txt) = body
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .map(|(idx, l)| (idx + 1, l.trim().to_string()))
        .unwrap_or_else(|| panic!("quote needle `{needle}` not found in `{}`", full.display()));
    let ev_before = ledger.lock().unwrap().read_session(session_id).unwrap();
    let q1 = verify_quote(&sandbox, session_id, &ev_before, &ing.source_id, &rel_path, line_no, line_no, &line_txt).unwrap();

    // Remove source from library, delete library.db, and delete the source directory on disk.
    assert!(remove_source(&lib, source_id).unwrap());
    drop(lib);
    fs::remove_dir_all(data_dir.join("library")).unwrap();
    fs::remove_dir_all(source_dir_to_delete).unwrap();

    let ev_after = ledger.lock().unwrap().read_session(session_id).unwrap();
    let q2 = verify_quote(&sandbox, session_id, &ev_after, &ing.source_id, &rel_path, line_no, line_no, &line_txt).unwrap();
    assert_eq!(q1.quote_ref.quote_sha256, q2.quote_ref.quote_sha256);
    assert!(ledger.lock().unwrap().verify_session(session_id).is_ok());
}

#[test]
fn set_b_kandinsky_vault_30_entries_planted_cases_and_quote_independence() {
    let cases = load_set_b_cases();
    let vault_case = find_case(&cases, "B-eval-vault");
    assert!(matches!(
        &vault_case.input,
        Input::LibraryFixture { source_kind: LibrarySourceKind::KandinskyVault, .. }
    ));

    let tmp = TempDir::new().unwrap();
    copy_dir_recursive(&eval_fixtures_dir().join("b"), tmp.path());
    fs::write(tmp.path().join("outside_escape.pdf"), b"%PDF-1.4\noutside\n").unwrap();
    let vault = tmp.path().join("vault");
    let shared = tmp.path().join("shared_pdfs");
    let tree_vault_before = hash_tree(&vault);
    let tree_shared_before = hash_tree(&shared);

    let data_dir = tmp.path().join("data_main");
    let lib = Library::open(&data_dir).unwrap();

    let (_, preview) = preview_source(&lib, SourceKind::KandinskyVault, &vault, json!({}), None, true).unwrap();
    assert!(preview.dry_run && preview.total_items == 30);
    assert!(lib.list_sources().unwrap().is_empty());

    let src = register_source(&lib, SourceKind::KandinskyVault, &vault, json!({}), None).unwrap();
    let r1 = sync_source(&lib, &src.id, &SyncOptions { dry_run: false, hash_files: true }).unwrap();
    assert_eq!((r1.total_items, r1.added, r1.updated, r1.moved, r1.missing), (30, 30, 0, 0, 0));
    assert_eq!(r1.with_tex, 1);
    assert_eq!(r1.exact_duplicates, vault_case.answer_key["expected_exact_duplicates"].as_u64().unwrap() as usize);
    assert_eq!(r1.possible_duplicates, vault_case.answer_key["expected_possible_duplicates"].as_u64().unwrap() as usize);
    assert!(r1.warnings.iter().any(|w| w.path.contains("b2025escape") && w.reason.contains("escapes")));
    let esc = r1.items.iter().find(|i| i.external_id == "b2025escape").unwrap();
    assert!(esc.files.iter().all(|f| f.role != FILE_ROLE_PDF));
    let tex_item = r1.items.iter().find(|i| i.external_id == "b2025texpref").unwrap();
    assert!(tex_item.files.iter().any(|f| f.role == FILE_ROLE_TEX));
    assert_items_match_key(&lib, &r1.items, &vault_case.answer_key);

    // Fuzzy duplicate pair stays flagged without merging canonical keys.
    assert_eq!(r1.duplicate_groups.iter().filter(|g| g.confidence == DuplicateConfidence::Possible).count(), 1);
    assert_ne!(
        lib.resolve_key(&PaperKey::parse("arxiv:2403.08801").unwrap()).unwrap(),
        lib.resolve_key(&PaperKey::parse("doi:10.9999/setb.jour.028").unwrap()).unwrap()
    );

    // Second sync changes 0 rows and leaves vault + external PDF root byte-identical.
    let r2 = sync_source(&lib, &src.id, &SyncOptions::default()).unwrap();
    assert_eq!((r2.added, r2.updated, r2.moved, r2.missing, r2.unchanged), (0, 0, 0, 0, 30));
    assert_eq!(hash_tree(&vault), tree_vault_before);
    assert_eq!(hash_tree(&shared), tree_shared_before);

    // Quote independence after removing source and deleting library + vault copy.
    let qt = &vault_case.answer_key["quote_check_target"];
    verify_quote_survives_source_removal(
        lib,
        &data_dir,
        &src.id,
        &vault,
        "s-setb-vault",
        qt["paper_key"].as_str().unwrap(),
        qt["quote"].as_str().unwrap(),
    );

    // Planted note edit + entry delete on a fresh copy.
    let tmp_mut = TempDir::new().unwrap();
    copy_dir_recursive(&eval_fixtures_dir().join("b"), tmp_mut.path());
    let v_mut = tmp_mut.path().join("vault");
    let lib_mut = Library::open(&tmp_mut.path().join("data_mut")).unwrap();
    let src_mut = register_source(&lib_mut, SourceKind::KandinskyVault, &v_mut, json!({}), None).unwrap();
    sync_source(&lib_mut, &src_mut.id, &SyncOptions { dry_run: false, hash_files: true }).unwrap();

    let note_p01 = v_mut.join("refs/papers/b2025p01/b2025p01.md");
    let edited = fs::read_to_string(&note_p01)
        .unwrap()
        .replace("Synthetic Vault Spectral Note 01", "Edited Vault Spectral Note 01");
    fs::write(&note_p01, edited).unwrap();
    fs::remove_dir_all(v_mut.join("refs/papers/b2025p02")).unwrap();

    let r3 = sync_source(&lib_mut, &src_mut.id, &SyncOptions::default()).unwrap();
    assert_eq!((r3.updated, r3.missing), (1, 1));
    assert_eq!(
        lib_mut.get_paper(&PaperKey::parse("doi:10.9999/setb.vault.001").unwrap()).unwrap().unwrap().paper.title,
        "Edited Vault Spectral Note 01"
    );
    assert_eq!(r3.items.iter().find(|i| i.external_id == "b2025p02").unwrap().state, SourceItemState::Missing);
}

#[test]
fn set_b_folder_bibtex_and_f8_gate_measures_all_pass() {
    let cases = load_set_b_cases();
    assert_eq!(cases.len(), 3);
    let folder_case = find_case(&cases, "B-eval-folder");
    let bib_case = find_case(&cases, "B-eval-bibtex");

    // 1. Plain folder tested via desktop-server `/api/sources` route + moved/deleted planted cases.
    let tmp_f = TempDir::new().unwrap();
    copy_dir_recursive(&eval_fixtures_dir().join("b/folder"), &tmp_f.path().join("folder"));
    let folder = tmp_f.path().join("folder");
    let tree_folder_before = hash_tree(&folder);

    let cfg = test_cfg(&tmp_f.path().join("srv_data"), None, None, None);
    let auth = AuthGuard::new(18_435).unwrap();
    let state = Arc::new(DesktopState::new(
        cfg.clone(),
        auth,
        Arc::new(SseHub::new()),
        ChatGptHub::new(&cfg).unwrap(),
    ));
    let empty_q = HashMap::new();

    let accept_body = serde_json::to_vec(&json!({
        "kind": "folder",
        "root": folder.to_string_lossy(),
        "id": "setb-folder",
        "accept": true,
        "hash_files": true
    }))
    .unwrap();
    let (code, body) = handle_sources(&state, "POST", &[], &empty_q, &accept_body).unwrap();
    assert_eq!(code, 201);
    assert_eq!(body["report"]["total_items"], json!(7));
    assert_eq!(body["report"]["exact_duplicates"], json!(1));
    assert_eq!(body["report"]["possible_duplicates"], json!(1));
    assert_eq!(body["report"]["with_tex"], json!(1));

    let lib_f = Library::open(&cfg.data_dir).unwrap();
    let synced_f = lib_f.list_source_items_for_source("setb-folder").unwrap();
    assert_items_match_key(&lib_f, &synced_f, &folder_case.answer_key);

    // Second sync via route changes 0 rows and folder stays byte-identical.
    let (code2, resync) = handle_sources(&state, "POST", &["setb-folder", "sync"], &empty_q, b"{}").unwrap();
    assert_eq!(code2, 200);
    assert_eq!(resync["report"]["unchanged"], json!(7));
    assert_eq!(
        (
            resync["report"]["added"].as_u64().unwrap(),
            resync["report"]["updated"].as_u64().unwrap(),
            resync["report"]["moved"].as_u64().unwrap(),
            resync["report"]["missing"].as_u64().unwrap(),
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(hash_tree(&folder), tree_folder_before);

    // Move file across folders (recognized by SHA-256) and delete another.
    fs::create_dir_all(folder.join("archive")).unwrap();
    fs::rename(folder.join("quantum/movable_study.pdf"), folder.join("archive/movable_relocated.pdf")).unwrap();
    fs::remove_file(folder.join("condensed/deletable_note.pdf")).unwrap();
    let (code3, mut_rep) = handle_sources(&state, "POST", &["setb-folder", "sync"], &empty_q, b"{}").unwrap();
    assert_eq!(code3, 200);
    assert_eq!(mut_rep["report"]["moved"], json!(1));
    assert_eq!(mut_rep["report"]["missing"], json!(1));
    assert_eq!(mut_rep["report"]["added"], json!(0));
    let moved_item = mut_rep["report"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["external_id"] == "quantum/movable_study.pdf")
        .unwrap();
    assert_eq!(moved_item["state"], json!("present"));
    assert_eq!(moved_item["collections"], json!(["archive"]));

    // 2. Standalone `.bib` adapter across all 7 planted cases + quote independence.
    let tmp_b = TempDir::new().unwrap();
    copy_dir_recursive(&eval_fixtures_dir().join("b/standalone"), &tmp_b.path().join("standalone"));
    fs::write(tmp_b.path().join("outside_escape.pdf"), b"%PDF-1.4\noutside\n").unwrap();
    let st_dir = tmp_b.path().join("standalone");
    let bib_path = st_dir.join("refs.bib");
    let tree_bib_before = hash_tree(&st_dir);

    let data_bib = tmp_b.path().join("data_bib");
    let lib_b = Library::open(&data_bib).unwrap();
    let src_b = register_source(&lib_b, SourceKind::Bibtex, &bib_path, json!({}), None).unwrap();
    let rb1 = sync_source(&lib_b, &src_b.id, &SyncOptions { dry_run: false, hash_files: true }).unwrap();
    assert_eq!(rb1.total_items, 10);
    assert_eq!(rb1.exact_duplicates, 2);
    assert_eq!(rb1.possible_duplicates, 1);
    assert!(rb1.items.iter().find(|i| i.external_id == "bibescape2025").unwrap().files.is_empty());
    assert_items_match_key(&lib_b, &rb1.items, &bib_case.answer_key);

    let rb2 = sync_source(&lib_b, &src_b.id, &SyncOptions::default()).unwrap();
    assert_eq!((rb2.added, rb2.updated, rb2.moved, rb2.missing, rb2.unchanged), (0, 0, 0, 0, 10));
    assert_eq!(hash_tree(&st_dir), tree_bib_before);

    // Apply move + edit + delete on a second copy of standalone `.bib`.
    let tmp_bm = TempDir::new().unwrap();
    copy_dir_recursive(&eval_fixtures_dir().join("b/standalone"), &tmp_bm.path().join("standalone"));
    let stm = tmp_bm.path().join("standalone");
    let lib_bm = Library::open(&tmp_bm.path().join("data_bm")).unwrap();
    let src_bm = register_source(&lib_bm, SourceKind::Bibtex, &stm.join("refs.bib"), json!({}), None).unwrap();
    sync_source(&lib_bm, &src_bm.id, &SyncOptions { dry_run: false, hash_files: true }).unwrap();

    fs::create_dir_all(stm.join("pdfs/moved")).unwrap();
    fs::rename(stm.join("pdfs/old/bib_move.pdf"), stm.join("pdfs/moved/bib_move_relocated.pdf")).unwrap();
    fs::remove_file(stm.join("pdfs/bib_del.pdf")).unwrap();
    let edited_bib = fs::read_to_string(stm.join("refs.bib"))
        .unwrap()
        .replace("Synthetic Standalone BibTeX Study", "Edited Standalone BibTeX Study")
        .replace("pdfs/old/bib_move.pdf", "pdfs/moved/bib_move_relocated.pdf");
    fs::write(stm.join("refs.bib"), edited_bib).unwrap();
    let rbm = sync_source(&lib_bm, &src_bm.id, &SyncOptions::default()).unwrap();
    assert_eq!((rbm.moved, rbm.updated, rbm.missing), (1, 1, 1));

    let qt_b = &bib_case.answer_key["quote_check_target"];
    verify_quote_survives_source_removal(
        lib_b,
        &data_bib,
        &src_b.id,
        &st_dir,
        "s-setb-bib",
        qt_b["paper_key"].as_str().unwrap(),
        qt_b["quote"].as_str().unwrap(),
    );

    // 3. Record all four F8 Set B measures and verify every Set B pass rule passes.
    let values = BTreeMap::from([
        ("sync_second_changes".to_string(), 0.0),
        ("sync_planted_case_mismatches".to_string(), 0.0),
        ("sync_source_folder_diffs".to_string(), 0.0),
        ("sync_removed_source_quote_breaks".to_string(), 0.0),
    ]);
    let report = evaluate(&values).unwrap();
    for id in [
        "sync_second_changes",
        "sync_planted_case_mismatches",
        "sync_source_folder_diffs",
        "sync_removed_source_quote_breaks",
    ] {
        assert_eq!(report.outcome_of(id), Some(Outcome::Pass), "{id}");
    }
}
