//! `herdr-review`: argument parsing and dispatch.

use std::process::ExitCode;

use herdr_review::{open, spike, tui};

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("open") => open::run(),
        Some("tui") => tui::run(),
        Some("spike-send") => spike::send(),
        _ => {
            #[allow(clippy::print_stderr, reason = "the command boundary reports failures")]
            {
                eprintln!("usage: herdr-review <open|tui>");
            }
            ExitCode::from(2)
        }
    }
}
