//! `message --json` and `message send`: the agent-message review the herdr-app panel runs
//! (design/message-review.md, the panel's commands).
//!
//! The panel holds no message logic. These two commands find the newest message with the code the
//! pane uses, build the prompt with `format_prompt` and deliver it with the same delivery as `send`.
//! The agent is named by a Herdr pane (`--pane`) or by an hc terminal (`--target`), which is what the
//! panel knows.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent_delivery::{Delivery, HerdrError, deliver_to_agent};
use crate::cli::{Failure, Output};
use crate::comment::{CommandError, MAX_BATCH_FILE, check_body, read_user_file};
use crate::env::Env;
use crate::hc::{self, HcError, Record};
use crate::json::{MessageJson, MessageSentJson, SCHEMA};
use crate::message::{
    AgentMessage, MessageComment, MessageError, find_transcript_in_folder, format_prompt, load,
    read_message,
};
use crate::store::TerminalAddress;

/// Where the agent is: a Herdr pane id (`w1:p2`) or an hc address (`t7`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Pane(String),
    Terminal(TerminalAddress),
}

/// What `message send --file` holds. `id` is the message the lines were counted in, which the panel
/// sends so that a newer message refuses the send and does not take the comments for its own lines.
#[derive(Debug, Deserialize)]
struct SendFile {
    id: Option<String>,
    comments: Vec<FileComment>,
}

#[derive(Debug, Deserialize)]
struct FileComment {
    start: u32,
    end: u32,
    body: String,
}

/// The agent behind a target and its newest message. `record` is the hc terminal, kept for the
/// delivery.
struct Found {
    agent: String,
    status: String,
    message: AgentMessage,
    record: Option<Record>,
}

/// An hc terminal's `status.app` for Claude Code.
const HC_CLAUDE: &str = "claude-code";

fn find(
    target: &Target,
    env: &Env,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
    hc: &mut impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Found, MessageError> {
    match target {
        Target::Pane(pane) => {
            let got = herdr(&["agent".to_owned(), "get".to_owned(), pane.clone()]);
            let (agent, message) = load(&got, env)?;
            Ok(Found {
                agent: agent.name,
                status: agent.status.as_str().to_owned(),
                message,
                record: None,
            })
        }
        Target::Terminal(address) => {
            let record = hc::get(address, hc).map_err(|error| {
                if error.exit == Some(3) || error.kind.as_deref() == Some("not_found") {
                    MessageError::NotAnAgent
                } else {
                    MessageError::Herdr(HerdrError {
                        code: None,
                        message: error.message,
                    })
                }
            })?;
            let agent = match record.agent.clone() {
                Some(name) if !record.finished => name,
                _ => return Err(MessageError::NotAnAgent),
            };
            if agent != HC_CLAUDE {
                return Err(MessageError::Unsupported(agent));
            }
            let path = find_transcript_in_folder(&record.cwd, env)?;
            Ok(Found {
                agent,
                status: record
                    .status
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                message: read_message(&path)?,
                record: Some(record),
            })
        }
    }
}

fn print(value: &impl Serialize) -> Output {
    Output {
        stdout: format!("{}\n", serde_json::to_string(value).unwrap_or_default()),
        ..Output::default()
    }
}

/// `message --json`: the agent's newest message. Nothing is written.
pub fn read(
    target: &Target,
    env: &Env,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Output, Failure> {
    let found =
        find(target, env, &mut herdr, &mut hc).map_err(|e| Failure::Message(e.to_string()))?;
    Ok(print(&MessageJson {
        schema: SCHEMA,
        agent: found.agent,
        status: found.status,
        id: found.message.id,
        lines: found.message.lines,
    }))
}

/// The comments of a send file, checked against the message's lines.
fn comments_of(
    file: SendFile,
    message: &AgentMessage,
) -> Result<Vec<MessageComment>, CommandError> {
    let bad = |index: Option<usize>, why: &str| CommandError::InvalidBatch {
        index,
        why: why.to_owned(),
    };
    if file.comments.is_empty() {
        return Err(bad(None, "there are no comments"));
    }
    if file.id.is_some_and(|id| id != message.id) {
        return Err(bad(
            None,
            "the agent's message changed since these lines were read, refresh and comment again",
        ));
    }
    let count = message.lines.len();
    file.comments
        .into_iter()
        .enumerate()
        .map(|(index, comment)| {
            if comment.start == 0 || comment.end < comment.start || comment.end as usize > count {
                return Err(bad(
                    Some(index),
                    &format!(
                        "lines {}-{} are not in the message",
                        comment.start, comment.end
                    ),
                ));
            }
            check_body(&comment.body).map_err(|error| bad(Some(index), &error.to_string()))?;
            Ok(MessageComment {
                start: comment.start,
                end: comment.end,
                chars: None,
                body: comment.body,
            })
        })
        .collect()
}

/// `message send --file`: read the comments, build the prompt and deliver it. The file is read once
/// and deleted, as a panel body file is. A refused delivery is a failure and nothing was typed.
pub fn send(
    target: &Target,
    file: &Path,
    env: &Env,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Output, Failure> {
    let text = read_user_file(file, MAX_BATCH_FILE).map_err(Failure::Command)?;
    let parsed = serde_json::from_str::<SendFile>(&text).map_err(|error| {
        Failure::Command(CommandError::InvalidBatch {
            index: None,
            why: error.to_string(),
        })
    })?;
    let found =
        find(target, env, &mut herdr, &mut hc).map_err(|e| Failure::Message(e.to_string()))?;
    let comments = comments_of(parsed, &found.message).map_err(Failure::Command)?;
    let prompt = format_prompt(&found.message.lines, &comments);
    let delivered = match (target, &found.record) {
        (Target::Pane(pane), _) => deliver_to_agent(Delivery::Send, Some(pane), &prompt, herdr),
        (Target::Terminal(_), Some(record)) => hc::deliver(record, &prompt, hc)
            .map(drop)
            .map_err(|error| error.to_string()),
        (Target::Terminal(_), None) => Err("The terminal was not read.".to_owned()),
    };
    delivered.map_err(Failure::Message)?;
    Ok(print(&MessageSentJson {
        schema: SCHEMA,
        sent: comments.len(),
    }))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unnecessary_wraps
)]
mod tests;
