//! Pure self-contained HTML renderer for Investigate `.rsmap` paper maps (V15).
//!
//! Emits zero `<script>` tags and zero external stylesheets, fonts, or CDNs. Every
//! untrusted string is strictly HTML-escaped. Respects the same copyright (full statements)
//! and privacy (notes) gates as `.rsmap`, applies F6 solid-vs-dashed border cues, labels
//! model-derived entries with `(model)`, and never emits `"verified"` anywhere.

use crate::paper_map::{
    ExportAnchor, ExportClaimBody, ExportHypothesis, ExportNodeReading, ExportSkeletonNode,
    PaperMapExport,
};
use research_tools::map_defs::{AuthorsClaimLabel, ModelEdgeBasis, ReadingRole};
use research_tools::quote_core::clip_quote;
use research_tools::skeleton::SkNodeKind;
use std::collections::BTreeMap;
use std::fmt::Write;

const INLINE_CSS: &str = r#"
:root { color-scheme: light; --bg: #f7f8fa; --card: #ffffff; --ink: #1f2328; --muted: #57606a; --code-border: #1f6feb; --model-border: #9a6700; --line: #d0d7de; }
body { font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; line-height: 1.5; background: var(--bg); color: var(--ink); max-width: 980px; margin: 24px auto; padding: 0 16px; }
header.doc-head, section.panel { background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 16px 20px; margin-bottom: 16px; }
h1, h2, h3 { margin: 0 0 8px 0; }
.meta-row { display: flex; flex-wrap: wrap; gap: 10px; font-size: 13px; color: var(--muted); }
.pill { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 12px; background: #eaeef2; color: var(--ink); }
.pill-code { border: 1px solid var(--code-border); background: #ddf4ff; color: #0550ae; }
.pill-model { border: 1px dashed var(--model-border); background: #fff8c5; color: #7d4e00; }
.cue-code { border-left: 4px solid var(--code-border); padding: 10px 12px; margin: 8px 0; background: #f8fbff; border-radius: 4px; }
.cue-model { border: 1.5px dashed var(--model-border); padding: 10px 12px; margin: 8px 0; background: #fffdf5; border-radius: 4px; }
.quote-box { border-left: 3px solid var(--code-border); margin: 6px 0; padding: 4px 10px; background: #f6f8fa; font-size: 13px; }
.quote-meta { font-size: 12px; color: var(--muted); }
pre.stmt { white-space: pre-wrap; background: #f6f8fa; padding: 10px; border-radius: 4px; border: 1px solid var(--line); overflow-x: auto; font-size: 13px; }
.muted { color: var(--muted); font-size: 13px; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 90%; }
"#;

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn kind_label(kind: SkNodeKind) -> &'static str {
    match kind {
        SkNodeKind::FrontMatter => "Front matter",
        SkNodeKind::Section => "Section",
        SkNodeKind::Definition => "Definition",
        SkNodeKind::Assumption => "Assumption",
        SkNodeKind::Theorem => "Theorem",
        SkNodeKind::Lemma => "Lemma",
        SkNodeKind::Proposition => "Proposition",
        SkNodeKind::Corollary => "Corollary",
        SkNodeKind::Conjecture => "Conjecture",
        SkNodeKind::Claim => "Claim",
        SkNodeKind::Proof => "Proof",
        SkNodeKind::Example => "Example",
        SkNodeKind::Remark => "Remark",
        SkNodeKind::Equation => "Equation",
        SkNodeKind::Figure => "Figure",
        SkNodeKind::Table => "Table",
        SkNodeKind::Algorithm => "Algorithm",
        SkNodeKind::Bibliography => "Bibliography",
    }
}

fn role_label(role: ReadingRole) -> &'static str {
    match role {
        ReadingRole::MainResult => "main result",
        ReadingRole::Supporting => "supporting",
        ReadingRole::Technical => "technical",
        ReadingRole::Background => "background",
        ReadingRole::StandardTool => "standard tool",
    }
}

fn claim_label(label: AuthorsClaimLabel) -> &'static str {
    match label {
        AuthorsClaimLabel::New => "Authors state as new",
        AuthorsClaimLabel::Restated => "Authors note as restated",
        AuthorsClaimLabel::Background => "Authors treat as background",
        AuthorsClaimLabel::NotStated => "Not stated by authors",
    }
}

fn render_anchor_html(out: &mut String, a: &ExportAnchor) {
    let q = clip_quote(a.quote.trim());
    let _ = writeln!(
        out,
        "<div class=\"quote-box\"><div class=\"quote-meta\">matched in <code>{}</code> lines {}-{}</div><blockquote>{}</blockquote></div>",
        esc(&a.path),
        a.start_line,
        a.end_line,
        esc(q),
    );
}

fn render_hypotheses(out: &mut String, hyps: &[ExportHypothesis]) {
    if hyps.is_empty() {
        return;
    }
    out.push_str("<div><strong>Hypotheses (model):</strong><ul>");
    for h in hyps {
        let _ = write!(out, "<li>{}", esc(&h.text));
        render_anchor_html(out, &h.anchor);
        out.push_str("</li>");
    }
    out.push_str("</ul></div>");
}

fn render_claim_block(out: &mut String, claim: &ExportClaimBody, stale: bool) {
    out.push_str("<div class=\"cue-model\">");
    let _ = write!(
        out,
        "<div><span class=\"pill pill-model\">Authors' claim (model)</span> <strong>{}</strong>{}</div>",
        esc(claim_label(claim.label)),
        if stale { " <span class=\"pill\">stale</span>" } else { "" },
    );
    if !claim.attributed_to.is_empty() {
        let _ = writeln!(
            out,
            "<div class=\"muted\">Cited keys: <code>{}</code></div>",
            esc(&claim.attributed_to.join(", "))
        );
    }
    if !claim.cues.is_empty() {
        let _ = writeln!(
            out,
            "<div class=\"muted\">Cues: {}</div>",
            esc(&claim.cues.join("; "))
        );
    }
    if let Some(note) = claim.relation_note.as_deref() {
        let _ = writeln!(out, "<p>{}</p>", esc(note));
    }
    for anchor in &claim.anchors {
        render_anchor_html(out, anchor);
    }
    out.push_str("</div>");
}

fn render_reading_box(out: &mut String, r: &ExportNodeReading) {
    out.push_str("<div class=\"cue-model\">");
    out.push_str("<div><span class=\"pill pill-model\">Reading (model)</span> ");
    if let Some(role) = r.role {
        let _ = write!(out, "<span class=\"pill\">role: {} (model)</span> ", esc(role_label(role)));
    }
    if r.effective_is_main_result {
        out.push_str("<span class=\"pill pill-code\">main result</span> ");
    }
    if r.stale {
        out.push_str("<span class=\"pill\">stale</span> ");
    }
    out.push_str("</div>");
    if let Some(gist) = r.gist.as_deref() {
        let _ = writeln!(out, "<p><strong>Gist (model):</strong> {}</p>", esc(gist));
    }
    if let Some(purpose) = r.purpose.as_deref() {
        let _ = writeln!(out, "<p><strong>Purpose (model):</strong> {}</p>", esc(purpose));
    }
    render_hypotheses(out, &r.hypotheses);
    for a in &r.anchors {
        render_anchor_html(out, a);
    }
    out.push_str("</div>");
}

fn node_heading(n: &ExportSkeletonNode) -> String {
    let mut s = kind_label(n.kind).to_string();
    if let Some(num) = n.number.as_deref().filter(|v| !v.is_empty()) {
        s.push(' ');
        s.push_str(num);
    }
    if let Some(title) = n.title.as_deref().filter(|v| !v.is_empty()) {
        s.push_str(": ");
        s.push_str(title);
    }
    s
}

/// Render a standalone, script-free HTML export of a `.rsmap` document.
pub fn render_paper_map_html(export: &PaperMapExport) -> String {
    let mut out = String::with_capacity(8192);
    let page_title = export.title.as_deref().unwrap_or(&export.paper_key);
    let _ = writeln!(
        out,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Paper Map: {}</title>\n<style>{}</style>\n</head>\n<body>",
        esc(page_title),
        INLINE_CSS
    );

    let licence_text = export.licence.as_deref().unwrap_or("unspecified");
    let stmt_notice = if export.statements_included {
        "Full theorem statements included"
    } else {
        "Full theorem statements omitted (non-open licence or export option off)"
    };
    let notes_notice = if export.notes_included {
        "Private notes included"
    } else {
        "Private notes excluded"
    };

    let _ = writeln!(
        out,
        "<header class=\"doc-head\">\n<h1>{}</h1>\n<div class=\"meta-row\"><span><strong>Paper:</strong> <code>{}</code>{}</span><span><strong>Builder:</strong> <code>{}</code></span><span><strong>Quality:</strong> <code>{:?}</code></span><span><strong>Licence:</strong> <code>{}</code></span></div>",
        esc(page_title),
        esc(&export.paper_key),
        export.version.as_deref().map(|v| format!(" (<code>{}</code>)", esc(v))).unwrap_or_default(),
        esc(export.builder.as_str()),
        export.skeleton.summary.quality,
        esc(licence_text),
    );
    if let Some(q) = export.question.as_deref() {
        let _ = writeln!(out, "<p><strong>Question:</strong> {}</p>", esc(q));
    }
    let _ = writeln!(
        out,
        "<div class=\"meta-row\"><span class=\"pill pill-code\">By code: {} nodes, {} edges</span><span class=\"pill\">{}</span><span class=\"pill\">{}</span></div>\n</header>",
        export.skeleton.summary.node_count,
        export.skeleton.summary.edge_count,
        esc(stmt_notice),
        esc(notes_notice),
    );

    let gists_by_tag: BTreeMap<&str, &ExportNodeReading> =
        export.gists.iter().map(|g| (g.tag.as_str(), g)).collect();
    let claims_by_tag: BTreeMap<&str, &crate::paper_map::ExportAuthorsClaim> =
        export.authors_claims.iter().map(|c| (c.tag.as_str(), c)).collect();

    out.push_str("<section class=\"panel\"><h2>Paper Structure &amp; Reading Map</h2>\n");
    for node in &export.skeleton.nodes {
        let tag = node.tag.as_deref().unwrap_or("(untagged)");
        out.push_str("<article class=\"cue-code\">\n");
        let _ = writeln!(
            out,
            "<div><span class=\"pill pill-code\">By code</span> <strong>{}</strong> <code>{}</code> <span class=\"muted\">({})</span></div>",
            esc(&node_heading(node)),
            esc(tag),
            esc(&node.fp),
        );
        if !node.head_cites.is_empty() {
            let _ = writeln!(
                out,
                "<div class=\"muted\">Heading citations: <code>{}</code></div>",
                esc(&node.head_cites.join(", "))
            );
        }
        if export.statements_included {
            if let Some(stmt) = node.statement.as_deref() {
                let _ = writeln!(out, "<pre class=\"stmt\">{}</pre>", esc(stmt));
            }
        } else if node.kind.is_theorem_like() || node.kind == SkNodeKind::Equation {
            out.push_str("<p class=\"muted\">Full statement omitted (non-open licence or opt-in disabled).</p>");
        }
        if let Some(g) = node.tag.as_deref().and_then(|t| gists_by_tag.get(t).copied()) {
            render_reading_box(&mut out, g);
        }
        if let Some(c) = node.tag.as_deref().and_then(|t| claims_by_tag.get(t).copied()) {
            let body = ExportClaimBody {
                label: c.label,
                relation_note: c.relation_note.clone(),
                attributed_to: c.attributed_to.clone(),
                cues: c.cues.clone(),
                anchors: c.anchors.clone(),
            };
            render_claim_block(&mut out, &body, c.stale);
        }
        out.push_str("</article>\n");
    }
    out.push_str("</section>\n");

    if !export.model_nodes.is_empty() || !export.model_edges.is_empty() {
        out.push_str("<section class=\"panel\"><h2>Model Extensions (model)</h2>\n");
        for m in &export.model_nodes {
            out.push_str("<article class=\"cue-model\">\n");
            let _ = writeln!(
                out,
                "<div><span class=\"pill pill-model\">Node (model): {}</span> <code>{}</code> &rarr; attached to <code>{}</code></div>",
                esc(&m.model_kind),
                esc(&m.tag),
                esc(&m.attach_to),
            );
            if let Some(gist) = m.gist.as_deref() {
                let _ = writeln!(out, "<p><strong>Gist (model):</strong> {}</p>", esc(gist));
            }
            if let Some(purpose) = m.purpose.as_deref() {
                let _ = writeln!(out, "<p><strong>Purpose (model):</strong> {}</p>", esc(purpose));
            }
            render_hypotheses(&mut out, &m.hypotheses);
            for a in &m.anchors {
                render_anchor_html(&mut out, a);
            }
            if let Some(ac) = m.authors_claim.as_ref() {
                render_claim_block(&mut out, ac, m.stale);
            }
            out.push_str("</article>\n");
        }
        for e in &export.model_edges {
            let basis_cls = if e.basis == ModelEdgeBasis::Inferred {
                "cue-model"
            } else {
                "cue-code"
            };
            let _ = writeln!(
                out,
                "<article class=\"{}\"><div><span class=\"pill pill-model\">Edge (model)</span> <code>{}</code> &rarr; <code>{}</code> ({:?} / {:?})</div><p>{}</p>",
                basis_cls,
                esc(&e.from),
                esc(&e.to),
                e.role,
                e.basis,
                esc(&e.reason),
            );
            render_anchor_html(&mut out, &e.anchor);
            out.push_str("</article>\n");
        }
        out.push_str("</section>\n");
    }

    if export.notes_included {
        if let Some(notes) = export.notes.as_ref().filter(|n| !n.is_empty()) {
            out.push_str("<section class=\"panel\"><h2>Reader Notes</h2>\n");
            for note in notes {
                let _ = writeln!(
                    out,
                    "<div class=\"cue-code\"><div><code>{}</code></div><p>{}</p></div>\n",
                    esc(&note.tag),
                    esc(&note.text),
                );
            }
            out.push_str("</section>\n");
        }
    }

    if !export.skeleton.bib.is_empty() {
        out.push_str("<section class=\"panel\"><h2>Bibliography</h2><ul>\n");
        for b in &export.skeleton.bib {
            let _ = writeln!(
                out,
                "<li><code>{}</code>: {}</li>",
                esc(&b.key),
                esc(b.title.as_deref().unwrap_or(&b.raw))
            );
        }
        out.push_str("</ul></section>\n");
    }

    out.push_str("</body>\n</html>\n");
    out
}
