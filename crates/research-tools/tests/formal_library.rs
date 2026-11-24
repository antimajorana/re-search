use research_tools::formal_library::handle_search_formal_library;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write_source(workspace: &Path, path: &str, text: &str) {
    let path = workspace.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn search(env: &ResearchEnv, query: &str) -> Value {
    handle_search_formal_library(env, "s-physlib", &json!({ "query": query })).unwrap()
}

#[test]
fn searches_physlib_declarations_with_exact_signature_namespace_and_digest() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("physics");
    fs::create_dir(&ws).unwrap();
    let env = ResearchEnv::with_workspace(tmp.path().join("data"), &ws).unwrap();
    assert_eq!(search(&env, "boost")["status"], "library_not_found");

    let source = concat!(
        "/- Outer comment /- Nested comment -/\n",
        "theorem fake_boost : True := trivial\n-/\n",
        "namespace SpecialRelativity\n",
        "section InertialFrame\n",
        "@[simp]\n",
        "theorem boost\n",
        "    (x : ℝ) : x = x := by\n",
        "  rfl\n",
        "end InertialFrame\n",
        "private theorem privateBoost : True := trivial\n",
        "def exampleText := r#\"\n",
        "theorem false_in_raw_string : True := trivial\n",
        "\"#\n",
        "instance : Inhabited Unit := ⟨()⟩\n",
        "protected theorem later : True := trivial\n",
        "end SpecialRelativity\n",
    );
    write_source(&ws, "Physlib/Relativity/Boost.lean", source);
    write_source(
        &ws,
        ".lake/packages/physlib/Physlib/Units.lean",
        "namespace Physlib.Units\naxiom scale : Rational\nend Physlib.Units\n",
    );
    write_source(
        &ws,
        "unrelated.lean",
        "theorem outside_physlib : True := trivial\n",
    );

    let hit = search(&env, "SpecialRelativity.boost")["results"][0].clone();
    assert_eq!(hit["name"], "boost");
    assert_eq!(hit["qualified_name"], "SpecialRelativity.boost");
    assert_eq!(hit["namespace"], "SpecialRelativity");
    assert_eq!(hit["kind"], "theorem");
    assert_eq!(hit["line"], 7);
    assert_eq!(hit["signature"], "theorem boost\n    (x : ℝ) : x = x");
    assert_eq!(hit["signature_truncated"], false);
    assert_eq!(hit["module"], "Physlib.Relativity.Boost");
    assert_eq!(hit["path"], "Physlib/Relativity/Boost.lean");
    assert_eq!(hit["source_sha256"], ledger::sha256_hex(source.as_bytes()));
    assert_eq!(hit["status"], "lexical_unverified");

    let private = search(&env, "privateBoost")["results"][0].clone();
    assert_eq!(private["visibility"], "private");
    assert!(private["qualified_name"].is_null());
    assert_eq!(
        search(&env, "SpecialRelativity.later")["results"][0]["visibility"],
        "protected"
    );
    let installed = search(&env, "Physlib.Units.scale")["results"][0].clone();
    assert_eq!(
        installed["path"],
        ".lake/packages/physlib/Physlib/Units.lean"
    );
    assert_eq!(installed["module"], "Physlib.Units");
    assert_eq!(installed["signature"], "axiom scale : Rational");

    for missed in [
        "fake_boost",
        "false_in_raw_string",
        "outside_physlib",
        "Inhabited Unit",
    ] {
        assert_eq!(search(&env, missed)["count"], 0, "{missed}");
    }

    let ranked = handle_search_formal_library(
        &env,
        "s-physlib",
        &json!({ "query": "boost", "max_results": 1 }),
    )
    .unwrap();
    assert_eq!(ranked["results"][0]["name"], "boost");
    assert_eq!(ranked["results_truncated"], true);
}

#[test]
fn requires_workspace_and_reports_oversize_sources_as_incomplete() {
    let tmp = tempfile::tempdir().unwrap();
    let unattached = ResearchEnv::new(tmp.path().join("data"));
    assert!(
        handle_search_formal_library(&unattached, "s-physlib", &json!({ "query": "lemma" }),)
            .unwrap_err()
            .contains("no workspace")
    );

    let ws = tmp.path().join("physics");
    write_source(&ws, "Physlib/Large.lean", &" ".repeat(513 * 1024));
    write_source(
        &ws,
        "Physlib/Small.lean",
        "theorem present : True := trivial\n",
    );
    let env = ResearchEnv::with_workspace(tmp.path().join("data"), &ws).unwrap();
    let result = search(&env, "present");
    assert_eq!(result["files_scanned"], 1);
    assert_eq!(result["skipped_files"], 1);
    assert_eq!(result["scan_truncated"], true);
    assert_eq!(result["results"][0]["name"], "present");

    for invalid in [
        json!({"query": "  "}),
        json!({"query": "x", "path": "../outside.lean"}),
        json!({"query": "x".repeat(161)}),
    ] {
        assert!(handle_search_formal_library(&env, "s-physlib", &invalid).is_err());
    }
}

#[cfg(unix)]
#[test]
fn excludes_symlinked_files_and_package_paths_even_under_the_workspace() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("physics");
    let external = tmp.path().join("external");
    write_source(
        &external,
        "Physlib/Escape.lean",
        "theorem escaped_decl : True := trivial\n",
    );
    write_source(
        &ws,
        "unused/Physlib/Internal.lean",
        "theorem symlinked_package : True := trivial\n",
    );
    write_source(
        &ws,
        "Physlib/Good.lean",
        "theorem local_decl : True := trivial\n",
    );
    symlink(
        external.join("Physlib/Escape.lean"),
        ws.join("Physlib/Escape.lean"),
    )
    .unwrap();
    fs::create_dir_all(ws.join(".lake/packages")).unwrap();
    symlink(ws.join("unused"), ws.join(".lake/packages/physlib")).unwrap();
    let env = ResearchEnv::with_workspace(tmp.path().join("data"), &ws).unwrap();
    assert_eq!(search(&env, "escaped_decl")["count"], 0);
    assert_eq!(search(&env, "symlinked_package")["count"], 0);
    assert_eq!(search(&env, "local_decl")["count"], 1);
}
