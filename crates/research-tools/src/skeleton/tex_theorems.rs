//! LaTeX theorem-declaration (`\newtheorem`, `\declaretheorem`, `\spnewtheorem`,
//! `\newmdtheoremenv`, wrapper `\newenvironment`) parsers.

use super::model::SkNodeKind;
use super::tex_lex::{clean_tex_title, read_balanced_group, skip_ws_and_comments, GroupOutcome};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One theorem-like environment declaration parsed from `\newtheorem` / `\declaretheorem` / etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TheoremDef {
    pub env_name: String,
    pub kind: SkNodeKind,
    pub display_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_counter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_counter: Option<String>,
    pub starred: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// Result of looking up an environment name in [`TheoremTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TheoremLookup {
    pub kind: SkNodeKind,
    pub declared: bool,
    pub starred: bool,
}

/// Table of declared theorem-like environments and custom proof wrappers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TheoremTable {
    pub defs: BTreeMap<String, TheoremDef>,
    pub proof_envs: BTreeSet<String>,
    pub current_style: Option<String>,
}

impl TheoremTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Looks up `env_name` among explicit declarations first, then conservative built-in
    /// standard LaTeX theorem environment names (for external `.cls`/`.sty` classes).
    pub fn lookup(&self, env_name: &str) -> Option<TheoremLookup> {
        if let Some(def) = self.defs.get(env_name) {
            return Some(TheoremLookup {
                kind: def.kind,
                declared: true,
                starred: def.starred,
            });
        }
        builtin_theorem_kind(env_name).map(|kind| TheoremLookup {
            kind,
            declared: false,
            starred: env_name.ends_with('*'),
        })
    }

    pub fn is_proof_env(&self, env_name: &str) -> bool {
        if self.proof_envs.contains(env_name) {
            return true;
        }
        matches!(
            env_name,
            "proof" | "proof*" | "Proof" | "Proof*" | "pf" | "pf*" | "proofof" | "proofof*"
        )
    }

    pub fn insert(&mut self, def: TheoremDef) {
        self.defs.insert(def.env_name.clone(), def);
    }
}

/// Maps a theorem declaration's display title, environment name, and optional `\theoremstyle`
/// to one of the theorem-like [`SkNodeKind`] variants.
pub fn map_theorem_kind(display_title: &str, env_name: &str, style: Option<&str>) -> SkNodeKind {
    let norm_title = strip_title_formatting(display_title).to_ascii_lowercase();
    if let Some(kind) = match_kind_phrase(&norm_title) {
        return kind;
    }
    let norm_env = env_name
        .trim()
        .trim_end_matches('*')
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_ascii_lowercase();
    if let Some(kind) = match_kind_env(&norm_env) {
        return kind;
    }
    match style.map(str::to_ascii_lowercase).as_deref() {
        Some("definition") => SkNodeKind::Definition,
        Some("remark") => SkNodeKind::Remark,
        _ => SkNodeKind::Theorem,
    }
}

fn strip_title_formatting(raw: &str) -> String {
    let mut s = raw.to_string();
    for mac in [
        "\\textbf{", "\\textit{", "\\textsc{", "\\textrm{", "\\emph{", "\\text{", "\\mathbf{",
    ] {
        s = s.replace(mac, " ");
    }
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '{' | '}' | '\\' | '~') { ' ' } else { c })
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn match_kind_phrase(phrase: &str) -> Option<SkNodeKind> {
    let words: Vec<&str> = phrase
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    if has("corollary") || has("corollaries") {
        return Some(SkNodeKind::Corollary);
    }
    if has("proposition") || has("propositions") {
        return Some(SkNodeKind::Proposition);
    }
    if has("lemma") || has("lemmas") || has("sublemma") {
        return Some(SkNodeKind::Lemma);
    }
    if has("theorem") || has("theorems") || has("metatheorem") || has("fact") || has("result") {
        return Some(SkNodeKind::Theorem);
    }
    if has("conjecture") || has("conjectures") || has("problem") || has("question") {
        return Some(SkNodeKind::Conjecture);
    }
    if has("definition") || has("definitions") || has("notation") || has("terminology")
        || has("convention") || has("conventions")
    {
        return Some(SkNodeKind::Definition);
    }
    if has("assumption") || has("assumptions") || has("hypothesis") || has("hypotheses")
        || has("axiom") || has("axioms") || has("condition") || has("postulate")
    {
        return Some(SkNodeKind::Assumption);
    }
    if has("claim") || has("claims") || has("assertion") || has("observation")
        || has("property") || has("statement")
    {
        return Some(SkNodeKind::Claim);
    }
    if has("example") || has("examples") || has("counterexample") || has("exercise") {
        return Some(SkNodeKind::Example);
    }
    if has("remark") || has("remarks") || has("note") || has("notes") || has("warning")
        || has("caution") || has("comment") || has("scholium")
    {
        return Some(SkNodeKind::Remark);
    }
    if has("algorithm") || has("algorithms") || has("procedure") || has("protocol") {
        return Some(SkNodeKind::Algorithm);
    }
    None
}

fn match_kind_env(env: &str) -> Option<SkNodeKind> {
    match env {
        "theorem" | "thm" | "theo" | "mainthm" | "maintheorem" | "thrm" | "fact" | "res"
        | "result" | "mainresult" => Some(SkNodeKind::Theorem),
        "lemma" | "lem" | "lma" | "lemm" | "sublemma" | "keylem" => Some(SkNodeKind::Lemma),
        "proposition" | "prop" | "prp" => Some(SkNodeKind::Proposition),
        "corollary" | "cor" | "corol" | "coro" => Some(SkNodeKind::Corollary),
        "conjecture" | "conj" | "cj" | "openproblem" => Some(SkNodeKind::Conjecture),
        "definition" | "defn" | "def" | "dfn" | "defin" | "notation" | "convention" => {
            Some(SkNodeKind::Definition)
        }
        "assumption" | "assump" | "ass" | "assm" | "hypothesis" | "hyp" | "axiom" | "ax"
        | "cond" | "condition" => Some(SkNodeKind::Assumption),
        "claim" | "clm" | "obs" | "observation" | "assertion" | "property" => {
            Some(SkNodeKind::Claim)
        }
        "example" | "ex" | "exmp" | "exm" | "expl" | "counterexample" | "exercise" => {
            Some(SkNodeKind::Example)
        }
        "remark" | "rem" | "rmk" | "rk" | "note" | "warning" | "caution" => {
            Some(SkNodeKind::Remark)
        }
        "algorithm" | "alg" | "algo" | "procedure" => Some(SkNodeKind::Algorithm),
        _ => None,
    }
}

fn builtin_theorem_kind(env_name: &str) -> Option<SkNodeKind> {
    let base = env_name.trim_end_matches('*').to_ascii_lowercase();
    match base.as_str() {
        "theorem" | "thm" => Some(SkNodeKind::Theorem),
        "lemma" | "lem" => Some(SkNodeKind::Lemma),
        "proposition" | "prop" => Some(SkNodeKind::Proposition),
        "corollary" | "cor" => Some(SkNodeKind::Corollary),
        "conjecture" | "conj" => Some(SkNodeKind::Conjecture),
        "definition" | "defn" | "def" => Some(SkNodeKind::Definition),
        "assumption" | "assump" | "hypothesis" | "hyp" | "axiom" => Some(SkNodeKind::Assumption),
        "claim" | "observation" => Some(SkNodeKind::Claim),
        "example" | "exmp" => Some(SkNodeKind::Example),
        "remark" | "rem" | "rmk" => Some(SkNodeKind::Remark),
        "algorithm" | "alg" => Some(SkNodeKind::Algorithm),
        _ => None,
    }
}

/// Attempts to parse a theorem/environment/style declaration starting at `cmd_end`.
pub(crate) fn try_parse_theorem_cmd(
    cmd: &str,
    text: &str,
    cmd_end: usize,
    table: &mut TheoremTable,
) -> Option<(usize, Option<String>)> {
    match cmd {
        "theoremstyle" => Some(parse_theoremstyle(text, cmd_end, table)),
        "newtheorem" | "spnewtheorem" | "newmdtheoremenv" => {
            Some(parse_newtheorem(cmd, text, cmd_end, table))
        }
        "declaretheorem" => Some(parse_declaretheorem(text, cmd_end, table)),
        "newenvironment" | "renewenvironment" => {
            Some(parse_newenvironment_wrapper(text, cmd_end, table))
        }
        _ => None,
    }
}

fn parse_theoremstyle(
    text: &str,
    cmd_end: usize,
    table: &mut TheoremTable,
) -> (usize, Option<String>) {
    let pos = skip_ws_and_comments(text, cmd_end);
    if text.as_bytes().get(pos) == Some(&b'{') {
        match read_balanced_group(text, pos, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => {
                let st = content.trim();
                table.current_style = (!st.is_empty()).then(|| st.to_string());
                (next_pos, None)
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                (next_pos, Some("unclosed `{` in \\theoremstyle".to_string()))
            }
        }
    } else {
        (cmd_end, None)
    }
}

fn parse_newtheorem(
    cmd: &str,
    text: &str,
    cmd_end: usize,
    table: &mut TheoremTable,
) -> (usize, Option<String>) {
    let bytes = text.as_bytes();
    let mut pos = cmd_end;
    let starred = bytes.get(pos) == Some(&b'*');
    if starred {
        pos += 1;
    }
    pos = skip_ws_and_comments(text, pos);
    if cmd == "newmdtheoremenv" && bytes.get(pos) == Some(&b'[') {
        match read_balanced_group(text, pos, b'[', b']', false) {
            GroupOutcome::Closed { next_pos, .. } => pos = skip_ws_and_comments(text, next_pos),
            GroupOutcome::Unclosed { next_pos, .. } => {
                return (next_pos, Some(format!("unclosed `[` in \\{cmd}")));
            }
        }
    }
    if bytes.get(pos) != Some(&b'{') {
        return (cmd_end, None);
    }
    let env_name = match read_balanced_group(text, pos, b'{', b'}', false) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = skip_ws_and_comments(text, next_pos);
            content.trim().to_string()
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return (next_pos, Some(format!("unclosed `{{` in \\{cmd}")));
        }
    };
    let mut shared_counter = None;
    if bytes.get(pos) == Some(&b'[') {
        match read_balanced_group(text, pos, b'[', b']', false) {
            GroupOutcome::Closed { content, next_pos } => {
                let s = content.trim();
                if !s.is_empty() {
                    shared_counter = Some(s.to_string());
                }
                pos = skip_ws_and_comments(text, next_pos);
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                return (next_pos, Some(format!("unclosed `[` in \\{cmd}{{{env_name}}}")));
            }
        }
    }
    if bytes.get(pos) != Some(&b'{') {
        return (pos, None);
    }
    let display_title = match read_balanced_group(text, pos, b'{', b'}', false) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = skip_ws_and_comments(text, next_pos);
            clean_tex_title(&content).unwrap_or_default()
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return (next_pos, Some(format!("unclosed `{{` title in \\{cmd}{{{env_name}}}")));
        }
    };
    let mut parent_counter = None;
    if bytes.get(pos) == Some(&b'[') {
        match read_balanced_group(text, pos, b'[', b']', false) {
            GroupOutcome::Closed { content, next_pos } => {
                let s = content.trim();
                if !s.is_empty() {
                    parent_counter = Some(s.to_string());
                }
                pos = next_pos;
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                return (next_pos, Some(format!("unclosed `[` parent counter in \\{cmd}{{{env_name}}}")));
            }
        }
    }
    if !env_name.is_empty() {
        let kind = map_theorem_kind(&display_title, &env_name, table.current_style.as_deref());
        table.insert(TheoremDef {
            env_name,
            kind,
            display_title,
            shared_counter,
            parent_counter,
            starred,
            style: table.current_style.clone(),
        });
    }
    (pos, None)
}

fn parse_declaretheorem(
    text: &str,
    cmd_end: usize,
    table: &mut TheoremTable,
) -> (usize, Option<String>) {
    let bytes = text.as_bytes();
    let mut pos = cmd_end;
    let mut starred = bytes.get(pos) == Some(&b'*');
    if starred {
        pos += 1;
    }
    pos = skip_ws_and_comments(text, pos);
    let mut opts_raw = String::new();
    if bytes.get(pos) == Some(&b'[') {
        match read_balanced_group(text, pos, b'[', b']', false) {
            GroupOutcome::Closed { content, next_pos } => {
                opts_raw = content;
                pos = skip_ws_and_comments(text, next_pos);
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                return (next_pos, Some("unclosed `[` in \\declaretheorem".to_string()));
            }
        }
    }
    if bytes.get(pos) != Some(&b'{') {
        return (pos, None);
    }
    let names_raw = match read_balanced_group(text, pos, b'{', b'}', false) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = next_pos;
            content
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return (next_pos, Some("unclosed `{` in \\declaretheorem".to_string()));
        }
    };

    let kv = parse_kv_options(&opts_raw);
    let opt_title = kv.get("name").or_else(|| kv.get("title")).cloned();
    let shared_counter = kv
        .get("sibling")
        .or_else(|| kv.get("sharenumber"))
        .or_else(|| kv.get("usecounter"))
        .cloned();
    let parent_counter = kv.get("numberwithin").or_else(|| kv.get("parent")).cloned();
    let opt_style = kv.get("style").cloned().or_else(|| table.current_style.clone());
    if kv.get("numbered").is_some_and(|v| v.eq_ignore_ascii_case("no")) {
        starred = true;
    }

    for part in names_raw.split(',') {
        let env_name = part.trim();
        if env_name.is_empty() {
            continue;
        }
        let display_title = opt_title.clone().unwrap_or_else(|| capitalize_first(env_name));
        let kind = map_theorem_kind(&display_title, env_name, opt_style.as_deref());
        table.insert(TheoremDef {
            env_name: env_name.to_string(),
            kind,
            display_title,
            shared_counter: shared_counter.clone(),
            parent_counter: parent_counter.clone(),
            starred,
            style: opt_style.clone(),
        });
    }
    (pos, None)
}

fn parse_newenvironment_wrapper(
    text: &str,
    cmd_end: usize,
    table: &mut TheoremTable,
) -> (usize, Option<String>) {
    let bytes = text.as_bytes();
    let mut pos = cmd_end;
    if bytes.get(pos) == Some(&b'*') {
        pos += 1;
    }
    pos = skip_ws_and_comments(text, pos);
    if bytes.get(pos) != Some(&b'{') {
        return (cmd_end, None);
    }
    let env_name = match read_balanced_group(text, pos, b'{', b'}', false) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = skip_ws_and_comments(text, next_pos);
            content.trim().to_string()
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return (next_pos, Some("unclosed `{` in \\newenvironment".to_string()));
        }
    };
    for _ in 0..2 {
        if bytes.get(pos) == Some(&b'[') {
            match read_balanced_group(text, pos, b'[', b']', false) {
                GroupOutcome::Closed { next_pos, .. } => pos = skip_ws_and_comments(text, next_pos),
                GroupOutcome::Unclosed { next_pos, .. } => {
                    return (next_pos, Some(format!("unclosed `[` in \\newenvironment{{{env_name}}}")));
                }
            }
        }
    }
    if bytes.get(pos) != Some(&b'{') {
        return (pos, None);
    }
    let begin_body = match read_balanced_group(text, pos, b'{', b'}', true) {
        GroupOutcome::Closed { content, next_pos } => {
            pos = skip_ws_and_comments(text, next_pos);
            content
        }
        GroupOutcome::Unclosed { next_pos, .. } => {
            return (next_pos, Some(format!("unclosed begin body in \\newenvironment{{{env_name}}}")));
        }
    };
    if bytes.get(pos) == Some(&b'{') {
        match read_balanced_group(text, pos, b'{', b'}', true) {
            GroupOutcome::Closed { next_pos, .. } => pos = next_pos,
            GroupOutcome::Unclosed { next_pos, .. } => {
                return (next_pos, Some(format!("unclosed end body in \\newenvironment{{{env_name}}}")));
            }
        }
    }
    if !env_name.is_empty() {
        if let Some(inner) = find_wrapped_begin_env(&begin_body) {
            if table.is_proof_env(&inner) {
                table.proof_envs.insert(env_name);
            } else if let Some(existing) = table.defs.get(&inner).cloned() {
                table.insert(TheoremDef { env_name, ..existing });
            } else if let Some(kind) = builtin_theorem_kind(&inner) {
                table.insert(TheoremDef {
                    display_title: capitalize_first(&env_name),
                    env_name,
                    kind,
                    shared_counter: Some(inner),
                    parent_counter: None,
                    starred: false,
                    style: table.current_style.clone(),
                });
            }
        }
    }
    (pos, None)
}

fn find_wrapped_begin_env(body: &str) -> Option<String> {
    let idx = body.find("\\begin{")?;
    let rest = &body[idx + "\\begin{".len()..];
    let end = rest.find('}')?;
    let name = rest[..end].trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn parse_kv_options(opts: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut cur = String::new();
    let mut brace_depth = 0usize;
    let mut parts = Vec::new();
    for ch in opts.chars() {
        match ch {
            '{' => {
                brace_depth += 1;
                cur.push(ch);
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if brace_depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    for part in parts {
        if let Some((k, v)) = part.split_once('=') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            let unbraced = val.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(val).trim();
            if !key.is_empty() && !unbraced.is_empty() {
                map.insert(key, unbraced.to_string());
            }
        }
    }
    map
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
