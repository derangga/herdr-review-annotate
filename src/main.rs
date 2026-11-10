//! `herdr-review`: reads the process once, then dispatches. The only file that touches
//! `std::env` besides `herdr.rs`.

use std::process::ExitCode;

use herdr_review::cli::{Command, Failure, Output, parse, run_comment};
use herdr_review::diff::{run_git, run_git_bytes};
use herdr_review::env::Env;
use herdr_review::hc::run_hc;
use herdr_review::herdr::run_herdr_output;
use herdr_review::message_json;
use herdr_review::{message_action, message_tui, open, send, state, theme_json, tui, user};

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
    )
    .with_exe(std::env::current_exe().unwrap_or_default());
    match parse(&args) {
        Err(usage) => finish(&Failure::Usage(usage).output()),
        Ok(Command::Open { repo, base }) => open::run(&env, repo.as_deref(), base.as_deref()),
        Ok(Command::Tui { repo }) => tui::run(&env, repo.as_deref()),
        Ok(Command::Send(args)) => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            finish(&send::command(
                &args,
                &env,
                &now,
                run_git,
                run_herdr_output,
                run_hc,
            ))
        }
        Ok(Command::State { query }) => {
            finish(&state::state(&query, &env, run_git_bytes).unwrap_or_else(|f| f.output()))
        }
        Ok(Command::File { query, path }) => {
            finish(&state::file(&query, &path, &env, run_git_bytes).unwrap_or_else(|f| f.output()))
        }
        Ok(Command::Files { query, paths }) => finish(
            &state::files(&query, &paths, &env, run_git_bytes).unwrap_or_else(|f| f.output()),
        ),
        Ok(Command::Theme) => finish(&theme_json::command(&env, run_herdr_output)),
        Ok(Command::Archive { repo, json }) => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            finish(
                &user::archive(repo.as_deref(), json, &env, &now, run_git)
                    .unwrap_or_else(|f| f.output()),
            )
        }
        Ok(Command::View { query, path, set }) => finish(
            &user::view(&query, &path, set, &env, run_git_bytes).unwrap_or_else(|f| f.output()),
        ),
        Ok(Command::Message) => message_action::run(&env),
        Ok(Command::MessageTui) => message_tui::run(&env),
        Ok(Command::MessageRead { target }) => finish(
            &message_json::read(&target, &env, run_herdr_output, run_hc)
                .unwrap_or_else(|f| f.output()),
        ),
        Ok(Command::MessageSend { target, file }) => finish(
            &message_json::send(&target, &file, &env, run_herdr_output, run_hc)
                .unwrap_or_else(|f| f.output()),
        ),
        Ok(Command::Comment { repo, action }) => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let output = run_comment(
                repo.as_deref(),
                &action,
                &env,
                &now,
                run_git,
                run_herdr_output,
                run_hc,
                std::io::stdin(),
            );
            finish(&output)
        }
    }
}
