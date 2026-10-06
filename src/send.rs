//! Send: the prompt the agent receives, and which agent receives it.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

use crate::agent_delivery::{Delivery, agent_ready, deliver_to_agent};
use crate::diff::{RepoRoot, run_git};
use crate::env::Env;
use crate::herdr::{notify, run_herdr_output};
use crate::meta::{self, Meta, Target};
use crate::open::parse_context;
use crate::store::{
    Anchor, AnchorTarget, Author, Comment, CommentId, Event, Kind, PaneId, Side, StoreError,
    TerminalId, Thread, Warning, read, write,
};

/// `text` as one POSIX shell word. A single quote ends the quoting, is escaped, and starts it again.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The cited path and line of a thread. A line on the old side counts in the original file, so it
/// cites `old_path` when the file was renamed.
fn location(anchor: &Anchor) -> String {
    let cited = |side: Side| match (side, &anchor.old_path) {
        (Side::Old, Some(old)) => old,
        _ => &anchor.path,
    };
    let letter = |side| if side == Side::Old { 'L' } else { 'R' };
    match &anchor.target {
        AnchorTarget::File => format!("{} (file)", anchor.path),
        AnchorTarget::Line { side, line, .. } => {
            format!("{}:{line} ({})", cited(*side), letter(*side))
        }
        AnchorTarget::Range {
            side, start, end, ..
        } => format!("{}:{start}-{end} ({})", cited(*side), letter(*side)),
    }
}

/// `text` with `prefix` in front of every line after the first.
fn continued(text: &str, prefix: &str) -> String {
    text.trim().replace('\n', &format!("\n{prefix}"))
}

fn reply_line(reply: &Comment) -> String {
    let who = if reply.author.is_user() {
        "user"
    } else {
        "agent"
    };
    format!("  > {who}: {}", continued(&reply.body, "  > "))
}

fn entry(thread: &Thread) -> String {
    let mut line = format!("- [{}] {}", thread.root.id, location(&thread.anchor));
    if thread.reopened {
        line.push_str(", reopened");
    }
    let body = if thread.root.author.is_user() {
        continued(&thread.root.body, "  ")
    } else {
        line.push_str(", your comment");
        thread
            .root
            .body
            .trim()
            .lines()
            .next()
            .unwrap_or("")
            .to_owned()
    };
    let _ = write!(line, ": {body}");
    for reply in &thread.replies {
        line.push('\n');
        line.push_str(&reply_line(reply));
    }
    line
}

/// The prompt for `threads`: the instructions with the commands to answer, then one entry per
/// thread (design/send.md, prompt). `bin` and `root` are quoted for a POSIX shell.
pub fn format(threads: &[&Thread], bin: &Path, root: &Path) -> String {
    let bin = quote(&bin.to_string_lossy());
    let root = quote(&root.to_string_lossy());
    let mut text = format!(
        "Address the review comments below. For each one: make the change, then resolve it with a one-line\n\
         reply. If you disagree or are unsure, reply and leave it open.\n\
         \n\
         Resolve:\n\
         {bin} comment resolve --repo {root} <id> --reply - <<'EOF'\n\
         <one line>\n\
         EOF\n\
         Reply without resolving:\n\
         {bin} comment reply --repo {root} <id> - <<'EOF'\n\
         <text>\n\
         EOF\n\
         \n\
         Comments on the diff (L = line in the original file, R = in the changed file):\n"
    );
    for thread in threads {
        text.push_str(&entry(thread));
        text.push('\n');
    }
    text
}

/// Resolution found no agent, or could not choose between several.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    NoAgent,
    Ambiguous(Vec<Target>),
    /// `herdr agent list` failed, so the search could not run.
    Herdr(String),
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAgent => f.write_str("No agent found for this review."),
            Self::Ambiguous(found) => write!(
                f,
                "{} agents match, pick one in the review pane.",
                found.len()
            ),
            Self::Herdr(message) => write!(f, "Cannot list agents: {message}"),
        }
    }
}

/// An agent as `herdr agent get` and `herdr agent list` describe it.
struct Seen {
    target: Target,
    cwd: PathBuf,
    workspace: Option<String>,
}

fn seen(agent: &Value) -> Option<Seen> {
    let text = |name| agent.get(name)?.as_str().filter(|text| !text.is_empty());
    Some(Seen {
        target: Target {
            pane: PaneId::parse(text("pane_id")?)?,
            terminal: TerminalId::parse(text("terminal_id")?)?,
            agent: text("agent")?.to_owned(),
        },
        cwd: text("cwd")?.into(),
        workspace: text("workspace_id").map(str::to_owned),
    })
}

/// The root, inside it, or above it: the places an agent for this review may be working.
fn belongs(cwd: &Path, root: &Path) -> bool {
    cwd.starts_with(root) || root.starts_with(cwd)
}

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// A stored pane is valid when it is ready, still holds the stored terminal and works in this
/// review's tree (ADR 0006).
fn valid(
    pane: &PaneId,
    terminal: &TerminalId,
    root: &Path,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> Option<Target> {
    let got = herdr(&arguments(&["agent", "get", pane.as_str()]));
    agent_ready(&got).ok()?;
    let value = serde_json::from_str::<Value>(&got.ok()?).ok()?;
    let found = seen(value.pointer("/result/agent")?)?;
    (found.target.terminal == *terminal && belongs(&found.cwd, root)).then_some(Target {
        pane: pane.clone(),
        ..found.target
    })
}

/// The agents `herdr agent list` reports.
fn listed(list: &str) -> Vec<Seen> {
    let value = serde_json::from_str::<Value>(list).ok();
    let agents = value
        .as_ref()
        .and_then(|value| value.pointer("/result/agents")?.as_array());
    agents.into_iter().flatten().filter_map(seen).collect()
}

/// One match is the answer, none falls to the next rule, several are a choice for the user.
fn one_of(found: &[&Seen]) -> Option<Result<Target, TargetError>> {
    match found {
        [] => None,
        [one] => Some(Ok(one.target.clone())),
        _ => Some(Err(TargetError::Ambiguous(
            found.iter().map(|seen| seen.target.clone()).collect(),
        ))),
    }
}

/// The agent a send goes to (design/send.md, target resolution, ADR 0006): the pane the review was opened beside,
/// the one saved in `meta`, then a search by terminal id and by working directory.
pub fn resolve_target(
    env: &Env,
    meta: &Meta,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
    root: &Path,
) -> Result<Target, TargetError> {
    let from_env = env
        .get("REVIEW_DELIVER_TO")
        .and_then(PaneId::parse)
        .zip(env.get("REVIEW_DELIVER_TERM").and_then(TerminalId::parse));
    let stored = meta
        .target
        .as_ref()
        .map(|target| (target.pane.clone(), target.terminal.clone()));
    let mut tried = Vec::new();
    for (pane, terminal) in from_env.iter().chain(&stored) {
        if tried.contains(&(pane, terminal)) {
            continue;
        }
        tried.push((pane, terminal));
        if let Some(target) = valid(pane, terminal, root, &mut herdr) {
            return Ok(target);
        }
    }
    let agents = listed(&herdr(&arguments(&["agent", "list"])).map_err(TargetError::Herdr)?);
    let terminals = tried
        .iter()
        .map(|(_, terminal)| *terminal)
        .collect::<Vec<_>>();
    let by_terminal = agents
        .iter()
        .filter(|seen| terminals.contains(&&seen.target.terminal))
        .collect::<Vec<_>>();
    let inside = agents
        .iter()
        .filter(|seen| seen.cwd.starts_with(root))
        .collect::<Vec<_>>();
    let workspace = env
        .get("HERDR_PANE_ID")
        .and_then(|pane| pane.split_once(':'))
        .map(|(workspace, _)| workspace);
    let above = agents
        .iter()
        .filter(|seen| root.starts_with(&seen.cwd))
        .filter(|seen| workspace.is_none_or(|ws| seen.workspace.as_deref() == Some(ws)))
        .collect::<Vec<_>>();
    [by_terminal, inside, above]
        .into_iter()
        .find_map(|found| one_of(&found))
        .unwrap_or(Err(TargetError::NoAgent))
}

/// Remember the target for the next send, which may come from the `send` action that never has
/// the pane's environment. Writes nothing when `meta.json` already holds it.
pub fn save_target(
    dir: &Path,
    root: &Path,
    meta: &Meta,
    target: &Target,
) -> Result<(), StoreError> {
    if meta.target.as_ref() == Some(target) {
        return Ok(());
    }
    meta::save(dir, root, |meta| meta.target = Some(target.clone()))
}

/// What a send did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Nothing,
    Sent {
        n: usize,
        agent: String,
    },
    /// The agent was working, so Claude Code holds the prompt until its turn ends.
    Queued {
        n: usize,
        agent: String,
    },
}

impl fmt::Display for SendOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nothing => f.write_str("nothing to send"),
            Self::Sent { n, agent } => write!(f, "sent {n} to {agent}"),
            Self::Queued { n, .. } => write!(f, "agent is working, {n} comments queued"),
        }
    }
}

/// A send that delivered, or had nothing to deliver, and what went wrong on the side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub outcome: SendOutcome,
    /// The agent that got the prompt. `None` when there was nothing to send.
    pub target: Option<Target>,
    pub warnings: Vec<Warning>,
}

/// Why nothing was delivered. No comment is marked sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    Store(StoreError),
    Target(TargetError),
    /// The agent cannot take a prompt now, or Herdr failed. The text says nothing was sent.
    Refused(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Target(error) => error.fmt(f),
            Self::Refused(message) => f.write_str(message),
        }
    }
}

/// `$HERDR_PLUGIN_ROOT/bin/herdr-review`, which Herdr sets for every plugin command and pane.
fn plugin_bin(env: &Env) -> PathBuf {
    env.get("HERDR_PLUGIN_ROOT").map_or_else(
        || "herdr-review".into(),
        |root| Path::new(root).join("bin/herdr-review"),
    )
}

/// The comments a thread puts in a `sent` event: its user comments that no send carried, or all of
/// them for a resend, and the root when the user reopened it, since that is what clears the reopen.
fn carried(thread: &Thread, resend: bool) -> Vec<CommentId> {
    let mut ids = thread
        .comments()
        .filter(|c| c.author.is_user() && (resend || c.sent_batch.is_none()))
        .map(|c| c.id.clone())
        .collect::<Vec<_>>();
    if thread.reopened && !ids.contains(&thread.root.id) {
        ids.push(thread.root.id.clone());
    }
    ids
}

fn is_working(agent_get: &Result<String, String>) -> bool {
    let value = agent_get
        .as_ref()
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    value
        .and_then(|value| {
            value
                .pointer("/result/agent/agent_status")?
                .as_str()
                .map(|s| s == "working")
        })
        .unwrap_or(false)
}

/// Which threads a send carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The unsent ones.
    Unsent,
    /// The unsent ones and every open thread, sent before or not.
    AllOpen,
    /// This one thread again, whatever it was sent before.
    Thread(CommentId),
}

/// Deliver the threads `scope` names to the target agent as one prompt (design/send.md, steps), then
/// record the send.
///
/// Nothing is retried. A second `herdr agent prompt` after an unclear failure could deliver the
/// batch twice, so the user presses the key again. Once Herdr accepts the prompt the send is a
/// success, and a failed write of the `sent` event is only a warning.
pub fn send(
    dir: &Path,
    root: &Path,
    env: &Env,
    now: &str,
    scope: &Scope,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<Sent, SendError> {
    let review = read(dir).map_err(SendError::Store)?;
    let threads = review
        .threads
        .iter()
        .filter(|thread| match scope {
            Scope::Unsent => thread.unsent,
            Scope::AllOpen => thread.unsent || thread.is_open(),
            Scope::Thread(id) => thread.root.id == *id,
        })
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if threads.is_empty() {
        return Ok(Sent {
            outcome: SendOutcome::Nothing,
            target: None,
            warnings,
        });
    }
    let (meta, warning) = meta::load(dir);
    warnings.extend(warning);
    let text = format(&threads, &plugin_bin(env), root);
    let target = resolve_target(env, &meta, &mut herdr, root).map_err(SendError::Target)?;
    let mut working = false;
    deliver_to_agent(Delivery::Send, Some(target.pane.as_str()), &text, |args| {
        let got = herdr(args);
        if args.starts_with(&arguments(&["agent", "get"])) {
            working = is_working(&got);
        }
        got
    })
    .map_err(SendError::Refused)?;
    let resend = *scope != Scope::Unsent;
    let ids = threads
        .iter()
        .flat_map(|thread| carried(thread, resend))
        .collect::<Vec<_>>();
    let recorded = write(dir, now, |review, now| {
        let batch = review.ids.clone().batch();
        let kind = Kind::Sent { ids, batch };
        let event = Event {
            at: now.to_owned(),
            by: Author::User,
            kind,
        };
        Ok::<_, ()>((vec![event], ()))
    });
    if recorded.is_err() {
        warnings.push(Warning::SentNotRecorded);
    }
    if save_target(dir, root, &meta, &target).is_err() {
        warnings.push(Warning::TargetNotSaved);
    }
    let (n, agent) = (threads.len(), target.agent.clone());
    let outcome = if working {
        SendOutcome::Queued { n, agent }
    } else {
        SendOutcome::Sent { n, agent }
    };
    Ok(Sent {
        outcome,
        target: Some(target),
        warnings,
    })
}

/// The `send` action's edge: every outcome and warning becomes a notification, since an action has
/// no terminal. A refusal and a missing target exit 1.
fn report(
    result: &Result<Sent, SendError>,
    mut notify: impl FnMut(&str, Option<&str>),
) -> ExitCode {
    match result {
        Ok(sent) => {
            let warnings = sent
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let body = (!warnings.is_empty()).then(|| warnings.join("; "));
            notify(&format!("review: {}", sent.outcome), body.as_deref());
            ExitCode::SUCCESS
        }
        Err(error) => {
            notify("review: not sent", Some(&error.to_string()));
            ExitCode::FAILURE
        }
    }
}

/// The `send` action: the same send as the TUI's key, reported through notifications.
pub fn run(env: &Env, repo: Option<&Path>, all_open: bool) -> ExitCode {
    let context = parse_context(env.get("HERDR_PLUGIN_CONTEXT_JSON"));
    let cwd = context.cwd.unwrap_or_else(|| env.cwd.clone());
    let located = RepoRoot::resolve(repo, &cwd, run_git)
        .map_err(|error| error.to_string())
        .and_then(|root| {
            let base = env.state_base().ok_or("set HOME or XDG_STATE_HOME")?;
            Ok((meta::locate(&base, root.path()), root))
        });
    let result = match located {
        Ok((dir, root)) => {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let scope = if all_open {
                Scope::AllOpen
            } else {
                Scope::Unsent
            };
            send(&dir, root.path(), env, &now, &scope, run_herdr_output)
        }
        Err(message) => Err(SendError::Refused(message)),
    };
    report(&result, notify)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::store::{Add, Author, CommentId, Event, Kind, RelPath, Spec, fold};

    fn event(by: &Author, kind: Kind) -> Event {
        Event {
            at: "t".into(),
            by: by.clone(),
            kind,
        }
    }

    /// A root comment. `place` is `(side, line, end_line)`, or `None` for a file comment.
    fn root(
        by: &Author,
        id: &str,
        path: &str,
        old_path: Option<&str>,
        place: Option<(Side, u32, Option<u32>)>,
        body: &str,
    ) -> Event {
        event(
            by,
            Kind::Add(Add {
                id: CommentId::parse(id).unwrap(),
                parent: None,
                path: RelPath::parse(path),
                old_path: old_path.and_then(RelPath::parse),
                side: place.map(|p| p.0),
                line: place.map(|p| p.1),
                end_line: place.and_then(|p| p.2),
                line_text: place.map(|_| "text".to_owned()),
                spec: Some(Spec::WorkTree),
                body: body.into(),
            }),
        )
    }

    fn reply(by: &Author, id: &str, parent: &str, body: &str) -> Event {
        event(
            by,
            Kind::Add(Add {
                id: CommentId::parse(id).unwrap(),
                parent: CommentId::parse(parent),
                path: None,
                old_path: None,
                side: None,
                line: None,
                end_line: None,
                line_text: None,
                spec: None,
                body: body.into(),
            }),
        )
    }

    fn id(text: &str) -> CommentId {
        CommentId::parse(text).unwrap()
    }

    const HEAD: &str = "Address the review comments below. For each one: make the change, then resolve it with a one-line
reply. If you disagree or are unsure, reply and leave it open.

Resolve:
'/p/bin/herdr-review' comment resolve --repo '/r' <id> --reply - <<'EOF'
<one line>
EOF
Reply without resolving:
'/p/bin/herdr-review' comment reply --repo '/r' <id> - <<'EOF'
<text>
EOF

Comments on the diff (L = line in the original file, R = in the changed file):
";

    #[test]
    fn the_prompt_matches_the_layout_in_the_plan() {
        let (user, agent) = (Author::User, Author::Agent(Some("claude".into())));
        let events = [
            root(
                &user,
                "u7",
                "src/lib.rs",
                None,
                Some((Side::New, 42, None)),
                "body text\nmore lines",
            ),
            root(
                &user,
                "u8",
                "src/lib.rs",
                None,
                Some((Side::New, 50, Some(57))),
                "comment on a range",
            ),
            root(
                &user,
                "u9",
                "src/store.rs",
                None,
                None,
                "comment on the whole file",
            ),
            root(
                &user,
                "u3",
                "src/cli.rs",
                None,
                Some((Side::New, 10, None)),
                "the original comment",
            ),
            reply(&agent, "a1", "u3", "Added with_capacity"),
            event(&agent, Kind::Resolve { id: id("u3") }),
            reply(&user, "u10", "u3", "still allocates twice"),
            event(&user, Kind::Reopen { id: id("u3") }),
            root(
                &agent,
                "a2",
                "src/cli.rs",
                None,
                Some((Side::New, 88, None)),
                "the agent's comment, first line only\nsecond line",
            ),
            reply(&user, "u11", "a2", "reply text\nsecond"),
            root(
                &user,
                "u12",
                "src/new.rs",
                Some("src/old.rs"),
                Some((Side::Old, 5, None)),
                "old side of a rename",
            ),
        ];
        let review = fold(&events);
        let threads = review.threads.iter().collect::<Vec<_>>();
        let expected = format!(
            "{HEAD}\
- [u7] src/lib.rs:42 (R): body text
  more lines
- [u8] src/lib.rs:50-57 (R): comment on a range
- [u9] src/store.rs (file): comment on the whole file
- [u3] src/cli.rs:10 (R), reopened: the original comment
  > agent: Added with_capacity
  > user: still allocates twice
- [a2] src/cli.rs:88 (R), your comment: the agent's comment, first line only
  > user: reply text
  > second
- [u12] src/old.rs:5 (L): old side of a rename
"
        );
        assert_eq!(
            format(&threads, Path::new("/p/bin/herdr-review"), Path::new("/r")),
            expected
        );
    }

    #[test]
    fn quoting_a_word_gives_back_the_same_word_to_a_posix_shell() {
        for word in [
            "/plain",
            "/with space",
            "/it's",
            "/a'b'c d",
            "''",
            "$(id) `id` \\ \"q\"",
        ] {
            let out = Command::new("sh")
                .args(["-c", &format!("printf %s {}", quote(word))])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), word);
        }
    }

    #[test]
    fn the_commands_in_the_prompt_run_with_a_binary_and_root_that_need_quoting() {
        let base = std::env::temp_dir().join(format!("herdr-review-prompt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bin_dir = base.join("bin dir's");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("herdr-review");
        std::fs::write(
            &bin,
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\ncat\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let root = base.join("re po's");
        let prompt = format(&[], &bin, &root);
        for (verb, tail, stdin) in [
            ("resolve", ["--reply", "-"], "<one line>"),
            ("reply", ["-", ""], "<text>"),
        ] {
            let start = prompt.find(&format!("comment {verb} ")).unwrap();
            let line_start = prompt[..start].rfind('\n').map_or(0, |at| at + 1);
            let end = prompt[line_start..]
                .find("EOF\n")
                .map(|at| line_start + at + 4)
                .unwrap();
            let script = prompt[line_start..end].replace("<id>", "u1");
            let out = Command::new("sh").args(["-c", &script]).output().unwrap();
            let got = String::from_utf8(out.stdout).unwrap();
            let args = [
                "comment",
                verb,
                "--repo",
                root.to_str().unwrap(),
                "u1",
                tail[0],
                tail[1],
            ];
            let mut want = args
                .iter()
                .filter(|part| !part.is_empty())
                .copied()
                .collect::<Vec<_>>();
            want.push(stdin);
            let want = want.join("\n") + "\n";
            assert_eq!(got, want);
        }
    }

    fn agent_json(pane: &str, term: &str, cwd: &str, status: &str, workspace: &str) -> String {
        format!(
            r#"{{"agent":"claude","agent_status":"{status}","cwd":"{cwd}","pane_id":"{pane}","terminal_id":"{term}","workspace_id":"{workspace}"}}"#
        )
    }

    /// A herdr that answers `agent get <pane>` from `panes` and `agent list` with `list`, and
    /// records every call.
    struct Herdr {
        panes: Vec<(&'static str, String)>,
        list: Vec<String>,
        prompt_fails: bool,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Herdr {
        fn new(panes: &[(&'static str, String)], list: &[String]) -> Self {
            Self {
                panes: panes.to_vec(),
                list: list.to_vec(),
                prompt_fails: false,
                calls: std::cell::RefCell::default(),
            }
        }

        fn ask(&self, args: &[String]) -> Result<String, String> {
            let call = args.join(" ");
            self.calls.borrow_mut().push(call.clone());
            if call.starts_with("agent prompt ") {
                return if self.prompt_fails {
                    Err("socket gone".into())
                } else {
                    Ok("{}".into())
                };
            }
            if call == "agent list" {
                return Ok(format!(
                    r#"{{"result":{{"agents":[{}]}}}}"#,
                    self.list.join(",")
                ));
            }
            let pane = call.strip_prefix("agent get ").unwrap_or_default();
            let found = self.panes.iter().find(|(name, _)| *name == pane);
            found.map_or(Err("pane_not_found".into()), |(_, agent)| {
                Ok(format!(r#"{{"result":{{"agent":{agent}}}}}"#))
            })
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn env(vars: &[(&str, &str)]) -> Env {
        Env::new(
            vars.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            "/cwd".into(),
        )
    }

    fn target(pane: &str, term: &str) -> Target {
        Target {
            pane: PaneId::parse(pane).unwrap(),
            terminal: TerminalId::parse(term).unwrap(),
            agent: "claude".into(),
        }
    }

    fn meta_with(pane: &str, term: &str) -> Meta {
        Meta {
            target: Some(target(pane, term)),
            ..Meta::default()
        }
    }

    const ROOT: &str = "/work/repo";
    const PANE_ENV: [(&str, &str); 2] = [
        ("REVIEW_DELIVER_TO", "w1:p1"),
        ("REVIEW_DELIVER_TERM", "term_1"),
    ];

    fn resolve(env: &Env, meta: &Meta, herdr: &Herdr) -> Result<Target, TargetError> {
        resolve_target(env, meta, |args| herdr.ask(args), Path::new(ROOT))
    }

    #[test]
    fn the_pane_the_review_was_opened_beside_is_the_target() {
        let herdr = Herdr::new(
            &[("w1:p1", agent_json("w1:p1", "term_1", ROOT, "idle", "w1"))],
            &[],
        );
        let got = resolve(&env(&PANE_ENV), &Meta::default(), &herdr);
        assert_eq!(got, Ok(target("w1:p1", "term_1")));
        assert_eq!(herdr.calls(), ["agent get w1:p1"]);
    }

    #[test]
    fn an_env_pane_with_another_terminal_id_falls_through_to_the_search() {
        let herdr = Herdr::new(
            &[(
                "w1:p1",
                agent_json("w1:p1", "term_other", ROOT, "idle", "w1"),
            )],
            &[agent_json("w1:p4", "term_1", ROOT, "idle", "w1")],
        );
        let got = resolve(&env(&PANE_ENV), &Meta::default(), &herdr);
        assert_eq!(got, Ok(target("w1:p4", "term_1")));
        assert_eq!(herdr.calls(), ["agent get w1:p1", "agent list"]);
    }

    #[test]
    fn the_target_in_meta_is_used_when_the_env_has_none() {
        let herdr = Herdr::new(
            &[(
                "w1:p2",
                agent_json("w1:p2", "term_2", "/work", "working", "w1"),
            )],
            &[],
        );
        let got = resolve(&env(&[]), &meta_with("w1:p2", "term_2"), &herdr);
        assert_eq!(got, Ok(target("w1:p2", "term_2")));
        assert_eq!(herdr.calls(), ["agent get w1:p2"]);
    }

    #[test]
    fn a_pane_that_meta_and_env_both_name_is_asked_once() {
        let herdr = Herdr::new(&[], &[]);
        let _ = resolve(&env(&PANE_ENV), &meta_with("w1:p1", "term_1"), &herdr);
        assert_eq!(herdr.calls(), ["agent get w1:p1", "agent list"]);
    }

    #[test]
    fn a_stored_pane_that_is_gone_or_moved_away_falls_to_the_list() {
        let outside = agent_json("w1:p2", "term_2", "/elsewhere", "idle", "w1");
        let herdr = Herdr::new(
            &[("w1:p2", outside)],
            &[agent_json("w1:p9", "term_2", ROOT, "idle", "w1")],
        );
        let got = resolve(&env(&[]), &meta_with("w1:p2", "term_2"), &herdr);
        assert_eq!(got, Ok(target("w1:p9", "term_2")));
    }

    #[test]
    fn a_blocked_agent_is_still_found_so_the_send_can_say_why_it_refused() {
        let blocked = agent_json("w1:p1", "term_1", ROOT, "blocked", "w1");
        let herdr = Herdr::new(&[("w1:p1", blocked.clone())], &[blocked]);
        let got = resolve(&env(&PANE_ENV), &Meta::default(), &herdr);
        assert_eq!(got, Ok(target("w1:p1", "term_1")));
    }

    #[test]
    fn a_single_agent_working_in_the_root_is_found_by_cwd() {
        let list = [
            agent_json("w1:p1", "term_1", "/work/other", "idle", "w1"),
            agent_json("w1:p2", "term_2", "/work/repo/crates/a", "idle", "w1"),
        ];
        let got = resolve(&env(&[]), &Meta::default(), &Herdr::new(&[], &list));
        assert_eq!(got, Ok(target("w1:p2", "term_2")));
    }

    #[test]
    fn an_agent_in_an_ancestor_of_the_root_counts_when_nothing_is_closer() {
        let list = [
            agent_json("w1:p1", "term_1", "/work", "idle", "w1"),
            agent_json("w2:p1", "term_9", "/work", "idle", "w2"),
        ];
        let herdr = Herdr::new(&[], &list);
        let got = resolve(
            &env(&[("HERDR_PANE_ID", "w1:p5")]),
            &Meta::default(),
            &herdr,
        );
        assert_eq!(got, Ok(target("w1:p1", "term_1")));
        let nearer = [
            agent_json("w1:p1", "term_1", "/work", "idle", "w1"),
            agent_json("w1:p2", "term_2", ROOT, "idle", "w1"),
        ];
        let got = resolve(&env(&[]), &Meta::default(), &Herdr::new(&[], &nearer));
        assert_eq!(got, Ok(target("w1:p2", "term_2")));
    }

    #[test]
    fn no_agent_for_this_tree_is_an_error() {
        let list = [agent_json("w1:p1", "term_1", "/work/other", "idle", "w1")];
        let got = resolve(&env(&[]), &Meta::default(), &Herdr::new(&[], &list));
        assert_eq!(got, Err(TargetError::NoAgent));
        let got = resolve(&env(&[]), &Meta::default(), &Herdr::new(&[], &[]));
        assert_eq!(got, Err(TargetError::NoAgent));
    }

    #[test]
    fn several_agents_are_a_choice_for_the_user() {
        let list = [
            agent_json("w1:p1", "term_1", ROOT, "idle", "w1"),
            agent_json("w1:p2", "term_2", ROOT, "idle", "w1"),
        ];
        let got = resolve(&env(&[]), &Meta::default(), &Herdr::new(&[], &list));
        assert_eq!(
            got,
            Err(TargetError::Ambiguous(vec![
                target("w1:p1", "term_1"),
                target("w1:p2", "term_2")
            ]))
        );
    }

    #[test]
    fn a_failing_agent_list_is_reported_and_not_taken_for_no_agent() {
        let got = resolve_target(
            &env(&[]),
            &Meta::default(),
            |_| Err("socket gone".into()),
            Path::new(ROOT),
        );
        assert_eq!(got, Err(TargetError::Herdr("socket gone".into())));
    }

    #[test]
    fn the_chosen_target_is_saved_and_not_rewritten_when_it_is_already_there() {
        let dir = std::env::temp_dir().join(format!("herdr-review-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let chosen = target("w1:p2", "term_2");
        save_target(&dir, Path::new(ROOT), &Meta::default(), &chosen).unwrap();
        let saved = meta::load(&dir).0;
        assert_eq!(saved.target, Some(chosen.clone()));
        std::fs::remove_file(dir.join("meta.json")).unwrap();
        save_target(&dir, Path::new(ROOT), &saved, &chosen).unwrap();
        assert!(!dir.join("meta.json").exists());
    }

    // The send.

    const PLUGIN_ENV: [(&str, &str); 3] = [
        ("REVIEW_DELIVER_TO", "w1:p1"),
        ("REVIEW_DELIVER_TERM", "term_1"),
        ("HERDR_PLUGIN_ROOT", "/plugin"),
    ];

    fn fresh(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-review-send-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn put(dir: &Path, events: Vec<Event>) {
        write(dir, "t", |_, _| Ok::<_, ()>((events, ()))).unwrap();
    }

    fn user_comment(id: &str, body: &str) -> Event {
        root(
            &Author::User,
            id,
            "src/a.rs",
            None,
            Some((Side::New, 3, None)),
            body,
        )
    }

    fn agent_ready_in(status: &str) -> Herdr {
        Herdr::new(
            &[("w1:p1", agent_json("w1:p1", "term_1", ROOT, status, "w1"))],
            &[],
        )
    }

    fn run_send(dir: &Path, herdr: &Herdr, all_open: bool) -> Result<Sent, SendError> {
        let scope = if all_open {
            Scope::AllOpen
        } else {
            Scope::Unsent
        };
        send(
            dir,
            Path::new(ROOT),
            &env(&PLUGIN_ENV),
            "t",
            &scope,
            |args| herdr.ask(args),
        )
    }

    fn prompts(herdr: &Herdr) -> Vec<String> {
        herdr
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("agent prompt "))
            .collect()
    }

    fn sent_ids(dir: &Path) -> Vec<String> {
        let log = std::fs::read_to_string(dir.join("review.jsonl")).unwrap();
        log.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["kind"] == "sent")
            .flat_map(|event| event["ids"].as_array().cloned().unwrap_or_default())
            .map(|id| id.as_str().unwrap().to_owned())
            .collect()
    }

    fn unsent(dir: &Path) -> Vec<String> {
        let review = read(dir).unwrap();
        let unsent = review.threads.iter().filter(|t| t.unsent);
        unsent.map(|t| t.root.id.to_string()).collect()
    }

    fn agent_answers(id: &str, reply_id: &str) -> Vec<Event> {
        let agent = Author::Agent(Some("claude".into()));
        vec![
            reply(&agent, reply_id, id, "done"),
            event(
                &agent,
                Kind::Resolve {
                    id: CommentId::parse(id).unwrap(),
                },
            ),
        ]
    }

    #[test]
    fn nothing_unsent_is_nothing_to_send_and_asks_herdr_nothing() {
        let dir = fresh("none");
        put(&dir, vec![user_comment("u1", "x")]);
        let herdr = agent_ready_in("idle");
        run_send(&dir, &herdr, false).unwrap();
        let again = run_send(&dir, &herdr, false).unwrap();
        assert_eq!(again.outcome, SendOutcome::Nothing);
        assert_eq!(again.outcome.to_string(), "nothing to send");
        assert_eq!(herdr.calls().len(), 3, "{:?}", herdr.calls());
        let empty = fresh("empty");
        let herdr = agent_ready_in("idle");
        assert_eq!(
            run_send(&empty, &herdr, false).unwrap().outcome,
            SendOutcome::Nothing
        );
        assert!(herdr.calls().is_empty());
    }

    #[test]
    fn a_send_delivers_one_prompt_marks_every_comment_sent_and_remembers_the_target() {
        let dir = fresh("happy");
        put(
            &dir,
            vec![user_comment("u1", "first"), user_comment("u2", "second")],
        );
        let herdr = agent_ready_in("idle");
        let sent = run_send(&dir, &herdr, false).unwrap();
        assert_eq!(
            sent,
            Sent {
                outcome: SendOutcome::Sent {
                    n: 2,
                    agent: "claude".into()
                },
                target: Some(target("w1:p1", "term_1")),
                warnings: vec![]
            }
        );
        assert_eq!(sent.outcome.to_string(), "sent 2 to claude");
        let calls = prompts(&herdr);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("agent prompt w1:p1 Address the review comments"));
        assert!(
            calls[0].contains("'/plugin/bin/herdr-review' comment resolve --repo '/work/repo'")
        );
        assert!(calls[0].contains("- [u1] src/a.rs:3 (R): first\n- [u2] src/a.rs:3 (R): second"));
        assert_eq!(sent_ids(&dir), ["u1", "u2"]);
        assert_eq!(unsent(&dir), Vec::<String>::new());
        assert_eq!(meta::load(&dir).0.target, Some(target("w1:p1", "term_1")));
    }

    #[test]
    fn a_working_agent_is_sent_to_and_the_comments_are_queued() {
        let dir = fresh("working");
        put(&dir, vec![user_comment("u1", "x")]);
        let herdr = agent_ready_in("working");
        let sent = run_send(&dir, &herdr, false).unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Queued {
                n: 1,
                agent: "claude".into()
            }
        );
        assert_eq!(
            sent.outcome.to_string(),
            "agent is working, 1 comments queued"
        );
        assert_eq!(sent_ids(&dir), ["u1"]);
    }

    #[test]
    fn a_refusal_marks_nothing_and_says_why() {
        let dir = fresh("refused");
        put(&dir, vec![user_comment("u1", "x")]);
        let blocked = agent_json("w1:p1", "term_1", ROOT, "blocked", "w1");
        let herdr = Herdr::new(&[("w1:p1", blocked.clone())], &[blocked]);
        let refused = run_send(&dir, &herdr, false);
        assert!(
            matches!(&refused, Err(SendError::Refused(why))
                if why.contains("waiting on a prompt") && why.contains("still unsent")),
            "{refused:?}"
        );
        assert!(prompts(&herdr).is_empty());
        assert_eq!(unsent(&dir), ["u1"]);
        assert!(sent_ids(&dir).is_empty());
        assert_eq!(meta::load(&dir).0.target, None);
    }

    #[test]
    fn a_failed_prompt_is_not_retried_and_marks_nothing() {
        let dir = fresh("prompt-fails");
        put(&dir, vec![user_comment("u1", "x")]);
        let mut herdr = agent_ready_in("idle");
        herdr.prompt_fails = true;
        assert!(matches!(
            run_send(&dir, &herdr, false),
            Err(SendError::Refused(_))
        ));
        assert_eq!(prompts(&herdr).len(), 1);
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn no_agent_and_several_agents_stop_the_send_before_any_prompt() {
        let dir = fresh("no-target");
        put(&dir, vec![user_comment("u1", "x")]);
        let none = Herdr::new(&[], &[]);
        assert_eq!(
            run_send(&dir, &none, false),
            Err(SendError::Target(TargetError::NoAgent))
        );
        let two = Herdr::new(
            &[],
            &[
                agent_json("w1:p2", "term_2", ROOT, "idle", "w1"),
                agent_json("w1:p3", "term_3", ROOT, "idle", "w1"),
            ],
        );
        assert!(matches!(
            run_send(&dir, &two, false),
            Err(SendError::Target(TargetError::Ambiguous(found))) if found.len() == 2
        ));
        assert!(prompts(&none).is_empty() && prompts(&two).is_empty());
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn a_sent_event_that_cannot_be_written_is_a_warning_and_the_send_still_succeeds() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fresh("not-recorded");
        put(&dir, vec![user_comment("u1", "x")]);
        let herdr = agent_ready_in("idle");
        let log = dir.join("review.jsonl");
        let sent = send(
            &dir,
            Path::new(ROOT),
            &env(&PLUGIN_ENV),
            "t",
            &Scope::Unsent,
            |args| {
                if args.get(1).is_some_and(|word| word == "prompt") {
                    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o400)).unwrap();
                }
                herdr.ask(args)
            },
        )
        .unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Sent {
                n: 1,
                agent: "claude".into()
            }
        );
        assert_eq!(sent.warnings, [Warning::SentNotRecorded]);
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn a_target_that_cannot_be_saved_is_a_warning_and_the_send_still_succeeds() {
        let dir = fresh("not-saved");
        put(&dir, vec![user_comment("u1", "x")]);
        std::fs::create_dir_all(dir.join("meta.json")).unwrap();
        let sent = run_send(&dir, &agent_ready_in("idle"), false).unwrap();
        assert!(matches!(sent.outcome, SendOutcome::Sent { .. }));
        assert_eq!(
            sent.warnings,
            [Warning::MetaUnreadable, Warning::TargetNotSaved]
        );
        assert_eq!(unsent(&dir), Vec::<String>::new());
    }

    #[test]
    fn a_reopened_thread_goes_out_alone_marked_reopened() {
        let dir = fresh("reopen");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        run_send(&dir, &agent_ready_in("idle"), false).unwrap();
        put(
            &dir,
            [agent_answers("u1", "a1"), agent_answers("u2", "a2")].concat(),
        );
        put(
            &dir,
            vec![event(
                &Author::User,
                Kind::Reopen {
                    id: CommentId::parse("u2").unwrap(),
                },
            )],
        );
        assert_eq!(unsent(&dir), ["u2"]);
        let herdr = agent_ready_in("idle");
        let sent = run_send(&dir, &herdr, false).unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Sent {
                n: 1,
                agent: "claude".into()
            }
        );
        let prompt = &prompts(&herdr)[0];
        assert!(
            prompt.contains("- [u2] src/a.rs:3 (R), reopened: two"),
            "{prompt}"
        );
        assert!(!prompt.contains("[u1]"));
        assert_eq!(sent_ids(&dir), ["u1", "u2", "u2"]);
        let again = run_send(&dir, &agent_ready_in("idle"), false).unwrap();
        assert_eq!(again.outcome, SendOutcome::Nothing);
    }

    #[test]
    fn all_open_resends_open_threads_and_clears_the_edited_mark() {
        let dir = fresh("all-open");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        run_send(&dir, &agent_ready_in("idle"), false).unwrap();
        put(&dir, agent_answers("u2", "a1"));
        put(
            &dir,
            vec![event(
                &Author::User,
                Kind::Edit {
                    id: CommentId::parse("u1").unwrap(),
                    body: "one, again".into(),
                },
            )],
        );
        assert!(read(&dir).unwrap().threads[0].root.edited_since_sent);
        assert_eq!(
            run_send(&dir, &agent_ready_in("idle"), false)
                .unwrap()
                .outcome,
            SendOutcome::Nothing
        );
        let herdr = agent_ready_in("idle");
        let sent = run_send(&dir, &herdr, true).unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Sent {
                n: 1,
                agent: "claude".into()
            }
        );
        let prompt = &prompts(&herdr)[0];
        assert!(prompt.contains("[u1] src/a.rs:3 (R): one, again") && !prompt.contains("[u2]"));
        assert!(!read(&dir).unwrap().threads[0].root.edited_since_sent);
    }

    #[test]
    fn one_thread_can_be_sent_again_on_its_own() {
        let dir = fresh("one-thread");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        run_send(&dir, &agent_ready_in("idle"), false).unwrap();
        let herdr = agent_ready_in("idle");
        let scope = Scope::Thread(CommentId::parse("u2").unwrap());
        let sent = send(
            &dir,
            Path::new(ROOT),
            &env(&PLUGIN_ENV),
            "t",
            &scope,
            |args| herdr.ask(args),
        )
        .unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Sent {
                n: 1,
                agent: "claude".into()
            }
        );
        let prompt = &prompts(&herdr)[0];
        assert!(prompt.contains("[u2]") && !prompt.contains("[u1]"));
    }

    #[test]
    fn the_action_reports_every_outcome_as_a_notification() {
        let note = |result: &Result<Sent, SendError>| {
            let mut seen = Vec::new();
            let code = report(result, |title, body| {
                seen.push((title.to_owned(), body.map(str::to_owned)));
            });
            (code, seen)
        };
        let ok = Ok(Sent {
            outcome: SendOutcome::Sent {
                n: 2,
                agent: "claude".into(),
            },
            target: None,
            warnings: vec![Warning::SentNotRecorded],
        });
        let (code, seen) = note(&ok);
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(seen[0].0, "review: sent 2 to claude");
        assert!(seen[0].1.as_deref().unwrap().contains("not recorded"));
        let (code, seen) = note(&Err(SendError::Target(TargetError::NoAgent)));
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(
            seen,
            [(
                "review: not sent".to_owned(),
                Some("No agent found for this review.".to_owned())
            )]
        );
        let (code, seen) = note(&Ok(Sent {
            outcome: SendOutcome::Nothing,
            target: None,
            warnings: vec![],
        }));
        assert_eq!(
            (code, seen),
            (
                ExitCode::SUCCESS,
                vec![("review: nothing to send".to_owned(), None)]
            )
        );
    }
}
