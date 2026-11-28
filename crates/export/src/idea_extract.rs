//! Ledger extraction, F4 quote re-matching, Rule C3/C8 checks, and BibTeX hint helpers for `crate::idea`.

use super::{
    check_text_s8, AbstractQuoteExport, IdeaExportJson, IdeaStatementExport, MatchedQuoteExport,
    PaperReadExport, EVENT_IDEA_ANSWER, EVENT_IDEA_BRIEF, EVENT_IDEA_PAPER, EVENT_IDEA_STATED,
    EVENT_SOURCE_INGESTED,
};
use ledger::Event;
use research_tools::paper_key::{PaperIds, PaperKey};
use research_tools::prior_work::{evidence_display_grade, TextBasis};
use research_tools::prior_work_wording::{display_grade_label, wording};
use research_tools::quote_core::{
    clip_quote, quote_sha256, verify_quote_ref, QuoteRef, EVIDENCE_ABSTRACT_EVENT,
    EVIDENCE_QUOTE_EVENT,
};
use research_tools::WorkspaceSandbox;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Default)]
pub(super) struct PaperBibHint {
    pub(super) ids: PaperIds,
    pub(super) version: Option<String>,
    pub(super) title: String,
    pub(super) authors: Vec<String>,
    pub(super) year: Option<i32>,
}

pub(super) fn merge_bib_hint(dst: &mut PaperBibHint, src: &PaperBibHint) {
    merge_paper_ids(&mut dst.ids, &src.ids);
    if dst.version.is_none() {
        dst.version = src.version.clone();
    }
    if dst.title.is_empty() && !src.title.is_empty() {
        dst.title = src.title.clone();
    }
    if dst.authors.is_empty() && !src.authors.is_empty() {
        dst.authors = src.authors.clone();
    }
    if dst.year.is_none() {
        dst.year = src.year;
    }
}

pub(super) fn extract_statement(events: &[Event], stmt_seq: u64) -> Option<IdeaStatementExport> {
    let ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_STATED && e.seq == stmt_seq)?;
    let rev = ev.payload.get("rev").and_then(Value::as_u64).unwrap_or(1);
    Some(IdeaStatementExport {
        seq: ev.seq,
        rev,
        text: str_field(&ev.payload, "text"),
        text_sha256: str_field(&ev.payload, "text_sha256"),
        fields: string_vec(&ev.payload, "fields"),
        seeds: string_vec(&ev.payload, "seeds"),
        author: ev
            .payload
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string(),
    })
}

/// Fold latest `idea.brief` and subsequent `idea.answer` edits/answers (mirroring `idea_view_coverage::fold_answers`).
pub(super) fn fold_brief_and_answers(
    events: &[Event],
    stmt_seq: u64,
) -> (Option<Value>, BTreeMap<String, String>) {
    let latest_brief = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > stmt_seq);
    let brief_seq = latest_brief.map_or(stmt_seq, |e| e.seq);
    let mut merged = latest_brief.map(|e| e.payload.clone());
    let mut question_answers = BTreeMap::new();

    for ev in events {
        if ev.kind != EVENT_IDEA_ANSWER || ev.seq <= stmt_seq || ev.seq < brief_seq {
            continue;
        }
        let p = &ev.payload;
        if let Some(ans_obj) = p.get("answers").and_then(Value::as_object) {
            for (qid, ans) in ans_obj {
                if let Some(text) = ans.as_str() {
                    question_answers.insert(qid.clone(), text.to_string());
                }
            }
        } else if let Some(ans_arr) = p.get("answers").and_then(Value::as_array) {
            for item in ans_arr {
                if let (Some(qid), Some(ans)) = (
                    item.get("question_id").and_then(Value::as_str),
                    item.get("answer").and_then(Value::as_str),
                ) {
                    question_answers.insert(qid.to_string(), ans.to_string());
                }
            }
        } else if let (Some(qid), Some(ans)) = (
            p.get("question_id").and_then(Value::as_str),
            p.get("answer").and_then(Value::as_str),
        ) {
            question_answers.insert(qid.to_string(), ans.to_string());
        }

        if let Some(Value::Object(ref mut bobj)) = merged {
            if let Some(restatement) = p.get("restatement").and_then(Value::as_str) {
                bobj.insert("restatement".into(), Value::String(restatement.to_string()));
            }
            if let Some(assumptions) = p.get("assumptions").and_then(Value::as_array) {
                bobj.insert("assumptions".into(), Value::Array(assumptions.clone()));
            }
            if let Some(perspectives) = p.get("perspectives").and_then(Value::as_array) {
                bobj.insert("perspectives".into(), Value::Array(perspectives.clone()));
            }
            if let Some(exclusions) = p.get("exclusions").and_then(Value::as_array) {
                bobj.insert("exclusions".into(), Value::Array(exclusions.clone()));
            }
        }
    }

    if let Some(Value::Object(ref mut bobj)) = merged {
        if !question_answers.is_empty() {
            let ans_map: Map<String, Value> = question_answers
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            bobj.insert("answers".into(), Value::Object(ans_map));
        }
    }
    (merged, question_answers)
}

pub(super) fn collect_papers_read(events: &[Event], stmt_seq: u64) -> Vec<PaperReadExport> {
    let mut by_key: BTreeMap<String, PaperReadExport> = BTreeMap::new();
    for ev in events {
        if ev.kind != EVENT_IDEA_PAPER || ev.seq <= stmt_seq {
            continue;
        }
        let Some(key) = ev
            .payload
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        by_key.insert(
            key.to_string(),
            PaperReadExport {
                key: key.to_string(),
                version: opt_str(&ev.payload, "version"),
                source_id: opt_str(&ev.payload, "source_id"),
                source_seq: ev.payload.get("source_seq").and_then(Value::as_u64),
                origin: str_field(&ev.payload, "origin"),
                text_basis: ev
                    .payload
                    .get("text_basis")
                    .and_then(Value::as_str)
                    .unwrap_or("tex")
                    .to_string(),
                status: str_field(&ev.payload, "status"),
                reason: opt_str(&ev.payload, "reason"),
                lines_sent: ev.payload.get("lines_sent").and_then(Value::as_u64).unwrap_or(0),
                total_lines: ev.payload.get("total_lines").and_then(Value::as_u64).unwrap_or(0),
                raw_blob: opt_str(&ev.payload, "raw_blob"),
            },
        );
    }
    by_key.into_values().collect()
}

pub(super) fn collect_and_match_quotes(
    data_dir: &Path,
    session: &str,
    events: &[Event],
    stmt_seq: u64,
    papers_read: &[PaperReadExport],
) -> Result<Vec<MatchedQuoteExport>, String> {
    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let pdf_pages = collect_source_pdf_pages(events);
    let paper_basis_map: BTreeMap<&str, &str> = papers_read
        .iter()
        .map(|p| (p.key.as_str(), p.text_basis.as_str()))
        .collect();
    let mut out = Vec::new();

    for ev in events {
        if ev.kind != EVIDENCE_QUOTE_EVENT || ev.seq <= stmt_seq {
            continue;
        }
        let p = &ev.payload;
        let source_id = opt_str(p, "source_id").ok_or_else(|| {
            format!("Explore export refused: quote xe-{} failed F4 re-matching (missing source_id)", ev.seq)
        })?;
        let path = opt_str(p, "path").ok_or_else(|| {
            format!("Explore export refused: quote xe-{} failed F4 re-matching (missing path)", ev.seq)
        })?;
        let file_sha256 = opt_str(p, "file_sha256")
            .filter(|s| is_sha256_hex(s))
            .ok_or_else(|| {
                format!("Explore export refused: quote xe-{} failed F4 re-matching (invalid file_sha256)", ev.seq)
            })?;
        let stored_quote_sha = opt_str(p, "quote_sha256")
            .filter(|s| is_sha256_hex(s))
            .ok_or_else(|| {
                format!("Explore export refused: quote xe-{} failed F4 re-matching (invalid quote_sha256)", ev.seq)
            })?;
        let raw_quote = str_field(p, "quote");
        if raw_quote.trim().is_empty() {
            return Err(format!(
                "Explore export refused: quote xe-{} failed F4 re-matching (empty quote)",
                ev.seq
            ));
        }
        let start_line = p.get("start_line").and_then(Value::as_u64).unwrap_or(0) as usize;
        let end_line = p
            .get("end_line")
            .and_then(Value::as_u64)
            .unwrap_or(start_line as u64) as usize;

        let expected = QuoteRef {
            source_id,
            path,
            start_line,
            end_line,
            file_sha256,
            quote_sha256: stored_quote_sha,
            evidence_seq: Some(ev.seq),
        };
        let matched = verify_quote_ref(&sandbox, session, events, &expected, &raw_quote).map_err(
            |fault| {
                format!(
                    "Explore export refused: quote xe-{} failed F4 re-matching ({fault})",
                    ev.seq
                )
            },
        )?;

        let paper_key = opt_str(p, "paper_key");
        let (page, page_basis) = resolve_pdf_page_and_basis(
            pdf_pages.get(&matched.quote_ref.source_id),
            matched.quote_ref.start_line,
            matched.quote_ref.end_line,
        );
        let explicit_basis = p.get("text_basis").and_then(Value::as_str).filter(|s| !s.trim().is_empty());
        let paper_basis = paper_key
            .as_deref()
            .and_then(|k| paper_basis_map.get(k).copied())
            .unwrap_or("tex");
        let eff_basis_str = pick_effective_basis(page_basis, explicit_basis, paper_basis);
        let eff_basis = TextBasis::parse(eff_basis_str).unwrap_or(TextBasis::Tex);
        let ocr_tag = eff_basis
            .is_ocr_source()
            .then(|| wording::TAG_OCR.to_string());
        let display_grade =
            display_grade_label(evidence_display_grade(false, eff_basis)).to_string();

        out.push(MatchedQuoteExport {
            id: format!("xe-{}", ev.seq),
            seq: ev.seq,
            paper_key,
            source_id: matched.quote_ref.source_id,
            path: matched.quote_ref.path,
            start_line: matched.quote_ref.start_line,
            end_line: matched.quote_ref.end_line,
            file_sha256: matched.quote_ref.file_sha256,
            quote_sha256: matched.quote_ref.quote_sha256,
            quote: clip_quote(raw_quote.trim()).to_string(),
            excerpt: clip_quote(matched.excerpt.trim()).to_string(),
            role: opt_str(p, "role"),
            gist: opt_str(p, "gist"),
            subclaim_ids: string_vec(p, "subclaim_ids"),
            page,
            text_basis: eff_basis_str.to_string(),
            ocr_tag,
            display_grade,
        });
    }
    Ok(out)
}

/// Validate and collect Rule C3 verbatim `evidence.abstract` records (`xa-<seq>`).
pub(super) fn collect_abstract_quotes(
    events: &[Event],
    stmt_seq: u64,
) -> Result<Vec<AbstractQuoteExport>, String> {
    let mut out = Vec::new();
    for e in events {
        if e.kind != EVIDENCE_ABSTRACT_EVENT || e.seq <= stmt_seq {
            continue;
        }
        let p = &e.payload;
        if p.get("verbatim").and_then(Value::as_bool) == Some(false) {
            return Err(format!(
                "Explore export refused: Rule C3 rejects non-verbatim abstract quote xa-{}",
                e.seq
            ));
        }
        let raw_quote = str_field(p, "quote");
        if raw_quote.trim().is_empty() {
            return Err(format!(
                "Explore export refused: Rule C3 rejects empty abstract quote xa-{}",
                e.seq
            ));
        }
        if let Some(qsha) = opt_str(p, "quote_sha256") {
            if !is_sha256_hex(&qsha) || quote_sha256(&raw_quote) != qsha {
                return Err(format!(
                    "Explore export refused: Rule C3 abstract quote xa-{} failed quote_sha256 check",
                    e.seq
                ));
            }
        }
        let start = p.get("start").and_then(Value::as_u64);
        let end = p.get("end").and_then(Value::as_u64);
        if let (Some(s), Some(en)) = (start, end) {
            if en <= s {
                return Err(format!(
                    "Explore export refused: Rule C3 abstract quote xa-{} has invalid span [{s}, {en})",
                    e.seq
                ));
            }
        }
        let provider = opt_str(p, "provider").ok_or_else(|| {
            format!("Explore export refused: Rule C3 abstract quote xa-{} missing provider", e.seq)
        })?;
        let raw_blob = opt_str(p, "raw_blob")
            .filter(|s| is_sha256_hex(s))
            .ok_or_else(|| {
                format!("Explore export refused: Rule C3 abstract quote xa-{} missing raw_blob sha256", e.seq)
            })?;
        out.push(AbstractQuoteExport {
            id: format!("xa-{}", e.seq),
            seq: e.seq,
            paper_key: opt_str(p, "paper_key"),
            quote: clip_quote(raw_quote.trim()).to_string(),
            start,
            end,
            provider: Some(provider),
            raw_blob: Some(raw_blob),
        });
    }
    Ok(out)
}

pub(super) fn collect_paper_bib_hints(model: &IdeaExportJson) -> BTreeMap<PaperKey, PaperBibHint> {
    let mut map: BTreeMap<PaperKey, PaperBibHint> = BTreeMap::new();
    for cand in &model.candidates {
        let Some(key) = cand.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) else {
            continue;
        };
        let hint = map.entry(key).or_default();
        if let Some(ids_val) = cand.get("ids") {
            if let Ok(parsed) = serde_json::from_value::<PaperIds>(ids_val.clone()) {
                merge_paper_ids(&mut hint.ids, &parsed);
            }
        }
        if hint.title.is_empty() { hint.title = str_field(cand, "title"); }
        if hint.authors.is_empty() { hint.authors = string_vec(cand, "authors"); }
        if hint.year.is_none() {
            hint.year = cand.get("year").and_then(Value::as_i64).and_then(|y| i32::try_from(y).ok());
        }
    }
    for pr in &model.papers_read {
        if let Ok(key) = PaperKey::parse(&pr.key) {
            let hint = map.entry(key).or_default();
            if hint.version.is_none() { hint.version = pr.version.clone(); }
        }
    }
    for q in &model.matched_quotes {
        if let Some(key) = q.paper_key.as_deref().and_then(|s| PaperKey::parse(s).ok()) {
            map.entry(key).or_default();
        }
    }
    for q in &model.abstract_quotes {
        if let Some(key) = q.paper_key.as_deref().and_then(|s| PaperKey::parse(s).ok()) {
            map.entry(key).or_default();
        }
    }
    if let Some(orig) = model.origin_paper.as_deref().and_then(|s| PaperKey::parse(s).ok()) {
        map.entry(orig).or_default();
    }
    for v in &model.verdicts {
        for ov in &v.overlaps { map.entry(ov.paper_key.clone()).or_default(); }
        for am in &v.abstract_matches { map.entry(am.paper_key.clone()).or_default(); }
    }
    if let Some(cmap) = &model.concept_map {
        for node in cmap.get("nodes").and_then(Value::as_array).into_iter().flatten() {
            for pk in string_vec(node, "papers") {
                if let Ok(key) = PaperKey::parse(&pk) { map.entry(key).or_default(); }
            }
            if let Some(key) = node.get("paper_key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) {
                map.entry(key).or_default();
            }
        }
        for ov in cmap.get("overlaps").and_then(Value::as_array).into_iter().flatten() {
            if let Some(key) = ov.get("paper_key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) {
                map.entry(key).or_default();
            }
        }
    }
    if let Some(stmt) = &model.statement {
        for seed in &stmt.seeds {
            if let Ok(key) = PaperKey::parse(seed) { map.entry(key).or_default(); }
        }
    }
    for (key, hint) in &mut map { fill_id_from_key(key, &mut hint.ids); }
    map
}

pub(super) fn render_bibtex_header(
    model: &IdeaExportJson,
    entry_count: usize,
) -> Result<String, String> {
    let scope_line = sanitize_bib_comment(&model.coverage_scope_summary);
    let session_line = sanitize_bib_comment(&model.session_id);
    let head_line = sanitize_bib_comment(&model.head_hash);
    let title_line = "% re:Search Explore bibliography export";
    let entries_line = format!("% Entries: {entry_count}");
    let scope_comment = format!("% Searched scope: {scope_line}");
    check_text_s8("bibtex_header.title", title_line)?;
    check_text_s8("bibtex_header.entries", &entries_line)?;
    check_text_s8("bibtex_header.scope", &scope_comment)?;
    let mut out = String::new();
    let _ = writeln!(out, "{title_line}");
    let _ = writeln!(out, "% Session: {session_line}");
    let _ = writeln!(out, "% Ledger head: {head_line}");
    let _ = writeln!(out, "{entries_line}");
    let _ = writeln!(out, "{scope_comment}");
    Ok(out)
}

/// Rule C8: match any PDF page overlapping `[start_line, end_line]` with OCR priority.
fn resolve_pdf_page_and_basis<'a>(
    spans: Option<&'a Vec<(usize, usize, usize, String)>>,
    start_line: usize,
    end_line: usize,
) -> (Option<usize>, Option<&'a str>) {
    let Some(spans) = spans else { return (None, None) };
    let lo = start_line.min(end_line);
    let hi = start_line.max(end_line);
    let mut first_page: Option<usize> = None;
    let mut best_basis: Option<&'a str> = None;
    for (pg, first, last, basis) in spans {
        if *first <= hi && *last >= lo {
            if first_page.is_none() {
                first_page = Some(*pg);
            }
            let is_ocr = TextBasis::parse(basis).is_some_and(TextBasis::is_ocr_source);
            if is_ocr || best_basis.is_none() {
                best_basis = Some(basis.as_str());
                if is_ocr {
                    break;
                }
            }
        }
    }
    (first_page, best_basis)
}

fn pick_effective_basis<'a>(
    page_basis: Option<&'a str>,
    explicit_basis: Option<&'a str>,
    paper_basis: &'a str,
) -> &'a str {
    for cand in [page_basis, explicit_basis, Some(paper_basis)].into_iter().flatten() {
        if TextBasis::parse(cand).is_some_and(TextBasis::is_ocr_source) {
            return cand;
        }
    }
    explicit_basis.or(page_basis).unwrap_or(paper_basis)
}

fn collect_source_pdf_pages(events: &[Event]) -> BTreeMap<String, Vec<(usize, usize, usize, String)>> {
    let mut map = BTreeMap::new();
    for ev in events {
        if ev.kind != EVENT_SOURCE_INGESTED { continue; }
        let (Some(sid), Some(pages)) = (
            ev.payload.get("source_id").and_then(Value::as_str),
            ev.payload.get("pdf_pages").and_then(Value::as_array),
        ) else {
            continue;
        };
        let spans: Vec<(usize, usize, usize, String)> = pages
            .iter()
            .filter_map(|p| {
                Some((
                    p.get("page")?.as_u64()? as usize,
                    p.get("first_line")?.as_u64()? as usize,
                    p.get("last_line")?.as_u64()? as usize,
                    p.get("basis").and_then(Value::as_str).unwrap_or("text_layer").to_string(),
                ))
            })
            .collect();
        if !spans.is_empty() { map.insert(sid.to_string(), spans); }
    }
    map
}

fn sanitize_bib_comment(raw: &str) -> String {
    raw.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

fn merge_paper_ids(dst: &mut PaperIds, src: &PaperIds) {
    if dst.arxiv.is_none() { dst.arxiv = src.arxiv.clone(); }
    if dst.doi.is_none() { dst.doi = src.doi.clone(); }
    if dst.inspire.is_none() { dst.inspire = src.inspire.clone(); }
    if dst.s2.is_none() { dst.s2 = src.s2.clone(); }
}

fn fill_id_from_key(key: &PaperKey, ids: &mut PaperIds) {
    let raw = key.as_str();
    if let Some(rest) = raw.strip_prefix("arxiv:") {
        if ids.arxiv.is_none() { ids.arxiv = Some(rest.to_string()); }
    } else if let Some(rest) = raw.strip_prefix("doi:") {
        if ids.doi.is_none() { ids.doi = Some(rest.to_string()); }
    } else if let Some(rest) = raw.strip_prefix("inspire:") {
        if ids.inspire.is_none() { ids.inspire = Some(rest.to_string()); }
    } else if let Some(rest) = raw.strip_prefix("s2:") {
        if ids.s2.is_none() { ids.s2 = Some(rest.to_string()); }
    }
}

fn str_field(val: &Value, key: &str) -> String {
    val.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn opt_str(val: &Value, key: &str) -> Option<String> {
    val.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn string_vec(val: &Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}
