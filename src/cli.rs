//! The command line: arguments into a `Command`, a `Command` into output and an exit code.
//!
//! Every error is matched once, in `Failure::output` (PLAN.md section 12.6).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::comment::{
    AuthorFilter, CommandError, Filter, StatusFilter, list, render_json, render_text, reopen,
    reply, resolve,
};
use crate::diff::{GitError, RepoRoot};
use crate::env::Env;
use crate::meta::locate;
use crate::store::{Author, StoreError, Warning, WriteError};

pub const USAGE: &str = "usage:
  herdr-review tui    [--repo <root>]
  herdr-review open   [--repo <root>] [--base <ref>]
  herdr-review comment apply   [--repo <root>] [--name <agent>] --stdin
  herdr-review comment list    [--repo <root>] [--status open|resolved] [--author user|agent] [--json]
  herdr-review comment reply   [--repo <root>] [--name <agent>] <id> -
  herdr-review comment resolve [--repo <root>] [--name <agent>] <id> --reply -
  herdr-review comment reopen  [--repo <root>] <id>";

/// The arguments are wrong. Exit code 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentAction {
    Apply { name: Option<String> },
    List { filter: Filter, json: bool },
    Reply { name: Option<String>, id: String },
    Resolve { name: Option<String>, id: String },
    Reopen { id: String },
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
    SpikeSend,
    Comment {
        repo: Option<PathBuf>,
        action: CommentAction,
    },
}

/// Positional arguments, flags that take a value, and flags that do not.
#[derive(Default)]
struct Parsed {
    positional: Vec<String>,
    values: BTreeMap<String, String>,
    switches: BTreeSet<String>,
}

impl Parsed {
    fn value(&mut self, name: &str) -> Option<String> {
        self.values.remove(name)
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
            parsed.values.insert(name, value);
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

fn parse_comment(args: &[String]) -> Result<Command, Usage> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(Usage(
            "comment needs apply, list, reply, resolve or reopen".into(),
        ));
    };
    let mut parsed = match sub.as_str() {
        "apply" => split(rest, &["--repo", "--name"], &["--stdin"])?,
        "list" => split(rest, &["--repo", "--status", "--author"], &["--json"])?,
        "reply" => split(rest, &["--repo", "--name"], &[])?,
        "resolve" => split(rest, &["--repo", "--name", "--reply"], &[])?,
        "reopen" => split(rest, &["--repo"], &[])?,
        other => return Err(Usage(format!("unknown comment command '{other}'"))),
    };
    let repo = parsed.value("--repo").map(PathBuf::from);
    let name = parsed.value("--name").filter(|name| !name.is_empty());
    let action = match sub.as_str() {
        "apply" => {
            if !parsed.switch("--stdin") {
                return Err(Usage(
                    "comment apply reads the batch from stdin, pass --stdin".into(),
                ));
            }
            no_arguments(parsed)?;
            CommentAction::Apply { name }
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
            let [id, dash] = parsed.positionals(["id", "-"])?;
            require_stdin_dash("reply", &dash)?;
            CommentAction::Reply { name, id }
        }
        "resolve" => {
            let reply = parsed.value("--reply").ok_or_else(|| {
                Usage("comment resolve needs --reply - (the reply text on stdin)".into())
            })?;
            require_stdin_dash("--reply", &reply)?;
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Resolve { name, id }
        }
        _ => {
            let [id] = parsed.positionals(["id"])?;
            CommentAction::Reopen { id }
        }
    };
    Ok(Command::Comment { repo, action })
}

/// Text only arrives on stdin, so no shell substitution in it can run (PLAN.md section 5).
fn require_stdin_dash(what: &str, text: &str) -> Result<(), Usage> {
    if text == "-" {
        Ok(())
    } else {
        Err(Usage(format!(
            "{what} takes the text from stdin, write - and pipe the text in"
        )))
    }
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
        "spike-send" => split(rest, &[], &[])
            .and_then(no_arguments)
            .map(|()| Command::SpikeSend),
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

fn author(name: Option<&String>) -> Author {
    Author::Agent(name.cloned())
}

fn comment(
    repo: Option<&Path>,
    action: &CommentAction,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
    mut stdin: impl Read,
) -> Result<Output, Failure> {
    let root = RepoRoot::resolve(repo, &env.cwd, git)?;
    let base = env.state_base().ok_or(Failure::NoStateDir)?;
    let dir = locate(&base, root.path());
    let printed = |id: Option<crate::store::CommentId>| Output {
        stdout: id.map(|id| format!("{id}\n")).unwrap_or_default(),
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
        CommentAction::Reply { name, id } => {
            let text = read_text(&mut stdin)?;
            Ok(printed(Some(reply(
                &dir,
                now,
                &author(name.as_ref()),
                id,
                &text,
            )?)))
        }
        CommentAction::Resolve { name, id } => {
            let text = read_text(&mut stdin)?;
            Ok(printed(resolve(
                &dir,
                now,
                &author(name.as_ref()),
                id,
                &text,
            )?))
        }
        CommentAction::Reopen { id } => {
            reopen(&dir, now, &author(None), id)?;
            Ok(printed(None))
        }
        CommentAction::Apply { .. } => Err(Failure::Usage(Usage("not implemented yet".into()))),
    }
}

/// Run a `comment` command. `git` answers every `git` call and `stdin` holds the text or batch.
pub fn run_comment(
    repo: Option<&Path>,
    action: &CommentAction,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
    stdin: impl Read,
) -> Output {
    comment(repo, action, env, now, git, stdin).unwrap_or_else(|failure| failure.output())
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
        assert_eq!(parse(&args("spike-send")), Ok(Command::SpikeSend));
        assert_eq!(
            comment_action("comment apply --stdin --name codex"),
            CommentAction::Apply {
                name: Some("codex".into())
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
                id: "u7".into()
            }
        );
        assert_eq!(
            comment_action("comment resolve --name x u7 --reply -"),
            CommentAction::Resolve {
                name: Some("x".into()),
                id: "u7".into()
            }
        );
        assert_eq!(
            comment_action("comment reopen u7"),
            CommentAction::Reopen { id: "u7".into() }
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

    use crate::store::{Add, CommentId, Event, Kind, RelPath, Side, Spec, read, state_dir, write};

    /// A repository root, a state directory under a temp `HOME`, and a `git` that names the root.
    struct Fixture {
        env: Env,
        root: PathBuf,
        dir: PathBuf,
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
            Self { env, root, dir }
        }

        /// Run a command line with `stdin` and a fixed clock.
        fn run(&self, line: &str, stdin: &[u8]) -> Output {
            let Ok(Command::Comment { repo, action }) = parse(&args(line)) else {
                panic!("not a comment command: {line}");
            };
            let git = |_: &[String]| Ok(format!("{}\n", self.root.display()));
            run_comment(
                repo.as_deref(),
                &action,
                &self.env,
                "2026-10-04T00:00:00Z",
                git,
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
        let huge = "x".repeat(crate::comment::MAX_BODY + 1);
        let at_limit = "x".repeat(crate::comment::MAX_BODY);
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
}
