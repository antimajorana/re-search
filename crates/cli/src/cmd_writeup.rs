//! CLI `re-search writeup` subcommand (package F15 Solve adapter).
//!
//! Usage:
//!   `re-search writeup SESSION --format tex|md|bib --out PATH [--no-inline-marks] [--data-dir DIR]`

use crate::cmd::{reject_unknown_flags, USAGE};
use crate::cmd_ext::parse_bool_flag;
use crate::session_lock::validate_session_id;
use export::writeup::WriteupOptions;
use export::{writeup_session_to_path, WriteupFormat};
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn writeup_command(
    positional: Vec<String>,
    mut flags: HashMap<String, String>,
    data_dir: PathBuf,
) -> Result<i32, String> {
    if positional.len() != 1 {
        return Err(format!("writeup expects exactly one SESSION id\n{USAGE}"));
    }
    let session = positional.into_iter().next().unwrap();
    validate_session_id(&session)?;

    let fmt_raw = flags.remove("format").unwrap_or_else(|| "tex".to_string());
    let format = WriteupFormat::parse(&fmt_raw)?;
    let out = flags
        .remove("out")
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("writeup requires --out PATH\n{USAGE}"))?;
    let no_inline_marks =
        parse_bool_flag("no-inline-marks", flags.remove("no-inline-marks"), false)?;
    reject_unknown_flags(&flags)?;

    let abs_data = std::path::absolute(&data_dir).map_err(|e| e.to_string())?;
    let opts = WriteupOptions {
        include_inline_badges: !no_inline_marks,
        bib_stem: String::new(),
    };
    let bytes = writeup_session_to_path(&abs_data, &session, format, &opts, &out)?;
    eprintln!(
        "[re-search] wrote session {session} write-up ({fmt_raw}, {} bytes) -> {}",
        bytes.len(),
        out.display()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use crate::cmd::run_args;
    use export::writeup::writeup_wording;
    use ledger::Ledger;
    use research_tools::solve_model::{
        compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, Claim,
        ClaimForm, ClaimRelation, HandleRef, SetupAssumption, SolveDoneStatus, StepEntryType,
        StepOrigin, SymbolSpec,
    };
    use research_tools::solve_status::Tier;
    use research_tools::tool_profile::MARKER_IDEA_STARTED;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    fn seed_solve_session(data_dir: &Path, sid: &str, fail_check: bool) {
        let ledger = Ledger::open(data_dir).unwrap();
        let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
        let dims = BTreeMap::new();
        let assumps = vec![SetupAssumption { id: "a-1".into(), text: "x real".into() }];
        let math_sha = compute_math_sha256(&symbols, &dims, &assumps, "hbar = 1");
        ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
        ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &json!({
            "revision": 1, "statement": "CLI write-up test.", "task_kind": "calculate",
            "target": { "description": "CLI Identity", "form": "closed_form" },
            "done": [], "conventions": "hbar = 1", "symbols": symbols, "dimensions": dims,
            "assumptions": assumps, "sources": [],
            "roles": {
                "solver": { "provider": "anthropic", "model": "s" },
                "verifier": { "provider": "openai", "model": "v" }
            },
            "tool_profile": event_kinds::SOLVE_TOOL_PROFILE_V1,
            "math_sha256": math_sha,
        })).unwrap();

        let srepr = "Add(x, 2)";
        let ssha = ledger::sha256_hex(srepr.as_bytes());
        let blob = ledger.put_blob(srepr.as_bytes()).unwrap();
        ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &json!({
            "name": "val", "version": 1, "srepr_blob": blob, "srepr_sha256": ssha,
            "latex": "x + 2", "str_preview": srepr, "free_symbols": [],
            "produced_by": { "call_id": "c-1", "result_seq": 1 }, "owner_entry": "e-1", "portable": true,
        })).unwrap();
        let claim = Claim {
            form: ClaimForm::Equation, lhs: HandleRef::new("val", 1).unwrap(),
            rhs: HandleRef::new("val", 1).unwrap(), relation: ClaimRelation::Eq,
            order: None, domain: None, how: None,
        };
        let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(ssha.as_str())).unwrap();
        let esha = compute_entry_sha256(Some(&csha), "CLI verified step.", "");
        ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
            "entry_id": "e-1", "entry_type": StepEntryType::FinalAnswer.as_str(),
            "summary": "CLI verified step.", "details": "", "depends_on": [],
            "claims": [claim], "claim_sha256": csha, "entry_sha256": esha,
            "origin": StepOrigin::Solver,
        })).unwrap();
        let ev = json!({ "residual": if fail_check { "1" } else { "0" } });
        let ev_sha = ledger::sha256_hex(ev.to_string().as_bytes());
        ledger.append_with_hash(sid, event_kinds::SOLVE_CHECK, 1, &json!({
            "check_id": "chk-cli", "claim_sha256": csha, "claim_index": 0, "math_sha256": math_sha,
            "name": "identity_symbolic", "registry_sha256": "c".repeat(64), "params": {},
            "outcome": if fail_check { "fail" } else { "pass" },
            "asked_by": "host", "receipt": { "call_id": "chk-cli", "result_seq": 1 },
            "evidence": ev, "evidence_sha256": ev_sha,
        })).unwrap();
        if !fail_check {
            ledger.append_with_hash(sid, event_kinds::SOLVE_DONE, 1, &json!({
                "final_entry_id": "e-1", "own_tier": Tier::Verified, "effective_tier": Tier::Verified,
                "criteria": [], "rests_on": {}, "status": SolveDoneStatus::Finished,
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 },
            })).unwrap();
        }
    }

    fn run_cli(args: &[&str]) -> Result<i32, String> {
        run_args(args.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn cli_writeup_writes_tex_sidecar_bib_md_and_respects_no_inline_marks() {
        let dir = std::env::temp_dir().join(format!(
            "research-cli-writeup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let data_dir = dir.join("data");
        let out_dir = dir.join("out");
        seed_solve_session(&data_dir, "s-cli-ok", false);
        seed_solve_session(&data_dir, "s-cli-fail", true);
        let ledger = Ledger::open(&data_dir).unwrap();
        ledger.append_with_hash("s-cli-exp", MARKER_IDEA_STARTED, 1, &json!({})).unwrap();

        let data_s = data_dir.to_string_lossy().into_owned();
        let tex_out = out_dir.join("paper.tex");
        let tex_s = tex_out.to_string_lossy().into_owned();
        assert_eq!(
            run_cli(&["writeup", "s-cli-ok", "--format", "tex", "--out", &tex_s, "--data-dir", &data_s]),
            Ok(0)
        );
        let tex = fs::read_to_string(&tex_out).unwrap();
        assert!(tex.contains("x + 2 = x + 2"));
        assert!(tex.contains("[Verified]"));
        assert!(tex.contains(writeup_wording::CHECK_TABLE_HEADING));
        assert!(out_dir.join("paper.bib").is_file(), "Tex write-up must emit sidecar .bib");

        // `--no-inline-marks` works both before other flags and at the end.
        let md_out = out_dir.join("paper.md");
        let md_s = md_out.to_string_lossy().into_owned();
        assert_eq!(
            run_cli(&[
                "writeup", "s-cli-ok", "--no-inline-marks", "--format", "md",
                "--out", &md_s, "--data-dir", &data_s
            ]),
            Ok(0)
        );
        let md = fs::read_to_string(&md_out).unwrap();
        assert!(!md.contains("[Verified]"));
        assert!(md.contains("Done (final answer: Verified)"));
        assert!(md.contains(writeup_wording::CHECK_TABLE_HEADING));

        let bib_out = out_dir.join("only.bib");
        let bib_s = bib_out.to_string_lossy().into_owned();
        assert_eq!(
            run_cli(&[
                "writeup", "s-cli-ok", "--format", "bib", "--out", &bib_s,
                "--data-dir", &data_s, "--no-inline-marks"
            ]),
            Ok(0)
        );
        assert!(bib_out.is_file());

        // Gate G7 refusal and unbuilt Explore adapter both error cleanly.
        let g7_err = run_cli(&["writeup", "s-cli-fail", "--format", "tex", "--out", &tex_s, "--data-dir", &data_s]).unwrap_err();
        assert!(g7_err.contains("Gate G7"), "{g7_err}");
        let exp_err = run_cli(&["writeup", "s-cli-exp", "--format", "tex", "--out", &tex_s, "--data-dir", &data_s]).unwrap_err();
        assert!(exp_err.contains("write-up adapter not built yet"), "{exp_err}");

        // `--no-inline-marks` is still rejected on unrelated commands.
        let bad_flag = run_cli(&["export", "s-cli-ok", "--format", "tex", "--out", &tex_s, "--data-dir", &data_s, "--no-inline-marks"]).unwrap_err();
        assert!(bad_flag.contains("unknown flag --no-inline-marks"), "{bad_flag}");

        let _ = fs::remove_dir_all(&dir);
    }
}
