//! Shared F8 harness bridge and fixtures/library helpers for Explore evaluation (`idea_eval.rs`, package X14).

#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/manifest.rs"]
pub mod manifest;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures_catalog.rs"]
pub mod measures_catalog;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures.rs"]
pub mod measures;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/sheet.rs"]
pub mod sheet;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/routed_mock.rs"]
pub mod routed_mock;

pub mod exchanges;
pub mod pdf_eval;
pub mod recall;

use agent::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, Transport,
    TransportError, TransportInfo,
};
use manifest::{parse_manifest, validate_manifest, Case, Set, Tier};
use research_tools::idea_queries::{first_queries_for, FirstQueryOptions, PlannedQuery, QueryService};
use research_tools::idea_terms::extract as extract_idea_terms;
use research_tools::library::{Library, SourceKind};
use research_tools::lit_providers::LitProvider;
use research_tools::source_sync::{register_source, sync_source, SyncOptions};
use routed_mock::{RoutedMock, Routes};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

pub fn eval_fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("eval")
}

pub fn set_i_dir() -> PathBuf {
    eval_fixtures_dir().join("i")
}

/// Load and strictly validate `tests/fixtures/eval/manifest-i.json`.
pub fn load_set_i_cases() -> Vec<Case> {
    let text = fs::read_to_string(eval_fixtures_dir().join("manifest-i.json"))
        .expect("read manifest-i.json");
    assert!(!text.contains('\r'), "manifest-i.json must be LF-only");
    let m = parse_manifest(&text).expect("parse manifest-i.json");
    assert_eq!(m.set, Set::I);
    assert_eq!(m.tier, Tier::Fixed);
    let errs = validate_manifest(&m);
    assert!(errs.is_empty(), "manifest-i.json errors: {errs:?}");
    m.cases
}

/// Start `RoutedMock` loaded from `tests/fixtures/eval/i/recordings/set_i_services.json`.
pub fn start_set_i_mock() -> RoutedMock {
    let path = set_i_dir().join("recordings/set_i_services.json");
    let routes = Routes::load(&path).expect("load frozen Set I recording");
    assert_eq!(routes.len(), 70, "expected 70 frozen routes in set_i_services.json");
    RoutedMock::start(routes)
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    let mut entries: Vec<_> = fs::read_dir(src).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&from).unwrap();
        assert!(
            !meta.file_type().is_symlink(),
            "unexpected symlink in fixture `{}`",
            from.display()
        );
        if meta.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

/// Sync a throwaway copy of `tests/fixtures/eval/i/user_library` into `data_dir`'s F10 library
/// via F13a's read-only `FolderAdapter` (`register_source` + `sync_source`).
pub fn sync_eval_user_library(data_dir: &Path, tmp_root: &Path) -> usize {
    let lib_copy = tmp_root.join("user_library");
    copy_dir_recursive(&set_i_dir().join("user_library"), &lib_copy);
    let lib = Library::open(data_dir).expect("open F10 library");
    let src = register_source(
        &lib,
        SourceKind::Folder,
        &lib_copy,
        json!({}),
        Some("eval-i-user-library".to_string()),
    )
    .expect("register F13a folder source");
    let report = sync_source(
        &lib,
        &src.id,
        &SyncOptions { dry_run: false, hash_files: true },
    )
    .expect("sync F13a user library");
    report.total_items
}

/// Build deterministic preview queries (1 arXiv + 1 INSPIRE, both exact Rule P4 preview queries).
pub fn deterministic_first_queries(idea_text: &str) -> Vec<PlannedQuery> {
    let extracted = extract_idea_terms(idea_text);
    let all = first_queries_for(
        &extracted,
        &FirstQueryOptions {
            services: vec![QueryService::Arxiv, QueryService::Inspire],
            max_queries: 6,
            categories: Vec::new(),
        },
    );
    let mut picked = Vec::new();
    for svc in [QueryService::Arxiv, QueryService::Inspire] {
        if let Some(q) = all.iter().find(|p| p.provider == svc) {
            picked.push(q.clone());
        }
    }
    picked
}

/// Build deterministic keyless extended query (Crossref) from `idea_text` without any model call.
pub fn deterministic_crossref_query(idea_text: &str) -> Option<String> {
    let extracted = extract_idea_terms(idea_text);
    first_queries_for(
        &extracted,
        &FirstQueryOptions {
            services: vec![QueryService::Crossref],
            max_queries: 1,
            categories: Vec::new(),
        },
    )
    .into_iter()
    .next()
    .map(|pq| pq.q)
}

/// Build deterministic keyed extended queries (Semantic Scholar + OpenAlex) from `idea_text`
/// without any model call.
pub fn deterministic_keyed_queries(idea_text: &str) -> Vec<(LitProvider, String)> {
    let extracted = extract_idea_terms(idea_text);
    let mut out = Vec::new();
    if let Some(s2) = first_queries_for(
        &extracted,
        &FirstQueryOptions {
            services: vec![QueryService::SemanticScholar],
            max_queries: 1,
            categories: Vec::new(),
        },
    )
    .into_iter()
    .next()
    {
        out.push((LitProvider::SemanticScholar, s2.q.clone()));
        out.push((LitProvider::OpenAlex, s2.q));
    }
    out
}

/// Deterministic scripted model transport for Explore exchanges (`agent::Transport`).
pub struct ScriptedExploreTransport {
    responses: Mutex<VecDeque<ModelResponse>>,
    calls: Mutex<usize>,
}

impl ScriptedExploreTransport {
    pub fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(0),
        }
    }

    pub fn calls_made(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl Transport for ScriptedExploreTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-x14",
            endpoint: "http://127.0.0.1:0/v1/messages".into(),
            api_key_env: "none".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        Ok(PreparedRequest {
            body: serde_json::to_vec(req).unwrap(),
        })
    }

    fn send_prepared(&self, prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        *self.calls.lock().unwrap() += 1;
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                let v: Value = serde_json::from_slice(&prepared.body).unwrap_or(Value::Null);
                let msgs = v.get("messages").cloned().unwrap_or(Value::Null);
                panic!("scripted transport exhausted; messages: {msgs}");
            }))
    }
}

pub fn scripted_tool_response(id: &str, name: &str, input: Value) -> ModelResponse {
    ModelResponse {
        raw_body: serde_json::to_vec(&json!({ "id": id, "name": name, "input": input })).unwrap(),
        id: Some(format!("msg-{id}")),
        model: "scripted-explore-eval-model".into(),
        stop_reason: StopReason::ToolUse,
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        usage: json!({ "input_tokens": 180, "output_tokens": 90 }),
    }
}
