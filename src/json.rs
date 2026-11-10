//! The JSON `state` and `file` print (design/shapes.md). Types and the schema number, nothing else.
//!
//! `null` is written and never left out, so a decoder checks a field and not its presence. The
//! types derive `Deserialize` so a test can decode a reply into the shapes the mod decodes.

use serde::{Deserialize, Serialize};

/// Raised by any change a decoder would notice. A field a decoder may ignore does not raise it.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeJson {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Binary,
    Submodule,
    TooLarge,
    Unparsed,
}

/// What `archive --json` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveJson {
    pub schema: u32,
    pub archived: usize,
    pub unsent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateJson {
    pub schema: u32,
    pub root: String,
    pub store_dir: String,
    pub spec: String,
    pub base: Option<String>,
    pub rev: String,
    pub notices: Vec<String>,
    pub warnings: Vec<String>,
    /// What `archive` would move now, orphans included. See `Thread::archivable`.
    pub archivable: usize,
    /// Resolved threads `archive` leaves only because the agent's resolve is unseen. See
    /// `Thread::unread_resolved`.
    pub unread_resolved: usize,
    pub files: Vec<FileSummary>,
    pub orphans: Vec<ThreadJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSummary {
    pub path: String,
    pub old_path: Option<String>,
    pub change: ChangeJson,
    pub add: usize,
    pub del: usize,
    /// Changes whenever anything drawn for the file does, its rows or its cards. See `DiffFile::content_hash`.
    pub hash: String,
    pub viewed: bool,
    pub threads: usize,
    pub open: usize,
    pub new: usize,
    pub unsent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileJson {
    pub schema: u32,
    pub path: String,
    pub old_path: Option<String>,
    pub change: ChangeJson,
    pub add: usize,
    pub del: usize,
    pub viewed: bool,
    pub note: Option<String>,
    pub rows: Vec<RowJson>,
    pub threads: Vec<ThreadJson>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowType {
    Hunk,
    Ctx,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowJson {
    pub kind: RowType,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    /// The coloured parts of `text`. Left out, not `null` or `[]`, when there are none: a build
    /// without the `syntax` feature, a language the highlighter does not know, a hunk header.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<TokenJson>,
}

/// What a piece of code is, as far as its colour goes: the kinds of `theme::Token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    Operator,
    Function,
    Type,
}

/// One coloured part of a row's `text`, written as `[start, end, kind]`. `start` and `end` count
/// UTF-16 code units, so a JavaScript `text.slice(start, end)` is the part. The parts of a row come
/// in order, do not overlap, and are not empty.
pub type TokenJson = (usize, usize, TokenKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    Matched,
    Outdated,
    NotInDiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementJson {
    pub kind: PlacementKind,
    pub row: Option<usize>,
    pub start_row: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorKind {
    Line,
    Range,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideJson {
    Old,
    New,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorJson {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: AnchorKind,
    pub side: Option<SideJson>,
    pub line: Option<u32>,
    pub end_line: Option<u32>,
    pub text: Option<String>,
    pub spec: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusJson {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadJson {
    pub id: String,
    pub status: StatusJson,
    pub resolved_by: Option<String>,
    pub is_new: bool,
    pub unsent: bool,
    pub placement: PlacementJson,
    pub anchor: AnchorJson,
    pub comments: Vec<CommentJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentJson {
    pub id: String,
    pub parent: Option<String>,
    pub author: String,
    pub at: String,
    pub body: String,
    pub sent: bool,
    pub edited_since_sent: bool,
}

/// The agent a send reached, tagged by backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "lowercase")]
pub enum TargetJson {
    Hc {
        address: String,
        agent: String,
    },
    Herdr {
        pane_id: String,
        terminal_id: String,
        agent: String,
    },
}

/// What `send --json` prints when a send happened or there was nothing to send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendJson {
    pub schema: u32,
    pub sent: usize,
    pub queued: bool,
    pub agent: Option<String>,
    pub target: Option<TargetJson>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendErrorKind {
    Ambiguous,
    NoAgent,
    Blocked,
    NotReady,
    AgentGone,
    Busy,
    Backend,
}

/// An agent the user can pick when several match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateJson {
    pub address: String,
    pub agent: String,
    pub cwd: String,
    pub status: String,
}

/// What `send --json` prints when the send failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendErrorJson {
    pub schema: u32,
    pub error: SendErrorKind,
    pub message: String,
    pub candidates: Vec<CandidateJson>,
}
