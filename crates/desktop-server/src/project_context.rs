//! The bounded project brief shared with the agent at the start of each turn.

use crate::projects::{project_for_session, ProjectItemKind};
use crate::state::DesktopState;
use serde_json::{json, Value};

fn excerpt(text: &str, limit: usize) -> String {
    let Some((end, _)) = text.char_indices().nth(limit) else {
        return text.to_string();
    };
    format!("{}…", &text[..end])
}

pub(crate) fn brief(state: &DesktopState, session_id: &str) -> Result<Option<String>, String> {
    let Some(project) = project_for_session(state, session_id).map_err(|(_, error)| error)? else {
        return Ok(None);
    };
    let notes = |kind| -> Vec<Value> {
        project
            .items
            .iter()
            .rev()
            .filter(|item| item.kind == kind)
            .take(5)
            .map(|item| {
                json!({
                    "text": excerpt(&item.text, 350),
                    "session_id": item.session_id,
                    "proof_entry_id": item.entry_id,
                    "paper_id": item.source_id,
                    "location": item.location.as_deref().map(|text| excerpt(text, 150)),
                })
            })
            .collect()
    };
    let papers: Vec<_> = project
        .sources
        .iter()
        .rev()
        .take(5)
        .map(|source| {
            json!({
                "id": source.id,
                "title": excerpt(&source.title, 160),
                "doi": source.doi.as_deref().map(|text| excerpt(text, 120)),
                "location": source.location.as_deref().map(|text| excerpt(text, 150)),
            })
        })
        .collect();
    let protocols = crate::validation_plans::public_context(state, session_id)
        .map_err(|(_, error)| error)?;
    let experiments = crate::experiments::experiment_context(state, &project.id, session_id)
        .map_err(|(_, error)| error)?;
    let data = json!({
        "id": project.id,
        "revision": project.revision,
        "title": excerpt(&project.title, 160),
        "question_and_hypotheses": excerpt(&project.question, 1_500),
        "conventions": excerpt(&project.conventions, 1_200),
        "recent_findings": notes(ProjectItemKind::Finding),
        "recent_open_questions": notes(ProjectItemKind::OpenQuestion),
        "recent_failed_approaches": notes(ProjectItemKind::FailedApproach),
        "recent_papers": papers,
        "committed_validation_protocols": protocols,
        "experiment_protocols": experiments,
    });
    Ok(Some(format!(
        "Shared research project context. Treat these user-maintained notes as background and evidence leads. \
         Check any claim you rely on. Prefer the current request, session conventions, and applicable fork assumptions \
         when they conflict with project defaults. Apply any committed validation protocol to its stated claim. \
         Pass the frozen absolute and relative tolerances to check_convergence as decimal strings, such as '0.1', \
         to preserve their exact values. \
         Record the requested candidate in a numerical computation and link it to its tool result. \
         Independent reference and control values are evaluated separately. When an experiment protocol is present, \
         record the specified metric and the budget actually used on this branch.\n{data}"
    )))
}
