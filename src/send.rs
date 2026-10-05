//! Send: the prompt the agent receives, and which agent receives it.

use std::fmt::{self, Write as _};
use std::path::Path;

use serde_json::Value;

use crate::agent_delivery::agent_ready;
use crate::env::Env;
use crate::meta::{self, Meta, Target};
use crate::store::{Anchor, AnchorTarget, Comment, PaneId, Side, StoreError, TerminalId, Thread};

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
/// thread (PLAN.md section 6.1). `bin` and `root` are quoted for a POSIX shell.
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
    cwd: std::path::PathBuf,
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

/// The agent a send goes to (PLAN.md section 6.2, ADR 0006): the pane the review was opened beside,
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
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Herdr {
        fn new(panes: &[(&'static str, String)], list: &[String]) -> Self {
            Self {
                panes: panes.to_vec(),
                list: list.to_vec(),
                calls: std::cell::RefCell::default(),
            }
        }

        fn ask(&self, args: &[String]) -> Result<String, String> {
            let call = args.join(" ");
            self.calls.borrow_mut().push(call.clone());
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
}
