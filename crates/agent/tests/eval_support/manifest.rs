//! Manifest format of the evaluation set (overview section 7.1 and 7.2).
//!
//! One format for all modes. A manifest file is an envelope
//! `{format_version, set, tier, cases}`; each case is exactly
//! `{id, mode, input, answer_key, notes}`. This module is pure (serde and
//! std only) so another crate's tests can include it with `#[path]`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Longest locator accepted ("Theorem 2.1", "eq. (3.4)"). A locator points
/// into a paper; it must not be able to carry the paper's text.
pub const MAX_LOCATOR_CHARS: usize = 80;

/// Longest user correction lesson stored by F16b (`00-overview.md` 2.14).
pub const MAX_LESSON_CHARS: usize = 300;

/// The case sets of section 7.2 (`P2` and `P4` are deferred per 0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Set {
    P,
    P2,
    P3,
    P4,
    P5,
    P6,
    B,
    I,
    Q,
    W,
    K,
    C,
    R,
}

impl Set {
    pub const ALL: [Set; 13] = [
        Set::P, Set::P2, Set::P3, Set::P4, Set::P5, Set::P6,
        Set::B, Set::I, Set::Q, Set::W, Set::K, Set::C, Set::R,
    ];

    pub fn letter(self) -> &'static str {
        match self {
            Set::P => "P",
            Set::P2 => "P2",
            Set::P3 => "P3",
            Set::P4 => "P4",
            Set::P5 => "P5",
            Set::P6 => "P6",
            Set::B => "B",
            Set::I => "I",
            Set::Q => "Q",
            Set::W => "W",
            Set::K => "K",
            Set::C => "C",
            Set::R => "R",
        }
    }

    /// The one mode a case of this set runs in.
    pub fn mode(self) -> Mode {
        match self {
            Set::P | Set::P2 | Set::P3 | Set::P4 | Set::P5 | Set::P6 => Mode::Investigate,
            Set::B => Mode::Library,
            Set::I => Mode::Explore,
            Set::Q => Mode::Solve,
            Set::W => Mode::Writeup,
            Set::K => Mode::Corrections,
            Set::C => Mode::Handover,
            Set::R => Mode::Resume,
        }
    }

    /// Top-level members an answer key of this set must have. They restate
    /// the last column of the table in 7.2; anything further is the key
    /// writer's business and is not checked here.
    pub fn required_key_fields(self) -> &'static [&'static str] {
        match self {
            Set::P => &["statements", "sections", "uses_edges"],
            Set::P2 => &["statements"],
            Set::P3 => &["results"],
            Set::P4 => &["earlier_source"],
            Set::P5 => &["statement"],
            Set::P6 => &["statements", "sections", "sampled_sentences"],
            Set::B => &["items", "planted_cases"],
            Set::I => &["parts"],
            Set::Q => &["final_answer", "steps", "planted_step"],
            Set::W => &["compiles", "check_table"],
            Set::K => &["expected_effect"],
            Set::C => &["target_answer"],
            Set::R => &["must_resume", "same_content"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Investigate,
    Library,
    Explore,
    Solve,
    Writeup,
    Corrections,
    Handover,
    Resume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySourceKind {
    KandinskyVault,
    Folder,
    Bibtex,
    Zotero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Fixed,
    Live,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Fixed => "fixed",
            Tier::Live => "live",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format_version: u32,
    pub set: Set,
    pub tier: Tier,
    pub cases: Vec<Case>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub mode: Mode,
    pub input: Input,
    pub answer_key: Value,
    pub notes: String,
}

/// Solve's task names (the hand-off uses the same three).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveTask {
    Calculate,
    CheckDerivation,
    Prove,
}

/// The four old-session ledgers of set R.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerVariant {
    Research,
    InvestigationBeforePointerCitations,
    InvestigationCurrent,
    HandoffChild,
}

/// What a case is started from. No variant can carry a paper's text: a
/// paper is named by identifier, or (fixed tier) by the path of a
/// self-written file or library tree under `tests/fixtures/eval/`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Arxiv {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        versions: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<String>,
    },
    Doi {
        doi: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<String>,
    },
    Fixture {
        files: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locator: Option<String>,
    },
    LibraryFixture {
        source_kind: LibrarySourceKind,
        path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        auxiliary_paths: Vec<String>,
    },
    Library {
        source_kind: LibrarySourceKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Idea {
        text: String,
    },
    Problem {
        task: SolveTask,
        statement: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        steps: Option<Vec<String>>,
    },
    Writeup {
        case_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ledger: Option<String>,
    },
    Correction {
        scenario: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_case: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lesson: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scripts: Vec<String>,
    },
    Chain {
        from: String,
        to: String,
    },
    Ledger {
        path: String,
        variant: LedgerVariant,
    },
}

impl Input {
    /// Regular fixture file paths this input names, relative to `tests/fixtures/eval/`.
    pub fn fixture_paths(&self) -> Vec<&str> {
        match self {
            Input::Fixture { files, .. } => files.iter().map(String::as_str).collect(),
            Input::LibraryFixture {
                source_kind: LibrarySourceKind::Bibtex | LibrarySourceKind::Zotero,
                path,
                ..
            } => vec![path.as_str()],
            Input::Writeup { ledger: Some(path), .. } => vec![path.as_str()],
            Input::Correction { scripts, .. } => scripts.iter().map(String::as_str).collect(),
            Input::Ledger { path, .. } => vec![path.as_str()],
            _ => Vec::new(),
        }
    }

    /// Directory fixture paths this input names, relative to `tests/fixtures/eval/`.
    pub fn fixture_dir_paths(&self) -> Vec<&str> {
        match self {
            Input::LibraryFixture {
                source_kind: LibrarySourceKind::KandinskyVault | LibrarySourceKind::Folder,
                path,
                ..
            } => vec![path.as_str()],
            _ => Vec::new(),
        }
    }

    /// Auxiliary file or directory paths this input names, relative to `tests/fixtures/eval/`.
    pub fn fixture_auxiliary_paths(&self) -> Vec<&str> {
        match self {
            Input::LibraryFixture { auxiliary_paths, .. } => auxiliary_paths.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }
}

/// Parses one manifest file. Unknown fields anywhere in the envelope, a
/// case or an input are rejected by serde.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    serde_json::from_str(text).map_err(|e| format!("manifest does not parse: {e}"))
}

/// Checks one manifest on its own. Returns every problem found, so a
/// reviewer fixing a key sees all of them at once.
pub fn validate_manifest(m: &Manifest) -> Vec<String> {
    let mut errs = Vec::new();
    if m.format_version != MANIFEST_FORMAT_VERSION {
        errs.push(format!("format_version {} is not {MANIFEST_FORMAT_VERSION}", m.format_version));
    }
    if m.set == Set::R && m.tier == Tier::Live {
        errs.push("set R is fixed tier only (old ledgers are fixtures and run in CI)".into());
    }
    let mut seen = BTreeSet::new();
    for case in &m.cases {
        if !seen.insert(case.id.as_str()) {
            errs.push(format!("{}: id appears twice in this manifest", case.id));
        }
        for e in validate_case(m.set, m.tier, case) {
            errs.push(format!("{}: {e}", case.id));
        }
    }
    errs
}

/// Checks several manifests together: each one alone, ids unique across
/// all, at most one manifest per (set, tier), and cross-set references in
/// sets C, W, and K pointing at cases that exist in the right sets.
pub fn validate_manifests(manifests: &[Manifest]) -> Vec<String> {
    let mut errs = Vec::new();
    let mut ids: BTreeMap<&str, Set> = BTreeMap::new();
    let mut files = BTreeSet::new();
    for m in manifests {
        errs.extend(validate_manifest(m));
        if !files.insert((m.set, m.tier.name())) {
            errs.push(format!("more than one {} manifest for set {}", m.tier.name(), m.set.letter()));
        }
        for case in &m.cases {
            if let Some(other) = ids.insert(case.id.as_str(), m.set) {
                if other != m.set {
                    errs.push(format!("{}: id is used in set {} and set {}", case.id, other.letter(), m.set.letter()));
                }
            }
        }
    }
    for m in manifests {
        for case in &m.cases {
            match (m.set, &case.input) {
                (Set::C, Input::Chain { from, to }) => match (ids.get(from.as_str()), ids.get(to.as_str())) {
                    (Some(Set::P | Set::P6), Some(Set::Q | Set::I)) | (Some(Set::I), Some(Set::Q)) => {}
                    (Some(a), Some(b)) => errs.push(format!(
                        "{}: a chain from set {} to set {} is not one of P to Q, P to I, I to Q",
                        case.id, a.letter(), b.letter()
                    )),
                    (None, _) => errs.push(format!("{}: chain start `{from}` is not a known case", case.id)),
                    (_, None) => errs.push(format!("{}: chain target `{to}` is not a known case", case.id)),
                },
                (Set::W, Input::Writeup { case_id, ledger }) => {
                    check_source_ref(&case.id, "writeup source", case_id, ledger.is_none(), &ids, &mut errs);
                }
                (Set::K, Input::Correction { target_case: Some(tc), scripts, .. }) => {
                    check_source_ref(&case.id, "correction target", tc, scripts.is_empty(), &ids, &mut errs);
                }
                _ => {}
            }
        }
    }
    errs
}

fn check_source_ref(
    case_id: &str,
    label: &str,
    target: &str,
    require_known: bool,
    ids: &BTreeMap<&str, Set>,
    errs: &mut Vec<String>,
) {
    match ids.get(target) {
        Some(Set::P | Set::P6 | Set::I | Set::Q) => {}
        Some(other) => errs.push(format!(
            "{case_id}: {label} `{target}` in set {} is not one of P, P6, I, Q",
            other.letter()
        )),
        None if require_known => errs.push(format!("{case_id}: {label} `{target}` is not a known case")),
        None => {}
    }
}

fn validate_case(set: Set, tier: Tier, case: &Case) -> Vec<String> {
    let mut errs = Vec::new();
    let prefix = format!("{}-", set.letter());
    let tail = case.id.strip_prefix(&prefix).unwrap_or("");
    if tail.is_empty() || !tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        errs.push(format!("id must be `{prefix}` followed by letters, digits, `-`, `_` or `.`"));
    }
    if case.mode != set.mode() {
        errs.push(format!("mode {:?} does not match set {} (expected {:?})", case.mode, set.letter(), set.mode()));
    }
    match case.answer_key.as_object() {
        None => errs.push("answer_key must be a JSON object".into()),
        Some(obj) => {
            for field in set.required_key_fields() {
                if !obj.contains_key(*field) {
                    errs.push(format!("answer_key lacks `{field}`"));
                }
            }
        }
    }
    errs.extend(validate_input(set, tier, &case.input));
    errs
}

fn input_allowed_for_set(set: Set, input: &Input) -> bool {
    match set {
        Set::P | Set::P2 | Set::P3 | Set::P4 | Set::P5 | Set::P6 => {
            matches!(input, Input::Arxiv { .. } | Input::Doi { .. } | Input::Fixture { .. })
        }
        Set::B => matches!(input, Input::LibraryFixture { .. } | Input::Library { .. }),
        Set::I => matches!(input, Input::Idea { .. }),
        Set::Q => matches!(input, Input::Problem { .. }),
        Set::W => matches!(input, Input::Writeup { .. }),
        Set::K => matches!(input, Input::Correction { .. }),
        Set::C => matches!(input, Input::Chain { .. }),
        Set::R => matches!(input, Input::Ledger { .. }),
    }
}

fn validate_input(set: Set, tier: Tier, input: &Input) -> Vec<String> {
    let mut errs = Vec::new();
    if !input_allowed_for_set(set, input) {
        errs.push(format!("this kind of input is not allowed in set {}", set.letter()));
        return errs;
    }
    match input {
        Input::Arxiv { id, versions, locator } => {
            if !is_arxiv_id(id) {
                errs.push(format!("`{id}` is not an arXiv identifier"));
            }
            if set == Set::P2 {
                match versions {
                    Some(v) if v.len() == 2 && v[0] != v[1] && v.iter().all(|s| is_version_tag(s)) => {
                        if has_version_suffix(id) {
                            errs.push("give the id without a version when `versions` is set".into());
                        }
                    }
                    _ => errs.push("set P2 needs `versions` with two different tags such as [\"v1\", \"v2\"]".into()),
                }
            } else if versions.is_some() {
                errs.push("`versions` belongs to set P2 only".into());
            }
            check_locator(locator, &mut errs);
        }
        Input::Doi { doi, locator } => {
            if !is_doi(doi) {
                errs.push(format!("`{doi}` is not a DOI"));
            }
            if set == Set::P2 {
                errs.push("set P2 compares two arXiv versions; a DOI cannot name them".into());
            }
            check_locator(locator, &mut errs);
        }
        Input::Fixture { files, locator } => {
            if tier == Tier::Live {
                errs.push("a live-tier paper is named by arXiv id or DOI only, never by a stored file".into());
            }
            let want = if set == Set::P2 { 2 } else { 1 };
            if files.len() != want {
                errs.push(format!("set {} needs exactly {want} fixture file(s)", set.letter()));
            }
            files.iter().for_each(|f| check_fixture_path(f, &mut errs));
            check_locator(locator, &mut errs);
        }
        Input::LibraryFixture { path, auxiliary_paths, .. } => {
            if tier == Tier::Live {
                errs.push("a live-tier library case cannot use a stored fixture".into());
            }
            check_fixture_path(path, &mut errs);
            auxiliary_paths.iter().for_each(|p| check_fixture_path(p, &mut errs));
        }
        Input::Library { label, .. } => {
            if let Some(l) = label { check_text("library label", l, &mut errs); }
        }
        Input::Idea { text } => check_text("idea text", text, &mut errs),
        Input::Problem { task, statement, steps } => {
            check_text("problem statement", statement, &mut errs);
            let has_steps = steps.as_ref().is_some_and(|s| !s.is_empty());
            if *task == SolveTask::CheckDerivation && !has_steps {
                errs.push("a check_derivation problem needs its `steps`".into());
            }
            if steps.as_ref().is_some_and(|s| s.iter().any(|x| x.trim().is_empty())) {
                errs.push("a step must not be empty".into());
            }
        }
        Input::Writeup { case_id, ledger } => {
            check_text("writeup case_id", case_id, &mut errs);
            if let Some(path) = ledger {
                if tier == Tier::Live { errs.push("a live-tier writeup cannot reference a stored ledger file".into()); }
                check_fixture_path(path, &mut errs);
            }
        }
        Input::Correction { scenario, target_case, lesson, scripts } => {
            check_text("correction scenario", scenario, &mut errs);
            if let Some(tc) = target_case { check_text("correction target_case", tc, &mut errs); }
            if let Some(text) = lesson {
                check_text("correction lesson", text, &mut errs);
                if text.chars().count() > MAX_LESSON_CHARS {
                    errs.push(format!("correction lesson must be at most {MAX_LESSON_CHARS} characters"));
                }
            }
            if !scripts.is_empty() {
                if tier == Tier::Live { errs.push("a live-tier correction case cannot reference stored script files".into()); }
                scripts.iter().for_each(|s| check_fixture_path(s, &mut errs));
            }
        }
        Input::Chain { from, to } => {
            if from == to { errs.push("a chain must join two different cases".into()); }
        }
        Input::Ledger { path, .. } => check_fixture_path(path, &mut errs),
    }
    errs
}

fn check_text(what: &str, text: &str, errs: &mut Vec<String>) {
    if text.trim().is_empty() { errs.push(format!("{what} must not be empty")); }
}

fn check_locator(locator: &Option<String>, errs: &mut Vec<String>) {
    if let Some(l) = locator {
        if l.trim().is_empty() || l.chars().count() > MAX_LOCATOR_CHARS || l.contains('\n') {
            errs.push(format!("locator must be one non-empty line of at most {MAX_LOCATOR_CHARS} characters"));
        }
    }
}

/// A fixture path is relative to `tests/fixtures/eval/`, written with `/`,
/// and cannot leave that directory on any platform.
fn check_fixture_path(path: &str, errs: &mut Vec<String>) {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        errs.push(format!("fixture path `{path}` must be relative, use `/`, and stay inside the fixtures directory"));
    }
}

fn is_version_tag(s: &str) -> bool {
    s.strip_prefix('v').is_some_and(|n| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()))
}

fn split_version(id: &str) -> (&str, Option<&str>) {
    match id.rfind('v') {
        Some(pos) if is_version_tag(&id[pos..]) => (&id[..pos], Some(&id[pos..])),
        _ => (id, None),
    }
}

fn has_version_suffix(id: &str) -> bool {
    split_version(id).1.is_some()
}

/// Shape check only (no network): `2401.12345`, `2401.12345v2`,
/// `hep-th/9905111`, `math.AG/0601001v1`. A URL or a prefixed form is
/// rejected so that one paper has one spelling in a manifest.
pub fn is_arxiv_id(id: &str) -> bool {
    let (base, _) = split_version(id);
    let digits = |s: &str, lens: &[usize]| lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if let Some((yymm, num)) = base.split_once('.') {
        if digits(yymm, &[4]) && digits(num, &[4, 5]) { return true; }
    }
    if let Some((archive, num)) = base.split_once('/') {
        let archive_ok = !archive.is_empty()
            && archive.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'.')
            && archive.as_bytes()[0].is_ascii_lowercase();
        return archive_ok && digits(num, &[7]);
    }
    false
}

/// Shape check only: `10.<registrant>/<suffix>` with no whitespace.
pub fn is_doi(doi: &str) -> bool {
    let Some(rest) = doi.strip_prefix("10.") else { return false };
    let Some((registrant, suffix)) = rest.split_once('/') else { return false };
    registrant.len() >= 4
        && registrant.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && !suffix.is_empty()
        && !doi.chars().any(char::is_whitespace)
}
