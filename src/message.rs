//! Find the agent's newest message in its Claude Code transcript.
//!
//! `herdr agent get` names the agent and its session. The session id is the transcript's file name,
//! and the transcript's last assistant message is what the message pane shows.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::agent_delivery::{HerdrError, parse_herdr_error};
use crate::env::Env;
use crate::store::{PaneId, TerminalId};

/// What Herdr reports about an agent's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    fn parse(text: Option<&str>) -> Self {
        match text {
            Some("idle") => Self::Idle,
            Some("working") => Self::Working,
            Some("blocked") => Self::Blocked,
            Some("done") => Self::Done,
            _ => Self::Unknown,
        }
    }
}

/// How `agent_session` names the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    Id(String),
    Path(PathBuf),
}

/// The agent in one pane, from one `herdr agent get` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageAgent {
    pub pane: PaneId,
    pub terminal: TerminalId,
    pub cwd: PathBuf,
    pub status: AgentStatus,
    pub session: Option<Session>,
    pub name: String,
}

/// A user comment on a line or a range of lines of an agent message. It lives in the message pane
/// until it is sent or the pane closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageComment {
    /// The first and the last line it covers, from 1, both included.
    pub start: u32,
    pub end: u32,
    pub body: String,
}

/// The text of one assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    pub id: String,
    pub lines: Vec<String>,
    pub transcript: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    NotAnAgent,
    Unsupported(String),
    NoSession,
    NoTranscript { id: String },
    NoMessage { path: PathBuf },
    Io { path: PathBuf, kind: ErrorKind },
    Herdr(HerdrError),
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnAgent => f.write_str("No agent is running in that pane."),
            Self::Unsupported(name) => {
                write!(f, "Message review reads Claude Code only, not {name}.")
            }
            Self::NoSession => f.write_str("Herdr has no session for this agent yet."),
            Self::NoTranscript { id } => write!(f, "Cannot find the transcript of session {id}."),
            Self::NoMessage { .. } => f.write_str("The session has no message yet."),
            Self::Io { path, kind } => write!(f, "Cannot read {}: {kind}.", path.display()),
            Self::Herdr(error) => write!(f, "Herdr failed: {}", error.message),
        }
    }
}

/// The agent in the pane one `herdr agent get` result is about.
pub fn parse_agent(agent_get: &Result<String, String>) -> Result<MessageAgent, MessageError> {
    let text = match agent_get {
        Ok(text) => text,
        Err(stderr) => {
            let error = parse_herdr_error(stderr);
            return Err(match error.code.as_deref() {
                Some("agent_not_found" | "pane_not_found") => MessageError::NotAnAgent,
                _ => MessageError::Herdr(error),
            });
        }
    };
    let value = serde_json::from_str::<Value>(text).ok();
    let agent = value
        .as_ref()
        .and_then(|value| value.pointer("/result/agent"));
    let field = |name: &str| {
        agent
            .and_then(|agent| agent.get(name))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    };
    let (Some(name), Some(pane), Some(terminal)) = (
        field("agent"),
        field("pane_id").and_then(PaneId::parse),
        field("terminal_id").and_then(TerminalId::parse),
    ) else {
        return Err(MessageError::NotAnAgent);
    };
    let session = agent
        .and_then(|agent| agent.pointer("/agent_session"))
        .and_then(|session| {
            let value = session.get("value")?.as_str().filter(|v| !v.is_empty())?;
            match session.get("kind")?.as_str()? {
                "id" => Some(Session::Id(value.to_owned())),
                "path" => Some(Session::Path(PathBuf::from(value))),
                _ => None,
            }
        });
    Ok(MessageAgent {
        pane,
        terminal,
        cwd: field("cwd").map(PathBuf::from).unwrap_or_default(),
        status: AgentStatus::parse(field("agent_status")),
        session,
        name: name.to_owned(),
    })
}

/// Where Claude Code may keep sessions: `$CLAUDE_CONFIG_DIR`, `~/.claude`, then every other
/// `~/.claude*` directory in name order.
fn config_roots(env: &Env) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut add = |path: PathBuf| {
        if !roots.contains(&path) {
            roots.push(path);
        }
    };
    if let Some(dir) = env.get("CLAUDE_CONFIG_DIR") {
        add(PathBuf::from(dir));
    }
    if let Some(home) = env.get("HOME").map(PathBuf::from) {
        add(home.join(".claude"));
        let mut others = std::fs::read_dir(&home)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".claude"))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        others.sort();
        others.into_iter().for_each(add);
    }
    roots
}

/// A session id is a uuid, and it becomes part of a path, so it holds nothing else.
fn is_session_id(id: &str) -> bool {
    id.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// The transcript file of `agent`'s session (PLAN.md 13.2).
pub fn find_transcript(agent: &MessageAgent, env: &Env) -> Result<PathBuf, MessageError> {
    if agent.name != "claude" {
        return Err(MessageError::Unsupported(agent.name.clone()));
    }
    let id = match agent.session.as_ref().ok_or(MessageError::NoSession)? {
        Session::Path(path) if path.is_file() => return Ok(path.clone()),
        Session::Path(path) => path.to_string_lossy().into_owned(),
        Session::Id(id) => id.clone(),
    };
    let file = format!("{id}.jsonl");
    let found = is_session_id(&id).then(|| {
        config_roots(env).into_iter().find_map(|root| {
            std::fs::read_dir(root.join("projects"))
                .ok()?
                .flatten()
                .map(|project| project.path().join(&file))
                .find(|path| path.is_file())
        })
    });
    found.flatten().ok_or(MessageError::NoTranscript { id })
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "isSidechain", default)]
    sidechain: bool,
    message: Option<ApiMessage>,
}

#[derive(Deserialize)]
struct ApiMessage {
    id: Option<String>,
    model: Option<String>,
    content: Option<Content>,
}

/// An assistant message holds blocks. A user message may hold plain text instead.
#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Blocks(Vec<Block>),
    Text(#[allow(dead_code, reason = "only assistant entries are read")] String),
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

/// `(message id, text)` of one transcript line, when it is an assistant text block.
fn text_blocks(line: &str) -> Vec<(String, String)> {
    let Ok(entry) = serde_json::from_str::<Entry>(line) else {
        return Vec::new();
    };
    let Some(message) = entry.message.filter(|message| {
        entry.kind.as_deref() == Some("assistant")
            && !entry.sidechain
            && message.model.as_deref() != Some("<synthetic>")
    }) else {
        return Vec::new();
    };
    let Some(Content::Blocks(blocks)) = message.content else {
        return Vec::new();
    };
    let id = message.id.unwrap_or_default();
    blocks
        .into_iter()
        .filter(|block| block.kind.as_deref() == Some("text"))
        .filter_map(|block| block.text)
        .filter(|text| !text.trim().is_empty())
        .map(|text| (id.clone(), text))
        .collect()
}

/// The newest message of the transcript at `path`: every text block of the last message id that
/// has text, in file order, joined by a blank line (PLAN.md 13.2).
pub fn read_message(path: &Path) -> Result<AgentMessage, MessageError> {
    let bytes = std::fs::read(path).map_err(|error| MessageError::Io {
        path: path.to_owned(),
        kind: error.kind(),
    })?;
    let blocks = String::from_utf8_lossy(&bytes)
        .lines()
        .flat_map(text_blocks)
        .collect::<Vec<_>>();
    let Some((id, _)) = blocks.last().cloned() else {
        return Err(MessageError::NoMessage {
            path: path.to_owned(),
        });
    };
    let text = blocks
        .iter()
        .filter(|(block_id, _)| *block_id == id)
        .map(|(_, text)| text.trim_end().trim_start_matches(['\n', '\r']))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(AgentMessage {
        id,
        lines: text.lines().map(str::to_owned).collect(),
        transcript: path.to_owned(),
    })
}

/// The file that names the message pane open beside the agent in `terminal`.
///
/// `herdr pane list` does not say which plugin entrypoint a pane runs, so the pane writes its own
/// id here and the action reads it to focus the pane on a second key press.
pub fn pointer_path(env: &Env, terminal: &TerminalId) -> Option<PathBuf> {
    let base = env.state_base()?;
    Some(base.join("message").join(terminal.as_str()))
}

pub fn write_pointer(path: &Path, pane: &PaneId) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, pane.as_str())
}

pub fn read_pointer(path: &Path) -> Option<PaneId> {
    PaneId::parse(std::fs::read_to_string(path).ok()?.trim())
}

/// Longest quote of a range in a prompt. A longer one shows its first lines and a count.
const QUOTE_LINES: usize = 6;

/// The prompt that sends `comments` on the message `lines` back to the agent (PLAN.md 13.5).
///
/// Comments go in line order, and in the order they were written when they start on the same
/// line. The quote only has to locate the comment, since the agent has the whole message in its
/// context.
pub fn format_prompt(lines: &[String], comments: &[MessageComment]) -> String {
    let mut order = comments.iter().collect::<Vec<_>>();
    order.sort_by_key(|comment| comment.start);
    let mut out = vec![
        "Feedback on your last message. Address each point.".to_owned(),
        String::new(),
    ];
    for comment in order {
        let (start, end) = (comment.start as usize, comment.end as usize);
        out.push(if start == end {
            format!("- line {start}:")
        } else {
            format!("- lines {start}-{end}:")
        });
        let quoted = lines
            .get(start.saturating_sub(1)..end.min(lines.len()))
            .unwrap_or_default();
        let shown = if quoted.len() > QUOTE_LINES {
            QUOTE_LINES - 1
        } else {
            quoted.len()
        };
        for line in quoted.iter().take(shown) {
            out.push(format!("  > {line}").trim_end().to_owned());
        }
        if quoted.len() > shown {
            out.push(format!("  > … ({} more lines)", quoted.len() - shown));
        }
        out.extend(
            comment
                .body
                .lines()
                .map(|line| format!("  {line}").trim_end().to_owned()),
        );
    }
    out.join("\n")
}

/// The agent behind one `herdr agent get` result and its newest message.
pub fn load(
    agent_get: &Result<String, String>,
    env: &Env,
) -> Result<(MessageAgent, AgentMessage), MessageError> {
    let agent = parse_agent(agent_get)?;
    let path = find_transcript(&agent, env)?;
    let message = read_message(&path)?;
    Ok((agent, message))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
