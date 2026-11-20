//! Deterministic repair for control characters produced when LLMs emit
//! single-backslash LaTeX macros (`\frac`, `\beta`, `\rho`, `\theta`, `\alpha`)
//! inside JSON strings.

use serde_json::Value;

const F_STEMS: &[&str] = &[
    "rac", "orall", "lat", "rown", "box", "ramebox", "ootnote", "ootnotemark", "ootnotetext",
    "lushleft", "lushright",
];

const B_STEMS: &[&str] = &[
    "eta", "ar", "egin", "ig", "igg", "igl", "igr", "iggl", "iggr", "igm", "igcup", "igcap",
    "igoplus", "igotimes", "igodot", "igvee", "igwedge", "igsqcup", "iguplus", "inom", "m",
    "oldsymbol", "oxed", "ox", "f", "fseries", "ullet", "ackslash", "acksim", "ackepsilon",
    "ibitem", "ibliography", "ibliographystyle", "ot", "owtie", "reve",
];

const R_STEMS: &[&str] = &[
    "ho", "ight", "ightarrow", "ightleftarrows", "ightleftharpoons", "ightharpoonup",
    "ightharpoondown", "ightarrowtail", "floor", "ceil", "angle", "aisebox", "ef",
    "enewcommand", "m", "mfamily", "ule", "hd", "times", "vert", "Vert",
];

const T_STEMS: &[&str] = &[
    "heta", "hetasym", "au", "imes", "ext", "extbf", "extit", "extrm", "extsf", "exttt",
    "extup", "extnormal", "extsc", "extcolor", "ilde", "o", "op", "anh", "an", "frac",
    "riangle", "riangleleft", "riangleright", "riangleq", "ag", "itle",
];

const A_STEMS: &[&str] = &[
    "lpha", "pprox", "pproxeq", "st", "symp", "cute", "leph", "malg", "ngle", "rray",
    "uthor", "utoref", "rcsin", "rccos", "rctan", "rg",
];

/// Returns `true` when `line` looks like a tab-separated values (TSV) header or data row
/// so `\t` (`\x09`) column delimiters are never rewritten as LaTeX `\t...` commands.
pub fn is_likely_tsv_line(line: &str) -> bool {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let tab_count = trimmed.bytes().filter(|&b| b == b'\t').count();
    if tab_count == 0 || trimmed.contains(['{', '}', '\\', '$']) {
        return false;
    }
    let cells: Vec<&str> = trimmed.split('\t').collect();
    for pair in cells.windows(2) {
        let prev = pair[0];
        let next = pair[1];
        if match_stem(next.as_bytes(), T_STEMS)
            && prev.as_bytes().last().is_some_and(|b| {
                matches!(
                    b,
                    b'=' | b'^' | b'_' | b'(' | b'[' | b'+' | b'-' | b'*' | b'/'
                        | b'<' | b'>' | b',' | b':' | b';' | b'|' | b'~'
                )
            })
        {
            return false;
        }
    }
    if tab_count >= 2 {
        return cells.iter().all(|c| {
            !c.is_empty() && !c.ends_with(' ') && c.len() <= 40 && c.split_whitespace().count() <= 3
        });
    }
    cells.len() == 2
        && cells.iter().all(|c| {
            let t = c.trim();
            !t.is_empty() && !t.contains(' ') && t.len() <= 32
        })
}

/// Repairs control-character LaTeX corruptions (`\x0crac` -> `\frac`, `\x08eta` -> `\beta`,
/// `\x0dho` -> `\rho`, `\x09heta` -> `\theta`, `\x07lpha` -> `\alpha`) while skipping
/// fenced code blocks (` ``` ` / `~~~`), preserving CRLF (`\r\n`), and skipping TSV rows.
pub fn repair_under_escaped_latex(input: &str) -> String {
    if !input.bytes().any(|b| matches!(b, 0x07 | 0x08 | 0x09 | 0x0c | 0x0d)) {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len() + 16);
    let mut active_fence: Option<(u8, usize)> = None;

    for raw_segment in split_preserving_line_endings(input) {
        let (body, ending) = split_line_ending(raw_segment);

        if let Some((fence_ch, fence_len)) = active_fence {
            out.push_str(body);
            out.push_str(ending);
            if is_closing_fence(body, fence_ch, fence_len) {
                active_fence = None;
            }
            continue;
        }

        if let Some((fence_ch, fence_len)) = detect_opening_fence(body) {
            active_fence = Some((fence_ch, fence_len));
            out.push_str(body);
            out.push_str(ending);
            continue;
        }

        let skip_tabs = is_likely_tsv_line(body);
        repair_line_body(body, skip_tabs, &mut out);
        out.push_str(ending);
    }

    out
}

fn repair_line_body(body: &str, skip_tabs: bool, out: &mut String) {
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let replacement: Option<char> = match b {
            0x0c => match_stem(&bytes[i + 1..], F_STEMS).then_some('f'),
            0x08 => match_stem(&bytes[i + 1..], B_STEMS).then_some('b'),
            0x0d => match_stem(&bytes[i + 1..], R_STEMS).then_some('r'),
            0x09 if !skip_tabs => match_stem(&bytes[i + 1..], T_STEMS).then_some('t'),
            0x07 => match_stem(&bytes[i + 1..], A_STEMS).then_some('a'),
            _ => None,
        };
        if let Some(letter) = replacement {
            out.push('\\');
            out.push(letter);
            i += 1;
        } else {
            let ch = body[i..].chars().next().expect("valid UTF-8");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
}

fn match_stem(rest: &[u8], allowed: &[&str]) -> bool {
    let len = rest.iter().take_while(|&&b| b.is_ascii_alphabetic()).count();
    if len == 0 {
        return false;
    }
    let stem = std::str::from_utf8(&rest[..len]).unwrap_or("");
    allowed.contains(&stem)
}

fn split_preserving_line_endings(s: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0usize;
    for (idx, b) in s.bytes().enumerate() {
        if b == b'\n' {
            segments.push(&s[start..=idx]);
            start = idx + 1;
        }
    }
    if start < s.len() {
        segments.push(&s[start..]);
    }
    segments
}

fn split_line_ending(seg: &str) -> (&str, &str) {
    if let Some(body) = seg.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = seg.strip_suffix('\n') {
        (body, "\n")
    } else {
        (seg, "")
    }
}

fn detect_opening_fence(line: &str) -> Option<(u8, usize)> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let first = *trimmed.as_bytes().first()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let count = trimmed.bytes().take_while(|&b| b == first).count();
    if count < 3 || (first == b'`' && trimmed[count..].contains('`')) {
        return None;
    }
    Some((first, count))
}

fn is_closing_fence(line: &str, fence_ch: u8, min_len: usize) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let count = trimmed.bytes().take_while(|&b| b == fence_ch).count();
    count >= min_len && trimmed[count..].trim_matches([' ', '\t']).is_empty()
}

/// Returns `true` for JSON object keys holding code, IDs, hashes, blobs, file paths, or URLs.
pub fn should_skip_json_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "code" | "python_code" | "lean_code" | "script" | "srepr" | "command" | "cmd"
            | "regex" | "pattern" | "raw" | "id" | "hash" | "digest" | "blob" | "raw_blob"
            | "prev_hash" | "event_hash" | "tool_def_hash" | "sha256" | "sha256_before"
            | "sha256_after" | "nonce" | "signature" | "thought_signature" | "token"
            | "path" | "rel_path" | "file" | "filename" | "file_name" | "dir" | "cwd"
            | "workspace_root" | "pdf_path" | "url" | "uri" | "href" | "endpoint"
            | "api_base" | "mailto" | "depends_on" | "replaced_by" | "arxiv_id" | "doi"
    ) {
        return true;
    }
    for suffix in [
        "_id", "id", "_hash", "_digest", "_blob", "_sha256", "_token", "_path", "_dir",
        "_file", "_url", "_uri", "_endpoint", "_ids",
    ] {
        if suffix == "id" {
            if key.ends_with("Id") || key.ends_with("Ids") {
                return true;
            }
        } else if lower.ends_with(suffix) {
            return true;
        }
    }
    false
}

/// Recursively repairs under-escaped LaTeX inside prose string fields of `value`,
/// skipping code, identifier, hash/digest, path, and URL keys.
pub fn repair_json_value_latex(value: &mut Value) {
    match value {
        Value::String(s) => {
            let fixed = repair_under_escaped_latex(s);
            if fixed != *s {
                *s = fixed;
            }
        }
        Value::Array(items) => {
            for item in items {
                repair_json_value_latex(item);
            }
        }
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if !should_skip_json_key(k) {
                    repair_json_value_latex(v);
                }
            }
        }
        _ => {}
    }
}
