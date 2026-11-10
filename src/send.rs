//! Send: the prompt the agent receives, and which agent receives it.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agent_delivery::{Delivery, agent_ready, deliver_to_agent};
use crate::cli::{Failure, Output};
use crate::comment::CommandError;
use crate::diff::{GitError, RepoRoot};
use crate::env::Env;
use crate::hc::{self, HcError};
use crate::json::{CandidateJson, SCHEMA, SendErrorJson, SendErrorKind, SendJson, TargetJson};
use crate::meta::{self, HcTarget, Meta, Target};
use crate::open::parse_context;
use crate::store::{
    Anchor, AnchorTarget, Author, Comment, CommentId, Event, Kind, PaneId, Side, StoreError,
    TerminalAddress, TerminalId, Thread, Warning, read, write,
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

/// An agent `hc ls` lists, which the user can pick when several match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub address: TerminalAddress,
    pub agent: String,
    pub cwd: PathBuf,
    pub status: String,
}

/// Resolution found no agent, or could not choose between several.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    NoAgent,
    Ambiguous(Vec<Target>),
    /// Several agents match under the hc backend.
    AmbiguousHc(Vec<Candidate>),
    /// `herdr agent list` or `hc ls` failed, so the search could not run.
    Backend(String),
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
            Self::AmbiguousHc(found) => {
                let names = found
                    .iter()
                    .map(|c| format!("{} ({}, {})", c.address, c.agent, c.cwd.display()))
                    .collect::<Vec<_>>();
                write!(
                    f,
                    "{} agents match, pass --target: {}",
                    found.len(),
                    names.join(", ")
                )
            }
            Self::Backend(message) => write!(f, "Cannot list agents: {message}"),
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
pub fn belongs(cwd: &Path, root: &Path) -> bool {
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
    let agents = listed(&herdr(&arguments(&["agent", "list"])).map_err(TargetError::Backend)?);
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

/// The agent a prompt reached, with the key of its backend in `meta.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reached {
    Herdr(Target),
    Hc(HcTarget),
}

impl Reached {
    fn agent(&self) -> &str {
        match self {
            Self::Herdr(target) => &target.agent,
            Self::Hc(target) => &target.agent,
        }
    }

    fn json(&self) -> TargetJson {
        match self {
            Self::Herdr(target) => TargetJson::Herdr {
                pane_id: target.pane.to_string(),
                terminal_id: target.terminal.to_string(),
                agent: target.agent.clone(),
            },
            Self::Hc(target) => TargetJson::Hc {
                address: target.address.to_string(),
                agent: target.agent.clone(),
            },
        }
    }
}

/// A send that delivered, or had nothing to deliver, and what went wrong on the side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub outcome: SendOutcome,
    /// The agent that got the prompt. `None` when there was nothing to send.
    pub target: Option<Reached>,
    pub warnings: Vec<Warning>,
}

/// Why the agent cannot take a prompt now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Blocked,
    NotReady,
    AgentGone,
}

impl Refusal {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Blocked => "The agent is waiting on a prompt.",
            Self::NotReady => "The agent is not ready for input yet.",
            Self::AgentGone => "No agent is running in that terminal.",
        }
    }

    /// The refusal Herdr's own words for it name. Herdr's other failures are not refusals.
    fn of_herdr(message: &str) -> Option<Self> {
        let has = |words: &[&str]| words.iter().any(|word| message.contains(word));
        if has(&["waiting on a prompt"]) {
            Some(Self::Blocked)
        } else if has(&["not ready", "did not report"]) {
            Some(Self::NotReady)
        } else if has(&["No agent is running", "no longer exists"]) {
            Some(Self::AgentGone)
        } else {
            None
        }
    }
}

/// Why nothing was delivered. No comment is marked sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    Store(StoreError),
    Target(TargetError),
    /// A named thread is not an open one.
    Command(CommandError),
    /// The agent cannot take a prompt now. The text says nothing was sent.
    Refused(Refusal, String),
    /// `herdr` or `hc` failed, or the repository could not be found. The text says nothing was sent
    /// when that is so.
    Failed(String),
}

impl SendError {
    pub fn refused(why: Refusal) -> Self {
        Self::Refused(
            why,
            format!(
                "{} Nothing was sent; your comments are still unsent.",
                why.reason()
            ),
        )
    }

    /// Herdr's refusal as a refusal, and anything else Herdr said as a failure.
    fn from_herdr(message: String) -> Self {
        Refusal::of_herdr(&message).map_or(Self::Failed(message.clone()), |why| {
            Self::Refused(why, message)
        })
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Target(error) => error.fmt(f),
            Self::Command(error) => error.fmt(f),
            Self::Refused(_, message) | Self::Failed(message) => f.write_str(message),
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
    /// Exactly these open threads, whatever they were sent before.
    Threads(Vec<String>),
}

/// Remember the hc agent for the next send. Writes nothing when `meta.json` already holds it.
pub fn save_hc_target(
    dir: &Path,
    root: &Path,
    meta: &Meta,
    target: &HcTarget,
) -> Result<(), StoreError> {
    if meta.hc_target.as_ref() == Some(target) {
        return Ok(());
    }
    meta::save(dir, root, |meta| meta.hc_target = Some(target.clone()))
}

/// Deliver the threads `scope` names as one prompt (design/send.md, steps), then record the send.
///
/// `bin` is the binary the prompt tells the agent to run. `reach` resolves the target and delivers
/// the text: it gets the saved `meta` and the prompt, and says who was reached and whether that agent
/// was working.
///
/// Nothing is retried. A second prompt after an unclear failure could deliver the batch twice, so the
/// user presses the key again. Once the prompt is accepted the send is a success, and a failed write
/// of the `sent` event is only a warning.
fn send_with(
    dir: &Path,
    root: &Path,
    now: &str,
    scope: &Scope,
    bin: &Path,
    reach: impl FnOnce(&Meta, &str) -> Result<(Reached, bool), SendError>,
) -> Result<Sent, SendError> {
    let review = read(dir).map_err(SendError::Store)?;
    if let Scope::Threads(ids) = scope {
        let open = review.open_ids();
        if let Some(id) = ids
            .iter()
            .find(|id| !open.iter().any(|open| open.as_str() == *id))
        {
            return Err(SendError::Command(CommandError::UnknownId {
                id: id.clone(),
                open: open.into_iter().cloned().collect(),
            }));
        }
    }
    let threads = review
        .threads
        .iter()
        .filter(|thread| match scope {
            Scope::Unsent => thread.unsent,
            Scope::AllOpen => thread.unsent || thread.is_open(),
            Scope::Threads(ids) => ids.iter().any(|id| id == thread.root.id.as_str()),
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
    let text = format(&threads, bin, root);
    let (target, working) = reach(&meta, &text)?;
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
    let saved = match &target {
        Reached::Herdr(target) => save_target(dir, root, &meta, target),
        Reached::Hc(target) => save_hc_target(dir, root, &meta, target),
    };
    if saved.is_err() {
        warnings.push(Warning::TargetNotSaved);
    }
    let (n, agent) = (threads.len(), target.agent().to_owned());
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

/// Send under the herdr backend: `herdr agent get`, `list` and `prompt`.
pub fn send(
    dir: &Path,
    root: &Path,
    env: &Env,
    now: &str,
    scope: &Scope,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<Sent, SendError> {
    send_with(dir, root, now, scope, &plugin_bin(env), |meta, text| {
        let target = resolve_target(env, meta, &mut herdr, root).map_err(SendError::Target)?;
        let mut working = false;
        deliver_to_agent(Delivery::Send, Some(target.pane.as_str()), text, |args| {
            let got = herdr(args);
            if args.starts_with(&arguments(&["agent", "get"])) {
                working = is_working(&got);
            }
            got
        })
        .map_err(SendError::from_herdr)?;
        Ok((Reached::Herdr(target), working))
    })
}

/// Send under the hc backend: `hc get`, `ls` and `send`. `explicit` is the `--target` address. The
/// prompt names the running binary, since herdr-app sets no plugin root and the one on `PATH` may be
/// older (design/send.md, prompt).
pub fn send_hc(
    dir: &Path,
    root: &Path,
    env: &Env,
    now: &str,
    scope: &Scope,
    explicit: Option<&TerminalAddress>,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Sent, SendError> {
    let bin = if env.exe.as_os_str().is_empty() {
        PathBuf::from("herdr-review")
    } else {
        env.exe.clone()
    };
    send_with(dir, root, now, scope, &bin, |meta, text| {
        let record = hc::resolve(explicit, meta, root, &mut hc).map_err(SendError::Target)?;
        let working = hc::deliver(&record, text, &mut hc)?;
        let target = HcTarget {
            address: record.address,
            agent: record.agent.unwrap_or_else(|| "agent".to_owned()),
        };
        Ok((Reached::Hc(target), working))
    })
}

/// The `send` action's edge under the herdr backend: every outcome and warning becomes a
/// notification, since an action has no terminal. A refusal and a missing target exit 1.
fn report(result: &Result<Sent, SendError>, mut notify: impl FnMut(&str, Option<&str>)) -> u8 {
    match result {
        Ok(sent) => {
            let warnings = sent
                .warnings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let body = (!warnings.is_empty()).then(|| warnings.join("; "));
            notify(&format!("review: {}", sent.outcome), body.as_deref());
            0
        }
        Err(error) => {
            notify("review: not sent", Some(&error.to_string()));
            1
        }
    }
}

/// Which CLI `send` talks to (design/cli.md, backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Herdr,
    Hc,
}

/// The flags of `send`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendArgs {
    pub repo: Option<PathBuf>,
    pub all_open: bool,
    pub backend: Option<Backend>,
    /// An hc address, which implies the hc backend.
    pub target: Option<TerminalAddress>,
    /// Ids of the open threads to send again, and no others.
    pub threads: Vec<String>,
    pub json: bool,
}

/// `--backend`, else hc for `--target` or when hc is the nearest host (`Env::hc_terminal`), else herdr.
fn backend_of(args: &SendArgs, env: &Env) -> Backend {
    args.backend.unwrap_or_else(|| {
        if args.target.is_some() || env.hc_terminal().is_some() {
            Backend::Hc
        } else {
            Backend::Herdr
        }
    })
}

fn attempt(
    args: &SendArgs,
    backend: Backend,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
    herdr: impl FnMut(&[String]) -> Result<String, String>,
    hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Result<Sent, SendError> {
    let context = parse_context(env.get("HERDR_PLUGIN_CONTEXT_JSON"));
    let cwd = context.cwd.unwrap_or_else(|| env.cwd.clone());
    let root = RepoRoot::resolve(args.repo.as_deref(), &cwd, git)
        .map_err(|error| SendError::Failed(error.to_string()))?;
    let base = env
        .state_base()
        .ok_or_else(|| SendError::Failed("set HOME or XDG_STATE_HOME".to_owned()))?;
    let dir = meta::locate(&base, root.path());
    let scope = if !args.threads.is_empty() {
        Scope::Threads(args.threads.clone())
    } else if args.all_open {
        Scope::AllOpen
    } else {
        Scope::Unsent
    };
    match backend {
        Backend::Herdr => send(&dir, root.path(), env, now, &scope, herdr),
        Backend::Hc => send_hc(
            &dir,
            root.path(),
            env,
            now,
            &scope,
            args.target.as_ref(),
            hc,
        ),
    }
}

fn error_json(error: &SendError) -> SendErrorJson {
    let kind = match error {
        SendError::Store(StoreError::Busy) => SendErrorKind::Busy,
        SendError::Target(TargetError::NoAgent) => SendErrorKind::NoAgent,
        SendError::Target(TargetError::Ambiguous(_) | TargetError::AmbiguousHc(_)) => {
            SendErrorKind::Ambiguous
        }
        SendError::Refused(Refusal::Blocked, _) => SendErrorKind::Blocked,
        SendError::Refused(Refusal::NotReady, _) => SendErrorKind::NotReady,
        SendError::Refused(Refusal::AgentGone, _) => SendErrorKind::AgentGone,
        SendError::Store(_)
        | SendError::Target(TargetError::Backend(_))
        | SendError::Command(_)
        | SendError::Failed(_) => SendErrorKind::Backend,
    };
    // Under herdr the picker is the TUI's, so only hc candidates are listed.
    let candidates = match error {
        SendError::Target(TargetError::AmbiguousHc(found)) => found
            .iter()
            .map(|c| CandidateJson {
                address: c.address.to_string(),
                agent: c.agent.clone(),
                cwd: c.cwd.to_string_lossy().into_owned(),
                status: c.status.clone(),
            })
            .collect(),
        _ => Vec::new(),
    };
    SendErrorJson {
        schema: SCHEMA,
        error: kind,
        message: error.to_string(),
        candidates,
    }
}

/// One JSON object in every outcome, exit 0 when a send happened or there was nothing to send.
fn json_output(result: &Result<Sent, SendError>) -> Output {
    let (body, code) = match result {
        Ok(sent) => {
            let (n, agent, queued) = match &sent.outcome {
                SendOutcome::Nothing => (0, None, false),
                SendOutcome::Sent { n, agent } => (*n, Some(agent.clone()), false),
                SendOutcome::Queued { n, agent } => (*n, Some(agent.clone()), true),
            };
            let reply = SendJson {
                schema: SCHEMA,
                sent: n,
                queued,
                agent,
                target: sent.target.as_ref().map(Reached::json),
                warnings: sent.warnings.iter().map(ToString::to_string).collect(),
            };
            (serde_json::to_string(&reply), 0)
        }
        Err(error) => (serde_json::to_string(&error_json(error)), 1),
    };
    Output {
        stdout: format!("{}\n", body.unwrap_or_default()),
        code,
        ..Output::default()
    }
}

/// `send` as a command: the TUI plugin's action under herdr, and the panel's call under hc. `herdr`,
/// `hc` and `git` answer every call to those tools. A named thread that is not open is a usage error,
/// exit 2 with a line on stderr, also with `--json`.
pub fn command(
    args: &SendArgs,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
    hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Output {
    let backend = backend_of(args, env);
    let result = attempt(args, backend, env, now, git, &mut herdr, hc);
    if let Err(SendError::Command(error)) = &result {
        return Failure::Command(error.clone()).output();
    }
    if args.json {
        return json_output(&result);
    }
    match (backend, &result) {
        (Backend::Herdr, _) => Output {
            code: report(&result, |title, body| {
                let mut call = ["notification", "show", title].map(str::to_owned).to_vec();
                call.extend(
                    body.into_iter()
                        .flat_map(|body| ["--body", body])
                        .map(str::to_owned),
                );
                let _ = herdr(&call);
            }),
            ..Output::default()
        },
        (Backend::Hc, Ok(sent)) => Output {
            stdout: format!("{}\n", sent.outcome),
            stderr: sent
                .warnings
                .iter()
                .fold(String::new(), |mut text, warning| {
                    let _ = writeln!(text, "warning: {warning}");
                    text
                }),
            code: 0,
        },
        (Backend::Hc, Err(error)) => Output {
            stderr: format!("{error}\n"),
            code: 1,
            ..Output::default()
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
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
        assert_eq!(got, Err(TargetError::Backend("socket gone".into())));
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
                target: Some(Reached::Herdr(target("w1:p1", "term_1"))),
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
            matches!(&refused, Err(SendError::Refused(Refusal::Blocked, why))
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
            Err(SendError::Failed(_))
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
        let scope = Scope::Threads(vec!["u2".to_owned()]);
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
        assert_eq!(code, 0);
        assert_eq!(seen[0].0, "review: sent 2 to claude");
        assert!(seen[0].1.as_deref().unwrap().contains("not recorded"));
        let (code, seen) = note(&Err(SendError::Target(TargetError::NoAgent)));
        assert_eq!(code, 1);
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
            (0, vec![("review: nothing to send".to_owned(), None)])
        );
    }

    // The hc backend, with `hc` faked.

    fn failure(exit: i32, kind: &str) -> HcError {
        HcError {
            exit: Some(exit),
            kind: Some(kind.into()),
            message: format!("{kind} happened"),
        }
    }

    /// What `hc get --json` prints. `status` is the state word, or `None` before the agent reports.
    fn terminal(
        address: &str,
        cwd: &str,
        state: &str,
        status: Option<&str>,
        reported: bool,
    ) -> String {
        let status = status.map_or("null".to_owned(), |word| {
            format!(r#"{{"app":"claude-code","state":"{word}"}}"#)
        });
        format!(
            r#"{{"address":"{address}","cwd":"{cwd}","state":"{state}","status":{status},"reported":{reported}}}"#
        )
    }

    fn live(address: &str, cwd: &str, status: &str) -> String {
        terminal(address, cwd, "live", Some(status), true)
    }

    /// One terminal of `hc ls --json`. A plain program has no `status`.
    fn listed_entry(address: &str, agent: &str, cwd: &str, status: Option<&str>) -> String {
        let status = status.map_or(String::new(), |word| format!(r#","status":"{word}""#));
        format!(r#"{{"address":"{address}","agent":"{agent}","cwd":"{cwd}"{status}}}"#)
    }

    const IDLE_REPLY: &str = r#"{"started":true,"status":"idle","turn":0}"#;

    /// An hc that answers `get <t> --json` from `gets` (exit 3 for any other terminal), `ls`
    /// from `list` and `send` from `reply`, and records every call.
    struct Hc {
        gets: Vec<(&'static str, String)>,
        list: Result<String, HcError>,
        reply: Result<String, HcError>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Hc {
        fn new(gets: &[(&'static str, String)]) -> Self {
            Self {
                gets: gets.to_vec(),
                list: Ok(r#"{"hidden_finished":0,"terminals":[]}"#.into()),
                reply: Ok(IDLE_REPLY.into()),
                calls: std::cell::RefCell::default(),
            }
        }

        fn listing(mut self, entries: &[String]) -> Self {
            self.list = Ok(format!(
                r#"{{"hidden_finished":0,"terminals":[{}]}}"#,
                entries.join(",")
            ));
            self
        }

        fn ask(&self, args: &[String]) -> Result<String, HcError> {
            self.calls.borrow_mut().push(args.join(" "));
            match args.first().map(String::as_str) {
                Some("get") => {
                    let found = self.gets.iter().find(|(name, _)| *name == args[1]);
                    found.map_or(Err(failure(3, "not_found")), |(_, record)| {
                        Ok(record.clone())
                    })
                }
                Some("ls") => self.list.clone(),
                _ => self.reply.clone(),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn sends(&self) -> Vec<String> {
            self.calls()
                .into_iter()
                .filter(|call| call.starts_with("send "))
                .collect()
        }
    }

    fn hc_env() -> Env {
        env(&[]).with_exe("/opt/bin/herdr-review".into())
    }

    fn address(text: &str) -> TerminalAddress {
        TerminalAddress::parse(text).unwrap()
    }

    fn hc_meta(text: &str) -> Meta {
        Meta {
            hc_target: Some(HcTarget {
                address: address(text),
                agent: "claude-code".into(),
            }),
            ..Meta::default()
        }
    }

    fn resolve_hc(explicit: Option<&str>, meta: &Meta, hc: &Hc) -> Result<hc::Record, TargetError> {
        let explicit = explicit.map(address);
        hc::resolve(explicit.as_ref(), meta, Path::new(ROOT), |args| {
            hc.ask(args)
        })
    }

    fn run_hc_send(
        dir: &Path,
        explicit: Option<&str>,
        scope: &Scope,
        hc: &Hc,
    ) -> Result<Sent, SendError> {
        let explicit = explicit.map(address);
        send_hc(
            dir,
            Path::new(ROOT),
            &hc_env(),
            "t",
            scope,
            explicit.as_ref(),
            |args| hc.ask(args),
        )
    }

    fn hc_target_of(dir: &Path) -> Option<HcTarget> {
        meta::load(dir).0.hc_target
    }

    #[test]
    fn an_address_from_the_caller_is_the_target_and_its_folder_is_not_checked() {
        let hc = Hc::new(&[("t7", live("t7", "/somewhere/else", "idle"))]);
        let got = resolve_hc(Some("t7"), &Meta::default(), &hc).unwrap();
        assert_eq!(got.address, address("t7"));
        assert_eq!(hc.calls(), ["get t7 --json"]);
    }

    #[test]
    fn the_saved_address_is_used_when_it_is_live_an_agent_and_in_the_tree() {
        let hc = Hc::new(&[("t7", live("t7", "/work/repo/crates/a", "working"))]);
        let got = resolve_hc(None, &hc_meta("t7"), &hc).unwrap();
        assert_eq!(got.address, address("t7"));
        assert_eq!(hc.calls(), ["get t7 --json"], "no listing");
    }

    #[test]
    fn a_saved_address_that_is_gone_or_finished_falls_to_the_list() {
        let finished = terminal("t7", ROOT, "finished", None, true);
        let hc = Hc::new(&[("t5", live("t5", ROOT, "idle")), ("t7", finished)])
            .listing(&[listed_entry("t5", "claude-code", ROOT, Some("idle"))]);
        for saved in ["t7", "t3"] {
            let got = resolve_hc(None, &hc_meta(saved), &hc).unwrap();
            assert_eq!(got.address, address("t5"));
        }
    }

    #[test]
    fn a_chosen_address_that_is_gone_or_finished_is_refused_and_no_other_agent_gets_the_prompt() {
        let finished = terminal("t7", ROOT, "finished", None, true);
        let hc = Hc::new(&[("t5", live("t5", ROOT, "idle")), ("t7", finished)])
            .listing(&[listed_entry("t5", "claude-code", ROOT, Some("idle"))]);
        for chosen in ["t7", "t3"] {
            let got = resolve_hc(Some(chosen), &hc_meta("t5"), &hc).unwrap();
            assert_eq!(got.address, address(chosen));
            assert_eq!(hc::ready(&got), Err(Refusal::AgentGone));
        }
        assert_eq!(
            hc.calls(),
            ["get t7 --json", "get t3 --json"],
            "no other candidate asked"
        );
    }

    #[test]
    fn a_saved_address_in_another_folder_or_not_yet_an_agent_falls_to_the_list() {
        for stale in [
            live("t7", "/another/project", "idle"),
            terminal("t7", ROOT, "live", None, false),
        ] {
            let hc = Hc::new(&[("t7", stale), ("t5", live("t5", ROOT, "idle"))])
                .listing(&[listed_entry("t5", "claude-code", ROOT, Some("idle"))]);
            let got = resolve_hc(None, &hc_meta("t7"), &hc).unwrap();
            assert_eq!(got.address, address("t5"));
        }
    }

    #[test]
    fn the_list_finds_one_agent_in_the_root_or_inside_it_else_one_above_it() {
        let inside = Hc::new(&[("t5", live("t5", "/work/repo/sub", "idle"))]).listing(&[
            listed_entry("t5", "claude-code", "/work/repo/sub", Some("idle")),
            listed_entry("t6", "fish", ROOT, None),
            listed_entry("t8", "claude-code", "/work/other", Some("idle")),
        ]);
        let got = resolve_hc(None, &Meta::default(), &inside).unwrap();
        assert_eq!(got.address, address("t5"));
        let above = Hc::new(&[("t9", live("t9", "/work", "idle"))]).listing(&[listed_entry(
            "t9",
            "claude-code",
            "/work",
            Some("idle"),
        )]);
        let got = resolve_hc(None, &Meta::default(), &above).unwrap();
        assert_eq!(got.address, address("t9"));
    }

    #[test]
    fn several_agents_in_the_tree_are_a_choice_that_names_each() {
        let hc = Hc::new(&[]).listing(&[
            listed_entry("t5", "claude-code", ROOT, Some("idle")),
            listed_entry("t6", "codex", "/work/repo/sub", Some("working")),
        ]);
        let got = resolve_hc(None, &Meta::default(), &hc);
        let Err(TargetError::AmbiguousHc(found)) = got else {
            panic!("{got:?}");
        };
        assert_eq!(
            found
                .iter()
                .map(|c| (c.address.as_str(), c.agent.as_str(), c.status.as_str()))
                .collect::<Vec<_>>(),
            [("t5", "claude-code", "idle"), ("t6", "codex", "working")]
        );
        assert_eq!(found[1].cwd, PathBuf::from("/work/repo/sub"));
        let message = TargetError::AmbiguousHc(found).to_string();
        assert!(message.contains("t5 (claude-code, /work/repo)") && message.contains("t6"));
    }

    #[test]
    fn no_agent_for_this_tree_and_a_failing_list_are_different_errors() {
        let plain = Hc::new(&[]).listing(&[
            listed_entry("t6", "fish", ROOT, None),
            listed_entry("t8", "claude-code", "/work/other", Some("idle")),
        ]);
        assert_eq!(
            resolve_hc(None, &Meta::default(), &plain),
            Err(TargetError::NoAgent)
        );
        let mut down = Hc::new(&[]);
        down.list = Err(failure(1, "core_down"));
        assert_eq!(
            resolve_hc(None, &Meta::default(), &down),
            Err(TargetError::Backend("core_down happened".into()))
        );
    }

    #[test]
    fn a_terminal_that_has_not_reported_is_not_ready_and_nothing_is_typed() {
        let dir = fresh("hc-not-ready");
        put(&dir, vec![user_comment("u1", "x")]);
        for silent in [
            terminal("t7", ROOT, "live", None, false),
            terminal("t7", ROOT, "live", None, true),
            terminal("t7", ROOT, "live", Some("idle"), false),
        ] {
            let hc = Hc::new(&[("t7", silent)]);
            let got = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc);
            assert!(
                matches!(got, Err(SendError::Refused(Refusal::NotReady, _))),
                "{got:?}"
            );
            assert!(hc.sends().is_empty());
        }
        assert_eq!(unsent(&dir), ["u1"]);
        assert_eq!(hc_target_of(&dir), None);
    }

    #[test]
    fn a_blocked_terminal_is_refused_before_anything_is_typed() {
        let dir = fresh("hc-blocked");
        put(&dir, vec![user_comment("u1", "x")]);
        let hc = Hc::new(&[("t7", live("t7", ROOT, "blocked"))]);
        let got = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc);
        assert!(
            matches!(&got, Err(SendError::Refused(Refusal::Blocked, why))
                if why.contains("waiting on a prompt") && why.contains("still unsent")),
            "{got:?}"
        );
        assert!(hc.sends().is_empty());
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn a_terminal_that_finished_between_the_list_and_the_get_is_gone() {
        let dir = fresh("hc-finished");
        put(&dir, vec![user_comment("u1", "x")]);
        let hc = Hc::new(&[("t5", terminal("t5", ROOT, "finished", None, true))])
            .listing(&[listed_entry("t5", "claude-code", ROOT, Some("idle"))]);
        let got = run_hc_send(&dir, None, &Scope::Unsent, &hc);
        assert!(
            matches!(got, Err(SendError::Refused(Refusal::AgentGone, _))),
            "{got:?}"
        );
        assert!(hc.sends().is_empty());
    }

    #[test]
    fn hc_send_refusals_after_the_get_map_to_refusals_and_mark_nothing() {
        let dir = fresh("hc-send-refused");
        put(&dir, vec![user_comment("u1", "x")]);
        let cases = [
            (9, Refusal::Blocked),
            (11, Refusal::AgentGone),
            (3, Refusal::AgentGone),
        ];
        for (exit, want) in cases {
            let mut hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
            hc.reply = Err(failure(exit, "x"));
            let got = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc);
            assert!(
                matches!(&got, Err(SendError::Refused(why, _)) if *why == want),
                "{exit}: {got:?}"
            );
            assert_eq!(hc.sends().len(), 1, "no retry");
        }
        assert_eq!(unsent(&dir), ["u1"]);
        assert_eq!(hc_target_of(&dir), None);
    }

    #[test]
    fn any_other_hc_send_failure_is_an_error_with_its_message_and_is_not_retried() {
        let dir = fresh("hc-send-fails");
        put(&dir, vec![user_comment("u1", "x")]);
        let mut hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        hc.reply = Err(failure(1, "io"));
        let got = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc);
        assert!(
            matches!(&got, Err(SendError::Failed(why))
                if why.contains("io happened") && why.contains("still unsent")),
            "{got:?}"
        );
        assert_eq!(hc.sends().len(), 1);
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn a_send_under_hc_types_one_prompt_marks_the_comments_and_saves_the_address() {
        let dir = fresh("hc-sent");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        let sent = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Sent {
                n: 2,
                agent: "claude-code".into()
            }
        );
        assert_eq!(
            sent.target,
            Some(Reached::Hc(HcTarget {
                address: address("t7"),
                agent: "claude-code".into()
            }))
        );
        assert_eq!(
            hc.calls().len(),
            2,
            "one get and one send: {:?}",
            hc.calls()
        );
        assert!(hc.sends()[0].starts_with("send --json t7 Address the review comments"));
        assert_eq!(sent_ids(&dir), ["u1", "u2"]);
        assert_eq!(hc_target_of(&dir).unwrap().address, address("t7"));
        assert_eq!(
            meta::load(&dir).0.target,
            None,
            "the herdr key is left alone"
        );
    }

    #[test]
    fn a_working_agent_takes_the_text_as_a_queued_message() {
        let dir = fresh("hc-queued");
        put(&dir, vec![user_comment("u1", "x")]);
        let mut hc = Hc::new(&[("t7", live("t7", ROOT, "working"))]);
        hc.reply = Ok(r#"{"started":true,"status":"working","turn":1}"#.into());
        let sent = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        assert_eq!(
            sent.outcome,
            SendOutcome::Queued {
                n: 1,
                agent: "claude-code".into()
            }
        );
        assert!(!hc.calls().iter().any(|call| call.contains("--if")));
    }

    #[test]
    fn the_prompt_under_hc_names_the_running_binary_and_has_no_escape() {
        let dir = fresh("hc-bin");
        put(
            &dir,
            vec![root(
                &Author::User,
                "u1",
                "src/a.rs",
                None,
                Some((Side::New, 3, None)),
                "see \u{1b}[201~ this",
            )],
        );
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        let call = &hc.sends()[0];
        assert!(
            call.contains("'/opt/bin/herdr-review' comment resolve --repo '/work/repo' <id>"),
            "{call}"
        );
        assert!(
            !call.contains("/plugin")
                && !call.contains('\u{1b}')
                && call.contains("see [201~ this")
        );
    }

    #[test]
    fn without_a_known_binary_the_prompt_says_herdr_review() {
        let dir = fresh("hc-no-exe");
        put(&dir, vec![user_comment("u1", "x")]);
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        send_hc(
            &dir,
            Path::new(ROOT),
            &env(&[]),
            "t",
            &Scope::Unsent,
            Some(&address("t7")),
            |a| hc.ask(a),
        )
        .unwrap();
        assert!(hc.sends()[0].contains("'herdr-review' comment resolve"));
    }

    #[test]
    fn a_sent_event_that_cannot_be_written_still_reports_success_under_hc() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fresh("hc-not-recorded");
        put(&dir, vec![user_comment("u1", "x")]);
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        let log = dir.join("review.jsonl");
        let sent = send_hc(
            &dir,
            Path::new(ROOT),
            &hc_env(),
            "t",
            &Scope::Unsent,
            Some(&address("t7")),
            |args| {
                if args.first().is_some_and(|word| word == "send") {
                    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o400)).unwrap();
                }
                hc.ask(args)
            },
        )
        .unwrap();
        assert_eq!(sent.warnings, [Warning::SentNotRecorded]);
        assert_eq!(unsent(&dir), ["u1"]);
    }

    #[test]
    fn an_address_that_cannot_be_saved_is_a_warning_under_hc() {
        let dir = fresh("hc-not-saved");
        put(&dir, vec![user_comment("u1", "x")]);
        std::fs::create_dir_all(dir.join("meta.json")).unwrap();
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        let sent = run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        assert_eq!(
            sent.warnings,
            [Warning::MetaUnreadable, Warning::TargetNotSaved]
        );
        assert!(unsent(&dir).is_empty());
    }

    #[test]
    fn the_saved_address_is_not_rewritten_when_it_is_already_there() {
        let dir = fresh("hc-same");
        put(&dir, vec![user_comment("u1", "x"), user_comment("u2", "y")]);
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        let before = std::fs::read(dir.join("meta.json")).unwrap();
        put(&dir, vec![user_comment("u3", "z")]);
        std::fs::remove_file(dir.join("meta.json")).unwrap();
        std::fs::write(dir.join("meta.json"), &before).unwrap();
        run_hc_send(&dir, None, &Scope::Unsent, &hc).unwrap();
        assert_eq!(std::fs::read(dir.join("meta.json")).unwrap(), before);
    }

    // `--thread`, under both backends.

    #[test]
    fn named_threads_are_sent_again_when_sent_before_and_no_others_go() {
        let dir = fresh("hc-threads");
        put(
            &dir,
            vec![
                user_comment("u1", "one"),
                user_comment("u2", "two"),
                user_comment("u3", "three"),
            ],
        );
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        run_hc_send(&dir, Some("t7"), &Scope::Unsent, &hc).unwrap();
        put(&dir, vec![user_comment("u4", "four")]);
        let scope = Scope::Threads(vec!["u1".into(), "u3".into()]);
        let again = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        let sent = run_hc_send(&dir, Some("t7"), &scope, &again).unwrap();
        assert_eq!(sent.outcome.to_string(), "sent 2 to claude-code");
        let prompt = &again.sends()[0];
        assert!(prompt.contains("[u1]") && prompt.contains("[u3]"));
        assert!(!prompt.contains("[u2]") && !prompt.contains("[u4]"));
        assert_eq!(unsent(&dir), ["u4"], "the unsent one stays unsent");
    }

    #[test]
    fn a_thread_the_user_resolved_before_sending_is_not_sent_until_it_is_reopened() {
        let dir = fresh("resolved-unsent");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        let id = CommentId::parse("u2").unwrap();
        put(
            &dir,
            vec![event(&Author::User, Kind::Resolve { id: id.clone() })],
        );
        assert_eq!(unsent(&dir), ["u1"]);
        let herdr = agent_ready_in("idle");
        let sent = run_send(&dir, &herdr, false).unwrap();
        assert_eq!(sent.outcome.to_string(), "sent 1 to claude");
        let prompt = &prompts(&herdr)[0];
        assert!(prompt.contains("[u1]") && !prompt.contains("[u2]"));
        put(&dir, vec![event(&Author::User, Kind::Reopen { id })]);
        assert_eq!(unsent(&dir), ["u2"]);
    }

    #[test]
    fn a_named_id_that_is_not_an_open_thread_is_an_unknown_id_under_both_backends() {
        let dir = fresh("threads-unknown");
        put(
            &dir,
            vec![user_comment("u1", "one"), user_comment("u2", "two")],
        );
        put(&dir, agent_answers("u2", "a1"));
        let scope = Scope::Threads(vec!["u1".into(), "u2".into(), "u9".into()]);
        let hc = Hc::new(&[("t7", live("t7", ROOT, "idle"))]);
        let herdr = agent_ready_in("idle");
        let want = |got: Result<Sent, SendError>| {
            let Err(SendError::Command(CommandError::UnknownId { id, open })) = got else {
                panic!("{got:?}");
            };
            assert_eq!(id, "u2", "a resolved thread is not open");
            assert_eq!(open, [CommentId::parse("u1").unwrap()]);
        };
        want(run_hc_send(&dir, Some("t7"), &scope, &hc));
        want(send(
            &dir,
            Path::new(ROOT),
            &env(&PLUGIN_ENV),
            "t",
            &scope,
            |a| herdr.ask(a),
        ));
        assert!(hc.calls().is_empty() && herdr.calls().is_empty());
        assert_eq!(unsent(&dir), ["u1"], "nothing was marked");
    }

    // `send` as a command: the backend, the output, `--json`.

    struct Work {
        env: Env,
        root: PathBuf,
        dir: PathBuf,
    }

    fn work(name: &str, vars: &[(&str, &str)]) -> Work {
        let home = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "herdr-review-send-cmd-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home_text = home.display().to_string();
        let mut all = vec![("HOME", home_text.as_str())];
        all.extend_from_slice(vars);
        let env = Env::new(
            all.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            "/work".into(),
        )
        .with_exe("/opt/bin/herdr-review".into());
        let dir = crate::store::state_dir(&env.state_base().unwrap(), &root);
        Work { env, root, dir }
    }

    impl Work {
        fn run(&self, args: &SendArgs, herdr: &Herdr, hc: &Hc) -> Output {
            let root = self.root.clone();
            command(
                args,
                &self.env,
                "t",
                |_| Ok(format!("{}\n", root.display())),
                |a| herdr.ask(a),
                |a| hc.ask(a),
            )
        }

        fn gets(&self, address: &'static str, status: &str) -> Hc {
            Hc::new(&[(
                address,
                live(address, &self.root.display().to_string(), status),
            )])
        }
    }

    fn json_of(output: &Output) -> Value {
        assert_eq!(output.stderr, "");
        assert!(output.stdout.ends_with('\n') && output.stdout.matches('\n').count() == 1);
        serde_json::from_str(&output.stdout).unwrap()
    }

    fn args(line: &str) -> SendArgs {
        match crate::cli::parse(
            &line
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        ) {
            Ok(crate::cli::Command::Send(args)) => args,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_backend_is_the_flag_else_hc_for_a_target_or_a_terminal_id_else_herdr() {
        let plain = env(&[]);
        let in_hc = env(&[("HC_TERMINAL_ID", "t1")]);
        let choose = |line: &str, env: &Env| backend_of(&args(line), env);
        assert_eq!(choose("send", &plain), Backend::Herdr);
        assert_eq!(choose("send --target t7", &plain), Backend::Hc);
        assert_eq!(choose("send", &in_hc), Backend::Hc);
        assert_eq!(choose("send --backend hc", &plain), Backend::Hc);
        assert_eq!(choose("send --backend herdr", &in_hc), Backend::Herdr);
        let herdr_inside_hc = env(&[("HC_TERMINAL_ID", "t1"), ("HERDR_PANE_ID", "w1:p2")]);
        assert_eq!(choose("send", &herdr_inside_hc), Backend::Herdr);
    }

    #[test]
    fn a_send_under_hc_calls_no_herdr_sends_no_notification_and_prints_the_outcome() {
        let work = work("hc-plain", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let herdr = Herdr::new(&[], &[]);
        let hc = work.gets("t7", "idle");
        let output = work.run(&args("send --target t7"), &herdr, &hc);
        assert_eq!(
            (output.code, output.stdout.as_str(), output.stderr.as_str()),
            (0, "sent 1 to claude-code\n", "")
        );
        assert!(herdr.calls().is_empty());
        assert_eq!(sent_ids(&work.dir), ["u1"]);
    }

    #[test]
    fn a_refusal_under_hc_without_json_is_a_line_on_stderr_and_exit_1() {
        let work = work("hc-refused", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let output = work.run(
            &args("send --target t7"),
            &Herdr::new(&[], &[]),
            &work.gets("t7", "blocked"),
        );
        assert_eq!(output.code, 1);
        assert_eq!(output.stdout, "");
        assert!(
            output
                .stderr
                .starts_with("The agent is waiting on a prompt.")
        );
    }

    #[test]
    fn json_of_a_send_that_happened_has_the_agent_the_target_and_the_warnings() {
        let work = work("json-sent", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let output = work.run(
            &args("send --target t7 --json"),
            &Herdr::new(&[], &[]),
            &work.gets("t7", "idle"),
        );
        assert_eq!(output.code, 0);
        assert_eq!(
            json_of(&output),
            serde_json::json!({
                "schema": 1, "sent": 1, "queued": false, "agent": "claude-code",
                "target": { "backend": "hc", "address": "t7", "agent": "claude-code" },
                "warnings": []
            })
        );
    }

    #[test]
    fn json_of_a_queued_send_and_of_nothing_to_send() {
        let work = work("json-queued", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let mut hc = work.gets("t7", "working");
        hc.reply = Ok(r#"{"started":true,"status":"working","turn":1}"#.into());
        let queued =
            json_of(&work.run(&args("send --target t7 --json"), &Herdr::new(&[], &[]), &hc));
        assert_eq!(
            (&queued["sent"], &queued["queued"]),
            (&Value::from(1), &Value::from(true))
        );
        let nothing = work.run(&args("send --target t7 --json"), &Herdr::new(&[], &[]), &hc);
        assert_eq!(nothing.code, 0);
        assert_eq!(
            json_of(&nothing),
            serde_json::json!({
                "schema": 1, "sent": 0, "queued": false, "agent": null, "target": null, "warnings": []
            })
        );
    }

    #[test]
    fn json_of_an_ambiguous_send_lists_the_candidates_and_exits_1() {
        let work = work("json-ambiguous", &[("HC_TERMINAL_ID", "t1")]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let cwd = work.root.display().to_string();
        let hc = Hc::new(&[]).listing(&[
            listed_entry("t5", "claude-code", &cwd, Some("idle")),
            listed_entry("t6", "codex", &cwd, Some("working")),
        ]);
        let output = work.run(&args("send --json"), &Herdr::new(&[], &[]), &hc);
        assert_eq!(output.code, 1);
        let json = json_of(&output);
        assert_eq!(json["error"], "ambiguous");
        assert_eq!(
            json["candidates"],
            serde_json::json!([
                { "address": "t5", "agent": "claude-code", "cwd": cwd, "status": "idle" },
                { "address": "t6", "agent": "codex", "cwd": cwd, "status": "working" }
            ])
        );
        assert!(json["message"].as_str().unwrap().contains("2 agents match"));
        assert_eq!(unsent(&work.dir), ["u1"]);
    }

    #[test]
    fn json_names_the_kind_of_every_refusal_and_target_error_and_exits_1() {
        let work = work("json-kinds", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let cwd = work.root.display().to_string();
        let silent = terminal("t7", &cwd, "live", None, false);
        let finished = terminal("t7", &cwd, "finished", None, true);
        let mut exit9 = work.gets("t7", "idle");
        exit9.reply = Err(failure(9, "blocked"));
        let mut down = Hc::new(&[]);
        down.list = Err(failure(1, "core_down"));
        let mut io = work.gets("t7", "idle");
        io.reply = Err(failure(1, "io"));
        let cases = [
            (
                "blocked",
                work.gets("t7", "blocked"),
                "send --target t7 --json",
            ),
            ("blocked", exit9, "send --target t7 --json"),
            (
                "not_ready",
                Hc::new(&[("t7", silent)]),
                "send --target t7 --json",
            ),
            (
                "agent_gone",
                Hc::new(&[("t7", finished)]).listing(&[listed_entry(
                    "t7",
                    "claude-code",
                    &cwd,
                    Some("idle"),
                )]),
                "send --target t7 --json",
            ),
            ("no_agent", Hc::new(&[]), "send --backend hc --json"),
            ("backend", down, "send --backend hc --json"),
            ("backend", io, "send --target t7 --json"),
        ];
        for (kind, hc, line) in cases {
            let output = work.run(&args(line), &Herdr::new(&[], &[]), &hc);
            assert_eq!(output.code, 1, "{kind}");
            let json = json_of(&output);
            assert_eq!(
                (json["schema"].as_u64(), json["error"].as_str()),
                (Some(1), Some(kind)),
                "{json}"
            );
            assert_eq!(json["candidates"], serde_json::json!([]));
            assert!(!json["message"].as_str().unwrap().is_empty());
        }
        assert_eq!(unsent(&work.dir), ["u1"]);
    }

    #[test]
    fn a_busy_store_and_a_missing_repository_are_busy_and_backend_in_json() {
        let busy = error_json(&SendError::Store(StoreError::Busy));
        assert_eq!(busy.error, SendErrorKind::Busy);
        let work = work("json-no-repo", &[]);
        let output = command(
            &args("send --backend hc --json"),
            &work.env,
            "t",
            |_| Err(GitError::NotARepo),
            |_| panic!("asked herdr"),
            |_| panic!("asked hc"),
        );
        assert_eq!(
            (
                output.code,
                json_of(&output)["error"].as_str().map(str::to_owned)
            ),
            (1, Some("backend".into()))
        );
    }

    #[test]
    fn an_unknown_thread_is_exit_2_with_the_open_ids_on_stderr_under_both_backends() {
        for line in [
            "send --thread u9 --json",
            "send --thread u9 --target t7 --json",
        ] {
            let work = work("unknown-thread", &[]);
            put(&work.dir, vec![user_comment("u1", "x")]);
            let output = work.run(&args(line), &Herdr::new(&[], &[]), &work.gets("t7", "idle"));
            assert_eq!(output.code, 2, "{line}");
            assert_eq!(output.stdout, "");
            assert!(
                output.stderr.contains("u9") && output.stderr.contains("u1"),
                "{}",
                output.stderr
            );
        }
    }

    #[test]
    fn json_under_herdr_has_the_kinds_and_a_herdr_target() {
        let work = work("json-herdr", &[("HERDR_PLUGIN_ROOT", "/plugin")]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let cwd = work.root.display().to_string();
        let blocked = agent_json("w1:p1", "term_1", &cwd, "blocked", "w1");
        let herdr = Herdr::new(&[("w1:p1", blocked.clone())], &[blocked]);
        let hc = Hc::new(&[]);
        let refused = work.run(&args("send --json"), &herdr, &hc);
        assert_eq!(refused.code, 1);
        assert_eq!(json_of(&refused)["error"], "blocked");
        assert!(hc.calls().is_empty(), "herdr backend never calls hc");
        let two = Herdr::new(
            &[],
            &[
                agent_json("w1:p1", "term_1", &cwd, "idle", "w1"),
                agent_json("w1:p2", "term_2", &cwd, "idle", "w1"),
            ],
        );
        let ambiguous = json_of(&work.run(&args("send --json"), &two, &hc));
        assert_eq!(ambiguous["error"], "ambiguous");
        assert_eq!(
            ambiguous["candidates"],
            serde_json::json!([]),
            "the picker is the TUI's"
        );
        let idle = agent_json("w1:p1", "term_1", &cwd, "idle", "w1");
        let ok = Herdr::new(&[("w1:p1", idle.clone())], &[idle]);
        let sent = json_of(&work.run(&args("send --json"), &ok, &hc));
        assert_eq!(
            sent["target"],
            serde_json::json!({ "backend": "herdr", "pane_id": "w1:p1", "terminal_id": "term_1", "agent": "claude" })
        );
        assert!(
            ok.calls()
                .iter()
                .any(|c| c.starts_with("agent prompt w1:p1 "))
        );
    }

    #[test]
    fn a_failed_herdr_prompt_is_backend_in_json_and_herdr_refusals_keep_their_kind() {
        let work = work("json-herdr-failed", &[]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let cwd = work.root.display().to_string();
        let idle = agent_json("w1:p1", "term_1", &cwd, "idle", "w1");
        let mut herdr = Herdr::new(&[("w1:p1", idle.clone())], &[idle]);
        herdr.prompt_fails = true;
        let output = work.run(&args("send --json"), &herdr, &Hc::new(&[]));
        assert_eq!(json_of(&output)["error"], "backend");
        assert_eq!(unsent(&work.dir), ["u1"]);
        for (words, kind) in [
            (
                "The agent is not ready for input yet. Nothing was sent.",
                Refusal::NotReady,
            ),
            (
                "No agent is running in that pane. Nothing was sent.",
                Refusal::AgentGone,
            ),
            (
                "That pane no longer exists. Nothing was sent.",
                Refusal::AgentGone,
            ),
        ] {
            assert!(
                matches!(SendError::from_herdr(words.into()), SendError::Refused(got, _) if got == kind)
            );
        }
    }

    #[test]
    fn under_herdr_the_command_still_reports_through_notifications() {
        let work = work("herdr-notify", &[("HERDR_PLUGIN_ROOT", "/plugin")]);
        put(&work.dir, vec![user_comment("u1", "x")]);
        let cwd = work.root.display().to_string();
        let idle = agent_json("w1:p1", "term_1", &cwd, "idle", "w1");
        let herdr = Herdr::new(&[("w1:p1", idle.clone())], &[idle]);
        let output = work.run(&args("send"), &herdr, &Hc::new(&[]));
        assert_eq!((output.code, output.stdout.as_str()), (0, ""));
        assert!(
            herdr
                .calls()
                .contains(&"notification show review: sent 1 to claude".to_owned())
        );
    }
}
