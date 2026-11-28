use super::*;
use crate::source_kandinsky::{clean_ref_token, read_entry_metadata, LEGACY_INFO_YAML};

#[test]
fn parses_frontmatter_scalars_lists_and_block_scalars() {
    let note = concat!(
        "---\n",
        "date: 2026-10-15 12:17\n",
        "type: Literature\n",
        "citekey: synth2024lattice\n",
        "itemType: journalArticle\n",
        "published: 2024-06-12\n",
        "authors:\n",
        "  - \"Voss, Elena\"\n",
        "  - K. Okafor\n",
        "keywords: [lattice, \"edge modes\"]\n",
        "DOI: 10.9999/synth.2024.001\n",
        "abstractNote: |-\n",
        "  First synthetic summary line.\n",
        "  Second synthetic summary line.\n",
        "---\n",
        "# Synthetic Title\n",
        "Body text.\n",
    );
    let (fm, body) = split_frontmatter(note);
    let fm = fm.expect("frontmatter present");
    assert!(body.starts_with("# Synthetic Title"));
    let map = parse_flat_yaml(fm);
    assert_eq!(map.get("citekey").and_then(FlatVal::as_str), Some("synth2024lattice"));
    assert_eq!(map.get("itemType").and_then(FlatVal::as_str), Some("journalArticle"));
    assert_eq!(map.get("published").and_then(FlatVal::as_str), Some("2024-06-12"));
    assert_eq!(
        map.get("authors").map(FlatVal::as_list),
        Some(vec!["Voss, Elena".to_string(), "K. Okafor".to_string()])
    );
    assert_eq!(
        map.get("keywords").map(FlatVal::as_list),
        Some(vec!["lattice".to_string(), "edge modes".to_string()])
    );
    assert_eq!(
        map.get("abstractNote").and_then(FlatVal::as_str),
        Some("First synthetic summary line.\nSecond synthetic summary line.")
    );
}

#[test]
fn parses_kandinsky_plugin_settings_toml() {
    let toml = r#"
# Kandinsky catalog-papers settings
dir = "refs/papers"
bib = "refs/refs.bib"
external_pdf_roots = [
  "../SharedPapers",
  "../ArchivePDFs",
]
arxiv_source_dir = ".kandinsky/cache/arxiv-src"
"#;
    let map = parse_flat_toml(toml);
    assert_eq!(map.get("dir").and_then(FlatVal::as_str), Some("refs/papers"));
    assert_eq!(
        map.get("external_pdf_roots").map(FlatVal::as_list),
        Some(vec!["../SharedPapers".to_string(), "../ArchivePDFs".to_string()])
    );
    assert_eq!(
        map.get("arxiv_source_dir").and_then(FlatVal::as_str),
        Some(".kandinsky/cache/arxiv-src")
    );
}

#[test]
fn kandinsky_entry_read_renames_note_fields_and_ignores_note_own_type_date_tags() {
    // Ported reading behavior from Kandinsky entry.rs tests with synthetic fixture.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("refs/papers/synth2024a");
    std::fs::create_dir_all(&dir).unwrap();
    let note = "---\n\
type: Literature\n\
date: 2026-10-15\n\
tags: [obsidian-only]\n\
citekey: synth2024a\n\
itemType: book\n\
published: 2024-03-01\n\
authors:\n\
  - Rivera, M.\n\
keywords:\n\
  - gauge-theory\n\
title: Synthetic Gauge Notes\n\
---\n\
# Synthetic Gauge Notes\n";
    std::fs::write(dir.join("synth2024a.md"), note).unwrap();

    let canon = std::fs::canonicalize(tmp.path()).unwrap();
    let (_, mapped) = read_entry_metadata(&canon, &canon.join("refs/papers/synth2024a"), "synth2024a")
        .expect("entry parsed");
    assert_eq!(mapped.get("type").and_then(FlatVal::as_str), Some("book"));
    assert_eq!(mapped.get("date").and_then(FlatVal::as_str), Some("2024-03-01"));
    assert_eq!(
        mapped.get("author").map(FlatVal::as_list),
        Some(vec!["Rivera, M.".to_string()])
    );
    assert_eq!(
        mapped.get("tags").map(FlatVal::as_list),
        Some(vec!["gauge-theory".to_string()])
    );
}

#[test]
fn kandinsky_entry_reads_legacy_info_yaml_and_skips_plain_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_empty = tmp.path().join("refs/papers/plain_dir");
    std::fs::create_dir_all(&dir_empty).unwrap();
    std::fs::write(dir_empty.join("plain_dir.md"), "---\ntitle: Just a note\n---\n").unwrap();
    let canon = std::fs::canonicalize(tmp.path()).unwrap();
    assert!(read_entry_metadata(&canon, &canon.join("refs/papers/plain_dir"), "plain_dir").is_none());

    let dir_legacy = tmp.path().join("refs/papers/legacy1999");
    std::fs::create_dir_all(&dir_legacy).unwrap();
    std::fs::write(
        dir_legacy.join(LEGACY_INFO_YAML),
        "title: Synthetic Legacy Paper\ntype: journalArticle\nauthor:\n- Cho, S.\ndate: 1999\nDOI: 10.9999/legacy.99\n",
    )
    .unwrap();
    let (_, mapped) = read_entry_metadata(&canon, &canon.join("refs/papers/legacy1999"), "legacy1999")
        .expect("legacy info.yaml parsed");
    assert_eq!(mapped.get("title").and_then(FlatVal::as_str), Some("Synthetic Legacy Paper"));
    assert_eq!(mapped.get("date").and_then(FlatVal::as_str), Some("1999"));
}

#[test]
fn clean_ref_token_strips_wikilinks_and_fragments() {
    assert_eq!(clean_ref_token("[[../Papers/Sub/foo.pdf#page=3|My PDF]]"), "../Papers/Sub/foo.pdf");
    assert_eq!(clean_ref_token("\"paper.pdf\""), "paper.pdf");
}
