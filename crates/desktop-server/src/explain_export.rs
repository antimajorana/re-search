//! Markdown and standalone HTML export for Explain (`sidetalk`) Q&A conversations.

use crate::routes_session::{bad, internal};
use crate::state::DesktopState;
use agent::SideTalkTurn;
use std::fmt::Write as _;

pub fn export_explain_conversation(
    state: &DesktopState,
    session_id: &str,
    format: &str,
) -> Result<(String, &'static str, Vec<u8>), (u16, String)> {
    let ledger = state.open_ledger().map_err(internal)?;
    let turns = agent::read_sidetalk_thread(&ledger, session_id).map_err(internal)?;
    let title = state
        .load_meta(session_id)
        .map(|m| m.title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| session_id.to_string());

    match format {
        "explain_md" => {
            let md = render_explain_markdown(&title, session_id, &turns);
            Ok((
                format!("explain_{session_id}.md"),
                "text/markdown; charset=utf-8",
                md.into_bytes(),
            ))
        }
        "explain_html" => {
            let html = render_explain_html(&title, session_id, &turns);
            Ok((
                format!("explain_{session_id}.html"),
                "text/html; charset=utf-8",
                html.into_bytes(),
            ))
        }
        other => Err(bad(format!("unknown explain export format `{other}`"))),
    }
}

fn render_explain_markdown(title: &str, session_id: &str, turns: &[SideTalkTurn]) -> String {
    let mut out = format!(
        "# Explain Conversation: {title}\n\nSession: `{session_id}` · Turns: {}\n\n",
        turns.len()
    );
    if turns.is_empty() {
        out.push_str("_No Explain turns recorded yet._\n");
        return out;
    }
    for (idx, turn) in turns.iter().enumerate() {
        let n = idx + 1;
        let _ = writeln!(
            out,
            "## Question {n}\n\n{}\n\n### Answer\n\n{}\n",
            turn.question.trim(),
            turn.answer.trim(),
        );
        if !turn.references.is_empty() {
            let refs = turn
                .references
                .iter()
                .map(|r| format!("`{}`", r.replace('`', "'")))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "Cited steps & sources: {refs}\n");
        }
    }
    out
}

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

fn render_explain_html(title: &str, session_id: &str, turns: &[SideTalkTurn]) -> String {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "<header class=\"meta\">\n\
         <h1>Explain Conversation: {}</h1>\n\
         <p class=\"sub\"><strong>Session:</strong> <code>{}</code> &middot; <strong>Turns:</strong> {}</p>\n\
         </header>",
        html_escape(title),
        html_escape(session_id),
        turns.len()
    );

    if turns.is_empty() {
        body.push_str("<p class=\"empty\"><em>No Explain turns recorded yet.</em></p>\n");
    } else {
        for (idx, turn) in turns.iter().enumerate() {
            let n = idx + 1;
            let refs_html = if turn.references.is_empty() {
                String::new()
            } else {
                let chips: String = turn
                    .references
                    .iter()
                    .map(|r| format!("<span class=\"ref-chip\">{}</span>", html_escape(r)))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("<div class=\"refs\"><strong>Cited:</strong> {chips}</div>\n")
            };
            let _ = writeln!(
                body,
                "<article class=\"turn\">\n\
                 <h2>Question {n}</h2>\n\
                 <div class=\"question prose\">{}</div>\n\
                 <h3>Answer</h3>\n\
                 <div class=\"answer prose\">{}</div>\n\
                 {refs_html}</article>",
                html_escape(turn.question.trim()),
                html_escape(turn.answer.trim()),
            );
        }
    }

    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Explain - {}</title>\n\
         <style>\n\
         body{{font-family:system-ui,-apple-system,Segoe UI,sans-serif;max-width:860px;margin:2rem auto;padding:0 1.25rem;line-height:1.65;color:#1f2328;background:#ffffff;}}\n\
         code{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;background:#f6f8fa;padding:.15em .35em;border-radius:4px;font-size:.9em;}}\n\
         .meta{{border-bottom:2px solid #d0d7de;padding-bottom:.85rem;margin-bottom:1.5rem;}}\n\
         .meta h1{{margin:0 0 .35rem 0;font-size:1.5rem;}}\n\
         .sub{{margin:0;color:#57606a;font-size:.92rem;}}\n\
         .turn{{border:1px solid #d0d7de;border-left:4px solid #0969da;border-radius:8px;padding:1rem 1.2rem;margin:1rem 0;background:#fbfcfd;}}\n\
         .turn h2{{margin:0 0 .45rem 0;font-size:1.08rem;color:#0969da;}}\n\
         .turn h3{{margin:.9rem 0 .4rem 0;font-size:.96rem;color:#334155;}}\n\
         .prose{{white-space:pre-wrap;overflow-wrap:anywhere;}}\n\
         .question{{background:#f1f5f9;padding:.7rem .9rem;border-radius:6px;}}\n\
         .answer{{padding:.2rem 0;}}\n\
         .refs{{margin-top:.75rem;padding-top:.6rem;border-top:1px solid #e2e8f0;font-size:.86rem;color:#475569;display:flex;gap:.4rem;flex-wrap:wrap;align-items:center;}}\n\
         .ref-chip{{background:#e2e8f0;color:#1e293b;padding:.1rem .5rem;border-radius:999px;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.82rem;}}\n\
         .empty{{color:#64748b;}}\n\
         </style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        html_escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_and_html_escape_and_preserve_math() {
        let empty_md = render_explain_markdown("Demo", "s-1", &[]);
        assert!(empty_md.contains("# Explain Conversation: Demo"));
        assert!(empty_md.contains("Turns: 0"));
        assert!(empty_md.contains("_No Explain turns recorded yet._"));

        let turns = vec![SideTalkTurn {
            seq: 1,
            ts_ms: 100,
            question: "What is $\\int x^2 dx$ & <script>alert('x')</script>?".into(),
            answer: "It is $$\\frac{x^3}{3} + C$$ in \"step\" e-1.".into(),
            references: vec!["e-1".into()],
            raw_blob: None,
        }];

        let md = render_explain_markdown("Calc <Title>", "s-1", &turns);
        assert!(md.contains("## Question 1"));
        assert!(md.contains("### Answer"));
        assert!(md.contains("`e-1`"));

        let html = render_explain_html("Calc <Title>", "s-1", &turns);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("cdn"));
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"));
        assert!(html.contains("&quot;step&quot;"));
        assert!(html.contains("$\\int x^2 dx$"));
        assert!(html.contains("$$\\frac{x^3}{3} + C$$"));
        assert!(html.contains("<span class=\"ref-chip\">e-1</span>"));
    }
}
