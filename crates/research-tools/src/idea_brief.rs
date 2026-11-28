//! Explore Sharpen recorder (`IdeaBrief`, `Perspective`, `record_idea_brief`) — package X6.
//!
//! Governance: `explore.md` 4.1 (`IdeaBrief`, `Perspective`), 4.2 (`idea.brief`, `idea.answer`),
//! 5.1 (`record_idea_brief`), `00-overview.md` 3.2 row 18 (`"Sharpened idea"`, never `"brief"` on
//! screen).
//!
//! Recorder input uses `#[serde(deny_unknown_fields)]` and collects all validation faults in one
//! pass so the exchange runner can repair everything cleanly.

use crate::lit_providers::LitProvider;
use crate::search_coverage::PerspectiveKind;
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tools::EVENT_SCHEMA_VERSION;

/// Ledger event kind for a recorded sharpened idea (`explore.md` 4.2).
pub const EVENT_IDEA_BRIEF: &str = "idea.brief";
/// Ledger event kind for user answers and edits to a sharpened idea (`explore.md` 4.2).
pub const EVENT_IDEA_ANSWER: &str = "idea.answer";
/// Single recorder tool name offered in the Sharpen exchange (`explore.md` 5.1).
pub const RECORDER_IDEA_BRIEF: &str = "record_idea_brief";
/// On-screen heading governed by `00-overview.md` 3.2 row 18 (never show `"brief"` on screen).
pub const UI_SHARPENED_IDEA_LABEL: &str = "Sharpened idea";

pub const MAX_RESTATEMENT_CHARS: usize = 1_200;
pub const MAX_ASSUMPTIONS: usize = 8;
pub const MAX_ASSUMPTION_CHARS: usize = 400;
pub const MIN_SUBCLAIMS: usize = 1;
pub const MAX_SUBCLAIMS: usize = 4;
pub const MAX_SUBCLAIM_CHARS: usize = 500;
pub const MIN_PERSPECTIVES: usize = 2;
pub const MAX_PERSPECTIVES: usize = 6;
pub const MIN_PERSPECTIVE_QUERIES: usize = 1;
pub const MAX_PERSPECTIVE_QUERIES: usize = 4;
pub const MAX_QUERY_CHARS: usize = 300;
pub const MAX_QUESTIONS: usize = 2;
pub const MAX_QUESTION_OPTIONS: usize = 6;
pub const MAX_TERMS: usize = 24;
pub const MAX_ALIASES_PER_TERM: usize = 12;
pub const MAX_EXCLUSIONS: usize = 12;

/// Strict model input for `record_idea_brief` (no ids; sequential `sc-N`/`p-N` assigned by code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeaBriefInput {
    pub restatement: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    pub subclaims: Vec<SubclaimInput>,
    #[serde(default)]
    pub terms: Vec<IdeaTermGroup>,
    pub perspectives: Vec<PerspectiveInput>,
    #[serde(default)]
    pub questions: Vec<SharpenQuestionInput>,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubclaimInput {
    pub text: String,
    /// Must be an exact substring of `IdeaStatement.text` (`explore.md` 4.1).
    pub idea_quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdeaTermGroup {
    pub term: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerspectiveQueryInput {
    pub provider: LitProvider,
    pub q: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerspectiveInput {
    pub kind: PerspectiveKind,
    pub question: String,
    pub queries: Vec<PerspectiveQueryInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharpenQuestionInput {
    pub question: String,
    pub options: Vec<String>,
    pub default: String,
}

/// Validated, id-assigned sharpened idea stored in `idea.brief` (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaBrief {
    pub idea_rev: u32,
    pub restatement: String,
    pub assumptions: Vec<String>,
    pub subclaims: Vec<Subclaim>,
    pub terms: Vec<IdeaTermGroup>,
    pub perspectives: Vec<Perspective>,
    pub questions: Vec<SharpenQuestion>,
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subclaim {
    pub id: String,
    pub text: String,
    pub idea_quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveQuery {
    pub provider: LitProvider,
    pub q: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perspective {
    pub id: String,
    pub kind: PerspectiveKind,
    pub question: String,
    pub queries: Vec<PerspectiveQuery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharpenQuestion {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    pub default: String,
}

/// JSON schema for the single `record_idea_brief` recorder tool offered in Sharpen.
pub fn record_idea_brief_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "restatement": {
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_RESTATEMENT_CHARS,
                "description": "Concise restatement of the idea (<= 1200 chars)."
            },
            "assumptions": {
                "type": "array",
                "maxItems": MAX_ASSUMPTIONS,
                "items": { "type": "string", "minLength": 1, "maxLength": MAX_ASSUMPTION_CHARS }
            },
            "subclaims": {
                "type": "array",
                "minItems": MIN_SUBCLAIMS,
                "maxItems": MAX_SUBCLAIMS,
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "minLength": 1, "maxLength": MAX_SUBCLAIM_CHARS },
                        "idea_quote": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Exact verbatim substring of the user's idea text."
                        }
                    },
                    "required": ["text", "idea_quote"],
                    "additionalProperties": false
                }
            },
            "terms": {
                "type": "array",
                "maxItems": MAX_TERMS,
                "items": {
                    "type": "object",
                    "properties": {
                        "term": { "type": "string", "minLength": 1 },
                        "aliases": { "type": "array", "items": { "type": "string", "minLength": 1 } }
                    },
                    "required": ["term"],
                    "additionalProperties": false
                }
            },
            "perspectives": {
                "type": "array",
                "minItems": MIN_PERSPECTIVES,
                "maxItems": MAX_PERSPECTIVES,
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": [
                                "exact", "other_names", "generalisation",
                                "special_case", "method", "obstruction", "adjacent_field"
                            ]
                        },
                        "question": { "type": "string", "minLength": 1 },
                        "queries": {
                            "type": "array",
                            "minItems": MIN_PERSPECTIVE_QUERIES,
                            "maxItems": MAX_PERSPECTIVE_QUERIES,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "provider": {
                                        "type": "string",
                                        "enum": [
                                            "arxiv", "inspire", "semantic_scholar",
                                            "crossref", "openalex", "theoremsearch"
                                        ]
                                    },
                                    "q": { "type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS }
                                },
                                "required": ["provider", "q"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["kind", "question", "queries"],
                    "additionalProperties": false
                }
            },
            "questions": {
                "type": "array",
                "maxItems": MAX_QUESTIONS,
                "items": {
                    "type": "object",
                    "properties": {
                        "question": { "type": "string", "minLength": 1 },
                        "options": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_QUESTION_OPTIONS,
                            "items": { "type": "string", "minLength": 1 }
                        },
                        "default": { "type": "string", "minLength": 1 }
                    },
                    "required": ["question", "options", "default"],
                    "additionalProperties": false
                }
            },
            "exclusions": {
                "type": "array",
                "maxItems": MAX_EXCLUSIONS,
                "items": { "type": "string", "minLength": 1 }
            }
        },
        "required": ["restatement", "subclaims", "perspectives"],
        "additionalProperties": false
    })
}

/// Validate raw model input against `explore.md` 4.1 constraints and assign sequential
/// `sc-N`, `p-N`, and `q-N` identifiers.
pub fn validate_and_build_idea_brief(
    idea_rev: u32,
    idea_text: &str,
    raw: &Value,
) -> Result<IdeaBrief, Vec<String>> {
    let input: IdeaBriefInput = serde_json::from_value(raw.clone())
        .map_err(|e| vec![format!("invalid `record_idea_brief` schema: {e}")])?;

    let mut faults = Vec::new();
    let restatement = input.restatement.trim().to_string();
    let restatement_len = restatement.chars().count();
    if restatement.is_empty() {
        faults.push("`restatement` must be non-empty".into());
    } else if restatement_len > MAX_RESTATEMENT_CHARS {
        faults.push(format!(
            "`restatement` exceeds {MAX_RESTATEMENT_CHARS} characters ({restatement_len})"
        ));
    }

    if input.assumptions.len() > MAX_ASSUMPTIONS {
        faults.push(format!(
            "`assumptions` may contain at most {MAX_ASSUMPTIONS} items (got {})",
            input.assumptions.len()
        ));
    }
    let mut assumptions = Vec::with_capacity(input.assumptions.len());
    for (idx, a) in input.assumptions.iter().enumerate() {
        let t = a.trim();
        if t.is_empty() {
            faults.push(format!("`assumptions[{idx}]` must be non-empty"));
        } else if t.chars().count() > MAX_ASSUMPTION_CHARS {
            faults.push(format!(
                "`assumptions[{idx}]` exceeds {MAX_ASSUMPTION_CHARS} characters"
            ));
        } else {
            assumptions.push(t.to_string());
        }
    }

    if input.subclaims.len() < MIN_SUBCLAIMS || input.subclaims.len() > MAX_SUBCLAIMS {
        faults.push(format!(
            "`subclaims` must have {MIN_SUBCLAIMS}..={MAX_SUBCLAIMS} items (got {})",
            input.subclaims.len()
        ));
    }
    let mut subclaims = Vec::with_capacity(input.subclaims.len());
    for (idx, sc) in input.subclaims.iter().enumerate() {
        let text = sc.text.trim();
        if text.is_empty() {
            faults.push(format!("`subclaims[{idx}].text` must be non-empty"));
        } else if text.chars().count() > MAX_SUBCLAIM_CHARS {
            faults.push(format!(
                "`subclaims[{idx}].text` exceeds {MAX_SUBCLAIM_CHARS} characters"
            ));
        }
        // `idea_quote` MUST be an exact substring of the verbatim `IdeaStatement.text`.
        if sc.idea_quote.trim().is_empty() {
            faults.push(format!("`subclaims[{idx}].idea_quote` must be non-empty"));
        } else if sc.idea_quote.chars().count() > 500 {
            faults.push(format!("`subclaims[{idx}].idea_quote` exceeds 500 characters"));
        } else if !idea_text.contains(sc.idea_quote.as_str()) {
            faults.push(format!(
                "`subclaims[{idx}].idea_quote` must be an exact substring of the user's idea text"
            ));
        }
        subclaims.push(Subclaim {
            id: format!("sc-{}", idx + 1),
            text: text.to_string(),
            idea_quote: sc.idea_quote.clone(),
        });
    }

    if input.terms.len() > MAX_TERMS {
        faults.push(format!(
            "`terms` may contain at most {MAX_TERMS} items (got {})",
            input.terms.len()
        ));
    }
    let mut terms = Vec::with_capacity(input.terms.len());
    for (idx, tg) in input.terms.iter().enumerate() {
        let term = tg.term.trim();
        if term.is_empty() || term.chars().count() > 160 {
            faults.push(format!("`terms[{idx}].term` must be non-empty and <= 160 characters"));
            continue;
        }
        if tg.aliases.len() > MAX_ALIASES_PER_TERM {
            faults.push(format!(
                "`terms[{idx}].aliases` may contain at most {MAX_ALIASES_PER_TERM} items"
            ));
        }
        let mut aliases = Vec::with_capacity(tg.aliases.len());
        for (aidx, raw_alias) in tg.aliases.iter().enumerate() {
            let a = raw_alias.trim();
            if a.is_empty() || a.chars().count() > 160 {
                faults.push(format!(
                    "`terms[{idx}].aliases[{aidx}]` must be non-empty and <= 160 characters"
                ));
            } else {
                aliases.push(a.to_string());
            }
        }
        terms.push(IdeaTermGroup {
            term: term.to_string(),
            aliases,
        });
    }

    if input.perspectives.len() < MIN_PERSPECTIVES || input.perspectives.len() > MAX_PERSPECTIVES {
        faults.push(format!(
            "`perspectives` must have {MIN_PERSPECTIVES}..={MAX_PERSPECTIVES} items (got {})",
            input.perspectives.len()
        ));
    }
    let has_exact = input.perspectives.iter().any(|p| p.kind == PerspectiveKind::Exact);
    let has_other_names = input
        .perspectives
        .iter()
        .any(|p| p.kind == PerspectiveKind::OtherNames);
    if !has_exact {
        faults.push("`perspectives` must include at least one `exact` perspective".into());
    }
    if !has_other_names {
        faults.push("`perspectives` must include at least one `other_names` perspective".into());
    }

    let mut perspectives = Vec::with_capacity(input.perspectives.len());
    for (idx, p) in input.perspectives.iter().enumerate() {
        let question = p.question.trim();
        if question.is_empty() || question.chars().count() > 400 {
            faults.push(format!(
                "`perspectives[{idx}].question` must be non-empty and <= 400 characters"
            ));
        }
        if p.queries.len() < MIN_PERSPECTIVE_QUERIES || p.queries.len() > MAX_PERSPECTIVE_QUERIES {
            faults.push(format!(
                "`perspectives[{idx}].queries` must have {MIN_PERSPECTIVE_QUERIES}..={MAX_PERSPECTIVE_QUERIES} items (got {})",
                p.queries.len()
            ));
        }
        let mut queries = Vec::with_capacity(p.queries.len());
        for (qidx, q) in p.queries.iter().enumerate() {
            let trimmed_q = q.q.trim();
            if trimmed_q.is_empty() {
                faults.push(format!(
                    "`perspectives[{idx}].queries[{qidx}].q` must be non-empty"
                ));
            } else if trimmed_q.chars().count() > MAX_QUERY_CHARS {
                faults.push(format!(
                    "`perspectives[{idx}].queries[{qidx}].q` exceeds {MAX_QUERY_CHARS} characters"
                ));
            }
            queries.push(PerspectiveQuery {
                provider: q.provider,
                q: trimmed_q.to_string(),
            });
        }
        perspectives.push(Perspective {
            id: format!("p-{}", idx + 1),
            kind: p.kind,
            question: question.to_string(),
            queries,
        });
    }

    if input.questions.len() > MAX_QUESTIONS {
        faults.push(format!(
            "`questions` may contain at most {MAX_QUESTIONS} items (got {})",
            input.questions.len()
        ));
    }
    let mut questions = Vec::with_capacity(input.questions.len());
    for (idx, q) in input.questions.iter().enumerate() {
        let qtext = q.question.trim();
        if qtext.is_empty() || qtext.chars().count() > 300 {
            faults.push(format!(
                "`questions[{idx}].question` must be non-empty and <= 300 characters"
            ));
        }
        if q.options.is_empty() || q.options.len() > MAX_QUESTION_OPTIONS {
            faults.push(format!(
                "`questions[{idx}].options` must have 1..={MAX_QUESTION_OPTIONS} options (got {})",
                q.options.len()
            ));
        }
        let mut options = Vec::with_capacity(q.options.len());
        for (oidx, raw_opt) in q.options.iter().enumerate() {
            let o = raw_opt.trim();
            if o.is_empty() || o.chars().count() > 160 {
                faults.push(format!(
                    "`questions[{idx}].options[{oidx}]` must be non-empty and <= 160 characters"
                ));
            } else {
                options.push(o.to_string());
            }
        }
        let default = q.default.trim().to_string();
        if default.is_empty() || !options.iter().any(|o| o == &default) {
            faults.push(format!(
                "`questions[{idx}].default` must match one of its `options`"
            ));
        }
        questions.push(SharpenQuestion {
            id: format!("q-{}", idx + 1),
            question: qtext.to_string(),
            options,
            default,
        });
    }

    if input.exclusions.len() > MAX_EXCLUSIONS {
        faults.push(format!(
            "`exclusions` may contain at most {MAX_EXCLUSIONS} items (got {})",
            input.exclusions.len()
        ));
    }
    let mut exclusions = Vec::with_capacity(input.exclusions.len());
    for (idx, raw_ex) in input.exclusions.iter().enumerate() {
        let s = raw_ex.trim();
        if s.is_empty() || s.chars().count() > 200 {
            faults.push(format!(
                "`exclusions[{idx}]` must be non-empty and <= 200 characters"
            ));
        } else {
            exclusions.push(s.to_string());
        }
    }

    if !faults.is_empty() {
        return Err(faults);
    }

    Ok(IdeaBrief {
        idea_rev,
        restatement,
        assumptions,
        subclaims,
        terms,
        perspectives,
        questions,
        exclusions,
    })
}

/// Validate and append an `idea.brief` event to the session ledger (`explore.md` 4.2 & 5.1).
pub fn record_idea_brief(
    ledger: &Ledger,
    session_id: &str,
    idea_rev: u32,
    idea_text: &str,
    request_seq: Option<u64>,
    raw: &Value,
) -> Result<(u64, IdeaBrief), Vec<String>> {
    let brief = validate_and_build_idea_brief(idea_rev, idea_text, raw)?;
    let mut payload = serde_json::to_value(&brief)
        .map_err(|e| vec![format!("failed to serialize idea brief: {e}")])?;
    if let Some(obj) = payload.as_object_mut() {
        if let Some(seq) = request_seq {
            obj.insert("request_seq".into(), json!(seq));
        }
    }
    let ev = ledger
        .append_with_hash(session_id, EVENT_IDEA_BRIEF, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| vec![format!("failed to append `{EVENT_IDEA_BRIEF}`: {e}")])?;
    Ok((ev.seq, brief))
}

#[cfg(test)]
#[path = "idea_brief_tests.rs"]
mod tests;
