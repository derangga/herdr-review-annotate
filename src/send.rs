//! Send: the prompt the agent receives.

use std::fmt::Write as _;
use std::path::Path;

use crate::store::{Anchor, AnchorTarget, Comment, Side, Thread};

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
}
