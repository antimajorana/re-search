//! Comment-safe LaTeX structural scanner with an environment stack.
//!
//! Scans a [`FlattenedTex`] stream into [`SkNode`]s (sections, front matter, theorem-like
//! environments, proofs, labelled equations, figures, tables, bibliography), preserving
//! physical-file spans via the origin map and recording quality notes on malformed TeX
//! without ever panicking or looping.

use super::flatten::{is_nested_tex_if, FlattenedTex, VERBATIM_ENVS};
use super::model::{
    compute_body_fp, MacroDef, SkConfidence, SkNode, SkNodeKind, SkeletonQuality,
    MAX_SKELETON_NODES,
};
pub use super::tex_lex::ProofHint;
use super::tex_lex::{
    has_active_begin_document, next_char_boundary, read_balanced_group, skip_inline_verb,
    skip_ws_and_comments, try_parse_macro_cmd, GroupOutcome,
};
use super::tex_math::{MULTIROW_MATH_ENVS, SINGLE_MATH_ENVS};
use super::tex_theorems::{try_parse_theorem_cmd, TheoremTable};
use std::collections::BTreeMap;

const FIGURE_ENVS: &[&str] = &["figure", "figure*", "wrapfigure", "SCfigure"];
const TABLE_ENVS: &[&str] = &["table", "table*", "wraptable", "longtable", "sidewaystable", "SCtable"];

/// Output of scanning one flattened TeX stream.
#[derive(Debug, Clone)]
pub struct TexScanOutput {
    pub nodes: Vec<SkNode>,
    pub parent_orders: Vec<Option<usize>>,
    pub proof_hints: BTreeMap<usize, ProofHint>,
    pub theorem_table: TheoremTable,
    pub macros: Vec<MacroDef>,
    pub quality: SkeletonQuality,
    pub quality_notes: Vec<String>,
}

pub(super) struct SectionFrame {
    pub(super) level: u8,
    pub(super) order: usize,
    pub(super) start_flat: usize,
    pub(super) max_child_end: usize,
    pub(super) env_depth: usize,
}

pub(super) enum EnvFrame {
    Document,
    Node { env_name: String, order: usize, start_flat: usize, max_child_end: usize },
    Math { env_name: String, multi_row: bool, start_flat: usize, body_start: usize, parent: Option<usize> },
    Wrapper { env_name: String, start_flat: usize },
}

impl EnvFrame {
    pub(super) fn env_name(&self) -> &str {
        match self {
            Self::Document => "document",
            Self::Node { env_name, .. }
            | Self::Math { env_name, .. }
            | Self::Wrapper { env_name, .. } => env_name,
        }
    }
}

/// Scans `flattened` into structural nodes, parent indices, proof hints, theorem table, and macros.
pub fn scan_tex_stream(flattened: &FlattenedTex) -> TexScanOutput {
    let (stream, line_starts) = join_flattened_lines(flattened);
    let has_document_env = has_active_begin_document(&stream);

    let mut st = Scanner {
        flattened,
        stream: &stream,
        line_starts: &line_starts,
        nodes: Vec::new(),
        parent_orders: Vec::new(),
        proof_hints: BTreeMap::new(),
        theorem_table: TheoremTable::new(),
        macros: Vec::new(),
        quality_notes: flattened.warnings.clone(),
        section_stack: Vec::new(),
        env_stack: Vec::new(),
        in_body: !has_document_env,
        saw_document_end: false,
        node_limit_hit: false,
        pending_else_fi: 0,
        last_provable_by_parent: BTreeMap::new(),
    };

    st.run();
    st.finish_all();

    let quality = if st.nodes.is_empty() {
        if st.quality_notes.is_empty() {
            st.quality_notes.push("no TeX structural elements found".to_string());
        }
        SkeletonQuality::Weak
    } else if !st.quality_notes.is_empty() {
        SkeletonQuality::Partial
    } else {
        SkeletonQuality::Full
    };

    TexScanOutput {
        nodes: st.nodes,
        parent_orders: st.parent_orders,
        proof_hints: st.proof_hints,
        theorem_table: st.theorem_table,
        macros: st.macros,
        quality,
        quality_notes: st.quality_notes,
    }
}

pub(super) struct Scanner<'a> {
    pub(super) flattened: &'a FlattenedTex,
    pub(super) stream: &'a str,
    pub(super) line_starts: &'a [usize],
    pub(super) nodes: Vec<SkNode>,
    pub(super) parent_orders: Vec<Option<usize>>,
    pub(super) proof_hints: BTreeMap<usize, ProofHint>,
    pub(super) theorem_table: TheoremTable,
    pub(super) macros: Vec<MacroDef>,
    pub(super) quality_notes: Vec<String>,
    pub(super) section_stack: Vec<SectionFrame>,
    pub(super) env_stack: Vec<EnvFrame>,
    pub(super) in_body: bool,
    pub(super) saw_document_end: bool,
    pub(super) node_limit_hit: bool,
    pub(super) pending_else_fi: usize,
    pub(super) last_provable_by_parent: BTreeMap<Option<usize>, Option<usize>>,
}

impl<'a> Scanner<'a> {
    pub(super) fn flat_line_at(&self, byte_pos: usize) -> usize {
        if self.line_starts.is_empty() {
            return 1;
        }
        let clamped = byte_pos.min(self.stream.len());
        self.line_starts.partition_point(|&start| start <= clamped).max(1)
    }

    pub(super) fn loc_str(&self, flat_line: usize) -> String {
        match self.flattened.origin_of(flat_line) {
            Some(o) => format!("{}:{}", o.path, o.line),
            None => format!("flat:{flat_line}"),
        }
    }

    pub(super) fn current_parent(&self) -> Option<usize> {
        for frame in self.env_stack.iter().rev() {
            if let EnvFrame::Node { order, .. } = frame {
                return Some(*order);
            }
        }
        self.section_stack.last().map(|s| s.order)
    }

    pub(super) fn record_child_end(&mut self, end_flat: usize) {
        for frame in self.env_stack.iter_mut().rev() {
            if let EnvFrame::Node { max_child_end, .. } = frame {
                *max_child_end = (*max_child_end).max(end_flat);
            }
        }
        for sec in &mut self.section_stack {
            sec.max_child_end = sec.max_child_end.max(end_flat);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn push_node(
        &mut self,
        kind: SkNodeKind,
        env_name: Option<String>,
        title: Option<String>,
        label: Option<String>,
        parent: Option<usize>,
        start_flat: usize,
        end_flat: usize,
        head_cites: Vec<String>,
        confidence: SkConfidence,
    ) -> Option<usize> {
        if self.nodes.len() >= MAX_SKELETON_NODES {
            if !self.node_limit_hit {
                self.node_limit_hit = true;
                self.quality_notes.push(format!("node limit ({MAX_SKELETON_NODES}) reached"));
            }
            return None;
        }
        let order = self.nodes.len();
        let end = end_flat.max(start_flat);
        self.nodes.push(SkNode {
            tag: None,
            kind,
            number: None,
            number_basis: None,
            env_name,
            title,
            label,
            parent: None,
            order,
            spans: self.flattened.spans_for_range(start_flat, end),
            proof_of: None,
            head_cites,
            intro_refs: false,
            fp: compute_body_fp(&self.flattened.raw_range_text(start_flat, end)),
            words: 0,
            eq_count: 0,
            read_minutes: 0.0,
            confidence,
        });
        self.parent_orders.push(parent);
        if kind.is_provable() {
            self.last_provable_by_parent.insert(parent, Some(order));
        } else if kind.is_theorem_like() || kind == SkNodeKind::Proof {
            self.last_provable_by_parent.insert(parent, None);
        }
        Some(order)
    }

    pub(super) fn push_env_node(
        &mut self,
        env_name: String,
        kind: SkNodeKind,
        title: Option<String>,
        head_cites: Vec<String>,
        flat_line: usize,
    ) -> Option<usize> {
        let parent = self.current_parent();
        let order = self.push_node(
            kind,
            Some(env_name.clone()),
            title,
            None,
            parent,
            flat_line,
            flat_line,
            head_cites,
            SkConfidence::TexEnv,
        )?;
        self.env_stack.push(EnvFrame::Node {
            env_name,
            order,
            start_flat: flat_line,
            max_child_end: flat_line,
        });
        Some(order)
    }

    pub(super) fn finalize_node_range(&mut self, order: usize, start_flat: usize, end_flat: usize) {
        let end = end_flat.max(start_flat);
        if let Some(node) = self.nodes.get_mut(order) {
            node.spans = self.flattened.spans_for_range(start_flat, end);
            node.fp = compute_body_fp(&self.flattened.raw_range_text(start_flat, end));
        }
        self.record_child_end(end);
    }

    fn run(&mut self) {
        let bytes = self.stream.as_bytes();
        let mut pos = 0usize;
        while pos < bytes.len() && !self.saw_document_end {
            let start_pos = pos;
            let b = bytes[pos];
            if b == b'%' {
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            if b == b'$' && bytes.get(pos + 1) == Some(&b'$') && self.in_body && !self.in_math_env() {
                pos = self.handle_display_dollars(pos);
                continue;
            }
            if b == b'\\' {
                pos = self.handle_backslash(pos);
                if pos <= start_pos {
                    pos = next_char_boundary(self.stream, start_pos);
                }
                continue;
            }
            pos = next_char_boundary(self.stream, pos);
        }
    }

    fn in_math_env(&self) -> bool {
        self.env_stack.iter().any(|f| matches!(f, EnvFrame::Math { .. }))
    }

    fn handle_backslash(&mut self, pos: usize) -> usize {
        let bytes = self.stream.as_bytes();
        if pos + 1 >= bytes.len() {
            return bytes.len();
        }
        if bytes[pos + 1] == b'[' && self.in_body && !self.in_math_env() {
            return self.handle_display_brackets(pos);
        }
        if !bytes[pos + 1].is_ascii_alphabetic() {
            return next_char_boundary(self.stream, pos + 1);
        }
        let mut cmd_end = pos + 1;
        while cmd_end < bytes.len() && bytes[cmd_end].is_ascii_alphabetic() {
            cmd_end += 1;
        }
        let cmd = &self.stream[pos + 1..cmd_end];

        if cmd == "verb" {
            return skip_inline_verb(self.stream, cmd_end);
        }
        if cmd == "iffalse" {
            return self.skip_iffalse_block(pos, cmd_end);
        }
        if cmd == "fi" && self.pending_else_fi > 0 {
            self.pending_else_fi -= 1;
            return cmd_end;
        }

        if !self.in_math_env() {
            if let Some((next_pos, note)) =
                try_parse_theorem_cmd(cmd, self.stream, cmd_end, &mut self.theorem_table)
            {
                if let Some(msg) = note {
                    let loc = self.loc_str(self.flat_line_at(pos));
                    self.quality_notes.push(format!("{msg} at {loc}"));
                }
                return next_pos.max(cmd_end);
            }
            if let Some((next_pos, note)) =
                try_parse_macro_cmd(cmd, self.stream, cmd_end, &mut self.macros)
            {
                if let Some(msg) = note {
                    let loc = self.loc_str(self.flat_line_at(pos));
                    self.quality_notes.push(format!("{msg} at {loc}"));
                }
                return next_pos.max(cmd_end);
            }
        }

        match cmd {
            "begin" => self.handle_begin(pos, cmd_end),
            "end" => self.handle_end(pos, cmd_end),
            "part" | "chapter" | "section" | "subsection" | "subsubsection"
                if self.in_body && !self.in_math_env() =>
            {
                self.handle_section(cmd, pos, cmd_end)
            }
            "label" if self.in_body && !self.in_math_env() => self.handle_label(pos, cmd_end),
            "caption" if self.in_body && !self.in_math_env() => self.handle_caption(pos, cmd_end),
            "bibliography" | "printbibliography" if self.in_body && !self.in_math_env() => {
                self.handle_bib_cmd(cmd, pos, cmd_end)
            }
            _ => cmd_end,
        }
    }

    fn skip_iffalse_block(&mut self, start_pos: usize, after_cmd: usize) -> usize {
        let bytes = self.stream.as_bytes();
        let mut pos = after_cmd;
        let mut depth = 1usize;
        while pos < bytes.len() && depth > 0 {
            if bytes[pos] == b'%' {
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            if bytes[pos] == b'\\' {
                if pos + 1 >= bytes.len() {
                    return bytes.len();
                }
                if !bytes[pos + 1].is_ascii_alphabetic() {
                    pos = next_char_boundary(self.stream, pos + 1);
                    continue;
                }
                let mut end = pos + 1;
                while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                    end += 1;
                }
                let w = &self.stream[pos + 1..end];
                if w == "fi" {
                    depth -= 1;
                } else if w == "else" && depth == 1 {
                    self.pending_else_fi += 1;
                    return end;
                } else if is_nested_tex_if(w) {
                    depth += 1;
                }
                pos = end;
                continue;
            }
            pos = next_char_boundary(self.stream, pos);
        }
        if depth > 0 {
            let loc = self.loc_str(self.flat_line_at(start_pos));
            self.quality_notes.push(format!("unclosed \\iffalse block at {loc}"));
        }
        pos
    }

    fn handle_begin(&mut self, begin_pos: usize, cmd_end: usize) -> usize {
        let p = skip_ws_and_comments(self.stream, cmd_end);
        if self.stream.as_bytes().get(p) != Some(&b'{') {
            return cmd_end;
        }
        let (env_name, mut pos) = match read_balanced_group(self.stream, p, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => (content.trim().to_string(), next_pos),
            GroupOutcome::Unclosed { next_pos, .. } => {
                let loc = self.loc_str(self.flat_line_at(begin_pos));
                self.quality_notes.push(format!("unclosed `{{` in \\begin at {loc}"));
                return next_pos;
            }
        };
        let flat_line = self.flat_line_at(begin_pos);

        if VERBATIM_ENVS.contains(&env_name.as_str()) {
            let marker = format!("\\end{{{env_name}}}");
            return match self.stream[pos..].find(&marker) {
                Some(rel) => pos + rel + marker.len(),
                None => {
                    let loc = self.loc_str(flat_line);
                    self.quality_notes
                        .push(format!("unclosed verbatim environment `{env_name}` at {loc}"));
                    self.stream.len()
                }
            };
        }
        if env_name == "document" {
            self.in_body = true;
            self.env_stack.push(EnvFrame::Document);
            return pos;
        }
        if !self.in_body {
            return pos;
        }
        if self.in_math_env() {
            self.env_stack.push(EnvFrame::Wrapper { env_name, start_flat: flat_line });
            return pos;
        }

        let is_single = SINGLE_MATH_ENVS.contains(&env_name.as_str());
        let is_multi = MULTIROW_MATH_ENVS.contains(&env_name.as_str());
        if is_single || is_multi {
            if env_name.starts_with("alignat") {
                let q = skip_ws_and_comments(self.stream, pos);
                if self.stream.as_bytes().get(q) == Some(&b'{') {
                    if let GroupOutcome::Closed { next_pos, .. } =
                        read_balanced_group(self.stream, q, b'{', b'}', false)
                    {
                        pos = next_pos;
                    }
                }
            }
            let parent = self.current_parent();
            self.env_stack.push(EnvFrame::Math {
                env_name,
                multi_row: is_multi,
                start_flat: flat_line,
                body_start: pos,
                parent,
            });
            return pos;
        }

        if env_name == "abstract" {
            self.push_env_node(
                env_name,
                SkNodeKind::FrontMatter,
                Some("Abstract".into()),
                Vec::new(),
                flat_line,
            );
            return pos;
        }
        if env_name == "thebibliography" {
            self.close_non_container_envs(flat_line.saturating_sub(1).max(1));
            self.close_sections_at_level(0, flat_line);
            self.push_env_node(
                env_name,
                SkNodeKind::Bibliography,
                Some("References".into()),
                Vec::new(),
                flat_line,
            );
            return pos;
        }
        if let Some(kind) = float_kind(&env_name) {
            let q = skip_ws_and_comments(self.stream, pos);
            if self.stream.as_bytes().get(q) == Some(&b'[') {
                if let GroupOutcome::Closed { next_pos, .. } =
                    read_balanced_group(self.stream, q, b'[', b']', false)
                {
                    pos = next_pos;
                }
            }
            self.push_env_node(env_name, kind, None, Vec::new(), flat_line);
            return pos;
        }
        if self.theorem_table.is_proof_env(&env_name) {
            return self.open_proof_env(env_name, flat_line, pos);
        }
        if let Some(lookup) = self.theorem_table.lookup(&env_name) {
            let (title, head_cites, next_pos) = self.read_optional_heading(pos, flat_line);
            self.push_env_node(env_name, lookup.kind, title, head_cites, flat_line);
            return next_pos;
        }

        self.env_stack.push(EnvFrame::Wrapper { env_name, start_flat: flat_line });
        pos
    }
}

fn float_kind(env_name: &str) -> Option<SkNodeKind> {
    if FIGURE_ENVS.contains(&env_name) {
        Some(SkNodeKind::Figure)
    } else if TABLE_ENVS.contains(&env_name) {
        Some(SkNodeKind::Table)
    } else {
        None
    }
}

fn join_flattened_lines(flattened: &FlattenedTex) -> (String, Vec<usize>) {
    let mut stream = String::new();
    let mut starts = Vec::with_capacity(flattened.lines.len());
    for (idx, fl) in flattened.lines.iter().enumerate() {
        if idx > 0 {
            stream.push('\n');
        }
        starts.push(stream.len());
        stream.push_str(&fl.text);
    }
    (stream, starts)
}
