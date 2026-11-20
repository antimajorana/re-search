//! `re-search`: the supervisor for re:Search.
//!
//!   re-search run --prompt "..." [--route cli|api] [--provider anthropic|openai|ollama|gemini]
//!                 [--model MODEL] [--python EXE] [--claude EXE] [--core-exe EXE]
//!                 [--data-dir DIR] [--worker-script PY] [--api-base URL]
//!                 [--max-turns N] [--max-tokens N]
//!   re-search resume <session> --prompt "..." [--provider anthropic|openai|ollama|gemini]
//!                    [--model MODEL] [--python EXE] [--data-dir DIR]
//!                    [--worker-script PY] [--api-base URL]
//!                    [--max-turns N] [--max-tokens N]
//!   re-search show <session> [--data-dir DIR]
//!   re-search sessions [--data-dir DIR]
//!   re-search verify [SESSION] [--data-dir DIR]
//!
//! `run` and `resume` print the session as versioned NDJSON on stdout and a
//! readable trace on stderr. Approval prompts appear on stderr and are
//! answered on the terminal.

mod api_session;
mod approval;
mod cmd;
mod events;
mod runtime;
mod session;
mod session_lock;

#[cfg(test)]
mod main_tests;

fn exit_code_for_args<I>(args: I) -> i32
where
    I: Iterator<Item = String>,
{
    match cmd::run_args(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("re-search: {e}");
            2
        }
    }
}

fn main() {
    std::process::exit(exit_code_for_args(std::env::args().skip(1)));
}
