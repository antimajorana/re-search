//! Bibliography and citation extraction for the deterministic TeX skeleton (Investigate V2).
//!
//! Extracts [`BibRef`] entries from inline `thebibliography` environments, `.bib` files, and
//! non-inlined `.bbl` files, normalizes arXiv IDs and DOIs via [`crate::paper_key`], and
//! records active citation occurrences and locators across the flattened TeX stream.

use super::flatten::{is_nested_tex_if, FlattenedTex, VERBATIM_ENVS};
use super::model::{BibLocator, BibRef, Span};
use super::tex_lex::{
    next_char_boundary, read_balanced_group, skip_inline_verb, skip_ws_and_comments, GroupOutcome,
};
use crate::paper_key::{canonical_doi, split_arxiv_id};
use crate::tex_scan::{
    parse_bib_entries, scan_arxiv_base_ids, scan_normalized_dois, strip_latex_comments,
    strip_latex_comments_line,
};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const MAX_RAW_CHARS: usize = 2_000;

/// One active citation occurrence located in the flattened TeX stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiteOccurrence {
    pub flat_line: usize,
    pub site: Span,
    pub key: String,
    pub locator: Option<String>,
}

/// Extracts deterministic bibliography entries (sorted by key) and all active citation occurrences.
pub fn extract_bibliography(
    flattened: &FlattenedTex,
    files: &BTreeMap<String, String>,
) -> (Vec<BibRef>, Vec<CiteOccurrence>) {
    let mut entries: BTreeMap<String, BibRef> = BTreeMap::new();
    let (active, line_starts) =
        mask_active_lines(flattened.lines.iter().map(|fl| fl.text.as_str()));

    parse_thebibliography_blocks(&active, &mut entries);
    for (path, content) in files {
        if has_ext_ci(path, "bib") {
            parse_bib_file(content, &mut entries);
        }
    }

    let mut inlined: BTreeSet<&str> = BTreeSet::from([flattened.primary_path.as_str()]);
    inlined.extend(flattened.inlined_files.iter().map(String::as_str));
    inlined.extend(flattened.lines.iter().map(|fl| fl.origin.path.as_str()));
    for (path, content) in files {
        if has_ext_ci(path, "bbl") && !inlined.contains(path.as_str()) {
            let (bbl_stream, _) = mask_active_lines(content.split('\n'));
            if !parse_thebibliography_blocks(&bbl_stream, &mut entries) {
                parse_bibitems(&bbl_stream, &mut entries);
            }
        }
    }

    let citations = scan_citations(&active, &line_starts, flattened, &mut entries);
    (entries.into_values().collect(), citations)
}

fn has_ext_ci(path: &str, ext: &str) -> bool { path.rsplit_once('.').is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext)) }
fn is_active_bs(bytes: &[u8], pos: usize) -> bool { bytes[..pos].iter().rev().take_while(|&&b| b == b'\\').count().is_multiple_of(2) }
fn canonical_arxiv(text: &str) -> Option<String> { scan_arxiv_base_ids(text).into_iter().find_map(|c| split_arxiv_id(&c).ok().map(|(b, _)| b)) }
fn canonical_doi_in(text: &str) -> Option<String> { scan_normalized_dois(text).into_iter().find_map(|c| canonical_doi(&c).ok()) }
fn offset_to_flat_line(line_starts: &[usize], offset: usize) -> usize { match line_starts.binary_search(&offset) { Ok(i) => i + 1, Err(i) => i.max(1) } }
fn empty_bib_ref(key: &str) -> BibRef { BibRef { key: key.to_string(), raw: String::new(), arxiv_id: None, doi: None, title: None, cited_at: Vec::new(), locators: Vec::new() } }
fn cap_raw(raw: &str) -> String { let t = raw.trim(); t.char_indices().nth(MAX_RAW_CHARS).map_or_else(|| t.to_string(), |(i, _)| t[..i].trim_end().to_string()) }

fn merge_bib_ref(
    entries: &mut BTreeMap<String, BibRef>, key: &str,
    raw: String, arxiv_id: Option<String>, doi: Option<String>, title: Option<String>,
) {
    let e = entries.entry(key.to_string()).or_insert_with(|| empty_bib_ref(key));
    if e.raw.is_empty() && !raw.is_empty() { e.raw = raw; }
    if e.arxiv_id.is_none() { e.arxiv_id = arxiv_id; }
    if e.doi.is_none() { e.doi = doi; }
    if e.title.is_none() { e.title = title; }
}

fn read_cmd_at(s: &str, pos: usize) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if pos + 1 >= b.len() || !b[pos + 1].is_ascii_alphabetic() { return None; }
    let mut k = pos + 1;
    while k < b.len() && b[k].is_ascii_alphabetic() { k += 1; }
    Some((&s[pos + 1..k], k))
}

fn read_closed(s: &str, pos: usize, open: u8, close: u8, par: bool) -> Option<(String, usize)> {
    let p = skip_ws_and_comments(s, pos);
    if s.as_bytes().get(p) != Some(&open) { return None; }
    match read_balanced_group(s, p, open, close, par) {
        GroupOutcome::Closed { content, next_pos } => Some((content, next_pos)),
        GroupOutcome::Unclosed { .. } => None,
    }
}

/// Joins lines into a comment-stripped active stream while skipping verbatim environments,
/// inline `\verb|...|`, and `\iffalse...\fi` blocks, preserving exact 1-based line offsets.
fn mask_active_lines<'a>(lines: impl Iterator<Item = &'a str>) -> (String, Vec<usize>) {
    let (mut stream, mut line_starts) = (String::new(), Vec::new());
    let (mut in_verb, mut iffalse_depth): (Option<String>, usize) = (None, 0);
    for (idx, raw_line) in lines.enumerate() {
        if idx > 0 { stream.push('\n'); }
        line_starts.push(stream.len());
        let mut slice = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        loop {
            if let Some(end_pat) = in_verb.as_deref().map(|e| format!("\\end{{{e}}}")) {
                let Some(rel) = slice.find(&end_pat) else { break; };
                slice = &slice[rel + end_pat.len()..];
                in_verb = None;
            }
            let code = strip_latex_comments_line(slice);
            let (bytes, mut i, mut resumed) = (code.as_bytes(), 0usize, None);
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    let Some((cmd, k)) = read_cmd_at(&code, i) else {
                        let end = next_char_boundary(&code, (i + 1).min(bytes.len()));
                        if iffalse_depth == 0 { stream.push_str(&code[i..end]); }
                        i = end;
                        continue;
                    };
                    if iffalse_depth > 0 {
                        if cmd == "fi" { iffalse_depth -= 1; }
                        else if cmd == "else" && iffalse_depth == 1 { iffalse_depth = 0; }
                        else if is_nested_tex_if(cmd) { iffalse_depth += 1; }
                        i = k;
                    } else if cmd == "iffalse" {
                        iffalse_depth = 1; stream.push(' '); i = k;
                    } else if cmd == "verb" {
                        stream.push(' '); i = skip_inline_verb(&code, k);
                    } else {
                        if cmd == "begin" {
                            if let Some((env, next_pos)) = read_closed(&code, k, b'{', b'}', false) {
                                let name = env.trim();
                                if VERBATIM_ENVS.contains(&name) {
                                    stream.push(' ');
                                    in_verb = Some(name.to_string());
                                    resumed = Some(next_pos);
                                    break;
                                }
                            }
                        }
                        stream.push_str(&code[i..k]);
                        i = k;
                    }
                    continue;
                }
                let end = next_char_boundary(&code, i);
                if iffalse_depth == 0 { stream.push_str(&code[i..end]); }
                i = end;
            }
            let Some(next_offset) = resumed else { break; };
            slice = &slice[next_offset.min(slice.len())..];
        }
    }
    (stream, line_starts)
}

fn parse_thebibliography_blocks(stream: &str, entries: &mut BTreeMap<String, BibRef>) -> bool {
    static ENV_RE: OnceLock<Regex> = OnceLock::new();
    let re = ENV_RE.get_or_init(|| Regex::new(r"\\(begin|end)\s*\{\s*thebibliography\s*\}").unwrap());
    let (bytes, mut open_start, mut found) = (stream.as_bytes(), None, false);
    for caps in re.captures_iter(stream) {
        let m = caps.get(0).unwrap();
        if !is_active_bs(bytes, m.start()) { continue; }
        if &caps[1] == "begin" && open_start.is_none() {
            found = true;
            open_start = Some(read_closed(stream, m.end(), b'{', b'}', false).map_or(m.end(), |(_, p)| p));
        } else if &caps[1] == "end" {
            if let Some(start) = open_start.take().filter(|&s| s <= m.start()) {
                parse_bibitems(&stream[start..m.start()], entries);
            }
        }
    }
    if let Some(start) = open_start { parse_bibitems(&stream[start..], entries); }
    found
}

fn parse_bibitems(block: &str, entries: &mut BTreeMap<String, BibRef>) {
    static ITEM_RE: OnceLock<Regex> = OnceLock::new();
    let re = ITEM_RE.get_or_init(|| Regex::new(r"\\bibitem\b").unwrap());
    let (bytes, mut items) = (block.as_bytes(), Vec::new());
    for m in re.find_iter(block) {
        if !is_active_bs(bytes, m.start()) { continue; }
        let cur = read_closed(block, m.end(), b'[', b']', false).map_or(m.end(), |(_, p)| p);
        if let Some((key, body_start)) = read_closed(block, cur, b'{', b'}', false) {
            items.push((m.start(), key.trim().to_string(), body_start));
        }
    }
    for (idx, (_, key, body_start)) in items.iter().enumerate() {
        if key.is_empty() { continue; }
        let raw = &block[*body_start..items.get(idx + 1).map_or(block.len(), |n| n.0)];
        merge_bib_ref(entries, key, cap_raw(raw), canonical_arxiv(raw), canonical_doi_in(raw), extract_bibitem_title(raw));
    }
}

fn extract_bibitem_title(raw: &str) -> Option<String> {
    let (bytes, mut i) = (raw.as_bytes(), 0usize);
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let Some((cmd, k)) = read_cmd_at(raw, i) else {
                i = next_char_boundary(raw, (i + 1).min(bytes.len()));
                continue;
            };
            if matches!(cmd, "emph" | "textit") {
                if let Some((c, next_pos)) = read_closed(raw, k, b'{', b'}', true) {
                    if let Some(t) = clean_tex_text(&c) { return Some(t); }
                    i = next_pos;
                    continue;
                }
            }
            i = k;
        } else if raw[i..].starts_with("``") {
            if let Some(rel) = raw[i + 2..].find("''") {
                if let Some(t) = clean_tex_text(raw[i + 2..i + 2 + rel].trim().trim_end_matches(',')) { return Some(t); }
                i += rel + 4;
            } else { i += 2; }
        } else if bytes[i] == b'"' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'"' {
                j = next_char_boundary(raw, if bytes[j] == b'\\' { (j + 1).min(bytes.len()) } else { j });
            }
            if j < bytes.len() {
                if let Some(t) = clean_tex_text(raw[i + 1..j].trim().trim_end_matches(',')) { return Some(t); }
                i = j + 1;
            } else { i += 1; }
        } else {
            i = next_char_boundary(raw, i);
        }
    }
    None
}

fn parse_bib_file(content: &str, entries: &mut BTreeMap<String, BibRef>) {
    static ENTRY_RE: OnceLock<Regex> = OnceLock::new();
    let re = ENTRY_RE.get_or_init(|| {
        Regex::new(r"(?m)^[^\S\r\n]*@([A-Za-z]+)\s*([\{\(])\s*([^,\s\{\}\(\)]+)\s*,").unwrap()
    });
    let (meta_map, stripped) = (parse_bib_entries(content), strip_latex_comments(content));
    let matches: Vec<_> = re.captures_iter(&stripped).collect();
    for (idx, caps) in matches.iter().enumerate() {
        if matches!(caps[1].to_ascii_lowercase().as_str(), "comment" | "preamble" | "string") { continue; }
        let key = caps[3].trim();
        if key.is_empty() { continue; }
        let (full, delim) = (caps.get(0).unwrap(), caps.get(2).unwrap());
        let open = delim.as_str().as_bytes()[0];
        let close = if open == b'{' { b'}' } else { b')' };
        let limit = matches.get(idx + 1).map_or(stripped.len(), |m| m.get(0).unwrap().start());
        let end = find_bib_entry_end(&stripped, delim.start(), open, close, limit);
        let raw = &stripped[full.start()..end];
        let body_end = end.saturating_sub(usize::from(end > full.end() && stripped.as_bytes().get(end - 1) == Some(&close)));
        let meta = meta_map.get(key);
        let arxiv = meta.and_then(|m| m.base_arxiv_id.as_deref())
            .and_then(|id| split_arxiv_id(id).ok().map(|(b, _)| b))
            .or_else(|| canonical_arxiv(raw));
        let doi = meta.and_then(|m| m.doi_lower.as_deref())
            .and_then(|d| canonical_doi(d).ok())
            .or_else(|| canonical_doi_in(raw));
        let title = extract_bib_title(&stripped[full.end()..body_end.max(full.end())]);
        merge_bib_ref(entries, key, cap_raw(raw), arxiv, doi, title);
    }
}

fn find_bib_entry_end(s: &str, open_pos: usize, open: u8, close: u8, limit: usize) -> usize {
    let (bytes, mut i, mut braces, mut parens, mut in_q) = (s.as_bytes(), open_pos, 0usize, 0usize, false);
    while i < limit {
        let b = bytes[i];
        if b == b'\\' { i = next_char_boundary(s, (i + 1).min(limit)).min(limit); continue; }
        if b == b'"' && braces == usize::from(open == b'{') { in_q = !in_q; }
        else if b == b'{' { braces += 1; }
        else if b == b'}' && braces > 0 { braces -= 1; if open == b'{' && braces == 0 { return i + 1; } }
        else if open == b'(' && braces == 0 && !in_q {
            if b == b'(' { parens += 1; }
            else if b == close && parens > 0 { parens -= 1; if parens == 0 { return i + 1; } }
        }
        i = next_char_boundary(s, i);
    }
    limit
}

fn extract_bib_title(body: &str) -> Option<String> {
    let (bytes, mut pos) = (body.as_bytes(), 0usize);
    while pos < bytes.len() {
        while pos < bytes.len() && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b',') { pos += 1; }
        let start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || matches!(bytes[pos], b'_' | b'-')) { pos += 1; }
        if pos == start { pos = next_char_boundary(body, pos); continue; }
        let field = &body[start..pos];
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if bytes.get(pos) != Some(&b'=') { continue; }
        let (val, next_pos) = read_bib_value(body, pos + 1);
        pos = next_pos;
        if field.eq_ignore_ascii_case("title") {
            if let Some(title) = clean_tex_text(&val) { return Some(title); }
        }
    }
    None
}

fn read_bib_value(s: &str, mut pos: usize) -> (String, usize) {
    let (bytes, mut out) = (s.as_bytes(), String::new());
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        match bytes.get(pos) {
            Some(&d @ (b'{' | b'"')) => {
                let (start, mut depth) = (pos + 1, usize::from(d == b'{'));
                pos += 1;
                while pos < bytes.len() {
                    if bytes[pos] == b'\\' { pos = next_char_boundary(s, (pos + 1).min(bytes.len())); continue; }
                    if bytes[pos] == b'{' { depth += 1; }
                    else if bytes[pos] == b'}' && depth > 0 { depth -= 1; if d == b'{' && depth == 0 { break; } }
                    else if d == b'"' && bytes[pos] == b'"' && depth == 0 { break; }
                    pos = next_char_boundary(s, pos);
                }
                out.push_str(&s[start..pos]);
                if bytes.get(pos) == Some(if d == b'{' { &b'}' } else { &b'"' }) { pos += 1; }
            }
            Some(_) => {
                let start = pos;
                while pos < bytes.len() && !matches!(bytes[pos], b',' | b'#' | b'}' | b')') && !bytes[pos].is_ascii_whitespace() {
                    pos = next_char_boundary(s, pos);
                }
                out.push_str(&s[start..pos]);
            }
            None => break,
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
        if bytes.get(pos) == Some(&b'#') { pos += 1; continue; }
        break;
    }
    (out, pos)
}

/// Strips protective LaTeX/BibTeX `{...}` braces (keeping command/script args) and collapses whitespace.
fn clean_tex_text(raw: &str) -> Option<String> {
    let (bytes, mut out, mut keep, mut arg_bearer, mut i) =
        (raw.as_bytes(), String::with_capacity(raw.len()), Vec::new(), false, 0usize);
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                if let Some((_, k)) = read_cmd_at(raw, i) {
                    out.push_str(&raw[i..k]); arg_bearer = true; i = k;
                } else {
                    let end = next_char_boundary(raw, (i + 1).min(bytes.len()));
                    out.push_str(&raw[i..end]);
                    arg_bearer = bytes.get(i + 1).is_some_and(|c| matches!(c, b'\'' | b'"' | b'`' | b'^' | b'~' | b'=' | b'.'));
                    i = end;
                }
            }
            b'{' => { keep.push(arg_bearer); if arg_bearer { out.push('{'); } arg_bearer = false; i += 1; }
            b'}' => { let kept = keep.pop().unwrap_or(false); if kept { out.push('}'); } arg_bearer = kept; i += 1; }
            b => { let end = next_char_boundary(raw, i); out.push_str(&raw[i..end]); arg_bearer = matches!(b, b'^' | b'_'); i = end; }
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

fn scan_citations(
    active: &str, line_starts: &[usize], flattened: &FlattenedTex, entries: &mut BTreeMap<String, BibRef>,
) -> Vec<CiteOccurrence> {
    static CITE_RE: OnceLock<Regex> = OnceLock::new();
    let re = CITE_RE.get_or_init(|| {
        Regex::new(r"\\(cite(?:author|year|alt|alp|p|t|s)|parencites?|textcites?|autocite|cite)\*?").unwrap()
    });
    let (bytes, mut out, mut min_pos) = (active.as_bytes(), Vec::new(), 0usize);
    for caps in re.captures_iter(active) {
        let (full, cmd) = (caps.get(0).unwrap(), caps.get(1).unwrap());
        if full.start() < min_pos || !is_active_bs(bytes, full.start())
            || bytes.get(cmd.end()).is_some_and(u8::is_ascii_alphabetic) { continue; }
        let is_multi = matches!(cmd.as_str(), "cites" | "parencites" | "textcites");
        let mut cur = full.end();
        if is_multi {
            for _ in 0..2 { if let Some((_, p)) = read_closed(active, cur, b'(', b')', false) { cur = p; } else { break; } }
        }
        let mut parsed = false;
        loop {
            let mut opts = Vec::new();
            for _ in 0..2 {
                if let Some((o, p)) = read_closed(active, cur, b'[', b']', false) { opts.push(o); cur = p; } else { break; }
            }
            let brace_pos = skip_ws_and_comments(active, cur);
            let Some((keys_raw, next_pos)) = read_closed(active, brace_pos, b'{', b'}', false) else { break; };
            let flat_line = offset_to_flat_line(line_starts, if parsed { brace_pos } else { full.start() });
            parsed = true;
            cur = next_pos;
            let locator = match opts.as_slice() {
                [] => None, [only] => clean_tex_text(only), [_, post, ..] => clean_tex_text(post),
            };
            if let Some(origin) = flattened.origin_of(flat_line) {
                let site = Span::new(origin.path.clone(), origin.line, origin.line);
                for key in keys_raw.split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "*") {
                    out.push(CiteOccurrence { flat_line, site: site.clone(), key: key.to_string(), locator: locator.clone() });
                    let e = entries.entry(key.to_string()).or_insert_with(|| empty_bib_ref(key));
                    if !e.cited_at.contains(&site) { e.cited_at.push(site.clone()); }
                    if let Some(ref text) = locator {
                        let loc = BibLocator { site: site.clone(), text: text.clone() };
                        if !e.locators.contains(&loc) { e.locators.push(loc); }
                    }
                }
            }
            if !is_multi { break; }
        }
        if parsed { min_pos = cur; }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::flatten::{FlattenedLine, OriginEntry};

    fn flat_from_tex(path: &str, tex: &str) -> FlattenedTex {
        let lines = tex.split('\n').enumerate().map(|(i, t)| FlattenedLine {
            text: t.to_string(), origin: OriginEntry { path: path.to_string(), line: i + 1 },
        }).collect();
        FlattenedTex { primary_path: path.to_string(), lines, inlined_files: Vec::new(), warnings: Vec::new() }
    }

    #[test]
    fn inline_bibitem_locators_and_cited_only() {
        let tex = "\\begin{document}\n\
By \\cite[Proposition 2.1]{keller2020} and \\citep[see][Theorem 2.1]{k1, k2}, plus \\cite{ghost}.\n\
\\begin{thebibliography}{99}\n\
\\bibitem[Kel20]{keller2020}\nB.~Keller, \\emph{On {DG} categories}, arXiv:math.KT/0601185v2, doi:10.4171/022-2/8.\n\
\\bibitem{k1}\nA.~Author, ``First Paper,'' J. Math. 2020.\n\
\\end{thebibliography}\n\\end{document}";
        let (refs, cites) = extract_bibliography(&flat_from_tex("main.tex", tex), &BTreeMap::new());
        let by_key: BTreeMap<&str, &BibRef> = refs.iter().map(|r| (r.key.as_str(), r)).collect();
        let kel = by_key["keller2020"];
        assert_eq!(kel.arxiv_id.as_deref(), Some("math/0601185"));
        assert_eq!(kel.doi.as_deref(), Some("10.4171/022-2/8"));
        assert_eq!(kel.title.as_deref(), Some("On DG categories"));
        assert_eq!(kel.cited_at, vec![Span::new("main.tex", 2, 2)]);
        assert_eq!(kel.locators, vec![BibLocator { site: Span::new("main.tex", 2, 2), text: "Proposition 2.1".into() }]);
        assert_eq!(by_key["k1"].title.as_deref(), Some("First Paper"));
        assert_eq!(by_key["k1"].locators[0].text, "Theorem 2.1");
        assert_eq!(by_key["k2"].raw, "");
        assert_eq!(by_key["k2"].locators[0].text, "Theorem 2.1");
        assert_eq!(by_key["ghost"].raw, "");
        assert_eq!(by_key["ghost"].cited_at, vec![Span::new("main.tex", 2, 2)]);
        assert_eq!(cites.len(), 4);
    }

    #[test]
    fn bib_nested_braces_bbl_and_verbatim_comment_ignoring() {
        let tex = "% \\cite{commented}\n\\verb|\\cite{verbinline}|\n\\begin{verbatim}\n\\cite{verbenv}\n\\end{verbatim}\n\
\\iffalse\n\\cite{disabled}\n\\fi\n\
Real \\cite[{Theorem 3.4, [a]}]{nested2023} and \\citep[see][]{nested2023} and \\citet*{bbl2024}.";
        let bib = "@article{nested2023,\n  author = {A. Researcher},\n  booktitle = {Not Title},\n\
  title = {On {{H}odge} {T}heory and {K3} {Surfaces}},\n  eprint = {2305.01234v3},\n\
  doi = {https://doi.org/10.1007/s00222-023-00001-1},\n}";
        let bbl = "\\begin{thebibliography}{9}\n\\bibitem{bbl2024}\nC. Writer, \"Quoted Title,\" arXiv:2401.00001.\n\\end{thebibliography}";
        let files = BTreeMap::from([("refs.BIB".into(), bib.into()), ("main.bbl".into(), bbl.into())]);
        let (refs, cites) = extract_bibliography(&flat_from_tex("main.tex", tex), &files);
        assert_eq!(refs.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), vec!["bbl2024", "nested2023"]);
        assert_eq!(refs[0].title.as_deref(), Some("Quoted Title"));
        assert_eq!(refs[0].arxiv_id.as_deref(), Some("2401.00001"));
        assert_eq!(refs[1].title.as_deref(), Some("On Hodge Theory and K3 Surfaces"));
        assert_eq!(refs[1].arxiv_id.as_deref(), Some("2305.01234"));
        assert_eq!(refs[1].doi.as_deref(), Some("10.1007/s00222-023-00001-1"));
        assert_eq!(refs[1].cited_at, vec![Span::new("main.tex", 9, 9)]);
        assert_eq!(refs[1].locators, vec![BibLocator { site: Span::new("main.tex", 9, 9), text: "Theorem 3.4, [a]".into() }]);
        assert_eq!(cites.len(), 3);
        assert_eq!(cites[0].locator.as_deref(), Some("Theorem 3.4, [a]"));
        assert_eq!(cites[1].locator, None);
    }
}
