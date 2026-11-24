//! Standalone HTML export with embedded CSS and KaTeX-ready math delimiters (F15).

use crate::latex::{recorded_reviews, SessionExportData};

fn html_escape(s: &str) -> String {
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

pub(crate) fn render_html_document(data: &SessionExportData) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<header class=\"meta\">\n\
         <h1>re:Search Session Report: <code>{}</code></h1>\n\
         <p><strong>Model:</strong> {} &middot; <strong>Policy:</strong> {} &middot; \
         <strong>Chain Head:</strong> <code>{}</code></p>\n\
         </header>\n\
         <section class=\"prompt\"><h2>Problem Statement</h2><p>{}</p></section>\n",
        html_escape(&data.session),
        html_escape(&data.model),
        html_escape(&data.policy_version),
        html_escape(&data.head_hash),
        html_escape(&data.prompt),
    ));

    for (question, answer, unverified) in &data.answers {
        body.push_str("<section><h2>Recorded Answer</h2>");
        if !question.is_empty() && question != &data.prompt {
            body.push_str(&format!("<p><strong>{}</strong></p>", html_escape(question)));
        }
        if *unverified { body.push_str("<p>The verifier reported unresolved issues.</p>"); }
        body.push_str(&format!("<p style=\"white-space:pre-wrap\">{}</p></section>\n", html_escape(answer)));
    }
    for (heading, lines) in recorded_reviews(&data.events) {
        body.push_str(&format!("<section><h2>{}</h2>", html_escape(&heading)));
        for line in lines { body.push_str(&format!("<p>{}</p>", html_escape(&line))); }
        body.push_str("</section>\n");
    }
    body.push_str("<section class=\"steps\"><h2>Proof Graph Steps</h2>\n");
    for e in &data.entries {
        let deps = if e.depends_on.is_empty() {
            "none".to_string()
        } else {
            html_escape(&e.depends_on.join(", "))
        };
        let latex_block = e
            .latex
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|l| format!("<div class=\"math-block\">\\[{}\\]</div>", html_escape(l)))
            .unwrap_or_default();
        body.push_str(&format!(
            "<article class=\"step status-{}\">\n\
             <div class=\"step-hdr\"><span class=\"step-id\">{}</span> \
             <span class=\"badge\">{}</span> <span class=\"status\">{}</span> \
             <span class=\"deps\">deps: {}</span></div>\n\
             <h3>{}</h3>\n<p>{}</p>\n{}\n</article>\n",
            html_escape(&e.status),
            html_escape(&e.entry_id),
            html_escape(&e.entry_type),
            html_escape(&e.status),
            deps,
            html_escape(&e.summary),
            html_escape(&e.details),
            latex_block,
        ));
    }
    body.push_str("</section>\n");

    if !data.sympy_calls.is_empty() {
        body.push_str("<section class=\"code\"><h2>Symbolic Computations</h2>\n");
        for c in &data.sympy_calls {
            body.push_str(&format!(
                "<h3>Step {} (<code>{}</code>)</h3><pre><code>{}</code></pre>\n",
                c.seq,
                html_escape(&c.call_id),
                html_escape(&c.code)
            ));
        }
        body.push_str("</section>\n");
    }

    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>re:Search Report - {}</title>\n\
         <style>\n\
         body{{font-family:system-ui,-apple-system,sans-serif;max-width:920px;margin:2rem auto;padding:0 1.25rem;line-height:1.6;color:#1f2328;background:#fff;}}\n\
         code,pre{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;background:#f6f8fa;padding:.15em .35em;border-radius:4px;}}\n\
         pre{{padding:.85rem;overflow-x:auto;border:1px solid #d0d7de;}}\n\
         .meta{{border-bottom:2px solid #d0d7de;padding-bottom:.75rem;margin-bottom:1.5rem;}}\n\
         .step{{border:1px solid #d0d7de;border-left:4px solid #0969da;border-radius:6px;padding:.9rem 1rem;margin:.85rem 0;}}\n\
         .status-accepted{{border-left-color:#1a7f37;}}.status-rejected{{border-left-color:#cf222e;}}.status-superseded{{opacity:.7;}}\n\
         .step-hdr{{font-size:.85rem;color:#57606a;display:flex;gap:.5rem;flex-wrap:wrap;}}\n\
         .badge,.status{{background:#eaeef2;padding:0 .45rem;border-radius:999px;font-weight:600;}}\n\
         .math-block{{margin:.6rem 0;padding:.5rem .75rem;background:#f8fafc;border-radius:4px;font-family:serif;}}\n\
         </style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        html_escape(&data.session)
    )
}
