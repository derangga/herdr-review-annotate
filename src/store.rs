//! The review event log: the shapes in the log, and the fold that turns events into threads.
//!
//! This part is pure. Reading and writing the file is in the functions below the fold.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{DirBuilder, File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Which side of a diff a line number counts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// A Herdr pane id such as `w1:p2`.
    PaneId
);
text_id!(
    /// A Herdr terminal id such as `term_65cfeec9f35116`.
    TerminalId
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

/// Herdr ids are short tokens. This accepts the characters they use and nothing a shell would
/// read, since an id ends up in a command line.
fn herdr_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.'))
}

impl PaneId {
    pub fn parse(text: &str) -> Option<Self> {
        herdr_token(text).then(|| Self(text.to_owned()))
    }
}

impl TerminalId {
    pub fn parse(text: &str) -> Option<Self> {
        herdr_token(text).then(|| Self(text.to_owned()))
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

    fn seen_batch(&mut self, batch: &BatchId) {
        self.batch = self.batch.max(counter(batch.as_str(), 'b').unwrap_or(0));
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
    Edit {
        id: CommentId,
        body: String,
    },
    Delete {
        id: CommentId,
    },
    Resolve {
        id: CommentId,
    },
    Reopen {
        id: CommentId,
    },
    Sent {
        ids: Vec<CommentId>,
        batch: BatchId,
    },
    Seen {
        id: CommentId,
    },
    /// Threads were archived. The counters are the highest ids the log had used by then, so an
    /// id that went to `archive.jsonl` is not given out again.
    Archived {
        user: u32,
        agent: u32,
        batch: u32,
    },
}

/// One line of `review.jsonl`. `at` is RFC 3339 and only for display. File order is the order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    pub by: Author,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub parent: Option<CommentId>,
    pub author: Author,
    /// When it was written: the `at` of its `add` event, RFC 3339. An edit does not change it.
    pub at: String,
    pub body: String,
    pub sent_batch: Option<BatchId>,
    pub edited_since_sent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub path: RelPath,
    pub old_path: Option<RelPath>,
    pub target: AnchorTarget,
    pub spec: Spec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Resolved { by: Author },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    fn comments_mut(&mut self) -> impl Iterator<Item = &mut Comment> {
        std::iter::once(&mut self.root).chain(&mut self.replies)
    }

    pub const fn is_open(&self) -> bool {
        matches!(self.status, Status::Open)
    }

    /// `archive` takes it: it is resolved, and the user has seen an agent's resolve.
    pub const fn archivable(&self) -> bool {
        !self.is_open() && !self.is_new
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

/// A review being folded, with the position of the thread that holds each comment id.
struct Fold {
    review: Review,
    index: HashMap<CommentId, usize>,
}

impl Fold {
    fn thread_of(&mut self, id: &CommentId) -> Option<&mut Thread> {
        let at = *self.index.get(id)?;
        self.review.threads.get_mut(at)
    }

    fn comment_mut(&mut self, id: &CommentId) -> Option<&mut Comment> {
        self.thread_of(id)?.comments_mut().find(|c| c.id == *id)
    }

    fn root_mut(&mut self, id: &CommentId) -> Option<&mut Thread> {
        self.thread_of(id).filter(|thread| thread.root.id == *id)
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (at, thread) in self.review.threads.iter().enumerate() {
            for comment in thread.comments() {
                self.index.insert(comment.id.clone(), at);
            }
        }
    }

    fn add(&mut self, by: &Author, at: &str, add: &Add) {
        if self.index.contains_key(&add.id) || add.id.is_user() != by.is_user() {
            return;
        }
        let comment = Comment {
            id: add.id.clone(),
            parent: add.parent.clone(),
            author: by.clone(),
            at: at.to_owned(),
            body: add.body.clone(),
            sent_batch: None,
            edited_since_sent: false,
        };
        let at = if let Some(parent) = &add.parent {
            let Some(at) = self.index.get(parent).copied() else {
                return;
            };
            let Some(thread) = self.root_mut(parent) else {
                return;
            };
            thread.replies.push(comment);
            at
        } else {
            let Some(anchor) = anchor(add) else { return };
            self.review.threads.push(Thread {
                root: comment,
                anchor,
                replies: Vec::new(),
                status: Status::Open,
                is_new: false,
                reopened: false,
                unsent: false,
            });
            self.review.threads.len() - 1
        };
        self.index.insert(add.id.clone(), at);
        self.review.ids.seen(&add.id);
    }

    fn apply(&mut self, event: &Event) {
        let user = event.by.is_user();
        match &event.kind {
            Kind::Add(add) => self.add(&event.by, &event.at, add),
            Kind::Edit { id, body } => {
                // Nobody edits the other side's words (ADR 0004).
                if let Some(comment) = self.comment_mut(id).filter(|c| c.author.is_user() == user) {
                    comment.body.clone_from(body);
                    comment.edited_since_sent =
                        comment.author.is_user() && comment.sent_batch.is_some();
                }
            }
            Kind::Delete { id } => {
                if self
                    .comment_mut(id)
                    .is_none_or(|c| c.author.is_user() != user)
                {
                    return;
                }
                if self.root_mut(id).is_some() {
                    self.review.threads.retain(|thread| thread.root.id != *id);
                    self.reindex();
                } else {
                    if let Some(thread) = self.thread_of(id) {
                        thread.replies.retain(|reply| reply.id != *id);
                    }
                    self.index.remove(id);
                }
            }
            Kind::Resolve { id } => {
                if let Some(thread) = self.root_mut(id).filter(|thread| thread.is_open()) {
                    thread.status = Status::Resolved {
                        by: event.by.clone(),
                    };
                    thread.is_new = !user;
                }
            }
            Kind::Reopen { id } => {
                if let Some(thread) = self.root_mut(id).filter(|thread| !thread.is_open()) {
                    thread.status = Status::Open;
                    thread.is_new = false;
                    thread.reopened = user;
                }
            }
            Kind::Sent { ids, batch } => {
                for id in ids {
                    if let Some(comment) = self.comment_mut(id) {
                        comment.sent_batch = Some(batch.clone());
                        comment.edited_since_sent = false;
                    }
                    if let Some(thread) = self.root_mut(id) {
                        thread.reopened = false;
                    }
                }
                self.review.ids.seen_batch(batch);
            }
            Kind::Seen { id } => {
                if let Some(thread) = self.root_mut(id) {
                    thread.is_new = false;
                }
            }
            Kind::Archived { user, agent, batch } => {
                let ids = &mut self.review.ids;
                ids.user = ids.user.max(*user);
                ids.agent = ids.agent.max(*agent);
                ids.batch = ids.batch.max(*batch);
            }
        }
    }
}

/// Apply the events in order. An event that names an unknown id, or breaks the rights rule, is
/// skipped.
pub fn fold<'a>(events: impl IntoIterator<Item = &'a Event>) -> Review {
    let mut fold = Fold {
        review: Review::default(),
        index: HashMap::new(),
    };
    for event in events {
        fold.apply(event);
    }
    let mut review = fold.review;
    for thread in &mut review.threads {
        let unsent_user = thread
            .comments()
            .any(|c| c.author.is_user() && c.sent_batch.is_none());
        thread.unsent = unsent_user || thread.reopened;
    }
    review
}

/// The disk failed, or the lock was not free in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Io { path: PathBuf, kind: io::ErrorKind },
    Busy,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, kind } => write!(f, "{}: {kind}", path.display()),
            Self::Busy => f.write_str("review is busy, try again"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Not an error. Collected and shown, and the operation still succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    SkippedLine(usize),
    Config(String),
    MetaUnreadable,
    SentNotRecorded,
    TargetNotSaved,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SkippedLine(1) => f.write_str("1 unreadable event"),
            Self::SkippedLine(n) => write!(f, "{n} unreadable events"),
            Self::Config(message) => f.write_str(message),
            Self::MetaUnreadable => f.write_str("meta.json is unreadable and was reset"),
            Self::SentNotRecorded => {
                f.write_str("sent, but not recorded, the next send will repeat these comments")
            }
            Self::TargetNotSaved => {
                f.write_str("could not save the target agent, the next send will look for it again")
            }
        }
    }
}

/// Why `write` did not write: the store failed, or `build` refused the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError<E> {
    Store(StoreError),
    Build(E),
}

impl<E> From<StoreError> for WriteError<E> {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

const REVIEW_FILE: &str = "review.jsonl";
const ARCHIVE_FILE: &str = "archive.jsonl";
const LOCK_FILE: &str = "lock";
const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(2);

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StoreError {
    let path = path.to_path_buf();
    move |error| StoreError::Io {
        path,
        kind: error.kind(),
    }
}

/// `${XDG_STATE_HOME:-~/.local/state}/herdr-review`, on macOS and Linux alike (ADR 0009).
pub fn state_base(xdg_state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let state = match xdg_state_home.filter(|path| path.is_absolute()) {
        Some(xdg) => xdg.to_path_buf(),
        None => home?.join(".local/state"),
    };
    Some(state.join("herdr-review"))
}

/// The directory of one review: the hash of the canonical root, in hex.
pub fn state_dir(base: &Path, root: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    base.join(format!("{:016x}", hasher.finish()))
}

/// Create `dir` with mode 0700 when it is missing.
pub(crate) fn ensure_dir(dir: &Path) -> Result<(), StoreError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io_error(dir))
}

/// The exclusive lock on a review directory. The kernel drops it when the holder exits or dies.
#[derive(Debug)]
pub(crate) struct Lock {
    file: File,
}

impl Drop for Lock {
    /// Unlock, then pause. A writer that loops would otherwise take the lock again before a waiter
    /// polling every few milliseconds sees it free, and the waiter would time out with `Busy`.
    fn drop(&mut self) {
        let _ = self.file.unlock();
        sleep(LOCK_POLL);
    }
}

/// Take the lock, trying every 2 ms for up to 2 seconds.
pub(crate) fn lock(dir: &Path) -> Result<Lock, StoreError> {
    let path = dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .map_err(io_error(&path))?;
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Lock { file }),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => sleep(LOCK_POLL),
            Err(TryLockError::WouldBlock) => return Err(StoreError::Busy),
            Err(TryLockError::Error(error)) => return Err(io_error(&path)(error)),
        }
    }
}

/// What one read of the log returned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chunk {
    pub events: Vec<Event>,
    /// Complete lines that did not parse.
    pub skipped: usize,
    /// Where the next read starts: after the last complete line.
    pub offset: u64,
    /// The file ends in a line with no newline, which is not read.
    pub unterminated: bool,
    /// The file is shorter than the offset asked for, so the read started at 0.
    pub restarted: bool,
}

/// Read the complete lines of `review.jsonl` from byte `from`. A missing file is an empty log.
pub fn read_events(dir: &Path, from: u64) -> Result<Chunk, StoreError> {
    let path = dir.join(REVIEW_FILE);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Chunk {
                restarted: from > 0,
                ..Chunk::default()
            });
        }
        Err(error) => return Err(io_error(&path)(error)),
    };
    let len = file.metadata().map_err(io_error(&path))?.len();
    let restarted = from > len;
    let start = if restarted { 0 } else { from };
    file.seek(SeekFrom::Start(start)).map_err(io_error(&path))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error(&path))?;
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let (lines, tail) = bytes.split_at_checked(complete).unwrap_or_default();
    let mut chunk = Chunk {
        offset: start + complete as u64,
        unterminated: !tail.is_empty(),
        restarted,
        ..Chunk::default()
    };
    for line in lines
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        match serde_json::from_slice(line) {
            Ok(event) => chunk.events.push(event),
            Err(_) => chunk.skipped += 1,
        }
    }
    Ok(chunk)
}

fn fold_chunk(chunk: &Chunk) -> Review {
    let mut review = fold(&chunk.events);
    review.skipped_lines = chunk.skipped + usize::from(chunk.unterminated);
    review
}

/// The byte length of `review.jsonl`, which changes whenever a writer appends. A missing file is 0.
pub fn log_len(dir: &Path) -> Result<u64, StoreError> {
    let path = dir.join(REVIEW_FILE);
    match std::fs::metadata(&path) {
        Ok(meta) => Ok(meta.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(io_error(&path)(error)),
    }
}

/// Read the whole log and fold it. Readers take no lock.
pub fn read(dir: &Path) -> Result<Review, StoreError> {
    read_events(dir, 0).map(|chunk| fold_chunk(&chunk))
}

/// Run one mutation under the lock: read and fold the log, let `build` validate the request and
/// produce the events, and append them with one `write_all`.
///
/// `build` gets the folded review and `now`, and returns the events with a value for the caller.
/// No events means nothing is written. An error from `build` leaves the file unchanged.
pub fn write<T, E>(
    dir: &Path,
    now: &str,
    build: impl FnOnce(&Review, &str) -> Result<(Vec<Event>, T), E>,
) -> Result<T, WriteError<E>> {
    ensure_dir(dir)?;
    let _lock = lock(dir)?;
    let chunk = read_events(dir, 0)?;
    let review = fold_chunk(&chunk);
    let (events, value) = build(&review, now).map_err(WriteError::Build)?;
    if events.is_empty() {
        return Ok(value);
    }
    let path = dir.join(REVIEW_FILE);
    // A crashed writer left half a line. End it, so the next event does not join it.
    let mut bytes = if chunk.unterminated {
        b"\n".to_vec()
    } else {
        Vec::new()
    };
    for event in &events {
        push_line(&mut bytes, event, &path)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(io_error(&path))?;
    Ok(value)
}

/// Add `event` to `bytes` as one line of the log at `path`.
fn push_line(bytes: &mut Vec<u8>, event: &Event, path: &Path) -> Result<(), StoreError> {
    serde_json::to_writer(&mut *bytes, event)
        .map_err(io::Error::from)
        .map_err(io_error(path))?;
    bytes.push(b'\n');
    Ok(())
}

/// What one archive moved out of the review.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Archived {
    pub threads: usize,
    /// How many of them had a comment of the user's that was never sent.
    pub unsent: usize,
}

/// The comment an event is about. A `sent` names several and `archived` names none.
const fn subject(kind: &Kind) -> Option<&CommentId> {
    match kind {
        Kind::Add(add) => Some(&add.id),
        Kind::Edit { id, .. }
        | Kind::Delete { id }
        | Kind::Resolve { id }
        | Kind::Reopen { id }
        | Kind::Seen { id } => Some(id),
        Kind::Sent { .. } | Kind::Archived { .. } => None,
    }
}

/// Move every resolved thread the user has seen out of the review, under the lock: append its
/// events to `archive.jsonl`, then replace `review.jsonl` with a log that has the rest.
///
/// The archive is written first, so a crash between the two leaves a thread in both files and
/// never in neither. The new log starts with an `archived` event that holds the highest ids, and
/// keeps every line that did not parse as it was. With nothing to archive nothing is written.
pub fn archive(dir: &Path, now: &str) -> Result<Archived, StoreError> {
    ensure_dir(dir)?;
    let _lock = lock(dir)?;
    let path = dir.join(REVIEW_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(io_error(&path)(error)),
    };
    // Nobody writes while the lock is held, so a last line with no newline is a line too.
    let lines = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| (line, serde_json::from_slice::<Event>(line).ok()))
        .collect::<Vec<_>>();
    let review = fold(lines.iter().filter_map(|(_, event)| event.as_ref()));
    let taken = review
        .threads
        .iter()
        .filter(|thread| thread.archivable())
        .collect::<Vec<_>>();
    if taken.is_empty() {
        return Ok(Archived::default());
    }
    let done = Archived {
        threads: taken.len(),
        unsent: taken.iter().filter(|thread| thread.unsent).count(),
    };
    // The roots, then every reply to one of them, deleted replies included.
    let mut gone = taken
        .iter()
        .map(|thread| &thread.root.id)
        .collect::<HashSet<_>>();
    for (_, event) in &lines {
        if let Some(Kind::Add(add)) = event.as_ref().map(|event| &event.kind)
            && add
                .parent
                .as_ref()
                .is_some_and(|parent| gone.contains(parent))
        {
            gone.insert(&add.id);
        }
    }
    let archive = dir.join(ARCHIVE_FILE);
    let (mut kept, mut moved) = (Vec::new(), Vec::new());
    let ids = review.ids;
    let carried = Event {
        at: now.to_owned(),
        by: Author::User,
        kind: Kind::Archived {
            user: ids.user,
            agent: ids.agent,
            batch: ids.batch,
        },
    };
    push_line(&mut kept, &carried, &path)?;
    let raw = |bytes: &mut Vec<u8>, line: &[u8]| {
        bytes.extend_from_slice(line);
        bytes.push(b'\n');
    };
    for (line, event) in &lines {
        match event.as_ref().map(|event| (event, &event.kind)) {
            // The one written above replaces it.
            Some((_, Kind::Archived { .. })) => {}
            // A send that carried comments of both kinds is split, and each part keeps the batch.
            Some((event, Kind::Sent { ids, batch })) => {
                let (theirs, ours) = ids
                    .iter()
                    .cloned()
                    .partition::<Vec<_>, _>(|id| gone.contains(id));
                if theirs.is_empty() || ours.is_empty() {
                    raw(
                        if theirs.is_empty() {
                            &mut kept
                        } else {
                            &mut moved
                        },
                        line,
                    );
                    continue;
                }
                for (ids, bytes, path) in [(theirs, &mut moved, &archive), (ours, &mut kept, &path)]
                {
                    let kind = Kind::Sent {
                        ids,
                        batch: batch.clone(),
                    };
                    push_line(
                        bytes,
                        &Event {
                            kind,
                            ..(*event).clone()
                        },
                        path,
                    )?;
                }
            }
            Some((_, kind)) if subject(kind).is_some_and(|id| gone.contains(id)) => {
                raw(&mut moved, line);
            }
            _ => raw(&mut kept, line),
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&archive)
        .and_then(|mut file| {
            file.write_all(&moved)?;
            file.sync_all()
        })
        .map_err(io_error(&archive))?;
    let temp = dir.join(format!("{REVIEW_FILE}.{}.tmp", std::process::id()));
    let written = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(&kept)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, &path));
    written.map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        io_error(&path)(error)
    })?;
    Ok(done)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

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
    fn a_comment_keeps_the_time_of_its_add_event_through_an_edit() {
        let mut later = reply(&agent(), "a1", "u1");
        later.at = "2026-10-04T00:05:00Z".into();
        let mut edit = event(
            &Author::User,
            Kind::Edit {
                id: id("u1"),
                body: "changed".into(),
            },
        );
        edit.at = "2026-10-04T09:00:00Z".into();
        let review = fold(&[root(&Author::User, "u1"), later, edit]);
        let thread = thread(&review, "u1");
        assert_eq!(thread.root.body, "changed");
        assert_eq!(thread.root.at, "2026-10-04T00:00:00Z");
        assert_eq!(thread.replies[0].at, "2026-10-04T00:05:00Z");
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

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("herdr-review-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[allow(clippy::unnecessary_wraps)]
    fn add_root(review: &Review, now: &str) -> Result<(Vec<Event>, CommentId), ()> {
        let id = review.ids.clone().comment(&Author::User);
        let Kind::Add(mut add) = root(&Author::User, id.as_str()).kind else {
            unreachable!()
        };
        add.id = id.clone();
        Ok((
            vec![Event {
                at: now.into(),
                by: Author::User,
                kind: Kind::Add(add),
            }],
            id,
        ))
    }

    fn log(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(REVIEW_FILE)).unwrap_or_default()
    }

    #[test]
    fn the_state_directory_follows_xdg_then_home() {
        let base = |xdg: Option<&str>, home: Option<&str>| {
            state_base(xdg.map(Path::new), home.map(Path::new))
        };
        assert_eq!(base(Some("/x"), Some("/h")), Some("/x/herdr-review".into()));
        assert_eq!(
            base(None, Some("/h")),
            Some("/h/.local/state/herdr-review".into())
        );
        assert_eq!(
            base(Some("rel"), Some("/h")),
            Some("/h/.local/state/herdr-review".into())
        );
        assert_eq!(base(None, None), None);
        let a = state_dir(Path::new("/s"), Path::new("/repo/a"));
        assert_eq!(a, state_dir(Path::new("/s"), Path::new("/repo/a")));
        assert_ne!(a, state_dir(Path::new("/s"), Path::new("/repo/b")));
        assert_eq!(a.file_name().map(std::ffi::OsStr::len), Some(16));
    }

    #[test]
    fn a_write_creates_private_files_and_a_read_folds_them() {
        let dir = temp_dir("write");
        let id = write(&dir, "t", add_root).unwrap();
        assert_eq!(id.as_str(), "u1");
        let id = write(&dir, "t", add_root).unwrap();
        assert_eq!(id.as_str(), "u2");
        let review = read(&dir).unwrap();
        assert_eq!(review.threads.len(), 2);
        assert_eq!(review.skipped_lines, 0);
        let mode = |path: PathBuf| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join(REVIEW_FILE)), 0o600);
        assert_eq!(mode(dir.join(LOCK_FILE)), 0o600);
    }

    #[test]
    fn a_missing_log_reads_as_an_empty_review() {
        let review = read(&temp_dir("missing")).unwrap();
        assert!(review.threads.is_empty());
    }

    #[test]
    fn a_build_error_leaves_the_file_unchanged() {
        let dir = temp_dir("build-error");
        write(&dir, "t", add_root).unwrap();
        let before = log(&dir);
        let result = write(&dir, "t", |_, _| {
            Err::<(Vec<Event>, ()), _>("unknown id zz9")
        });
        assert_eq!(result, Err(WriteError::Build("unknown id zz9")));
        assert_eq!(log(&dir), before);
    }

    #[test]
    fn a_build_with_no_events_writes_nothing() {
        let dir = temp_dir("no-events");
        write(&dir, "t", |_, _| Ok::<_, ()>((Vec::new(), 7))).unwrap();
        assert!(!dir.join(REVIEW_FILE).exists());
    }

    #[test]
    fn a_lock_held_for_longer_than_two_seconds_gives_busy() {
        let dir = temp_dir("busy");
        ensure_dir(&dir).unwrap();
        let held = lock(&dir).unwrap();
        let started = Instant::now();
        let result = write(&dir, "t", add_root);
        let waited = started.elapsed();
        assert_eq!(result, Err(WriteError::Store(StoreError::Busy)));
        assert!(
            waited >= Duration::from_millis(1900) && waited < Duration::from_secs(4),
            "{waited:?}"
        );
        drop(held);
        assert!(write(&dir, "t", add_root).is_ok());
    }

    #[test]
    fn a_lock_freed_within_two_seconds_is_taken() {
        let dir = temp_dir("freed");
        ensure_dir(&dir).unwrap();
        let held = lock(&dir).unwrap();
        let release = std::thread::spawn(move || {
            sleep(Duration::from_millis(300));
            drop(held);
        });
        assert!(write(&dir, "t", add_root).is_ok());
        release.join().unwrap();
    }

    #[test]
    fn a_panic_in_build_releases_the_lock() {
        let dir = temp_dir("panic");
        let result = std::panic::catch_unwind(|| {
            let _ = write::<(), ()>(&dir, "t", |_, _| panic!("broken invariant"));
        });
        assert!(result.is_err());
        assert!(write(&dir, "t", add_root).is_ok());
    }

    #[test]
    fn an_unusable_state_directory_is_an_io_error() {
        let dir = temp_dir("io");
        std::fs::write(&dir, "a file").unwrap();
        let result = write(&dir.join("inner"), "t", add_root);
        assert!(matches!(
            result,
            Err(WriteError::Store(StoreError::Io { .. }))
        ));
        std::fs::remove_file(&dir).unwrap();
    }

    #[test]
    fn an_unreadable_log_is_an_io_error() {
        let dir = temp_dir("unreadable");
        std::fs::create_dir_all(dir.join(REVIEW_FILE)).unwrap();
        assert!(matches!(read(&dir), Err(StoreError::Io { .. })));
        assert!(matches!(
            write(&dir, "t", add_root),
            Err(WriteError::Store(StoreError::Io { .. }))
        ));
    }

    #[test]
    fn a_log_that_cannot_be_appended_to_is_an_io_error() {
        let dir = temp_dir("readonly");
        write(&dir, "t", add_root).unwrap();
        let path = dir.join(REVIEW_FILE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let result = write(&dir, "t", add_root);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            result,
            Err(WriteError::Store(StoreError::Io { .. }))
        ));
    }

    #[test]
    fn a_bad_line_is_skipped_and_counted() {
        let dir = temp_dir("bad-line");
        write(&dir, "t", add_root).unwrap();
        let mut file = OpenOptions::new()
            .append(true)
            .open(dir.join(REVIEW_FILE))
            .unwrap();
        file.write_all(b"not json\n{\"kind\":\"nope\"}\n\n")
            .unwrap();
        drop(file);
        write(&dir, "t", add_root).unwrap();
        let review = read(&dir).unwrap();
        assert_eq!(review.threads.len(), 2);
        assert_eq!(review.skipped_lines, 2);
    }

    #[test]
    fn a_file_cut_mid_line_yields_every_earlier_event_and_one_skipped_line() {
        let dir = temp_dir("truncated");
        write(&dir, "t", add_root).unwrap();
        write(&dir, "t", add_root).unwrap();
        let text = log(&dir);
        std::fs::write(dir.join(REVIEW_FILE), &text[..text.len() - 20]).unwrap();
        let review = read(&dir).unwrap();
        assert_eq!(review.threads.len(), 1);
        assert_eq!(review.skipped_lines, 1);
    }

    #[test]
    fn a_write_after_a_cut_line_does_not_join_it() {
        let dir = temp_dir("rejoin");
        write(&dir, "t", add_root).unwrap();
        let text = log(&dir);
        std::fs::write(dir.join(REVIEW_FILE), &text[..text.len() - 20]).unwrap();
        write(&dir, "t", add_root).unwrap();
        let review = read(&dir).unwrap();
        assert_eq!(review.threads.len(), 1);
        assert_eq!(review.skipped_lines, 1);
    }

    #[test]
    fn a_read_from_an_offset_returns_only_the_new_lines() {
        let dir = temp_dir("offset");
        write(&dir, "t", add_root).unwrap();
        let first = read_events(&dir, 0).unwrap();
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.offset, log(&dir).len() as u64);
        assert!(read_events(&dir, first.offset).unwrap().events.is_empty());
        write(&dir, "t", add_root).unwrap();
        let next = read_events(&dir, first.offset).unwrap();
        assert_eq!(next.events.len(), 1);
        assert!(!next.restarted);
        assert_eq!(next.offset, log(&dir).len() as u64);
    }

    #[test]
    fn a_read_stops_before_a_line_that_is_still_being_written() {
        let dir = temp_dir("partial");
        write(&dir, "t", add_root).unwrap();
        let done = log(&dir).len() as u64;
        let mut file = OpenOptions::new()
            .append(true)
            .open(dir.join(REVIEW_FILE))
            .unwrap();
        file.write_all(b"{\"kind\":\"add\"").unwrap();
        let chunk = read_events(&dir, 0).unwrap();
        assert_eq!(
            (chunk.events.len(), chunk.skipped, chunk.offset),
            (1, 0, done)
        );
        assert!(chunk.unterminated);
    }

    #[test]
    fn a_file_shorter_than_the_offset_is_read_from_the_start() {
        let dir = temp_dir("rewritten");
        write(&dir, "t", add_root).unwrap();
        let chunk = read_events(&dir, 100_000).unwrap();
        assert!(chunk.restarted);
        assert_eq!(chunk.events.len(), 1);
        assert!(read_events(&temp_dir("none"), 5).unwrap().restarted);
    }

    /// A review directory whose log holds `events`.
    fn seeded(name: &str, events: &[Event]) -> PathBuf {
        let dir = temp_dir(name);
        ensure_dir(&dir).unwrap();
        let lines = events
            .iter()
            .map(|event| serde_json::to_string(event).unwrap() + "\n")
            .collect::<String>();
        std::fs::write(dir.join(REVIEW_FILE), lines).unwrap();
        dir
    }

    fn archived(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(ARCHIVE_FILE)).unwrap_or_default()
    }

    fn events_of(text: &str) -> Vec<Event> {
        text.lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn roots(review: &Review) -> Vec<&str> {
        review.threads.iter().map(|t| t.root.id.as_str()).collect()
    }

    #[test]
    fn resolved_threads_leave_the_log_and_their_events_are_in_the_archive() {
        let user = Author::User;
        let dir = seeded(
            "archive",
            &[
                root(&user, "u1"),
                root(&user, "u2"),
                reply(&agent(), "a1", "u1"),
                sent(&["u1", "u2"], "b1"),
                resolve(&user, "u1"),
            ],
        );
        let done = archive(&dir, "now").unwrap();
        assert_eq!(
            done,
            Archived {
                threads: 1,
                unsent: 0
            }
        );
        let review = read(&dir).unwrap();
        assert_eq!(roots(&review), ["u2"]);
        // The send carried both threads. The part that stays still marks u2 sent.
        assert_eq!(thread(&review, "u2").root.sent_batch, BatchId::parse("b1"));
        assert_eq!(review.skipped_lines, 0);
        let moved = fold(&events_of(&archived(&dir)));
        assert_eq!(roots(&moved), ["u1"]);
        let u1 = thread(&moved, "u1");
        assert_eq!(u1.replies.len(), 1);
        assert_eq!(u1.root.at, "2026-10-04T00:00:00Z");
        assert_eq!(u1.root.sent_batch, BatchId::parse("b1"));
        assert!(!u1.is_open());
        let mode = std::fs::metadata(dir.join(ARCHIVE_FILE)).unwrap();
        assert_eq!(mode.permissions().mode() & 0o777, 0o600);
        assert!(
            !dir.join(format!("{REVIEW_FILE}.{}.tmp", std::process::id()))
                .exists()
        );
    }

    #[test]
    fn open_threads_and_new_resolved_threads_stay_and_nothing_is_written() {
        let dir = seeded(
            "archive-none",
            &[
                root(&Author::User, "u1"),
                root(&Author::User, "u2"),
                resolve(&agent(), "u2"),
            ],
        );
        let before = log(&dir);
        assert_eq!(archive(&dir, "now"), Ok(Archived::default()));
        assert_eq!(log(&dir), before);
        assert!(!dir.join(ARCHIVE_FILE).exists());
        // An empty review has nothing to archive either.
        let empty = temp_dir("archive-empty");
        assert_eq!(archive(&empty, "now"), Ok(Archived::default()));
        assert!(!empty.join(REVIEW_FILE).exists());
    }

    #[test]
    fn an_agent_s_resolve_is_archived_once_it_was_seen() {
        let seen = event(&Author::User, Kind::Seen { id: id("u1") });
        let dir = seeded(
            "archive-seen",
            &[root(&Author::User, "u1"), resolve(&agent(), "u1"), seen],
        );
        assert_eq!(archive(&dir, "now").unwrap().threads, 1);
        assert_eq!(archived(&dir).lines().count(), 3);
    }

    #[test]
    fn an_unsent_resolved_thread_is_archived_and_counted() {
        let user = Author::User;
        let dir = seeded(
            "archive-unsent",
            &[
                root(&user, "u1"),
                resolve(&user, "u1"),
                root(&user, "u2"),
                sent(&["u2"], "b1"),
                resolve(&user, "u2"),
            ],
        );
        let done = archive(&dir, "now").unwrap();
        assert_eq!(
            done,
            Archived {
                threads: 2,
                unsent: 1
            }
        );
        assert!(read(&dir).unwrap().threads.is_empty());
    }

    #[test]
    fn the_next_ids_after_an_archive_are_above_every_archived_one() {
        let user = Author::User;
        let mut events = Vec::new();
        for n in 1..=5 {
            events.push(root(&user, &format!("u{n}")));
            events.push(resolve(&user, &format!("u{n}")));
        }
        events.extend([
            root(&agent(), "a1"),
            reply(&agent(), "a2", "a1"),
            resolve(&user, "a1"),
            sent(&["u1"], "b1"),
            sent(&["u2"], "b2"),
        ]);
        let dir = seeded("archive-ids", &events);
        assert_eq!(archive(&dir, "now").unwrap().threads, 6);
        // What is left is one line, which carries the counters.
        assert_eq!(log(&dir).lines().count(), 1, "{}", log(&dir));
        // `read` is what a pane does when it starts again.
        let mut ids = read(&dir).unwrap().ids;
        assert_eq!(ids.comment(&user).as_str(), "u6");
        assert_eq!(ids.comment(&agent()).as_str(), "a3");
        assert_eq!(ids.batch().as_str(), "b3");
        assert_eq!(write(&dir, "t", add_root), Ok(id("u6")));
        // A second archive carries them on, in one line that replaces the first.
        write(&dir, "t", |_, _| {
            Ok::<_, ()>((vec![resolve(&user, "u6")], ()))
        })
        .unwrap();
        assert_eq!(archive(&dir, "later").unwrap().threads, 1);
        assert_eq!(log(&dir).lines().count(), 1, "{}", log(&dir));
        assert!(log(&dir).contains("\"at\":\"later\""));
        let mut ids = read(&dir).unwrap().ids;
        assert_eq!(ids.comment(&user).as_str(), "u7");
        assert_eq!(ids.comment(&agent()).as_str(), "a3");
        assert_eq!(ids.batch().as_str(), "b3");
    }

    #[test]
    fn a_second_archive_appends_to_the_archive() {
        let user = Author::User;
        let dir = seeded("archive-twice", &[root(&user, "u1"), resolve(&user, "u1")]);
        archive(&dir, "now").unwrap();
        let first = archived(&dir);
        write(&dir, "t", |_, _| {
            Ok::<_, ()>((vec![root(&user, "u2"), resolve(&user, "u2")], ()))
        })
        .unwrap();
        archive(&dir, "now").unwrap();
        assert!(archived(&dir).starts_with(&first));
        assert_eq!(roots(&fold(&events_of(&archived(&dir)))), ["u1", "u2"]);
    }

    #[test]
    fn a_deleted_reply_goes_with_its_thread_and_a_line_that_does_not_parse_stays() {
        let user = Author::User;
        let dir = seeded(
            "archive-lines",
            &[
                root(&user, "u1"),
                reply(&user, "u2", "u1"),
                event(&user, Kind::Delete { id: id("u2") }),
                resolve(&user, "u1"),
            ],
        );
        let mut text = log(&dir);
        text.push_str("not json\n{\"cut");
        std::fs::write(dir.join(REVIEW_FILE), text).unwrap();
        assert_eq!(archive(&dir, "now").unwrap().threads, 1);
        assert_eq!(archived(&dir).lines().count(), 4);
        let left = log(&dir);
        assert_eq!(left.lines().count(), 3, "{left}");
        assert!(left.ends_with("not json\n{\"cut\n"), "{left}");
        assert_eq!(read(&dir).unwrap().skipped_lines, 2);
    }

    #[test]
    fn an_archive_under_a_held_lock_is_busy_and_changes_neither_file() {
        let user = Author::User;
        let dir = seeded("archive-busy", &[root(&user, "u1"), resolve(&user, "u1")]);
        let before = log(&dir);
        let held = lock(&dir).unwrap();
        assert_eq!(archive(&dir, "now"), Err(StoreError::Busy));
        assert_eq!(log(&dir), before);
        assert!(!dir.join(ARCHIVE_FILE).exists());
        drop(held);
        assert_eq!(archive(&dir, "now").unwrap().threads, 1);
    }

    #[test]
    fn an_archive_that_cannot_be_appended_to_leaves_the_log_as_it_was() {
        let user = Author::User;
        let dir = seeded("archive-io", &[root(&user, "u1"), resolve(&user, "u1")]);
        let before = log(&dir);
        std::fs::create_dir(dir.join(ARCHIVE_FILE)).unwrap();
        let error = archive(&dir, "now").unwrap_err();
        assert!(matches!(error, StoreError::Io { path, .. } if path.ends_with(ARCHIVE_FILE)));
        assert_eq!(log(&dir), before);
    }

    #[test]
    fn a_failed_rename_leaves_the_log_as_it_was_and_the_thread_in_both_files() {
        let user = Author::User;
        let dir = seeded("archive-rename", &[root(&user, "u1"), resolve(&user, "u1")]);
        let before = log(&dir);
        // The lock, the archive and the temp file exist, so each opens in a directory that
        // takes no new name. Only the rename needs to write the directory.
        drop(lock(&dir).unwrap());
        std::fs::write(dir.join(ARCHIVE_FILE), "").unwrap();
        let temp = dir.join(format!("{REVIEW_FILE}.{}.tmp", std::process::id()));
        std::fs::write(&temp, "").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let error = archive(&dir, "now").unwrap_err();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(error, StoreError::Io { path, .. } if path.ends_with(REVIEW_FILE)));
        assert_eq!(log(&dir), before);
        assert_eq!(roots(&read(&dir).unwrap()), ["u1"]);
        assert_eq!(archived(&dir).lines().count(), 2);
        // The next archive moves it for good. The archive then holds its events twice, and a
        // fold of them is still one thread.
        assert_eq!(archive(&dir, "now").unwrap().threads, 1);
        assert!(read(&dir).unwrap().threads.is_empty());
        assert_eq!(archived(&dir).lines().count(), 4);
        let moved = fold(&events_of(&archived(&dir)));
        assert_eq!(roots(&moved), ["u1"]);
        assert!(!thread(&moved, "u1").is_open());
    }
}
