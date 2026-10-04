//! `herdr-review`: review a diff in a Herdr pane and send the comments to an agent.

mod open;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("open") => open::run(),
        Some("tui") => tui::run(),
        _ => {
            #[allow(clippy::print_stderr, reason = "the command boundary reports failures")]
            {
                eprintln!("usage: herdr-review <open|tui>");
            }
            ExitCode::from(2)
        }
    }
}
