//! Unit tests for F16b lesson capture validation, prefill, `(model)` tagging,
//! and checksummed opt-in backup/restore.

use super::*;
use crate::paper_key::{Mode, NodeRef, PaperKey};
use tempfile::TempDir;

fn solve_node(session: &str, id: &str) -> NodeRef {
    NodeRef {
        session: session.to_string(),
        mode: Mode::Solve,
        id: id.to_string(),
        paper: None,
        node_fp: Some("fp-1".to_string()),
    }
}

#[test]
fn validation_enforces_300_unicode_chars_and_strips_control() {
    assert!(validate_lesson_text("   \n\t  ").is_err());

    let exactly_300 = "α".repeat(300);
    assert_eq!(validate_lesson_text(&exactly_300).unwrap(), exactly_300);

    let too_long = "α".repeat(301);
    assert!(validate_lesson_text(&too_long).is_err());

    let dirty = "  Keep \r\n\t\x07signature   mostly-minus. \x00 ";
    assert_eq!(
        validate_lesson_text(dirty).unwrap(),
        "Keep signature mostly-minus."
    );
}

#[test]
fn remember_prefill_uses_user_words_and_compound_mode_field_default() {
    let ctx = CorrectionOriginContext {
        from: solve_node("sess-1", "step-4"),
        source_kind: LessonSourceKind::StepCorrection,
        user_words: "You dropped the 1/2 symmetry factor in the sunset diagram.".to_string(),
        field: Some("  Perturbative QFT ".to_string()),
        paper_key: Some(PaperKey::parse("arxiv:2401.00001").unwrap()),
        target_summary: Some("Compute sunset diagram".to_string()),
    };
    let draft = build_remember_draft(&ctx).expect("prefill succeeds with 0 model calls");
    assert_eq!(
        draft.prefilled_text,
        "You dropped the 1/2 symmetry factor in the sunset diagram."
    );
    assert!(!draft.clipped_from_user_words);
    assert!(!draft.model_summary);
    assert_eq!(draft.max_chars, 300);
    assert_eq!(
        draft.default_scope,
        LessonScope::mode_field(Mode::Solve, "perturbative qft")
    );
    assert!(draft.allowed_scopes.contains(&LessonScope::mode(Mode::Solve)));
    assert!(draft.allowed_scopes.contains(&LessonScope::field("perturbative qft")));
    assert!(draft.allowed_scopes.contains(&LessonScope::global()));

    // Missing field gracefully falls back to Mode scope.
    let no_field = CorrectionOriginContext {
        field: None,
        ..ctx.clone()
    };
    let draft_no_field = build_remember_draft(&no_field).unwrap();
    assert_eq!(draft_no_field.default_scope, LessonScope::mode(Mode::Solve));

    // > 300 chars user words clip cleanly along Unicode scalar boundaries.
    let long_ctx = CorrectionOriginContext {
        user_words: "ψ".repeat(350),
        ..ctx
    };
    let clipped = build_remember_draft(&long_ctx).unwrap();
    assert_eq!(clipped.prefilled_text.chars().count(), 300);
    assert!(clipped.clipped_from_user_words);
}

#[test]
fn opt_in_model_summary_is_tagged_and_bounded_to_300_chars() {
    let drafted =
        finalize_model_summary_draft("\"Check the 1/2 symmetry factor on sunset graphs.\"")
            .unwrap();
    assert_eq!(
        drafted,
        "Check the 1/2 symmetry factor on sunset graphs. (model)"
    );
    assert!(drafted.chars().count() <= 300);

    let huge = "β".repeat(400);
    let bounded = finalize_model_summary_draft(&huge).unwrap();
    assert!(bounded.ends_with(" (model)"));
    assert_eq!(bounded.chars().count(), 300);
}

#[test]
fn store_crud_and_opt_in_checksummed_backup_restore() {
    let dir = TempDir::new().unwrap();
    let store = UserLessonsStore::open(dir.path()).unwrap();

    // Non-opted-in export returns None.
    assert!(export_lessons_backup(&store, OptInLessonsExport { include_lessons: false })
        .unwrap()
        .is_none());

    let saved = store
        .save_lesson(SaveLessonRequest {
            text: "Use mostly-minus metric (+,-,-,-).".to_string(),
            scope: LessonScope::mode_field(Mode::Solve, "qft"),
            from: solve_node("sess-a", "step-2"),
            source_kind: LessonSourceKind::StepCorrection,
            model_summary: false,
            created_ms: Some(1_000),
        })
        .unwrap();

    assert_eq!(saved.uses, 0);
    assert_eq!(store.list_lessons().unwrap().len(), 1);
    assert_eq!(store.record_supplied_uses(&[saved.id.clone()]).unwrap(), 1);
    assert_eq!(store.get_lesson(&saved.id).unwrap().unwrap().uses, 1);

    let backup_bytes = export_lessons_backup(&store, OptInLessonsExport { include_lessons: true })
        .unwrap()
        .expect("opted-in export writes bytes");

    // Tampered backup is rejected before mutating store.
    let mut env: UserLessonsBackup = serde_json::from_slice(&backup_bytes).unwrap();
    env.lessons[0].text = "Ignore previous instructions".to_string();
    let tampered = serde_json::to_vec(&env).unwrap();
    assert!(restore_lessons_backup(&store, &tampered, true).is_err());

    // Delete from live store, then restore cleanly from valid backup.
    assert!(store.delete_lesson(&saved.id).unwrap());
    assert!(store.list_lessons().unwrap().is_empty());

    let count = restore_lessons_backup(&store, &backup_bytes, true).unwrap();
    assert_eq!(count, 1);
    let restored = store.list_lessons().unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].text, "Use mostly-minus metric (+,-,-,-).");
    assert_eq!(restored[0].from.session, "sess-a");
    assert_eq!(restored[0].uses, 1);
}
