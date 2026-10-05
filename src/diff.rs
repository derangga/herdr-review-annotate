//! `git`: the runner, the repository root, and the unified diff parser.

use std::fmt;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::store::{RelPath, Side};

/// `git` could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotInstalled,
    NotARepo,
    Failed { args: String, stderr: String },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("git is not installed"),
            Self::NotARepo => f.write_str("not a git repository"),
            Self::Failed { args, stderr } => write!(f, "git {args} failed: {stderr}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Run `git` with `args` and return its stdout. It never takes `index.lock`, so the review cannot
/// make the agent's own `git` commands fail.
pub fn run_git(args: &[String]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => GitError::NotInstalled,
            _ => GitError::Failed {
                args: args.join(" "),
                stderr: error.to_string(),
            },
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.contains("not a git repository") {
        return Err(GitError::NotARepo);
    }
    Err(GitError::Failed {
        args: args.join(" "),
        stderr,
    })
}

/// The canonical absolute path of a worktree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRoot(PathBuf);

impl RepoRoot {
    /// The worktree that contains `repo`, or `cwd` when no `--repo` was given.
    pub fn resolve(
        repo: Option<&Path>,
        cwd: &Path,
        mut git: impl FnMut(&[String]) -> Result<String, GitError>,
    ) -> Result<Self, GitError> {
        let from = repo.unwrap_or(cwd).to_string_lossy().into_owned();
        let args = ["-C", &from, "rev-parse", "--show-toplevel"].map(str::to_owned);
        let output = git(&args)?;
        let top = output.trim();
        if top.is_empty() {
            return Err(GitError::NotARepo);
        }
        Path::new(top)
            .canonicalize()
            .map(Self)
            .map_err(|error| GitError::Failed {
                args: args.join(" "),
                stderr: format!("{top}: {error}"),
            })
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// The most patch text a review renders. Files past it are listed and collapsed.
pub const MAX_PATCH: usize = 3 * 1024 * 1024;

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Binary,
    Submodule,
    /// Past the patch cap, or an untracked file too big to read. Listed, not rendered.
    TooLarge,
    /// `git` printed a section the parser could not read. Listed, not rendered.
    Unparsed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Context,
    Added,
    Removed,
}

/// One line of a hunk. `text` has no `\r`, so it compares the same on every platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    /// The line number in the old file, on context and removed rows.
    pub old: Option<u32>,
    /// The line number in the new file, on context and added rows.
    pub new: Option<u32>,
    pub text: String,
    /// `\ No newline at end of file` follows this row.
    pub no_newline: bool,
}

impl Row {
    /// The line number on `side`, if the row exists there.
    pub const fn line(&self, side: Side) -> Option<u32> {
        match side {
            Side::Old => self.old,
            Side::New => self.new,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -a,b +c,d @@ section` line.
    pub header: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    /// The file mode changed, alone or with the content.
    pub mode_changed: bool,
}

/// One file of a diff. A deleted file has the path it had. A renamed file has its new path, and
/// `old_path` is the one an old-side comment cites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: RelPath,
    pub old_path: Option<RelPath>,
    pub change: Change,
    pub hunks: Vec<Hunk>,
    pub flags: Flags,
}

impl DiffFile {
    fn listed(path: RelPath, change: Change) -> Self {
        Self {
            path,
            old_path: None,
            change,
            hunks: Vec::new(),
            flags: Flags::default(),
        }
    }

    /// The rows of every hunk that exist on `side`, in file order, with their line numbers.
    pub fn rows(&self, side: Side) -> impl Iterator<Item = (u32, &Row)> {
        self.hunks
            .iter()
            .flat_map(|hunk| &hunk.rows)
            .filter_map(move |row| Some((row.line(side)?, row)))
    }
}

/// Parse the output of `git diff` into files. A section that cannot be read becomes
/// `Change::Unparsed` and the sections around it are unaffected. Once the sections read so far add
/// up to more than `cap` bytes, that section and every later one is `Change::TooLarge`.
pub fn parse(patch: &[u8], cap: usize) -> Vec<DiffFile> {
    let mut used = 0;
    sections(patch)
        .into_iter()
        .filter_map(|section| {
            used += section.len();
            let too_large = used > cap;
            let parsed = parse_section(section, !too_large).map(|mut file| {
                if too_large {
                    file.change = Change::TooLarge;
                }
                file
            });
            parsed.or_else(|| unparsed(section))
        })
        .collect()
}

/// The text from each `diff --git` line to the next. Anything before the first is dropped.
fn sections(patch: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in patch.split_inclusive(|&byte| byte == b'\n') {
        if line.starts_with(b"diff --git ") {
            starts.push(offset);
        }
        offset += line.len();
    }
    let ends = starts.iter().skip(1).copied().chain([patch.len()]);
    starts
        .iter()
        .zip(ends)
        .filter_map(|(&start, end)| patch.get(start..end))
        .collect()
}

/// A section that could not be read, named by its header when that names one path.
fn unparsed(section: &[u8]) -> Option<DiffFile> {
    let first = section
        .split(|&byte| byte == b'\n')
        .next()
        .unwrap_or_default();
    let path = header_path(first)
        .and_then(|path| RelPath::parse(&path))
        .or_else(|| RelPath::parse("(unreadable diff section)"))?;
    Some(DiffFile::listed(path, Change::Unparsed))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A row's text: the bytes after the `+`, `-` or space tag, without the carriage return.
fn row_text(body: &[u8]) -> String {
    lossy(body.strip_suffix(b"\r").unwrap_or(body))
}

/// Decode a C-style quoted path such as `"a/tab\there"`.
fn unquote(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut rest = bytes.strip_prefix(b"\"")?.iter().copied();
    let mut out = Vec::new();
    loop {
        match rest.next()? {
            b'"' => return Some(out),
            b'\\' => {
                let escape = rest.next()?;
                out.push(match escape {
                    b'a' => 7,
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 11,
                    b'\\' | b'"' => escape,
                    b'0'..=b'3' => {
                        let mut value = u32::from(escape - b'0');
                        for _ in 0..2 {
                            let digit = rest.next()?.checked_sub(b'0').filter(|d| *d < 8)?;
                            value = value * 8 + u32::from(digit);
                        }
                        u8::try_from(value).ok()?
                    }
                    _ => return None,
                });
            }
            byte => out.push(byte),
        }
    }
}

/// A path as `---`, `+++` and `rename` lines print it. Git ends a path that holds a space with a
/// tab, and quotes one that holds a control character.
fn path_field(field: &[u8]) -> Option<String> {
    if field.starts_with(b"\"") {
        return unquote(field).map(|path| lossy(&path));
    }
    let end = field
        .iter()
        .position(|&byte| byte == b'\t')
        .unwrap_or(field.len());
    Some(lossy(field.get(..end)?))
}

/// A `---` or `+++` path without its `a/` or `b/`.
fn prefixed(field: &[u8], prefix: &str) -> Option<String> {
    path_field(field)?.strip_prefix(prefix).map(str::to_owned)
}

/// The path of a `diff --git` line. Only sections with no `---`, `+++` or `rename` line need it
/// (binary files, a mode change alone), and those have the same path on both sides.
fn header_path(line: &[u8]) -> Option<String> {
    let rest = line.strip_prefix(b"diff --git ")?;
    if rest.starts_with(b"\"") {
        return lossy(&unquote(rest)?).strip_prefix("a/").map(str::to_owned);
    }
    let half = rest.len().checked_sub(5).filter(|n| n % 2 == 0)? / 2;
    let old = rest.get(2..2 + half)?;
    let new = rest.get(rest.len() - half..)?;
    let joined = rest.starts_with(b"a/") && rest.get(2 + half..5 + half)? == b" b/";
    (joined && old == new).then(|| lossy(old))
}

/// What the lines before the first hunk say.
#[derive(Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one flag per header line git prints"
)]
struct Head {
    old: Option<String>,
    new: Option<String>,
    rename_from: Option<String>,
    rename_to: Option<String>,
    new_file: bool,
    deleted: bool,
    binary: bool,
    submodule: bool,
    mode_changed: bool,
}

impl Head {
    /// Take one line. A line `git` does not print before a hunk fails the section, so a garbled
    /// `@@` line cannot pass as an unknown header.
    fn line(&mut self, line: &[u8]) -> Option<()> {
        let starts = |prefix: &[u8]| line.strip_prefix(prefix);
        if let Some(field) = starts(b"--- ") {
            self.old = if field == b"/dev/null" {
                None
            } else {
                Some(prefixed(field, "a/")?)
            };
        } else if let Some(field) = starts(b"+++ ") {
            self.new = if field == b"/dev/null" {
                None
            } else {
                Some(prefixed(field, "b/")?)
            };
        } else if let Some(field) = starts(b"rename from ") {
            self.rename_from = Some(path_field(field)?);
        } else if let Some(field) = starts(b"rename to ") {
            self.rename_to = Some(path_field(field)?);
        } else if starts(b"new file mode ").is_some() {
            self.new_file = true;
            self.submodule |= line.ends_with(b" 160000");
        } else if starts(b"deleted file mode ").is_some() {
            self.deleted = true;
            self.submodule |= line.ends_with(b" 160000");
        } else if starts(b"old mode ").is_some() || starts(b"new mode ").is_some() {
            self.mode_changed = true;
        } else if starts(b"index ").is_some() {
            self.submodule |= line.ends_with(b" 160000");
        } else if starts(b"Binary files ").is_some() {
            self.binary = true;
        } else if starts(b"similarity index ").is_none()
            && starts(b"dissimilarity index ").is_none()
        {
            return None;
        }
        Some(())
    }
}

/// Read one section. With `render` false the hunks are skipped, and the section is only named.
fn parse_section(section: &[u8], render: bool) -> Option<DiffFile> {
    let mut lines: Vec<&[u8]> = section.split(|&byte| byte == b'\n').collect();
    if lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    let mut lines = lines.into_iter().peekable();
    let header = lines.next()?;
    let mut head = Head::default();
    while let Some(line) = lines.next_if(|line| !line.starts_with(b"@@")) {
        head.line(line)?;
    }
    let mut hunks = Vec::new();
    while render && let Some(line) = lines.next() {
        hunks.push(hunk(line, &mut lines)?);
    }
    let renamed = head.rename_from.is_some() && head.rename_to.is_some();
    let (path, old_path) = if let (Some(from), Some(to)) = (head.rename_from, head.rename_to) {
        (to, Some(from))
    } else {
        let path = head.new.or(head.old).or_else(|| header_path(header))?;
        (path, None)
    };
    let change = if head.submodule {
        Change::Submodule
    } else if head.binary {
        Change::Binary
    } else if renamed {
        Change::Renamed
    } else if head.new_file {
        Change::Added
    } else if head.deleted {
        Change::Deleted
    } else {
        Change::Modified
    };
    Some(DiffFile {
        path: RelPath::parse(&path)?,
        old_path: match old_path {
            Some(path) => Some(RelPath::parse(&path)?),
            None => None,
        },
        change,
        hunks,
        flags: Flags {
            mode_changed: head.mode_changed,
        },
    })
}

/// `a` or `a,b` of a hunk header: the first line and the line count, which is 1 when omitted.
fn range(text: &str) -> Option<(u32, u32)> {
    match text.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// Read a hunk whose `@@` line is `header`, through the row that makes both counts add up. The
/// next line must then start another hunk or end the section, which the caller checks.
fn hunk<'a>(header: &[u8], lines: &mut Peekable<impl Iterator<Item = &'a [u8]>>) -> Option<Hunk> {
    let header = lossy(header);
    let (ranges, _) = header.strip_prefix("@@ -")?.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let ((mut old_line, mut old_left), (mut new_line, mut new_left)) = (range(old)?, range(new)?);
    let mut rows: Vec<Row> = Vec::new();
    while old_left > 0 || new_left > 0 {
        let line = lines.next()?;
        let (&tag, body) = line.split_first()?;
        let (kind, old, new) = match tag {
            b'\\' => {
                rows.last_mut()?.no_newline = true;
                continue;
            }
            b' ' if old_left > 0 && new_left > 0 => {
                (RowKind::Context, Some(old_line), Some(new_line))
            }
            b'-' if old_left > 0 => (RowKind::Removed, Some(old_line), None),
            b'+' if new_left > 0 => (RowKind::Added, None, Some(new_line)),
            _ => return None,
        };
        if old.is_some() {
            old_line += 1;
            old_left -= 1;
        }
        if new.is_some() {
            new_line += 1;
            new_left -= 1;
        }
        rows.push(Row {
            kind,
            old,
            new,
            text: row_text(body),
            no_newline: false,
        });
    }
    while lines.next_if(|line| line.starts_with(b"\\")).is_some() {
        rows.last_mut()?.no_newline = true;
    }
    Some(Hunk { header, rows })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_canonical_toplevel() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let mut seen = Vec::new();
        let root = RepoRoot::resolve(None, Path::new("/work"), |args| {
            seen.push(args.join(" "));
            Ok(format!(
                "{}/../{}\n",
                dir.display(),
                dir.file_name().unwrap().to_string_lossy()
            ))
        })
        .unwrap();
        assert_eq!(root.path(), dir);
        assert_eq!(seen, ["-C /work rev-parse --show-toplevel"]);
    }

    #[test]
    fn a_repo_argument_replaces_the_working_directory() {
        let mut seen = Vec::new();
        let _ = RepoRoot::resolve(Some(Path::new("/other")), Path::new("/work"), |args| {
            seen.push(args.join(" "));
            Err(GitError::NotARepo)
        });
        assert_eq!(seen, ["-C /other rev-parse --show-toplevel"]);
    }

    #[test]
    fn git_failing_or_printing_nothing_is_not_a_repo_or_an_error() {
        let failed = RepoRoot::resolve(None, Path::new("/work"), |_| Err(GitError::NotInstalled));
        assert_eq!(failed, Err(GitError::NotInstalled));
        let empty = RepoRoot::resolve(None, Path::new("/work"), |_| Ok("\n".into()));
        assert_eq!(empty, Err(GitError::NotARepo));
        let gone = RepoRoot::resolve(None, Path::new("/work"), |_| Ok("/no/such/dir\n".into()));
        assert!(matches!(gone, Err(GitError::Failed { .. })));
    }

    const MIXED: &[u8] = include_bytes!("../tests/fixtures/mixed.patch");

    fn find<'a>(files: &'a [DiffFile], path: &str) -> &'a DiffFile {
        files
            .iter()
            .find(|file| file.path.as_str() == path)
            .unwrap()
    }

    fn texts(file: &DiffFile) -> Vec<(RowKind, &str)> {
        file.hunks
            .iter()
            .flat_map(|hunk| &hunk.rows)
            .map(|row| (row.kind, row.text.as_str()))
            .collect()
    }

    #[test]
    fn a_renamed_file_keeps_its_old_path_and_its_edits() {
        let files = parse(MIXED, MAX_PATCH);
        let file = find(&files, "new_name.txt");
        assert_eq!(file.change, Change::Renamed);
        assert_eq!(file.old_path.as_ref().unwrap().as_str(), "old_name.txt");
        let hunk = &file.hunks[0];
        assert_eq!(hunk.header, "@@ -2,9 +2,10 @@ l1");
        let removed = hunk
            .rows
            .iter()
            .find(|r| r.kind == RowKind::Removed)
            .unwrap();
        assert_eq!(
            (removed.old, removed.new, removed.text.as_str()),
            (Some(5), None, "l5")
        );
        let last = hunk.rows.last().unwrap();
        assert_eq!(
            (last.old, last.new, last.text.as_str()),
            (None, Some(11), "l11")
        );
    }

    #[test]
    fn a_binary_file_is_listed_without_rows() {
        let files = parse(MIXED, MAX_PATCH);
        let file = find(&files, "bin.dat");
        assert_eq!(file.change, Change::Binary);
        assert!(file.hunks.is_empty());
    }

    #[test]
    fn the_missing_newline_marks_the_row_it_follows() {
        let files = parse(MIXED, MAX_PATCH);
        let rows = &find(&files, "nonl.txt").hunks[0].rows;
        let marked = rows.iter().filter(|row| row.no_newline).collect::<Vec<_>>();
        assert_eq!(marked.len(), 1);
        assert_eq!(
            (marked[0].kind, marked[0].text.as_str()),
            (RowKind::Added, "c")
        );
    }

    #[test]
    fn a_missing_newline_between_a_removed_and_an_added_row_marks_the_removed_one() {
        let patch = b"diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-c\n\\ No newline at end of file\n+d\n";
        let files = parse(patch, MAX_PATCH);
        let rows = &files[0].hunks[0].rows;
        assert_eq!(files[0].change, Change::Modified);
        assert_eq!((rows[0].no_newline, rows[1].no_newline), (true, false));
    }

    #[test]
    fn carriage_returns_are_not_part_of_a_rows_text() {
        let files = parse(MIXED, MAX_PATCH);
        assert_eq!(
            texts(find(&files, "crlf.txt")),
            [
                (RowKind::Context, "a"),
                (RowKind::Removed, "b"),
                (RowKind::Added, "B"),
                (RowKind::Context, "c"),
            ]
        );
    }

    #[test]
    fn paths_with_spaces_non_ascii_letters_and_a_tab_are_read_whole() {
        let files = parse(MIXED, MAX_PATCH);
        for path in ["with space.txt", "café é.txt", "tab\there.txt"] {
            let file = find(&files, path);
            assert_eq!(file.change, Change::Modified, "{path}");
            assert_eq!(file.hunks.len(), 1, "{path}");
        }
    }

    #[test]
    fn a_submodule_is_one_pair_of_rows_with_the_commit_ids() {
        let files = parse(MIXED, MAX_PATCH);
        let file = find(&files, "subm");
        assert_eq!(file.change, Change::Submodule);
        let rows = texts(file);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].1.starts_with("Subproject commit 49ba847"));
        assert!(rows[1].1.starts_with("Subproject commit 6800a5f"));
    }

    #[test]
    fn added_and_deleted_files_carry_every_row() {
        let files = parse(MIXED, MAX_PATCH);
        let added = find(&files, "added.txt");
        assert_eq!(added.change, Change::Added);
        assert_eq!(
            texts(added),
            [(RowKind::Added, "new"), (RowKind::Added, "file")]
        );
        let deleted = find(&files, "gone.txt");
        assert_eq!(deleted.change, Change::Deleted);
        assert_eq!(
            texts(deleted),
            [(RowKind::Removed, "keep"), (RowKind::Removed, "remove")]
        );
    }

    #[test]
    fn a_mode_change_alone_is_a_modified_file_with_no_hunks() {
        let files = parse(MIXED, MAX_PATCH);
        let file = find(&files, "mode.sh");
        assert_eq!(file.change, Change::Modified);
        assert!(file.flags.mode_changed);
        assert!(file.hunks.is_empty());
        assert!(!find(&files, "crlf.txt").flags.mode_changed);
    }

    #[test]
    fn every_section_of_the_fixture_is_a_file_in_order() {
        let paths = parse(MIXED, MAX_PATCH)
            .iter()
            .map(|file| file.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "added.txt",
                "bin.dat",
                "café é.txt",
                "crlf.txt",
                "gone.txt",
                "mode.sh",
                "new_name.txt",
                "nonl.txt",
                "subm",
                "tab\there.txt",
                "with space.txt",
            ]
        );
    }

    #[test]
    fn a_corrupted_section_is_unparsed_and_its_neighbours_are_not() {
        let clean = parse(MIXED, MAX_PATCH);
        let text = String::from_utf8(MIXED.to_vec()).unwrap();
        // crlf.txt now claims four new lines but holds three.
        let corrupted = text.replace("@@ -1,3 +1,3 @@\n a\r", "@@ -1,3 +1,4 @@\n a\r");
        assert_ne!(corrupted, text);
        let files = parse(corrupted.as_bytes(), MAX_PATCH);
        assert_eq!(files.len(), clean.len());
        for (after, before) in files.iter().zip(&clean) {
            if before.path.as_str() == "crlf.txt" {
                assert_eq!(after.change, Change::Unparsed);
                assert!(after.hunks.is_empty());
            } else {
                assert_eq!(after, before);
            }
        }
    }

    #[test]
    fn a_garbled_header_line_or_hunk_line_is_unparsed() {
        let bad_hunk = b"diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n?x\n";
        let bad_head = b"diff --git a/f b/f\n--- a/f\n+++ b/f\nwhat\n@@ -1 +1 @@\n-x\n+y\n";
        let junk = b"preamble\ndiff --git nonsense\n";
        for (patch, path) in [
            (&bad_hunk[..], "f"),
            (&bad_head[..], "f"),
            (&junk[..], "(unreadable diff section)"),
        ] {
            let files = parse(patch, MAX_PATCH);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].change, Change::Unparsed);
            assert_eq!(files[0].path.as_str(), path);
        }
        assert!(parse(b"nothing here\n", MAX_PATCH).is_empty());
    }

    #[test]
    fn files_past_the_cap_are_listed_and_marked_too_large() {
        let at = MIXED
            .windows(b"diff --git a/crlf.txt".len())
            .position(|window| window == b"diff --git a/crlf.txt")
            .unwrap();
        let files = parse(MIXED, at);
        let clean = parse(MIXED, MAX_PATCH);
        assert_eq!(files.len(), clean.len());
        assert_eq!(files[..3], clean[..3]);
        let crlf = find(&files, "crlf.txt");
        assert_eq!(crlf.change, Change::TooLarge);
        assert!(crlf.hunks.is_empty());
        let later = [
            "gone.txt",
            "mode.sh",
            "new_name.txt",
            "nonl.txt",
            "subm",
            "with space.txt",
        ];
        for path in later {
            let file = find(&files, path);
            assert_eq!(file.change, Change::TooLarge, "{path}");
            assert!(file.hunks.is_empty(), "{path}");
        }
        assert_eq!(
            find(&files, "new_name.txt")
                .old_path
                .as_ref()
                .unwrap()
                .as_str(),
            "old_name.txt"
        );
        assert_eq!(find(&files, "added.txt").change, Change::Added);
    }
}
