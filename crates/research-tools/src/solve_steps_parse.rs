//! Input parsing and schema validation for `step_record` (`solve.md` 5.1, rule R5).

use crate::latex_repair::repair_under_escaped_latex;
use crate::quote_core::QuoteRef;
use crate::solve_defs::STEP_RECORD;
use crate::solve_model::{
    validate_step_id, Claim, ClaimForm, ClaimRelation, HandleRef, PlanItem, SeriesOrder,
    StepEntryType,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_SUMMARY_CHARS: usize = 500;
const MAX_DETAILS_CHARS: usize = 16_000;
const MAX_DOMAIN_HOW_CHARS: usize = 500;
const MAX_PLAN_TITLE_CHARS: usize = 200;
const ALLOWED_TOP_KEYS: &[&str] = &[
    "entry_type",
    "summary",
    "details",
    "depends_on",
    "tool_call_refs",
    "claims",
    "source_ref",
    "plan",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaimInput {
    form: ClaimForm,
    lhs: String,
    rhs: String,
    relation: ClaimRelation,
    #[serde(default)]
    order: Option<SeriesOrder>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    how: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQuoteRefInput {
    source_id: String,
    path: String,
    start_line: usize,
    end_line: usize,
    file_sha256: String,
    quote_sha256: String,
    evidence_seq: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlanItemInput {
    title: String,
}

pub(super) struct ParsedStepInput {
    pub entry_type: StepEntryType,
    pub summary: String,
    pub details: String,
    pub depends_on: Vec<String>,
    pub tool_call_refs: Vec<String>,
    pub claims: Vec<Claim>,
    pub source_ref: Option<QuoteRef>,
    pub plan: Option<Vec<PlanItem>>,
}

pub(super) fn parse_step_input(args: &Value) -> Result<ParsedStepInput, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| format!("{STEP_RECORD} arguments must be a JSON object"))?;
    if obj.contains_key("latex") {
        return Err(
            "step_record never accepts `latex`; formulas are rendered from handles (rule R5)"
                .to_string(),
        );
    }
    for key in obj.keys() {
        if !ALLOWED_TOP_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown field `{key}` in {STEP_RECORD}"));
        }
    }

    let entry_type: StepEntryType = serde_json::from_value(
        obj.get("entry_type")
            .ok_or("`entry_type` is required")?
            .clone(),
    )
    .map_err(|e| format!("invalid `entry_type`: {e}"))?;

    let raw_summary = obj
        .get("summary")
        .and_then(Value::as_str)
        .ok_or("`summary` must be a string")?;
    let summary = repair_under_escaped_latex(raw_summary).trim().to_string();
    if summary.is_empty() {
        return Err("`summary` must not be empty".to_string());
    }
    if summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(format!("`summary` exceeds {MAX_SUMMARY_CHARS} characters"));
    }

    let raw_details = obj
        .get("details")
        .and_then(Value::as_str)
        .ok_or("`details` must be a string")?;
    let details = repair_under_escaped_latex(raw_details);
    if details.chars().count() > MAX_DETAILS_CHARS {
        return Err(format!("`details` exceeds {MAX_DETAILS_CHARS} characters"));
    }

    let depends_on = parse_depends_on(obj.get("depends_on"))?;
    let tool_call_refs = parse_tool_call_refs(obj.get("tool_call_refs"))?;
    let claims = parse_claims(obj.get("claims"))?;
    let source_ref = parse_source_ref(obj.get("source_ref"))?;
    let plan = parse_plan(obj.get("plan"))?;

    Ok(ParsedStepInput {
        entry_type,
        summary,
        details,
        depends_on,
        tool_call_refs,
        claims,
        source_ref,
        plan,
    })
}

fn parse_depends_on(val: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`depends_on` must be an array")?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or("`depends_on` entries must be strings")?
            .trim()
            .to_string();
        if s.chars().count() > 64 {
            return Err("`depends_on` step id exceeds 64 characters".to_string());
        }
        validate_step_id(&s)?;
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

fn parse_tool_call_refs(val: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`tool_call_refs` must be an array")?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or("`tool_call_refs` entries must be strings")?
            .trim()
            .to_string();
        if s.is_empty() || s.chars().count() > 200 {
            return Err("`tool_call_refs` entries must be 1..=200 characters".to_string());
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    Ok(out)
}

fn parse_claims(val: Option<&Value>) -> Result<Vec<Claim>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let arr = v.as_array().ok_or("`claims` must be an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let raw: RawClaimInput = serde_json::from_value(item.clone())
            .map_err(|e| format!("invalid claims[{idx}]: {e}"))?;
        let lhs = HandleRef::parse(raw.lhs.trim())
            .map_err(|e| format!("claims[{idx}].lhs: {e}"))?;
        let rhs = HandleRef::parse(raw.rhs.trim())
            .map_err(|e| format!("claims[{idx}].rhs: {e}"))?;
        let order = match raw.order {
            Some(mut ord) => {
                ord.variable = ord.variable.trim().to_string();
                ord.point = ord.point.trim().to_string();
                if ord.variable.is_empty() || ord.variable.chars().count() > 64 {
                    return Err(format!("claims[{idx}].order.variable must be 1..=64 characters"));
                }
                if ord.point.is_empty() || ord.point.chars().count() > 128 {
                    return Err(format!("claims[{idx}].order.point must be 1..=128 characters"));
                }
                Some(ord)
            }
            None if raw.form == ClaimForm::Series => {
                return Err(format!("claims[{idx}] with form `series` requires `order`"));
            }
            None => None,
        };
        let domain = validate_optional_text(raw.domain, "domain", idx)?;
        let how = validate_optional_text(raw.how, "how", idx)?;
        out.push(Claim {
            form: raw.form,
            lhs,
            rhs,
            relation: raw.relation,
            order,
            domain,
            how,
        });
    }
    Ok(out)
}

fn validate_optional_text(
    raw: Option<String>,
    field: &str,
    idx: usize,
) -> Result<Option<String>, String> {
    let Some(text) = raw else {
        return Ok(None);
    };
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_DOMAIN_HOW_CHARS {
        return Err(format!(
            "claims[{idx}].{field} exceeds {MAX_DOMAIN_HOW_CHARS} characters"
        ));
    }
    Ok(Some(trimmed))
}

fn parse_source_ref(val: Option<&Value>) -> Result<Option<QuoteRef>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let raw: RawQuoteRefInput =
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid `source_ref`: {e}"))?;
    if raw.start_line == 0 || raw.end_line < raw.start_line {
        return Err("`source_ref` must satisfy 1 <= start_line <= end_line".to_string());
    }
    Ok(Some(QuoteRef {
        source_id: raw.source_id,
        path: raw.path,
        start_line: raw.start_line,
        end_line: raw.end_line,
        file_sha256: raw.file_sha256,
        quote_sha256: raw.quote_sha256,
        evidence_seq: Some(raw.evidence_seq),
    }))
}

fn parse_plan(val: Option<&Value>) -> Result<Option<Vec<PlanItem>>, String> {
    let Some(v) = val.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let arr = v.as_array().ok_or("`plan` must be an array")?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let raw: RawPlanItemInput =
            serde_json::from_value(item.clone()).map_err(|e| format!("invalid plan[{idx}]: {e}"))?;
        let title = raw.title.trim().to_string();
        if title.is_empty() || title.chars().count() > MAX_PLAN_TITLE_CHARS {
            return Err(format!("plan[{idx}].title must be 1..={MAX_PLAN_TITLE_CHARS} characters"));
        }
        out.push(PlanItem { title });
    }
    Ok(Some(out))
}
