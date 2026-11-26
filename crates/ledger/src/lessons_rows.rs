//! Plain row types for `<data_dir>/library/lessons.db` (package F16a).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermAliasRow {
    pub term_a: String,
    pub term_b: String,
    pub decision: String,
    pub origin_session: String,
    pub origin_node_json: String,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolDefaultRow {
    pub symbol: String,
    pub domain: String,
    pub assumptions_json: String,
    pub dimensions: Option<String>,
    pub convention_note: Option<String>,
    pub origin_session: String,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConventionDefaultRow {
    pub conv_key: String,
    pub conv_value: String,
    pub domain: String,
    pub origin_session: String,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkeletonOverrideRow {
    pub node_tag: String,
    pub paper_key: Option<String>,
    pub kind_override: Option<String>,
    pub end_line_override: Option<i64>,
    pub note: String,
    pub origin_session: String,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOverlayRow {
    pub file_sha256: String,
    pub line_no: i64,
    pub corrected_text: String,
    pub original_sha256: Option<String>,
    pub origin_session: String,
    pub updated_ms: i64,
}

/// All F16a user correction rows for atomic backup/restore.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LessonsSnapshot {
    pub term_aliases: Vec<TermAliasRow>,
    pub symbol_defaults: Vec<SymbolDefaultRow>,
    pub convention_defaults: Vec<ConventionDefaultRow>,
    pub skeleton_overrides: Vec<SkeletonOverrideRow>,
    pub ocr_overlays: Vec<OcrOverlayRow>,
}
