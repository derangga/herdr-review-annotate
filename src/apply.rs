//! `comment apply`: decode a batch from stdin, check every entry, read the anchored lines, and
//! append the whole batch in one write.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::comment::{CommandError, check_body};
use crate::diff::GitError;
use crate::store::{Add, Author, CommentId, Event, Kind, RelPath, Side, Spec, WriteError, write};

/// The most comments one batch may hold.
pub const MAX_BATCH: usize = 200;

/// The input of one entry, before it is checked. The agent supplies no line text.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: Option<String>,
    side: Option<Side>,
    line: Option<u32>,
    end_line: Option<u32>,
    reply_to: Option<String>,
    body: String,
}

/// Where a root comment points, within one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub side: Side,
    pub line: u32,
    /// Set only when the comment covers more than one line.
    pub end_line: Option<u32>,
}

/// An entry that passed every check that needs no file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewComment {
    Reply {
        reply_to: String,
        body: String,
    },
    Root {
        path: RelPath,
        lines: Option<Lines>,
        body: String,
    },
}

/// A `NewComment` with the text of its first anchored line read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Reply {
        reply_to: String,
        body: String,
    },
    Root {
        path: RelPath,
        line: Option<(Lines, String)>,
        body: String,
    },
}

/// What stops a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The batch is wrong. Exit code 2.
    Invalid(CommandError),
    /// `git` could not answer. Exit code 1.
    Git(GitError),
}

impl From<CommandError> for ApplyError {
    fn from(error: CommandError) -> Self {
        Self::Invalid(error)
    }
}

pub fn invalid(index: Option<usize>, why: impl Into<String>) -> CommandError {
    CommandError::InvalidBatch {
        index,
        why: why.into(),
    }
}

fn check_entry(index: usize, value: Value) -> Result<NewComment, CommandError> {
    let entry = serde_json::from_value::<Entry>(value)
        .map_err(|error| invalid(Some(index), error.to_string()))?;
    let bad = |why: &str| invalid(Some(index), why);
    check_body(&entry.body).map_err(|error| bad(&error.to_string()))?;
    if let Some(reply_to) = entry.reply_to {
        let anchored = entry.path.is_some()
            || entry.side.is_some()
            || entry.line.is_some()
            || entry.end_line.is_some();
        if anchored {
            return Err(bad("a reply has only reply_to and body"));
        }
        return Ok(NewComment::Reply {
            reply_to,
            body: entry.body,
        });
    }
    let path = entry.path.ok_or_else(|| bad("path is missing"))?;
    let path = RelPath::parse(&path)
        .ok_or_else(|| bad("path must be relative to the repository and hold no .."))?;
    let lines = match (entry.line, entry.side, entry.end_line) {
        (None, None, None) => None,
        (None, ..) => return Err(bad("side and end_line need a line")),
        (Some(0), ..) => return Err(bad("line must be 1 or more")),
        (Some(line), side, end) => match end {
            Some(end) if end < line => return Err(bad("end_line is before line")),
            _ => Some(Lines {
                side: side.unwrap_or(Side::New),
                line,
                end_line: end.filter(|end| *end > line),
            }),
        },
    };
    Ok(NewComment::Root {
        path,
        lines,
        body: entry.body,
    })
}

/// Decode `{"comments":[...]}` and check every entry. The first bad entry rejects the batch and
/// names its index, counted from 0.
pub fn decode(text: &str) -> Result<Vec<NewComment>, CommandError> {
    let batch = serde_json::from_str::<Value>(text)
        .map_err(|error| invalid(None, format!("stdin is not JSON: {error}")))?;
    let Some(Value::Array(entries)) = batch.get("comments").cloned() else {
        return Err(invalid(None, r#"expected {"comments":[...]}"#));
    };
    match entries.len() {
        0 => return Err(invalid(None, "the batch holds no comments")),
        n if n > MAX_BATCH => {
            return Err(invalid(
                None,
                format!("the batch holds {n} comments, the limit is {MAX_BATCH}"),
            ));
        }
        _ => {}
    }
    entries
        .into_iter()
        .enumerate()
        .map(|(index, value)| check_entry(index, value))
        .collect()
}

/// The text of one file on one side: the worktree file, or the file at the diff's base.
struct Files<'a, G> {
    root: &'a Path,
    spec: &'a Spec,
    git: G,
    base: Option<String>,
    cache: HashMap<(Side, String), Result<String, String>>,
}

impl<G: FnMut(&[String]) -> Result<String, GitError>> Files<'_, G> {
    /// The revision the old side is read from: `HEAD`, or the merge base of the branch spec.
    fn revision(&mut self) -> Result<String, GitError> {
        if let Some(base) = &self.base {
            return Ok(base.clone());
        }
        let revision = match self.spec {
            Spec::WorkTree => "HEAD".to_owned(),
            Spec::Branch { base } => {
                let args = [
                    "-C",
                    &self.root.to_string_lossy(),
                    "merge-base",
                    base,
                    "HEAD",
                ]
                .map(str::to_owned);
                (self.git)(&args)?.trim().to_owned()
            }
        };
        self.base = Some(revision.clone());
        Ok(revision)
    }

    /// The file's text, or why it cannot be read. A `git` that is missing is an error.
    fn text(&mut self, side: Side, path: &RelPath) -> Result<Result<String, String>, GitError> {
        let key = (side, path.to_string());
        if let Some(found) = self.cache.get(&key) {
            return Ok(found.clone());
        }
        let found = match side {
            Side::New => std::fs::read(self.root.join(path.as_str()))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("cannot read {path} in the worktree: {error}")),
            Side::Old => {
                let revision = self.revision()?;
                let args = [
                    "-C".to_owned(),
                    self.root.to_string_lossy().into_owned(),
                    "show".to_owned(),
                    format!("{revision}:{path}"),
                ];
                match (self.git)(&args) {
                    Ok(text) => Ok(text),
                    Err(GitError::Failed { .. }) => {
                        Err(format!("{path} does not exist in {revision}"))
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        self.cache.insert(key, found.clone());
        Ok(found)
    }
}

/// Read the first anchored line of every root comment. A line that is not in the file rejects the
/// batch. A trailing `\r` is not part of the text.
pub fn read_lines(
    comments: Vec<NewComment>,
    root: &Path,
    spec: &Spec,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
) -> Result<Vec<Ready>, ApplyError> {
    let mut files = Files {
        root,
        spec,
        git,
        base: None,
        cache: HashMap::new(),
    };
    let mut ready = Vec::with_capacity(comments.len());
    for (index, comment) in comments.into_iter().enumerate() {
        ready.push(match comment {
            NewComment::Reply { reply_to, body } => Ready::Reply { reply_to, body },
            NewComment::Root {
                path,
                lines: None,
                body,
            } => Ready::Root {
                path,
                line: None,
                body,
            },
            NewComment::Root {
                path,
                lines: Some(lines),
                body,
            } => {
                let text = files
                    .text(lines.side, &path)
                    .map_err(ApplyError::Git)?
                    .map_err(|why| invalid(Some(index), why))?;
                let count = text.lines().count();
                let last = lines.end_line.unwrap_or(lines.line);
                if last as usize > count {
                    return Err(invalid(
                        Some(index),
                        format!("line {last} is past the end of {path} ({count} lines)"),
                    )
                    .into());
                }
                let first = text
                    .lines()
                    .nth(lines.line as usize - 1)
                    .unwrap_or_default()
                    .to_owned();
                Ready::Root {
                    path,
                    line: Some((lines, first)),
                    body,
                }
            }
        });
    }
    Ok(ready)
}

/// Append the batch. Replies are checked against the review under the lock. Returns the new ids in
/// batch order. An unknown `reply_to` rejects the whole batch.
pub fn append(
    dir: &Path,
    now: &str,
    by: &Author,
    spec: &Spec,
    comments: Vec<Ready>,
) -> Result<Vec<CommentId>, WriteError<CommandError>> {
    write(dir, now, |review, now| {
        let mut ids = review.ids;
        let mut events = Vec::with_capacity(comments.len());
        let mut new_ids = Vec::with_capacity(comments.len());
        for (index, comment) in comments.into_iter().enumerate() {
            let id = ids.comment(by);
            let add = match comment {
                Ready::Reply { reply_to, body } => {
                    let parent = CommentId::parse(&reply_to)
                        .filter(|id| review.thread(id).is_some())
                        .ok_or_else(|| {
                            let open = review
                                .open_ids()
                                .iter()
                                .map(|id| id.as_str())
                                .collect::<Vec<_>>();
                            invalid(
                                Some(index),
                                format!(
                                    "reply_to {reply_to} is not a thread, open threads: {}",
                                    open.join(" ")
                                ),
                            )
                        })?;
                    Add {
                        id: id.clone(),
                        parent: Some(parent),
                        path: None,
                        old_path: None,
                        side: None,
                        line: None,
                        end_line: None,
                        line_text: None,
                        spec: None,
                        body,
                    }
                }
                Ready::Root { path, line, body } => {
                    let (side, line, end_line, line_text) = match line {
                        Some((lines, text)) => (
                            Some(lines.side),
                            Some(lines.line),
                            lines.end_line,
                            Some(text.trim_end_matches('\r').to_owned()),
                        ),
                        None => (None, None, None, None),
                    };
                    Add {
                        id: id.clone(),
                        parent: None,
                        path: Some(path),
                        old_path: None,
                        side,
                        line,
                        end_line,
                        line_text,
                        spec: Some(spec.clone()),
                        body,
                    }
                }
            };
            events.push(Event {
                at: now.to_owned(),
                by: by.clone(),
                kind: Kind::Add(add),
            });
            new_ids.push(id);
        }
        Ok((events, new_ids))
    })
}
