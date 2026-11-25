//! Dependency closure and topological ordering over declared (`depends_on`) plus
//! host-derived (`derived_depends_on`) proof-graph edges (package S2).
//!
//! Works uniformly on both [`ProofEntry`] (declared edges only) and [`SolveEntry`]
//! (declared plus derived edges), without modifying `proof_dag.rs`.

use crate::proof_dag::{tool_reference_matches, ProofEntry};
use crate::solve_model::{Claim, HandleRef, SolveEntry, SolveExpr};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::cmp::Reverse;

/// Any proof-graph node exposing an id, declared dependencies, and optional derived dependencies.
pub trait DepNode {
    fn entry_id(&self) -> &str;
    fn declared_deps(&self) -> &[String];
    fn derived_deps(&self) -> &[String] {
        &[]
    }
}

impl DepNode for ProofEntry {
    fn entry_id(&self) -> &str {
        &self.entry_id
    }
    fn declared_deps(&self) -> &[String] {
        &self.depends_on
    }
}

impl DepNode for SolveEntry {
    fn entry_id(&self) -> &str {
        &self.entry_id
    }
    fn declared_deps(&self) -> &[String] {
        &self.depends_on
    }
    fn derived_deps(&self) -> &[String] {
        &self.derived_depends_on
    }
}

impl<T: DepNode> DepNode for &T {
    fn entry_id(&self) -> &str {
        (*self).entry_id()
    }
    fn declared_deps(&self) -> &[String] {
        (*self).declared_deps()
    }
    fn derived_deps(&self) -> &[String] {
        (*self).derived_deps()
    }
}

/// Deduplicated union of declared (`depends_on`) and derived (`derived_depends_on`) edges
/// for one node, preserving deterministic order (declared first, then derived).
pub fn all_deps<N: DepNode>(node: &N) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(node.declared_deps().len() + node.derived_deps().len());
    for dep in node.declared_deps().iter().chain(node.derived_deps().iter()) {
        if seen.insert(dep.as_str()) {
            out.push(dep.as_str());
        }
    }
    out
}

/// All existing node ids reachable backwards from `targets` over `depends_on ∪ derived_depends_on`,
/// returned in the original slice order (cycle-safe).
pub fn closure_ids<N: DepNode>(nodes: &[N], targets: &[&str]) -> Vec<String> {
    let visited = collect_reachable(nodes, targets);
    nodes
        .iter()
        .filter(|n| visited.contains(n.entry_id()))
        .map(|n| n.entry_id().to_string())
        .collect()
}

/// All proper ancestors of `target` (excluding `target` itself) over `depends_on ∪ derived_depends_on`,
/// returned in the original slice order (cycle-safe).
pub fn proper_ancestor_ids<N: DepNode>(nodes: &[N], target: &str) -> Vec<String> {
    let Some(node) = nodes.iter().find(|n| n.entry_id() == target) else {
        return Vec::new();
    };
    let direct: Vec<&str> = all_deps(node);
    let mut visited = collect_reachable(nodes, &direct);
    visited.remove(target);
    nodes
        .iter()
        .filter(|n| visited.contains(n.entry_id()))
        .map(|n| n.entry_id().to_string())
        .collect()
}

/// References to all nodes in the transitive closure of `targets`, in original slice order.
pub fn closure_entries<'a, N: DepNode>(nodes: &'a [N], targets: &[&str]) -> Vec<&'a N> {
    let visited = collect_reachable(nodes, targets);
    nodes.iter().filter(|n| visited.contains(n.entry_id())).collect()
}

/// Closure of one target id, returning an error if `target_id` is absent from `nodes`.
pub fn closure_for_target<N: DepNode>(nodes: &[N], target_id: &str) -> Result<Vec<String>, String> {
    if !nodes.iter().any(|n| n.entry_id() == target_id) {
        return Err(format!("proof entry `{target_id}` not found"));
    }
    Ok(closure_ids(nodes, &[target_id]))
}

fn collect_reachable<N: DepNode>(nodes: &[N], targets: &[&str]) -> BTreeSet<String> {
    let by_id: BTreeMap<&str, &N> = nodes.iter().map(|n| (n.entry_id(), n)).collect();
    let mut visited = BTreeSet::new();
    let mut stack: Vec<&str> = targets.to_vec();
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur.to_string()) {
            continue;
        }
        if let Some(node) = by_id.get(cur) {
            for dep in all_deps(*node) {
                if !visited.contains(dep) {
                    stack.push(dep);
                }
            }
        }
    }
    visited
}

/// Deterministic topological order (prerequisites before dependents) over `depends_on ∪ derived_depends_on`.
///
/// Dependencies that do not name a node in `nodes` are ignored for ordering.
/// Returns `Ok(ordered_ids)` for an acyclic graph, or `Err(remaining_ids)` listing all nodes
/// involved in or blocked by a cycle.
pub fn topological_order<N: DepNode>(nodes: &[N]) -> Result<Vec<String>, Vec<String>> {
    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (idx, n) in nodes.iter().enumerate() {
        index_of.insert(n.entry_id(), idx);
    }

    let count = nodes.len();
    let mut in_degree = vec![0usize; count];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];

    for (idx, n) in nodes.iter().enumerate() {
        for dep in all_deps(n) {
            if let Some(&dep_idx) = index_of.get(dep) {
                dependents[dep_idx].push(idx);
                in_degree[idx] += 1;
            }
        }
    }

    // Min-heap on input index keeps append order when nodes are independent.
    let mut ready: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    for (idx, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            ready.push(Reverse(idx));
        }
    }

    let mut ordered = Vec::with_capacity(count);
    let mut emitted = vec![false; count];
    while let Some(Reverse(idx)) = ready.pop() {
        emitted[idx] = true;
        ordered.push(nodes[idx].entry_id().to_string());
        for &child in &dependents[idx] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                ready.push(Reverse(child));
            }
        }
    }

    if ordered.len() == count {
        Ok(ordered)
    } else {
        let cyclic: Vec<String> = nodes
            .iter()
            .enumerate()
            .filter(|(idx, _)| !emitted[*idx])
            .map(|(_, n)| n.entry_id().to_string())
            .collect();
        Err(cyclic)
    }
}

/// Compute host-derived dependencies for a candidate step (`solve.md` 4.4, rule R9):
/// the `owner_entry` of every handle named in `claims`, plus any existing entry that owns
/// a receipt cited in `tool_call_refs` (matched by `call_id` or `seq`).
/// Excludes `exclude_entry_id` (if provided) and deduplicates in deterministic order.
pub fn compute_derived_deps(
    claims: &[Claim],
    tool_call_refs: &[String],
    handles: &BTreeMap<HandleRef, SolveExpr>,
    entries: &[SolveEntry],
    exclude_entry_id: Option<&str>,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut push = |owner: &str| {
        if exclude_entry_id == Some(owner) || owner.is_empty() {
            return;
        }
        if seen.insert(owner.to_string()) {
            out.push(owner.to_string());
        }
    };

    // 1. Owner steps of every handle named in claims.
    for claim in claims {
        for handle in [&claim.lhs, &claim.rhs] {
            if let Some(expr) = handles.get(handle) {
                if let Some(owner) = expr.owner_entry.as_deref() {
                    push(owner);
                }
            }
        }
    }

    // 2. Owner steps of every receipt cited in `tool_call_refs`.
    for reference in tool_call_refs {
        // Check handles whose producing receipt matches `reference`.
        for expr in handles.values() {
            if tool_reference_matches(reference, &expr.produced_by.call_id, expr.produced_by.result_seq)
            {
                if let Some(owner) = expr.owner_entry.as_deref() {
                    push(owner);
                }
            }
        }
        // Check earlier entries that already recorded the same receipt reference.
        for entry in entries {
            let entry_cites = entry
                .tool_call_ref
                .iter()
                .chain(entry.tool_call_refs.iter())
                .any(|existing| existing.trim() == reference.trim());
            if entry_cites {
                push(&entry.entry_id);
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "proof_closure_tests.rs"]
mod tests;
