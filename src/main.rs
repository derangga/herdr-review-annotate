//! `herdr-review`: reads the process once, then dispatches. The only file that touches
//! `std::env` besides `herdr.rs`.

use std::process::ExitCode;

use herdr_review::cli::{Command, Failure, Output, parse, run_comment};
use herdr_review::diff::run_git;
use herdr_review::env::Env;
use herdr_review::herdr::run_herdr_output;
use herdr_review::{open, send, spike, tui};

#[allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the command boundary prints"
)]
fn finish(output: &Output) -> ExitCode {
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    ExitCode::from(output.code)
}

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let env = Env::new(
        std::env::vars(),
        std::env::current_dir().unwrap_or_default(),
    );
    match parse(&args) {
        Err(usage) => finish(&Failure::Usage(usage).output()),
        Ok(Command::Open { repo, base }) => open::run(&env, repo.as_deref(), base.as_deref()),
        Ok(Command::Tui { repo }) => tui::run(&env, repo.as_deref()),
        Ok(Command::Send { repo, all_open }) => send::run(&env, repo.as_deref(), all_open),
        Ok(Command::SpikeSend) => spike::send(&env),
        Ok(Command::Comment { repo, action }) => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let output = run_comment(
                repo.as_deref(),
                &action,
                &env,
                &now,
                run_git,
                run_herdr_output,
                std::io::stdin(),
            );
            finish(&output)
        }
    }
}
