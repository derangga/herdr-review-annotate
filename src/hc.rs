//! The hc backend of `send` (design/send.md, backends): `hc get`, `hc ls` and `hc send` in place of
//! `herdr agent get`, `herdr agent list` and `herdr agent prompt`.
//!
//! Every function takes `hc`, a closure that runs one `hc` command and returns its stdout, or the
//! parsed failure. Nothing here reads raw JSON outside this file.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::agent_delivery::without_escapes;
use crate::meta::Meta;
use crate::send::{Candidate, Refusal, SendError, TargetError, belongs};
use crate::store::TerminalAddress;

/// `hc` failed: its exit code and the `{"error":{"kind","message"}}` it prints on stderr.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HcError {
    /// `None` when `hc` could not be run, or was killed by a signal.
    pub exit: Option<i32>,
    pub kind: Option<String>,
    pub message: String,
}

/// The failure `hc` reported, from its exit code and stderr. Stderr that is not the JSON envelope
/// is kept as the message.
pub fn parse_error(exit: Option<i32>, stderr: &str) -> HcError {
    let trimmed = stderr.trim();
    let envelope = serde_json::from_str::<Value>(trimmed).ok();
    let field = |pointer: &str| {
        envelope
            .as_ref()
            .and_then(|value| value.pointer(pointer))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    HcError {
        exit,
        kind: field("/error/kind"),
        message: field("/error/message").unwrap_or_else(|| trimmed.to_owned()),
    }
}

/// Run `hc` synchronously. A failure that leaves stderr empty names only the command, never the
/// remaining arguments, because `send` passes the prompt as an argument.
pub fn run_hc(args: &[String]) -> Result<String, HcError> {
    let command = args.first().map_or("", String::as_str);
    let output = Command::new("hc")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| HcError {
            exit: None,
            kind: None,
            message: format!("cannot run hc {command}: {error}"),
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let mut error = parse_error(
        output.status.code(),
        &String::from_utf8_lossy(&output.stderr),
    );
    if error.message.is_empty() {
        error.message = format!("hc {command} failed");
    }
    Err(error)
}

/// One terminal as `hc get --json` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub address: TerminalAddress,
    /// `status.app`, such as `claude-code`. `None` before the agent reports.
    pub agent: Option<String>,
    /// The launch folder. `hc ls` has no other.
    pub cwd: PathBuf,
    pub finished: bool,
    /// `status.state`: `idle`, `working`, `blocked`, `done` or another word. `None` before the agent
    /// reports.
    pub status: Option<String>,
}

fn parse_record(text: &str) -> Option<Record> {
    let value = serde_json::from_str::<Value>(text).ok()?;
    let status = value.get("status").filter(|status| status.is_object());
    let reported = value.get("reported").and_then(Value::as_bool) != Some(false);
    let word = |value: Option<&Value>, name| {
        Some(value?.get(name)?.as_str()?.to_owned()).filter(|word: &String| !word.is_empty())
    };
    Some(Record {
        address: TerminalAddress::parse(value.get("address")?.as_str()?)?,
        agent: word(status, "app"),
        cwd: value.get("cwd")?.as_str()?.into(),
        finished: value.get("state").and_then(Value::as_str) == Some("finished"),
        status: word(status, "state").filter(|_| reported),
    })
}

/// `hc get <t> --json`, parsed.
pub fn get(
    address: &TerminalAddress,
    hc: &mut impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Record, HcError> {
    let args = ["get", address.as_str(), "--json"].map(str::to_owned);
    let text = hc(&args)?;
    parse_record(&text).ok_or_else(|| HcError {
        exit: None,
        kind: None,
        message: format!("hc get {address} did not describe a terminal"),
    })
}

/// The agents `hc ls --json` lists. A terminal with no `status` is a plain program.
fn listed(text: &str) -> Vec<Candidate> {
    let value = serde_json::from_str::<Value>(text).ok();
    let terminals = value
        .as_ref()
        .and_then(|value| value.get("terminals")?.as_array());
    let candidate = |terminal: &Value| {
        let text = |name| terminal.get(name)?.as_str().filter(|text| !text.is_empty());
        Some(Candidate {
            address: TerminalAddress::parse(text("address")?)?,
            agent: text("agent")?.to_owned(),
            cwd: text("cwd")?.into(),
            status: text("status")?.to_owned(),
        })
    };
    terminals
        .into_iter()
        .flatten()
        .filter_map(candidate)
        .collect()
}

/// The terminal a send goes to (design/send.md, target resolution under hc): the address the caller
/// chose, then the one saved in `meta`, then a search by folder.
///
/// The chosen address is the only candidate when there is one, whatever its folder or state, because
/// the caller picked it: a finished or missing terminal comes back finished, so `ready` refuses it as
/// gone and nothing goes to an agent the caller did not pick. The saved address must also be an agent
/// that has reported, and work in this review's tree.
pub fn resolve(
    explicit: Option<&TerminalAddress>,
    meta: &Meta,
    root: &Path,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Record, TargetError> {
    if let Some(address) = explicit {
        return match get(address, &mut hc) {
            Ok(record) => Ok(record),
            Err(error) if error.exit == Some(3) || error.kind.as_deref() == Some("not_found") => {
                Ok(Record {
                    address: address.clone(),
                    agent: None,
                    cwd: root.to_path_buf(),
                    finished: true,
                    status: None,
                })
            }
            Err(error) => Err(TargetError::Backend(error.message)),
        };
    }
    let stored = meta.hc_target.as_ref().map(|target| &target.address);
    let saved = stored.and_then(|address| get(address, &mut hc).ok());
    if let Some(record) = saved
        .filter(|record| !record.finished && record.status.is_some() && belongs(&record.cwd, root))
    {
        return Ok(record);
    }
    let ls = ["ls", "--json"].map(str::to_owned);
    let agents = listed(&hc(&ls).map_err(|error| TargetError::Backend(error.message))?);
    let inside = agents.iter().filter(|agent| agent.cwd.starts_with(root));
    let above = agents.iter().filter(|agent| root.starts_with(&agent.cwd));
    for found in [inside.collect::<Vec<_>>(), above.collect()] {
        match found.as_slice() {
            [] => {}
            [one] => {
                return get(&one.address, &mut hc)
                    .map_err(|error| TargetError::Backend(error.message));
            }
            _ => {
                return Err(TargetError::AmbiguousHc(
                    found.into_iter().cloned().collect(),
                ));
            }
        }
    }
    Err(TargetError::NoAgent)
}

/// Whether the terminal can take a prompt now, from its `hc get` record.
pub fn ready(record: &Record) -> Result<(), Refusal> {
    match record.status.as_deref() {
        _ if record.finished => Err(Refusal::AgentGone),
        None => Err(Refusal::NotReady),
        Some("blocked") => Err(Refusal::Blocked),
        Some(_) => Ok(()),
    }
}

/// Send `text` to the terminal. Returns whether the agent was working, in which case it takes the text
/// as a message in its current turn.
///
/// `hc send` runs without `--if`, so a working agent is a success. A refusal after the `get` means the
/// terminal changed in between. Nothing is typed in either case.
pub fn deliver(
    record: &Record,
    text: &str,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<bool, SendError> {
    ready(record).map_err(SendError::refused)?;
    let args = [
        "send",
        "--json",
        record.address.as_str(),
        &without_escapes(text),
    ]
    .map(str::to_owned);
    match hc(&args) {
        Ok(reply) => Ok(serde_json::from_str::<Value>(&reply)
            .is_ok_and(|reply| reply.get("status").and_then(Value::as_str) == Some("working"))),
        Err(error) => Err(match error.exit {
            Some(9) => SendError::refused(Refusal::Blocked),
            Some(3 | 11) => SendError::refused(Refusal::AgentGone),
            _ => SendError::Failed(format!(
                "hc send failed: {} Nothing was sent; your comments are still unsent.",
                error.message.trim_end_matches('.').to_owned() + "."
            )),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn hc_errors_parse_to_exit_kind_and_message_and_other_stderr_is_the_message() {
        let envelope = r#"{"error":{"hint":"nothing was typed","kind":"blocked","message":"t7 is blocked (permission)"}}"#;
        assert_eq!(
            parse_error(Some(9), &format!("{envelope}\n")),
            HcError {
                exit: Some(9),
                kind: Some("blocked".into()),
                message: "t7 is blocked (permission)".into(),
            }
        );
        assert_eq!(
            parse_error(None, " boom \n"),
            HcError {
                exit: None,
                kind: None,
                message: "boom".into(),
            }
        );
    }

    #[test]
    fn a_record_has_the_agent_the_state_and_the_status_as_an_object() {
        let working = r#"{"address":"t1d","cwd":"/r","state":"live","reported":true,
            "status":{"app":"claude-code","state":"working"}}"#;
        let record = parse_record(working).unwrap();
        assert_eq!(record.agent.as_deref(), Some("claude-code"));
        assert_eq!(record.status.as_deref(), Some("working"));
        assert!(!record.finished && ready(&record).is_ok());
        let done = working.replace("working", "done");
        assert!(ready(&parse_record(&done).unwrap()).is_ok());
        assert!(parse_record(r#"{"cwd":"/r"}"#).is_none());
    }

    #[test]
    fn only_a_reported_live_status_other_than_blocked_is_ready() {
        let record = |state: &str, status: &str, reported: bool| {
            parse_record(&format!(
                r#"{{"address":"t1","cwd":"/r","state":"{state}","reported":{reported},"status":{status}}}"#
            ))
            .unwrap()
        };
        let idle = r#"{"app":"x","state":"idle"}"#;
        assert_eq!(ready(&record("live", idle, true)), Ok(()));
        assert_eq!(
            ready(&record("live", "null", false)),
            Err(Refusal::NotReady)
        );
        assert_eq!(ready(&record("live", idle, false)), Err(Refusal::NotReady));
        assert_eq!(
            ready(&record("live", r#"{"app":"x","state":"blocked"}"#, true)),
            Err(Refusal::Blocked)
        );
        assert_eq!(
            ready(&record("finished", "null", true)),
            Err(Refusal::AgentGone)
        );
    }
}
