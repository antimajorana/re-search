//! Sheet format of the evaluation set (overview section 7.1): date, build,
//! models, model calls, wall time, and the measures of 7.3. A sheet is
//! data (JSON) and is rendered to Markdown for
//! `docs/design/eval-2026-11/`. Anything that was not supplied is printed
//! as "not measured" or "not recorded". Pure module (serde and std only).

use super::manifest::Tier;
use super::measures::{evaluate, GateReport, Outcome, Rule, MEASURES};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SHEET_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// Full or abbreviated commit hash of the build that was run.
    pub git_commit: Option<String>,
    /// Workspace crate version of that build.
    pub crate_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub format_version: u32,
    /// Day of the run, `YYYY-MM-DD`.
    pub date: String,
    pub tier: Tier,
    /// The milestone this run is the gate for, such as "M1"; `None` for a run of its own.
    pub milestone: Option<String>,
    pub build: Build,
    /// Models used, as the provider names them. Empty for a run with no model.
    pub models: Vec<String>,
    /// One request to the provider is one model call.
    pub model_calls: Option<u64>,
    pub wall_time_s: Option<f64>,
    /// Ids of the manifest cases that were run.
    pub cases_run: Vec<String>,
    /// Measured values by measure id (see `measures::MEASURES`).
    pub values: BTreeMap<String, f64>,
    /// Free text per measure id: reasons, the cap in force, how a number was aggregated.
    pub measure_notes: BTreeMap<String, String>,
    pub remarks: String,
}

impl Sheet {
    /// A sheet with nothing measured. Rendering it gives the template.
    pub fn blank(tier: Tier) -> Self {
        Sheet {
            format_version: SHEET_FORMAT_VERSION,
            date: String::new(),
            tier,
            milestone: None,
            build: Build::default(),
            models: Vec::new(),
            model_calls: None,
            wall_time_s: None,
            cases_run: Vec::new(),
            values: BTreeMap::new(),
            measure_notes: BTreeMap::new(),
            remarks: String::new(),
        }
    }

    pub fn set_value(&mut self, id: &str, value: f64) {
        self.values.insert(id.to_string(), value);
    }

    /// Records the value when there is one; `None` leaves the measure not measured.
    pub fn set_value_opt(&mut self, id: &str, value: Option<f64>) {
        if let Some(v) = value {
            self.set_value(id, v);
        }
    }

    pub fn gate(&self) -> Result<GateReport, String> {
        evaluate(&self.values)
    }
}

pub fn parse_sheet(text: &str) -> Result<Sheet, String> {
    serde_json::from_str(text).map_err(|e| format!("sheet does not parse: {e}"))
}

/// What a sheet must have before it is filed. Measures may be missing
/// (they print as "not measured"); the identity of the run may not.
pub fn validate_sheet(sheet: &Sheet) -> Vec<String> {
    let mut errs = Vec::new();
    if sheet.format_version != SHEET_FORMAT_VERSION {
        errs.push(format!("format_version {} is not {SHEET_FORMAT_VERSION}", sheet.format_version));
    }
    if !is_iso_date(&sheet.date) {
        errs.push(format!("date `{}` is not YYYY-MM-DD", sheet.date));
    }
    match sheet.build.git_commit.as_deref() {
        Some(c) if (7..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit()) => {}
        _ => errs.push("build.git_commit must be the commit hash of the build (7 to 40 hex digits)".into()),
    }
    if sheet.build.crate_version.as_deref().map_or(true, |v| v.trim().is_empty()) {
        errs.push("build.crate_version is missing".into());
    }
    if sheet.wall_time_s.is_some_and(|t| !t.is_finite() || t < 0.0) {
        errs.push("wall_time_s must be a finite number, 0 or above".into());
    }
    if sheet.model_calls.is_some_and(|n| n > 0) && sheet.models.is_empty() {
        errs.push("model calls were made but no model is named".into());
    }
    for id in sheet.measure_notes.keys() {
        if super::measures::measure(id).is_none() {
            errs.push(format!("note for `{id}`, which is not a measure of section 7.3"));
        }
    }
    if let Err(e) = sheet.gate() {
        errs.push(e);
    }
    errs
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
    match (num(0..4), num(5..7), num(8..10)) {
        (Some(y), Some(m), Some(d)) => y >= 2026 && (1..=12).contains(&m) && (1..=31).contains(&d),
        _ => false,
    }
}

/// Today's date in UTC as `YYYY-MM-DD` (civil-from-days; no dependency).
pub fn today_utc() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The build under test. The commit comes from `EVAL_GIT_COMMIT` if set,
/// otherwise from `git rev-parse HEAD` in `repo_root`; if neither works it
/// stays `None` and the sheet says "not recorded".
pub fn current_build(repo_root: &Path, crate_version: &str) -> Build {
    let from_env = std::env::var("EVAL_GIT_COMMIT").ok().filter(|s| !s.trim().is_empty());
    let git_commit = from_env.or_else(|| {
        let out = std::process::Command::new("git").arg("rev-parse").arg("HEAD").current_dir(repo_root).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
    });
    Build { git_commit, crate_version: Some(crate_version.to_string()) }
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn or_not_recorded(text: Option<&str>) -> String {
    match text {
        Some(t) if !t.trim().is_empty() => cell(t),
        _ => "not recorded".into(),
    }
}

fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Renders a sheet to Markdown. Fails only if the sheet carries a value
/// the gate does not know (see `measures::evaluate`).
pub fn render_markdown(sheet: &Sheet) -> Result<String, String> {
    let report = sheet.gate()?;
    let mut out = String::new();
    let date = or_not_recorded(Some(&sheet.date));
    out.push_str(&format!("# Evaluation sheet: {date}, {} tier\n\n", sheet.tier.name()));
    out.push_str("Format: one sheet per run (overview section 7.1). Measures and targets: section 7.3.\n\n");
    out.push_str("## Run\n\n| Field | Value |\n|---|---|\n");
    out.push_str(&format!("| Date | {date} |\n"));
    out.push_str(&format!("| Tier | {} |\n", sheet.tier.name()));
    out.push_str(&format!("| Gate for milestone | {} |\n", sheet.milestone.as_deref().map(cell).unwrap_or_else(|| "none".into())));
    out.push_str(&format!("| Build: git commit | {} |\n", or_not_recorded(sheet.build.git_commit.as_deref())));
    out.push_str(&format!("| Build: crate version | {} |\n", or_not_recorded(sheet.build.crate_version.as_deref())));
    let models = if sheet.models.is_empty() { "none".to_string() } else { cell(&sheet.models.join(", ")) };
    out.push_str(&format!("| Models | {models} |\n"));
    out.push_str(&format!("| Model calls | {} |\n", sheet.model_calls.map(|n| n.to_string()).unwrap_or_else(|| "not measured".into())));
    out.push_str(&format!("| Wall time | {} |\n", sheet.wall_time_s.map(|t| format!("{} s", number(t))).unwrap_or_else(|| "not measured".into())));
    let cases = if sheet.cases_run.is_empty() { "none".to_string() } else { cell(&sheet.cases_run.join(", ")) };
    out.push_str(&format!("| Cases run ({}) | {cases} |\n\n", sheet.cases_run.len()));

    out.push_str(&format!("## Gate: {}\n\n", report.verdict.label()));
    let failed = report.ids_with(Outcome::Fail);
    let open = report.unmeasured_rules();
    out.push_str("PASS needs every measure that has a pass rule to be measured and to pass. ");
    out.push_str("A measure without a value counts as not measured, never as a pass.\n\n");
    out.push_str(&format!("- Failed ({}): {}\n", failed.len(), list(&failed)));
    out.push_str(&format!("- Pass rule, not measured ({}): {}\n\n", open.len(), list(&open)));

    out.push_str("## Measures\n\n| Measure | Id | Applies to | Rule | Value | Outcome | Says in 7.3 | Note |\n|---|---|---|---|---|---|---|---|\n");
    for r in &report.results {
        let value = match r.value {
            Some(v) => format!("{} {}", number(v), r.def.unit),
            None => "not measured".into(),
        };
        let note = sheet.measure_notes.get(r.def.id).map(|n| cell(n)).unwrap_or_default();
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} | {} |\n",
            cell(r.def.label),
            r.def.id,
            r.def.spec_applies_to,
            r.def.rule.describe(),
            value,
            r.outcome.label(),
            cell(r.def.consequence),
            note
        ));
    }
    let recorded_rules = MEASURES.iter().filter(|d| d.rule == Rule::Recorded).count();
    out.push_str(&format!(
        "\n{} measures: {} with a pass rule, {} recorded only.\n\n## Remarks\n\n{}\n",
        MEASURES.len(),
        MEASURES.len() - recorded_rules,
        recorded_rules,
        if sheet.remarks.trim().is_empty() { "None." } else { sheet.remarks.trim() }
    ));
    Ok(out)
}

fn list(ids: &[&str]) -> String {
    if ids.is_empty() {
        "none".into()
    } else {
        ids.iter().map(|i| format!("`{i}`")).collect::<Vec<_>>().join(", ")
    }
}

/// File name of a sheet: `sheet-<date>-<tier>-<commit, 7 digits>.md`.
pub fn sheet_file_name(sheet: &Sheet) -> String {
    let commit = sheet.build.git_commit.as_deref().unwrap_or("nocommit");
    format!("sheet-{}-{}-{}.md", sheet.date, sheet.tier.name(), &commit[..commit.len().min(7)])
}

/// Validates, renders and writes one sheet into `dir`. A sheet is never
/// overwritten: a second run of the same day and build gets `-2`, `-3`, ...
pub fn write_sheet(dir: &Path, sheet: &Sheet) -> Result<PathBuf, String> {
    let errs = validate_sheet(sheet);
    if !errs.is_empty() {
        return Err(format!("sheet is not ready to file: {}", errs.join("; ")));
    }
    let text = render_markdown(sheet)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let name = sheet_file_name(sheet);
    let stem = name.trim_end_matches(".md");
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem}-{n}.md"));
        n += 1;
    }
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}
