//! Validation and Rule D1/D2/V8 enforcement for `record_idea_directions` (package X13).
//!
//! Separated from [`crate::idea_directions`] so every source file stays well below 600 lines.

use crate::idea_directions::{
    Concreteness, Direction, DirectionInput, DirectionKnown, DirectionMissing, DirectionRisk,
    FirstCalcCheck, FirstCalculation, IdeaDirectionsPayload, MissingBasis,
    RecordIdeaDirectionsInput, RejectedDirection, SolveSetup, SolveSetupTarget,
    MAX_DIRECTIONS, MAX_DIRECTION_TEXT_CHARS, MAX_DIRECTION_TITLE_CHARS,
    MAX_SOLVE_CONVENTIONS_CHARS, MAX_SOLVE_STATEMENT_CHARS, MIN_CONCRETE_SETUP_CHARS,
    RECORDER_IDEA_DIRECTIONS,
};
use crate::idea_map::{
    check_explore_prose, IdeaMapPayload, MapNode, MapNodeKind, MapNodeStatus, EVENT_IDEA_MAP,
};
use crate::paper_key::PaperKey;
use crate::quote_core::EVIDENCE_QUOTE_EVENT;
use ledger::Event;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Validate a raw `record_idea_directions` payload against the active session map & ledger events,
/// enforcing Rules D1 (valid node refs, valid `missing.basis`, Rule V8 banned phrases), D2
/// (app-computed `Concreteness::Concrete` vs `Sketch`), `<= 5` directions, and no `"lean"` method.
pub fn validate_record_idea_directions(
    events: &[Event],
    request_seq: Option<u64>,
    raw_input: &Value,
) -> Result<IdeaDirectionsPayload, Vec<String>> {
    let input: RecordIdeaDirectionsInput = serde_json::from_value(raw_input.clone())
        .map_err(|e| vec![format!("invalid `{RECORDER_IDEA_DIRECTIONS}` payload: {e}")])?;

    if input.directions.is_empty() {
        return Err(vec!["`directions` must contain at least one direction".into()]);
    }
    if input.directions.len() > MAX_DIRECTIONS {
        return Err(vec![format!(
            "`directions` exceeds maximum of {MAX_DIRECTIONS} (got {})",
            input.directions.len()
        )]);
    }

    let stmt_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == "idea.stated")
        .map_or(0, |e| e.seq);
    let map_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
        .ok_or_else(|| {
            vec!["cannot record directions without an active `idea.map` in this revision".into()]
        })?;
    let map_payload: IdeaMapPayload = serde_json::from_value(map_ev.payload.clone())
        .map_err(|e| vec![format!("active `idea.map` payload is unreadable: {e}")])?;

    let nodes_by_id: BTreeMap<&str, &MapNode> =
        map_payload.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let session_xe_ids: BTreeSet<String> = events
        .iter()
        .filter(|e| e.kind == EVIDENCE_QUOTE_EVENT && e.seq > stmt_seq)
        .map(|e| format!("xe-{}", e.seq))
        .collect();

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut next_id = 1u64;

    for (idx, item) in input.directions.into_iter().enumerate() {
        let title_for_err = item.title.trim().to_string();
        match validate_single_direction(item, &nodes_by_id, &session_xe_ids, next_id) {
            Ok(dir) => {
                next_id += 1;
                accepted.push(dir);
            }
            Err(reasons) => rejected.push(RejectedDirection {
                index: idx,
                title: title_for_err,
                reasons,
            }),
        }
    }

    if accepted.is_empty() {
        let errs = rejected
            .into_iter()
            .flat_map(|r| {
                r.reasons
                    .into_iter()
                    .map(move |reason| format!("directions[{}]: {reason}", r.index))
            })
            .collect();
        return Err(errs);
    }

    Ok(IdeaDirectionsPayload {
        map_seq: map_ev.seq,
        request_seq,
        directions: accepted,
        rejected,
    })
}

fn validate_single_direction(
    item: DirectionInput,
    nodes_by_id: &BTreeMap<&str, &MapNode>,
    session_xe_ids: &BTreeSet<String>,
    seq_id: u64,
) -> Result<Direction, Vec<String>> {
    let mut errs = Vec::new();
    check_text_field(&item.title, MAX_DIRECTION_TITLE_CHARS, "title", true, &mut errs);
    check_text_field(&item.question, MAX_DIRECTION_TEXT_CHARS, "question", true, &mut errs);
    check_text_field(&item.first_calculation.setup, MAX_DIRECTION_TEXT_CHARS, "first_calculation.setup", false, &mut errs);
    check_text_field(&item.first_calculation.expected_form, MAX_DIRECTION_TEXT_CHARS, "first_calculation.expected_form", true, &mut errs);
    check_text_field(&item.first_calculation.check.text, MAX_DIRECTION_TEXT_CHARS, "first_calculation.check.text", false, &mut errs);
    check_text_field(&item.stop_if, MAX_DIRECTION_TEXT_CHARS, "stop_if", false, &mut errs);
    check_text_field(&item.solve_setup.statement, MAX_SOLVE_STATEMENT_CHARS, "solve_setup.statement", true, &mut errs);
    check_text_field(&item.solve_setup.target.description, MAX_DIRECTION_TEXT_CHARS, "solve_setup.target.description", true, &mut errs);
    check_text_field(&item.solve_setup.conventions, MAX_SOLVE_CONVENTIONS_CHARS, "solve_setup.conventions", false, &mut errs);

    // Rule D1: validate all node references in known, missing, inputs, check.reproduces, risks.
    let mut known_clean = Vec::with_capacity(item.known.len());
    for (k_i, k) in item.known.into_iter().enumerate() {
        check_text_field(&k.text, MAX_DIRECTION_TEXT_CHARS, &format!("known[{k_i}].text"), true, &mut errs);
        let ids = validate_node_id_list(&k.node_ids, &format!("known[{k_i}].node_ids"), nodes_by_id, &mut errs);
        known_clean.push(DirectionKnown { text: k.text.trim().to_string(), node_ids: ids });
    }

    let mut missing_clean = Vec::with_capacity(item.missing.len());
    for (m_i, m) in item.missing.into_iter().enumerate() {
        check_text_field(&m.text, MAX_DIRECTION_TEXT_CHARS, &format!("missing[{m_i}].text"), true, &mut errs);
        if let MissingBasis::OpenQuestion(ref qid) = m.basis {
            match nodes_by_id.get(qid.as_str()) {
                Some(node) if node.status == MapNodeStatus::NotInCurrentMap => errs.push(format!(
                    "missing[{m_i}].basis references `{qid}` which is `not_in_current_map` (Rule D1)"
                )),
                Some(node) if node.kind == MapNodeKind::OpenQuestion => {}
                Some(node) => errs.push(format!(
                    "missing[{m_i}].basis references `{qid}` which is `{}`, not `open_question` (Rule D1)",
                    node.kind.as_str()
                )),
                None => errs.push(format!(
                    "missing[{m_i}].basis references unknown open_question node `{qid}` (Rule D1)"
                )),
            }
        }
        missing_clean.push(DirectionMissing { text: m.text.trim().to_string(), basis: m.basis });
    }

    let clean_inputs = validate_node_id_list(
        &item.first_calculation.inputs,
        "first_calculation.inputs",
        nodes_by_id,
        &mut errs,
    );

    let clean_reproduces = match item.first_calculation.check.reproduces {
        None => None,
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                match nodes_by_id.get(trimmed) {
                    Some(node) if node.status == MapNodeStatus::NotInCurrentMap => {
                        errs.push(format!(
                            "first_calculation.check.reproduces `{trimmed}` is `not_in_current_map` (Rule D1)"
                        ));
                        None
                    }
                    Some(node) if node.kind == MapNodeKind::KnownResult => Some(trimmed.to_string()),
                    Some(node) => {
                        errs.push(format!(
                            "first_calculation.check.reproduces `{trimmed}` must be a `known_result` (`k-N`), got `{}` (Rule D1)",
                            node.kind.as_str()
                        ));
                        None
                    }
                    None => {
                        errs.push(format!(
                            "first_calculation.check.reproduces references unknown node `{trimmed}` (Rule D1)"
                        ));
                        None
                    }
                }
            }
        }
    };

    let mut risks_clean = Vec::with_capacity(item.risks.len());
    for (r_i, r) in item.risks.into_iter().enumerate() {
        check_text_field(&r.text, MAX_DIRECTION_TEXT_CHARS, &format!("risks[{r_i}].text"), true, &mut errs);
        let ids = validate_node_id_list(&r.node_ids, &format!("risks[{r_i}].node_ids"), nodes_by_id, &mut errs);
        risks_clean.push(DirectionRisk { text: r.text.trim().to_string(), kind: r.kind, node_ids: ids });
    }

    let mut clean_source_keys = Vec::with_capacity(item.solve_setup.source_keys.len());
    let mut seen_keys = BTreeSet::new();
    for (s_i, raw_key) in item.solve_setup.source_keys.into_iter().enumerate() {
        match PaperKey::parse(raw_key.trim()) {
            Ok(pk) => {
                let canonical = pk.to_string();
                if seen_keys.insert(canonical.clone()) {
                    clean_source_keys.push(canonical);
                }
            }
            Err(_) => errs.push(format!(
                "solve_setup.source_keys[{s_i}] `{raw_key}` is not a valid canonical PaperKey"
            )),
        }
    }

    if !errs.is_empty() {
        return Err(errs);
    }

    let first_calc = FirstCalculation {
        setup: item.first_calculation.setup.trim().to_string(),
        inputs: clean_inputs,
        method: item.first_calculation.method,
        expected_form: item.first_calculation.expected_form.trim().to_string(),
        check: FirstCalcCheck {
            text: item.first_calculation.check.text.trim().to_string(),
            reproduces: clean_reproduces,
        },
        effort: item.first_calculation.effort,
    };
    let stop_if = item.stop_if.trim().to_string();

    // Rule D2: compute concreteness purely from validated records.
    let (concreteness, concreteness_reasons) = compute_concreteness(
        &known_clean,
        &first_calc,
        &risks_clean,
        &stop_if,
        nodes_by_id,
        session_xe_ids,
    );

    Ok(Direction {
        id: format!("r-{seq_id}"),
        title: item.title.trim().to_string(),
        question: item.question.trim().to_string(),
        known: known_clean,
        missing: missing_clean,
        first_calculation: first_calc,
        risks: risks_clean,
        stop_if,
        solve_setup: SolveSetup {
            statement: item.solve_setup.statement.trim().to_string(),
            task_kind: item.solve_setup.task_kind,
            target: SolveSetupTarget {
                description: item.solve_setup.target.description.trim().to_string(),
                form: item.solve_setup.target.form,
            },
            conventions: item.solve_setup.conventions.trim().to_string(),
            source_keys: clean_source_keys,
        },
        concreteness,
        concreteness_reasons,
    })
}

fn compute_concreteness(
    known: &[DirectionKnown],
    first_calc: &FirstCalculation,
    risks: &[DirectionRisk],
    stop_if: &str,
    nodes_by_id: &BTreeMap<&str, &MapNode>,
    session_xe_ids: &BTreeSet<String>,
) -> (Concreteness, Vec<String>) {
    let mut missing = Vec::new();
    let has_known_fulltext = known.iter().any(|k| {
        k.node_ids.iter().any(|nid| {
            nodes_by_id.get(nid.as_str()).is_some_and(|node| {
                node.evidence
                    .iter()
                    .any(|ev| ev.starts_with("xe-") && session_xe_ids.contains(ev))
            })
        })
    });
    if !has_known_fulltext {
        missing.push("needs >= 1 known item naming a node backed by full-text xe evidence".into());
    }
    if first_calc.setup.chars().count() < MIN_CONCRETE_SETUP_CHARS {
        missing.push(format!(
            "first_calculation.setup must be >= {MIN_CONCRETE_SETUP_CHARS} characters"
        ));
    }
    if first_calc.inputs.is_empty() {
        missing.push("first_calculation.inputs must name >= 1 resolvable map node".into());
    }
    if first_calc.check.text.is_empty() {
        missing.push("first_calculation.check.text must not be empty".into());
    }
    if risks.is_empty() {
        missing.push("needs >= 1 explicit risk".into());
    }
    if stop_if.is_empty() {
        missing.push("stop_if must not be empty".into());
    }
    if missing.is_empty() {
        (Concreteness::Concrete, missing)
    } else {
        (Concreteness::Sketch, missing)
    }
}

fn validate_node_id_list(
    raw: &[String],
    field: &str,
    nodes_by_id: &BTreeMap<&str, &MapNode>,
    errs: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for id in raw {
        let trimmed = id.trim();
        match nodes_by_id.get(trimmed) {
            None => errs.push(format!("{field} references unknown map node `{trimmed}` (Rule D1)")),
            Some(node) if node.status == MapNodeStatus::NotInCurrentMap => {
                errs.push(format!("{field} references node `{trimmed}` which is `not_in_current_map` (Rule D1)"));
            }
            Some(_) => {
                if seen.insert(trimmed.to_string()) {
                    out.push(trimmed.to_string());
                }
            }
        }
    }
    out
}

fn check_text_field(
    raw: &str,
    max_chars: usize,
    field: &str,
    required: bool,
    errs: &mut Vec<String>,
) {
    let trimmed = raw.trim();
    if required && trimmed.is_empty() {
        errs.push(format!("`{field}` must not be empty"));
        return;
    }
    let count = trimmed.chars().count();
    if count > max_chars {
        errs.push(format!("`{field}` exceeds {max_chars} characters (got {count})"));
    }
    if !trimmed.is_empty() {
        if let Err(banned) = check_explore_prose(trimmed) {
            errs.push(format!(
                "`{field}` contains banned absolute phrase(s) [{}]; use a scoped statement (Rule V8)",
                banned.join(", ")
            ));
        }
    }
}
