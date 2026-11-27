//! Shared F8 harness includes and non-builder fixture key-verification helpers for Set P5.

#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/manifest.rs"]
pub mod manifest;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures_catalog.rs"]
pub mod measures_catalog;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures.rs"]
pub mod measures;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn eval_fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("eval")
}

/// Independently verifies that `answer_key["statement"]` agrees with the raw source file text or
/// extracted PDF text, without invoking any skeleton builder or `locate_in_skeleton` function.
pub fn verify_statement_key_against_raw_source(case_id: &str, rel_file: &str, stmt: &Value) {
    let full = eval_fixtures_dir().join(rel_file);
    let format = stmt["format"].as_str().expect("statement.format");
    let present = stmt["present"].as_bool().expect("statement.present");

    match format {
        "tex" => {
            let text = fs::read_to_string(&full).unwrap();
            assert!(!text.contains('\r'), "{case_id}: CRLF found in TeX fixture");
            if present {
                if let Some(label) = stmt.get("label").and_then(Value::as_str) {
                    let needle = format!("\\label{{{label}}}");
                    assert!(text.contains(&needle), "{case_id}: raw TeX lacks `{needle}`");
                }
                if let Some(title) = stmt.get("title").and_then(Value::as_str) {
                    assert!(text.contains(title), "{case_id}: raw TeX lacks title `{title}`");
                }
                if stmt["number_basis"].as_str() == Some("printed") {
                    let num = stmt["number"].as_str().unwrap();
                    let tag_needle = format!("\\tag{{{num}}}");
                    assert!(text.contains(&tag_needle), "{case_id}: raw TeX lacks `{tag_needle}`");
                }
                if let Some(start) = stmt.get("source_line_start").and_then(Value::as_u64) {
                    let end = stmt["source_line_end"].as_u64().unwrap();
                    let all_lines: Vec<&str> = text.lines().collect();
                    assert!(
                        start >= 1 && (end as usize) <= all_lines.len() && end >= start,
                        "{case_id}: invalid line range {start}..={end}"
                    );
                    if let Some(label) = stmt.get("label").and_then(Value::as_str) {
                        let slice = all_lines[(start as usize - 1)..(end as usize)].join("\n");
                        assert!(
                            slice.contains(&format!("\\label{{{label}}}")),
                            "{case_id}: lines {start}..={end} lack `\\label{{{label}}}`"
                        );
                    }
                }
            } else if let Some(num) = stmt.get("number").and_then(Value::as_str) {
                assert!(!text.contains(num), "{case_id}: missing target `{num}` unexpectedly found in raw TeX");
            }
        }
        "html" => {
            let html = fs::read_to_string(&full).unwrap();
            assert!(!html.contains('\r'), "{case_id}: CRLF found in HTML fixture");
            let id = stmt["html_id"].as_str().unwrap();
            let num = stmt["number"].as_str().unwrap();
            let title = stmt["title"].as_str().unwrap();
            assert!(html.contains(&format!("id=\"{id}\"")), "{case_id}: raw HTML lacks id `{id}`");
            assert!(html.contains(num), "{case_id}: raw HTML lacks number `{num}`");
            assert!(html.contains(title), "{case_id}: raw HTML lacks title `{title}`");
        }
        "pdf" => {
            let raw = fs::read(&full).unwrap();
            assert!(!raw.contains(&b'\r'), "{case_id}: CR byte found in PDF fixture");
            let opts = research_tools::pdf_pages::PdfExtractOptions::default();
            let doc = research_tools::pdf_pages::extract_pdf_pages(&raw, &opts).unwrap();
            let all_lines: Vec<&str> = doc.text.lines().collect();
            let page_text = |rec: &research_tools::pdf_pages::PageRecord| -> String {
                match (rec.first_line, rec.last_line) {
                    (Some(a), Some(b)) if a >= 1 && b >= a && b <= all_lines.len() => {
                        all_lines[(a - 1)..b].join("\n")
                    }
                    _ => String::new(),
                }
            };
            let target_page = stmt["page"].as_u64().unwrap() as usize;
            let rec = doc
                .page_map
                .record(target_page)
                .unwrap_or_else(|| panic!("{case_id}: raw PDF missing page {target_page}"));
            let txt = page_text(rec);
            let title = stmt["title"].as_str().unwrap();
            assert!(
                txt.contains(title),
                "{case_id}: raw extracted page {target_page} text `{txt}` lacks `{title}`"
            );
            if let Some(img_only) = stmt.get("image_only_page").and_then(Value::as_u64) {
                let p_img = doc.page_map.record(img_only as usize).unwrap();
                let img_txt = page_text(p_img);
                assert!(
                    img_txt.trim().is_empty(),
                    "{case_id}: expected image-only page {img_only} to extract empty text, got `{img_txt}`"
                );
            }
        }
        other => panic!("{case_id}: unknown statement format `{other}`"),
    }
}
