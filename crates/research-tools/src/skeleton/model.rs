//! Data model for the deterministic paper skeleton (Investigate layer 1, section 4.1).
//!
//! All types are pure, serde-serializable with stable field order, and independent of
//! model calls or ledger access. Tags (`SkNode::tag`), `parent`, `proof_of`, edges,
//! computed counters, reading stats, and bibliography resolution are populated in V2;
//! V1 leaves tags unset (`None`) and never calls `paper_key::node_tag`.

use crate::paper_key::PaperKey;
use crate::tex_scan::strip_latex_comments_line;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SKELETON_SCHEMA_VERSION: u32 = 1;
pub const MAX_SKELETON_NODES: usize = 6_000;
pub const MAX_SKELETON_EDGES: usize = 20_000;
pub const MAX_SKELETON_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const NODE_FP_HEX_CHARS: usize = 12;

/// Which deterministic builder produced a [`PaperSkeleton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkeletonBuilder {
    #[serde(rename = "tex-1")]
    Tex1,
    #[serde(rename = "html-1")]
    Html1,
    #[serde(rename = "pdf-1")]
    Pdf1,
}

impl SkeletonBuilder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tex1 => "tex-1",
            Self::Html1 => "html-1",
            Self::Pdf1 => "pdf-1",
        }
    }
}

/// Overall structural fidelity of a built skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkeletonQuality {
    Full,
    Partial,
    Weak,
}

/// Provenance of `SkNode::number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberBasis {
    Computed,
    Html,
    Printed,
    Confirmed,
}

/// Structural node kinds defined in Investigate section 4.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkNodeKind {
    FrontMatter,
    Section,
    Definition,
    Assumption,
    Theorem,
    Lemma,
    Proposition,
    Corollary,
    Conjecture,
    Claim,
    Proof,
    Example,
    Remark,
    Equation,
    Figure,
    Table,
    Algorithm,
    Bibliography,
}

impl SkNodeKind {
    /// Whether this kind represents a theorem-like statement environment
    /// (excluding proofs, sections, equations, figures, tables, front/back matter).
    pub fn is_theorem_like(self) -> bool {
        matches!(
            self,
            Self::Definition
                | Self::Assumption
                | Self::Theorem
                | Self::Lemma
                | Self::Proposition
                | Self::Corollary
                | Self::Conjecture
                | Self::Claim
                | Self::Example
                | Self::Remark
                | Self::Algorithm
        )
    }

    /// Whether a plain adjacent `\begin{proof}` can attach to a node of this kind.
    pub fn is_provable(self) -> bool {
        matches!(
            self,
            Self::Theorem
                | Self::Lemma
                | Self::Proposition
                | Self::Corollary
                | Self::Conjecture
                | Self::Claim
        )
    }
}

/// How a skeleton node was recognized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkConfidence {
    TexEnv,
    TexHeading,
    HtmlClass,
    PdfHeuristic,
}

/// A 1-based inclusive line range in one physical ingested source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Span {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

impl Span {
    pub fn new(path: impl Into<String>, start_line: usize, end_line: usize) -> Self {
        Self { path: path.into(), start_line, end_line }
    }

    /// Returns true iff `other` lies in the same file and within `self`'s inclusive line range.
    pub fn contains(&self, other: &Span) -> bool {
        self.path == other.path
            && self.start_line <= other.start_line
            && other.end_line <= self.end_line
    }
}

/// One node in the deterministic skeleton.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkNode {
    /// Permanent tag (`n-...`), assigned in V2 (`skeleton/tags.rs`). Unset in V1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub kind: SkNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_basis: Option<NumberBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Parent node's tag, populated in V2 once tags exist. Unset in V1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// 0-based document order index.
    pub order: usize,
    /// Physical file spans covered by this node (in reading order).
    pub spans: Vec<Span>,
    /// For `Proof` nodes, the target node's tag (populated in V2). Unset in V1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_of: Option<String>,
    /// Bibliography keys cited inside the environment/section heading.
    #[serde(default)]
    pub head_cites: Vec<String>,
    /// Whether the abstract or introduction refers to this node (populated in V2).
    #[serde(default)]
    pub intro_refs: bool,
    /// First 12 lower-case hex characters of SHA-256 of the comment- and whitespace-normalised body.
    pub fp: String,
    #[serde(default)]
    pub words: u32,
    /// Count of unlabelled display equations directly inside this node.
    #[serde(default)]
    pub eq_count: u32,
    #[serde(default)]
    pub read_minutes: f64,
    pub confidence: SkConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkEdgeKind {
    Contains,
    Uses,
    Proves,
    Cites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkEdgeRole {
    Statement,
    Proof,
    Prose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkEdgeRule {
    Ref,
    Eqref,
    Cref,
    ProofHeader,
    ProofAdjacent,
    Cite,
    HeadCite,
    TextMention,
    Lifted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeConfidence {
    Explicit,
    Heuristic,
}

/// Directed dependency edge (`from` = prerequisite / thing used, `to` = dependent / user).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkEdge {
    pub from: String,
    pub to: String,
    pub kind: SkEdgeKind,
    pub role: SkEdgeRole,
    pub site: Span,
    pub rule: SkEdgeRule,
    pub confidence: EdgeConfidence,
}

/// One citation locator such as `[NS06, Theorem 2.1]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibLocator {
    pub site: Span,
    pub text: String,
}

/// One bibliography entry discovered in the source (populated in V2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibRef {
    pub key: String,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arxiv_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub cited_at: Vec<Span>,
    #[serde(default)]
    pub locators: Vec<BibLocator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolDefHow {
    Newcommand,
    DefinePhrase,
}

/// One symbol definition discovered in the source (populated in V5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDef {
    pub symbol_tex: String,
    pub norm: String,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_node: Option<String>,
    pub how: SymbolDefHow,
}

/// One TeX macro passed to the math renderer; never expanded into source text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroDef {
    pub name: String,
    pub arity: u8,
    pub body: String,
}

/// Per-section reading statistics (populated in V2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_tag: Option<String>,
    pub words: u32,
    pub read_minutes: f64,
    #[serde(default)]
    pub counts: BTreeMap<SkNodeKind, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarryBasis {
    Label,
    Text,
    Similar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarryMatch {
    pub tag: String,
    pub basis: CarryBasis,
    pub changed: bool,
}

/// Tag carry-over report between paper versions or builder upgrades (populated in V10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Carry {
    pub prev_skeleton_seq: u64,
    #[serde(default)]
    pub matched: Vec<CarryMatch>,
    #[serde(default)]
    pub retired: Vec<String>,
    #[serde(default)]
    pub added: Vec<String>,
}

/// Top-level deterministic skeleton of one ingested source receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperSkeleton {
    pub schema: u32,
    pub builder: SkeletonBuilder,
    pub source_id: String,
    pub source_seq: u64,
    pub files_root_sha256: String,
    pub paper_key: PaperKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub quality: SkeletonQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_reason: Option<String>,
    #[serde(default)]
    pub nodes: Vec<SkNode>,
    #[serde(default)]
    pub edges: Vec<SkEdge>,
    #[serde(default)]
    pub bib: Vec<BibRef>,
    #[serde(default)]
    pub symbols: Vec<SymbolDef>,
    #[serde(default)]
    pub macros: Vec<MacroDef>,
    #[serde(default)]
    pub stats: Vec<SectionStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carry: Option<Carry>,
}

/// Deterministic root hash over a source receipt's `files_sha256` map.
///
/// Entries are sorted by path (guaranteed by `BTreeMap`) and length-prefixed so the digest
/// is unambiguous and independent of filesystem traversal order.
pub fn compute_files_root_sha256(files_sha256: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"re-search.files-root.v1\0");
    for (path, digest) in files_sha256 {
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update((digest.len() as u64).to_le_bytes());
        hasher.update(digest.as_bytes());
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Computes `SkNode::fp`: first 12 lower-case hex characters of SHA-256 of the
/// comment-stripped and whitespace-normalised node body.
///
/// Invariant to CRLF vs LF line endings and comment-only edits (rule 6.2).
pub fn compute_body_fp(raw_body: &str) -> String {
    let mut normalized = String::with_capacity(raw_body.len().min(4096));
    let mut first = true;
    for raw_line in raw_body.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let stripped = strip_latex_comments_line(line);
        for token in stripped.split_whitespace() {
            if !first {
                normalized.push(' ');
            }
            normalized.push_str(token);
            first = false;
        }
    }
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(NODE_FP_HEX_CHARS);
    for byte in &digest[..NODE_FP_HEX_CHARS / 2] {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
