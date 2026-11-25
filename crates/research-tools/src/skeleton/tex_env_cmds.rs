//! Command handlers and stack-closing helpers for [`super::tex_env::Scanner`].

use super::model::{compute_body_fp, NumberBasis, SkConfidence, SkNodeKind};
use super::tex_env::{EnvFrame, Scanner, SectionFrame};
use super::tex_lex::{
    classify_proof_hint, clean_tex_title, extract_unique_cites, read_balanced_group,
    skip_ws_and_comments, GroupOutcome,
};
use super::tex_math::{
    find_closing_display_bracket, find_closing_double_dollar, inspect_math_body,
};

impl<'a> Scanner<'a> {
    pub(super) fn open_proof_env(&mut self, env_name: String, flat_line: usize, mut pos: usize) -> usize {
        let (title, head_cites) = if env_name.starts_with("proofof") {
            let q = skip_ws_and_comments(self.stream, pos);
            if self.stream.as_bytes().get(q) == Some(&b'{') {
                match read_balanced_group(self.stream, q, b'{', b'}', false) {
                    GroupOutcome::Closed { content, next_pos } => {
                        pos = next_pos;
                        (clean_tex_title(&content), extract_unique_cites(&content))
                    }
                    GroupOutcome::Unclosed { partial, next_pos } => {
                        let loc = self.loc_str(flat_line);
                        self.quality_notes
                            .push(format!("unclosed `{{` heading in \\begin{{{env_name}}} at {loc}"));
                        pos = next_pos;
                        (clean_tex_title(&partial), extract_unique_cites(&partial))
                    }
                }
            } else {
                (None, Vec::new())
            }
        } else {
            let (t, c, next_pos) = self.read_optional_heading(pos, flat_line);
            pos = next_pos;
            (t, c)
        };

        let parent = self.current_parent();
        let prev_provable = self.last_provable_by_parent.get(&parent).copied().flatten();
        let hint = classify_proof_hint(title.as_deref(), prev_provable);
        if let Some(order) =
            self.push_env_node(env_name, SkNodeKind::Proof, title, head_cites, flat_line)
        {
            self.proof_hints.insert(order, hint);
        }
        pos
    }

    pub(super) fn read_optional_heading(
        &mut self,
        pos: usize,
        flat_line: usize,
    ) -> (Option<String>, Vec<String>, usize) {
        let q = skip_ws_and_comments(self.stream, pos);
        if self.stream.as_bytes().get(q) != Some(&b'[') {
            return (None, Vec::new(), pos);
        }
        match read_balanced_group(self.stream, q, b'[', b']', false) {
            GroupOutcome::Closed { content, next_pos } => {
                (clean_tex_title(&content), extract_unique_cites(&content), next_pos)
            }
            GroupOutcome::Unclosed { partial, next_pos } => {
                let loc = self.loc_str(flat_line);
                self.quality_notes.push(format!("unclosed `[` heading at {loc}"));
                (clean_tex_title(&partial), extract_unique_cites(&partial), next_pos)
            }
        }
    }

    pub(super) fn handle_end(&mut self, end_pos: usize, cmd_end: usize) -> usize {
        let p = skip_ws_and_comments(self.stream, cmd_end);
        if self.stream.as_bytes().get(p) != Some(&b'{') {
            return cmd_end;
        }
        let (env_name, next_pos) = match read_balanced_group(self.stream, p, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => (content.trim().to_string(), next_pos),
            GroupOutcome::Unclosed { next_pos, .. } => {
                let loc = self.loc_str(self.flat_line_at(end_pos));
                self.quality_notes.push(format!("unclosed `{{` in \\end at {loc}"));
                return next_pos;
            }
        };
        let flat_line = self.flat_line_at(end_pos);

        if env_name == "document" {
            self.close_all_open(flat_line.saturating_sub(1).max(1));
            self.saw_document_end = true;
            return next_pos;
        }
        if !self.in_body {
            return next_pos;
        }
        let Some(match_idx) = self.env_stack.iter().rposition(|f| f.env_name() == env_name) else {
            let loc = self.loc_str(flat_line);
            self.quality_notes.push(format!("unmatched \\end{{{env_name}}} at {loc}"));
            return next_pos;
        };

        self.close_sections_above_env_depth(match_idx, flat_line);
        while self.env_stack.len() > match_idx + 1 {
            if let Some(unclosed) = self.env_stack.pop() {
                self.close_unclosed_frame(unclosed, flat_line);
            }
        }
        if let Some(frame) = self.env_stack.pop() {
            match frame {
                EnvFrame::Document => self.saw_document_end = true,
                EnvFrame::Node { order, start_flat, max_child_end, .. } => {
                    self.finalize_node_range(order, start_flat, flat_line.max(max_child_end));
                }
                EnvFrame::Math { env_name, multi_row, start_flat, body_start, parent } => {
                    self.emit_math_flush(
                        &env_name,
                        multi_row,
                        start_flat,
                        flat_line,
                        body_start,
                        end_pos,
                        parent,
                    );
                }
                EnvFrame::Wrapper { .. } => {}
            }
        }
        next_pos
    }
    pub(super) fn handle_section(&mut self, cmd: &str, start_pos: usize, cmd_end: usize) -> usize {
        let bytes = self.stream.as_bytes();
        let mut pos = cmd_end;
        let starred = bytes.get(pos) == Some(&b'*');
        if starred {
            pos += 1;
        }
        let flat_line = self.flat_line_at(start_pos);
        pos = skip_ws_and_comments(self.stream, pos);
        if bytes.get(pos) == Some(&b'[') {
            match read_balanced_group(self.stream, pos, b'[', b']', false) {
                GroupOutcome::Closed { next_pos, .. } => {
                    pos = skip_ws_and_comments(self.stream, next_pos)
                }
                GroupOutcome::Unclosed { next_pos, .. } => {
                    let loc = self.loc_str(flat_line);
                    self.quality_notes.push(format!("unclosed `[` in \\{cmd} at {loc}"));
                    pos = skip_ws_and_comments(self.stream, next_pos);
                }
            }
        }
        if bytes.get(pos) != Some(&b'{') {
            return cmd_end;
        }
        let (title, head_cites, next_pos) =
            match read_balanced_group(self.stream, pos, b'{', b'}', false) {
                GroupOutcome::Closed { content, next_pos } => {
                    (clean_tex_title(&content), extract_unique_cites(&content), next_pos)
                }
                GroupOutcome::Unclosed { partial, next_pos } => {
                    let loc = self.loc_str(flat_line);
                    self.quality_notes.push(format!("unclosed `{{` in \\{cmd} at {loc}"));
                    (clean_tex_title(&partial), extract_unique_cites(&partial), next_pos)
                }
            };

        let level = match cmd {
            "part" => 0,
            "chapter" => 1,
            "section" => 2,
            "subsection" => 3,
            _ => 4,
        };
        self.close_non_container_envs(flat_line.saturating_sub(1).max(1));
        self.close_sections_at_level(level, flat_line);

        let parent = self.section_stack.last().map(|s| s.order);
        let env_name = if starred { format!("{cmd}*") } else { cmd.to_string() };
        if let Some(order) = self.push_node(
            SkNodeKind::Section,
            Some(env_name),
            title,
            None,
            parent,
            flat_line,
            flat_line,
            head_cites,
            SkConfidence::TexHeading,
        ) {
            self.section_stack.push(SectionFrame {
                level,
                order,
                start_flat: flat_line,
                max_child_end: flat_line,
                env_depth: self.env_stack.len(),
            });
        }
        next_pos
    }

    pub(super) fn handle_label(&mut self, pos: usize, cmd_end: usize) -> usize {
        let p = skip_ws_and_comments(self.stream, cmd_end);
        if self.stream.as_bytes().get(p) != Some(&b'{') {
            return cmd_end;
        }
        match read_balanced_group(self.stream, p, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => {
                let lbl = content.trim();
                if !lbl.is_empty() {
                    if let Some(owner) = self.current_parent() {
                        if let Some(node) = self.nodes.get_mut(owner) {
                            if node.label.is_none() {
                                node.label = Some(lbl.to_string());
                            }
                        }
                    }
                }
                next_pos
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                let loc = self.loc_str(self.flat_line_at(pos));
                self.quality_notes.push(format!("unclosed `{{` in \\label at {loc}"));
                next_pos
            }
        }
    }

    pub(super) fn handle_caption(&mut self, pos: usize, cmd_end: usize) -> usize {
        let bytes = self.stream.as_bytes();
        let mut p = if bytes.get(cmd_end) == Some(&b'*') { cmd_end + 1 } else { cmd_end };
        p = skip_ws_and_comments(self.stream, p);
        if bytes.get(p) == Some(&b'[') {
            match read_balanced_group(self.stream, p, b'[', b']', false) {
                GroupOutcome::Closed { next_pos, .. } => {
                    p = skip_ws_and_comments(self.stream, next_pos)
                }
                GroupOutcome::Unclosed { next_pos, .. } => {
                    let loc = self.loc_str(self.flat_line_at(pos));
                    self.quality_notes.push(format!("unclosed `[` in \\caption at {loc}"));
                    return next_pos;
                }
            }
        }
        if bytes.get(p) != Some(&b'{') {
            return cmd_end;
        }
        match read_balanced_group(self.stream, p, b'{', b'}', false) {
            GroupOutcome::Closed { content, next_pos } => {
                if let Some(title) = clean_tex_title(&content) {
                    if let Some(owner) = self.current_parent() {
                        if let Some(node) = self.nodes.get_mut(owner) {
                            if matches!(node.kind, SkNodeKind::Figure | SkNodeKind::Table)
                                && node.title.is_none()
                            {
                                node.title = Some(title);
                            }
                        }
                    }
                }
                next_pos
            }
            GroupOutcome::Unclosed { next_pos, .. } => {
                let loc = self.loc_str(self.flat_line_at(pos));
                self.quality_notes.push(format!("unclosed `{{` in \\caption at {loc}"));
                next_pos
            }
        }
    }

    pub(super) fn handle_bib_cmd(&mut self, cmd: &str, pos: usize, cmd_end: usize) -> usize {
        let flat_line = self.flat_line_at(pos);
        let mut next_pos = cmd_end;
        let p = skip_ws_and_comments(self.stream, cmd_end);
        if cmd == "bibliography" && self.stream.as_bytes().get(p) == Some(&b'{') {
            if let GroupOutcome::Closed { next_pos: np, .. } =
                read_balanced_group(self.stream, p, b'{', b'}', false)
            {
                next_pos = np;
            }
        }
        self.close_non_container_envs(flat_line.saturating_sub(1).max(1));
        self.close_sections_at_level(0, flat_line);
        let parent = self.current_parent();
        let _ = self.push_node(
            SkNodeKind::Bibliography,
            Some(cmd.to_string()),
            Some("References".into()),
            None,
            parent,
            flat_line,
            flat_line,
            Vec::new(),
            SkConfidence::TexHeading,
        );
        self.record_child_end(flat_line);
        next_pos
    }

    pub(super) fn handle_display_brackets(&mut self, pos: usize) -> usize {
        let start_flat = self.flat_line_at(pos);
        if let Some(end_pos) = find_closing_display_bracket(self.stream, pos + 2) {
            let end_flat = self.flat_line_at(end_pos);
            let parent = self.current_parent();
            self.emit_math_flush(
                "displaymath",
                false,
                start_flat,
                end_flat,
                pos + 2,
                end_pos,
                parent,
            );
            end_pos + 2
        } else {
            let loc = self.loc_str(start_flat);
            self.quality_notes.push(format!("unclosed `\\[` display math at {loc}"));
            pos + 2
        }
    }

    pub(super) fn handle_display_dollars(&mut self, pos: usize) -> usize {
        let start_flat = self.flat_line_at(pos);
        if let Some(end_pos) = find_closing_double_dollar(self.stream, pos + 2) {
            let end_flat = self.flat_line_at(end_pos);
            let parent = self.current_parent();
            self.emit_math_flush(
                "displaymath",
                false,
                start_flat,
                end_flat,
                pos + 2,
                end_pos,
                parent,
            );
            end_pos + 2
        } else {
            let loc = self.loc_str(start_flat);
            self.quality_notes.push(format!("unclosed `$$` display math at {loc}"));
            pos + 2
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_math_flush(
        &mut self,
        env_name: &str,
        multi_row: bool,
        start_flat: usize,
        end_flat: usize,
        body_start: usize,
        body_end: usize,
        parent: Option<usize>,
    ) {
        let flush = inspect_math_body(self.stream, body_start, body_end, multi_row);
        if flush.unlabelled_count > 0 {
            if let Some(p) = parent {
                if let Some(pnode) = self.nodes.get_mut(p) {
                    pnode.eq_count += flush.unlabelled_count;
                }
            }
        }
        let max_end = end_flat.max(start_flat);
        for eq in flush.labelled {
            let (rf_start, rf_end) = if multi_row {
                let s = self.flat_line_at(eq.raw_slice_start).clamp(start_flat, max_end);
                let e = self
                    .flat_line_at(eq.raw_slice_end.saturating_sub(1).max(eq.raw_slice_start))
                    .clamp(s, max_end);
                (s, e)
            } else {
                (start_flat, max_end)
            };
            if let Some(order) = self.push_node(
                SkNodeKind::Equation,
                Some(env_name.to_string()),
                None,
                Some(eq.label),
                parent,
                rf_start,
                rf_end,
                Vec::new(),
                SkConfidence::TexEnv,
            ) {
                if let Some(node) = self.nodes.get_mut(order) {
                    if multi_row {
                        let slice =
                            self.stream.get(eq.raw_slice_start..eq.raw_slice_end).unwrap_or("");
                        node.fp = compute_body_fp(slice);
                    }
                    if let Some(tag) = eq.printed_tag {
                        node.number = Some(tag);
                        node.number_basis = Some(NumberBasis::Printed);
                    }
                }
                self.record_child_end(rf_end);
            }
        }
    }

    pub(super) fn close_non_container_envs(&mut self, close_flat: usize) {
        while let Some(idx) = self
            .env_stack
            .iter()
            .rposition(|f| matches!(f, EnvFrame::Node { .. } | EnvFrame::Math { .. }))
        {
            while self.env_stack.len() > idx {
                if let Some(frame) = self.env_stack.pop() {
                    self.close_unclosed_frame(frame, close_flat);
                }
            }
        }
    }

    pub(super) fn close_sections_at_level(&mut self, level: u8, next_flat: usize) {
        while let Some(top) = self.section_stack.last() {
            if top.level < level {
                break;
            }
            let sec = self.section_stack.pop().unwrap();
            let close_line = next_flat.saturating_sub(1).max(sec.start_flat);
            while self.env_stack.len() > sec.env_depth {
                if let Some(frame) = self.env_stack.pop() {
                    self.close_unclosed_frame(frame, close_line);
                }
            }
            self.finalize_node_range(sec.order, sec.start_flat, close_line.max(sec.max_child_end));
        }
    }

    pub(super) fn close_sections_above_env_depth(&mut self, env_depth: usize, close_flat: usize) {
        while let Some(top) = self.section_stack.last() {
            if top.env_depth <= env_depth {
                break;
            }
            let sec = self.section_stack.pop().unwrap();
            self.finalize_node_range(
                sec.order,
                sec.start_flat,
                close_flat.max(sec.start_flat).max(sec.max_child_end),
            );
        }
    }

    pub(super) fn close_unclosed_frame(&mut self, frame: EnvFrame, close_flat: usize) {
        match frame {
            EnvFrame::Document => {}
            EnvFrame::Node { env_name, order, start_flat, max_child_end } => {
                let loc = self.loc_str(start_flat);
                self.quality_notes.push(format!("unclosed environment `{env_name}` opened at {loc}"));
                self.finalize_node_range(
                    order,
                    start_flat,
                    close_flat.max(start_flat).max(max_child_end),
                );
            }
            EnvFrame::Math { env_name, multi_row, start_flat, body_start, parent } => {
                let loc = self.loc_str(start_flat);
                self.quality_notes.push(format!("unclosed environment `{env_name}` opened at {loc}"));
                let end_byte = self.line_starts.get(close_flat).copied().unwrap_or(self.stream.len());
                self.emit_math_flush(
                    &env_name,
                    multi_row,
                    start_flat,
                    close_flat.max(start_flat),
                    body_start,
                    end_byte.max(body_start),
                    parent,
                );
            }
            EnvFrame::Wrapper { env_name, start_flat } => {
                let loc = self.loc_str(start_flat);
                self.quality_notes.push(format!("unclosed environment `{env_name}` opened at {loc}"));
            }
        }
    }

    pub(super) fn close_all_open(&mut self, end_flat: usize) {
        while let Some(frame) = self.env_stack.pop() {
            if matches!(frame, EnvFrame::Document) {
                break;
            }
            self.close_sections_above_env_depth(self.env_stack.len(), end_flat);
            self.close_unclosed_frame(frame, end_flat);
        }
        while let Some(sec) = self.section_stack.pop() {
            self.finalize_node_range(
                sec.order,
                sec.start_flat,
                end_flat.max(sec.start_flat).max(sec.max_child_end),
            );
        }
    }

    pub(super) fn finish_all(&mut self) {
        let last_line = self.flattened.lines.len().max(1);
        let had_unclosed_doc = self.env_stack.iter().any(|f| matches!(f, EnvFrame::Document));
        self.close_all_open(last_line);
        if had_unclosed_doc && !self.saw_document_end {
            self.quality_notes
                .push("unclosed `document` environment (missing \\end{document})".to_string());
        }
    }
}
