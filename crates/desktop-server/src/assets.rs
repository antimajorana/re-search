//! Static asset and offline KaTeX vendor bundle serving via `include_bytes!`.

pub struct StaticAsset {
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub const INDEX_HTML: &str = include_str!("../assets/index.html");

const INDEX_PARTIALS: &[(&str, &str)] = &[
    (
        "      <!-- INCLUDE:index_investigation.html -->\n",
        include_str!("../assets/index_investigation.html"),
    ),
    (
        "          <!-- INCLUDE:index_latex_body.html -->\n",
        include_str!("../assets/index_latex_body.html"),
    ),
    (
        "          <!-- INCLUDE:index_dag_top.html -->\n",
        include_str!("../assets/index_dag_top.html"),
    ),
    (
        "        <!-- INCLUDE:index_sources.html -->\n",
        include_str!("../assets/index_sources.html"),
    ),
    (
        "    <!-- INCLUDE:index_source_reader.html -->\n",
        include_str!("../assets/index_source_reader.html"),
    ),
    (
        "  <!-- INCLUDE:index_dialogs.html -->\n",
        include_str!("../assets/index_dialogs.html"),
    ),
];

/// Returns the complete desktop UI document by splicing the extracted HTML partials
/// into `assets/index.html`.
pub fn render_index_html() -> String {
    let mut html = INDEX_HTML.to_string();
    for (marker, partial) in INDEX_PARTIALS {
        // Normalize CRLF on Windows checkouts if needed while preserving LF checkouts byte-for-byte.
        let crlf_marker = marker.replace('\n', "\r\n");
        if html.contains(marker) {
            html = html.replacen(marker, partial, 1);
        } else if html.contains(&crlf_marker) {
            html = html.replacen(&crlf_marker, partial, 1);
        }
    }
    html
}

fn pdf_renderer_asset(name: &str) -> Option<StaticAsset> {
    const BUNDLE: &[u8] = include_bytes!("../vendor/pdfjs/renderer.zip");
    let mut assets = zip::ZipArchive::new(std::io::Cursor::new(BUNDLE)).ok()?;
    let file = assets.by_name(name).ok()?;
    let start = usize::try_from(file.data_start()?).ok()?;
    let size = usize::try_from(file.size()).ok()?;
    let content_type = match name.rsplit('.').next() {
        Some("mjs" | "js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    };
    Some(StaticAsset { content_type, bytes: BUNDLE.get(start..start.checked_add(size)?)? })
}

pub fn lookup_asset(path: &str) -> Option<StaticAsset> {
    let clean = path.split('?').next().unwrap_or(path);
    if let Some(name) = clean.strip_prefix("/vendor/pdfjs/") {
        return pdf_renderer_asset(name);
    }
    match clean {
        "/assets/pdf_preview.js" => Some(StaticAsset {
            content_type: "text/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/pdf_preview.js"),
        }),
        "/assets/pdf_preview.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/pdf_preview.css"),
        }),
        "/assets/re-search-icon.png" => Some(StaticAsset {
            content_type: "image/png",
            bytes: include_bytes!("../assets/re-search-icon.png"),
        }),
        "/assets/tokens.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/tokens.css"),
        }),
        "/assets/layout.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/layout.css"),
        }),
        "/assets/components.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/components.css"),
        }),
        "/assets/workbench.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/workbench.css"),
        }),
        "/assets/modals.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/modals.css"),
        }),
        "/assets/dag.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/dag.css"),
        }),
        "/assets/projects.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/projects.css"),
        }),
        "/assets/project_library.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/project_library.css"),
        }),
        "/assets/project_validation.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/project_validation.css"),
        }),
        "/assets/experiments.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/experiments.css"),
        }),
        "/assets/investigation.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/investigation.css"),
        }),
        "/assets/investigation.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation.js"),
        }),
        "/assets/investigation_map.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_map.js"),
        }),
        "/assets/investigation_graph.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_graph.js"),
        }),
        "/assets/graph_layout.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/graph_layout.js"),
        }),
        "/assets/graph_view.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/graph_view.js"),
        }),
        "/assets/ui_badge.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/ui_badge.js"),
        }),
        "/assets/run_bar.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/run_bar.js"),
        }),
        "/assets/quote_chip.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/quote_chip.js"),
        }),
        "/assets/approval_card.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/approval_card.js"),
        }),
        "/assets/solve_state.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_state.js"),
        }),
        "/assets/solve_setup.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_setup.js"),
        }),
        "/assets/solve_badges.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_badges.js"),
        }),
        "/assets/solve_steps.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_steps.js"),
        }),
        "/assets/solve_why.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_why.js"),
        }),
        "/assets/solve_exprs.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_exprs.js"),
        }),
        "/assets/solve_transcribe.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_transcribe.js"),
        }),
        "/assets/solve_artifacts.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/solve_artifacts.js"),
        }),
        "/assets/writeup_btn.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/writeup_btn.js"),
        }),
        "/assets/solve.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/solve.css"),
        }),
        "/assets/api_idea.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/api_idea.js"),
        }),
        "/assets/idea_state.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/idea_state.js"),
        }),
        "/assets/idea_setup.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/idea_setup.js"),
        }),
        "/assets/idea_search.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/idea_search.js"),
        }),
        "/assets/idea.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/idea.js"),
        }),
        "/assets/idea.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/idea.css"),
        }),
        "/assets/investigation_conventions.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_conventions.js"),
        }),
        "/assets/investigation_state.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_state.js"),
        }),
        "/assets/investigation_answer.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_answer.js"),
        }),
        "/assets/paper_cites.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/paper_cites.js"),
        }),
        "/assets/investigation_scope.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_scope.js"),
        }),
        "/assets/investigation_outline.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_outline.js"),
        }),
        "/assets/investigation_results.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_results.js"),
        }),
        "/assets/investigation_node.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_node.js"),
        }),
        "/assets/investigation_v3_shell.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_v3_shell.js"),
        }),
        "/assets/mode_stages.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/mode_stages.js"),
        }),
        "/assets/investigation_shell.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_shell.css"),
        }),
        "/assets/investigation_entry.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_entry.js"),
        }),
        "/assets/investigation_skip.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_skip.js"),
        }),
        "/assets/investigation_wholemap_model.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_wholemap_model.js"),
        }),
        "/assets/investigation_wholemap.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_wholemap.js"),
        }),
        "/assets/investigation_wholemap.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_wholemap.css"),
        }),
        "/assets/investigation_badges.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_badges.js"),
        }),
        "/assets/investigation_node_model.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_node_model.js"),
        }),
        "/assets/investigation_notes_ui.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_notes_ui.js"),
        }),
        "/assets/investigation_context.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_context.js"),
        }),
        "/assets/investigation_context_cards.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_context_cards.js"),
        }),
        "/assets/investigation_orient_ui.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_orient_ui.js"),
        }),
        "/assets/investigation_share_ui.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_share_ui.js"),
        }),
        "/assets/investigation_handoff_ui.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_handoff_ui.js"),
        }),
        "/assets/investigation_legacy.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/investigation_legacy.js"),
        }),
        "/assets/api.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/api.js"),
        }),
        "/assets/math.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/math.js"),
        }),
        "/assets/code_highlight.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/code_highlight.js"),
        }),
        "/assets/math_scan.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/math_scan.js"),
        }),
        "/assets/math_katex.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/math_katex.js"),
        }),
        "/assets/math_equations.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/math_equations.js"),
        }),
        "/assets/markdown.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/markdown.js"),
        }),
        "/assets/tex_preview.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/tex_preview.js"),
        }),
        "/assets/trace_cards.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/trace_cards.js"),
        }),
        "/assets/workbench.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/workbench.js"),
        }),
        "/assets/session_rail.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/session_rail.js"),
        }),
        "/assets/new_menu.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/new_menu.js"),
        }),
        "/assets/session_folders.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/session_folders.js"),
        }),
        "/assets/claim_trace.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/claim_trace.js"),
        }),
        "/assets/stream.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/stream.js"),
        }),
        "/assets/composer.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/composer.js"),
        }),
        "/assets/model_picker.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/model_picker.js"),
        }),
        "/assets/effort_picker.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/effort_picker.js"),
        }),
        "/assets/subagents.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/subagents.js"),
        }),
        "/assets/guide.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/guide.js"),
        }),
        "/assets/source_links.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/source_links.js"),
        }),
        "/assets/connect.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/connect.js"),
        }),
        "/assets/canvas_latex.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/canvas_latex.js"),
        }),
        "/assets/canvas_dag.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/canvas_dag.js"),
        }),
        "/assets/canvas_dag_detail.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/canvas_dag_detail.js"),
        }),
        "/assets/canvas_sources.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/canvas_sources.js"),
        }),
        "/assets/library_send_back.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/library_send_back.js"),
        }),
        "/assets/lessons.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/lessons.js"),
        }),
        "/assets/lesson_remember.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/lesson_remember.js"),
        }),
        "/assets/projects.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/projects.js"),
        }),
        "/assets/project_library.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/project_library.js"),
        }),
        "/assets/project_edits.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/project_edits.js"),
        }),
        "/assets/project_validation.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/project_validation.js"),
        }),
        "/assets/experiments.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/experiments.js"),
        }),
        "/assets/formal_library.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/formal_library.js"),
        }),
        "/assets/hitl.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/hitl.js"),
        }),
        "/assets/worker_activity.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/worker_activity.js"),
        }),
        "/assets/ocr_settings.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/ocr_settings.js"),
        }),
        "/assets/page_image_viewer.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/page_image_viewer.js"),
        }),
        "/assets/app.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/app.js"),
        }),
        "/vendor/katex/katex.min.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../vendor/katex/katex.min.js"),
        }),
        "/vendor/katex/auto-render.min.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../vendor/katex/auto-render.min.js"),
        }),
        "/vendor/katex/katex.min.css" => Some(StaticAsset {
            content_type: "text/css; charset=utf-8",
            bytes: include_bytes!("../vendor/katex/katex.min.css"),
        }),
        _ => crate::assets_fonts::lookup_katex_font(clean),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_index_html_expands_all_partials_cleanly() {
        let html = render_index_html();
        assert!(!html.contains("<!-- INCLUDE:"), "unexpanded partial marker in rendered index.html");
        for id in [
            "investigation-panel",
            "latex-editor",
            "dag-fork-bar",
            "panel-sources",
            "source-reader-body",
            "workspace-dialog",
            "session-folder-dialog",
            "worked-example",
        ] {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "missing #{id} in rendered index.html"
            );
        }
    }
}
