//! Atomic pre-send model-call budget pool (package F9b, `00-overview.md` §3.4).
//!
//! Wraps F9's ledger-derived [`count_model_calls_breakdown`] with an in-memory mutex-guarded
//! lease protocol so parallel workers, verifiers, and the frontier orchestrator cannot race
//! past `total_cap` or encroach on `frontier_reserve` / `verifier_reserve` before an HTTP
//! request leaves the host.
//!
//! Lifecycle per provider call:
//! 1. Build and validate `ModelRequest` + run `transport.prepare_request` locally (0 budget).
//! 2. Call [`RunBudgetPool::try_acquire`] right before sending over HTTP.
//! 3. Immediately before HTTP POST begins, call [`BudgetLease::mark_sent`] to convert the
//!    in-flight slot into a non-refundable committed count (failed/retried/cancelled sends
//!    still count once).
//! 4. If local preparation or cancellation aborts before send, dropping [`BudgetLease`]
//!    releases the unused slot automatically.

use crate::run_budget::{
    count_model_calls_breakdown, ModelCallBreakdown, ModelCallBudget, RunScope,
};
use ledger::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Default frontier reserve for Investigate (`Orient`/`Open`) and Solve closing (`record_map_reading`/`solve_finish` + `EndTurn`).
pub const DEFAULT_FRONTIER_RESERVE: u64 = 2;
/// Default frontier reserve for Explore (`X11` Map + `X13` Directions, up to 3 calls each).
pub const EXPLORE_FRONTIER_RESERVE: u64 = 6;
/// Default verifier reserve for Solve when closing review is enabled and not yet completed.
pub const SOLVE_VERIFIER_RESERVE: u64 = 5;

/// Caller class competing for a provider call inside [`RunBudgetPool`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PoolCaller {
    /// Primary frontier orchestrator / solver (`agent:*`). Exempt from `frontier_reserve`,
    /// still respects `total_cap` and unspent `verifier_reserve`.
    Frontier,
    /// Bounded worker sub-loop or stage reader (`paper_reader`, `idea_reader`, `compute_worker`,
    /// `compute_blind`, etc.). Must respect both `frontier_reserve` and unspent `verifier_reserve`,
    /// plus optional per-item `sub_key` caps (e.g. Explore `calls_per_paper`).
    Worker {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sub_key: Option<String>,
    },
    /// Adversarial/step verifier (`verifier:solve`, `step_verifier`, `full_verifier`).
    /// Exempt from `verifier_reserve`, still respects `total_cap` and `frontier_reserve`.
    Verifier { role: String },
}

impl PoolCaller {
    pub fn frontier() -> Self {
        Self::Frontier
    }

    pub fn worker(role: impl Into<String>, sub_key: Option<String>) -> Self {
        Self::Worker { role: role.into(), sub_key }
    }

    pub fn verifier(role: impl Into<String>) -> Self {
        Self::Verifier { role: role.into() }
    }

    fn sub_key(&self) -> Option<&str> {
        match self {
            Self::Worker { sub_key, .. } => sub_key.as_deref(),
            Self::Frontier | Self::Verifier { .. } => None,
        }
    }

    fn is_verifier(&self) -> bool {
        matches!(self, Self::Verifier { .. })
    }
}

/// Per-item call cap inside a [`RunBudgetPool`] (for example Explore `calls_per_paper`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubCap {
    pub cap: u64,
    pub committed: u64,
    pub in_flight: u64,
}

impl SubCap {
    pub fn new(cap: u64) -> Self {
        Self { cap, committed: 0, in_flight: 0 }
    }

    pub fn with_committed(cap: u64, committed: u64) -> Self {
        Self { cap, committed, in_flight: 0 }
    }

    pub fn used(&self) -> u64 {
        self.committed.saturating_add(self.in_flight)
    }

    pub fn remaining(&self) -> u64 {
        self.cap.saturating_sub(self.used())
    }
}

/// Reason a [`RunBudgetPool::try_acquire`] call was refused pre-send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExhaustedScope {
    TotalCap,
    ReservedHeadroom,
    SubCap(String),
    LockPoisoned,
}

/// Error returned when acquiring a pre-send call permit would breach the cap or reserves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetExhausted {
    pub caller: PoolCaller,
    pub scope: ExhaustedScope,
    pub committed: u64,
    pub in_flight: u64,
    pub reserved: u64,
    pub total_cap: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_cap: Option<SubCap>,
}

impl fmt::Display for BudgetExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            ExhaustedScope::SubCap(key) => {
                let cap = self.sub_cap.map_or(0, |s| s.cap);
                let used = self.sub_cap.map_or(0, |s| s.used());
                write!(f, "sub-budget exhausted for `{key}` ({used}/{cap} model calls)")
            }
            ExhaustedScope::ReservedHeadroom => write!(
                f,
                "worker budget exhausted ({} committed + {} in_flight + {} reserved >= {} total_cap)",
                self.committed, self.in_flight, self.reserved, self.total_cap
            ),
            ExhaustedScope::TotalCap => write!(
                f,
                "model call budget exhausted ({} committed + {} in_flight >= {} total_cap)",
                self.committed, self.in_flight, self.total_cap
            ),
            ExhaustedScope::LockPoisoned => write!(f, "run budget pool lock poisoned"),
        }
    }
}

impl std::error::Error for BudgetExhausted {}

/// Point-in-time snapshot of [`RunBudgetPool`] counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub total_cap: u64,
    pub committed: u64,
    pub in_flight: u64,
    pub frontier_reserve: u64,
    pub verifier_reserve: u64,
    pub unspent_verifier_reserve: u64,
    pub verifier_committed: u64,
    pub verifier_in_flight: u64,
    pub available_for_workers: u64,
    pub available_for_frontier: u64,
    pub available_for_verifier: u64,
    pub sub_caps: BTreeMap<String, SubCap>,
    pub initial_breakdown: ModelCallBreakdown,
}

#[derive(Debug, Clone)]
struct PoolState {
    total_cap: u64,
    committed: u64,
    in_flight: u64,
    frontier_reserve: u64,
    verifier_reserve: u64,
    verifier_committed: u64,
    verifier_in_flight: u64,
    sub_caps: BTreeMap<String, SubCap>,
    default_sub_cap: Option<u64>,
    initial_breakdown: ModelCallBreakdown,
}

impl PoolState {
    fn unspent_verifier_reserve(&self) -> u64 {
        let verifier_used = self.verifier_committed.saturating_add(self.verifier_in_flight);
        self.verifier_reserve.saturating_sub(verifier_used)
    }

    fn reserved_for(&self, caller: &PoolCaller) -> u64 {
        let unspent_verifier = self.unspent_verifier_reserve();
        match caller {
            PoolCaller::Frontier => unspent_verifier,
            PoolCaller::Worker { .. } => self.frontier_reserve.saturating_add(unspent_verifier),
            PoolCaller::Verifier { .. } => self.frontier_reserve,
        }
    }

    fn snapshot(&self) -> BudgetSnapshot {
        let unspent_verifier = self.unspent_verifier_reserve();
        let used = self.committed.saturating_add(self.in_flight);
        let worker_floor = used
            .saturating_add(self.frontier_reserve)
            .saturating_add(unspent_verifier);
        let frontier_floor = used.saturating_add(unspent_verifier);
        let verifier_floor = used.saturating_add(self.frontier_reserve);
        BudgetSnapshot {
            total_cap: self.total_cap,
            committed: self.committed,
            in_flight: self.in_flight,
            frontier_reserve: self.frontier_reserve,
            verifier_reserve: self.verifier_reserve,
            unspent_verifier_reserve: unspent_verifier,
            verifier_committed: self.verifier_committed,
            verifier_in_flight: self.verifier_in_flight,
            available_for_workers: self.total_cap.saturating_sub(worker_floor),
            available_for_frontier: self.total_cap.saturating_sub(frontier_floor),
            available_for_verifier: self.total_cap.saturating_sub(verifier_floor),
            sub_caps: self.sub_caps.clone(),
            initial_breakdown: self.initial_breakdown,
        }
    }
}

/// Receipt returned when [`BudgetLease::mark_sent`] commits an in-flight call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedCall {
    pub caller: PoolCaller,
    pub snapshot: BudgetSnapshot,
}

/// RAII pre-send permit issued by [`RunBudgetPool::try_acquire`].
///
/// Calling [`BudgetLease::mark_sent`] right as HTTP transmission starts commits the call
/// permanently. Dropping the lease without calling `mark_sent` refunds the in-flight slot.
pub struct BudgetLease {
    state: Arc<Mutex<PoolState>>,
    caller: PoolCaller,
    active: bool,
}

impl BudgetLease {
    pub fn caller(&self) -> &PoolCaller {
        &self.caller
    }

    /// Convert this in-flight lease into a non-refundable committed model call at HTTP send start.
    pub fn mark_sent(mut self) -> CommittedCall {
        self.active = false;
        let snapshot = if let Ok(mut guard) = self.state.lock() {
            guard.in_flight = guard.in_flight.saturating_sub(1);
            guard.committed = guard.committed.saturating_add(1);
            if self.caller.is_verifier() {
                guard.verifier_in_flight = guard.verifier_in_flight.saturating_sub(1);
                guard.verifier_committed = guard.verifier_committed.saturating_add(1);
            }
            if let Some(key) = self.caller.sub_key() {
                if let Some(sub) = guard.sub_caps.get_mut(key) {
                    sub.in_flight = sub.in_flight.saturating_sub(1);
                    sub.committed = sub.committed.saturating_add(1);
                }
            }
            guard.snapshot()
        } else {
            BudgetSnapshot {
                total_cap: 0,
                committed: 0,
                in_flight: 0,
                frontier_reserve: 0,
                verifier_reserve: 0,
                unspent_verifier_reserve: 0,
                verifier_committed: 0,
                verifier_in_flight: 0,
                available_for_workers: 0,
                available_for_frontier: 0,
                available_for_verifier: 0,
                sub_caps: BTreeMap::new(),
                initial_breakdown: ModelCallBreakdown::default(),
            }
        };
        CommittedCall { caller: self.caller.clone(), snapshot }
    }
}

impl Drop for BudgetLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        if let Ok(mut guard) = self.state.lock() {
            guard.in_flight = guard.in_flight.saturating_sub(1);
            if self.caller.is_verifier() {
                guard.verifier_in_flight = guard.verifier_in_flight.saturating_sub(1);
            }
            if let Some(key) = self.caller.sub_key() {
                if let Some(sub) = guard.sub_caps.get_mut(key) {
                    sub.in_flight = sub.in_flight.saturating_sub(1);
                }
            }
        }
    }
}

/// Thread-safe shared pre-send budget pool for a single run, task, or stage.
#[derive(Debug, Clone)]
pub struct RunBudgetPool {
    inner: Arc<Mutex<PoolState>>,
}

impl RunBudgetPool {
    /// Construct a pool from an explicit [`ModelCallBreakdown`] and reserve parameters.
    pub fn from_breakdown(
        breakdown: ModelCallBreakdown,
        total_cap: u64,
        frontier_reserve: u64,
        verifier_reserve: u64,
        sub_caps: BTreeMap<String, SubCap>,
    ) -> Self {
        let state = PoolState {
            total_cap,
            committed: breakdown.total(),
            in_flight: 0,
            frontier_reserve,
            verifier_reserve,
            verifier_committed: 0,
            verifier_in_flight: 0,
            sub_caps,
            default_sub_cap: None,
            initial_breakdown: breakdown,
        };
        Self { inner: Arc::new(Mutex::new(state)) }
    }

    /// Initialize directly from ledger events using [`count_model_calls_breakdown`].
    pub fn from_events(
        events: &[Event],
        scope: RunScope<'_>,
        total_cap: u64,
        frontier_reserve: u64,
        verifier_reserve: u64,
    ) -> Self {
        let breakdown = count_model_calls_breakdown(events, scope);
        Self::from_breakdown(breakdown, total_cap, frontier_reserve, verifier_reserve, BTreeMap::new())
    }

    /// Initialize with explicit per-item sub-caps (e.g. Explore `calls_per_paper`).
    pub fn with_sub_caps(self, caps: BTreeMap<String, SubCap>) -> Self {
        if let Ok(mut guard) = self.inner.lock() {
            guard.sub_caps = caps;
        }
        self
    }

    /// Set a fallback per-key cap automatically created the first time a `sub_key` is seen.
    pub fn with_default_sub_cap(self, cap: u64) -> Self {
        if let Ok(mut guard) = self.inner.lock() {
            guard.default_sub_cap = Some(cap);
        }
        self
    }

    /// Register or update a per-item sub-cap (for instance before dispatching a new paper batch).
    pub fn set_sub_cap(&self, key: impl Into<String>, cap: u64) {
        if let Ok(mut guard) = self.inner.lock() {
            let entry = guard.sub_caps.entry(key.into()).or_default();
            entry.cap = cap;
        }
    }

    /// Raise `total_cap` by `extra_calls` (for user `Continue (+N calls)` / `budget_delta`).
    pub fn raise_cap(&self, extra_calls: u64) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.total_cap = guard.total_cap.saturating_add(extra_calls);
        }
    }

    /// Update the frontier and verifier reserves (e.g. clearing `verifier_reserve` once closing review finishes).
    pub fn set_reserves(&self, frontier_reserve: u64, verifier_reserve: u64) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.frontier_reserve = frontier_reserve;
            guard.verifier_reserve = verifier_reserve;
        }
    }

    /// Read a consistent snapshot of all counters and available headroom.
    pub fn snapshot(&self) -> BudgetSnapshot {
        self.inner
            .lock()
            .map(|g| g.snapshot())
            .unwrap_or(BudgetSnapshot {
                total_cap: 0,
                committed: 0,
                in_flight: 0,
                frontier_reserve: 0,
                verifier_reserve: 0,
                unspent_verifier_reserve: 0,
                verifier_committed: 0,
                verifier_in_flight: 0,
                available_for_workers: 0,
                available_for_frontier: 0,
                available_for_verifier: 0,
                sub_caps: BTreeMap::new(),
                initial_breakdown: ModelCallBreakdown::default(),
            })
    }

    /// View current committed + in-flight usage as an F9 [`ModelCallBudget`].
    pub fn to_model_call_budget(&self) -> ModelCallBudget {
        let snap = self.snapshot();
        ModelCallBudget::new(snap.total_cap, snap.committed.saturating_add(snap.in_flight))
    }

    /// Atomically check headroom under a single mutex and reserve 1 in-flight call slot.
    ///
    /// Guarantees `committed + in_flight + reserved_for_frontier_and_verifier < total_cap`
    /// before any HTTP bytes leave the host.
    pub fn try_acquire(&self, caller: PoolCaller) -> Result<BudgetLease, Box<BudgetExhausted>> {
        let mut guard = self.inner.lock().map_err(|_| {
            Box::new(BudgetExhausted {
                caller: caller.clone(),
                scope: ExhaustedScope::LockPoisoned,
                committed: 0,
                in_flight: 0,
                reserved: 0,
                total_cap: 0,
                sub_cap: None,
            })
        })?;

        let reserved = guard.reserved_for(&caller);
        let used_plus_reserved = guard
            .committed
            .saturating_add(guard.in_flight)
            .saturating_add(reserved);
        if used_plus_reserved >= guard.total_cap {
            let scope = if guard.committed.saturating_add(guard.in_flight) >= guard.total_cap {
                ExhaustedScope::TotalCap
            } else {
                ExhaustedScope::ReservedHeadroom
            };
            return Err(Box::new(BudgetExhausted {
                caller,
                scope,
                committed: guard.committed,
                in_flight: guard.in_flight,
                reserved,
                total_cap: guard.total_cap,
                sub_cap: None,
            }));
        }

        if let Some(key) = caller.sub_key() {
            if !guard.sub_caps.contains_key(key) {
                if let Some(default_cap) = guard.default_sub_cap {
                    guard.sub_caps.insert(key.to_string(), SubCap::new(default_cap));
                }
            }
            if let Some(sub) = guard.sub_caps.get(key).copied() {
                if sub.used() >= sub.cap {
                    return Err(Box::new(BudgetExhausted {
                        caller: caller.clone(),
                        scope: ExhaustedScope::SubCap(key.to_string()),
                        committed: guard.committed,
                        in_flight: guard.in_flight,
                        reserved,
                        total_cap: guard.total_cap,
                        sub_cap: Some(sub),
                    }));
                }
            }
        }

        guard.in_flight = guard.in_flight.saturating_add(1);
        if caller.is_verifier() {
            guard.verifier_in_flight = guard.verifier_in_flight.saturating_add(1);
        }
        if let Some(key) = caller.sub_key() {
            if let Some(sub) = guard.sub_caps.get_mut(key) {
                sub.in_flight = sub.in_flight.saturating_add(1);
            }
        }

        Ok(BudgetLease { state: Arc::clone(&self.inner), caller, active: true })
    }
}

#[cfg(test)]
#[path = "budget_pool_tests.rs"]
mod tests;
