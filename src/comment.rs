//! The `comment` subcommands an agent runs: what each one reads from the review and prints.

use std::fmt::{self, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::{
    Add, Anchor, AnchorTarget, Author, Comment, CommentId, Event, Kind, Review, Side, Status,
    StoreError, Thread, Warning, WriteError, read, write,
};

/// The most a body may hold after trimming.
pub const MAX_BODY: usize = 16 * 1024;

/// The request cannot be done. Exit code 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// No open thread has this id. `open` lists the ones that do.
    UnknownId { id: String, open: Vec<CommentId> },
    /// The text on stdin cannot be a comment body.
    InvalidBody(String),
    /// One entry of a batch is wrong, or the batch as a whole when `index` is `None`.
    InvalidBatch { index: Option<usize>, why: String },
    /// The user asked to edit or delete a comment the agent wrote (ADR 0004).
    NotAllowed { id: CommentId },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId { id, open } if open.is_empty() => {
                write!(f, "unknown id {id}, there are no open threads")
            }
            Self::UnknownId { id, open } => {
                let open = open.iter().map(CommentId::as_str).collect::<Vec<_>>();
                write!(f, "unknown id {id}, open threads: {}", open.join(" "))
            }
            Self::InvalidBody(why) => f.write_str(why),
            Self::InvalidBatch {
                index: Some(index),
                why,
            } => write!(f, "comments[{index}]: {why}"),
            Self::NotAllowed { id } => {
                write!(f, "{id} was written by the agent, you cannot change it")
            }
            Self::InvalidBatch { index: None, why } => write!(f, "invalid batch: {why}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Check a body as it came from stdin. It is stored as written, so shell syntax in it stays text.
pub fn check_body(text: &str) -> Result<(), CommandError> {
    match text.trim().len() {
        0 => Err(CommandError::InvalidBody("the text is empty".into())),
        n if n > MAX_BODY => Err(CommandError::InvalidBody(format!(
            "the text is {n} bytes, the limit is {MAX_BODY}"
        ))),
        _ => Ok(()),
    }
}

/// The root of the thread called `id`. A reply's id, or one nobody has used, is an unknown id.
fn thread<'a>(review: &'a Review, id: &str) -> Result<&'a Thread, CommandError> {
    CommentId::parse(id)
        .and_then(|id| review.thread(&id))
        .ok_or_else(|| CommandError::UnknownId {
            id: id.to_owned(),
            open: review.open_ids().into_iter().cloned().collect(),
        })
}

fn event(now: &str, by: &Author, kind: Kind) -> Event {
    Event {
        at: now.to_owned(),
        by: by.clone(),
        kind,
    }
}

fn reply_event(
    review: &Review,
    now: &str,
    by: &Author,
    parent: &CommentId,
    body: &str,
) -> (Event, CommentId) {
    let id = review.ids.clone().comment(by);
    let add = Add {
        id: id.clone(),
        parent: Some(parent.clone()),
        path: None,
        old_path: None,
        side: None,
        line: None,
        end_line: None,
        line_text: None,
        spec: None,
        body: body.to_owned(),
    };
    (event(now, by, Kind::Add(add)), id)
}

/// `comment reply <id> -`: add a reply and leave the status alone. Returns the reply's id.
pub fn reply(
    dir: &Path,
    now: &str,
    by: &Author,
    id: &str,
    text: &str,
) -> Result<CommentId, WriteError<CommandError>> {
    write(dir, now, |review, now| {
        let root = thread(review, id)?;
        check_body(text)?;
        let (added, reply_id) = reply_event(review, now, by, &root.root.id, text);
        Ok((vec![added], reply_id))
    })
}

/// `comment resolve <id> --reply -`: add the reply, then resolve. A thread that is already
/// resolved is left as it is, and the reply is not written.
pub fn resolve(
    dir: &Path,
    now: &str,
    by: &Author,
    id: &str,
    text: &str,
) -> Result<Option<CommentId>, WriteError<CommandError>> {
    write(dir, now, |review, now| {
        let root = thread(review, id)?;
        check_body(text)?;
        if !root.is_open() {
            return Ok((Vec::new(), None));
        }
        let (added, reply_id) = reply_event(review, now, by, &root.root.id, text);
        let resolved = event(
            now,
            by,
            Kind::Resolve {
                id: root.root.id.clone(),
            },
        );
        Ok((vec![added, resolved], Some(reply_id)))
    })
}

/// `comment reopen <id>`. A thread that is already open is left as it is.
pub fn reopen(
    dir: &Path,
    now: &str,
    by: &Author,
    id: &str,
) -> Result<(), WriteError<CommandError>> {
    write(dir, now, |review, now| {
        let root = thread(review, id)?;
        let events = if root.is_open() {
            Vec::new()
        } else {
            vec![event(
                now,
                by,
                Kind::Reopen {
                    id: root.root.id.clone(),
                },
            )]
        };
        Ok((events, ()))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    Open,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorFilter {
    User,
    Agent,
}

/// `comment list --status` and `--author`. The author is the root comment's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Filter {
    pub status: Option<StatusFilter>,
    pub author: Option<AuthorFilter>,
}

impl Filter {
    fn keeps(self, thread: &Thread) -> bool {
        let status = self.status.is_none_or(|status| match status {
            StatusFilter::Open => thread.is_open(),
            StatusFilter::Resolved => !thread.is_open(),
        });
        let author = self.author.is_none_or(|author| match author {
            AuthorFilter::User => thread.root.author.is_user(),
            AuthorFilter::Agent => !thread.root.author.is_user(),
        });
        status && author
    }
}

/// The JSON `comment list --json` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub threads: Vec<Thread>,
}

/// The threads that pass `filter`, in file order, and a warning when the log had bad lines.
pub fn list(dir: &Path, filter: Filter) -> Result<(Listing, Option<Warning>), StoreError> {
    let review = read(dir)?;
    let warning = (review.skipped_lines > 0).then_some(Warning::SkippedLine(review.skipped_lines));
    let threads = review
        .threads
        .into_iter()
        .filter(|thread| filter.keeps(thread))
        .collect();
    Ok((Listing { threads }, warning))
}

fn place(anchor: &Anchor) -> String {
    let side = |side: &Side| match side {
        Side::Old => "L",
        Side::New => "R",
    };
    match &anchor.target {
        AnchorTarget::Line { side: s, line, .. } => format!("{}:{line} ({})", anchor.path, side(s)),
        AnchorTarget::Range {
            side: s,
            start,
            end,
            ..
        } => format!("{}:{start}-{end} ({})", anchor.path, side(s)),
        AnchorTarget::File => format!("{} (file)", anchor.path),
    }
}

/// `text` with every line after the first indented by `indent`.
fn indented(text: &str, indent: &str) -> String {
    text.trim_end().replace('\n', &format!("\n{indent}"))
}

/// One block per thread, which a person or an agent can read:
///
/// ```text
/// u7 open src/lib.rs:42 (R) by user
///   body text
///   > a1 agent:claude: reply text
/// ```
pub fn render_text(listing: &Listing) -> String {
    let mut out = String::new();
    for thread in &listing.threads {
        let status = match &thread.status {
            Status::Open => "open".to_owned(),
            Status::Resolved { by } => format!("resolved by {by}"),
        };
        let tags = [(thread.is_new, ", new"), (thread.unsent, ", unsent")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, tag)| *tag)
            .collect::<String>();
        let Comment {
            id, author, body, ..
        } = &thread.root;
        let _ = writeln!(
            out,
            "{id} {status}{tags} {} by {author}\n  {}",
            place(&thread.anchor),
            indented(body, "  ")
        );
        for reply in &thread.replies {
            let _ = writeln!(
                out,
                "  > {} {}: {}",
                reply.id,
                reply.author,
                indented(&reply.body, "    ")
            );
        }
    }
    out
}

pub fn render_json(listing: &Listing) -> String {
    serde_json::to_string(listing).unwrap_or_else(|_| r#"{"threads":[]}"#.to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::store::{RelPath, Spec, fold};

    fn add(by: &Author, id: &str, parent: Option<&str>, body: &str) -> Event {
        let anchored = parent.is_none();
        Event {
            at: "t".into(),
            by: by.clone(),
            kind: Kind::Add(Add {
                id: CommentId::parse(id).unwrap(),
                parent: parent.and_then(CommentId::parse),
                path: anchored.then(|| RelPath::parse("src/a b.rs").unwrap()),
                old_path: None,
                side: anchored.then_some(Side::New),
                line: anchored.then_some(42),
                end_line: None,
                line_text: anchored.then(|| "let x = `1`;".to_owned()),
                spec: anchored.then_some(Spec::WorkTree),
                body: body.into(),
            }),
        }
    }

    fn resolve(by: &Author, id: &str) -> Event {
        Event {
            at: "t".into(),
            by: by.clone(),
            kind: Kind::Resolve {
                id: CommentId::parse(id).unwrap(),
            },
        }
    }

    fn fixture() -> Listing {
        let agent = Author::Agent(Some("claude".into()));
        let events = [
            add(&Author::User, "u1", None, "first\nsecond"),
            add(&agent, "a1", Some("u1"), "done"),
            resolve(&agent, "u1"),
            add(&agent, "a2", None, "my own"),
            add(&Author::User, "u2", None, "open one"),
        ];
        Listing {
            threads: fold(&events).threads,
        }
    }

    fn ids(listing: &Listing) -> Vec<String> {
        listing
            .threads
            .iter()
            .map(|t| t.root.id.to_string())
            .collect()
    }

    #[test]
    fn filters_keep_file_order_and_combine() {
        let all = fixture();
        let run = |status, author| {
            let listing = Listing {
                threads: all
                    .threads
                    .iter()
                    .filter(|t| Filter { status, author }.keeps(t))
                    .cloned()
                    .collect(),
            };
            ids(&listing)
        };
        assert_eq!(run(None, None), ["u1", "a2", "u2"]);
        assert_eq!(run(Some(StatusFilter::Open), None), ["a2", "u2"]);
        assert_eq!(run(Some(StatusFilter::Resolved), None), ["u1"]);
        assert_eq!(run(None, Some(AuthorFilter::Agent)), ["a2"]);
        assert_eq!(
            run(Some(StatusFilter::Open), Some(AuthorFilter::User)),
            ["u2"]
        );
    }

    #[test]
    fn list_reads_the_store_and_warns_about_bad_lines() {
        let dir = std::env::temp_dir().join(format!("herdr-review-list2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (empty, warning) = list(&dir, Filter::default()).unwrap();
        assert!(empty.threads.is_empty() && warning.is_none());
        write(&dir, "t", |_, _| {
            Ok::<_, ()>((vec![add(&Author::User, "u1", None, "x")], ()))
        })
        .unwrap();
        let mut log = std::fs::read_to_string(dir.join("review.jsonl")).unwrap();
        log.push_str("garbage\n");
        std::fs::write(dir.join("review.jsonl"), log).unwrap();
        let (listing, warning) = list(
            &dir,
            Filter {
                status: Some(StatusFilter::Open),
                author: None,
            },
        )
        .unwrap();
        assert_eq!(ids(&listing), ["u1"]);
        assert_eq!(warning, Some(Warning::SkippedLine(1)));
    }

    #[test]
    fn json_parses_back_into_the_same_threads() {
        let listing = fixture();
        let json = render_json(&listing);
        assert_eq!(serde_json::from_str::<Listing>(&json).unwrap(), listing);
        let value = serde_json::from_str::<serde_json::Value>(&json).unwrap();
        assert_eq!(
            value["threads"][0]["status"]["resolved"]["by"],
            "agent:claude"
        );
    }

    #[test]
    fn text_shows_place_status_body_and_replies() {
        let text = render_text(&fixture());
        let expected = "u1 resolved by agent:claude, new, unsent src/a b.rs:42 (R) by user\n  first\n  second\n  > a1 agent:claude: done\n";
        assert!(text.starts_with(expected), "{text}");
        assert!(text.contains("a2 open src/a b.rs:42 (R) by agent:claude\n  my own\n"));
        assert!(text.contains("u2 open, unsent src/a b.rs:42 (R) by user\n  open one\n"));
    }
}
