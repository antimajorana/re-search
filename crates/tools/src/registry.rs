//! Tool definitions, their hashes, and a small argument check.

use crate::approval::{ApprovalScope, WorkerEpoch};
use crate::canon::{canonical_json, hash_fields};
use ledger::Ledger;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub id: String,
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub input_schema: Value,
    pub effect: EffectClass,
    pub approval_scope: ApprovalScope,
}

impl ToolDef {
    /// SHA-256 of the canonical definition. Changes whenever the id, the
    /// description, the schema, the effect class or the approval scope changes.
    pub fn def_hash(&self) -> String {
        let canonical = canonical_json(&json!({
            "id": self.id,
            "description": self.description,
            "input_schema": self.input_schema,
            "effect": self.effect.as_str(),
            "approval_scope": self.approval_scope.as_str(),
        }));
        hash_fields(&["tool-def-v2", &canonical])
    }
}

/// What a handler is told about the call it serves.
pub struct CallCtx<'a> {
    pub session: &'a str,
    pub caller: &'a str,
    pub call_id: &'a str,
    pub ledger: &'a Mutex<Ledger>,
    /// If `Some(e)`, a session-reused call must run inside worker epoch `e`
    /// without respawning a fresh worker.
    pub required_epoch: Option<u64>,
    ran_epoch: AtomicU64,
}

impl<'a> CallCtx<'a> {
    pub(crate) fn new(
        session: &'a str,
        caller: &'a str,
        call_id: &'a str,
        ledger: &'a Mutex<Ledger>,
        required_epoch: Option<u64>,
    ) -> Self {
        CallCtx { session, caller, call_id, ledger, required_epoch, ran_epoch: AtomicU64::new(0) }
    }

    /// Record the worker epoch (`>= 1`) that actually executed this call while
    /// holding the worker lock.
    pub fn record_worker_epoch(&self, epoch: u64) {
        assert!(epoch > 0, "worker epoch must be >= 1");
        self.ran_epoch.store(epoch, Ordering::Release);
    }

    pub(crate) fn worker_epoch(&self) -> Option<u64> {
        match self.ran_epoch.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }
}

pub trait ToolHandler: Send + Sync {
    fn call(&self, ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String>;
}

impl<F> ToolHandler for F
where
    F: Fn(&CallCtx<'_>, &Value) -> Result<Value, String> + Send + Sync,
{
    fn call(&self, ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String> {
        self(ctx, args)
    }
}

type RegistryEntry = (ToolDef, Arc<dyn ToolHandler>, Option<Arc<dyn WorkerEpoch>>);
pub(crate) type EntryRef<'a> = (&'a ToolDef, &'a Arc<dyn ToolHandler>, Option<&'a Arc<dyn WorkerEpoch>>);

#[derive(Default)]
pub struct Registry {
    tools: BTreeMap<String, RegistryEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a `Once`-scoped tool.
    pub fn register(&mut self, def: ToolDef, handler: Arc<dyn ToolHandler>) {
        assert_eq!(
            def.approval_scope,
            ApprovalScope::Once,
            "WorkerSession tool `{}` must use register_worker_session",
            def.id
        );
        self.tools.insert(def.id.clone(), (def, handler, None));
    }

    /// Register a `WorkerSession`-scoped tool together with its epoch provider.
    pub fn register_worker_session(
        &mut self,
        def: ToolDef,
        epoch: Arc<dyn WorkerEpoch>,
        handler: Arc<dyn ToolHandler>,
    ) {
        assert_eq!(
            def.approval_scope,
            ApprovalScope::WorkerSession,
            "register_worker_session requires ApprovalScope::WorkerSession for `{}`",
            def.id
        );
        self.tools.insert(def.id.clone(), (def, handler, Some(epoch)));
    }

    pub fn get(&self, id: &str) -> Option<(&ToolDef, &Arc<dyn ToolHandler>)> {
        self.tools.get(id).map(|(d, h, _)| (d, h))
    }

    pub(crate) fn get_entry(&self, id: &str) -> Option<EntryRef<'_>> {
        self.tools.get(id).map(|(d, h, e)| (d, h, e.as_ref()))
    }

    /// Definitions in id order.
    pub fn defs(&self) -> Vec<&ToolDef> {
        self.tools.values().map(|(d, _, _)| d).collect()
    }
}

/// Check `args` against the subset of JSON Schema that v0 tool definitions
/// use: an object with typed properties, `required`, `maxLength`, and
/// `additionalProperties: false`. This is an input check, not a full
/// validator.
pub fn check_args(schema: &Value, args: &Value) -> Result<(), String> {
    let obj = args.as_object().ok_or("arguments must be a JSON object")?;
    let props = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(name) {
                return Err(format!("missing required argument `{name}`"));
            }
        }
    }
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    for (name, value) in obj {
        match props.and_then(|p| p.get(name)) {
            Some(prop) => {
                if let Some(ty) = prop.get("type").and_then(Value::as_str) {
                    let ok = match ty {
                        "string" => value.is_string(),
                        "number" => value.is_number(),
                        "integer" => value.is_i64() || value.is_u64(),
                        "boolean" => value.is_boolean(),
                        "object" => value.is_object(),
                        "array" => value.is_array(),
                        _ => true,
                    };
                    if !ok {
                        return Err(format!("argument `{name}` must be of type {ty}"));
                    }
                }
                if let (Some(max), Some(s)) = (prop.get("maxLength").and_then(Value::as_u64), value.as_str()) {
                    if s.chars().count() as u64 > max {
                        return Err(format!("argument `{name}` is longer than {max} characters"));
                    }
                }
            }
            None if closed => return Err(format!("unknown argument `{name}`")),
            None => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def() -> ToolDef {
        ToolDef {
            id: "t".into(),
            description: "d".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"code": {"type": "string", "maxLength": 5}, "n": {"type": "integer"}},
                "required": ["code"],
                "additionalProperties": false
            }),
            effect: EffectClass::ObserveLocal,
            approval_scope: ApprovalScope::Once,
        }
    }

    #[test]
    fn def_hash_changes_with_every_part() {
        let base = def().def_hash();
        let mut d = def();
        d.description = "other".into();
        assert_ne!(d.def_hash(), base);
        let mut d = def();
        d.effect = EffectClass::ExecuteCode;
        assert_ne!(d.def_hash(), base);
        let mut d = def();
        d.approval_scope = ApprovalScope::WorkerSession;
        assert_ne!(d.def_hash(), base);
        let mut d = def();
        d.input_schema["required"] = json!([]);
        assert_ne!(d.def_hash(), base);
        assert_eq!(def().def_hash(), base);
    }

    #[test]
    fn args_are_checked() {
        let s = def().input_schema;
        assert!(check_args(&s, &json!({"code": "x"})).is_ok());
        assert!(check_args(&s, &json!({"code": "x", "n": 2})).is_ok());
        assert!(check_args(&s, &json!({})).is_err());
        assert!(check_args(&s, &json!({"code": 1})).is_err());
        assert!(check_args(&s, &json!({"code": "x", "n": 1.5})).is_err());
        assert!(check_args(&s, &json!({"code": "x", "extra": 1})).is_err());
        assert!(check_args(&s, &json!({"code": "toolong"})).is_err());
        assert!(check_args(&s, &json!("x")).is_err());
    }
}
