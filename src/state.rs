//! `state` and `file`: the reads the herdr-app panel makes (design/graphs/json.md).
//!
//! Both print one JSON object and write nothing: no event, no `meta.json`, no lock and no state
//! directory. The panel watches the repo root and the state directory, so a write here would wake
//! it and it would read again.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::cli::{Failure, Output};
use crate::diff::{
    Change, Diff, DiffFile, GitError, Placement, RepoRoot, Row, RowKind, default_base,
    load as load_diff, place,
};
use crate::env::Env;
use crate::json::{
    AnchorJson, AnchorKind, ChangeJson, CommentJson, FileJson, FileSummary, FilesJson, MarkJson,
    PlacementJson, PlacementKind, RowJson, RowType, SCHEMA, SideJson, StateJson, StatusJson,
    ThreadJson, TokenJson, TokenKind,
};
use crate::meta::{Meta, load as load_meta, locate};
use crate::store::{
    AnchorTarget, Comment, RelPath, Review, Side, Spec, Status, Thread, Warning, fold, read_events,
};
use crate::syntax::{Cache, FileTokens, Span, side_of};
use crate::theme::Token;
use crate::words::{Marks, changed};

/// `--spec`: which diff the caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecChoice {
    WorkTree,
    Branch,
}

/// What `state` and `file` are asked: the flags both take.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub repo: Option<PathBuf>,
    pub spec: Option<SpecChoice>,
    pub base: Option<String>,
}

/// Everything the two commands read before they print.
struct Loaded {
    root: RepoRoot,
    dir: PathBuf,
    meta: Meta,
    diff: Diff,
    review: Review,
    base: Option<String>,
    notices: Vec<String>,
    warnings: Vec<String>,
}

/// The spec to read, the base to offer for the branch view, and a notice when the spec asked for
/// could not be had. `--spec` and `--base` win over `meta.json`, which wins over the working tree.
/// A base alone means the branch spec. A branch spec with no base uses the saved one, else the
/// default base, and with none of them the working tree is read and a notice says so.
pub fn choose(
    query: &Query,
    meta: &Meta,
    root: &Path,
    git: &mut impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<(Spec, Option<String>, Option<String>), GitError> {
    let saved = meta.base.clone().or_else(|| match &meta.spec {
        Some(Spec::Branch { base }) => Some(base.clone()),
        _ => None,
    });
    let asked = query.base.clone().or(saved);
    let branch = |base: String| (Spec::Branch { base: base.clone() }, Some(base), None);
    match (query.spec, &query.base) {
        (None, None) => {
            let spec = meta.spec.clone().unwrap_or(Spec::WorkTree);
            let base = asked.or_else(|| default_base(root, &mut *git).ok());
            Ok((spec, base, None))
        }
        (Some(SpecChoice::WorkTree), _) => {
            let base = asked.or_else(|| default_base(root, &mut *git).ok());
            Ok((Spec::WorkTree, base, None))
        }
        _ => match asked.map_or_else(|| default_base(root, &mut *git), Ok) {
            Ok(base) => Ok(branch(base)),
            Err(error @ GitError::NoBase { .. }) => Ok((
                Spec::WorkTree,
                None,
                Some(format!("{error}, showing the working tree against HEAD")),
            )),
            Err(error) => Err(error),
        },
    }
}

fn read(
    query: &Query,
    env: &Env,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Loaded, Failure> {
    let root = {
        let mut text =
            |args: &[String]| git(args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        RepoRoot::resolve(query.repo.as_deref(), &env.cwd, &mut text)?
    };
    let state_base = env.state_base().ok_or(Failure::NoStateDir)?;
    let dir = locate(&state_base, root.path());
    let (meta, meta_warning) = load_meta(&dir);
    let (spec, base, spec_notice) = choose(query, &meta, root.path(), &mut git)?;
    let diff = load_diff(root.path(), &spec, &mut git)?;
    // A last line with no newline is a writer in the middle of an append. It is not counted.
    let chunk = read_events(&dir, 0)?;
    let review = fold(&chunk.events);
    let warnings = meta_warning
        .into_iter()
        .chain((chunk.skipped > 0).then_some(Warning::SkippedLine(chunk.skipped)))
        .map(|warning| warning.to_string())
        .collect();
    let notices = spec_notice
        .into_iter()
        .chain(diff.notices.iter().map(ToString::to_string))
        .collect();
    Ok(Loaded {
        root,
        dir,
        meta,
        diff,
        review,
        base,
        notices,
        warnings,
    })
}

const fn change_json(change: Change) -> ChangeJson {
    match change {
        Change::Modified => ChangeJson::Modified,
        Change::Added => ChangeJson::Added,
        Change::Deleted => ChangeJson::Deleted,
        Change::Renamed => ChangeJson::Renamed,
        Change::Untracked => ChangeJson::Untracked,
        Change::Binary => ChangeJson::Binary,
        Change::Submodule => ChangeJson::Submodule,
        Change::TooLarge => ChangeJson::TooLarge,
        Change::Unparsed => ChangeJson::Unparsed,
    }
}

/// Why a file has no rows. Only the four kinds that are listed and not rendered have one.
const fn note(change: Change) -> Option<&'static str> {
    match change {
        Change::Binary => Some("binary file"),
        Change::TooLarge => Some("too large to show"),
        Change::Submodule => Some("submodule"),
        Change::Unparsed => Some("could not be parsed"),
        _ => None,
    }
}

/// The most lines of one file that `file` has the highlighter parse, new side first and then the old
/// side. Parsing takes 50 to 110 microseconds a line in a release build, so this is under half a
/// second at most.
const MAX_PARSED_LINES: usize = 4_000;

const fn token_kind(token: Token) -> TokenKind {
    match token {
        Token::Comment => TokenKind::Comment,
        Token::String => TokenKind::String,
        Token::Number => TokenKind::Number,
        Token::Keyword => TokenKind::Keyword,
        Token::Operator => TokenKind::Operator,
        Token::Function => TokenKind::Function,
        Token::Type => TokenKind::Type,
    }
}

/// Byte ranges of one line as the JSON writes them: UTF-16 offsets in place of bytes, in order. A
/// range that does not fall on characters of `text`, or is empty, is left out.
fn utf16<T>(text: &str, ranges: impl Iterator<Item = (Range<usize>, T)>) -> Vec<(usize, usize, T)> {
    let (mut byte, mut unit) = (0, 0);
    let mut advance = |to: usize| {
        unit += text.get(byte..to)?.encode_utf16().count();
        byte = to;
        Some(unit)
    };
    ranges
        .filter_map(|(range, with)| {
            let from = advance(range.start)?;
            Some((from, advance(range.end)?, with))
        })
        .filter(|(from, to, _)| from < to)
        .collect()
}

fn tokens_json(text: &str, spans: &[Span]) -> Vec<TokenJson> {
    utf16(
        text,
        spans
            .iter()
            .map(|(range, token)| (range.clone(), token_kind(*token))),
    )
}

fn marks_json(text: &str, marks: &Marks) -> Vec<MarkJson> {
    utf16(text, marks.iter().map(|range| (range.clone(), ())))
        .into_iter()
        .map(|(from, to, ())| MarkJson { from, to })
        .collect()
}

/// The marks of each row of a hunk: a run of removed rows pairs with the run of added rows right
/// after it, the first with the first, as the TUI does, and `changed` says what to mark. A row that
/// is no pair, or whose pair gets no marks, has none.
fn hunk_marks(rows: &[Row]) -> Vec<Marks> {
    let mut out = vec![Marks::new(); rows.len()];
    let mut at = 0;
    while at < rows.len() {
        let run = |from: usize, kind| {
            rows.iter()
                .skip(from)
                .take_while(|row| row.kind == kind)
                .count()
        };
        let removed = run(at, RowKind::Removed);
        let added = run(at + removed, RowKind::Added);
        for k in 0..removed.min(added) {
            let (old, new) = (at + k, at + removed + k);
            if let (Some(a), Some(b)) = (rows.get(old), rows.get(new))
                && let Some((was, now)) = changed(&a.text, &b.text)
            {
                for (at, marks) in [(old, was), (new, now)] {
                    if let Some(slot) = out.get_mut(at) {
                        *slot = marks;
                    }
                }
            }
        }
        at += (removed + added).max(1);
    }
    out
}

/// The hunk header, then the rows of each hunk, in order, with the tokens `tokens` has for them. A
/// file that is listed and not rendered has none.
fn rows(file: &DiffFile, tokens: &FileTokens) -> Vec<RowJson> {
    if note(file.change).is_some() {
        return Vec::new();
    }
    let row = |(row, marks): (&Row, Marks)| RowJson {
        kind: match row.kind {
            RowKind::Context => RowType::Ctx,
            RowKind::Added => RowType::Add,
            RowKind::Removed => RowType::Del,
        },
        old: row.old,
        new: row.new,
        text: row.text.clone(),
        tokens: row
            .line(side_of(row))
            .and_then(|line| tokens.line(side_of(row), line))
            .map_or_else(Vec::new, |spans| tokens_json(&row.text, spans)),
        marks: marks_json(&row.text, &marks),
    };
    file.hunks
        .iter()
        .flat_map(|hunk| {
            let header = RowJson {
                kind: RowType::Hunk,
                old: None,
                new: None,
                text: hunk.header.clone(),
                tokens: Vec::new(),
                marks: Vec::new(),
            };
            let marks = hunk_marks(&hunk.rows);
            std::iter::once(header).chain(hunk.rows.iter().zip(marks).map(row))
        })
        .collect()
}

fn line_on(row: &RowJson, side: Side) -> Option<u32> {
    match side {
        Side::Old => row.old,
        Side::New => row.new,
    }
}

/// Where a thread's card goes in `rows`. A matched line is drawn under its row and a matched range
/// under the last row of its lines, with `start_row` at the first. An outdated thread is drawn
/// under the row nearest where it was written.
fn placement_json(thread: &Thread, placement: Placement, rows: &[RowJson]) -> PlacementJson {
    let side = thread.anchor.side();
    let first_of = |line: u32, last: u32| {
        let side = side?;
        let on = |row: &RowJson| line_on(row, side).is_some_and(|n| (line..=last).contains(&n));
        Some((rows.iter().position(on)?, rows.iter().rposition(on)?))
    };
    let (kind, row, start_row) = match placement {
        Placement::NotInDiff => (PlacementKind::NotInDiff, None, None),
        Placement::Outdated { near } => {
            let row = near.and_then(|line| first_of(line, line));
            (PlacementKind::Outdated, row.map(|(first, _)| first), None)
        }
        Placement::Matched { line: None } => (PlacementKind::Matched, None, None),
        Placement::Matched { line: Some(line) } => {
            let (end, range) = match &thread.anchor.target {
                AnchorTarget::Range { start, end, .. } => (line + end.saturating_sub(*start), true),
                _ => (line, false),
            };
            let found = first_of(line, end);
            let start = found.map(|(first, _)| first);
            (
                PlacementKind::Matched,
                found.map(|(_, last)| last),
                start.filter(|_| range),
            )
        }
    };
    PlacementJson {
        kind,
        row,
        start_row,
    }
}

fn comment_json(comment: &Comment) -> CommentJson {
    CommentJson {
        id: comment.id.to_string(),
        parent: comment.parent.as_ref().map(ToString::to_string),
        author: comment.author.to_string(),
        at: comment.at.clone(),
        body: comment.body.clone(),
        sent: comment.sent_batch.is_some(),
        edited_since_sent: comment.edited_since_sent,
    }
}

fn thread_json(thread: &Thread, placement: Placement, rows: &[RowJson]) -> ThreadJson {
    let anchor = &thread.anchor;
    let side = anchor.side().map(|side| match side {
        Side::Old => SideJson::Old,
        Side::New => SideJson::New,
    });
    let (kind, line, end_line, text) = match &anchor.target {
        AnchorTarget::Line { line, text, .. } => {
            (AnchorKind::Line, Some(*line), None, Some(text.clone()))
        }
        AnchorTarget::Range {
            start, end, text, ..
        } => (
            AnchorKind::Range,
            Some(*start),
            Some(*end),
            Some(text.clone()),
        ),
        AnchorTarget::File => (AnchorKind::File, None, None, None),
    };
    let (status, resolved_by) = match &thread.status {
        Status::Open => (StatusJson::Open, None),
        Status::Resolved { by } => (StatusJson::Resolved, Some(by.to_string())),
    };
    ThreadJson {
        id: thread.root.id.to_string(),
        status,
        resolved_by,
        is_new: thread.is_new,
        unsent: thread.unsent,
        placement: placement_json(thread, placement, rows),
        anchor: AnchorJson {
            path: anchor.path.to_string(),
            old_path: anchor.old_path.as_ref().map(ToString::to_string),
            kind,
            side,
            line,
            end_line,
            text,
            spec: anchor.spec.to_string(),
        },
        comments: thread.comments().map(comment_json).collect(),
    }
}

/// Each thread with where it lands, in the order of the review.
fn placed(loaded: &Loaded) -> impl Iterator<Item = (&Thread, Placement)> {
    loaded
        .review
        .threads
        .iter()
        .map(|thread| (thread, place(thread, &loaded.diff)))
}

fn viewed(loaded: &Loaded, file: &DiffFile) -> bool {
    loaded.meta.viewed.get(file.path.as_str()) == Some(&file.fingerprint())
}

fn print(value: &impl serde::Serialize) -> Output {
    let json = serde_json::to_string(value).unwrap_or_default();
    Output {
        stdout: format!("{json}\n"),
        ..Output::default()
    }
}

/// `state --json`: every file with its counts, and the threads that are in no file.
pub fn state(
    query: &Query,
    env: &Env,
    git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Output, Failure> {
    let loaded = read(query, env, git)?;
    let diff = &loaded.diff;
    let mut files = diff
        .files
        .iter()
        .map(|file| {
            let (add, del) = file.stat();
            FileSummary {
                path: file.path.to_string(),
                old_path: file.old_path.as_ref().map(ToString::to_string),
                change: change_json(file.change),
                add,
                del,
                hash: String::new(),
                viewed: viewed(&loaded, file),
                threads: 0,
                open: 0,
                new: 0,
                unsent: 0,
            }
        })
        .collect::<Vec<_>>();
    let mut orphans = Vec::new();
    // The cards each file draws, as JSON, so a reply or an edit that leaves the counts as they were still changes `hash`.
    let mut cards = vec![Vec::new(); files.len()];
    for (thread, placement) in placed(&loaded) {
        let index = match placement {
            Placement::NotInDiff => None,
            _ => diff.file_index(&thread.anchor.path),
        };
        let json = thread_json(thread, placement, &[]);
        match index.and_then(|index| Some((files.get_mut(index)?, cards.get_mut(index)?))) {
            Some((file, card)) => {
                file.threads += 1;
                file.open += usize::from(thread.is_open());
                file.new += usize::from(thread.is_new);
                file.unsent += usize::from(thread.unsent);
                card.extend(serde_json::to_vec(&json).unwrap_or_default());
            }
            None => orphans.push(json),
        }
    }
    for ((summary, file), card) in files.iter_mut().zip(&diff.files).zip(&cards) {
        summary.hash = file.content_hash(card);
    }
    Ok(print(&StateJson {
        schema: SCHEMA,
        root: loaded.root.path().display().to_string(),
        store_dir: loaded.dir.display().to_string(),
        spec: diff.spec.to_string(),
        base: loaded.base.clone(),
        rev: diff.rev.clone(),
        notices: loaded.notices.clone(),
        warnings: loaded.warnings.clone(),
        archivable: loaded
            .review
            .threads
            .iter()
            .filter(|thread| thread.archivable())
            .count(),
        unread_resolved: loaded
            .review
            .threads
            .iter()
            .filter(|thread| thread.unread_resolved())
            .count(),
        files,
        orphans,
    }))
}

/// `file --json --path P`: the rows of one file and the threads drawn in them.
pub fn file(
    query: &Query,
    path: &str,
    env: &Env,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Output, Failure> {
    let loaded = read(query, env, &mut git)?;
    Ok(print(&file_json(&loaded, path, &mut git)?))
}

/// `files --json --path P --path Q`: `file` for each path, from one read of the repo. One path that
/// is not in the diff fails the whole command.
pub fn files(
    query: &Query,
    paths: &[String],
    env: &Env,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Output, Failure> {
    let loaded = read(query, env, &mut git)?;
    let files = paths
        .iter()
        .map(|path| file_json(&loaded, path, &mut git))
        .collect::<Result<_, _>>()?;
    Ok(print(&FilesJson {
        schema: SCHEMA,
        files,
    }))
}

fn file_json(
    loaded: &Loaded,
    path: &str,
    git: &mut impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<FileJson, Failure> {
    let diff = &loaded.diff;
    // The path wins. The old path of a renamed file answers only when no file has the path.
    let wanted = RelPath::parse(path);
    let index = wanted.as_ref().and_then(|wanted| {
        let by_path = diff.files.iter().position(|file| file.path == *wanted);
        by_path.or_else(|| {
            let renamed = |file: &DiffFile| file.old_path.as_ref() == Some(wanted);
            diff.files.iter().position(renamed)
        })
    });
    let (index, file) = index
        .and_then(|index| Some((index, diff.files.get(index)?)))
        .ok_or_else(|| Failure::NotInDiff(path.to_owned()))?;
    let (add, del) = file.stat();
    // The same highlighter and limits as the pane: a side over 1 MiB is parsed by hunks. The pane
    // parses a bit at a time and this command in one go, so it stops after `MAX_PARSED_LINES`
    // lines of the file, and the rows it did not reach have no tokens. A file with no rows to show
    // is not read.
    let mut cache = Cache::default();
    if note(file.change).is_none() {
        let mut parsed = 0;
        let mut more = || {
            parsed += 1;
            parsed < MAX_PARSED_LINES
        };
        cache.step(loaded.root.path(), &diff.rev, file, git, &mut more);
    }
    let none = FileTokens::default();
    let rows = rows(file, cache.file(file.path.as_str()).unwrap_or(&none));
    let threads = placed(loaded)
        .filter(|(_, placement)| *placement != Placement::NotInDiff)
        .filter(|(thread, _)| diff.file_index(&thread.anchor.path) == Some(index))
        .map(|(thread, placement)| thread_json(thread, placement, &rows))
        .collect();
    Ok(FileJson {
        schema: SCHEMA,
        path: file.path.to_string(),
        old_path: file.old_path.as_ref().map(ToString::to_string),
        change: change_json(file.change),
        add,
        del,
        viewed: viewed(loaded, file),
        note: note(file.change).map(str::to_owned),
        rows,
        threads,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;
    use crate::json::SCHEMA;
    use crate::store::state_dir;
    use PlacementKind::{Matched, Outdated};

    const PATCH: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,4 +1,5 @@
 fn main() {
-    old();
+    new();
+    more();
     keep();
 }
diff --git a/img.png b/img.png
index 3333333..4444444 100644
Binary files a/img.png and b/img.png differ
diff --git a/src/old_b.rs b/src/b.rs
similarity index 100%
rename from src/old_b.rs
rename to src/b.rs
";

    /// A `git` that answers what `state` and `file` ask, a root and a state directory in a temp
    /// directory, and a review log the test writes line by line.
    struct World {
        env: Env,
        root: PathBuf,
        dir: PathBuf,
        patch: String,
        listing: Result<Vec<u8>, GitError>,
        /// The refs that resolve, for the default base.
        refs: Vec<&'static str>,
        merge_base: bool,
        top: Result<(), GitError>,
        diff_error: bool,
    }

    impl World {
        fn new(name: &str) -> Self {
            let home = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("herdr-review-state-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            let root = home.join("repo");
            std::fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let env = Env::new(
                [(
                    "XDG_STATE_HOME".to_owned(),
                    home.join("state").display().to_string(),
                )],
                root.clone(),
            );
            let dir = state_dir(&env.state_base().unwrap(), &root);
            Self {
                env,
                root,
                dir,
                patch: PATCH.to_owned(),
                listing: Ok(Vec::new()),
                refs: vec!["main"],
                merge_base: true,
                top: Ok(()),
                diff_error: false,
            }
        }

        fn git(&self, args: &[String]) -> Result<Vec<u8>, GitError> {
            let failed = || GitError::Failed {
                args: args.join(" "),
                stderr: "fatal".into(),
            };
            let words = args.iter().map(String::as_str).collect::<Vec<_>>();
            match words.as_slice() {
                [_, _, "rev-parse", "--show-toplevel"] => self
                    .top
                    .clone()
                    .map(|()| format!("{}\n", self.root.display()).into_bytes()),
                [_, _, "rev-parse", "--verify", "--quiet", "HEAD"] => Ok(b"abc\n".to_vec()),
                [_, _, "rev-parse", "--verify", "--quiet", name] => {
                    self.refs.contains(name).then(Vec::new).ok_or_else(failed)
                }
                [_, _, "merge-base", ..] => self
                    .merge_base
                    .then(|| b"mb123\n".to_vec())
                    .ok_or_else(failed),
                [_, _, "-c", _, "diff", ..] if self.diff_error => Err(failed()),
                [_, _, "-c", _, "diff", ..] => Ok(self.patch.clone().into_bytes()),
                [_, _, "ls-files", ..] => self.listing.clone(),
                _ => Err(failed()),
            }
        }

        fn state(&self, line: &str) -> Result<Output, Failure> {
            state(&query(line), &self.env, |args| self.git(args))
        }

        fn file(&self, line: &str, path: &str) -> Result<Output, Failure> {
            file(&query(line), path, &self.env, |args| self.git(args))
        }

        fn log(&self, text: &str) {
            std::fs::create_dir_all(&self.dir).unwrap();
            std::fs::write(self.dir.join("review.jsonl"), text).unwrap();
        }

        fn meta(&self, text: &str) {
            std::fs::create_dir_all(&self.dir).unwrap();
            std::fs::write(self.dir.join("meta.json"), text).unwrap();
        }
    }

    fn query(line: &str) -> Query {
        let args = line
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let args = ["state", "--json"]
            .map(str::to_owned)
            .into_iter()
            .chain(args)
            .collect::<Vec<_>>();
        match crate::cli::parse(&args) {
            Ok(crate::cli::Command::State { query }) => query,
            other => panic!("{other:?}"),
        }
    }

    fn state_of(output: &Output) -> StateJson {
        assert_eq!(output.code, 0);
        serde_json::from_str(&output.stdout).unwrap()
    }

    fn file_of(output: &Output) -> FileJson {
        serde_json::from_str(&output.stdout).unwrap()
    }

    fn add(id: &str, by: &str, rest: &str) -> String {
        format!(r#"{{"at":"2026-11-09T10:00:00Z","by":"{by}","kind":"add","id":"{id}",{rest}}}"#)
    }

    /// A user's comment on a line of `src/a.rs`, written against the working tree.
    fn on_line(id: &str, side: &str, line: u32, text: &str) -> String {
        add(
            id,
            "user",
            &format!(
                r#""path":"src/a.rs","side":"{side}","line":{line},"line_text":"{text}","spec":"worktree","body":"b""#
            ),
        )
    }

    fn lines(events: &[String]) -> String {
        events.iter().map(|line| line.clone() + "\n").collect()
    }

    #[test]
    fn a_reply_decodes_into_the_shapes_with_schema_1() {
        let world = World::new("decode");
        let state = state_of(&world.state("").unwrap());
        assert_eq!(state.schema, SCHEMA);
        assert_eq!(state.schema, 1);
        let paths = state
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["img.png", "src/a.rs", "src/b.rs"]);
        let file = file_of(&world.file("", "src/a.rs").unwrap());
        assert_eq!((file.schema, file.add, file.del), (1, 2, 1));
        assert!(world.state("").unwrap().stdout.ends_with("}\n"));
    }

    #[test]
    fn a_missing_json_flag_or_a_bad_spec_is_a_usage_error() {
        let args = |line: &str| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        for line in [
            "state",
            "state --repo /r",
            "state --json --spec staged",
            "state --json --bogus",
            "state --json extra",
            "file --json",
            "file --path a",
            "file --json --path",
        ] {
            let error = crate::cli::parse(&args(line)).unwrap_err();
            let output = Failure::Usage(error).output();
            assert_eq!(output.code, 2, "{line}");
            assert!(
                output.stdout.is_empty() && !output.stderr.is_empty(),
                "{line}"
            );
        }
        assert!(crate::cli::parse(&args("state --json --repo /r --spec branch --base x")).is_ok());
    }

    #[test]
    fn a_folder_that_is_not_a_repo_or_no_git_exits_1() {
        let mut world = World::new("not-a-repo");
        for error in [GitError::NotARepo, GitError::NotInstalled] {
            world.top = Err(error);
            for failure in [
                world.state("").unwrap_err(),
                world.file("", "src/a.rs").unwrap_err(),
            ] {
                assert_eq!(failure.output().code, 1);
            }
        }
    }

    #[test]
    fn no_home_and_no_state_home_exits_1() {
        let mut world = World::new("no-home");
        world.env = Env::new([], world.root.clone());
        for failure in [
            world.state("").unwrap_err(),
            world.file("", "src/a.rs").unwrap_err(),
        ] {
            assert_eq!(failure, Failure::NoStateDir);
            assert_eq!(failure.output().code, 1);
        }
    }

    #[test]
    fn a_missing_state_directory_is_an_empty_review_and_is_not_created() {
        let world = World::new("no-dir");
        let state = state_of(&world.state("").unwrap());
        assert!(state.orphans.is_empty() && state.warnings.is_empty());
        assert!(
            state
                .files
                .iter()
                .all(|f| (f.open, f.new, f.unsent) == (0, 0, 0))
        );
        assert_eq!(state.store_dir, world.dir.display().to_string());
        assert!(!world.dir.exists());
        // A directory with no log is the same.
        std::fs::create_dir_all(&world.dir).unwrap();
        assert!(state_of(&world.state("").unwrap()).warnings.is_empty());
        assert!(!world.dir.join("review.jsonl").exists());
    }

    #[test]
    fn an_unreadable_meta_json_is_an_empty_meta_with_a_warning() {
        let world = World::new("bad-meta");
        world.meta("{ not json");
        let state = state_of(&world.state("").unwrap());
        assert_eq!(state.warnings, [Warning::MetaUnreadable.to_string()]);
        assert_eq!(state.spec, "worktree");
        assert_eq!(
            std::fs::read_to_string(world.dir.join("meta.json")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn a_branch_spec_with_no_base_anywhere_reads_the_working_tree_with_a_notice() {
        let mut world = World::new("no-base");
        world.refs = Vec::new();
        let state = state_of(&world.state("--spec branch").unwrap());
        assert_eq!(state.spec, "worktree");
        assert_eq!(state.base, None);
        assert_eq!(state.notices.len(), 1);
        assert!(
            state.notices[0].starts_with("no base branch found, tried origin/HEAD, main, master")
        );
        assert!(!state.files.is_empty());
    }

    #[test]
    fn the_spec_comes_from_the_flags_then_meta_then_the_working_tree() {
        let world = World::new("choose");
        let spec = |line: &str| {
            let state = state_of(&world.state(line).unwrap());
            (state.spec, state.base)
        };
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(spec(""), ("worktree".into(), some("main")));
        world.meta(r#"{"spec":"branch:dev","base":"dev"}"#);
        assert_eq!(spec(""), ("branch:dev".into(), some("dev")));
        assert_eq!(spec("--spec worktree"), ("worktree".into(), some("dev")));
        assert_eq!(spec("--spec branch"), ("branch:dev".into(), some("dev")));
        assert_eq!(
            spec("--spec branch --base topic"),
            ("branch:topic".into(), some("topic"))
        );
        assert_eq!(spec("--base topic"), ("branch:topic".into(), some("topic")));
        world.meta(r#"{"spec":"worktree"}"#);
        assert_eq!(spec("--spec branch"), ("branch:main".into(), some("main")));
    }

    #[test]
    fn a_base_that_does_not_merge_keeps_its_name_and_reads_the_working_tree() {
        let mut world = World::new("bad-base");
        world.merge_base = false;
        let state = state_of(&world.state("--spec branch --base gone").unwrap());
        assert_eq!(
            (state.spec.as_str(), state.base.as_deref()),
            ("worktree", Some("gone"))
        );
        assert_eq!(state.notices.len(), 1);
        assert!(state.notices[0].starts_with("base gone not found"));
    }

    #[test]
    fn a_failed_diff_exits_1_with_what_git_said() {
        let mut world = World::new("diff-fails");
        world.diff_error = true;
        for failure in [
            world.state("").unwrap_err(),
            world.file("", "src/a.rs").unwrap_err(),
        ] {
            let output = failure.output();
            assert_eq!(output.code, 1);
            assert!(output.stderr.contains("fatal"), "{}", output.stderr);
        }
    }

    #[test]
    fn a_failed_ls_files_is_a_notice_and_the_tracked_files_are_still_there() {
        let mut world = World::new("ls-fails");
        world.listing = Err(GitError::Failed {
            args: "ls-files".into(),
            stderr: "fatal".into(),
        });
        let state = state_of(&world.state("").unwrap());
        assert_eq!(state.notices, ["untracked files not shown"]);
        assert_eq!(state.files.len(), 3);
    }

    #[test]
    fn a_bad_line_in_the_log_is_counted_in_the_warnings() {
        let world = World::new("bad-line");
        world.log(&format!(
            "{}\nnot json\n{{\"kind\":\n{}\n",
            on_line("u1", "new", 2, "    new();"),
            on_line("u2", "new", 3, "    more();")
        ));
        let state = state_of(&world.state("").unwrap());
        assert_eq!(state.warnings, ["2 unreadable events"]);
        let a = state.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!(a.open, 2);
    }

    #[test]
    fn a_last_line_with_no_newline_is_ignored_and_not_a_warning() {
        let world = World::new("half-line");
        let whole = on_line("u1", "new", 2, "    new();");
        let half = on_line("u2", "new", 3, "    more();");
        world.log(&format!("{whole}\n{}", &half[..half.len() - 10]));
        let state = state_of(&world.state("").unwrap());
        assert!(state.warnings.is_empty());
        let a = state.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!(a.open, 1);
    }

    #[test]
    fn a_thread_in_no_file_is_an_orphan_and_in_no_count() {
        let world = World::new("orphan");
        let gone = add(
            "u1",
            "user",
            r#""path":"src/gone.rs","spec":"worktree","body":"b""#,
        );
        let other_spec = add(
            "u2",
            "user",
            r#""path":"src/a.rs","spec":"branch:main","body":"b""#,
        );
        world.log(&lines(&[
            gone,
            other_spec,
            on_line("u3", "new", 2, "    new();"),
        ]));
        let state = state_of(&world.state("").unwrap());
        let ids = state
            .orphans
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["u1", "u2"]);
        assert!(
            state
                .orphans
                .iter()
                .all(|t| t.placement.kind == PlacementKind::NotInDiff
                    && (t.placement.row, t.placement.start_row) == (None, None))
        );
        let open = state.files.iter().map(|f| f.open).sum::<usize>();
        assert_eq!(open, 1);
    }

    #[test]
    fn the_counts_are_open_new_and_unsent_threads_of_each_file() {
        let world = World::new("counts");
        let mut events = vec![
            on_line("u1", "new", 2, "    new();"),
            on_line("u2", "new", 3, "    more();"),
            on_line("u3", "new", 99, "outdated text"),
            on_line("u4", "new", 4, "    keep();"),
        ];
        let at = r#""at":"2026-11-09T10:01:00Z""#;
        events.extend([
            // u1 sent, so not unsent.
            format!(r#"{{{at},"by":"user","kind":"sent","ids":["u1"],"batch":"b1"}}"#),
            // u2 resolved by the agent: closed, and new.
            format!(r#"{{{at},"by":"agent:claude","kind":"resolve","id":"u2"}}"#),
            // u4 resolved by the user: closed and not new.
            format!(r#"{{{at},"by":"user","kind":"resolve","id":"u4"}}"#),
        ]);
        world.log(&lines(&events));
        let state = state_of(&world.state("").unwrap());
        let a = state.files.iter().find(|f| f.path == "src/a.rs").unwrap();
        // Open: u1 and u3. New: u2. Unsent: u3 only (u1 was sent, u2 and u4 are resolved).
        assert_eq!((a.open, a.new, a.unsent), (2, 1, 1));
        // Every thread counts, the two closed ones too.
        assert_eq!(a.threads, 4);
        assert!(state.orphans.is_empty());
    }

    #[test]
    fn an_edit_that_keeps_the_counts_still_changes_the_files_hash() {
        let world = World::new("cards-hash");
        let summary = |world: &World| {
            let state = state_of(&world.state("").unwrap());
            let file = |path: &str| state.files.iter().find(|f| f.path == path).unwrap().clone();
            let (a, b) = (file("src/a.rs"), file("src/b.rs"));
            assert_eq!(b.threads, 1);
            (a.hash, (a.threads, a.open, a.new, a.unsent), b.hash)
        };
        let root = on_line("u1", "new", 2, "    new();");
        // A thread on src/b.rs, so its hash takes cards that do not change.
        let other = add(
            "u2",
            "user",
            r#""path":"src/b.rs","spec":"worktree","body":"b""#,
        );
        world.log(&lines(&[root.clone(), other.clone()]));
        let (before, counts, b_before) = summary(&world);
        let at = r#""at":"2026-11-09T10:01:00Z""#;
        let edit = format!(r#"{{{at},"by":"user","kind":"edit","id":"u1","body":"b2"}}"#);
        world.log(&lines(&[root, other, edit]));
        let (after, counts_after, b_after) = summary(&world);
        assert_eq!(counts, counts_after);
        assert_ne!(before, after);
        // A file whose cards did not change keeps its hash.
        assert_eq!(b_before, b_after);
    }

    #[test]
    fn archivable_counts_the_resolved_threads_whose_agent_resolve_was_seen() {
        let world = World::new("archivable");
        let gone = add(
            "u5",
            "user",
            r#""path":"src/gone.rs","spec":"worktree","body":"b""#,
        );
        let at = r#""at":"2026-11-09T10:01:00Z""#;
        let by = |who: &str, kind: &str, id: &str| {
            format!(r#"{{{at},"by":"{who}","kind":"{kind}","id":"{id}"}}"#)
        };
        world.log(&lines(&[
            // u1 stays open.
            on_line("u1", "new", 2, "    new();"),
            // u2: the user resolved it.
            on_line("u2", "new", 3, "    more();"),
            by("user", "resolve", "u2"),
            // u3: an agent resolved it and nobody looked, so it stays.
            on_line("u3", "new", 4, "    keep();"),
            by("agent:claude", "resolve", "u3"),
            // u4: an agent resolved it and the user looked.
            on_line("u4", "new", 99, "outdated text"),
            by("agent:claude", "resolve", "u4"),
            by("user", "seen", "u4"),
            // u5: resolved by the user, in no file.
            gone,
            by("user", "resolve", "u5"),
        ]));
        let state = state_of(&world.state("").unwrap());
        assert_eq!(state.archivable, 3);
        // Only u3: open threads and archivable ones are not unread, and the two numbers never share a thread.
        assert_eq!(state.unread_resolved, 1);
        let none = state_of(&World::new("archivable-none").state("").unwrap());
        assert_eq!((none.archivable, none.unread_resolved), (0, 0));
    }

    #[test]
    fn a_file_is_viewed_when_its_fingerprint_is_the_one_in_meta() {
        let world = World::new("viewed");
        let fingerprint = |path: &str| {
            let files = crate::diff::parse(PATCH.as_bytes(), crate::diff::MAX_PATCH);
            files
                .into_iter()
                .find(|f| f.path.as_str() == path)
                .unwrap()
                .fingerprint()
        };
        world.meta(&format!(
            r#"{{"viewed":{{"img.png":"{}","src/a.rs":"0000000000000000"}}}}"#,
            fingerprint("img.png")
        ));
        let state = state_of(&world.state("").unwrap());
        let viewed = |path: &str| state.files.iter().find(|f| f.path == path).unwrap().viewed;
        assert!(viewed("img.png"));
        assert!(!viewed("src/a.rs") && !viewed("src/b.rs"));
        assert!(file_of(&world.file("", "img.png").unwrap()).viewed);
        assert!(!file_of(&world.file("", "src/a.rs").unwrap()).viewed);
    }

    #[test]
    fn a_path_that_is_not_in_the_diff_exits_2() {
        let world = World::new("not-in-diff");
        for path in ["src/zzz.rs", "../etc/passwd", "/abs.rs", ""] {
            let output = world.file("", path).unwrap_err().output();
            assert_eq!(output.code, 2);
            assert_eq!(output.stderr, format!("not in diff: {path}\n"));
            assert!(output.stdout.is_empty());
        }
    }

    fn files_of(output: &Output) -> FilesJson {
        assert_eq!(output.code, 0);
        serde_json::from_str(&output.stdout).unwrap()
    }

    #[test]
    fn files_returns_what_file_returns_for_each_path_in_the_order_asked() {
        let world = World::new("files");
        let paths = ["src/a.rs".to_owned(), "img.png".to_owned()];
        let out = files(&query(""), &paths, &world.env, |args| world.git(args)).unwrap();
        assert!(out.stdout.ends_with("}\n"));
        let reply = files_of(&out);
        assert_eq!(reply.schema, 1);
        let each = paths
            .iter()
            .map(|path| file_of(&world.file("", path).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(reply.files, each);
    }

    #[test]
    fn files_reads_the_repo_diff_once_where_file_reads_it_once_per_path() {
        let world = World::new("files-once");
        let paths = ["src/a.rs".to_owned(), "src/b.rs".to_owned()];
        let diffs = std::cell::Cell::new(0);
        let counting = |args: &[String]| {
            diffs.set(diffs.get() + args.iter().filter(|a| *a == "diff").count());
            world.git(args)
        };
        files(&query(""), &paths, &world.env, counting).unwrap();
        assert_eq!(diffs.get(), 1);
        diffs.set(0);
        for path in &paths {
            file(&query(""), path, &world.env, counting).unwrap();
        }
        assert_eq!(diffs.get(), 2);
    }

    #[test]
    fn one_path_that_is_not_in_the_diff_fails_the_whole_files_command() {
        let world = World::new("files-not-in-diff");
        let paths = ["src/a.rs".to_owned(), "src/zzz.rs".to_owned()];
        let output = files(&query(""), &paths, &world.env, |args| world.git(args))
            .unwrap_err()
            .output();
        assert_eq!(output.code, 2);
        assert_eq!(output.stderr, "not in diff: src/zzz.rs\n");
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn a_renamed_file_is_found_by_its_new_path_and_by_its_old_one_when_no_path_fits() {
        let mut world = World::new("rename");
        let new = file_of(&world.file("", "src/b.rs").unwrap());
        let old = file_of(&world.file("", "src/old_b.rs").unwrap());
        assert_eq!(new, old);
        assert_eq!(
            (new.path.as_str(), new.old_path.as_deref(), new.change),
            ("src/b.rs", Some("src/old_b.rs"), ChangeJson::Renamed)
        );
        // A file that has the old name as its own path wins over the rename.
        world.patch.push_str(
            "diff --git a/src/old_b.rs b/src/old_b.rs\n--- a/src/old_b.rs\n+++ b/src/old_b.rs\n@@ -1 +1 @@\n-x\n+y\n",
        );
        let found = file_of(&world.file("", "src/old_b.rs").unwrap());
        assert_eq!(
            (found.path.as_str(), found.old_path),
            ("src/old_b.rs", None)
        );
    }

    #[test]
    fn a_file_that_is_listed_and_not_rendered_has_no_rows_and_a_note() {
        let mut world = World::new("notes");
        world.patch.push_str(
            "diff --git a/mod b/mod\nnew file mode 160000\nindex 0000000..5555555\n--- /dev/null\n+++ b/mod\n@@ -0,0 +1 @@\n+Subproject commit 5555555\n",
        );
        world
            .patch
            .push_str("diff --git a/weird b/weird\nnonsense line\n");
        std::fs::write(world.root.join("big.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        world.listing = Ok(b"big.txt\0".to_vec());
        let note = |path: &str| {
            let file = file_of(&world.file("", path).unwrap());
            assert!(file.rows.is_empty(), "{path}");
            (file.change, file.note)
        };
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(note("img.png"), (ChangeJson::Binary, some("binary file")));
        assert_eq!(note("mod"), (ChangeJson::Submodule, some("submodule")));
        assert_eq!(
            note("big.txt"),
            (ChangeJson::TooLarge, some("too large to show"))
        );
        assert_eq!(
            note("weird"),
            (ChangeJson::Unparsed, some("could not be parsed"))
        );
        // A plain renamed file with no edits has no rows and no note.
        let renamed = file_of(&world.file("", "src/b.rs").unwrap());
        assert!(renamed.rows.is_empty() && renamed.note.is_none());
    }

    #[test]
    fn an_untracked_file_is_read_and_its_rows_are_all_added() {
        let mut world = World::new("untracked");
        std::fs::write(world.root.join("notes.txt"), "one\ntwo\n").unwrap();
        world.listing = Ok(b"notes.txt\0".to_vec());
        let file = file_of(&world.file("", "notes.txt").unwrap());
        assert_eq!(
            (file.change, file.add, file.del),
            (ChangeJson::Untracked, 2, 0)
        );
        let rows = file
            .rows
            .iter()
            .map(|r| (r.kind, r.old, r.new, r.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (RowType::Hunk, None, None, "@@ -0,0 +1,2 @@"),
                (RowType::Add, None, Some(1), "one"),
                (RowType::Add, None, Some(2), "two"),
            ]
        );
    }

    #[test]
    fn the_rows_of_a_file_hold_a_header_then_each_row_without_its_marker() {
        let world = World::new("rows");
        let file = file_of(&world.file("", "src/a.rs").unwrap());
        let rows = file
            .rows
            .iter()
            .map(|r| (r.kind, r.old, r.new, r.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                (RowType::Hunk, None, None, "@@ -1,4 +1,5 @@"),
                (RowType::Ctx, Some(1), Some(1), "fn main() {"),
                (RowType::Del, Some(2), None, "    old();"),
                (RowType::Add, None, Some(2), "    new();"),
                (RowType::Add, None, Some(3), "    more();"),
                (RowType::Ctx, Some(3), Some(4), "    keep();"),
                (RowType::Ctx, Some(4), Some(5), "}"),
            ]
        );
    }

    #[test]
    fn a_thread_is_placed_on_the_row_of_its_line_range_or_nearest_line() {
        let world = World::new("placement");
        let file_comment = add(
            "u6",
            "user",
            r#""path":"src/a.rs","spec":"worktree","body":"b""#,
        );
        let range = add(
            "u2",
            "user",
            r#""path":"src/a.rs","side":"new","line":2,"end_line":3,"line_text":"    new();","spec":"worktree","body":"b""#,
        );
        world.log(&lines(&[
            on_line("u1", "new", 2, "    new();"),
            range,
            on_line("u3", "old", 2, "    old();"),
            on_line("u4", "new", 40, "text that is gone"),
            on_line("u5", "old", 9, "also gone"),
            file_comment,
        ]));
        let file = file_of(&world.file("", "src/a.rs").unwrap());
        let at = |id: &str| {
            let thread = file.threads.iter().find(|t| t.id == id).unwrap();
            (
                thread.placement.kind,
                thread.placement.row,
                thread.placement.start_row,
            )
        };
        assert_eq!(at("u1"), (Matched, Some(3), None));
        assert_eq!(at("u2"), (Matched, Some(4), Some(3)));
        assert_eq!(at("u3"), (Matched, Some(2), None));
        // Nearest new line to 40 is 5, the last row of the hunk. Nearest old line to 9 is 4.
        assert_eq!(at("u4"), (Outdated, Some(6), None));
        assert_eq!(at("u5"), (Outdated, Some(6), None));
        assert_eq!(at("u6"), (Matched, None, None));
        let ids = file
            .threads
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["u1", "u2", "u3", "u4", "u5", "u6"]);
    }

    #[test]
    fn the_file_holds_only_the_threads_of_its_own_path_and_none_that_are_not_in_the_diff() {
        let world = World::new("own-threads");
        let elsewhere = add(
            "u2",
            "user",
            r#""path":"src/gone.rs","spec":"worktree","body":"b""#,
        );
        let renamed = add(
            "u3",
            "user",
            r#""path":"src/old_b.rs","spec":"worktree","body":"b""#,
        );
        world.log(&lines(&[
            on_line("u1", "new", 2, "    new();"),
            elsewhere,
            renamed,
        ]));
        let a = file_of(&world.file("", "src/a.rs").unwrap());
        assert_eq!(a.threads.len(), 1);
        let b = file_of(&world.file("", "src/b.rs").unwrap());
        assert_eq!(
            b.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["u3"]
        );
    }

    #[test]
    fn a_thread_carries_its_comments_the_status_and_who_resolved_it() {
        let world = World::new("thread");
        let reply = add("a1", "agent:claude", r#""parent":"u1","body":"done""#);
        let at = r#""at":"2026-11-09T10:02:00Z""#;
        world.log(&lines(&[
            on_line("u1", "new", 2, "    new();"),
            reply,
            format!(r#"{{{at},"by":"user","kind":"sent","ids":["u1"],"batch":"b1"}}"#),
            format!(r#"{{{at},"by":"user","kind":"edit","id":"u1","body":"b2"}}"#),
            format!(r#"{{{at},"by":"agent:claude","kind":"resolve","id":"u1"}}"#),
        ]));
        let file = file_of(&world.file("", "src/a.rs").unwrap());
        let thread = &file.threads[0];
        assert_eq!(thread.status, StatusJson::Resolved);
        assert_eq!(thread.resolved_by.as_deref(), Some("agent:claude"));
        assert!(thread.is_new && !thread.unsent);
        assert_eq!(thread.anchor.kind, AnchorKind::Line);
        assert_eq!(thread.anchor.side, Some(SideJson::New));
        assert_eq!(
            (thread.anchor.line, thread.anchor.end_line),
            (Some(2), None)
        );
        assert_eq!(thread.anchor.spec, "worktree");
        let [root, reply] = thread.comments.as_slice() else {
            panic!("{:?}", thread.comments)
        };
        assert_eq!((root.id.as_str(), root.parent.as_deref()), ("u1", None));
        assert_eq!((root.author.as_str(), root.body.as_str()), ("user", "b2"));
        assert!(root.sent && root.edited_since_sent);
        assert_eq!(reply.parent.as_deref(), Some("u1"));
        assert_eq!(reply.author, "agent:claude");
        assert!(!reply.sent);
    }

    /// Compare `json` with the golden file the mod's decoder tests also read. The file is written
    /// in the order of the design. With `REVIEW_UPDATE_FIXTURES` set it is written instead of
    /// compared, and the same commit carries it.
    fn golden(name: &str, json: &impl serde::Serialize) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("herdr-app-mod/test/fixtures")
            .join(name);
        if std::env::var_os("REVIEW_UPDATE_FIXTURES").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(json).unwrap() + "\n").unwrap();
        }
        let saved = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            saved,
            serde_json::to_value(json).unwrap(),
            "{name} differs, rerun with REVIEW_UPDATE_FIXTURES=1 and commit the new file"
        );
    }

    /// A review with every kind of thread, over a diff with every kind of file.
    fn golden_world(name: &str) -> World {
        let mut world = World::new(name);
        world.patch.push_str(
            "diff --git a/mod b/mod\nnew file mode 160000\nindex 0000000..5555555\n--- /dev/null\n+++ b/mod\n@@ -0,0 +1 @@\n+Subproject commit 5555555\n",
        );
        std::fs::write(world.root.join("notes.txt"), "one\ntwo\n").unwrap();
        world.listing = Ok(b"notes.txt\0".to_vec());
        let range = add(
            "u2",
            "user",
            r#""path":"src/a.rs","side":"new","line":2,"end_line":3,"line_text":"    new();","spec":"worktree","body":"Why two lines?""#,
        );
        let file_comment = add(
            "u5",
            "user",
            r#""path":"src/a.rs","spec":"worktree","body":"Add a doc comment.""#,
        );
        let gone = add(
            "u6",
            "user",
            r#""path":"src/gone.rs","spec":"worktree","body":"This file is no longer in the diff.""#,
        );
        let reply = add("a1", "agent:claude", r#""parent":"u2","body":"Fixed.""#);
        let at = r#""at":"2026-11-09T10:05:00Z""#;
        world.log(&lines(&[
            on_line("u1", "new", 2, "    new();"),
            range,
            on_line("u3", "old", 2, "    old();"),
            on_line("u4", "new", 40, "text that is gone"),
            file_comment,
            gone,
            reply,
            format!(r#"{{{at},"by":"user","kind":"sent","ids":["u2"],"batch":"b1"}}"#),
            format!(r#"{{{at},"by":"agent:claude","kind":"resolve","id":"u2"}}"#),
            // The user closed u3 themselves, so archive would take it.
            format!(r#"{{{at},"by":"user","kind":"resolve","id":"u3"}}"#),
        ]));
        let fingerprint = crate::diff::parse(PATCH.as_bytes(), crate::diff::MAX_PATCH)
            .into_iter()
            .find(|file| file.path.as_str() == "img.png")
            .unwrap()
            .fingerprint();
        world.meta(&format!(
            r#"{{"spec":"worktree","base":"main","viewed":{{"img.png":"{fingerprint}"}}}}"#
        ));
        world
    }

    #[test]
    fn state_prints_what_the_golden_file_says() {
        let world = golden_world("golden-state");
        // The root and the state directory are in a temp directory, so they are replaced.
        let mut state = state_of(&world.state("").unwrap());
        assert_eq!(state.root, world.root.display().to_string());
        state.root = "/repo".into();
        state.store_dir = "/state".into();
        golden("state.json", &state);
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn file_prints_what_the_golden_file_says() {
        let world = golden_world("golden-file");
        golden("file.json", &file_of(&world.file("", "src/a.rs").unwrap()));
    }

    /// The patch of `app.js` that the token tests read: a context line inside a block comment,
    /// and a changed line with non-ASCII text.
    fn app_js(world: &mut World) {
        world.patch = "diff --git a/app.js b/app.js\n--- a/app.js\n+++ b/app.js\n@@ -2,2 +2,2 @@\n  end */\n-let s = \"x\";\n+let s = \"\u{e9}\u{1f600}\"; // x\n".to_owned();
        std::fs::write(
            world.root.join("app.js"),
            "/* \u{e9}\n end */\nlet s = \"\u{e9}\u{1f600}\"; // x\n",
        )
        .unwrap();
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn tokens_are_kinds_with_utf16_offsets_from_the_whole_file() {
        let mut world = World::new("tokens");
        app_js(&mut world);
        let file = file_of(&world.file("", "app.js").unwrap());
        let tokens = |row: usize| file.rows[row].tokens.clone();
        // A hunk header has none. A context line in a comment that opens above the hunk is a comment.
        assert!(tokens(0).is_empty());
        assert_eq!(file.rows[1].text, " end */");
        assert_eq!(tokens(1), [(0, 7, TokenKind::Comment)]);
        // `let s = "<e acute><grinning face>"; // x`: the face is 4 bytes and 2 UTF-16 units, the
        // e acute 2 bytes and 1 unit, so the string is bytes 8..16 and units 8..13.
        let add = tokens(3);
        assert_eq!(file.rows[3].text, "let s = \"\u{e9}\u{1f600}\"; // x");
        assert!(add.contains(&(0, 3, TokenKind::Keyword)), "{add:?}");
        assert!(add.contains(&(8, 13, TokenKind::String)), "{add:?}");
        assert!(add.contains(&(15, 19, TokenKind::Comment)), "{add:?}");
        // The parts come in order, do not overlap and stay inside the text.
        let units = file.rows[3].text.encode_utf16().count();
        let mut end = 0;
        for (from, to, _) in &add {
            assert!(*from >= end && from < to && *to <= units, "{add:?}");
            end = *to;
        }
        // Written as three-element arrays, and a row with none has no `tokens` key.
        let printed = world.file("", "app.js").unwrap().stdout;
        assert!(
            printed.contains(r#""tokens":[[0,3,"keyword"]"#),
            "{printed}"
        );
        let value: serde_json::Value = serde_json::from_str(&printed).unwrap();
        assert!(value["rows"][0].get("tokens").is_none());
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn a_file_is_parsed_up_to_a_budget_of_lines_and_the_rows_past_it_have_no_tokens() {
        let mut world = World::new("budget");
        let lines = MAX_PARSED_LINES + 5;
        let text = (1..=lines).map(|n| format!("let v{n} = {n};\n"));
        std::fs::write(world.root.join("app.js"), text.collect::<String>()).unwrap();
        let hunk = |n: usize| format!("@@ -{n} +{n} @@\n-let v{n} = 0;\n+let v{n} = {n};\n");
        world.patch = format!(
            "diff --git a/app.js b/app.js\n--- a/app.js\n+++ b/app.js\n{}{}",
            hunk(2),
            hunk(lines - 1)
        );
        let file = file_of(&world.file("", "app.js").unwrap());
        let row = |text: &str| file.rows.iter().find(|row| row.text == text).unwrap();
        assert!(!row("let v2 = 2;").tokens.is_empty());
        let last = format!("let v{} = {};", lines - 1, lines - 1);
        assert!(row(&last).tokens.is_empty());
    }

    #[test]
    fn a_language_the_highlighter_does_not_know_has_no_tokens() {
        let mut world = World::new("unknown-language");
        app_js(&mut world);
        world.patch = world.patch.replace("app.js", "app.zzz");
        std::fs::rename(world.root.join("app.js"), world.root.join("app.zzz")).unwrap();
        let output = world.file("", "app.zzz").unwrap();
        assert!(!output.stdout.contains("tokens"), "{}", output.stdout);
        assert_eq!(file_of(&output).rows.len(), 4);
    }

    /// The marks of each row of `a.zzz`, as `(from, to)` pairs, for a patch of one hunk.
    fn marks_of(name: &str, body: &str) -> (FileJson, Vec<Vec<(usize, usize)>>) {
        let mut world = World::new(name);
        let (old, new) = (
            body.lines().filter(|l| !l.starts_with('+')).count(),
            body.lines().filter(|l| !l.starts_with('-')).count(),
        );
        world.patch = format!(
            "diff --git a/a.zzz b/a.zzz\n--- a/a.zzz\n+++ b/a.zzz\n@@ -1,{old} +1,{new} @@\n{body}"
        );
        std::fs::write(world.root.join("a.zzz"), "x\n").unwrap();
        let file = file_of(&world.file("", "a.zzz").unwrap());
        let marks = file
            .rows
            .iter()
            .map(|row| row.marks.iter().map(|m| (m.from, m.to)).collect())
            .collect();
        (file, marks)
    }

    #[test]
    fn a_removed_row_and_the_added_row_that_replaced_it_mark_the_words_that_changed() {
        let body = " keep\n-let count = 1;\n-fn old_name(a) {}\n-all of this goes\n\
                    +let total = 1;\n+fn new_name(a) {}\n+x\n+and this is extra\n end\n";
        let (file, marks) = marks_of("marks", body);
        // Rows: header, keep, three removed, four added, end.
        assert_eq!(file.rows.len(), 10);
        assert_eq!(file.rows[2].text, "let count = 1;");
        assert_eq!(marks[2], [(4, 9)]);
        assert_eq!(marks[3], [(3, 11)]);
        assert_eq!(marks[5], [(4, 9)]);
        assert_eq!(marks[6], [(3, 11)]);
        // A rewritten pair, a row with no partner, context and the header have none.
        for row in [0, 1, 4, 7, 8, 9] {
            assert!(marks[row].is_empty(), "row {row}: {:?}", marks[row]);
        }
        // Written for every row, `[]` when there are none.
        let value: serde_json::Value = serde_json::to_value(&file).unwrap();
        assert_eq!(
            value["rows"][2]["marks"],
            serde_json::json!([{"from": 4, "to": 9}])
        );
        assert_eq!(value["rows"][1]["marks"], serde_json::json!([]));
    }

    #[test]
    fn marks_count_utf16_units_on_a_row_with_non_ascii_characters() {
        // The e acute is 2 bytes and 1 unit, the grinning face 4 bytes and 2 units.
        let body = "-let s = \"\u{e9}\u{1f600}\" + a;\n+let s = \"\u{e9}\u{1f600}\" + b;\n";
        let (file, marks) = marks_of("marks-utf16", body);
        assert_eq!(marks[1], [(16, 17)]);
        assert_eq!(marks[2], [(16, 17)]);
        let text = &file.rows[1].text;
        let units: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[16..17]).unwrap(), "a");
    }

    #[cfg(not(feature = "syntax"))]
    #[test]
    fn a_build_without_the_syntax_feature_prints_no_tokens() {
        let mut world = World::new("no-syntax");
        app_js(&mut world);
        let output = world.file("", "app.js").unwrap();
        assert!(!output.stdout.contains("tokens"), "{}", output.stdout);
        let file = file_of(&output);
        assert_eq!(file.rows.len(), 4);
        assert!(file.rows.iter().all(|row| row.tokens.is_empty()));
    }

    #[test]
    fn a_span_that_is_not_on_characters_is_left_out() {
        let spans = [
            (0..2, Token::Keyword),
            // Inside the two bytes of the e acute.
            (3..4, Token::String),
            (2..5, Token::Number),
            (5..5, Token::Type),
        ];
        let text = "ab\u{e9}c";
        assert_eq!(
            tokens_json(text, &spans),
            [(0, 2, TokenKind::Keyword), (2, 4, TokenKind::Number)]
        );
    }
}
