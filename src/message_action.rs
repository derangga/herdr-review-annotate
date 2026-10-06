//! The `message` action: put the message pane to the right of the agent pane, or focus the one
//! that is open (PLAN.md 13.3 and 13.7).

use std::process::ExitCode;

use crate::env::Env;
use crate::herdr::{notify, run_herdr_output};
use crate::message::{
    MessageAgent, MessageError, find_transcript, parse_agent, pointer_path, read_message,
    read_pointer,
};
use crate::open::{Opened, parse_context, report};

/// The arguments of `herdr plugin pane open` for the message pane.
fn pane_open_args(agent: &MessageAgent) -> Vec<String> {
    let mut args = [
        "plugin",
        "pane",
        "open",
        "--plugin",
        "review",
        "--entrypoint",
        "message",
        "--placement",
        "split",
        "--direction",
        "right",
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(["--target-pane".to_owned(), agent.pane.to_string()]);
    args.extend([
        "--env".to_owned(),
        format!("REVIEW_DELIVER_TO={}", agent.pane),
        "--env".to_owned(),
        format!("REVIEW_DELIVER_TERM={}", agent.terminal),
    ]);
    if !agent.cwd.as_os_str().is_empty() {
        args.extend(["--cwd".to_owned(), agent.cwd.to_string_lossy().into_owned()]);
    }
    args.push("--focus".to_owned());
    args
}

/// Focus the message pane the pointer file names, when it still exists. Any failure means "open one".
fn focus_existing(
    env: &Env,
    agent: &MessageAgent,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> bool {
    let Some(pane) = pointer_path(env, &agent.terminal).and_then(|path| read_pointer(&path)) else {
        return false;
    };
    let pane = pane.to_string();
    herdr(&["pane".to_owned(), "get".to_owned(), pane.clone()]).is_ok()
        && herdr(&[
            "plugin".to_owned(),
            "pane".to_owned(),
            "focus".to_owned(),
            pane,
        ])
        .is_ok()
}

/// Open the message pane beside the focused agent, or focus the one that is open.
///
/// The message is read only to refuse early: the pane reads it again.
pub fn open(
    env: &Env,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<Opened, String> {
    let pane = parse_context(env.get("HERDR_PLUGIN_CONTEXT_JSON"))
        .pane
        .ok_or("Focus the agent's pane first.")?;
    let got = herdr(&["agent".to_owned(), "get".to_owned(), pane]);
    let agent = parse_agent(&got).map_err(|error| error.to_string())?;
    if agent.name != "claude" {
        return Err(MessageError::Unsupported(agent.name).to_string());
    }
    if focus_existing(env, &agent, &mut herdr) {
        return Ok(Opened::Focused);
    }
    find_transcript(&agent, env)
        .and_then(|path| read_message(&path))
        .map_err(|error| error.to_string())?;
    herdr(&pane_open_args(&agent)).map(|_| Opened::Opened)
}

pub fn run(env: &Env) -> ExitCode {
    report(&open(env, run_herdr_output), notify)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
