use super::*;
use crate::investigation_markers::{MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED};
use crate::lines_sent::{ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT};
use crate::map_fold::fold_map;
use crate::paper_key::PaperKey;
use crate::skeleton::{build_tex_skeleton, TexSkeletonInput};
use crate::tool_profile::{investigation_v1, investigation_v2, register_profile, ProfileHostTools, MARKER_INVESTIGATION_STARTED};
use crate::{all_research_tool_defs, ingest_bytes, register_research_tools, IngestRequest, ResearchEnv};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tools::{ToolDef, WorkerEpoch};

const SID: &str = "s-map-context";
const CALLER: &str = "main";

const PRIMARY_TEX: &str = r#"\documentclass{article}
\newtheorem{definition}{Definition}
\newtheorem{lemma}{Lemma}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Notation}
Let $\gamma > 0$ denote the spectral gap constant fixed throughout this paper.

\begin{definition}\label{def:kernel}
A kernel $K$ is admissible when $\|K\|_{L^1} \le 1$ and $K$ is symmetric.
\end{definition}

\begin{theorem}\label{thm:decay}
By Theorem~\ref{thm:ext} of \cite{Smith24} and Definition~\ref{def:kernel}, the flow decays at rate $\gamma$.
\end{theorem}
\begin{proof}
Combining admissibility with the external contraction estimate finishes the bound.
\end{proof}

\begin{lemma}\label{lem:other}
An unrelated stability estimate holds for bounded perturbations.
\end{lemma}

\begin{thebibliography}{9}
\bibitem{Smith24} J. Smith, Uniform contraction for coupled flows, 2024.
\bibitem{Uncited99} A. Uncited, Background note without lookup, 1999.
\end{thebibliography}
\end{document}
"#;

const EXTERNAL_TEX: &str = r#"\documentclass{article}
\newtheorem{lemma}{Lemma}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{External Results}
\begin{lemma}\label{lem:ext_aux}
Every bounded potential satisfies a local coercivity inequality.
\end{lemma}

\begin{theorem}\label{thm:ext}
Assume the spectral gap satisfies $\lambda_1 \ge \gamma > 0$ and the boundary weight is continuous.
Then the semigroup decays exponentially in $L^2$.
\end{theorem}
\begin{proof}
Follows from Gronwall's inequality on energy norm spheres.
\end{proof}
\end{document}
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    env: ResearchEnv,
    ledger: Ledger,
    source_id: String,
    path: String,
    file_sha256: String,
    skeleton_seq: u64,
    skeleton: PaperSkeleton,
    ext_source_id: String,
    ext_path: String,
    ext_sha256: String,
    ext_skeleton: PaperSkeleton,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(tmp.path());
        let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
        let ing = ingest_bytes(
            &env.sandbox(),
            &db,
            &IngestRequest {
                session_id: SID,
                file_name: "paper.tex",
                origin: "primary fixture",
                raw_bytes: PRIMARY_TEX.as_bytes(),
                source_id_hint: Some("src1"),
                section_filter: None,
                title: Some("Primary Context Fixture"),
            },
        )
        .unwrap();
        let ext_ing = ingest_bytes(
            &env.sandbox(),
            &db,
            &IngestRequest {
                session_id: SID,
                file_name: "smith24.tex",
                origin: "arxiv:2401.00042v1",
                raw_bytes: EXTERNAL_TEX.as_bytes(),
                source_id_hint: Some("src_ext"),
                section_filter: None,
                title: Some("External Cited Fixture"),
            },
        )
        .unwrap();
        let ledger = db.into_inner().unwrap();

        let path = ing.primary_path.unwrap();
        let file_sha256 = ledger::sha256_hex(PRIMARY_TEX.as_bytes());
        let mut files = BTreeMap::new();
        files.insert(path.clone(), PRIMARY_TEX.to_string());
        let skeleton = build_tex_skeleton(&TexSkeletonInput {
            source_id: ing.source_id.clone(),
            source_seq: ing.event_seq,
            paper_key: PaperKey::parse("arxiv:2501.00001").unwrap(),
            version: Some("v1".into()),
            files,
            primary_path: Some(path.clone()),
            files_sha256: None,
        })
        .skeleton;

        let ext_path = ext_ing.primary_path.unwrap();
        let ext_sha256 = ledger::sha256_hex(EXTERNAL_TEX.as_bytes());
        let mut ext_files = BTreeMap::new();
        ext_files.insert(ext_path.clone(), EXTERNAL_TEX.to_string());
        let ext_skeleton = build_tex_skeleton(&TexSkeletonInput {
            source_id: ext_ing.source_id.clone(),
            source_seq: ext_ing.event_seq,
            paper_key: PaperKey::parse("arxiv:2401.00042").unwrap(),
            version: Some("v1".into()),
            files: ext_files,
            primary_path: Some(ext_path.clone()),
            files_sha256: None,
        })
        .skeleton;

        let blob = ledger.put_blob(&serde_json::to_vec(&skeleton).unwrap()).unwrap();
        ledger.append(SID, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
        ledger.append(SID, "investigation.paper", 1, &json!({ "source_id": ing.source_id })).unwrap();
        let skeleton_seq = ledger
            .append(
                SID,
                EVENT_INVESTIGATION_SKELETON,
                1,
                &json!({ "source_id": ing.source_id, "source_seq": ing.event_seq, "blob": blob }),
            )
            .unwrap();

        Self {
            _tmp: tmp,
            env,
            ledger,
            source_id: ing.source_id,
            path,
            file_sha256,
            skeleton_seq,
            skeleton,
            ext_source_id: ext_ing.source_id,
            ext_path,
            ext_sha256,
            ext_skeleton,
        }
    }

    fn tag_by_label(skeleton: &PaperSkeleton, label: &str) -> String {
        skeleton
            .nodes
            .iter()
            .find(|n| n.label.as_deref() == Some(label))
            .and_then(|n| n.tag.clone())
            .unwrap_or_else(|| panic!("missing label {label}"))
    }

    fn line_in(src: &str, needle: &str) -> usize {
        src.lines()
            .enumerate()
            .find(|(_, l)| l.contains(needle))
            .map(|(i, _)| i + 1)
            .unwrap_or_else(|| panic!("missing line `{needle}`"))
    }

    fn start_task(&self, task: &str, tags: &[&str]) -> u64 {
        self.ledger
            .append(
                SID,
                MARKER_TASK_REQUESTED,
                1,
                &json!({
                    "task": task,
                    "tags": tags,
                    "source_id": self.source_id,
                    "skeleton_seq": self.skeleton_seq,
                }),
            )
            .unwrap()
    }

    fn supply_both_sources(&self, req_seq: u64, caller: &str) {
        let payload = ContextSuppliedPayload::new(
            req_seq.to_string(),
            caller,
            vec![
                SuppliedRange {
                    source_id: self.source_id.clone(),
                    path: self.path.clone(),
                    file_sha256: self.file_sha256.clone(),
                    start_line: 1,
                    end_line: PRIMARY_TEX.lines().count(),
                },
                SuppliedRange {
                    source_id: self.ext_source_id.clone(),
                    path: self.ext_path.clone(),
                    file_sha256: self.ext_sha256.clone(),
                    start_line: 1,
                    end_line: EXTERNAL_TEX.lines().count(),
                },
            ],
        )
        .unwrap();
        self.ledger
            .append(SID, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(&payload).unwrap())
            .unwrap();
    }

    fn record_cited_lookup(&self, target_tag: &str, bib_key: &str, ext_tag: Option<&str>, status: &str) {
        self.ledger
            .append(
                SID,
                EVENT_INVESTIGATION_CITED_LOOKUP,
                1,
                &json!({
                    "tag": target_tag,
                    "bib_key": bib_key,
                    "resolved": { "paper_key": "arxiv:2401.00042", "version": "v1" },
                    "external_source_id": self.ext_source_id,
                    "external_tag": ext_tag,
                    "status": status,
                }),
            )
            .unwrap();
    }

    fn primary_anchor(&self, line: usize, quote: &str) -> Value {
        json!({
            "source_id": self.source_id,
            "path": self.path,
            "start_line": line,
            "end_line": line,
            "quote": quote,
        })
    }

    fn ext_anchor(&self, line: usize, quote: &str) -> Value {
        json!({
            "source_id": self.ext_source_id,
            "path": self.ext_path,
            "start_line": line,
            "end_line": line,
            "quote": quote,
        })
    }
}

#[test]
fn gate_rejects_app_only_fields_wrong_task_and_out_of_scope_tag() {
    let f = Fixture::new();
    let thm_tag = Fixture::tag_by_label(&f.skeleton, "thm:decay");
    let lem_tag = Fixture::tag_by_label(&f.skeleton, "lem:other");

    let map_req = f.start_task("orient", &[]);
    let err = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": map_req,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [{ "type": "prerequisite", "title": "T", "body": "Short.", "anchors": [] }]
        }),
    )
    .unwrap_err();
    assert!(err.contains("context") || err.contains("open"), "{err}");

    let ctx_req = f.start_task("context", &[&thm_tag]);
    // Caller must not pass `origin` or `excerpt` / `file_sha256`.
    let origin_err = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": ctx_req,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [{
                "type": "prerequisite",
                "title": "T",
                "body": "Short.",
                "origin": "paper",
                "anchors": []
            }]
        }),
    )
    .unwrap_err();
    assert!(origin_err.contains("origin"), "{origin_err}");

    // Out-of-scope tag on context task fails closed (rule 6.4).
    let scope_err = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": ctx_req,
            "source_id": f.source_id,
            "tag": lem_tag,
            "cards": [{ "type": "prerequisite", "title": "T", "body": "Short.", "anchors": [] }]
        }),
    )
    .unwrap_err();
    assert!(scope_err.contains("outside the scope"), "{scope_err}");

    // Finished task closes marker gate (rule 6.3).
    f.ledger
        .append(
            SID,
            MARKER_TASK_FINISHED,
            1,
            &json!({ "for_request_seq": ctx_req, "task": "context", "status": "done" }),
        )
        .unwrap();
    let closed_err = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": ctx_req,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [{ "type": "prerequisite", "title": "T", "body": "Short.", "anchors": [] }]
        }),
    )
    .unwrap_err();
    assert!(closed_err.contains("closed") || closed_err.contains("marker"), "{closed_err}");
}

#[test]
fn rule_6_8_provenance_never_downgrades_and_enforces_model_background_limits() {
    let f = Fixture::new();
    let thm_tag = Fixture::tag_by_label(&f.skeleton, "thm:decay");
    let req_seq = f.start_task("context", &[&thm_tag]);
    f.supply_both_sources(req_seq, CALLER);

    let sym_line = Fixture::line_in(PRIMARY_TEX, "spectral gap constant");
    let def_line = Fixture::line_in(PRIMARY_TEX, "admissible when");
    let ext_line = Fixture::line_in(EXTERNAL_TEX, "semigroup decays");

    // Mix valid paper, valid external, valid model_background, and failing anchors.
    let res = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [
                {
                    "type": "symbol",
                    "title": "Spectral gap constant gamma",
                    "body": "The constant gamma > 0 is fixed in Section 1.",
                    "anchors": [f.primary_anchor(sym_line, "spectral gap constant fixed throughout")]
                },
                {
                    "type": "prerequisite",
                    "title": "Admissible kernel definition",
                    "body": &"G".repeat(350),
                    "anchors": [f.primary_anchor(def_line, "is admissible when")]
                },
                {
                    "type": "prerequisite",
                    "title": "External contraction note",
                    "body": "Uses semigroup decay from external paper.",
                    "anchors": [f.ext_anchor(ext_line, "semigroup decays exponentially")]
                },
                {
                    "type": "prerequisite",
                    "title": "Valid model background",
                    "body": "Standard Gronwall setup.",
                    "anchors": []
                },
                {
                    "type": "prerequisite",
                    "title": "Bad anchor MUST NOT downgrade",
                    "body": "Short enough for bg.",
                    "anchors": [f.primary_anchor(sym_line, "completely fabricated quote")]
                },
                {
                    "type": "prerequisite",
                    "title": "Too long model background",
                    "body": &"x".repeat(MAX_MODEL_BACKGROUND_BODY_CHARS + 1),
                    "anchors": []
                },
                {
                    "type": "symbol",
                    "title": "Unsourced symbol forbidden",
                    "body": "Missing anchor.",
                    "anchors": []
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(res["status"], "recorded", "{res}");
    let accepted = res["accepted"].as_array().unwrap();
    assert_eq!(accepted.len(), 4, "{res}");
    assert_eq!(accepted[0]["origin"], "paper");
    assert_eq!(accepted[1]["origin"], "paper");
    assert_eq!(accepted[2]["origin"], "external");
    assert_eq!(accepted[3]["origin"], "model_background");

    let rejected = res["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 3);
    assert_eq!(rejected[0]["index"], 4);
    assert!(rejected[0]["true_text"].as_str().unwrap().contains("spectral gap"));
    assert_eq!(rejected[1]["code"], "model_background_too_long");
    assert_eq!(rejected[2]["code"], "missing_anchors");

    // 4 more valid model_background cards fit (#2..#5), #6 is rejected across separate call.
    let res2 = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [
                { "type": "prerequisite", "title": "BG2", "body": "B2.", "anchors": [] },
                { "type": "prerequisite", "title": "BG3", "body": "B3.", "anchors": [] },
                { "type": "prerequisite", "title": "BG4", "body": "B4.", "anchors": [] },
                { "type": "prerequisite", "title": "BG5", "body": "B5.", "anchors": [] },
                { "type": "prerequisite", "title": "BG6 over limit", "body": "B6.", "anchors": [] }
            ]
        }),
    )
    .unwrap();
    assert_eq!(res2["accepted"].as_array().unwrap().len(), 4);
    assert_eq!(res2["rejected"].as_array().unwrap().len(), 1);
    assert_eq!(res2["rejected"][0]["code"], "model_background_limit_exceeded");
}

#[test]
fn rule_6_9_cited_result_checks_lookup_and_external_node_containment() {
    let f = Fixture::new();
    let thm_tag = Fixture::tag_by_label(&f.skeleton, "thm:decay");
    let ext_thm_tag = Fixture::tag_by_label(&f.ext_skeleton, "thm:ext");
    let req_seq = f.start_task("context", &[&thm_tag]);
    f.supply_both_sources(req_seq, CALLER);

    // Record a candidates-only lookup for Uncited99 (not located) and found lookup for Smith24.
    f.record_cited_lookup("n-fake", "Uncited99", None, "candidates");
    f.record_cited_lookup(&thm_tag, "Smith24", Some(&ext_thm_tag), CITED_STATUS_FOUND);

    let ext_lem_line = Fixture::line_in(EXTERNAL_TEX, "local coercivity");
    let ext_thm_h_line = Fixture::line_in(EXTERNAL_TEX, "boundary weight is continuous");
    let ext_thm_c_line = Fixture::line_in(EXTERNAL_TEX, "semigroup decays exponentially");
    let pri_thm_line = Fixture::line_in(PRIMARY_TEX, "external contraction estimate");

    let res = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "tag": thm_tag,
            "cards": [
                {
                    "type": "cited_result",
                    "title": "Valid Smith24 Theorem",
                    "body": "States exponential semigroup decay under continuous boundary weight.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially in $L^2$.")],
                    "hypotheses": [
                        {
                            "text": "Spectral gap lower bound & continuous weight",
                            "anchor": f.ext_anchor(ext_thm_h_line, "boundary weight is continuous")
                        }
                    ]
                },
                {
                    "type": "cited_result",
                    "title": "Missing lookup",
                    "body": "Uncited99 is candidates-only.",
                    "bib_key": "Uncited99",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially")]
                },
                {
                    "type": "cited_result",
                    "title": "Wrong external node",
                    "body": "Anchored in ext lemma instead of ext theorem.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_lem_line, "local coercivity inequality")]
                },
                {
                    "type": "cited_result",
                    "title": "Hypothesis outside located node",
                    "body": "Main anchor ok, hypothesis in wrong node.",
                    "bib_key": "Smith24",
                    "anchors": [f.ext_anchor(ext_thm_c_line, "semigroup decays exponentially")],
                    "hypotheses": [
                        {
                            "text": "Wrong place",
                            "anchor": f.ext_anchor(ext_lem_line, "local coercivity inequality")
                        }
                    ]
                },
                {
                    "type": "cited_result",
                    "title": "Primary-only anchor",
                    "body": "Primary anchor without external node anchor.",
                    "bib_key": "Smith24",
                    "anchors": [f.primary_anchor(pri_thm_line, "external contraction estimate")]
                }
            ]
        }),
    )
    .unwrap();

    assert_eq!(res["status"], "recorded");
    let accepted = res["accepted"].as_array().unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0]["origin"], "external");
    assert_eq!(accepted[0]["bib_key"], "Smith24");
    assert_eq!(accepted[0]["external_source_id"], f.ext_source_id);
    assert_eq!(accepted[0]["external_tag"], ext_thm_tag);

    let rejected = res["rejected"].as_array().unwrap();
    assert_eq!(rejected.len(), 4);
    assert_eq!(rejected[0]["code"], "cited_result_not_located");
    assert_eq!(rejected[1]["code"], "anchor_outside_cited_node");
    assert_eq!(rejected[2]["code"], "hypothesis_outside_cited_node");
    assert_eq!(rejected[3]["code"], "anchor_outside_cited_node");
}

#[test]
fn open_task_allows_context_cards_on_model_child_and_fold_map_projects_them() {
    let f = Fixture::new();
    let thm_tag = Fixture::tag_by_label(&f.skeleton, "thm:decay");
    let req_seq = f.start_task("open", &[&thm_tag]);
    f.supply_both_sources(req_seq, CALLER);

    // Record a real `m-` child step node under `thm_tag` via `record_map_reading`.
    let pri_line = Fixture::line_in(PRIMARY_TEX, "flow decays at rate");
    let rec = crate::map_reading::record_map_reading(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "items": [{
                "new": { "id": "hyp_1", "attach_to": thm_tag, "kind": "step" },
                "gist": "Applies external estimate.",
                "anchors": [f.primary_anchor(pri_line, "flow decays at rate")]
            }],
            "edges": [],
            "limitations": [],
            "done": false
        }),
    )
    .unwrap();
    let m_tag = rec["items"][0]["tag"].as_str().unwrap().to_string();

    let sym_line = Fixture::line_in(PRIMARY_TEX, "spectral gap constant");
    let out = record_context_cards(
        &f.env,
        &f.ledger,
        SID,
        CALLER,
        &json!({
            "request_seq": req_seq,
            "source_id": f.source_id,
            "tag": m_tag,
            "cards": [{
                "type": "symbol",
                "title": "Gamma symbol on child step",
                "body": "Fixed positive spectral gap constant.",
                "anchors": [f.primary_anchor(sym_line, "spectral gap constant")]
            }]
        }),
    )
    .unwrap();
    assert_eq!(out["status"], "recorded");

    let events = f.ledger.read_session(SID).unwrap();
    let folded = fold_map(&f.skeleton, &events, false);
    let child_fold = folded.nodes.get(&m_tag).unwrap();
    assert_eq!(child_fold.context_cards.len(), 1);
    assert!(!child_fold.context_cards[0].stale);
    assert_eq!(child_fold.context_cards[0].value.origin, ContextCardOrigin::Paper);
    assert_eq!(folded.tasks[0].saved_items, 2); // 1 reading + 1 context card
}

struct DummyEpoch;
impl WorkerEpoch for DummyEpoch {
    fn live_epoch(&self) -> Option<u64> { None }
    fn is_contained(&self) -> bool { false }
}

#[test]
fn record_context_cards_is_never_in_research_tools_and_is_registered_on_investigation_v2() {
    assert!(!all_research_tool_defs().iter().any(|d: &ToolDef| d.id == RECORD_CONTEXT_CARDS));
    let mut r_reg = Registry::new();
    let tmp = tempfile::tempdir().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    register_research_tools(&mut r_reg, Arc::clone(&env));
    assert!(r_reg.get(RECORD_CONTEXT_CARDS).is_none());

    let stub = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
    let mut inv2_reg = Registry::new();
    register_profile(
        &investigation_v2(),
        &mut inv2_reg,
        Arc::clone(&env),
        ProfileHostTools::new(Arc::new(DummyEpoch), stub.clone()),
    )
    .unwrap();
    assert!(inv2_reg.get(RECORD_CONTEXT_CARDS).is_some());

    let mut inv1_reg = Registry::new();
    register_profile(
        &investigation_v1(),
        &mut inv1_reg,
        env,
        ProfileHostTools::new(Arc::new(DummyEpoch), stub),
    )
    .unwrap();
    assert!(inv1_reg.get(RECORD_CONTEXT_CARDS).is_none());
}
