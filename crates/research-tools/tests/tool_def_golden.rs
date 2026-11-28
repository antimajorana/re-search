//! Golden pin of every tool definition registered today: (set, id, def_hash, order).
//!
//! A session records the id and def_hash of each tool it started with, and resume is
//! fail-closed on any difference. A change to a description, a schema, an effect class, an
//! approval scope, or to which tools a registration function adds therefore decides whether
//! earlier sessions still resume. This test makes such a change visible in review: it fails
//! until `tests/golden/tool_defs.txt` is edited in the same change.
//!
//! To regenerate after an intended change: `UPDATE_TOOL_DEF_GOLDEN=1 cargo test -p
//! research-tools --test tool_def_golden`, then review the diff of the golden file.

use research_tools::{
    all_research_tool_defs, m1_tool_defs, register_lean_tool, register_reader_tools,
    register_research_tools, ResearchEnv,
};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use tools::{builtin, Registry, ToolDef};

const HEADER: &str = "\
# Golden tool definitions. One line per tool: <set> <position> <id> <def_hash>.
# Checked by crates/research-tools/tests/tool_def_golden.rs. Do not edit by hand without
# reading that file: a changed line here means earlier sessions may stop resuming.
";

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join("tool_defs.txt")
}

fn env() -> Arc<ResearchEnv> {
    // Definitions do not depend on the directory; nothing is read or written here.
    Arc::new(ResearchEnv::new(std::env::temp_dir().join("re-search-tool-def-golden-unused")))
}

fn registered(fill: impl FnOnce(&mut Registry)) -> Vec<ToolDef> {
    let mut registry = Registry::new();
    fill(&mut registry);
    registry.defs().into_iter().cloned().collect()
}

/// The sets a session can be given today, in the order they are written to the golden file.
fn sets() -> Vec<(&'static str, Vec<ToolDef>)> {
    vec![
        // Host tools that the desktop, CLI and MCP routers add next to the research tools.
        (
            "builtin",
            vec![builtin::ledger_note_def(), builtin::session_title_def(), builtin::sympy_eval_def()],
        ),
        // `register_research_tools`: the main session registry (the future profile `research/1`).
        ("research", registered(|r| register_research_tools(r, env()))),
        // `m1_tool_defs`, exported as `research_tool_defs()`: the workspace and LaTeX subset.
        ("research_m1", m1_tool_defs()),
        // `register_reader_tools`: the isolated paper-reader router.
        ("reader", registered(|r| register_reader_tools(r, env()))),
        // `register_lean_tool`: the Lean router.
        ("lean", registered(|r| register_lean_tool(r, env()))),
    ]
}

fn render(sets: &[(&str, Vec<ToolDef>)]) -> String {
    let mut out = String::from(HEADER);
    for (name, defs) in sets {
        out.push('\n');
        for (position, def) in defs.iter().enumerate() {
            writeln!(out, "{name} {position} {} {}", def.id, def.def_hash()).unwrap();
        }
    }
    out
}

#[test]
fn registered_tool_definitions_match_the_golden_file() {
    let actual = render(&sets());
    let path = golden_path();
    if std::env::var_os("UPDATE_TOOL_DEF_GOLDEN").is_some() {
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
        "tool definitions differ from {}:\n{report}\
         Earlier sessions recorded the old id, def_hash and order, and resume refuses any \
         difference. If the change is intended, regenerate with UPDATE_TOOL_DEF_GOLDEN=1 and \
         say in the change how earlier sessions keep resuming.",
        path.display()
    );
}

#[test]
fn the_advertised_research_list_is_exactly_what_is_registered() {
    // `all_research_tool_defs()` is the exported list; the registry is what a router offers and
    // what `session.start` records. They must be the same list in the same order.
    let registered = registered(|r| register_research_tools(r, env()));
    assert_eq!(all_research_tool_defs(), registered);
}

#[test]
fn every_set_is_in_id_order_without_duplicates() {
    for (name, defs) in sets() {
        for pair in defs.windows(2) {
            assert!(pair[0].id < pair[1].id, "set `{name}`: `{}` before `{}`", pair[0].id, pair[1].id);
        }
    }
}
