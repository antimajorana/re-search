//! Interactive Human-in-the-Loop primitives (`ask_user`, steer injection,
//! and post-exploration branch selection checkpoints).

use crate::delegate::clip_chars;
use crate::runner::Audit;
use crate::transport::{CancelToken, ContentBlock, Message, Role, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ASK_USER: &str = "ask_user";
const MAX_TOOL_JSON_LEN: usize = 3_800;

pub fn is_human_tool(name: &str) -> bool {
    name == ASK_USER
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPrompt {
    pub checkpoint_id: String,
    pub kind: String,
    pub question: String,
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanReply {
    pub checkpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_option: Option<String>,
    pub text: String,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteerMessage {
    pub steer_id: String,
    pub text: String,
    pub ts_ms: i64,
}

pub trait HumanChannel: Send + Sync {
    fn drain_steers(&self) -> Vec<SteerMessage>;
    fn ask_checkpoint(&self, prompt: &CheckpointPrompt, cancel: Option<&CancelToken>) -> HumanReply;
}

pub fn ask_user_tool_spec() -> ToolSpec {
    ToolSpec {
        name: ASK_USER.to_string(),
        description: "Ask the human researcher for a high-level mathematical decision, convention \
            choice, ambiguity resolution, or research plan approval (never for low-level tool permissions)."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "Concise, self-contained question for the human." },
                "kind": { "type": "string", "enum": ["decision", "plan", "convention", "branch", "clarification"] },
                "options": { "type": "array", "items": { "type": "string" }, "description": "Optional short choices (up to 6)." },
                "context": { "type": "string", "description": "Optional brief context or plan summary." }
            },
            "required": ["question"],
            "additionalProperties": false
        }),
    }
}

pub fn build_interactive_prompt_section(interactive_plan_mode: bool, pause_after_explore: bool) -> String {
    let mut lines = vec![
        "Interactive Human-in-the-Loop:".to_string(),
        format!("- `{ASK_USER}` asks the human for high-level mathematical decisions, convention choices, ambiguity resolution, or plan approval (never low-level tool permissions)."),
        "- Treat `<human_steer>` messages arriving between turns as authoritative guidance.".to_string(),
    ];
    if interactive_plan_mode {
        lines.push(format!(
            "- Interactive Plan Mode is active: FIRST formulate a concise numbered plan (assumptions, derivation steps, planned compute/literature/Lean/explore workers) and call `{ASK_USER}` with `kind: \"plan\"`, `question: \"Approve or edit this research plan before execution?\"`, and `options: [\"Approve plan\", \"Adjust approach\"]` BEFORE running heavy workers."
        ));
    }
    if pause_after_explore {
        lines.push("- Pause After Explore is active: `delegate_explore` pauses for user selection and attaches `\"human_selection\"` to its output; follow the user's chosen branch or instructions.".to_string());
    }
    lines.join("\n")
}

pub(crate) fn upsert_interactive_prompt_section(sys: &mut String, plan_mode: bool, pause_explore: bool) {
    let sec = build_interactive_prompt_section(plan_mode, pause_explore);
    if let Some(idx) = sys.find("Interactive Human-in-the-Loop:") {
        let end = sys[idx..].find("\n\n").map(|o| idx + o).unwrap_or(sys.len());
        sys.replace_range(idx..end, &sec);
    } else if !sys.contains("Interactive Human-in-the-Loop") {
        let trimmed = sys.trim_end();
        *sys = if trimmed.is_empty() { sec } else { format!("{trimmed}\n\n{sec}") };
    }
}

pub(crate) fn inject_pending_steers(audit: &mut Audit<'_>, human: &dyn HumanChannel, messages: &mut Vec<Message>) -> Result<usize, String> {
    let mut count = 0usize;
    for s in human.drain_steers() {
        if s.text.trim().is_empty() { continue; }
        audit.emit("human.steer", json!({ "steer_id": s.steer_id, "text": s.text, "ts_ms": s.ts_ms }))?;
        audit.sink.on_thinking_delta(&format!("\n[Human steer: {}]\n", clip_chars(&s.text, 120)));
        let wrapped = format!("<human_steer>\n{}\n</human_steer>", s.text);
        if let Some(last) = messages.last_mut().filter(|m| m.role == Role::User) {
            last.content.push(ContentBlock::Text { text: wrapped });
        } else {
            messages.push(Message { role: Role::User, content: vec![ContentBlock::Text { text: wrapped }] });
        }
        count += 1;
    }
    Ok(count)
}

fn ask_and_record(audit: &mut Audit<'_>, h: &dyn HumanChannel, prompt: &CheckpointPrompt, cancel: Option<&CancelToken>) -> Result<HumanReply, String> {
    audit.emit("human.checkpoint_requested", serde_json::to_value(prompt).map_err(|e| e.to_string())?)?;
    let reply = h.ask_checkpoint(prompt, cancel);
    audit.emit("human.decision", json!({
        "checkpoint_id": reply.checkpoint_id, "kind": prompt.kind,
        "selected_option": reply.selected_option, "text": reply.text, "cancelled": reply.cancelled,
    }))?;
    Ok(reply)
}

fn fit_ask_user_json(reply: &HumanReply) -> String {
    let (mut tlim, mut olim) = (MAX_TOOL_JSON_LEN, 512usize);
    loop {
        let encoded = tools::canon::canonical_json(&json!({
            "checkpoint_id": reply.checkpoint_id,
            "selected_option": reply.selected_option.as_deref().map(|s| clip_chars(s, olim)),
            "text": clip_chars(&reply.text, tlim),
            "cancelled": reply.cancelled,
        }));
        if encoded.len() <= MAX_TOOL_JSON_LEN || (tlim == 0 && olim <= 32) { return encoded; }
        if tlim > 0 { tlim = (tlim * 3 / 4).min(tlim - 1); } else { olim = (olim / 2).max(32); }
    }
}

pub(crate) fn run_ask_user_tool(
    audit: &mut Audit<'_>,
    human: Option<&dyn HumanChannel>,
    tool_use_id: &str,
    input: &Value,
    cancel: Option<&CancelToken>,
) -> Result<(bool, String), String> {
    let Some(h) = human else {
        return Ok((true, "DENIED: interactive human channel is not enabled for this session".into()));
    };
    let question = input.get("question").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    if question.is_empty() { return Ok((true, "ERROR: `question` must be a non-empty string".into())); }
    let kind = input.get("kind").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("decision").to_string();
    let options = input.get("options").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).take(6).map(str::to_string).collect()
    }).unwrap_or_default();
    let context = input.get("context").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let prompt = CheckpointPrompt { checkpoint_id: format!("chk-{tool_use_id}"), kind, question, options, context, branches: Vec::new() };
    let reply = ask_and_record(audit, h, &prompt, cancel)?;
    Ok((reply.cancelled, fit_ask_user_json(&reply)))
}

fn fit_explore_with_selection(explore_val: &mut Value, reply: &HumanReply) -> String {
    let orig: Vec<Option<String>> = explore_val.get("branches").and_then(Value::as_array)
        .map(|a| a.iter().map(|b| b.get("summary").and_then(Value::as_str).map(str::to_string)).collect()).unwrap_or_default();
    let apply = |v: &mut Value, cap: Option<usize>, tlim: usize| {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("human_selection".into(), json!({
                "selected_option": reply.selected_option, "text": clip_chars(&reply.text, tlim), "cancelled": reply.cancelled,
            }));
            if let (Some(c), Some(Value::Array(br))) = (cap, obj.get_mut("branches")) {
                for (b, f) in br.iter_mut().zip(&orig) {
                    if let (Some(bo), Some(s)) = (b.as_object_mut(), f.as_ref()) {
                        bo.insert("summary".into(), Value::String(if c == 0 { String::new() } else { clip_chars(s, c) }));
                    }
                }
            }
        }
        tools::canon::canonical_json(v)
    };
    let mut cap = orig.iter().flatten().map(|s| s.chars().count()).max().unwrap_or(0);
    let mut out = apply(explore_val, None, 600);
    while out.len() > MAX_TOOL_JSON_LEN && cap > 0 {
        cap = cap.saturating_sub(((out.len() - MAX_TOOL_JSON_LEN) / orig.len().max(1)).max(16));
        out = apply(explore_val, Some(cap), 600);
    }
    let mut tlim = 600usize;
    while out.len() > MAX_TOOL_JSON_LEN && tlim > 0 {
        tlim /= 2;
        out = apply(explore_val, Some(0), tlim);
    }
    out
}

pub(crate) fn maybe_pause_explore_branch_picker(
    audit: &mut Audit<'_>,
    human: Option<&dyn HumanChannel>,
    pause_after_explore: bool,
    tool_use_id: &str,
    explore_result: (bool, String),
    cancel: Option<&CancelToken>,
) -> Result<(bool, String), String> {
    if !pause_after_explore || explore_result.0 || cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok(explore_result);
    }
    let Some(h) = human else { return Ok(explore_result); };
    let Ok(mut explore_val) = serde_json::from_str::<Value>(&explore_result.1) else { return Ok(explore_result); };
    let Some(branches_vec) = explore_val.get("branches").and_then(Value::as_array).cloned() else { return Ok(explore_result); };
    if branches_vec.len() < 2 { return Ok(explore_result); }
    let mut options: Vec<String> = branches_vec.iter().enumerate().map(|(i, b)| {
        let app = b.get("approach").and_then(Value::as_str).unwrap_or("").trim();
        let lbl = if app.is_empty() { format!("Branch {}", i + 1) } else { clip_chars(&app.replace(['\r', '\n'], " "), 72) };
        format!("Adopt Branch {}: {lbl}", i + 1)
    }).collect();
    options.extend(["Combine best ideas".into(), "Let Orchestrator decide".into()]);
    let prompt = CheckpointPrompt {
        checkpoint_id: format!("br-{tool_use_id}"), kind: "branch".into(),
        question: "Choose which parallel exploration branch to pursue:".into(),
        options, context: None, branches: branches_vec,
    };
    let reply = ask_and_record(audit, h, &prompt, cancel)?;
    Ok((false, fit_explore_with_selection(&mut explore_val, &reply)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::NoopSink;
    use ledger::Ledger;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct StubHuman(Mutex<Vec<SteerMessage>>, Mutex<Option<HumanReply>>, Mutex<Vec<CheckpointPrompt>>);
    impl HumanChannel for StubHuman {
        fn drain_steers(&self) -> Vec<SteerMessage> { std::mem::take(&mut *self.0.lock().unwrap()) }
        fn ask_checkpoint(&self, p: &CheckpointPrompt, _: Option<&CancelToken>) -> HumanReply {
            self.2.lock().unwrap().push(p.clone());
            self.1.lock().unwrap().take().unwrap()
        }
    }

    fn setup(sess: &str) -> (TempDir, Ledger, NoopSink) {
        let tmp = TempDir::new().unwrap();
        let ledger = Ledger::open(tmp.path()).unwrap();
        let mut sink = NoopSink;
        Audit { ledger: &ledger, session: sess, sink: &mut sink, last_seq: 0 }.emit("session.start", json!({"route":"api"})).unwrap();
        (tmp, ledger, sink)
    }

    #[test]
    fn inject_pending_steers_records_event_and_merges_user_message() {
        let (_tmp, ledger, mut sink) = setup("s-steer");
        let mut audit = Audit { ledger: &ledger, session: "s-steer", sink: &mut sink, last_seq: 1 };
        let ch = StubHuman(Mutex::new(vec![
            SteerMessage { steer_id: "st1".into(), text: "Focus on k=0".into(), ts_ms: 42 },
            SteerMessage { steer_id: "st2".into(), text: "  ".into(), ts_ms: 43 },
        ]), Mutex::new(None), Mutex::new(Vec::new()));
        let mut msgs = vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] }];
        assert_eq!(inject_pending_steers(&mut audit, &ch, &mut msgs).unwrap(), 1);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(&msgs[0].content[1], ContentBlock::Text { text } if text == "<human_steer>\nFocus on k=0\n</human_steer>"));
        assert!(ledger.read_session("s-steer").unwrap().iter().any(|e| e.kind == "human.steer" && e.payload["steer_id"] == "st1"));
        ledger.verify_session("s-steer").unwrap();
    }

    #[test]
    fn run_ask_user_tool_records_checkpoint_and_decision() {
        let (_tmp, ledger, mut sink) = setup("s-ask");
        let mut audit = Audit { ledger: &ledger, session: "s-ask", sink: &mut sink, last_seq: 1 };
        let ch = StubHuman(Mutex::new(Vec::new()), Mutex::new(Some(HumanReply {
            checkpoint_id: "chk-tu1".into(), selected_option: Some("Approve plan".into()), text: "x".repeat(5000), cancelled: false,
        })), Mutex::new(Vec::new()));
        let (is_err, out) = run_ask_user_tool(&mut audit, Some(&ch), "tu1", &json!({"question":"Approve?","kind":"plan","options":["A","","B","C","D","E","F","G"]}), None).unwrap();
        assert!(!is_err && out.len() <= MAX_TOOL_JSON_LEN && ch.2.lock().unwrap()[0].options.len() == 6);
        let evs = ledger.read_session("s-ask").unwrap();
        assert!(evs.iter().any(|e| e.kind == "human.checkpoint_requested" && e.payload["checkpoint_id"] == "chk-tu1"));
        assert!(evs.iter().any(|e| e.kind == "human.decision" && e.payload["selected_option"] == "Approve plan"));
        ledger.verify_session("s-ask").unwrap();
    }

    #[test]
    fn maybe_pause_explore_branch_picker_attaches_human_selection_within_limit() {
        let (_tmp, ledger, mut sink) = setup("s-br");
        let mut audit = Audit { ledger: &ledger, session: "s-br", sink: &mut sink, last_seq: 1 };
        let ch = StubHuman(Mutex::new(Vec::new()), Mutex::new(Some(HumanReply {
            checkpoint_id: "br-ex1".into(), selected_option: Some("Adopt Branch 1: Perturbative".into()), text: "y".repeat(700), cancelled: false,
        })), Mutex::new(Vec::new()));
        let exp = json!({"ok":true,"branches":[{"index":0,"approach":"Perturbative","summary":"a".repeat(1700)},{"index":1,"approach":"Exact","summary":"b".repeat(1700)}]});
        let (is_err, out) = maybe_pause_explore_branch_picker(&mut audit, Some(&ch), true, "ex1", (false, exp.to_string()), None).unwrap();
        assert!(!is_err && out.len() <= MAX_TOOL_JSON_LEN);
        let p: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(p["human_selection"]["selected_option"], "Adopt Branch 1: Perturbative");
        assert!(p["human_selection"]["text"].as_str().unwrap().chars().count() <= 600);
        assert_eq!(ch.2.lock().unwrap()[0].options.len(), 4);
        ledger.verify_session("s-br").unwrap();
    }
}
