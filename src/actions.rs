//! What the user's actions write: a comment, a reply, an edit, a delete, a resolve or a reopen.
//!
//! Each is one `store::write`, so the request is checked against the review and appended under
//! one lock. Nothing here touches the terminal.

use std::path::Path;

use crate::comment::{CommandError, check_body, reply as add_reply};
use crate::store::{
    Add, Anchor, AnchorTarget, Author, Comment, CommentId, Event, Kind, Review, Thread, WriteError,
    write,
};

type Written<T> = Result<T, WriteError<CommandError>>;

fn event(now: &str, kind: Kind) -> Event {
    Event {
        at: now.to_owned(),
        by: Author::User,
        kind,
    }
}

fn unknown(review: &Review, id: &CommentId) -> CommandError {
    CommandError::UnknownId {
        id: id.to_string(),
        open: review.open_ids().into_iter().cloned().collect(),
    }
}

/// A comment of the user's, which is what an edit or a delete may touch.
fn own<'a>(review: &'a Review, id: &CommentId) -> Result<&'a Comment, CommandError> {
    let comment = review
        .threads
        .iter()
        .flat_map(Thread::comments)
        .find(|comment| comment.id == *id)
        .ok_or_else(|| unknown(review, id))?;
    if comment.author.is_user() {
        Ok(comment)
    } else {
        Err(CommandError::NotAllowed { id: id.clone() })
    }
}

/// A new thread at `anchor`, which was captured when the action was pressed. Returns its id.
pub fn comment(dir: &Path, now: &str, anchor: &Anchor, body: &str) -> Written<CommentId> {
    write(dir, now, |review, now| {
        check_body(body)?;
        let id = review.ids.clone().comment(&Author::User);
        let (side, line, end_line, line_text) = match &anchor.target {
            AnchorTarget::File => (None, None, None, None),
            AnchorTarget::Line { side, line, text } => {
                (Some(*side), Some(*line), None, Some(text.clone()))
            }
            AnchorTarget::Range {
                side,
                start,
                end,
                text,
            } => (Some(*side), Some(*start), Some(*end), Some(text.clone())),
        };
        let add = Add {
            id: id.clone(),
            parent: None,
            path: Some(anchor.path.clone()),
            old_path: anchor.old_path.clone(),
            side,
            line,
            end_line,
            line_text,
            spec: Some(anchor.spec.clone()),
            body: body.to_owned(),
        };
        Ok((vec![event(now, Kind::Add(add))], id))
    })
}

/// A reply to the thread whose root is `root`. The status stays as it is.
pub fn reply(dir: &Path, now: &str, root: &CommentId, body: &str) -> Written<CommentId> {
    add_reply(dir, now, &Author::User, root.as_str(), body)
}

/// New text for one of the user's comments. The same text writes nothing, so a comment that was
/// sent is not marked edited.
pub fn edit(dir: &Path, now: &str, id: &CommentId, body: &str) -> Written<()> {
    write(dir, now, |review, now| {
        let comment = own(review, id)?;
        check_body(body)?;
        let events = if comment.body == body {
            Vec::new()
        } else {
            vec![event(
                now,
                Kind::Edit {
                    id: id.clone(),
                    body: body.to_owned(),
                },
            )]
        };
        Ok((events, ()))
    })
}

/// Delete one of the user's comments. A root takes its replies with it.
pub fn delete(dir: &Path, now: &str, id: &CommentId) -> Written<()> {
    write(dir, now, |review, now| {
        own(review, id)?;
        Ok((vec![event(now, Kind::Delete { id: id.clone() })], ()))
    })
}

/// Resolve an open thread or reopen a resolved one, whoever wrote it. Returns whether the thread
/// is resolved afterwards.
pub fn toggle(dir: &Path, now: &str, id: &CommentId) -> Written<bool> {
    write(dir, now, |review, now| {
        let thread = review.thread(id).ok_or_else(|| unknown(review, id))?;
        let resolve = thread.is_open();
        let kind = if resolve {
            Kind::Resolve { id: id.clone() }
        } else {
            Kind::Reopen { id: id.clone() }
        };
        Ok((vec![event(now, kind)], resolve))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::store::{RelPath, Side, Spec, Status, read};

    const NOW: &str = "2026-10-05T00:00:00Z";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-review-actions-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn anchor(target: AnchorTarget) -> Anchor {
        Anchor {
            path: RelPath::parse("src/a.rs").unwrap(),
            old_path: None,
            target,
            spec: Spec::WorkTree,
        }
    }

    fn line() -> AnchorTarget {
        AnchorTarget::Line {
            side: Side::New,
            line: 4,
            text: "let x = 1;".into(),
        }
    }

    fn id(text: &str) -> CommentId {
        CommentId::parse(text).unwrap()
    }

    /// A review with the user's `u1` and an agent's `a1`, each a thread.
    fn two_threads(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        comment(&dir, NOW, &anchor(line()), "mine").unwrap();
        let events = Event {
            at: NOW.into(),
            by: Author::Agent(Some("claude".into())),
            kind: Kind::Add(Add {
                id: id("a1"),
                parent: None,
                path: RelPath::parse("src/a.rs"),
                old_path: None,
                side: Some(Side::New),
                line: Some(9),
                end_line: None,
                line_text: Some("y".into()),
                spec: Some(Spec::WorkTree),
                body: "theirs".into(),
            }),
        };
        let path = dir.join("review.jsonl");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&serde_json::to_string(&events).unwrap());
        text.push('\n');
        std::fs::write(path, text).unwrap();
        dir
    }

    #[test]
    fn a_comment_keeps_the_anchor_it_was_given() {
        let dir = temp_dir("anchors");
        let line_id = comment(&dir, NOW, &anchor(line()), "on a line").unwrap();
        let range = AnchorTarget::Range {
            side: Side::Old,
            start: 3,
            end: 6,
            text: "first".into(),
        };
        let mut renamed = anchor(range.clone());
        renamed.old_path = RelPath::parse("src/old.rs");
        let range_id = comment(&dir, NOW, &renamed, "on a range").unwrap();
        let file_id = comment(&dir, NOW, &anchor(AnchorTarget::File), "on a file").unwrap();
        assert_eq!(
            [line_id.as_str(), range_id.as_str(), file_id.as_str()],
            ["u1", "u2", "u3"]
        );
        let review = read(&dir).unwrap();
        assert_eq!(review.threads[0].anchor, anchor(line()));
        assert_eq!(review.threads[1].anchor, renamed);
        assert_eq!(review.threads[2].anchor, anchor(AnchorTarget::File));
        assert!(review.threads.iter().all(|thread| thread.unsent));
    }

    #[test]
    fn an_empty_or_oversized_body_is_refused_and_nothing_is_written() {
        let dir = temp_dir("bad-body");
        for body in ["  \n", &"x".repeat(16 * 1024 + 1)] {
            let result = comment(&dir, NOW, &anchor(line()), body);
            assert!(matches!(
                result,
                Err(WriteError::Build(CommandError::InvalidBody(_)))
            ));
        }
        assert!(read(&dir).unwrap().threads.is_empty());
    }

    #[test]
    fn a_reply_joins_the_thread_and_leaves_its_status() {
        let dir = two_threads("reply");
        toggle(&dir, NOW, &id("a1")).unwrap();
        // The user may reply to either side's thread, resolved or not.
        let first = reply(&dir, NOW, &id("u1"), "more").unwrap();
        let second = reply(&dir, NOW, &id("a1"), "and here").unwrap();
        assert_eq!((first.as_str(), second.as_str()), ("u2", "u3"));
        let review = read(&dir).unwrap();
        assert_eq!(review.threads[0].replies[0].body, "more");
        assert!(matches!(review.threads[1].status, Status::Resolved { .. }));
        assert_eq!(review.threads[1].replies[0].body, "and here");
        let missing = reply(&dir, NOW, &id("u9"), "x");
        assert!(matches!(
            missing,
            Err(WriteError::Build(CommandError::UnknownId { .. }))
        ));
    }

    #[test]
    fn an_edit_changes_the_text_and_the_same_text_writes_nothing() {
        let dir = two_threads("edit");
        edit(&dir, NOW, &id("u1"), "better").unwrap();
        assert_eq!(read(&dir).unwrap().threads[0].root.body, "better");
        let length = std::fs::metadata(dir.join("review.jsonl")).unwrap().len();
        edit(&dir, NOW, &id("u1"), "better").unwrap();
        assert_eq!(
            std::fs::metadata(dir.join("review.jsonl")).unwrap().len(),
            length
        );
        let empty = edit(&dir, NOW, &id("u1"), " ");
        assert!(matches!(
            empty,
            Err(WriteError::Build(CommandError::InvalidBody(_)))
        ));
    }

    #[test]
    fn the_user_cannot_edit_or_delete_an_agent_comment() {
        let dir = two_threads("rights");
        let before = std::fs::read(dir.join("review.jsonl")).unwrap();
        let edited = edit(&dir, NOW, &id("a1"), "mine now");
        let deleted = delete(&dir, NOW, &id("a1"));
        for result in [edited, deleted] {
            assert_eq!(
                result,
                Err(WriteError::Build(CommandError::NotAllowed { id: id("a1") }))
            );
        }
        assert_eq!(std::fs::read(dir.join("review.jsonl")).unwrap(), before);
        // A reply of the agent's inside the user's thread is the agent's too.
        let reply_event = Event {
            at: NOW.into(),
            by: Author::Agent(None),
            kind: Kind::Add(Add {
                id: id("a2"),
                parent: Some(id("u1")),
                path: None,
                old_path: None,
                side: None,
                line: None,
                end_line: None,
                line_text: None,
                spec: None,
                body: "done".into(),
            }),
        };
        let mut text = String::from_utf8(before).unwrap();
        text.push_str(&serde_json::to_string(&reply_event).unwrap());
        text.push('\n');
        std::fs::write(dir.join("review.jsonl"), text).unwrap();
        assert!(matches!(
            delete(&dir, NOW, &id("a2")),
            Err(WriteError::Build(CommandError::NotAllowed { .. }))
        ));
    }

    #[test]
    fn deleting_a_root_deletes_its_thread_and_deleting_a_reply_only_the_reply() {
        let dir = two_threads("delete");
        let reply_id = reply(&dir, NOW, &id("u1"), "extra").unwrap();
        delete(&dir, NOW, &reply_id).unwrap();
        let review = read(&dir).unwrap();
        assert!(review.threads[0].replies.is_empty());
        delete(&dir, NOW, &id("u1")).unwrap();
        let review = read(&dir).unwrap();
        assert_eq!(review.threads.len(), 1);
        // The id is never handed out again.
        let next = comment(&dir, NOW, &anchor(line()), "again").unwrap();
        assert_eq!(next.as_str(), "u3");
        assert!(matches!(
            delete(&dir, NOW, &id("u1")),
            Err(WriteError::Build(CommandError::UnknownId { .. }))
        ));
    }

    #[test]
    fn toggling_resolves_an_open_thread_and_reopens_a_resolved_one_of_either_side() {
        let dir = two_threads("toggle");
        for thread in ["u1", "a1"] {
            assert!(toggle(&dir, NOW, &id(thread)).unwrap());
        }
        let review = read(&dir).unwrap();
        assert!(review.threads.iter().all(|thread| {
            matches!(&thread.status, Status::Resolved { by } if *by == Author::User)
        }));
        // The user resolved it, so it is not new.
        assert!(review.threads.iter().all(|thread| !thread.is_new));
        assert!(!toggle(&dir, NOW, &id("a1")).unwrap());
        let review = read(&dir).unwrap();
        assert!(review.threads[1].is_open());
        assert!(review.threads[1].reopened && review.threads[1].unsent);
        // A reply's id is not a thread.
        let reply_id = reply(&dir, NOW, &id("u1"), "x").unwrap();
        assert!(matches!(
            toggle(&dir, NOW, &reply_id),
            Err(WriteError::Build(CommandError::UnknownId { .. }))
        ));
    }
}
