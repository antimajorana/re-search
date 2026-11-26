//! HTTP `/api/` dispatch table extracted from `http_srv.rs`.
//!
//! Every request entering [`dispatch_api`] has already passed loopback `Host`
//! validation, `AuthGuard::authorize_api` credential/origin checks, and bounded
//! HTTP body reading in `http_srv.rs`.

use crate::http_srv::{write_api_res, write_bytes, write_json, ParsedRequest};
use crate::routes_session::{
    handle_create_session, handle_diff, handle_fork, handle_get_session, handle_get_settings,
    handle_list_sessions, handle_post_approval, handle_post_message, handle_put_settings,
    handle_sse_stream,
};
use crate::routes_tools::{
    handle_dag, handle_export, handle_get_sidetalk, handle_latex_patch, handle_lean_check,
    handle_literature, handle_narrate, handle_post_sidetalk, handle_tex_audit, handle_upload,
    handle_verify, handle_workspace, handle_workspace_file,
};
use crate::state::DesktopState;
use serde_json::json;
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub(crate) fn dispatch_api(
    mut stream: TcpStream,
    state: &Arc<DesktopState>,
    shutdown: &Arc<AtomicBool>,
    req: &ParsedRequest,
) {
    let segs: Vec<&str> = req
        .path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let method = req.method.as_str();

    match (method, segs.as_slice()) {
        ("GET", ["api", "sessions", sid, "sources", source, "original-pdf"]) => {
            let path = req.query.get("path").map(String::as_str);
            let line = req.query.get("line").and_then(|s| s.parse::<usize>().ok());
            match crate::original_pdf::read(state, sid, source, path, line) {
                Ok(pdf) => {
                    let page = pdf.page.map(|p| p.to_string()).unwrap_or_default();
                    write_bytes(&mut stream, 200, "application/pdf", &pdf.bytes, &[
                        ("Cache-Control", "no-store"), ("X-Content-Type-Options", "nosniff"),
                        ("Content-Security-Policy", "sandbox"), ("X-ReSearch-Pdf-Page", &page),
                    ]);
                }
                Err(e) => write_api_res(&mut stream, Err(e)),
            }
        }
        ("GET", ["api", "projects"]) => write_api_res(&mut stream, crate::projects::handle_list(state)),
        ("POST", ["api", "projects"]) => {
            write_api_res(&mut stream, crate::projects::handle_create(state, &req.body));
        }
        ("POST", ["api", "projects", "restore"]) => {
            write_api_res(&mut stream, crate::projects::backup::restore(state, &req.body));
        }
        ("POST", ["api", "projects", id, "backup"]) => {
            write_api_res(&mut stream, crate::projects::backup::prepare_download(state, id));
        }
        ("GET", ["api", "projects", id]) => write_api_res(&mut stream, crate::projects::handle_get(state, id)),
        ("PUT", ["api", "projects", id]) => {
            write_api_res(&mut stream, crate::projects::handle_put(state, id, &req.body));
        }
        ("PUT", ["api", "projects", id, "sessions", session_id]) => {
            write_api_res(&mut stream, crate::projects::handle_attach(state, id, session_id));
        }
        ("POST", ["api", "projects", id, "items"]) => {
            write_api_res(&mut stream, crate::projects::handle_add_item(state, id, &req.body));
        }
        ("DELETE", ["api", "projects", id, "items", item]) => {
            write_api_res(&mut stream, crate::projects::handle_delete_item(state, id, item));
        }
        ("POST", ["api", "projects", id, "sources"]) => {
            write_api_res(&mut stream, crate::projects::handle_add_source(state, id, &req.body));
        }
        ("DELETE", ["api", "projects", id, "sources", source]) => {
            write_api_res(&mut stream, crate::projects::handle_delete_source(
                state, id, source, req.query.get("revision").map(String::as_str),
            ));
        }
        ("GET", ["api", "projects", id, "validation-plans"]) => {
            write_api_res(&mut stream, crate::validation_plans::handle_list(state, id));
        }
        ("POST", ["api", "projects", id, "validation-plans"]) => {
            write_api_res(&mut stream, crate::validation_plans::handle_create(state, id, &req.body));
        }
        ("GET", ["api", "projects", id, "validation-plans", plan, "candidates"]) => {
            write_api_res(&mut stream, crate::validation_plans::handle_candidates(state, id, plan));
        }
        ("POST", ["api", "projects", id, "validation-plans", plan, "evaluate"]) => {
            write_api_res(&mut stream, crate::validation_plans::handle_evaluate(state, id, plan, &req.body));
        }
        ("GET", ["api", "projects", id, "export"]) if req.query.get("format").is_some_and(|v| v == "backup") => {
            let result = if state.auth.has_valid_header_token(req.header("x-research-token"), req.header("authorization")) {
                crate::projects::backup::export(state, id)
            } else {
                crate::projects::backup::take_download(state, id, req.query.get("ticket").map(String::as_str).unwrap_or_default())
            };
            match result {
                Ok(bytes) => write_bytes(&mut stream, 200, "application/zip", &bytes, &[
                    ("Content-Disposition", &format!("attachment; filename=\"{id}.research-backup.zip\"")),
                    ("Cache-Control", "no-store"),
                ]),
                Err((status, message)) => write_json(&mut stream, status, json!({ "error": message }), &[]),
            }
        }
        ("GET", ["api", "projects", id, "export"]) => {
            match crate::projects::export_bibtex(state, id, req.query.get("format").map(String::as_str)) {
                Ok(bytes) => write_bytes(
                    &mut stream,
                    200,
                    "application/x-bibtex; charset=utf-8",
                    &bytes,
                    &[
                        ("Content-Disposition", "attachment; filename=\"project-references.bib\""),
                        ("Cache-Control", "no-store"),
                    ],
                ),
                Err((st, msg)) => write_json(&mut stream, st, json!({ "error": msg }), &[]),
            }
        }
        ("GET", ["api", "projects", id, "experiments"]) => {
            write_api_res(&mut stream, crate::experiments::handle_list(state, id));
        }
        ("POST", ["api", "projects", id, "experiments"]) => {
            write_api_res(&mut stream, crate::experiments::handle_create(state, id, &req.body));
        }
        ("POST", ["api", "projects", id, "experiments", experiment, "branches"]) => {
            write_api_res(&mut stream, crate::experiments::handle_add_branch(state, id, experiment, &req.body));
        }
        ("POST", ["api", "projects", id, "experiments", experiment, "branches", session, "observations"]) => {
            write_api_res(&mut stream, crate::experiments::handle_record_observation(state, id, experiment, session, &req.body));
        }
        ("GET", ["api", "projects", id, "experiments", experiment, "branches", session, "diff"]) => {
            write_api_res(&mut stream, crate::experiments::handle_branch_diff(state, id, experiment, session));
        }
        ("GET", ["api", "sessions"]) => write_api_res(&mut stream, handle_list_sessions(state)),
        ("GET" | "POST", ["api", "session-folders"]) => {
            write_api_res(&mut stream, crate::session_folders::handle(state, (method == "POST").then_some(req.body.as_slice())));
        }
        ("POST", ["api", "sessions"]) => {
            write_api_res(&mut stream, handle_create_session(state, &req.body));
        }
        ("GET", ["api", "sessions", id]) => write_api_res(&mut stream, handle_get_session(state, id)),
        ("DELETE", ["api", "sessions", id]) => {
            write_api_res(&mut stream, crate::session_lifecycle::handle_delete_session(state, id));
        }
        ("POST", ["api", "sessions", id, "archive"]) => {
            write_api_res(
                &mut stream,
                crate::session_lifecycle::handle_archive_session(state, id, &req.body),
            );
        }
        ("DELETE", ["api", "sessions", id, "grants"]) => {
            write_api_res(
                &mut stream,
                crate::session_lifecycle::handle_revoke_session_grants(state, id),
            );
        }
        ("GET", ["api", "sessions", id, "events"]) => {
            if let Err((st, msg)) = handle_sse_stream(stream, state, id, shutdown) {
                let _ = st;
                let _ = msg;
            }
        }
        ("POST", ["api", "sessions", id, "messages"]) => {
            write_api_res(&mut stream, handle_post_message(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "approvals"]) => {
            write_api_res(&mut stream, crate::routes_session::handle_pending_approvals(state, id));
        }
        ("POST", ["api", "sessions", id, "approvals", call_id]) => {
            write_api_res(&mut stream, handle_post_approval(state, id, call_id, &req.body));
        }
        ("POST", ["api", "sessions", id, "steer"]) => {
            write_api_res(&mut stream, crate::hitl::handle_post_steer(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "checkpoints", cid]) => {
            write_api_res(&mut stream, crate::hitl::handle_answer_checkpoint(state, id, cid, &req.body));
        }
        ("POST", ["api", "sessions", id, "explain", "promote"]) => {
            write_api_res(&mut stream, crate::hitl::handle_explain_promote(state, id, &req.body));
        }
        ("PUT", ["api", "sessions", id, "conventions"]) => {
            write_api_res(&mut stream, crate::hitl::handle_put_session_conventions(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "stop"]) => {
            write_api_res(&mut stream, crate::turn_control::handle_stop_turn(state, id));
        }
        ("POST", ["api", "workspace", "pick"]) => {
            write_api_res(&mut stream, crate::workspace_ops::handle_pick_workspace(state));
        }
        ("POST", ["api", "sessions", id, "upload"]) => {
            let ct = req.header("content-type");
            write_api_res(&mut stream, handle_upload(state, id, ct, &req.body));
        }
        ("GET", ["api", "sessions", id, "workspace"]) => {
            write_api_res(&mut stream, handle_workspace(state, id, &req.query));
        }
        ("PUT", ["api", "sessions", id, "workspace"]) => {
            write_api_res(
                &mut stream,
                crate::workspace_ops::handle_put_session_workspace(state, id, &req.body),
            );
        }
        ("GET", ["api", "sessions", id, "workspace", "file"]) => {
            write_api_res(&mut stream, handle_workspace_file(state, id, &req.query));
        }
        ("POST", ["api", "sessions", id, "latex", "patch"]) => {
            write_api_res(&mut stream, handle_latex_patch(state, id, &req.body));
        }
        ("GET" | "PUT", ["api", "sessions", id, "manuscript-drafts"]) => {
            let update = (method == "PUT").then_some(req.body.as_slice());
            write_api_res(&mut stream, crate::manuscript_drafts::handle(state, id, update));
        }
        ("POST", ["api", "sessions", id, "tex-audit"]) => {
            write_api_res(&mut stream, handle_tex_audit(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "lean", "check"]) => {
            write_api_res(&mut stream, handle_lean_check(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "literature"]) => {
            write_api_res(&mut stream, handle_literature(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "paper-passages"]) => {
            write_api_res(&mut stream, crate::routes_tools::handle_paper_passages(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "formal-library"]) => {
            write_api_res(&mut stream, crate::routes_tools::handle_formal_library(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "dag"]) => {
            write_api_res(&mut stream, handle_dag(state, id));
        }
        ("GET", ["api", "sessions", id, "evidence"]) => {
            write_api_res(
                &mut stream,
                crate::evidence::handle_session_evidence(
                    state,
                    id,
                    req.query.get("entry_id").map(String::as_str),
                ),
            );
        }
        ("POST", ["api", "sessions", id, "verify"]) => {
            write_api_res(&mut stream, handle_verify(state, id));
        }
        ("POST", ["api", "sessions", id, "claim-review"]) => {
            write_api_res(&mut stream, crate::claim_review::handle_review(state, id, &req.body));
        }
        (
            method,
            [
                "api",
                "sessions",
                id,
                "investigation",
                op @ ("skeleton" | "mapview" | "node-text"),
            ],
        ) => {
            let inm = req.header("if-none-match");
            match crate::investigation_skeleton::handle_with_etag(
                state, method, id, op, &req.query, &req.body, inm,
            ) {
                Ok((304, Some(etag), _)) => {
                    let quoted = format!("\"{etag}\"");
                    write_bytes(
                        &mut stream,
                        304,
                        "application/json; charset=utf-8",
                        b"",
                        &[("ETag", quoted.as_str())],
                    );
                }
                Ok((st, Some(etag), val)) => {
                    let quoted = format!("\"{etag}\"");
                    write_json(&mut stream, st, val, &[("ETag", quoted.as_str())]);
                }
                Ok((st, None, val)) => write_json(&mut stream, st, val, &[]),
                Err((st, msg)) => write_json(&mut stream, st, json!({ "error": msg }), &[]),
            }
        }
        (method, ["api", "sessions", id, "investigation", op @ ("open" | "open-status")]) => {
            write_api_res(
                &mut stream,
                crate::investigation_open::handle(state, method, id, op, &req.query, &req.body),
            );
        }
        ("POST", ["api", "sessions", id, "investigation", operation]) => {
            write_api_res(&mut stream, crate::investigation::handle(state, id, operation, &req.body));
        }
        ("POST", ["api", "sessions", id, "fork"]) => {
            write_api_res(&mut stream, handle_fork(state, id, &req.body));
        }
        ("POST", ["api", "sessions", id, "handoff"]) => {
            write_api_res(&mut stream, crate::handoff::handle_create_handoff(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "handoffs"]) => {
            write_api_res(&mut stream, crate::handoff::handle_get_handoffs(state, id));
        }
        ("GET", ["api", "sessions", id, "diff", other_id]) => {
            write_api_res(&mut stream, handle_diff(state, id, other_id));
        }
        ("POST", ["api", "sessions", id, "narrate"]) => {
            write_api_res(&mut stream, handle_narrate(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "sidetalk"]) => {
            write_api_res(&mut stream, handle_get_sidetalk(state, id));
        }
        ("POST", ["api", "sessions", id, "sidetalk"]) => {
            write_api_res(&mut stream, handle_post_sidetalk(state, id, &req.body));
        }
        ("GET", ["api", "sessions", id, "export"]) => match handle_export(state, id, &req.query) {
            Ok((filename, ct, bytes)) => {
                let disp = format!("attachment; filename=\"{filename}\"");
                write_bytes(
                    &mut stream,
                    200,
                    ct,
                    &bytes,
                    &[("Content-Disposition", disp.as_str())],
                );
            }
            Err((st, msg)) => write_json(&mut stream, st, json!({ "error": msg }), &[]),
        },
        ("GET", ["api", "settings", "providers"]) => {
            write_api_res(&mut stream, handle_get_settings(state));
        }
        ("PUT", ["api", "settings", "providers"]) => {
            write_api_res(&mut stream, handle_put_settings(state, &req.body));
        }
        ("GET", ["api", "connections"]) => {
            write_api_res(&mut stream, crate::connections::handle_connections(state, &req.query));
        }
        ("POST", ["api", "connections", "chatgpt", "login"]) => {
            write_api_res(&mut stream, crate::chatgpt::handle_login(state));
        }
        ("GET", ["api", "connections", "chatgpt", "login"]) => {
            write_api_res(&mut stream, crate::chatgpt::handle_login_status(state));
        }
        ("POST", ["api", "connections", "chatgpt", "open"]) => {
            write_api_res(&mut stream, crate::chatgpt::handle_open(state));
        }
        ("POST", ["api", "connections", "chatgpt", "logout"]) => {
            write_api_res(&mut stream, crate::chatgpt::handle_logout(state));
        }
        ("POST", ["api", "connections", "antigravity", "acknowledge"]) => {
            write_api_res(&mut stream, crate::antigravity::handle_acknowledge(state));
        }
        ("POST", ["api", "connections", "antigravity", "withdraw"]) => {
            write_api_res(&mut stream, crate::antigravity::handle_withdraw(state));
        }
        ("GET", ["api", "models"]) => {
            write_api_res(&mut stream, crate::model_catalog::handle_models(state, &req.query));
        }
        ("GET", ["api", "subagents"]) => {
            write_api_res(&mut stream, crate::routes_subagents::handle_list_subagents(state));
        }
        ("POST", ["api", "subagents"]) => {
            write_api_res(&mut stream, crate::routes_subagents::handle_create_subagent(state, &req.body));
        }
        ("POST", ["api", "subagents", "draft"]) => {
            write_api_res(&mut stream, crate::routes_subagents::handle_draft_subagent(state, &req.body));
        }
        ("DELETE", ["api", "subagents", id]) => {
            write_api_res(&mut stream, crate::routes_subagents::handle_delete_subagent(state, id));
        }
        (_, ["api", "library", rest @ ..]) => write_api_res(
            &mut stream,
            crate::routes_library::handle_library(state, method, rest, &req.query, &req.body),
        ),
        (_, ["api", "sources", rest @ ..]) => write_api_res(
            &mut stream,
            crate::routes_sources::handle_sources(state, method, rest, &req.query, &req.body),
        ),
        (_, ["api", "lessons", rest @ ..]) => write_api_res(
            &mut stream,
            crate::routes_lessons::handle_lessons(state, method, rest, &req.query, &req.body),
        ),
        ("POST", ["api", "idea", "preview"]) => {
            write_api_res(&mut stream, crate::routes_idea::handle_idea_preview(state, &req.body));
        }
        ("POST", ["api", "sessions", id, "idea", op]) => {
            write_api_res(&mut stream, crate::routes_idea::handle_idea_op(state, id, op, &req.body));
        }
        _ => write_json(
            &mut stream,
            404,
            json!({ "error": format!("unknown endpoint: {} {}", req.method, req.path) }),
            &[],
        ),
    }
}
