//! Golden pin of every built tool profile (package F1, overview section 2.2).
//!
//! One golden section per profile (`research/1`, `investigation/1`) pinning policy version,
//! `(position, id, def_hash)` of every definition in order, `may_gain` ids, and `earlier_defs`
//! hashes. Lives beside `tests/golden/tool_defs.txt` without modifying or regenerating it.
//!
//! To regenerate after an intentional profile addition: `UPDATE_TOOL_PROFILE_GOLDEN=1 cargo test
//! -p research-tools --test tool_profile_golden`, then review the diff carefully.

use research_tools::tool_profile::{
    built_profiles, lookup, ProfileError, ToolProfile, PROFILE_EXPLORE_V1, PROFILE_INVESTIGATION_V1,
    PROFILE_INVESTIGATION_V2, PROFILE_RESEARCH_V1, PROFILE_SOLVE_V1, RESERVED_PROFILE_NAMES,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

const HEADER: &str = "\
# Golden tool profiles (package F1). Checked by crates/research-tools/tests/tool_profile_golden.rs.
# Sections: [profile <name>], policy <version>, def <position> <id> <def_hash>,
# may_gain <position> <id>, earlier_def <id> <position> <def_hash>.
";

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("tool_profiles.txt")
}

fn tool_defs_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("tool_defs.txt")
}

fn render(profiles: &[ToolProfile]) -> String {
    let mut out = String::from(HEADER);
    for profile in profiles {
        out.push('\n');
        writeln!(out, "[profile {}]", profile.name).unwrap();
        writeln!(out, "policy {}", profile.policy.version).unwrap();
        for (pos, def) in profile.defs.iter().enumerate() {
            writeln!(out, "def {pos} {} {}", def.id, def.def_hash()).unwrap();
        }
        for (pos, id) in profile.may_gain.iter().enumerate() {
            writeln!(out, "may_gain {pos} {id}").unwrap();
        }
        for (id, earlier) in &profile.earlier_defs {
            for (pos, def) in earlier.iter().enumerate() {
                writeln!(out, "earlier_def {id} {pos} {}", def.def_hash()).unwrap();
            }
        }
    }
    out
}

#[test]
fn built_tool_profiles_match_the_golden_file() {
    let profiles = built_profiles();
    let actual = render(&profiles);
    let path = golden_path();
    if std::env::var_os("UPDATE_TOOL_PROFILE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n");
    if golden == actual {
        return;
    }
    let golden_lines: Vec<&str> = golden.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let mut report = String::new();
    for line in &golden_lines {
        if !actual_lines.contains(line) {
            writeln!(report, "  - {line}").unwrap();
        }
    }
    for line in &actual_lines {
        if !golden_lines.contains(line) {
            writeln!(report, "  + {line}").unwrap();
        }
    }
    if report.is_empty() {
        report.push_str("  (same lines, different order)\n");
    }
    panic!(
        "tool profiles differ from {}:\n{report}\
         Changing an existing profile breaks resume for sessions recorded under it.",
        path.display()
    );
}

#[test]
fn every_built_profile_def_hash_agrees_with_tool_defs_golden() {
    // Parse `builtin` and `research` sets from the frozen F0 golden file and verify every
    // tool in `research/1` and `investigation/1` matches its pinned hash byte for byte.
    let text = std::fs::read_to_string(tool_defs_golden_path()).unwrap();
    let mut pinned: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 4 && matches!(parts[0], "builtin" | "research") {
            pinned.insert(parts[2].to_string(), parts[3].to_string());
        }
    }

    for name in [PROFILE_RESEARCH_V1, PROFILE_INVESTIGATION_V1, PROFILE_INVESTIGATION_V2] {
        let profile = lookup(name).unwrap();
        for pair in profile.defs.windows(2) {
            assert!(pair[0].id < pair[1].id, "profile `{name}` out of id order");
        }
        for def in &profile.defs {
            if name == PROFILE_INVESTIGATION_V2
                && matches!(
                    def.id.as_str(),
                    research_tools::map_defs::READ_MAP_NODE
                        | research_tools::map_defs::RECORD_MAP_READING
                )
            {
                assert_eq!(def.effect, policy::EffectClass::ReadWorkspace);
                continue;
            }
            let expected = pinned
                .get(&def.id)
                .unwrap_or_else(|| panic!("profile `{name}` tool `{}` missing from tool_defs.txt", def.id));
            assert_eq!(
                &def.def_hash(),
                expected,
                "profile `{name}` tool `{}` hash differs from tool_defs.txt",
                def.id
            );
        }
    }

    // Verify every reused tool in `solve/1` matches its frozen hash in `tool_defs.txt` byte for byte.
    let s1 = lookup(PROFILE_SOLVE_V1).unwrap();
    for pair in s1.defs.windows(2) {
        assert!(pair[0].id < pair[1].id, "profile `{PROFILE_SOLVE_V1}` out of id order");
    }
    let solve_only = [
        research_tools::solve_defs::CHECK_RUN,
        research_tools::solve_defs::EXPR_SHOW,
        research_tools::solve_defs::SOLVE_FINISH,
        research_tools::solve_defs::STEP_RECORD,
    ];
    for def in &s1.defs {
        if solve_only.contains(&def.id.as_str()) {
            assert!(
                !pinned.contains_key(&def.id),
                "new Solve tool `{}` must not exist in frozen tool_defs.txt",
                def.id
            );
        } else {
            let expected = pinned
                .get(&def.id)
                .unwrap_or_else(|| panic!("profile `solve/1` reused tool `{}` missing from tool_defs.txt", def.id));
            assert_eq!(
                &def.def_hash(),
                expected,
                "profile `solve/1` reused tool `{}` hash differs from tool_defs.txt",
                def.id
            );
        }
    }
}

#[test]
fn reserved_profiles_return_clear_not_built_yet_error() {
    assert_eq!(
        RESERVED_PROFILE_NAMES,
        &[PROFILE_EXPLORE_V1]
    );
    for name in RESERVED_PROFILE_NAMES {
        let err = lookup(name).expect_err("reserved profile must return Err");
        assert_eq!(err, ProfileError::NotBuiltYet { name: (*name).to_string() });
        assert!(err.to_string().contains("profile not built yet"), "{err}");
    }
}
