//! The review event log: the shapes in the log, and the fold that turns events into threads.
//!
//! This part is pure. Reading and writing the file is in the functions below the fold.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which side of a diff a line number counts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Old,
    New,
}

/// Which diff a comment was written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    WorkTree,
    Branch { base: String },
}

impl fmt::Display for Spec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkTree => f.write_str("worktree"),
            Self::Branch { base } => write!(f, "branch:{base}"),
        }
    }
}

impl TryFrom<String> for Spec {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        match text.strip_prefix("branch:") {
            _ if text == "worktree" => Ok(Self::WorkTree),
            Some(base) if !base.is_empty() => Ok(Self::Branch {
                base: base.to_owned(),
            }),
            _ => Err(format!("unknown spec '{text}'")),
        }
    }
}

impl From<Spec> for String {
    fn from(spec: Spec) -> Self {
        spec.to_string()
    }
}

impl Serialize for Spec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Spec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Who wrote an event: `user`, `agent:<name>`, or plain `agent` when the name is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Author {
    User,
    Agent(Option<String>),
}

impl Author {
    pub const fn is_user(&self) -> bool {
        matches!(self, Self::User)
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Agent(None) => f.write_str("agent"),
            Self::Agent(Some(name)) => write!(f, "agent:{name}"),
        }
    }
}

impl Serialize for Author {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Author {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match text.strip_prefix("agent:") {
            _ if text == "user" => Ok(Self::User),
            _ if text == "agent" => Ok(Self::Agent(None)),
            Some(name) if !name.is_empty() => Ok(Self::Agent(Some(name.to_owned()))),
            _ => Err(serde::de::Error::custom(format!("unknown author '{text}'"))),
        }
    }
}

/// Define a string newtype that only its parser can build, and that serializes as that string.
macro_rules! text_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(text: String) -> Result<Self, String> {
                Self::parse(&text).ok_or_else(|| format!("not a valid {}: '{text}'", stringify!($name)))
            }
        }
    };
}

text_id!(
    /// `u<n>` for the user or `a<n>` for an agent.
    CommentId
);
text_id!(
    /// `b<n>`, one per send.
    BatchId
);
text_id!(
    /// A repo-relative path with no `..` and no leading `/`.
    RelPath
);

/// `<prefix><n>` with a decimal counter and no leading zero.
fn counter(text: &str, prefix: char) -> Option<u32> {
    let digits = text.strip_prefix(prefix)?;
    let valid = !digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit());
    valid.then(|| digits.parse().ok()).flatten()
}

impl CommentId {
    pub fn parse(text: &str) -> Option<Self> {
        let valid = counter(text, 'u').or_else(|| counter(text, 'a')).is_some();
        valid.then(|| Self(text.to_owned()))
    }

    /// The side that allocates this id.
    pub fn is_user(&self) -> bool {
        self.0.starts_with('u')
    }
}

impl BatchId {
    pub fn parse(text: &str) -> Option<Self> {
        counter(text, 'b').map(|_| Self(text.to_owned()))
    }
}

impl RelPath {
    pub fn parse(text: &str) -> Option<Self> {
        let valid = !text.is_empty()
            && !text.starts_with('/')
            && !text.contains('\0')
            && text.split('/').all(|part| part != "..");
        valid.then(|| Self(text.to_owned()))
    }
}

/// Allocates ids above every id the log has used, deleted ones included.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ids {
    user: u32,
    agent: u32,
    batch: u32,
}

impl Ids {
    pub fn comment(&mut self, by: &Author) -> CommentId {
        let (prefix, counter) = if by.is_user() {
            ('u', &mut self.user)
        } else {
            ('a', &mut self.agent)
        };
        *counter += 1;
        CommentId(format!("{prefix}{counter}"))
    }

    pub fn batch(&mut self) -> BatchId {
        self.batch += 1;
        BatchId(format!("b{}", self.batch))
    }

    fn seen(&mut self, id: &CommentId) {
        let n = counter(id.as_str(), 'u')
            .or_else(|| counter(id.as_str(), 'a'))
            .unwrap_or(0);
        let slot = if id.is_user() {
            &mut self.user
        } else {
            &mut self.agent
        };
        *slot = (*slot).max(n);
    }
}

/// The fields of an `add` event. A reply has only `id`, `parent` and `body`. A file comment has no
/// `side`, `line` or `line_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Add {
    pub id: CommentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<CommentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<RelPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<RelPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Spec>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    Add(Add),
    Edit { id: CommentId, body: String },
    Delete { id: CommentId },
    Resolve { id: CommentId },
    Reopen { id: CommentId },
    Sent { ids: Vec<CommentId>, batch: BatchId },
    Seen { id: CommentId },
}

/// One line of `review.jsonl`. `at` is RFC 3339 and only for display. File order is the order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    pub by: Author,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: CommentId,
    pub parent: Option<CommentId>,
    pub author: Author,
    pub body: String,
    pub sent_batch: Option<BatchId>,
    pub edited_since_sent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorTarget {
    Line {
        side: Side,
        line: u32,
        text: String,
    },
    Range {
        side: Side,
        start: u32,
        end: u32,
        text: String,
    },
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub path: RelPath,
    pub old_path: Option<RelPath>,
    pub target: AnchorTarget,
    pub spec: Spec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Open,
    Resolved { by: Author },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub root: Comment,
    pub anchor: Anchor,
    pub replies: Vec<Comment>,
    pub status: Status,
    /// An agent resolved it and no `seen` event follows.
    pub is_new: bool,
    /// The user reopened it after its last send.
    pub reopened: bool,
    /// A user comment in it has no send, or the user reopened it after its last send.
    pub unsent: bool,
}

impl Thread {
    /// The root, then the replies.
    pub fn comments(&self) -> impl Iterator<Item = &Comment> {
        std::iter::once(&self.root).chain(&self.replies)
    }

    pub const fn is_open(&self) -> bool {
        matches!(self.status, Status::Open)
    }
}

/// The result of the fold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    /// In the file order of the root comments.
    pub threads: Vec<Thread>,
    /// Lines of the log that did not parse. Set by the reader, not by the fold.
    pub skipped_lines: usize,
    pub ids: Ids,
}

impl Review {
    pub fn thread(&self, id: &CommentId) -> Option<&Thread> {
        self.threads.iter().find(|thread| thread.root.id == *id)
    }

    /// The ids of the open threads, for an error message.
    pub fn open_ids(&self) -> Vec<&CommentId> {
        self.threads
            .iter()
            .filter(|thread| thread.is_open())
            .map(|thread| &thread.root.id)
            .collect()
    }
}

fn comment_mut<'a>(threads: &'a mut [Thread], id: &CommentId) -> Option<&'a mut Comment> {
    threads.iter_mut().find_map(|thread| {
        std::iter::once(&mut thread.root)
            .chain(&mut thread.replies)
            .find(|comment| comment.id == *id)
    })
}

fn root_mut<'a>(threads: &'a mut [Thread], id: &CommentId) -> Option<&'a mut Thread> {
    threads.iter_mut().find(|thread| thread.root.id == *id)
}

fn known(threads: &[Thread], id: &CommentId) -> bool {
    threads
        .iter()
        .any(|thread| thread.comments().any(|comment| comment.id == *id))
}

/// The anchor of a root `add`, or `None` when the event lacks a field its shape needs.
fn anchor(add: &Add) -> Option<Anchor> {
    let target = match (add.side, add.line, &add.line_text) {
        (None, None, None) => AnchorTarget::File,
        (Some(side), Some(line), Some(text)) => match add.end_line {
            Some(end) if end > line => AnchorTarget::Range {
                side,
                start: line,
                end,
                text: text.clone(),
            },
            _ => AnchorTarget::Line {
                side,
                line,
                text: text.clone(),
            },
        },
        _ => return None,
    };
    Some(Anchor {
        path: add.path.clone()?,
        old_path: add.old_path.clone(),
        target,
        spec: add.spec.clone()?,
    })
}

fn apply_add(review: &mut Review, by: &Author, add: &Add) {
    if known(&review.threads, &add.id) || add.id.is_user() != by.is_user() {
        return;
    }
    let comment = Comment {
        id: add.id.clone(),
        parent: add.parent.clone(),
        author: by.clone(),
        body: add.body.clone(),
        sent_batch: None,
        edited_since_sent: false,
    };
    if let Some(parent) = &add.parent {
        let Some(thread) = root_mut(&mut review.threads, parent) else {
            return;
        };
        thread.replies.push(comment);
    } else {
        let Some(anchor) = anchor(add) else { return };
        review.threads.push(Thread {
            root: comment,
            anchor,
            replies: Vec::new(),
            status: Status::Open,
            is_new: false,
            reopened: false,
            unsent: false,
        });
    }
    review.ids.seen(&add.id);
}

fn apply(review: &mut Review, event: &Event) {
    let threads = &mut review.threads;
    match &event.kind {
        Kind::Add(add) => apply_add(review, &event.by, add),
        Kind::Edit { id, body } => {
            // Nobody edits the other side's words (ADR 0004).
            if let Some(comment) =
                comment_mut(threads, id).filter(|c| c.author.is_user() == event.by.is_user())
            {
                comment.body.clone_from(body);
                comment.edited_since_sent =
                    comment.author.is_user() && comment.sent_batch.is_some();
            }
        }
        Kind::Delete { id } => {
            let own =
                comment_mut(threads, id).is_some_and(|c| c.author.is_user() == event.by.is_user());
            if own {
                threads.retain(|thread| thread.root.id != *id);
                for thread in threads.iter_mut() {
                    thread.replies.retain(|reply| reply.id != *id);
                }
            }
        }
        Kind::Resolve { id } => {
            if let Some(thread) = root_mut(threads, id).filter(|thread| thread.is_open()) {
                thread.status = Status::Resolved {
                    by: event.by.clone(),
                };
                thread.is_new = !event.by.is_user();
            }
        }
        Kind::Reopen { id } => {
            if let Some(thread) = root_mut(threads, id).filter(|thread| !thread.is_open()) {
                thread.status = Status::Open;
                thread.is_new = false;
                thread.reopened = event.by.is_user();
            }
        }
        Kind::Sent { ids, batch } => {
            for id in ids {
                if let Some(comment) = comment_mut(threads, id) {
                    comment.sent_batch = Some(batch.clone());
                    comment.edited_since_sent = false;
                }
                if let Some(thread) = root_mut(threads, id) {
                    thread.reopened = false;
                }
            }
        }
        Kind::Seen { id } => {
            if let Some(thread) = root_mut(threads, id) {
                thread.is_new = false;
            }
        }
    }
}

/// Apply the events in order. An event that names an unknown id, or breaks the rights rule, is
/// skipped.
pub fn fold(events: &[Event]) -> Review {
    let mut review = Review::default();
    for event in events {
        apply(&mut review, event);
    }
    for thread in &mut review.threads {
        let unsent_user = thread
            .comments()
            .any(|c| c.author.is_user() && c.sent_batch.is_none());
        thread.unsent = unsent_user || thread.reopened;
    }
    review
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::*;

    fn agent() -> Author {
        Author::Agent(Some("claude".into()))
    }

    fn id(text: &str) -> CommentId {
        CommentId::parse(text).unwrap()
    }

    fn event(by: &Author, kind: Kind) -> Event {
        Event {
            at: "2026-10-04T00:00:00Z".into(),
            by: by.clone(),
            kind,
        }
    }

    fn root(by: &Author, text: &str) -> Event {
        let add = Add {
            id: id(text),
            parent: None,
            path: RelPath::parse("src/lib.rs"),
            old_path: None,
            side: Some(Side::New),
            line: Some(4),
            end_line: None,
            line_text: Some("let x = 1;".into()),
            spec: Some(Spec::WorkTree),
            body: format!("body {text}"),
        };
        event(by, Kind::Add(add))
    }

    fn reply(by: &Author, text: &str, parent: &str) -> Event {
        let add = Add {
            id: id(text),
            parent: Some(id(parent)),
            path: None,
            old_path: None,
            side: None,
            line: None,
            end_line: None,
            line_text: None,
            spec: None,
            body: format!("body {text}"),
        };
        event(by, Kind::Add(add))
    }

    fn sent(ids: &[&str], batch: &str) -> Event {
        let ids = ids.iter().map(|text| id(text)).collect();
        event(
            &Author::User,
            Kind::Sent {
                ids,
                batch: BatchId::parse(batch).unwrap(),
            },
        )
    }

    fn resolve(by: &Author, text: &str) -> Event {
        event(by, Kind::Resolve { id: id(text) })
    }

    fn reopen(by: &Author, text: &str) -> Event {
        event(by, Kind::Reopen { id: id(text) })
    }

    fn thread<'a>(review: &'a Review, text: &str) -> &'a Thread {
        review.thread(&id(text)).unwrap()
    }

    #[test]
    fn ids_only_parse_their_own_form() {
        for good in ["u1", "a12", "u100"] {
            assert!(CommentId::parse(good).is_some(), "{good}");
        }
        for bad in ["", "u", "u0", "u01", "x1", "u-1", "u1 ", "b1"] {
            assert!(CommentId::parse(bad).is_none(), "{bad}");
        }
        assert!(BatchId::parse("b3").is_some() && BatchId::parse("u3").is_none());
        assert!(serde_json::from_str::<CommentId>("\"zz9\"").is_err());
    }

    #[test]
    fn rel_paths_stay_inside_the_repo() {
        assert!(RelPath::parse("src/a b/é.rs").is_some());
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a/.."] {
            assert!(RelPath::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn events_round_trip_through_json() {
        let line = r#"{"kind":"add","at":"t","by":"agent:claude","id":"a1","path":"a.rs","side":"old","line":3,"line_text":"x","spec":"branch:main","body":"b"}"#;
        let parsed: Event = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.by, agent());
        assert_eq!(
            serde_json::from_str::<Event>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
            parsed
        );
        let sent = r#"{"kind":"sent","at":"t","by":"user","ids":["u1"],"batch":"b1"}"#;
        assert!(matches!(
            serde_json::from_str::<Event>(sent).unwrap().kind,
            Kind::Sent { .. }
        ));
    }

    #[test]
    fn events_apply_in_file_order() {
        let review = fold(&[
            root(&Author::User, "u1"),
            resolve(&agent(), "u1"),
            reopen(&Author::User, "u1"),
        ]);
        assert!(thread(&review, "u1").is_open());
        let review = fold(&[
            root(&Author::User, "u1"),
            reopen(&Author::User, "u1"),
            resolve(&agent(), "u1"),
        ]);
        assert!(!thread(&review, "u1").is_open());
    }

    #[test]
    fn threads_keep_the_file_order_of_their_roots() {
        let review = fold(&[
            root(&agent(), "a1"),
            root(&Author::User, "u1"),
            root(&Author::User, "u2"),
        ]);
        let order = review
            .threads
            .iter()
            .map(|t| t.root.id.to_string())
            .collect::<Vec<_>>();
        assert_eq!(order, ["a1", "u1", "u2"]);
    }

    #[test]
    fn an_unknown_id_is_skipped() {
        let review = fold(&[
            root(&Author::User, "u1"),
            resolve(&agent(), "u9"),
            reply(&agent(), "a1", "u9"),
            event(
                &Author::User,
                Kind::Edit {
                    id: id("u9"),
                    body: "x".into(),
                },
            ),
            event(&Author::User, Kind::Delete { id: id("u9") }),
        ]);
        assert_eq!(review.threads.len(), 1);
        assert!(thread(&review, "u1").is_open() && thread(&review, "u1").replies.is_empty());
    }

    #[test]
    fn an_agent_editing_a_user_comment_is_skipped() {
        let edit = event(
            &agent(),
            Kind::Edit {
                id: id("u1"),
                body: "mine".into(),
            },
        );
        let delete = event(&agent(), Kind::Delete { id: id("u1") });
        let review = fold(&[root(&Author::User, "u1"), edit, delete]);
        assert_eq!(thread(&review, "u1").root.body, "body u1");
    }

    #[test]
    fn the_user_cannot_edit_or_delete_an_agent_reply() {
        let edit = event(
            &Author::User,
            Kind::Edit {
                id: id("a1"),
                body: "mine".into(),
            },
        );
        let delete = event(&Author::User, Kind::Delete { id: id("a1") });
        let review = fold(&[
            root(&Author::User, "u1"),
            reply(&agent(), "a1", "u1"),
            edit,
            delete,
        ]);
        assert_eq!(thread(&review, "u1").replies[0].body, "body a1");
    }

    #[test]
    fn either_side_resolves_and_reopens_any_thread() {
        let review = fold(&[root(&Author::User, "u1"), resolve(&agent(), "u1")]);
        assert_eq!(
            thread(&review, "u1").status,
            Status::Resolved { by: agent() }
        );
        let review = fold(&[root(&agent(), "a1"), resolve(&Author::User, "a1")]);
        assert_eq!(
            thread(&review, "a1").status,
            Status::Resolved { by: Author::User }
        );
        let review = fold(&[
            root(&Author::User, "u1"),
            resolve(&agent(), "u1"),
            reopen(&agent(), "u1"),
        ]);
        assert!(thread(&review, "u1").is_open());
    }

    #[test]
    fn only_a_root_has_a_status() {
        let review = fold(&[
            root(&Author::User, "u1"),
            reply(&agent(), "a1", "u1"),
            resolve(&agent(), "a1"),
        ]);
        assert!(thread(&review, "u1").is_open());
    }

    #[test]
    fn a_reply_must_name_a_root() {
        let review = fold(&[
            root(&Author::User, "u1"),
            reply(&agent(), "a1", "u1"),
            reply(&Author::User, "u2", "a1"),
        ]);
        assert_eq!(thread(&review, "u1").replies.len(), 1);
    }

    #[test]
    fn an_id_from_the_wrong_side_or_used_twice_is_skipped() {
        let review = fold(&[
            root(&agent(), "u1"),
            root(&Author::User, "u2"),
            root(&Author::User, "u2"),
        ]);
        assert_eq!(review.threads.len(), 1);
    }

    #[test]
    fn a_thread_is_unsent_until_a_send_carries_the_user_comments() {
        let review = fold(&[root(&Author::User, "u1")]);
        assert!(thread(&review, "u1").unsent);
        let review = fold(&[root(&Author::User, "u1"), sent(&["u1"], "b1")]);
        assert!(!thread(&review, "u1").unsent);
        let review = fold(&[
            root(&Author::User, "u1"),
            sent(&["u1"], "b1"),
            reply(&Author::User, "u2", "u1"),
        ]);
        assert!(thread(&review, "u1").unsent);
    }

    #[test]
    fn an_agent_root_and_agent_replies_are_never_unsent() {
        let review = fold(&[root(&agent(), "a1"), reply(&agent(), "a2", "a1")]);
        assert!(!thread(&review, "a1").unsent);
        let review = fold(&[root(&agent(), "a1"), reply(&Author::User, "u1", "a1")]);
        assert!(thread(&review, "a1").unsent);
    }

    #[test]
    fn a_user_reopen_after_the_last_send_makes_the_thread_unsent() {
        let events = [
            root(&Author::User, "u1"),
            sent(&["u1"], "b1"),
            resolve(&agent(), "u1"),
        ];
        assert!(!thread(&fold(&events), "u1").unsent);
        let mut events = events.to_vec();
        events.push(reopen(&Author::User, "u1"));
        let review = fold(&events);
        assert!(thread(&review, "u1").unsent && thread(&review, "u1").reopened);
        events.push(sent(&["u1"], "b2"));
        let review = fold(&events);
        assert!(!thread(&review, "u1").unsent && !thread(&review, "u1").reopened);
    }

    #[test]
    fn an_agent_reopen_does_not_make_a_thread_unsent() {
        let review = fold(&[
            root(&Author::User, "u1"),
            sent(&["u1"], "b1"),
            resolve(&Author::User, "u1"),
            reopen(&agent(), "u1"),
        ]);
        assert!(!thread(&review, "u1").unsent);
    }

    #[test]
    fn an_edit_never_makes_a_comment_unsent() {
        let edit = event(
            &Author::User,
            Kind::Edit {
                id: id("u1"),
                body: "new text".into(),
            },
        );
        let review = fold(&[root(&Author::User, "u1"), sent(&["u1"], "b1"), edit]);
        let thread = thread(&review, "u1");
        assert!(!thread.unsent && thread.root.edited_since_sent);
        assert_eq!(thread.root.body, "new text");
    }

    #[test]
    fn an_edit_before_any_send_is_not_edited_since_sent_and_a_resend_clears_it() {
        let edit = event(
            &Author::User,
            Kind::Edit {
                id: id("u1"),
                body: "v2".into(),
            },
        );
        let review = fold(&[root(&Author::User, "u1"), edit.clone()]);
        assert!(!thread(&review, "u1").root.edited_since_sent);
        let review = fold(&[
            root(&Author::User, "u1"),
            sent(&["u1"], "b1"),
            edit,
            sent(&["u1"], "b2"),
        ]);
        assert!(!thread(&review, "u1").root.edited_since_sent);
    }

    #[test]
    fn a_thread_is_new_after_an_agent_resolve_until_seen() {
        let seen = event(&Author::User, Kind::Seen { id: id("u1") });
        let review = fold(&[root(&Author::User, "u1"), resolve(&agent(), "u1")]);
        assert!(thread(&review, "u1").is_new);
        let review = fold(&[
            root(&Author::User, "u1"),
            resolve(&agent(), "u1"),
            seen.clone(),
        ]);
        assert!(!thread(&review, "u1").is_new);
        let review = fold(&[root(&Author::User, "u1"), resolve(&Author::User, "u1")]);
        assert!(!thread(&review, "u1").is_new);
        let review = fold(&[
            root(&Author::User, "u1"),
            resolve(&agent(), "u1"),
            reopen(&Author::User, "u1"),
            resolve(&agent(), "u1"),
        ]);
        assert!(thread(&review, "u1").is_new);
    }

    #[test]
    fn deleting_a_root_removes_its_thread_and_a_reply_only_itself() {
        let delete = |text| event(&Author::User, Kind::Delete { id: id(text) });
        let review = fold(&[
            root(&Author::User, "u1"),
            reply(&agent(), "a1", "u1"),
            delete("u1"),
        ]);
        assert!(review.threads.is_empty());
        let review = fold(&[
            root(&Author::User, "u1"),
            reply(&Author::User, "u2", "u1"),
            delete("u2"),
        ]);
        assert_eq!(review.threads.len(), 1);
        assert!(thread(&review, "u1").replies.is_empty());
    }

    #[test]
    fn ids_continue_above_every_id_used_even_deleted_ones() {
        let delete = event(&Author::User, Kind::Delete { id: id("u7") });
        let mut ids = fold(&[root(&Author::User, "u7"), root(&agent(), "a2"), delete]).ids;
        assert_eq!(ids.comment(&Author::User).as_str(), "u8");
        assert_eq!(ids.comment(&agent()).as_str(), "a3");
        assert_eq!(ids.comment(&agent()).as_str(), "a4");
        assert_eq!(ids.batch().as_str(), "b1");
    }

    #[test]
    fn a_root_is_a_line_a_range_or_a_file() {
        let add = |edit: fn(&mut Add)| {
            let Kind::Add(mut add) = root(&Author::User, "u1").kind else {
                unreachable!()
            };
            edit(&mut add);
            fold(&[event(&Author::User, Kind::Add(add))])
                .threads
                .into_iter()
                .next()
                .map(|t| t.anchor.target)
        };
        assert!(matches!(
            add(|_| {}),
            Some(AnchorTarget::Line { line: 4, .. })
        ));
        assert!(matches!(
            add(|a| a.end_line = Some(9)),
            Some(AnchorTarget::Range {
                start: 4,
                end: 9,
                ..
            })
        ));
        assert!(matches!(
            add(|a| a.end_line = Some(4)),
            Some(AnchorTarget::Line { .. })
        ));
        let file = |a: &mut Add| (a.side, a.line, a.line_text) = (None, None, None);
        assert!(matches!(add(file), Some(AnchorTarget::File)));
        assert!(add(|a| a.line_text = None).is_none());
        assert!(add(|a| a.path = None).is_none());
    }
}
