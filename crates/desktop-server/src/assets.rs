//! Static asset and offline KaTeX vendor bundle serving via `include_bytes!`.

pub struct StaticAsset {
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub const INDEX_HTML: &str = include_str!("../assets/index.html");

pub fn lookup_asset(path: &str) -> Option<StaticAsset> {
    let clean = path.split('?').next().unwrap_or(path);
    match clean {
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
        "/assets/api.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/api.js"),
        }),
        "/assets/math.js" => Some(StaticAsset {
            content_type: "application/javascript; charset=utf-8",
            bytes: include_bytes!("../assets/math.js"),
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
        "/vendor/katex/fonts/KaTeX_AMS-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_AMS-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Math-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Math-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Script-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Script-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size1-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size1-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size2-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size2-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size3-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size3-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size4-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size4-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2"),
        }),
        _ => None,
    }
}
