//! The command line: arguments into a `Command`, a `Command` into output and an exit code.
//!
//! Every error is matched once, in `Failure::output` (design/errors.md).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::actions::{delete, edit, seen};
use crate::agent::author;
use crate::apply::{ApplyError, append, decode, read_lines};
use crate::comment::{
    AuthorFilter, CommandError, Filter, MAX_BATCH_FILE, MAX_BODY, StatusFilter, list,
    read_user_file, render_json, render_text, reopen, reply, resolve,
};
use crate::diff::{GitError, RepoRoot};
use crate::env::Env;
use crate::hc::HcError;
use crate::meta::{load, locate};
use crate::send::{Backend, SendArgs};
use crate::state::{Query, SpecChoice, choose};
use crate::store::{
    Author, CommentId, Spec, StoreError, TerminalAddress, Warning, WriteError, read,
};

pub const USAGE: &str = "usage:
  herdr-review tui    [--repo <root>]
  herdr-review open   [--repo <root>] [--base <ref>]
  herdr-review send   [--repo <root>] [--all-open] [--backend herdr|hc] [--target <t7>] [--thread <id>]... [--json]
  herdr-review state  [--repo <root>] [--spec worktree|branch] [--base <ref>] --json
  herdr-review file   [--repo <root>] [--spec worktree|branch] [--base <ref>] --path <p> --json
  herdr-review archive [--repo <root>] [--json]
  herdr-review view   [--repo <root>] [--spec worktree|branch] [--base <ref>] --path <p> --set|--unset
  herdr-review message
  herdr-review message-tui
  herdr-review comment apply   [--repo <root>] [--name <agent>] [--spec worktree|branch] [--base <ref>] --stdin
  herdr-review comment apply   --as user [--repo <root>] [--spec ..] [--base ..] --file <f>
  herdr-review comment list    [--repo <root>] [--status open|resolved] [--author user|agent] [--json]
  herdr-review comment reply   [--repo <root>] [--name <agent>] <id> -
  herdr-review comment resolve [--repo <root>] [--name <agent>] <id> --reply -
  herdr-review comment reopen  [--repo <root>] <id>
  herdr-review comment reply   --as user [--repo <root>] <id> --body-file <f>
  herdr-review comment resolve --as user [--repo <root>] <id> [--body-file <f>]
  herdr-review comment reopen  --as user [--repo <root>] <id>
  herdr-review comment edit    [--repo <root>] <id> --body-file <f>
  herdr-review comment delete  [--repo <root>] <id>
  herdr-review comment seen    [--repo <root>] <id>";

/// The arguments are wrong. Exit code 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub String);

/// Where a command reads its text from. A file is the panel's, and is deleted once it is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Stdin,
    File(PathBuf),
}

/// `user` is `--as user`: the comment is the user's, and neither `herdr` nor `hc` is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentAction {
    Apply {
        name: Option<String>,
        user: bool,
        input: Input,
        spec: Option<SpecChoice>,
        base: Option<String>,
    },
    List {
        filter: Filter,
        json: bool,
    },
    Reply {
        name: Option<String>,
        user: bool,
        id: String,
        input: Input,
    },
    Resolve {
        name: Option<String>,
        user: bool,
        id: String,
        input: Option<Input>,
    },
    Reopen {
        user: bool,
        id: String,
    },
    Edit {
        id: String,
        file: PathBuf,
    },
    Delete {
        id: String,
    },
    Seen {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Tui {
        repo: Option<PathBuf>,
    },
    Open {
        repo: Option<PathBuf>,
        base: Option<String>,
    },
    Send(SendArgs),
    Comment {
        repo: Option<PathBuf>,
        action: CommentAction,
    },
    Archive {
        repo: Option<PathBuf>,
        json: bool,
    },
    View {
        query: Query,
        path: String,
        set: bool,
    },
    State {
        query: Query,
    },
    File {
        query: Query,
        path: String,
    },
    Message,
    MessageTui,
}

/// Positional arguments, flags that take a value, and flags that do not.
#[derive(Default)]
struct Parsed {
    positional: Vec<String>,
    values: BTreeMap<String, Vec<String>>,
    switches: BTreeSet<String>,
}

impl Parsed {
    /// The last value given for a flag.
    fn value(&mut self, name: &str) -> Option<String> {
        self.values.remove(name)?.pop()
    }

    /// Every value given for a flag that may repeat, in order.
    fn all(&mut self, name: &str) -> Vec<String> {
        self.values.remove(name).unwrap_or_default()
    }

    fn switch(&mut self, name: &str) -> bool {
        self.switches.remove(name)
    }

    /// The positional arguments, which must number exactly `names.len()`.
    fn positionals<const N: usize>(self, names: [&str; N]) -> Result<[String; N], Usage> {
        let found = self.positional.len();
        <[String; N]>::try_from(self.positional).map_err(|_| {
            Usage(format!(
                "expected {} (<{}>), got {found}",
                N,
                names.join("> <")
            ))
        })
    }
}

/// Split `args` by the flags a command knows. `--flag=value` is the same as `--flag value`, and a
/// lone `-` is a positional argument that means stdin.
fn split(args: &[String], values: &[&str], switches: &[&str]) -> Result<Parsed, Usage> {
    let mut parsed = Parsed::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let Some(flag) = arg.strip_prefix("--") else {
            parsed.positional.push(arg.clone());
            continue;
        };
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(n, v)| (n, Some(v)));
        let name = format!("--{name}");
        if switches.contains(&name.as_str()) && inline.is_none() {
            parsed.switches.insert(name);
        } else if values.contains(&name.as_str()) {
            let value = inline
                .map(str::to_owned)
                .or_else(|| args.next().cloned())
                .ok_or_else(|| Usage(format!("{name} needs a value")))?;
            parsed.values.entry(name).or_default().push(value);
        } else {
            return Err(Usage(format!("unknown option {name}")));
        }
    }
    Ok(parsed)
}

fn one_of<T: Copy>(
    flag: &str,
    text: Option<String>,
    choices: &[(&str, T)],
) -> Result<Option<T>, Usage> {
    let Some(text) = text else { return Ok(None) };
    let found = choices
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, value)| *value);
    found.map(Some).ok_or_else(|| {
        let names = choices.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        Usage(format!("{flag} takes {}, not '{text}'", names.join(" or ")))
    })
}

fn no_arguments(parsed: Parsed) -> Result<(), Usage> {
    parsed.positionals([]).map(|[]| ())
}

const NEEDS_USER: &str = "--file and --body-file go with --as user, or with comment edit";

fn parse_comment(args: &[String]) -> Result<Command, Usage> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(Usage(
            "comment needs apply, list, reply, resolve, reopen, edit, delete or seen".into(),
        ));
    };
    let mut parsed = match sub.as_str() {
        "apply" => split(
            rest,
            &["--repo", "--name", "--spec", "--base", "--as", "--file"],
            &["--stdin"],
        )?,
        "list" => split(rest, &["--repo", "--status", "--author"], &["--json"])?,
        "reply" => split(rest, &["--repo", "--name", "--as", "--body-file"], &[])?,
        "resolve" => split(
            rest,
            &["--repo", "--name", "--reply", "--as", "--body-file"],
            &[],
        )?,
        "reopen" => split(rest, &["--repo", "--as"], &[])?,
        "edit" => split(rest, &["--repo", "--body-file"], &[])?,
        "delete" | "seen" => split(rest, &["--repo"], &[])?,
        other => return Err(Usage(format!("unknown comment command '{other}'"))),
    };
    let repo = parsed.value("--repo").map(PathBuf::from);
    let name = parsed.value("--name").filter(|name| !name.is_empty());
    let user = one_of("--as", parsed.value("--as"), &[("user", ())])?.is_some();
    if user && name.is_some() {
        return Err(Usage("--name is for an agent, not with --as user".into()));
    }
    let file = parsed.value("--body-file").map(PathBuf::from);
    if file.is_some() && !user && sub != "edit" {
        return Err(Usage(NEEDS_USER.into()));
    }
    let action = match sub.as_str() {
        "apply" => {
            let stdin = parsed.switch("--stdin");
            let batch = parsed.value("--file").map(PathBuf::from);
            let input = match (user, stdin, batch) {
                (_, false, None) if !user => {
                    return Err(Usage(
                        "comment apply reads the batch from stdin, pass --stdin".into(),
                    ));
                }
                (false, _, Some(_)) => return Err(Usage(NEEDS_USER.into())),
                (_, true, Some(_)) => {
                    return Err(Usage("give --stdin or --file, not both".into()));
                }
                (_, true, None) => Input::Stdin,
                (_, false, Some(batch)) => Input::File(batch),
                (_, false, None) => {
                    return Err(Usage(
                        "comment apply --as user needs --file <f> or --stdin".into(),
                    ));
                }
            };
            let spec = one_of(
                "--spec",
                parsed.value("--spec"),
                &[
                    ("worktree", SpecChoice::WorkTree),
                    ("branch", SpecChoice::Branch),
                ],
            )?;
            let base = parsed.value("--base");
            no_arguments(parsed)?;
            CommentAction::Apply {
                name,
                user,
                input,
                spec,
                base,
            }
        }
        "list" => {
            let status = one_of(
                "--status",
                parsed.value("--status"),
                &[
                    ("open", StatusFilter::Open),
                    ("resolved", StatusFilter::Resolved),
                ],
            )?;
            let author = one_of(
                "--author",
                parsed.value("--author"),
                &[("user", AuthorFilter::User), ("agent", AuthorFilter::Agent)],
            )?;
            let json = parsed.switch("--json");
            no_arguments(parsed)?;
            CommentAction::List {
                filter: Filter { status, author },
                json,
            }
        }
        "reply" => {
            let (id, input) = if let Some(file) = file {
                let [id] = parsed.positionals(["id"])?;
                (id, Input::File(file))
            } else {
                let [id, dash] = parsed.positionals(["id", "-"])?;
                require_stdin_dash("reply", &dash)?;
                (id, Input::Stdin)
            };
            CommentAction::Reply {
                name,
                user,
                id,
                input,
            }
        }
        "resolve" => {
            let input = match (parsed.value("--reply"), file) {
                (Some(_), Some(_)) => {
                    return Err(Usage("give --reply - or --body-file, not both".into()));
                }
                (Some(dash), None) => {
                    require_stdin_dash("--reply", &dash)?;
                    Some(Input::Stdin)
                }
                (None, Some(file)) => Some(Input::File(file)),
                (None, None) if user => None,
                (None, None) => {
                    return Err(Usage(
                        "comment resolve needs --reply - (the reply text on stdin)".into(),
                    ));
                }
            };
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Resolve {
                name,
                user,
                id,
                input,
            }
        }
        "reopen" => {
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Reopen { user, id }
        }
        "edit" => {
            let file = file
                .ok_or_else(|| Usage("comment edit needs --body-file <f> (the new text)".into()))?;
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Edit { id, file }
        }
        "delete" => {
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Delete { id }
        }
        _ => {
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Seen { id }
        }
    };
    Ok(Command::Comment { repo, action })
}

/// Text only arrives on stdin, so no shell substitution in it can run (design/cli.md).
fn require_stdin_dash(what: &str, text: &str) -> Result<(), Usage> {
    if text == "-" {
        Ok(())
    } else {
        Err(Usage(format!(
            "{what} takes the text from stdin, write - and pipe the text in"
        )))
    }
}

/// The flags `state` and `file` share, and `--path` for `file`. Both print JSON and nothing else, so
/// `--json` is required.
fn parse_read(args: &[String], with_path: bool) -> Result<(Query, Option<String>), Usage> {
    let values: &[&str] = if with_path {
        &["--repo", "--spec", "--base", "--path"]
    } else {
        &["--repo", "--spec", "--base"]
    };
    let mut parsed = split(args, values, &["--json"])?;
    if !parsed.switch("--json") {
        return Err(Usage("this command prints JSON only, pass --json".into()));
    }
    let query = query_of(&mut parsed)?;
    let path = parsed.value("--path");
    no_arguments(parsed)?;
    Ok((query, path))
}

/// `--repo`, `--spec` and `--base`.
fn query_of(parsed: &mut Parsed) -> Result<Query, Usage> {
    Ok(Query {
        repo: parsed.value("--repo").map(PathBuf::from),
        spec: one_of(
            "--spec",
            parsed.value("--spec"),
            &[
                ("worktree", SpecChoice::WorkTree),
                ("branch", SpecChoice::Branch),
            ],
        )?,
        base: parsed.value("--base"),
    })
}

/// `view --path <p> --set|--unset`.
fn parse_view(args: &[String]) -> Result<Command, Usage> {
    let mut parsed = split(
        args,
        &["--repo", "--spec", "--base", "--path"],
        &["--set", "--unset"],
    )?;
    let query = query_of(&mut parsed)?;
    let path = parsed
        .value("--path")
        .ok_or_else(|| Usage("view needs --path <p>".into()))?;
    let set = match (parsed.switch("--set"), parsed.switch("--unset")) {
        (true, false) => true,
        (false, true) => false,
        _ => return Err(Usage("view needs --set or --unset, one of them".into())),
    };
    no_arguments(parsed)?;
    Ok(Command::View { query, path, set })
}

/// `send` and its flags. `--target` is an hc address and implies hc, so it cannot go with
/// `--backend herdr`, and `--thread` replaces `--all-open`.
fn parse_send(args: &[String]) -> Result<Command, Usage> {
    let mut parsed = split(
        args,
        &["--repo", "--backend", "--target", "--thread"],
        &["--all-open", "--json"],
    )?;
    let backend = one_of(
        "--backend",
        parsed.value("--backend"),
        &[("herdr", Backend::Herdr), ("hc", Backend::Hc)],
    )?;
    let target = parsed
        .value("--target")
        .map(|text| {
            TerminalAddress::parse(&text).ok_or_else(|| {
                Usage(format!(
                    "--target takes an hc address such as t7, not '{text}'"
                ))
            })
        })
        .transpose()?;
    if target.is_some() && backend == Some(Backend::Herdr) {
        return Err(Usage(
            "--target is an hc address, not with --backend herdr".into(),
        ));
    }
    let send = SendArgs {
        repo: parsed.value("--repo").map(PathBuf::from),
        all_open: parsed.switch("--all-open"),
        backend,
        target,
        threads: parsed.all("--thread"),
        json: parsed.switch("--json"),
    };
    if send.all_open && !send.threads.is_empty() {
        return Err(Usage("give --thread or --all-open, not both".into()));
    }
    no_arguments(parsed)?;
    Ok(Command::Send(send))
}

pub fn parse(args: &[String]) -> Result<Command, Usage> {
    let Some((command, rest)) = args.split_first() else {
        return Err(Usage("a command is needed".into()));
    };
    match command.as_str() {
        "tui" => {
            let mut parsed = split(rest, &["--repo"], &[])?;
            let repo = parsed.value("--repo").map(PathBuf::from);
            no_arguments(parsed)?;
            Ok(Command::Tui { repo })
        }
        "open" => {
            let mut parsed = split(rest, &["--repo", "--base"], &[])?;
            let (repo, base) = (
                parsed.value("--repo").map(PathBuf::from),
                parsed.value("--base"),
            );
            no_arguments(parsed)?;
            Ok(Command::Open { repo, base })
        }
        "send" => parse_send(rest),
        "message" => {
            no_arguments(split(rest, &[], &[])?)?;
            Ok(Command::Message)
        }
        "message-tui" => {
            no_arguments(split(rest, &[], &[])?)?;
            Ok(Command::MessageTui)
        }
        "state" => {
            let (query, _) = parse_read(rest, false)?;
            Ok(Command::State { query })
        }
        "file" => {
            let (query, path) = parse_read(rest, true)?;
            let path = path.ok_or_else(|| Usage("file needs --path <p>".into()))?;
            Ok(Command::File { query, path })
        }
        "archive" => {
            let mut parsed = split(rest, &["--repo"], &["--json"])?;
            let (repo, json) = (
                parsed.value("--repo").map(PathBuf::from),
                parsed.switch("--json"),
            );
            no_arguments(parsed)?;
            Ok(Command::Archive { repo, json })
        }
        "view" => parse_view(rest),
        "comment" => parse_comment(rest),
        other => Err(Usage(format!("unknown command '{other}'"))),
    }
}

/// What a command prints and the exit code it ends with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}

/// Everything that can end a `comment` command with a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Usage(Usage),
    Git(GitError),
    Store(StoreError),
    Command(CommandError),
    /// `file --path` named a path the diff does not have.
    NotInDiff(String),
    NoStateDir,
}

impl Failure {
    /// One line on stderr. Exit 2 when the request is wrong, 1 when trying again later may work.
    pub fn output(&self) -> Output {
        let (message, code) = match self {
            Self::Usage(Usage(message)) => (format!("{message}\n{USAGE}"), 2),
            Self::Git(error) => (error.to_string(), 1),
            Self::Store(error) => (error.to_string(), 1),
            Self::Command(error) => (error.to_string(), 2),
            Self::NotInDiff(path) => (format!("not in diff: {path}"), 2),
            Self::NoStateDir => (
                "cannot find a state directory, set HOME or XDG_STATE_HOME".to_owned(),
                1,
            ),
        };
        Output {
            stderr: format!("{message}\n"),
            code,
            ..Output::default()
        }
    }
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<Usage> for Failure {
    fn from(usage: Usage) -> Self {
        Self::Usage(usage)
    }
}

impl From<GitError> for Failure {
    fn from(error: GitError) -> Self {
        Self::Git(error)
    }
}

impl From<WriteError<CommandError>> for Failure {
    fn from(error: WriteError<CommandError>) -> Self {
        match error {
            WriteError::Store(error) => Self::Store(error),
            WriteError::Build(error) => Self::Command(error),
        }
    }
}

impl From<ApplyError> for Failure {
    fn from(error: ApplyError) -> Self {
        match error {
            ApplyError::Invalid(error) => Self::Command(error),
            ApplyError::Git(error) => Self::Git(error),
        }
    }
}

impl From<CommandError> for Failure {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

impl From<StoreError> for Failure {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

fn warnings(warning: Option<Warning>) -> String {
    warning
        .map(|warning| format!("warning: {warning}\n"))
        .unwrap_or_default()
}

/// The text a command reads from stdin. It must be UTF-8.
fn read_text(stdin: &mut impl Read) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    stdin.read_to_end(&mut bytes).map_err(|error| {
        Failure::Command(CommandError::InvalidBody(format!(
            "cannot read stdin: {error}"
        )))
    })?;
    String::from_utf8(bytes)
        .map_err(|_| Failure::Command(CommandError::InvalidBody("stdin is not valid UTF-8".into())))
}

/// `herdr notification show "N review comments from <name>"`. A failure is ignored.
fn notify(count: usize, by: &Author, mut herdr: impl FnMut(&[String]) -> Result<String, String>) {
    let from = match by {
        Author::Agent(Some(name)) => name.as_str(),
        _ => "an agent",
    };
    let noun = if count == 1 { "comment" } else { "comments" };
    let title = format!("{count} review {noun} from {from}");
    let _ = herdr(&["notification".to_owned(), "show".to_owned(), title]);
}

fn comment(
    repo: Option<&Path>,
    action: &CommentAction,
    env: &Env,
    now: &str,
    mut git: impl FnMut(&[String]) -> Result<String, GitError>,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
    mut hc: impl FnMut(&[String]) -> Result<String, HcError>,
    mut stdin: impl Read,
) -> Result<Output, Failure> {
    let root = RepoRoot::resolve(repo, &env.cwd, &mut git)?;
    let base = env.state_base().ok_or(Failure::NoStateDir)?;
    let dir = locate(&base, root.path());
    let printed = |ids: &[CommentId]| Output {
        stdout: ids.iter().fold(String::new(), |mut out, id| {
            let _ = writeln!(out, "{id}");
            out
        }),
        ..Output::default()
    };
    match action {
        CommentAction::List { filter, json } => {
            let (listing, warning) = list(&dir, *filter)?;
            let stdout = if *json {
                format!("{}\n", render_json(&listing))
            } else {
                render_text(&listing)
            };
            Ok(Output {
                stdout,
                stderr: warnings(warning),
                code: 0,
            })
        }
        CommentAction::Reply {
            name,
            user,
            id,
            input,
        } => {
            let text = text_of(input, MAX_BODY, &mut stdin)?;
            let by = by(*user, name.as_deref(), env, &mut herdr, &mut hc);
            Ok(printed(&[reply(&dir, now, &by, id, &text)?]))
        }
        CommentAction::Resolve {
            name,
            user,
            id,
            input,
        } => {
            let text = input
                .as_ref()
                .map(|input| text_of(input, MAX_BODY, &mut stdin))
                .transpose()?;
            let by = by(*user, name.as_deref(), env, &mut herdr, &mut hc);
            Ok(printed(
                &resolve(&dir, now, &by, id, text.as_deref())?
                    .into_iter()
                    .collect::<Vec<_>>(),
            ))
        }
        CommentAction::Reopen { user, id } => {
            let by = by(*user, None, env, &mut herdr, &mut hc);
            reopen(&dir, now, &by, id)?;
            Ok(printed(&[]))
        }
        CommentAction::Apply {
            name,
            user,
            input,
            spec,
            base,
        } => {
            let comments = decode(&text_of(input, MAX_BATCH_FILE, &mut stdin)?)?;
            let (meta, warning) = load(&dir);
            let (spec, notice) = if spec.is_none() && base.is_none() {
                (meta.spec.clone().unwrap_or(Spec::WorkTree), None)
            } else {
                let query = Query {
                    repo: None,
                    spec: *spec,
                    base: base.clone(),
                };
                let mut bytes = |args: &[String]| git(args).map(String::into_bytes);
                let (spec, _, notice) = choose(&query, &meta, root.path(), &mut bytes)?;
                (spec, notice)
            };
            let ready = read_lines(comments, root.path(), &spec, &mut git)?;
            let by = by(*user, name.as_deref(), env, &mut herdr, &mut hc);
            let ids = append(&dir, now, &by, &spec, ready)?;
            if !user && env.hc_terminal().is_none() {
                notify(ids.len(), &by, &mut herdr);
            }
            Ok(Output {
                stderr: format!(
                    "{}{}",
                    warnings(warning),
                    notice
                        .map(|text| format!("notice: {text}\n"))
                        .unwrap_or_default()
                ),
                ..printed(&ids)
            })
        }
        CommentAction::Edit { id, file } => {
            let body = read_user_file(file, MAX_BODY)?;
            edit(&dir, now, &user_id(&dir, id)?, &body)?;
            Ok(printed(&[]))
        }
        CommentAction::Delete { id } => {
            delete(&dir, now, &user_id(&dir, id)?)?;
            Ok(printed(&[]))
        }
        CommentAction::Seen { id } => {
            seen(&dir, now, &user_id(&dir, id)?)?;
            Ok(printed(&[]))
        }
    }
}

/// The author of a write: the user under `--as user`, with no `herdr` or `hc` call, else the agent.
fn by(
    user: bool,
    name: Option<&str>,
    env: &Env,
    herdr: impl FnMut(&[String]) -> Result<String, String>,
    hc: impl FnMut(&[String]) -> Result<String, HcError>,
) -> Author {
    if user {
        Author::User
    } else {
        author(name, env, herdr, hc)
    }
}

/// The text of a command, from stdin or from the panel's file.
fn text_of(input: &Input, limit: usize, stdin: &mut impl Read) -> Result<String, Failure> {
    match input {
        Input::Stdin => read_text(stdin),
        Input::File(path) => Ok(read_user_file(path, limit)?),
    }
}

/// An id from the command line. Text that no comment could have as its id is an unknown id like
/// any other, and the error lists the open threads.
fn user_id(dir: &Path, text: &str) -> Result<CommentId, Failure> {
    CommentId::parse(text).ok_or_else(|| {
        read(dir).map_or_else(Failure::Store, |review| {
            Failure::Command(CommandError::UnknownId {
                id: text.to_owned(),
                open: review.open_ids().into_iter().cloned().collect(),
            })
        })
    })
}

/// Run a `comment` command. `git`, `herdr` and `hc` answer every call to those tools, and `stdin`
/// holds the text or the batch.
pub fn run_comment(
    repo: Option<&Path>,
    action: &CommentAction,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
    herdr: impl FnMut(&[String]) -> Result<String, String>,
    hc: impl FnMut(&[String]) -> Result<String, HcError>,
    stdin: impl Read,
) -> Output {
    comment(repo, action, env, now, git, herdr, hc, stdin)
        .unwrap_or_else(|failure| failure.output())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    fn comment_action(line: &str) -> CommentAction {
        match parse(&args(line)) {
            Ok(Command::Comment { action, .. }) => action,
            other => panic!("{other:?}"),
        }
    }

    fn usage(line: &str) -> String {
        parse(&args(line)).unwrap_err().0
    }

    #[test]
    fn every_command_in_the_plan_parses() {
        assert_eq!(
            parse(&args("tui --repo /r")),
            Ok(Command::Tui {
                repo: Some("/r".into())
            })
        );
        assert_eq!(
            parse(&args("open --base=main")),
            Ok(Command::Open {
                repo: None,
                base: Some("main".into())
            })
        );
        assert_eq!(
            parse(&args("send --all-open --repo /r")),
            Ok(Command::Send(SendArgs {
                repo: Some("/r".into()),
                all_open: true,
                ..SendArgs::default()
            }))
        );
        assert_eq!(parse(&args("send")), Ok(Command::Send(SendArgs::default())));
        assert!(parse(&args("send now")).is_err());
        assert_eq!(
            parse(&args(
                "send --backend hc --target t7 --thread u1 --thread=u3 --json"
            )),
            Ok(Command::Send(SendArgs {
                backend: Some(Backend::Hc),
                target: TerminalAddress::parse("t7"),
                threads: vec!["u1".into(), "u3".into()],
                json: true,
                ..SendArgs::default()
            }))
        );
        let query = Query {
            repo: Some("/r".into()),
            spec: Some(SpecChoice::Branch),
            base: Some("main".into()),
        };
        assert_eq!(
            parse(&args("state --json --repo /r --spec branch --base main")),
            Ok(Command::State {
                query: query.clone()
            })
        );
        assert_eq!(
            parse(&args(
                "file --path a.rs --json --repo=/r --spec=branch --base=main"
            )),
            Ok(Command::File {
                query,
                path: "a.rs".into()
            })
        );
        assert_eq!(
            parse(&args("state --json")),
            Ok(Command::State {
                query: Query::default()
            })
        );
        assert_eq!(parse(&args("message")), Ok(Command::Message));
        assert!(parse(&args("message --repo /r")).is_err());
        assert_eq!(parse(&args("message-tui")), Ok(Command::MessageTui));
        assert_eq!(
            comment_action("comment apply --stdin --name codex"),
            CommentAction::Apply {
                name: Some("codex".into()),
                user: false,
                input: Input::Stdin,
                spec: None,
                base: None
            }
        );
        assert_eq!(
            comment_action("comment list --status open --author agent --json"),
            CommentAction::List {
                filter: Filter {
                    status: Some(StatusFilter::Open),
                    author: Some(AuthorFilter::Agent)
                },
                json: true
            }
        );
        assert_eq!(
            comment_action("comment reply u7 -"),
            CommentAction::Reply {
                name: None,
                user: false,
                id: "u7".into(),
                input: Input::Stdin
            }
        );
        assert_eq!(
            comment_action("comment resolve --name x u7 --reply -"),
            CommentAction::Resolve {
                name: Some("x".into()),
                user: false,
                id: "u7".into(),
                input: Some(Input::Stdin)
            }
        );
        assert_eq!(
            comment_action("comment reopen u7"),
            CommentAction::Reopen {
                user: false,
                id: "u7".into()
            }
        );
        let Ok(Command::Comment { repo, .. }) = parse(&args("comment reopen --repo /r u7")) else {
            panic!()
        };
        assert_eq!(repo, Some("/r".into()));
    }

    #[test]
    fn a_wrong_command_line_is_a_usage_error_that_says_what_is_wrong() {
        assert!(usage("").contains("command is needed"));
        assert!(usage("frobnicate").contains("unknown command"));
        assert!(usage("comment").contains("apply, list"));
        assert!(usage("comment nope").contains("unknown comment command"));
        assert!(usage("comment list --bogus").contains("unknown option --bogus"));
        assert!(usage("comment list --status").contains("needs a value"));
        assert!(usage("comment list --status closed").contains("open or resolved"));
        assert!(usage("comment list --author robot").contains("user or agent"));
        assert!(usage("comment list extra").contains("expected 0"));
        assert!(usage("comment apply").contains("--stdin"));
        assert!(usage("comment reopen").contains("expected 1 (<id>)"));
        assert!(usage("comment reply u1").contains("expected 2"));
        assert!(usage("comment reply u1 some-text").contains("from stdin"));
        assert!(usage("comment resolve u1").contains("--reply -"));
        assert!(usage("comment resolve u1 --reply done").contains("from stdin"));
        assert!(usage("comment list --json=1").contains("unknown option --json"));
    }

    #[test]
    fn usage_errors_exit_2_and_show_the_usage() {
        let output = Failure::Usage(Usage("bad".into())).output();
        assert_eq!(output.code, 2);
        assert!(output.stderr.starts_with("bad\nusage:") && output.stdout.is_empty());
    }

    #[test]
    fn other_failures_exit_1_with_one_line() {
        for failure in [
            Failure::Git(GitError::NotARepo),
            Failure::Git(GitError::NotInstalled),
            Failure::Store(StoreError::Busy),
            Failure::Store(StoreError::Io {
                path: "/p".into(),
                kind: std::io::ErrorKind::PermissionDenied,
            }),
            Failure::NoStateDir,
        ] {
            let output = failure.output();
            assert_eq!(output.code, 1, "{failure:?}");
            assert_eq!(output.stderr.matches('\n').count(), 1, "{failure:?}");
        }
        assert!(
            Failure::Store(StoreError::Io {
                path: "/p".into(),
                kind: std::io::ErrorKind::PermissionDenied
            })
            .output()
            .stderr
            .contains("/p")
        );
    }

    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use crate::store::{
        Add, Author, CommentId, Event, Kind, RelPath, Side, Spec, read, state_dir, write,
    };

    /// A repository root, a state directory under a temp `HOME`, a `git` that names the root and
    /// serves `old` files, and a `herdr` that records its calls.
    struct Fixture {
        env: Env,
        root: PathBuf,
        dir: PathBuf,
        /// `git show` arguments (`HEAD:a.rs`) and the file text they print.
        old: BTreeMap<String, String>,
        /// The answer to `herdr agent list`.
        agents: Result<String, String>,
        herdr_calls: RefCell<Vec<String>>,
        /// The answer to `hc get`.
        terminal: Result<String, HcError>,
        hc_calls: RefCell<Vec<String>>,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let home = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("herdr-review-cli-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            let root = home.join("repo");
            std::fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let env = Env::new(
                [("HOME".to_owned(), home.display().to_string())],
                "/work".into(),
            );
            let dir = state_dir(&env.state_base().unwrap(), &root);
            Self {
                env,
                root,
                dir,
                old: BTreeMap::new(),
                agents: Err("herdr is not running".into()),
                herdr_calls: RefCell::new(Vec::new()),
                terminal: Err(HcError {
                    exit: Some(3),
                    kind: None,
                    message: "no terminal".into(),
                }),
                hc_calls: RefCell::new(Vec::new()),
            }
        }

        /// Write a worktree file.
        fn file(&self, path: &str, text: &str) {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn git(&self, args: &[String]) -> Result<String, GitError> {
            let failed = || GitError::Failed {
                args: args.join(" "),
                stderr: "fatal".into(),
            };
            match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                [_, _, "rev-parse", ..] => Ok(format!("{}\n", self.root.display())),
                [_, _, "merge-base", ..] => Ok("abc123\n".into()),
                [_, _, "show", spec] => self.old.get(*spec).cloned().ok_or_else(failed),
                _ => Err(failed()),
            }
        }

        fn herdr(&self, args: &[String]) -> Result<String, String> {
            self.herdr_calls.borrow_mut().push(args.join(" "));
            match args.first().map(String::as_str) {
                Some("agent") => self.agents.clone(),
                _ => self.agents.clone().map(|_| String::new()),
            }
        }

        fn hc(&self, args: &[String]) -> Result<String, HcError> {
            self.hc_calls.borrow_mut().push(args.join(" "));
            self.terminal.clone()
        }

        /// Run a command line with `stdin` and a fixed clock.
        fn run(&self, line: &str, stdin: &[u8]) -> Output {
            let Ok(Command::Comment { repo, action }) = parse(&args(line)) else {
                panic!("not a comment command: {line}");
            };
            run_comment(
                repo.as_deref(),
                &action,
                &self.env,
                "2026-10-04T00:00:00Z",
                |args| self.git(args),
                |args| self.herdr(args),
                |args| self.hc(args),
                stdin,
            )
        }

        /// Add root comments by the user: u1, u2, ...
        fn seed(&self, count: usize) {
            for n in 1..=count {
                let add = Add {
                    id: CommentId::parse(&format!("u{n}")).unwrap(),
                    parent: None,
                    path: RelPath::parse("a.rs"),
                    old_path: None,
                    side: Some(Side::New),
                    line: Some(3),
                    end_line: None,
                    line_text: Some("x".into()),
                    spec: Some(Spec::WorkTree),
                    body: format!("comment {n}"),
                };
                let event = Event {
                    at: "t".into(),
                    by: Author::User,
                    kind: Kind::Add(add),
                };
                write(&self.dir, "t", |_, _| Ok::<_, ()>((vec![event], ()))).unwrap();
            }
        }

        fn log(&self) -> String {
            std::fs::read_to_string(self.dir.join("review.jsonl")).unwrap_or_default()
        }
    }

    #[test]
    fn a_directory_that_is_not_a_repo_exits_1() {
        let env = Env::new([("HOME".to_owned(), "/h".to_owned())], "/work".into());
        let action = CommentAction::List {
            filter: Filter::default(),
            json: false,
        };
        let output = run_comment(
            None,
            &action,
            &env,
            "t",
            |_| Err(GitError::NotARepo),
            |_| Err(String::new()),
            |_| Err(HcError::default()),
            &b""[..],
        );
        assert_eq!(
            (output.code, output.stderr.as_str()),
            (1, "not a git repository\n")
        );
    }

    #[test]
    fn list_finds_the_review_of_the_repo_root() {
        let fixture = Fixture::new("list");
        fixture.seed(1);
        let text = fixture.run("comment list", b"");
        assert_eq!(
            (text.code, text.stdout.as_str()),
            (0, "u1 open, unsent a.rs:3 (R) by user\n  comment 1\n")
        );
        let json = fixture.run("comment list --json", b"");
        assert!(
            json.stdout.starts_with("{\"threads\":[{") && json.stdout.ends_with("}\n"),
            "{}",
            json.stdout
        );
    }

    #[test]
    fn an_archived_thread_is_gone_from_list_and_the_next_id_is_not_its_own() {
        let fixture = Fixture::new("archived");
        fixture.seed(2);
        let resolve = Event {
            at: "t".into(),
            by: Author::User,
            kind: Kind::Resolve {
                id: CommentId::parse("u2").unwrap(),
            },
        };
        write(&fixture.dir, "t", |_, _| Ok::<_, ()>((vec![resolve], ()))).unwrap();
        assert_eq!(crate::store::archive(&fixture.dir, "t").unwrap().threads, 1);
        let text = fixture.run("comment list", b"");
        assert_eq!(
            (text.code, text.stdout.as_str()),
            (0, "u1 open, unsent a.rs:3 (R) by user\n  comment 1\n")
        );
        assert!(
            !fixture
                .run("comment list --json", b"")
                .stdout
                .contains("u2")
        );
        assert_eq!(
            fixture.run("comment list --status resolved", b"").stdout,
            ""
        );
        assert_eq!(fixture.run("comment reopen u2", b"").code, 2);
        let next = write(&fixture.dir, "t", |review, _| {
            Ok::<_, ()>((Vec::new(), review.ids.clone().comment(&Author::User)))
        });
        assert_eq!(next.unwrap().as_str(), "u3");
    }

    #[test]
    fn a_wrong_id_exits_2_and_prints_the_open_ids() {
        let fixture = Fixture::new("unknown");
        fixture.seed(2);
        assert_eq!(fixture.run("comment resolve u2 --reply -", b"done").code, 0);
        let before = fixture.log();
        for line in [
            "comment resolve zz9 --reply -",
            "comment reply zz9 -",
            "comment reopen zz9",
        ] {
            let output = fixture.run(line, b"text");
            assert_eq!(output.code, 2, "{line}");
            assert_eq!(
                output.stderr, "unknown id zz9, open threads: u1\n",
                "{line}"
            );
        }
        assert_eq!(fixture.log(), before);
    }

    #[test]
    fn a_reply_id_is_not_a_thread_id() {
        let fixture = Fixture::new("reply-id");
        fixture.seed(1);
        assert_eq!(fixture.run("comment reply u1 -", b"first").stdout, "a1\n");
        let output = fixture.run("comment reply a1 -", b"second");
        assert_eq!(
            (output.code, output.stderr.as_str()),
            (2, "unknown id a1, open threads: u1\n")
        );
    }

    #[test]
    fn with_no_open_thread_the_error_says_so() {
        let fixture = Fixture::new("none-open");
        let output = fixture.run("comment reply u1 -", b"text");
        assert_eq!(output.stderr, "unknown id u1, there are no open threads\n");
    }

    #[test]
    fn resolve_without_a_reply_is_a_usage_error() {
        let usage = parse(&args("comment resolve u1")).unwrap_err();
        assert_eq!(Failure::Usage(usage).output().code, 2);
    }

    #[test]
    fn a_resolve_adds_the_reply_and_marks_the_thread_new() {
        let fixture = Fixture::new("resolve");
        fixture.seed(1);
        let output = fixture.run(
            "comment resolve --name claude u1 --reply -",
            b"Added with_capacity\n",
        );
        assert_eq!(
            (output.code, output.stdout.as_str(), output.stderr.as_str()),
            (0, "a1\n", "")
        );
        let review = read(&fixture.dir).unwrap();
        let thread = &review.threads[0];
        assert_eq!(
            thread.status,
            crate::store::Status::Resolved {
                by: Author::Agent(Some("claude".into()))
            }
        );
        assert!(thread.is_new);
        assert_eq!(
            thread.replies[0].author,
            Author::Agent(Some("claude".into()))
        );
        assert_eq!(thread.replies[0].body, "Added with_capacity\n");
    }

    #[test]
    fn a_second_resolve_or_reopen_exits_0_and_writes_nothing() {
        let fixture = Fixture::new("repeat");
        fixture.seed(1);
        assert_eq!(fixture.run("comment reopen u1", b"").code, 0);
        assert_eq!(fixture.run("comment resolve u1 --reply -", b"done").code, 0);
        let resolved = fixture.log();
        let again = fixture.run("comment resolve u1 --reply -", b"done again");
        assert_eq!(
            (again.code, again.stdout.as_str(), again.stderr.as_str()),
            (0, "", "")
        );
        assert_eq!(fixture.log(), resolved);
        assert_eq!(fixture.run("comment reopen u1", b"").code, 0);
        let reopened = fixture.log();
        assert_ne!(reopened, resolved);
        assert!(read(&fixture.dir).unwrap().threads[0].is_open());
        assert_eq!(fixture.run("comment reopen u1", b"").code, 0);
        assert_eq!(fixture.log(), reopened);
    }

    #[test]
    fn a_reply_leaves_the_status_alone() {
        let fixture = Fixture::new("reply");
        fixture.seed(1);
        fixture.run("comment reply u1 -", b"question?");
        let review = read(&fixture.dir).unwrap();
        assert!(review.threads[0].is_open() && review.threads[0].replies.len() == 1);
        assert_eq!(review.threads[0].replies[0].author, Author::Agent(None));
    }

    #[test]
    fn reply_text_with_shell_syntax_is_stored_byte_for_byte() {
        let fixture = Fixture::new("bytes");
        fixture.seed(1);
        let text =
            "uses `id` and $(id) and \"quotes\" and 'single' and \\ and ${HOME}\n  indented\n";
        assert_eq!(fixture.run("comment reply u1 -", text.as_bytes()).code, 0);
        assert_eq!(read(&fixture.dir).unwrap().threads[0].replies[0].body, text);
    }

    #[test]
    fn an_empty_oversized_or_non_utf8_text_is_rejected_and_nothing_is_written() {
        let fixture = Fixture::new("bad-text");
        fixture.seed(1);
        let before = fixture.log();
        let huge = "x".repeat(MAX_BODY + 1);
        let at_limit = "x".repeat(MAX_BODY);
        for (stdin, why) in [
            (&b""[..], "empty"),
            (b" \n\t ", "empty"),
            (huge.as_bytes(), "limit is 16384"),
            (&[0xff, 0xfe][..], "UTF-8"),
        ] {
            for line in ["comment reply u1 -", "comment resolve u1 --reply -"] {
                let output = fixture.run(line, stdin);
                assert_eq!(output.code, 2, "{line} {why}");
                assert!(output.stderr.contains(why), "{}", output.stderr);
            }
        }
        assert_eq!(fixture.log(), before);
        assert_eq!(
            fixture.run("comment reply u1 -", at_limit.as_bytes()).code,
            0
        );
    }

    fn batch(entries: &[&str]) -> String {
        format!(r#"{{"comments":[{}]}}"#, entries.join(","))
    }

    const LINE: &str = r#"{"path":"src/a.rs","side":"new","line":2,"body":"rename"}"#;

    fn with_source(name: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture.seed(1);
        fixture.file("src/a.rs", "one\r\ntwo\r\nthree\n");
        fixture
    }

    #[test]
    fn a_batch_is_written_with_the_line_text_the_cli_read() {
        let mut fixture = with_source("apply");
        fixture
            .old
            .insert("HEAD:src/a.rs".into(), "was one\nwas two\n".into());
        let input = batch(&[
            LINE,
            r#"{"path":"src/a.rs","line":1,"end_line":3,"body":"range"}"#,
            r#"{"path":"src/a.rs","body":"whole file"}"#,
            r#"{"path":"src/a.rs","side":"old","line":2,"body":"old side"}"#,
            r#"{"reply_to":"u1","body":"a reply"}"#,
        ]);
        let output = fixture.run("comment apply --stdin", input.as_bytes());
        assert_eq!(
            (output.code, output.stdout.as_str(), output.stderr.as_str()),
            (0, "a1\na2\na3\na4\na5\n", "")
        );
        let review = read(&fixture.dir).unwrap();
        let anchors = review
            .threads
            .iter()
            .skip(1)
            .map(|t| t.anchor.clone())
            .collect::<Vec<_>>();
        let path = RelPath::parse("src/a.rs").unwrap();
        let line = |side, line, text: &str| crate::store::AnchorTarget::Line {
            side,
            line,
            text: text.into(),
        };
        assert_eq!(anchors[0].target, line(Side::New, 2, "two"));
        assert_eq!(
            anchors[1].target,
            crate::store::AnchorTarget::Range {
                side: Side::New,
                start: 1,
                end: 3,
                text: "one".into()
            }
        );
        assert_eq!(anchors[2].target, crate::store::AnchorTarget::File);
        assert_eq!(anchors[3].target, line(Side::Old, 2, "was two"));
        assert!(
            anchors
                .iter()
                .all(|a| a.path == path && a.spec == Spec::WorkTree)
        );
        assert_eq!(review.threads[0].replies[0].id.as_str(), "a5");
        assert_eq!(review.threads[1].root.author, Author::Agent(None));
        // The lookup failed, so the batch says "an agent"; the notification is the last call.
        assert_eq!(
            fixture.herdr_calls.borrow().last().map(String::as_str),
            Some("notification show 5 review comments from an agent")
        );
    }

    #[test]
    fn a_batch_ends_with_one_notification_naming_the_agent() {
        let mut fixture = with_source("notify");
        fixture.agents = Ok(
            r#"{"result":{"agents":[{"pane_id":"w1:p2","agent":"claude","cwd":"/else"}]}}"#.into(),
        );
        fixture.env = Env::new(
            [
                (
                    "HOME".to_owned(),
                    fixture.env.get("HOME").unwrap().to_owned(),
                ),
                ("HERDR_PANE_ID".to_owned(), "w1:p2".to_owned()),
            ],
            "/work".into(),
        );
        let input = batch(&[LINE, r#"{"path":"src/a.rs","body":"two"}"#]);
        assert_eq!(
            fixture.run("comment apply --stdin", input.as_bytes()).code,
            0
        );
        assert_eq!(
            fixture.herdr_calls.into_inner(),
            [
                "agent list",
                "notification show 2 review comments from claude"
            ]
        );
        let review = read(&fixture.dir).unwrap();
        assert_eq!(
            review.threads[1].root.author,
            Author::Agent(Some("claude".into()))
        );
    }

    #[test]
    fn under_hc_the_agent_name_comes_from_hc_get_and_nothing_is_notified() {
        let mut fixture = with_source("hc-name");
        fixture.env = Env::new(
            [
                (
                    "HOME".to_owned(),
                    fixture.env.get("HOME").unwrap().to_owned(),
                ),
                ("HC_TERMINAL_ID".to_owned(), "t7".to_owned()),
            ],
            "/work".into(),
        );
        fixture.terminal =
            Ok(r#"{"address":"t7","status":{"app":"claude-code","state":"working"}}"#.into());
        let input = batch(&[LINE]);
        assert_eq!(
            fixture.run("comment apply --stdin", input.as_bytes()).code,
            0
        );
        assert_eq!(fixture.hc_calls.borrow().as_slice(), ["get t7 --json"]);
        assert!(
            fixture.herdr_calls.borrow().is_empty(),
            "no lookup, no notification"
        );
        assert_eq!(
            read(&fixture.dir)
                .unwrap()
                .threads
                .last()
                .unwrap()
                .root
                .author,
            Author::Agent(Some("claude-code".into()))
        );
        let reply = fixture.run("comment reply u1 -", b"ok");
        assert_eq!(reply.code, 0);
        assert_eq!(
            read(&fixture.dir).unwrap().threads[0].replies[0].author,
            Author::Agent(Some("claude-code".into()))
        );
    }

    #[test]
    fn under_hc_a_failed_hc_get_gives_a_plain_agent_and_does_not_fail_the_command() {
        let mut fixture = with_source("hc-down");
        fixture.env = Env::new(
            [
                (
                    "HOME".to_owned(),
                    fixture.env.get("HOME").unwrap().to_owned(),
                ),
                ("HC_TERMINAL_ID".to_owned(), "t7".to_owned()),
            ],
            "/work".into(),
        );
        let input = batch(&[LINE]);
        assert_eq!(
            fixture.run("comment apply --stdin", input.as_bytes()).code,
            0
        );
        assert_eq!(
            read(&fixture.dir)
                .unwrap()
                .threads
                .last()
                .unwrap()
                .root
                .author,
            Author::Agent(None)
        );
    }

    #[test]
    fn the_send_flags_parse_and_the_wrong_ones_are_usage_errors() {
        assert!(usage("send --target nope").contains("hc address"));
        assert!(usage("send --target t7 --backend herdr").contains("not with --backend herdr"));
        assert!(usage("send --backend other").contains("herdr or hc"));
        assert!(usage("send --thread u1 --all-open").contains("not both"));
        assert!(usage("send --thread").contains("needs a value"));
    }

    #[test]
    fn one_comment_reads_as_a_singular_and_the_name_flag_skips_the_lookup() {
        let fixture = with_source("single");
        let output = fixture.run(
            "comment apply --stdin --name codex",
            batch(&[LINE]).as_bytes(),
        );
        assert_eq!(output.code, 0);
        assert_eq!(
            fixture.herdr_calls.into_inner(),
            ["notification show 1 review comment from codex"]
        );
    }

    #[test]
    fn a_failing_herdr_still_exits_0_and_labels_the_author_agent() {
        let fixture = with_source("herdr-down");
        let output = fixture.run("comment apply --stdin", batch(&[LINE]).as_bytes());
        assert_eq!((output.code, output.stderr.as_str()), (0, ""));
        assert_eq!(
            read(&fixture.dir).unwrap().threads[1].root.author,
            Author::Agent(None)
        );
    }

    #[test]
    fn each_bad_entry_rejects_the_whole_batch_with_its_index_and_writes_nothing() {
        let fixture = with_source("invalid");
        let before = fixture.log();
        let huge = format!(
            r#"{{"path":"src/a.rs","body":"{}"}}"#,
            "x".repeat(MAX_BODY + 1)
        );
        let cases: Vec<(String, &str)> = vec![
            (
                r#"{"path":"src/a.rs","line":4,"body":"x"}"#.into(),
                "line 4 is past the end of src/a.rs (3 lines)",
            ),
            (
                r#"{"path":"src/a.rs","line":1,"end_line":9,"body":"x"}"#.into(),
                "line 9 is past the end",
            ),
            (
                r#"{"path":"/etc/passwd","body":"x"}"#.into(),
                "relative to the repository",
            ),
            (
                r#"{"path":"../x.rs","body":"x"}"#.into(),
                "relative to the repository",
            ),
            (
                r#"{"path":"a/../../x.rs","body":"x"}"#.into(),
                "relative to the repository",
            ),
            (
                r#"{"path":"src/a.rs","body":"  \n"}"#.into(),
                "the text is empty",
            ),
            (huge, "limit is 16384"),
            (
                r#"{"path":"src/a.rs","line":0,"body":"x"}"#.into(),
                "line must be 1 or more",
            ),
            (
                r#"{"path":"src/a.rs","line":-1,"body":"x"}"#.into(),
                "invalid value",
            ),
            (
                r#"{"path":"src/a.rs","line":3,"end_line":2,"body":"x"}"#.into(),
                "end_line is before line",
            ),
            (
                r#"{"path":"src/a.rs","side":"old","body":"x"}"#.into(),
                "need a line",
            ),
            (
                r#"{"path":"src/a.rs","side":"left","line":1,"body":"x"}"#.into(),
                "unknown variant",
            ),
            (
                r#"{"path":"src/a.rs","line":1,"body":"x","tag":1}"#.into(),
                "unknown field",
            ),
            (r#"{"body":"x"}"#.into(), "path is missing"),
            (r#"{"path":"src/a.rs"}"#.into(), "missing field"),
            (
                r#"{"reply_to":"u1","path":"src/a.rs","body":"x"}"#.into(),
                "only reply_to and body",
            ),
            (
                r#"{"reply_to":"zz9","body":"x"}"#.into(),
                "reply_to zz9 is not a thread, open threads: u1",
            ),
            (
                r#"{"path":"src/missing.rs","line":1,"body":"x"}"#.into(),
                "cannot read src/missing.rs",
            ),
            (
                r#"{"path":"src/a.rs","side":"old","line":1,"body":"x"}"#.into(),
                "src/a.rs does not exist in HEAD",
            ),
        ];
        for (entry, why) in cases {
            let input = batch(&[LINE, &entry, LINE]);
            let output = fixture.run("comment apply --stdin", input.as_bytes());
            assert_eq!(output.code, 2, "{why}: {}", output.stderr);
            assert!(
                output.stderr.starts_with("comments[1]: "),
                "{why}: {}",
                output.stderr
            );
            assert!(output.stderr.contains(why), "{why}: {}", output.stderr);
            assert_eq!(
                output.stderr.matches('\n').count(),
                1,
                "one line: {}",
                output.stderr
            );
            assert_eq!(fixture.log(), before, "{why}");
        }
        assert!(
            fixture
                .herdr_calls
                .borrow()
                .iter()
                .all(|call| !call.starts_with("notification"))
        );
    }

    #[test]
    fn a_bad_batch_as_a_whole_exits_2_without_an_index() {
        let fixture = with_source("bad-batch");
        let before = fixture.log();
        let too_many = batch(&vec![LINE; crate::apply::MAX_BATCH + 1]);
        for (input, why) in [
            ("not json".to_owned(), "stdin is not JSON"),
            ("{}".to_owned(), "expected {\"comments\""),
            (r#"{"comments":{}}"#.to_owned(), "expected {\"comments\""),
            (batch(&[]), "holds no comments"),
            (too_many, "holds 201 comments, the limit is 200"),
        ] {
            let output = fixture.run("comment apply --stdin", input.as_bytes());
            assert_eq!(output.code, 2, "{why}");
            assert!(
                output.stderr.starts_with("invalid batch: ") && output.stderr.contains(why),
                "{}",
                output.stderr
            );
        }
        assert_eq!(fixture.run("comment apply --stdin", &[0xff]).code, 2);
        assert_eq!(fixture.log(), before);
        assert_eq!(
            fixture
                .run(
                    "comment apply --stdin",
                    batch(&vec![LINE; crate::apply::MAX_BATCH]).as_bytes()
                )
                .code,
            0
        );
    }

    #[test]
    fn the_branch_spec_reads_the_old_side_at_the_merge_base_and_records_the_spec() {
        use crate::meta::save;
        let mut fixture = with_source("branch");
        let spec = Spec::Branch {
            base: "main".into(),
        };
        save(&fixture.dir, &fixture.root, |meta| {
            meta.spec = Some(spec.clone());
        })
        .unwrap();
        fixture
            .old
            .insert("abc123:src/a.rs".into(), "base one\n".into());
        let old = r#"{"path":"src/a.rs","side":"old","line":1,"body":"x"}"#;
        let output = fixture.run("comment apply --stdin", batch(&[old]).as_bytes());
        assert_eq!((output.code, output.stderr.as_str()), (0, ""));
        let review = read(&fixture.dir).unwrap();
        assert_eq!(review.threads[1].anchor.spec, spec);
        assert_eq!(
            review.threads[1].anchor.target,
            crate::store::AnchorTarget::Line {
                side: Side::Old,
                line: 1,
                text: "base one".into()
            }
        );
    }

    #[test]
    fn an_unreadable_meta_is_a_warning_and_the_batch_still_lands() {
        let fixture = with_source("meta-bad");
        std::fs::create_dir_all(&fixture.dir).unwrap();
        std::fs::write(fixture.dir.join("meta.json"), "{broken").unwrap();
        let output = fixture.run("comment apply --stdin", batch(&[LINE]).as_bytes());
        assert_eq!(output.code, 0);
        assert_eq!(
            output.stderr,
            "warning: meta.json is unreadable and was reset\n"
        );
    }

    #[test]
    fn a_missing_git_on_the_old_side_exits_1() {
        let root = std::env::temp_dir();
        let comments = decode(&batch(&[
            r#"{"path":"a.rs","side":"old","line":1,"body":"x"}"#,
        ]))
        .unwrap();
        let error = read_lines(comments, &root, &Spec::WorkTree, |_| {
            Err(GitError::NotInstalled)
        })
        .unwrap_err();
        assert_eq!(Failure::from(error).output().code, 1);
    }

    // The user CLI: `--as user`, body files, `edit`, `delete` and `seen`.

    use std::os::unix::fs::PermissionsExt;

    impl Fixture {
        /// A body file in the fixture's folder. Returns its path.
        fn body(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.root.parent().unwrap().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        /// An agent's root comment on line 2 of `src/a.rs`: `a1`.
        fn agent_root(&self) -> String {
            let out = self.run(
                "comment apply --stdin --name claude",
                batch(&[LINE]).as_bytes(),
            );
            assert_eq!(out.code, 0);
            out.stdout.trim_end().to_owned()
        }

        fn thread(&self, id: &str) -> crate::store::Thread {
            read(&self.dir)
                .unwrap()
                .threads
                .into_iter()
                .find(|t| t.root.id.as_str() == id)
                .unwrap()
        }
    }

    fn sent(fixture: &Fixture, ids: &[&str]) {
        let event = Event {
            at: "t".into(),
            by: Author::User,
            kind: Kind::Sent {
                ids: ids.iter().map(|id| CommentId::parse(id).unwrap()).collect(),
                batch: crate::store::BatchId::parse("b1").unwrap(),
            },
        };
        write(&fixture.dir, "t", |_, _| Ok::<_, ()>((vec![event], ()))).unwrap();
    }

    #[test]
    fn the_user_commands_parse_and_the_refusals_are_usage_errors() {
        assert_eq!(
            comment_action("comment reply --as user u1 --body-file /b"),
            CommentAction::Reply {
                name: None,
                user: true,
                id: "u1".into(),
                input: Input::File("/b".into())
            }
        );
        assert_eq!(
            comment_action("comment reply --as user u1 -"),
            CommentAction::Reply {
                name: None,
                user: true,
                id: "u1".into(),
                input: Input::Stdin
            }
        );
        let resolve = |line| match comment_action(line) {
            CommentAction::Resolve { user, input, .. } => (user, input),
            other => panic!("{other:?}"),
        };
        assert_eq!(resolve("comment resolve --as user u1"), (true, None));
        assert_eq!(
            resolve("comment resolve --as user u1 --body-file /b"),
            (true, Some(Input::File("/b".into())))
        );
        assert_eq!(
            resolve("comment resolve --as user u1 --reply -"),
            (true, Some(Input::Stdin))
        );
        assert_eq!(
            comment_action("comment reopen --as user u1"),
            CommentAction::Reopen {
                user: true,
                id: "u1".into()
            }
        );
        assert_eq!(
            comment_action("comment apply --as user --file /f --spec branch --base main"),
            CommentAction::Apply {
                name: None,
                user: true,
                input: Input::File("/f".into()),
                spec: Some(SpecChoice::Branch),
                base: Some("main".into())
            }
        );
        assert_eq!(
            comment_action("comment edit u1 --body-file /b"),
            CommentAction::Edit {
                id: "u1".into(),
                file: "/b".into()
            }
        );
        assert_eq!(
            comment_action("comment delete u1"),
            CommentAction::Delete { id: "u1".into() }
        );
        assert_eq!(
            comment_action("comment seen u1"),
            CommentAction::Seen { id: "u1".into() }
        );
        assert_eq!(
            parse(&args("archive --repo /r --json")),
            Ok(Command::Archive {
                repo: Some("/r".into()),
                json: true
            })
        );
        assert_eq!(
            parse(&args("view --path a.rs --set --spec branch --base main")),
            Ok(Command::View {
                query: Query {
                    repo: None,
                    spec: Some(SpecChoice::Branch),
                    base: Some("main".into())
                },
                path: "a.rs".into(),
                set: true
            })
        );
        for (line, why) in [
            ("comment reply u1 --body-file /b", "--as user"),
            ("comment resolve u1 --body-file /b", "--as user"),
            ("comment apply --stdin --file /f", "--as user"),
            ("comment apply --file /f", "--as user"),
            (
                "comment apply --as robot --stdin",
                "takes user, not 'robot'",
            ),
            ("comment apply --as user", "--file <f> or --stdin"),
            ("comment apply --as user --stdin --file /f", "not both"),
            ("comment apply --as user --name x --file /f", "--name"),
            ("comment reply --as user u1 - --body-file /b", "expected 1"),
            ("comment reply --as user u1", "expected 2"),
            (
                "comment resolve --as user u1 --reply - --body-file /b",
                "not both",
            ),
            ("comment edit u1", "--body-file"),
            ("comment edit --body-file /b", "expected 1"),
            (
                "comment delete u1 --body-file /b",
                "unknown option --body-file",
            ),
            ("comment seen u1 --as user", "unknown option --as"),
            ("comment list --as user", "unknown option --as"),
            ("comment resolve u1 --reply -", ""),
            ("view --path a.rs", "--set or --unset"),
            ("view --path a.rs --set --unset", "--set or --unset"),
            ("view --set", "--path"),
            ("archive now", "expected 0"),
        ] {
            if line == "comment resolve u1 --reply -" {
                assert!(parse(&args(line)).is_ok(), "{line}");
                continue;
            }
            assert!(usage(line).contains(why), "{line}: {}", usage(line));
        }
    }

    #[test]
    fn the_unknown_messages_a_panel_reads_as_a_binary_too_old_stay_as_written() {
        for (line, message) in [
            ("frobnicate", "unknown command 'frobnicate'"),
            ("comment frobnicate", "unknown comment command 'frobnicate'"),
            ("comment list --bogus", "unknown option --bogus"),
            ("state --json --bogus", "unknown option --bogus"),
            ("archive --bogus", "unknown option --bogus"),
            ("view --bogus", "unknown option --bogus"),
        ] {
            let output = Failure::Usage(parse(&args(line)).unwrap_err()).output();
            assert_eq!(output.code, 2, "{line}");
            assert_eq!(output.stderr.lines().next(), Some(message), "{line}");
        }
    }

    #[test]
    fn a_directory_that_is_not_a_repo_exits_1_for_the_user_commands_and_keeps_the_body_file() {
        let fixture = Fixture::new("user-not-a-repo");
        let file = fixture.body("body", b"text");
        for line in [
            format!("comment edit u1 --body-file {}", file.display()),
            format!("comment apply --as user --file {}", file.display()),
            "comment delete u1".to_owned(),
            "comment seen u1".to_owned(),
        ] {
            let Ok(Command::Comment { repo, action }) = parse(&args(&line)) else {
                panic!("{line}");
            };
            let output = run_comment(
                repo.as_deref(),
                &action,
                &fixture.env,
                "t",
                |_| Err(GitError::NotARepo),
                |_| Err(String::new()),
                |_| Err(HcError::default()),
                &b""[..],
            );
            assert_eq!(
                (output.code, output.stderr.as_str()),
                (1, "not a git repository\n"),
                "{line}"
            );
        }
        assert!(file.exists());
    }

    #[test]
    fn as_user_writes_by_user_and_calls_no_herdr_and_sends_no_notification() {
        let mut fixture = with_source("as-user");
        fixture.agents = Ok(
            r#"{"result":{"agents":[{"pane_id":"w1:p2","agent":"claude","cwd":"/else"}]}}"#.into(),
        );
        let file = fixture.body("batch.json", batch(&[LINE]).as_bytes());
        let applied = fixture.run(
            &format!("comment apply --as user --file {}", file.display()),
            b"",
        );
        assert_eq!(
            (
                applied.code,
                applied.stdout.as_str(),
                applied.stderr.as_str()
            ),
            (0, "u2\n", "")
        );
        let body = fixture.body("reply", b"a reply\n");
        let replied = fixture.run(
            &format!("comment reply --as user u2 --body-file {}", body.display()),
            b"",
        );
        assert_eq!(
            replied.stdout,
            "u1_placeholder\n".replace("u1_placeholder", "u3")
        );
        let resolved = fixture.run("comment resolve --as user u2", b"");
        assert_eq!((resolved.code, resolved.stdout.as_str()), (0, ""));
        assert_eq!(fixture.run("comment reopen --as user u2", b"").code, 0);
        let thread = fixture.thread("u2");
        assert_eq!(thread.root.author, Author::User);
        assert_eq!(thread.replies[0].author, Author::User);
        assert_eq!(thread.replies[0].body, "a reply\n");
        assert_eq!(
            fixture.herdr_calls.into_inner(),
            Vec::<String>::new(),
            "no herdr call of any kind"
        );
        assert!(
            fixture.hc_calls.borrow().is_empty(),
            "no hc call of any kind"
        );
        assert!(!file.exists() && !body.exists());
    }

    #[test]
    fn a_body_file_is_read_and_deleted_also_when_the_body_is_refused() {
        let fixture = Fixture::new("body-file");
        fixture.seed(1);
        let before = fixture.log();
        let at_limit = "x".repeat(MAX_BODY);
        let over = "x".repeat(MAX_BODY + 1);
        for (bytes, why) in [
            (over.as_bytes(), "more than 16384 bytes"),
            (&[0xff, 0xfe][..], "UTF-8"),
            (b"  \n", "the text is empty"),
        ] {
            let file = fixture.body("refused", bytes);
            let output = fixture.run(
                &format!("comment reply --as user u1 --body-file {}", file.display()),
                b"",
            );
            assert_eq!(output.code, 2, "{why}");
            assert!(output.stderr.contains(why), "{}", output.stderr);
            assert!(!file.exists(), "{why}: the file is deleted");
        }
        assert_eq!(fixture.log(), before);
        let file = fixture.body("fits", at_limit.as_bytes());
        let output = fixture.run(
            &format!("comment reply --as user u1 --body-file {}", file.display()),
            b"",
        );
        assert_eq!(output.code, 0);
        assert!(!file.exists());
        assert_eq!(fixture.thread("u1").replies[0].body, at_limit);
    }

    #[test]
    fn a_symlink_a_directory_and_a_missing_file_are_refused_and_left_alone() {
        let fixture = Fixture::new("body-refused");
        fixture.seed(1);
        let before = fixture.log();
        let home = fixture.root.parent().unwrap();
        let target = fixture.body("target", b"secret");
        let link = home.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let folder = home.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        for path in [&link, &folder, &home.join("missing")] {
            let output = fixture.run(
                &format!("comment reply --as user u1 --body-file {}", path.display()),
                b"",
            );
            assert_eq!(output.code, 2, "{}", path.display());
            assert!(output.stderr.contains(&path.display().to_string()));
        }
        assert!(link.is_symlink() && folder.is_dir());
        assert_eq!(std::fs::read(&target).unwrap(), b"secret");
        assert_eq!(fixture.log(), before);
    }

    #[test]
    fn a_body_file_that_cannot_be_deleted_is_read_anyway() {
        let fixture = Fixture::new("body-undeletable");
        fixture.seed(1);
        let folder = fixture.root.parent().unwrap().join("locked");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("body");
        std::fs::write(&file, "kept\n").unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
        let output = fixture.run(
            &format!("comment reply --as user u1 --body-file {}", file.display()),
            b"",
        );
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!((output.code, output.stderr.as_str()), (0, ""));
        assert_eq!(fixture.thread("u1").replies[0].body, "kept\n");
    }

    #[test]
    fn an_agent_reply_still_takes_stdin_and_refuses_a_body_file() {
        let fixture = Fixture::new("agent-stdin");
        fixture.seed(1);
        let file = fixture.body("body", b"from file");
        let line = format!("comment reply u1 --body-file {}", file.display());
        assert!(parse(&args(&line)).is_err());
        assert_eq!(
            fixture.run("comment reply u1 -", b"from stdin").stdout,
            "a1\n"
        );
        assert!(file.exists());
    }

    #[test]
    fn a_bad_user_batch_exits_2_and_writes_nothing_and_the_file_is_gone() {
        let fixture = with_source("user-bad-batch");
        let before = fixture.log();
        let entry = r#"{"path":"src/a.rs","line":9,"body":"x"}"#;
        let file = fixture.body("batch.json", batch(&[entry]).as_bytes());
        let output = fixture.run(
            &format!("comment apply --as user --file {}", file.display()),
            b"",
        );
        assert_eq!(output.code, 2);
        assert!(
            output
                .stderr
                .starts_with("comments[0]: line 9 is past the end")
        );
        assert_eq!(fixture.log(), before);
        assert!(!file.exists());
        // A body of the limit fits a batch file, and one past it is the batch check's to refuse.
        let huge = format!(
            r#"{{"path":"src/a.rs","body":"{}"}}"#,
            "x".repeat(MAX_BODY + 1)
        );
        let file = fixture.body("huge.json", batch(&[&huge]).as_bytes());
        let output = fixture.run(
            &format!("comment apply --as user --file {}", file.display()),
            b"",
        );
        assert!(
            output.stderr.contains("limit is 16384"),
            "{}",
            output.stderr
        );
    }

    #[test]
    fn a_user_batch_is_read_under_the_spec_it_names() {
        let mut fixture = with_source("user-spec");
        fixture
            .old
            .insert("abc123:src/a.rs".into(), "base one\n".into());
        let old = r#"{"path":"src/a.rs","side":"old","line":1,"body":"x"}"#;
        for flags in ["--spec branch --base main", "--base main"] {
            let file = fixture.body("batch.json", batch(&[old]).as_bytes());
            let output = fixture.run(
                &format!("comment apply --as user --file {} {flags}", file.display()),
                b"",
            );
            assert_eq!((output.code, output.stderr.as_str()), (0, ""), "{flags}");
        }
        let review = read(&fixture.dir).unwrap();
        for thread in &review.threads[1..] {
            assert_eq!(
                thread.anchor.spec,
                Spec::Branch {
                    base: "main".into()
                }
            );
            assert_eq!(
                thread.anchor.target,
                crate::store::AnchorTarget::Line {
                    side: Side::Old,
                    line: 1,
                    text: "base one".into()
                }
            );
        }
        // The panel's spec is not saved: meta.json is untouched.
        assert!(!fixture.dir.join("meta.json").exists());
        let file = fixture.body("batch.json", batch(&[LINE]).as_bytes());
        let worktree = fixture.run(
            &format!(
                "comment apply --as user --file {} --spec worktree",
                file.display()
            ),
            b"",
        );
        assert_eq!(worktree.code, 0);
        assert_eq!(fixture.thread("u4").anchor.spec, Spec::WorkTree);
    }

    #[test]
    fn an_old_side_anchor_on_a_renamed_file_reads_the_old_path_and_keeps_both() {
        let mut fixture = with_source("renamed");
        fixture
            .old
            .insert("HEAD:src/old.rs".into(), "was one\nwas two\n".into());
        let entry = |old: &str| {
            format!(r#"{{"path":"src/a.rs","old_path":"{old}","side":"old","line":2,"body":"x"}}"#)
        };
        let file = fixture.body("batch.json", batch(&[&entry("src/old.rs")]).as_bytes());
        let output = fixture.run(
            &format!("comment apply --as user --file {}", file.display()),
            b"",
        );
        assert_eq!((output.code, output.stdout.as_str()), (0, "u2\n"));
        let anchor = fixture.thread("u2").anchor;
        assert_eq!(anchor.path, RelPath::parse("src/a.rs").unwrap());
        assert_eq!(anchor.old_path, RelPath::parse("src/old.rs"));
        assert_eq!(
            anchor.target,
            crate::store::AnchorTarget::Line {
                side: Side::Old,
                line: 2,
                text: "was two".into()
            }
        );
        let file = fixture.body("batch.json", batch(&[&entry("src/gone.rs")]).as_bytes());
        let output = fixture.run(
            &format!("comment apply --as user --file {}", file.display()),
            b"",
        );
        assert!(output.stderr.contains("src/gone.rs does not exist in HEAD"));
        let reply = r#"{"reply_to":"u1","old_path":"x.rs","body":"x"}"#;
        let output = fixture.run("comment apply --stdin", batch(&[reply]).as_bytes());
        assert!(output.stderr.contains("only reply_to and body"));
    }

    #[test]
    fn a_user_reopen_makes_the_thread_unsent() {
        let fixture = Fixture::new("reopen-unsent");
        fixture.seed(1);
        sent(&fixture, &["u1"]);
        assert!(!fixture.thread("u1").unsent);
        assert_eq!(fixture.run("comment resolve --as user u1", b"").code, 0);
        assert!(!fixture.thread("u1").unsent);
        assert_eq!(fixture.run("comment reopen --as user u1", b"").code, 0);
        let thread = fixture.thread("u1");
        assert!(thread.is_open() && thread.unsent);
    }

    #[test]
    fn a_user_resolve_takes_an_optional_reply_and_repeats_write_nothing() {
        let fixture = Fixture::new("user-resolve");
        fixture.seed(2);
        let output = fixture.run("comment resolve --as user u1", b"");
        assert_eq!((output.code, output.stdout.as_str()), (0, ""));
        let thread = fixture.thread("u1");
        assert_eq!(
            thread.status,
            crate::store::Status::Resolved { by: Author::User }
        );
        assert!(thread.replies.is_empty() && !thread.is_new);
        let resolved = fixture.log();
        assert_eq!(fixture.run("comment resolve --as user u1", b"").code, 0);
        assert_eq!(fixture.log(), resolved);
        let file = fixture.body("body", b"done\n");
        let output = fixture.run(
            &format!(
                "comment resolve --as user u2 --body-file {}",
                file.display()
            ),
            b"",
        );
        assert_eq!(output.stdout, "u3\n");
        let thread = fixture.thread("u2");
        assert!(!thread.is_open() && thread.replies[0].body == "done\n");
        let reopened = fixture.run("comment reopen --as user u2", b"");
        assert_eq!(reopened.code, 0);
        let log = fixture.log();
        assert_eq!(fixture.run("comment reopen --as user u2", b"").code, 0);
        assert_eq!(fixture.log(), log);
        let unknown = fixture.run("comment resolve --as user zz9", b"");
        assert_eq!(
            (unknown.code, unknown.stderr.as_str()),
            (2, "unknown id zz9, open threads: u2\n")
        );
    }

    #[test]
    fn edit_changes_the_users_comment_and_refuses_an_agents() {
        let fixture = with_source("edit");
        fixture.agent_root();
        let file = fixture.body("body", b"better\n");
        let output = fixture.run(
            &format!("comment edit u1 --body-file {}", file.display()),
            b"",
        );
        assert_eq!(
            (output.code, output.stdout.as_str(), output.stderr.as_str()),
            (0, "", "")
        );
        assert_eq!(fixture.thread("u1").root.body, "better\n");
        assert!(!file.exists());
        let before = fixture.log();
        // The same text writes nothing.
        let file = fixture.body("body", b"better\n");
        assert_eq!(
            fixture
                .run(
                    &format!("comment edit u1 --body-file {}", file.display()),
                    b""
                )
                .code,
            0
        );
        assert_eq!(fixture.log(), before);
        for (id, code, why) in [
            ("a1", 2, "a1 was written by the agent, you cannot change it"),
            ("u9", 2, "unknown id u9, open threads: u1 a1"),
            ("zz9", 2, "unknown id zz9, open threads: u1 a1"),
        ] {
            let file = fixture.body("body", b"mine now");
            let output = fixture.run(
                &format!("comment edit {id} --body-file {}", file.display()),
                b"",
            );
            assert_eq!((output.code, output.stderr.trim_end()), (code, why), "{id}");
            assert!(!file.exists(), "{id}: the body was opened");
        }
        assert_eq!(fixture.log(), before);
        assert_eq!(fixture.thread("a1").root.body, "rename");
    }

    #[test]
    fn delete_removes_the_users_comment_and_refuses_an_agents() {
        let fixture = with_source("delete");
        fixture.seed(1);
        fixture.agent_root();
        fixture.run("comment reply u1 -", b"agent reply");
        let before = fixture.log();
        for (line, why) in [
            (
                "comment delete a1",
                "a1 was written by the agent, you cannot change it",
            ),
            (
                "comment delete a2",
                "a2 was written by the agent, you cannot change it",
            ),
            ("comment delete u9", "unknown id u9, open threads: u1 a1"),
        ] {
            let output = fixture.run(line, b"");
            assert_eq!((output.code, output.stderr.trim_end()), (2, why), "{line}");
        }
        assert_eq!(fixture.log(), before);
        let output = fixture.run("comment delete u1", b"");
        assert_eq!((output.code, output.stdout.as_str()), (0, ""));
        let review = read(&fixture.dir).unwrap();
        assert_eq!(review.threads.len(), 1);
        assert_eq!(review.threads[0].root.id.as_str(), "a1");
    }

    #[test]
    fn seen_marks_a_new_thread_once_and_writes_nothing_for_any_other() {
        let fixture = with_source("seen");
        fixture.seed(1);
        assert_eq!(fixture.run("comment resolve u1 --reply -", b"done").code, 0);
        assert!(fixture.thread("u1").is_new);
        let output = fixture.run("comment seen u1", b"");
        assert_eq!((output.code, output.stdout.as_str()), (0, ""));
        assert!(!fixture.thread("u1").is_new);
        let seen = fixture.log();
        assert_eq!(fixture.run("comment seen u1", b"").code, 0);
        let agent = fixture.agent_root();
        let with_agent = fixture.log();
        assert_eq!(fixture.run(&format!("comment seen {agent}"), b"").code, 0);
        assert_eq!(fixture.log(), with_agent);
        assert_ne!(seen, with_agent);
        let unknown = fixture.run("comment seen u9", b"");
        assert_eq!(
            (unknown.code, unknown.stderr.as_str()),
            (2, "unknown id u9, open threads: a2\n")
        );
    }

    #[test]
    fn a_log_that_cannot_be_read_exits_1_for_every_write() {
        let fixture = Fixture::new("user-io");
        std::fs::create_dir_all(fixture.dir.join("review.jsonl")).unwrap();
        let file = fixture.body("body", b"text");
        for line in [
            "comment seen u1".to_owned(),
            "comment delete u1".to_owned(),
            "comment resolve --as user u1".to_owned(),
            format!("comment edit u1 --body-file {}", file.display()),
        ] {
            let output = fixture.run(&line, b"");
            assert_eq!(output.code, 1, "{line}");
            assert!(output.stderr.contains("review.jsonl"), "{line}");
        }
    }
}
