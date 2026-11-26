//! Structural integrity tests for the fixed-tier Investigate Set P evaluation
//! cases (`tests/fixtures/eval/manifest-p.json` and `tests/fixtures/eval/p/eval-*`).
//!
//! These checks verify the raw TeX fixtures and hand-authored answer keys
//! directly against source lines without invoking any TeX scanner, keeping
//! expected keys independent of scanner output.

#[allow(dead_code)]
mod eval_support;
#[allow(dead_code)]
mod support;

use eval_support::manifest::{Input, Set, Tier};
use eval_support::{fixture_path, fixtures_dir, load_manifests, EvalSet};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn read_raw_lf(dir: &Path, rel: &str) -> Vec<String> {
    let path = fixture_path(dir, rel);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(!raw.contains('\r'), "fixture `{rel}` must use LF line endings only");
    raw.lines().map(ToOwned::to_owned).collect()
}

fn line_at<'a>(files: &'a BTreeMap<String, Vec<String>>, file: &str, line: usize, ctx: &str) -> &'a str {
    let lines = files.get(file).unwrap_or_else(|| panic!("{ctx}: fixture `{file}` not loaded"));
    assert!(line >= 1 && line <= lines.len(), "{ctx}: line {line} out of range 1..={} in `{file}`", lines.len());
    &lines[line - 1]
}

fn case_files(set: &EvalSet, case: &eval_support::manifest::Case) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    let Input::Fixture { files, .. } = &case.input else {
        panic!("{}: expected Input::Fixture", case.id);
    };
    for rel in files {
        map.insert(rel.clone(), read_raw_lf(&set.dir, rel));
    }
    if let Some(inc) = case.answer_key.get("included_files").and_then(Value::as_array) {
        for item in inc {
            let rel = item.as_str().unwrap_or_else(|| panic!("{}: included_files entry must be string", case.id));
            map.insert(rel.to_owned(), read_raw_lf(&set.dir, rel));
        }
    }
    map
}

#[test]
fn eval_p_cases_cover_required_fixture_shapes() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).expect("fixed manifests must load");
    let eval_cases: Vec<_> = set.cases_of(Set::P).into_iter().filter(|c| c.id.starts_with("P-eval-")).collect();
    assert_eq!(eval_cases.len(), 10, "expected 10 fixed-tier Set P eval cases");

    // Preserve both harness self-tests.
    assert!(set.case("P-selftest-theorems").is_some());
    assert!(set.case("P-selftest-oscillator").is_some());

    let mut physics_no_thm = 0usize;
    let mut multifile_count = 0usize;
    let mut long_doc_count = 0usize;
    let mut locator_case_count = 0usize;

    for case in &eval_cases {
        let files = case_files(&set, case);
        let has_thm = case.answer_key["has_theorem_envs"].as_bool().expect("has_theorem_envs bool");
        let stmts = case.answer_key["statements"].as_array().expect("statements array");
        if !has_thm {
            physics_no_thm += 1;
            assert!(stmts.is_empty(), "{}: physics note must have empty statements", case.id);
            for (rel, lines) in &files {
                let text = lines.join("\n");
                for forbidden in ["\\newtheorem", "\\begin{theorem}", "\\begin{lemma}", "\\begin{proof}"] {
                    assert!(!text.contains(forbidden), "{rel} must not contain `{forbidden}`");
                }
            }
        }
        if files.len() > 1 {
            multifile_count += 1;
            let Input::Fixture { files: entry, .. } = &case.input else { unreachable!() };
            let entry_text = files[&entry[0]].join("\n");
            for rel in files.keys() {
                if rel != &entry[0] {
                    let stem = rel.rsplit('/').next().unwrap().trim_end_matches(".tex");
                    assert!(
                        entry_text.contains(&format!("\\input{{{stem}}}"))
                            || entry_text.contains(&format!("\\input{{{stem}.tex}}")),
                        "{}: entry file does not \\input `{stem}`",
                        case.id
                    );
                }
            }
        }
        let total_lines: usize = files.values().map(Vec::len).sum();
        let total_words: usize = files.values().flat_map(|ls| ls.iter()).map(|l| l.split_whitespace().count()).sum();
        if total_lines >= 3_500 && total_words >= 30_000 {
            long_doc_count += 1;
        }
        if let Input::Fixture { locator: Some(loc), .. } = &case.input {
            locator_case_count += 1;
            let located = &case.answer_key["located_statement"];
            assert_eq!(located["locator"].as_str(), Some(loc.as_str()), "{}: located_statement mismatch", case.id);
        }
    }

    assert!(physics_no_thm >= 4, "need at least 4 physics notes without theorem envs, found {physics_no_thm}");
    assert!(multifile_count >= 1, "need at least 1 multi-file fixture");
    assert!(long_doc_count >= 1, "need at least 1 long generated fixture (>=3500 lines, >=30k words)");
    assert!(locator_case_count >= 1, "need at least 1 case exercising Input::Fixture.locator");
}

#[test]
fn eval_p_cases_sections_statements_equations_and_citations_match_tex() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).expect("fixed manifests must load");
    for case in set.cases_of(Set::P).into_iter().filter(|c| c.id.starts_with("P-eval-")) {
        let files = case_files(&set, case);
        let mut section_levels: BTreeMap<String, u64> = BTreeMap::new();
        for sec in case.answer_key["sections"].as_array().expect("sections array") {
            let id = sec["id"].as_str().unwrap();
            let level = sec["level"].as_u64().unwrap();
            let numbered = sec["numbered"].as_bool().unwrap();
            let title = sec["title"].as_str().unwrap();
            let file = sec["file"].as_str().unwrap();
            let line = sec["line"].as_u64().unwrap() as usize;
            let raw = line_at(&files, file, line, &format!("{}:{id}", case.id));
            let cmd = match (level, numbered) {
                (1, true) => "\\section{",
                (1, false) => "\\section*{",
                (2, true) => "\\subsection{",
                (2, false) => "\\subsection*{",
                (3, true) => "\\subsubsection{",
                (3, false) => "\\subsubsection*{",
                other => panic!("{}:{id}: unsupported section (level, numbered)={other:?}", case.id),
            };
            assert!(raw.contains(&format!("{cmd}{title}}}")), "{}:{id}: expected `{cmd}{title}}}` on `{file}:{line}`, got `{raw}`", case.id);
            if let Some(parent) = sec["parent"].as_str() {
                let p_lvl = *section_levels.get(parent).unwrap_or_else(|| panic!("{}:{id}: unknown parent `{parent}`", case.id));
                assert!(p_lvl < level, "{}:{id}: parent `{parent}` level {p_lvl} not < {level}", case.id);
            } else {
                assert_eq!(level, 1, "{}:{id}: non-root section must have a parent", case.id);
            }
            if let Some(lbl) = sec["label"].as_str() {
                let l1 = line_at(&files, file, line, &case.id);
                let l2 = if line < files[file].len() { line_at(&files, file, line + 1, &case.id) } else { "" };
                let pat = format!("\\label{{{lbl}}}");
                assert!(l1.contains(&pat) || l2.contains(&pat), "{}:{id}: missing `{pat}` at {file}:{line}..={}", case.id, line + 1);
            }
            assert!(section_levels.insert(id.to_owned(), level).is_none(), "{}: duplicate section id `{id}`", case.id);
        }

        let mut stmt_spans: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
        for st in case.answer_key["statements"].as_array().expect("statements array") {
            let id = st["id"].as_str().unwrap();
            let env = st["env_name"].as_str().unwrap();
            let file = st["file"].as_str().unwrap();
            let start = st["start_line"].as_u64().unwrap() as usize;
            let end = st["end_line"].as_u64().unwrap() as usize;
            assert!(start < end, "{}:{id}: invalid span {start}..{end}", case.id);
            assert!(section_levels.contains_key(st["section"].as_str().unwrap()), "{}:{id}: unknown section", case.id);
            assert!(line_at(&files, file, start, &case.id).contains(&format!("\\begin{{{env}}}")), "{}:{id}: start line mismatch", case.id);
            assert!(line_at(&files, file, end, &case.id).contains(&format!("\\end{{{env}}}")), "{}:{id}: end line mismatch", case.id);
            if let Some(lbl) = st["label"].as_str() {
                let body = files[file][start - 1..end].join("\n");
                assert!(body.contains(&format!("\\label{{{lbl}}}")), "{}:{id}: missing label inside span", case.id);
            }
            if st["has_proof"].as_bool().unwrap() {
                let pf = st["proof_file"].as_str().unwrap();
                let ps = st["proof_start_line"].as_u64().unwrap() as usize;
                let pe = st["proof_end_line"].as_u64().unwrap() as usize;
                assert!(line_at(&files, pf, ps, &case.id).contains("\\begin{proof}"), "{}:{id}: proof start mismatch", case.id);
                assert!(line_at(&files, pf, pe, &case.id).contains("\\end{proof}"), "{}:{id}: proof end mismatch", case.id);
            }
            assert!(stmt_spans.insert(id.to_owned(), (file.to_owned(), start, end)).is_none(), "{}: duplicate statement id `{id}`", case.id);
        }

        for eq in case.answer_key["equations"].as_array().expect("equations array") {
            let lbl = eq["label"].as_str().unwrap();
            let enc = eq["enclosing_node"].as_str().unwrap();
            let file = eq["file"].as_str().unwrap();
            let start = eq["start_line"].as_u64().unwrap() as usize;
            let end = eq["end_line"].as_u64().unwrap() as usize;
            let head = line_at(&files, file, start, &case.id);
            assert!(head.contains("\\begin{equation}") || head.contains("\\begin{align}"), "{}:{lbl}: equation start mismatch", case.id);
            let tail = line_at(&files, file, end, &case.id);
            assert!(tail.contains("\\end{equation}") || tail.contains("\\end{align}"), "{}:{lbl}: equation end mismatch", case.id);
            let body = files[file][start - 1..end].join("\n");
            assert!(body.contains(&format!("\\label{{{lbl}}}")), "{}:{lbl}: missing label", case.id);
            if let Some((sf, ss, se)) = stmt_spans.get(enc) {
                assert_eq!(sf, file, "{}:{lbl}: enclosing statement file mismatch", case.id);
                assert!(*ss <= start && end <= *se, "{}:{lbl}: equation not inside enclosing statement `{enc}`", case.id);
            } else {
                assert!(section_levels.contains_key(enc), "{}:{lbl}: unknown enclosing_node `{enc}`", case.id);
            }
        }

        if let Some(cites) = case.answer_key.get("citations_with_locators").and_then(Value::as_array) {
            for c in cites {
                let key = c["bib_key"].as_str().unwrap();
                let role = c["role"].as_str().unwrap();
                assert!(matches!(role, "statement" | "proof" | "prose"), "{}: bad citation role `{role}`", case.id);
                let file = c["site"]["file"].as_str().unwrap();
                let line = c["site"]["line"].as_u64().unwrap() as usize;
                let raw = line_at(&files, file, line, &case.id);
                assert!(raw.contains("\\cite[") && raw.contains(&format!("{{{key}}}")), "{}: missing \\cite[..]{{{key}}} at {file}:{line}", case.id);
            }
        }
        if let Some(bib) = case.answer_key.get("bibliography").and_then(Value::as_array) {
            for b in bib {
                let key = b["key"].as_str().unwrap();
                let file = b["file"].as_str().unwrap();
                let line = b["line"].as_u64().unwrap() as usize;
                assert!(line_at(&files, file, line, &case.id).contains(&format!("\\bibitem{{{key}}}")), "{}: missing \\bibitem{{{key}}}", case.id);
            }
        }
    }
}

#[test]
fn eval_p_cases_thirty_sampled_uses_are_directed_and_site_verified() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).expect("fixed manifests must load");
    let mut total_uses = 0usize;
    let mut conn_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut lifted_count = 0usize;

    for case in set.cases_of(Set::P).into_iter().filter(|c| c.id.starts_with("P-eval-")) {
        let files = case_files(&set, case);
        let mut all_nodes: BTreeSet<String> = BTreeSet::new();
        let mut stmt_info: BTreeMap<String, &Value> = BTreeMap::new();
        let mut eq_enclosing: BTreeMap<String, String> = BTreeMap::new();

        for sec in case.answer_key["sections"].as_array().unwrap() {
            all_nodes.insert(sec["id"].as_str().unwrap().to_owned());
        }
        for st in case.answer_key["statements"].as_array().unwrap() {
            let id = st["id"].as_str().unwrap().to_owned();
            all_nodes.insert(id.clone());
            stmt_info.insert(id, st);
        }
        for eq in case.answer_key["equations"].as_array().unwrap() {
            let lbl = eq["label"].as_str().unwrap().to_owned();
            all_nodes.insert(lbl.clone());
            eq_enclosing.insert(lbl, eq["enclosing_node"].as_str().unwrap().to_owned());
        }

        for edge in case.answer_key["uses_edges"].as_array().unwrap() {
            total_uses += 1;
            let from = edge["from"].as_str().unwrap();
            let to = edge["to"].as_str().unwrap();
            let role = edge["role"].as_str().unwrap();
            let conn = edge["connection"].as_str().unwrap();
            let conf = edge["confidence"].as_str().unwrap();
            let rule = edge["rule"].as_str().unwrap();
            assert_eq!(edge["right"].as_bool(), Some(true), "{}: sampled edge must set right=true", case.id);
            assert_ne!(from, to, "{}: self-loop edge `{from}`", case.id);
            assert!(all_nodes.contains(from), "{}: unknown `from` `{from}`", case.id);
            assert!(all_nodes.contains(to), "{}: unknown `to` `{to}`", case.id);
            assert!(matches!(role, "statement" | "proof" | "prose"), "{}: invalid role `{role}`", case.id);
            assert!(matches!(conn, "statement_use" | "proof_use" | "heuristic_mention"), "{}: invalid connection `{conn}`", case.id);
            assert!(matches!(conf, "explicit" | "heuristic"), "{}: invalid confidence `{conf}`", case.id);
            *conn_counts.entry(conn.to_owned()).or_insert(0) += 1;

            let mut sites = vec![&edge["site"]];
            if let Some(alts) = edge.get("alt_sites").and_then(Value::as_array) {
                sites.extend(alts.iter());
            }

            for site in sites {
                let file = site["file"].as_str().unwrap();
                let line = site["line"].as_u64().unwrap() as usize;
                let raw = line_at(&files, file, line, &format!("{}:{from}->{to}", case.id));
                assert!(!raw.trim_start().starts_with('%'), "{}: edge site {file}:{line} must not be commented", case.id);

                if role == "statement" {
                    let st = stmt_info.get(to).unwrap_or_else(|| panic!("{}: statement-role `to` `{to}` not a statement", case.id));
                    assert_eq!(st["file"].as_str(), Some(file));
                    let s = st["start_line"].as_u64().unwrap() as usize;
                    let e = st["end_line"].as_u64().unwrap() as usize;
                    assert!(s <= line && line <= e, "{}: site {file}:{line} outside statement `{to}` ({s}..={e})", case.id);
                } else if role == "proof" {
                    let st = stmt_info.get(to).unwrap_or_else(|| panic!("{}: proof-role `to` `{to}` not a statement", case.id));
                    assert_eq!(st["has_proof"].as_bool(), Some(true), "{}: proof-role `to` `{to}` lacks proof", case.id);
                    assert_eq!(st["proof_file"].as_str(), Some(file));
                    let s = st["proof_start_line"].as_u64().unwrap() as usize;
                    let e = st["proof_end_line"].as_u64().unwrap() as usize;
                    assert!(s <= line && line <= e, "{}: site {file}:{line} outside proof of `{to}` ({s}..={e})", case.id);
                }

                match rule {
                    "ref" => assert!(raw.contains(&format!("\\ref{{{from}}}")), "{}: missing \\ref{{{from}}} at {file}:{line}", case.id),
                    "eqref" => assert!(raw.contains(&format!("\\eqref{{{from}}}")), "{}: missing \\eqref{{{from}}} at {file}:{line}", case.id),
                    "cref" => assert!(raw.contains("\\cref{") && raw.contains(from), "{}: missing \\cref containing `{from}` at {file}:{line}", case.id),
                    "lifted" => {
                        let direct = edge["direct_from"].as_str().expect("lifted edge needs direct_from");
                        assert_eq!(eq_enclosing.get(direct).map(String::as_str), Some(from), "{}: direct_from `{direct}` not enclosed in `{from}`", case.id);
                        assert!(raw.contains(&format!("\\eqref{{{direct}}}")) || raw.contains(&format!("\\ref{{{direct}}}")), "{}: missing ref to `{direct}` at {file}:{line}", case.id);
                    }
                    "text_mention" => {
                        assert_eq!(conf, "heuristic");
                        assert_eq!(conn, "heuristic_mention");
                        let mention = edge["printed_mention"].as_str().expect("text_mention needs printed_mention");
                        assert!(raw.contains(mention), "{}: missing `{mention}` at {file}:{line}", case.id);
                        assert!(!raw.contains(&format!("\\ref{{{from}}}")) && !raw.contains(&format!("\\eqref{{{from}}}")), "{}: heuristic mention must not use \\ref/\\eqref for `{from}`", case.id);
                    }
                    other => panic!("{}: unexpected rule `{other}`", case.id),
                }
            }
            if rule == "lifted" {
                lifted_count += 1;
            }
        }

        if let Some(non_edges) = case.answer_key.get("non_edges").and_then(Value::as_array) {
            for ne in non_edges {
                let file = ne["site"]["file"].as_str().unwrap();
                let line = ne["site"]["line"].as_u64().unwrap() as usize;
                let _ = line_at(&files, file, line, &format!("{}:non_edge", case.id));
            }
        }
    }

    assert_eq!(total_uses, 30, "Set P eval cases must contain exactly 30 sampled uses_edges");
    assert!(conn_counts.get("statement_use").copied().unwrap_or(0) > 0);
    assert!(conn_counts.get("proof_use").copied().unwrap_or(0) > 0);
    assert!(conn_counts.get("heuristic_mention").copied().unwrap_or(0) > 0);
    assert!(lifted_count >= 1, "expected at least one lifted inner-equation proof use");
}
