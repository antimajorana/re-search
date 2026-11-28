use super::*;
use crate::investigation_markers::{MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED};
use crate::lines_sent::{ContextSuppliedPayload, SuppliedRange, CONTEXT_SUPPLIED_EVENT};
use crate::paper_key::PaperKey;
use crate::skeleton::{build_tex_skeleton, SkNodeKind, TexSkeletonInput};
use crate::tool_profile::MARKER_INVESTIGATION_STARTED;
use crate::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;

const SID: &str = "s-map-reading";
const CALLER: &str = "main";
const WRONG_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

const SAMPLE_TEX: &str = r#"\documentclass{article}
\newtheorem{theorem}{Theorem}
\newtheorem{lemma}{Lemma}
\newtheorem{definition}{Definition}
\newtheorem{remark}{Remark}
\newtheorem{example}{Example}
\begin{document}
\section{Introduction}
We prove a sharp bound on finite lattice spectra following \cite{NS06}.
This prose passage connects our argument to standard spectral methods.

\section{Main Results}
\begin{definition}\label{def:decay}
A function $f$ has rapid decay if $f(r) \le C (1+r)^{-4}$ for all $r \ge 0$.
\end{definition}

\begin{lemma}\label{lem:helper}
Every rapidly decaying kernel satisfies the uniform commutator estimate.
\end{lemma}

\begin{theorem}\label{thm:main}
Under Definition~\ref{def:decay}, the spectral flow preserves rapid decay.
\end{theorem}
\begin{proof}
Applying Lemma~\ref{lem:helper} along the adiabatic path yields the claimed decay bound.
\end{proof}

\begin{remark}\label{rem:note}
The constant in the estimate depends only on the spatial dimension.
\end{remark}

\begin{example}\label{ex:toy}
For a one-dimensional chain with nearest-neighbour couplings, the bound holds with $C=2$.
\end{example}

\begin{figure}
\caption{Illustration of the decay envelope.}\label{fig:one}
\end{figure}

\begin{table}
\caption{Summary of decay exponents.}\label{tab:one}
\end{table}

\section{Auxiliary}
\begin{theorem}\label{thm:other}
An unrelated auxiliary identity holds for free bosonic oscillators.
\end{theorem}

\begin{thebibliography}{9}
\bibitem{NS06} B. Nachtergaele and R. Sims, Lieb-Robinson bounds, 2006.
\end{thebibliography}
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
                session_id: SID, file_name: "paper.tex", origin: "unit fixture",
                raw_bytes: SAMPLE_TEX.as_bytes(), source_id_hint: Some("src1"),
                section_filter: None, title: Some("Synthetic Spectral Flow"),
            },
        )
        .unwrap();
        let ledger = db.into_inner().unwrap();
        let path = ing.primary_path.unwrap();
        let file_sha256 = ledger::sha256_hex(SAMPLE_TEX.as_bytes());
        let mut files = BTreeMap::new();
        files.insert(path.clone(), SAMPLE_TEX.to_string());
        let skeleton = build_tex_skeleton(&TexSkeletonInput {
            source_id: ing.source_id.clone(), source_seq: ing.event_seq,
            paper_key: PaperKey::parse("arxiv:2501.00001").unwrap(), version: Some("v1".into()),
            files, primary_path: Some(path.clone()), files_sha256: None,
        })
        .skeleton;
        let blob = ledger.put_blob(&serde_json::to_vec(&skeleton).unwrap()).unwrap();
        ledger.append(SID, MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
        ledger.append(SID, "investigation.paper", 1, &json!({ "source_id": ing.source_id })).unwrap();
        let skeleton_seq = ledger
            .append(SID, EVENT_INVESTIGATION_SKELETON, 1, &json!({
                "source_id": ing.source_id, "source_seq": ing.event_seq, "blob": blob
            }))
            .unwrap();
        Self { _tmp: tmp, env, ledger, source_id: ing.source_id, path, file_sha256, skeleton_seq, skeleton }
    }

    fn tag_by_label(&self, label: &str) -> String {
        self.skeleton.nodes.iter()
            .find(|n| n.label.as_deref() == Some(label))
            .and_then(|n| n.tag.clone())
            .unwrap_or_else(|| panic!("missing label {label}"))
    }

    fn tag_by_kind(&self, kind: SkNodeKind) -> String {
        self.skeleton.nodes.iter()
            .find(|n| n.kind == kind)
            .and_then(|n| n.tag.clone())
            .unwrap_or_else(|| panic!("missing kind {kind:?}"))
    }

    fn line_of(&self, needle: &str) -> usize {
        SAMPLE_TEX.lines().enumerate()
            .find(|(_, l)| l.contains(needle))
            .map(|(i, _)| i + 1)
            .unwrap_or_else(|| panic!("missing line `{needle}`"))
    }

    fn start_task(&self, task: &str, tags: &[&str]) -> u64 {
        self.ledger.append(SID, MARKER_TASK_REQUESTED, 1, &json!({
            "task": task, "tags": tags, "source_id": self.source_id, "skeleton_seq": self.skeleton_seq,
        })).unwrap()
    }

    fn supply_all_lines(&self, req_seq: u64, caller: &str, hash: &str) {
        let payload = ContextSuppliedPayload::new(req_seq.to_string(), caller, vec![SuppliedRange {
            source_id: self.source_id.clone(), path: self.path.clone(),
            file_sha256: hash.to_string(), start_line: 1, end_line: SAMPLE_TEX.lines().count(),
        }]).unwrap();
        self.ledger.append(SID, CONTEXT_SUPPLIED_EVENT, 1, &serde_json::to_value(&payload).unwrap()).unwrap();
    }

    fn anchor(&self, line: usize, quote: &str) -> Value {
        json!({ "source_id": self.source_id, "path": self.path, "start_line": line, "end_line": line, "quote": quote })
    }

    fn call_args(&self, req_seq: u64, items: Value, edges: Value, limitations: Value, done: bool) -> Value {
        json!({
            "request_seq": req_seq, "source_id": self.source_id,
            "items": items, "edges": edges, "limitations": limitations, "done": done,
        })
    }
}

#[test]
fn mixed_validity_saves_valid_siblings_returns_true_text_and_allows_multiple_calls() {
    let fx = Fixture::new();
    let req_seq = fx.start_task("orient", &[]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);

    let (thm_tag, lem_tag) = (fx.tag_by_label("thm:main"), fx.tag_by_label("lem:helper"));
    let thm_line = fx.line_of("spectral flow preserves rapid decay");
    let def_line = fx.line_of("rapid decay if");
    let lem_line = fx.line_of("uniform commutator estimate");

    let args = fx.call_args(req_seq, json!([
        {
            "tag": thm_tag, "role": "main_result",
            "gist": "The spectral flow preserves rapid decay under Definition 1.",
            "purpose": "Establishes the primary quasi-locality theorem.",
            "hypotheses": [{ "text": "Rapid decay on f.", "anchor": fx.anchor(def_line, "function $f$ has rapid decay") }],
            "anchors": [
                fx.anchor(thm_line, "spectral flow preserves rapid decay"),
                fx.anchor(thm_line, "nonexistent fabricated sibling quote here")
            ],
            "authors_claim": { "label": "new", "relation_note": "Extends NS06.", "attributed_to": ["NS06"] }
        },
        {
            "tag": lem_tag, "role": "technical", "gist": "Bad quote item.",
            "anchors": [fx.anchor(lem_line, "this quote does not exist anywhere in file")]
        }
    ]), json!([]), json!(["Section 3 not yet inspected."]), false);

    let res = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &args).unwrap();
    assert_eq!(res["status"], "recorded");
    assert!(res["reading_seq"].as_u64().is_some());
    assert_eq!(res["accepted_items"], 1);
    assert_eq!(res["items"][0]["status"], "accepted");
    assert_eq!(res["items"][1]["status"], "rejected");
    assert_eq!(res["items"][1]["code"], "quote_not_found");
    assert!(res["items"][1]["true_text"].as_str().unwrap().contains("uniform commutator estimate"));
    assert_eq!(res["rejected_items"].as_array().unwrap().len(), 1);
    assert_eq!(res["anchor_report"]["dropped"].as_array().unwrap().len(), 1);

    let events = fx.ledger.read_session(SID).unwrap();
    let rec_ev = events.iter().find(|e| e.kind == EVENT_INVESTIGATION_READING).unwrap();
    let stored: MapReadingEventPayload = serde_json::from_value(rec_ev.payload.clone()).unwrap();
    assert_eq!(stored.items.len(), 1);
    assert!(stored.items[0].authors_claim.as_ref().unwrap().cues.is_empty());
    let expected_fp = fx.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&thm_tag)).unwrap().fp.clone();
    assert_eq!(stored.items[0].node_fp, expected_fp);

    // Second call for the same request_seq succeeds (done-only completion).
    let res2 = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, json!([]), json!([]), json!(["Done orienting."]), true),
    ).unwrap();
    assert_eq!(res2["status"], "recorded");
    assert_ne!(res2["reading_seq"], res["reading_seq"]);

    // All-bad call appends nothing.
    let before = fx.ledger.read_session(SID).unwrap().len();
    let res3 = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, json!([{
            "tag": lem_tag, "gist": "Still bad.",
            "anchors": [fx.anchor(lem_line, "definitely not present in source text")]
        }]), json!([]), json!([]), true),
    ).unwrap();
    assert_eq!(res3["status"], "no_valid_records");
    assert!(res3["reading_seq"].is_null());
    assert_eq!(fx.ledger.read_session(SID).unwrap().len(), before);
}

#[test]
fn containment_role_and_authors_claim_rules_enforced() {
    let fx = Fixture::new();
    let req_seq = fx.start_task("orient", &[]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);

    let thm_tag = fx.tag_by_label("thm:main");
    let lem_line = fx.line_of("uniform commutator estimate");
    let thm_line = fx.line_of("spectral flow preserves rapid decay");

    // 1) Anchor outside node spans -> anchor_outside_node.
    let r = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req_seq, json!([{
        "tag": thm_tag, "gist": "Outside anchor.", "anchors": [fx.anchor(lem_line, "uniform commutator estimate")]
    }]), json!([]), json!([]), false)).unwrap();
    assert_eq!(r["rejected_items"][0]["code"], "anchor_outside_node");

    // 2) main_result refused on Proof, Remark, Example, Figure, Table, Bibliography.
    let forbidden = [
        SkNodeKind::Proof, SkNodeKind::Remark, SkNodeKind::Example,
        SkNodeKind::Figure, SkNodeKind::Table, SkNodeKind::Bibliography,
    ];
    let items: Vec<Value> = forbidden.iter().map(|&k| {
        let tag = fx.tag_by_kind(k);
        let node = fx.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&tag)).unwrap();
        let line = node.spans[0].start_line;
        let txt = SAMPLE_TEX.lines().nth(line - 1).unwrap().trim();
        json!({ "tag": tag, "role": "main_result", "anchors": [fx.anchor(line, txt)] })
    }).collect();
    let r_roles = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, Value::Array(items), json!([]), json!([]), false),
    ).unwrap();
    assert_eq!(r_roles["accepted_items"], 0);
    assert_eq!(r_roles["rejected_items"].as_array().unwrap().len(), 6);
    for rej in r_roles["rejected_items"].as_array().unwrap() {
        assert_eq!(rej["code"], "invalid_role_for_kind");
    }

    // 3) authors_claim: unknown_bib_key, missing_authors_claim_anchor, and not_stated without anchor.
    let r_ac = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req_seq, json!([
        {
            "tag": thm_tag,
            "authors_claim": { "label": "restated", "attributed_to": ["MISSING99"], "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")] }
        },
        { "tag": thm_tag, "authors_claim": { "label": "new" } },
        { "tag": thm_tag, "authors_claim": { "label": "not_stated" } }
    ]), json!([]), json!([]), false)).unwrap();
    assert_eq!(r_ac["accepted_items"], 1);
    assert_eq!(r_ac["items"][0]["code"], "unknown_bib_key");
    assert_eq!(r_ac["items"][1]["code"], "missing_authors_claim_anchor");
    assert_eq!(r_ac["items"][2]["status"], "accepted");
}

#[test]
fn edge_role_containment_proof_statement_argument_and_self_edge() {
    let fx = Fixture::new();
    let req_seq = fx.start_task("orient", &[]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);

    let (def_tag, lem_tag, thm_tag) = (fx.tag_by_label("def:decay"), fx.tag_by_label("lem:helper"), fx.tag_by_label("thm:main"));
    let stmt_line = fx.line_of("spectral flow preserves rapid decay");
    let proof_line = fx.line_of("along the adiabatic path");
    let intro_line = fx.line_of("connects our argument to standard spectral");

    let edges = json!([
        { "from": lem_tag, "to": thm_tag, "role": "proof", "basis": "explicit", "reason": "In proof.", "anchor": fx.anchor(proof_line, "along the adiabatic path yields") },
        { "from": lem_tag, "to": thm_tag, "role": "proof", "basis": "explicit", "reason": "Moved out of proof.", "anchor": fx.anchor(stmt_line, "spectral flow preserves rapid decay") },
        { "from": def_tag, "to": thm_tag, "role": "statement", "basis": "explicit", "reason": "In statement.", "anchor": fx.anchor(stmt_line, "spectral flow preserves rapid decay") },
        { "from": def_tag, "to": thm_tag, "role": "statement", "basis": "explicit", "reason": "Moved into proof.", "anchor": fx.anchor(proof_line, "along the adiabatic path yields") },
        { "from": def_tag, "to": thm_tag, "role": "argument", "basis": "inferred", "reason": "In intro.", "anchor": fx.anchor(intro_line, "connects our argument to standard spectral") },
        { "from": thm_tag, "to": thm_tag, "role": "argument", "basis": "inferred", "reason": "Self edge.", "anchor": fx.anchor(stmt_line, "spectral flow preserves rapid decay") }
    ]);
    let res = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req_seq, json!([]), edges, json!([]), false)).unwrap();
    assert_eq!(res["accepted_edges"], 3);
    assert_eq!(res["edges"][0]["status"], "accepted");
    assert_eq!(res["edges"][1]["code"], "scope_mismatch");
    assert_eq!(res["edges"][2]["status"], "accepted");
    assert_eq!(res["edges"][3]["code"], "scope_mismatch");
    assert_eq!(res["edges"][4]["status"], "accepted");
    assert_eq!(res["edges"][5]["code"], "self_edge");
}

#[test]
fn lines_sent_enforced_for_context_supplied_wrong_hash_caller_and_read_map_node() {
    let fx = Fixture::new();
    let req_seq = fx.start_task("orient", &[]);
    let thm_tag = fx.tag_by_label("thm:main");
    let thm_line = fx.line_of("spectral flow preserves rapid decay");

    let call = |caller: &str| {
        record_map_reading(&fx.env, &fx.ledger, SID, caller, &fx.call_args(req_seq, json!([{
            "tag": thm_tag, "gist": "Checks lines sent.", "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")]
        }]), json!([]), json!([]), false)).unwrap()
    };

    // Wrong hash in context.supplied -> lines_not_sent.
    fx.supply_all_lines(req_seq, CALLER, WRONG_HASH);
    assert_eq!(call(CALLER)["rejected_items"][0]["code"], "lines_not_sent");

    // Different caller -> lines_not_sent.
    fx.supply_all_lines(req_seq, "other_caller", &fx.file_sha256);
    assert_eq!(call(CALLER)["rejected_items"][0]["code"], "lines_not_sent");

    // Credited via read_map_node result -> accepted.
    fx.ledger.append(SID, "tool.request", 1, &json!({
        "call_id": "c1", "tool": "read_map_node", "caller": CALLER, "run_id": req_seq
    })).unwrap();
    fx.ledger.append(SID, "tool.result", 1, &json!({
        "call_id": "c1", "ok": true,
        "result": {
            "source_id": fx.source_id,
            "files": [{ "path": fx.path, "file_sha256": fx.file_sha256, "start_line": thm_line, "end_line": thm_line }]
        }
    })).unwrap();
    let ok = call(CALLER);
    assert_eq!(ok["status"], "recorded");
    assert_eq!(ok["accepted_items"], 1);
}

#[test]
fn open_task_scope_unknown_tag_new_node_app_only_and_marker_gate() {
    let fx = Fixture::new();
    let (thm_tag, lem_tag, def_tag, other_tag) = (
        fx.tag_by_label("thm:main"), fx.tag_by_label("lem:helper"),
        fx.tag_by_label("def:decay"), fx.tag_by_label("thm:other"),
    );
    let proof_tag = fx.tag_by_kind(SkNodeKind::Proof);

    let req_seq = fx.start_task("open", &[&thm_tag]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);
    let thm_line = fx.line_of("spectral flow preserves rapid decay");
    let proof_line = fx.line_of("along the adiabatic path");
    let other_line = fx.line_of("free bosonic oscillators");
    let def_line = fx.line_of("rapid decay if");

    let args = fx.call_args(req_seq, json!([
        { "tag": thm_tag, "gist": "In scope.", "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")] },
        { "tag": proof_tag, "gist": "Attached proof in scope.", "anchors": [fx.anchor(proof_line, "along the adiabatic path yields")] },
        { "tag": def_tag, "gist": "Edge-scope only.", "anchors": [fx.anchor(def_line, "function $f$ has rapid decay")] },
        { "tag": other_tag, "gist": "Unrelated.", "anchors": [fx.anchor(other_line, "unrelated auxiliary identity")] },
        { "tag": "n-nonexistent", "gist": "Unknown.", "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")] },
        { "new": { "id": "m1", "attach_to": thm_tag, "kind": "step" }, "gist": "V8 node.", "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")] }
    ]), json!([
        { "from": lem_tag, "to": thm_tag, "role": "proof", "basis": "explicit", "reason": "Uses lemma.", "anchor": fx.anchor(proof_line, "along the adiabatic path yields") },
        { "from": other_tag, "to": thm_tag, "role": "argument", "basis": "inferred", "reason": "Out of scope.", "anchor": fx.anchor(other_line, "unrelated auxiliary identity") }
    ]), json!([]), true);
    let res = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &args).unwrap();
    assert_eq!(res["accepted_items"], 3);
    assert_eq!(res["items"][0]["status"], "accepted");
    assert_eq!(res["items"][1]["status"], "accepted");
    assert_eq!(res["items"][2]["code"], "out_of_scope_tag");
    assert_eq!(res["items"][3]["code"], "out_of_scope_tag");
    assert_eq!(res["items"][4]["code"], "unknown_tag");
    assert!(res["items"][4]["message"].as_str().unwrap().contains(&thm_tag));
    assert_eq!(res["items"][5]["status"], "accepted");
    assert_eq!(res["items"][5]["temp_id"], "m1");
    assert!(res["items"][5]["tag"].as_str().unwrap().starts_with("m-"));
    assert_eq!(res["assigned_tags"]["m1"], res["items"][5]["tag"]);
    assert_eq!(res["accepted_edges"], 1);
    assert_eq!(res["edges"][0]["status"], "accepted");
    assert_eq!(res["edges"][1]["code"], "out_of_scope_tag");

    // App-only fields rejected before deserialization.
    for field in APP_ONLY_FIELDS {
        let mut bad = fx.call_args(req_seq, json!([{ "tag": thm_tag }]), json!([]), json!([]), false);
        bad["items"][0][field] = json!("injected");
        let err = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &bad).unwrap_err();
        assert!(err.contains(field), "expected `{field}` rejection, got: {err}");
    }

    // Closed marker gate after task_finished.
    fx.ledger.append(SID, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req_seq, "status": "done" })).unwrap();
    let closed_err = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req_seq, json!([]), json!([]), json!([]), true),
    ).unwrap_err();
    assert!(closed_err.contains("investigation.task_finished"));
}

#[test]
fn open_edge_destination_must_be_in_item_scope_and_empty_done_requires_limitations() {
    let fx = Fixture::new();
    let (thm_tag, lem_tag, def_tag) = (
        fx.tag_by_label("thm:main"),
        fx.tag_by_label("lem:helper"),
        fx.tag_by_label("def:decay"),
    );
    let req_seq = fx.start_task("open", &[&thm_tag]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);
    let lem_line = fx.line_of("uniform commutator estimate");

    // Both `def_tag` and `lem_tag` are in `edge_scope` (dependencies of `thm_tag`),
    // but `lem_tag` is NOT in `item_scope`, so an edge targeting `lem_tag` must be rejected.
    let edge_res = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, json!([]), json!([{
            "from": def_tag, "to": lem_tag, "role": "statement", "basis": "explicit",
            "reason": "Outside item scope target.", "anchor": fx.anchor(lem_line, "uniform commutator estimate")
        }]), json!([]), false),
    ).unwrap();
    assert_eq!(edge_res["status"], "no_valid_records");
    assert_eq!(edge_res["edges"][0]["code"], "out_of_scope_tag");

    // Vacuous `done: true` with zero items, zero edges, and zero limitations must NOT append.
    let empty_done = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, json!([]), json!([]), json!([]), true),
    ).unwrap();
    assert_eq!(empty_done["status"], "no_valid_records");
    assert!(empty_done["reading_seq"].is_null());

    // `done: true` with a documented limitation IS recorded.
    let lim_done = record_map_reading(
        &fx.env, &fx.ledger, SID, CALLER,
        &fx.call_args(req_seq, json!([]), json!([]), json!(["Source omits proof details."]), true),
    ).unwrap();
    assert_eq!(lim_done["status"], "recorded");
    assert!(lim_done["reading_seq"].as_u64().is_some());
}

#[test]
fn v8_model_nodes_stable_across_reruns_arbitrary_depth_and_sibling_scope_isolation() {
    let fx = Fixture::new();
    let thm_tag = fx.tag_by_label("thm:main");
    let proof_line = fx.line_of("along the adiabatic path");
    let other_line = fx.line_of("unrelated auxiliary identity");

    // Run 1: Open thm_tag, create two depth-1 child proof steps.
    let req1 = fx.start_task("open", &[&thm_tag]);
    fx.supply_all_lines(req1, CALLER, &fx.file_sha256);
    let r1 = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req1, json!([
        {
            "new": { "id": "step-1", "attach_to": thm_tag, "kind": "proof_step" },
            "gist": "First pass step 1.",
            "anchors": [fx.anchor(proof_line, "along the adiabatic path yields")]
        },
        {
            "new": { "id": "step-2", "attach_to": thm_tag, "kind": "proof_step" },
            "gist": "Sibling step 2.",
            "anchors": [fx.anchor(proof_line, "the claimed decay bound")]
        },
        {
            "new": { "id": "bad-role", "attach_to": thm_tag, "kind": "proof_step" },
            "role": "main_result",
            "gist": "Forbidden role.",
            "anchors": [fx.anchor(proof_line, "along the adiabatic path yields")]
        },
        {
            "new": { "id": "bad-span", "attach_to": thm_tag, "kind": "proof_step" },
            "gist": "Anchor outside theorem/proof.",
            "anchors": [fx.anchor(other_line, "unrelated auxiliary identity")]
        }
    ]), json!([]), json!([]), true)).unwrap();
    assert_eq!(r1["accepted_items"], 2);
    assert_eq!(r1["items"][2]["code"], "invalid_role_for_kind");
    assert_eq!(r1["items"][3]["code"], "anchor_outside_node");
    let m_step1 = r1["assigned_tags"]["step-1"].as_str().unwrap().to_string();
    let m_step2 = r1["assigned_tags"]["step-2"].as_str().unwrap().to_string();
    assert!(m_step1.starts_with("m-") && m_step2.starts_with("m-") && m_step1 != m_step2);
    fx.ledger.append(SID, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req1, "status": "done" })).unwrap();

    // Run 2: Re-open thm_tag with same quote -> identical m_step1 tag (updates history, no duplicate).
    let req2 = fx.start_task("open", &[&thm_tag]);
    fx.supply_all_lines(req2, CALLER, &fx.file_sha256);
    let r2 = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req2, json!([
        {
            "new": { "id": "rerun-1", "attach_to": thm_tag, "kind": "proof_step" },
            "gist": "Refined step 1 gist.",
            "anchors": [fx.anchor(proof_line, "along the adiabatic path yields")]
        }
    ]), json!([]), json!([]), true)).unwrap();
    assert_eq!(r2["assigned_tags"]["rerun-1"].as_str().unwrap(), m_step1);
    fx.ledger.append(SID, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req2, "status": "done" })).unwrap();

    // Run 3: Open ONLY m_step1 (depth 2): can attach substep to m_step1, CANNOT touch sibling m_step2!
    let req3 = fx.start_task("open", &[&m_step1]);
    fx.supply_all_lines(req3, CALLER, &fx.file_sha256);
    let r3 = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req3, json!([
        {
            "new": { "id": "sub-1a", "attach_to": m_step1, "kind": "substep" },
            "gist": "Depth-2 expansion of step 1.",
            "anchors": [fx.anchor(proof_line, "along the adiabatic path")]
        },
        {
            "new": { "id": "sub-sib", "attach_to": m_step2, "kind": "substep" },
            "gist": "Attempt to expand sibling.",
            "anchors": [fx.anchor(proof_line, "the claimed decay bound")]
        }
    ]), json!([]), json!([]), true)).unwrap();
    assert_eq!(r3["accepted_items"], 1);
    let m_sub1a = r3["assigned_tags"]["sub-1a"].as_str().unwrap().to_string();
    assert!(m_sub1a.starts_with("m-"));
    assert_eq!(r3["items"][1]["code"], "out_of_scope_tag");
    fx.ledger.append(SID, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req3, "status": "done" })).unwrap();

    // Fold verifies additive history on m_step1, no duplicate child on thm_tag, and depth-2 link.
    let events = fx.ledger.read_session(SID).unwrap();
    let folded = crate::map_fold::fold_map(&fx.skeleton, &events, false);
    assert_eq!(folded.nodes[&thm_tag].children, vec![m_step1.clone(), m_step2.clone()]);
    assert_eq!(folded.nodes[&m_step1].children, vec![m_sub1a.clone()]);
    assert_eq!(folded.nodes[&m_step1].gist.current.as_ref().unwrap().value, "Refined step 1 gist.");
    assert_eq!(folded.nodes[&m_step1].gist.history.len(), 1);
    assert!(!folded.nodes[&m_step1].any_stale);
    assert!(!folded.nodes[&m_sub1a].any_stale);
}

#[test]
fn anchor_outside_node_message_reports_spans_and_authors_claim_guidance() {
    let fx = Fixture::new();
    let req_seq = fx.start_task("orient", &[]);
    fx.supply_all_lines(req_seq, CALLER, &fx.file_sha256);

    let thm_tag = fx.tag_by_label("thm:main");
    let thm_node = fx.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&thm_tag)).unwrap();
    let proof_node = fx.skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Proof && n.proof_of.as_deref() == Some(&thm_tag)).unwrap();
    let thm_span = format!("{}:{}-{}", thm_node.spans[0].path, thm_node.spans[0].start_line, thm_node.spans[0].end_line);
    let proof_span = format!("{}:{}-{}", proof_node.spans[0].path, proof_node.spans[0].start_line, proof_node.spans[0].end_line);
    let intro_line = fx.line_of("We prove a sharp bound on finite lattice spectra");
    let thm_line = fx.line_of("spectral flow preserves rapid decay");
    let intro_span = format!("{}:{intro_line}-{intro_line}", fx.path);

    let res = record_map_reading(&fx.env, &fx.ledger, SID, CALLER, &fx.call_args(req_seq, json!([
        {
            "tag": thm_tag, "gist": "Intro quote placed in item anchors.",
            "anchors": [fx.anchor(intro_line, "We prove a sharp bound on finite lattice spectra")]
        },
        {
            "new": { "id": "step-out", "attach_to": thm_tag, "kind": "proof_step" },
            "gist": "Model step with outside anchor.",
            "anchors": [fx.anchor(intro_line, "We prove a sharp bound on finite lattice spectra")]
        },
        {
            "tag": thm_tag, "gist": "Inside anchor with intro quote in authors_claim.anchors.",
            "anchors": [fx.anchor(thm_line, "spectral flow preserves rapid decay")],
            "authors_claim": { "label": "new", "anchors": [fx.anchor(intro_line, "We prove a sharp bound on finite lattice spectra")] }
        }
    ]), json!([]), json!([]), false)).unwrap();

    assert_eq!(res["accepted_items"], 1);
    assert_eq!(res["items"][2]["status"], "accepted");

    let sk_rej = &res["items"][0];
    assert_eq!(sk_rej["code"], "anchor_outside_node");
    assert!(sk_rej.get("true_text").is_none(), "must not overload true_text: {sk_rej}");
    let sk_msg = sk_rej["message"].as_str().unwrap();
    assert!(sk_msg.contains(&thm_tag) && sk_msg.contains(&thm_span), "missing node span: {sk_msg}");
    assert!(sk_msg.contains(&intro_span), "missing matched anchor span: {sk_msg}");
    assert!(sk_msg.contains("authors_claim.anchors") && sk_msg.contains("Introduction/Conclusion"), "missing rule: {sk_msg}");

    let model_rej = &res["items"][1];
    assert_eq!(model_rej["code"], "anchor_outside_node");
    assert!(model_rej.get("true_text").is_none(), "must not overload true_text: {model_rej}");
    let m_msg = model_rej["message"].as_str().unwrap();
    assert!(m_msg.contains(&thm_span) && m_msg.contains(&proof_span), "model node must list statement + root proof spans: {m_msg}");
    assert!(m_msg.contains(&intro_span) && m_msg.contains("authors_claim.anchors"), "got: {m_msg}");
}

