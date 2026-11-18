//! Tool registry and router.
//!
//! Every call goes: record request -> policy -> (approval) -> dispatch ->
//! record result. If the ledger cannot record a step, the call is refused.

pub mod approval;
pub mod builtin;
pub mod canon;
pub mod registry;
pub mod router;

pub use approval::{approval_digest, ApprovalReply, ApprovalRequest, ApprovalStore, Approver};
pub use registry::{CallCtx, Registry, ToolDef, ToolHandler};
pub use router::{CallOutcome, Router};

/// Version of the event payloads this crate writes to the ledger.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Status a recorded claim can have in v0. There is deliberately no
/// `Verified`: no model-callable tool may mark a claim verified, and v0 has
/// no checker that could. `Tested` means bounded evidence only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimStatus {
    Unverified,
    Tested,
}
