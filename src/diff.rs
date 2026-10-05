//! `git`: the runner, the repository root, and the unified diff parser.

use std::fmt;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::store::{AnchorTarget, RelPath, Side, Spec, Thread};

/// `git` could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotInstalled,
    NotARepo,
    /// None of the refs tried resolves.
    NoBase {
        tried: Vec<String>,
    },
    Failed {
        args: String,
        stderr: String,
    },
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled => f.write_str("git is not installed"),
            Self::NotARepo => f.write_str("not a git repository"),
            Self::NoBase { tried } => write!(f, "no base branch found, tried {}", tried.join(", ")),
            Self::Failed { args, stderr } => write!(f, "git {args} failed: {stderr}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Run `git` with `args` and return its stdout. It never takes `index.lock`, so the review cannot
/// make the agent's own `git` commands fail.
pub fn run_git_bytes(args: &[String]) -> Result<Vec<u8>, GitError> {
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
        return Ok(output.stdout);
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

/// `run_git_bytes` for output that is text.
pub fn run_git(args: &[String]) -> Result<String, GitError> {
    run_git_bytes(args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
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
    /// The added and removed lines of the file, for the sidebar. A file with no hunks has none.
    pub fn stat(&self) -> (usize, usize) {
        let rows = self.hunks.iter().flat_map(|hunk| &hunk.rows);
        rows.fold((0, 0), |(added, removed), row| match row.kind {
            RowKind::Added => (added + 1, removed),
            RowKind::Removed => (added, removed + 1),
            RowKind::Context => (added, removed),
        })
    }

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

/// The most an untracked file may hold and still be rendered.
const MAX_UNTRACKED: u64 = 1024 * 1024;

/// Something the user should know about the diff on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The branch spec's base does not resolve, so the working tree against `HEAD` is shown.
    BaseMissing { base: String },
    /// `git ls-files` failed.
    UntrackedNotShown,
    /// The diff is over the cap, and later files are collapsed.
    PatchCapped,
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BaseMissing { base } => {
                write!(
                    f,
                    "base {base} not found, showing the working tree against HEAD"
                )
            }
            Self::UntrackedNotShown => f.write_str("untracked files not shown"),
            Self::PatchCapped => f.write_str("diff is over 3 MiB, later files are collapsed"),
        }
    }
}

/// The diff a review shows. `spec` is the one that produced it, which differs from the one asked
/// for when the base is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub files: Vec<DiffFile>,
    pub spec: Spec,
    pub notices: Vec<Notice>,
}

/// The branch the branch spec compares against when `open` got no `--base`: `origin/HEAD`, else
/// `main`, else `master`.
pub fn default_base(
    root: &Path,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<String, GitError> {
    let tried = ["origin/HEAD", "main", "master"];
    for name in tried {
        let args = [
            "-C",
            &root.to_string_lossy(),
            "rev-parse",
            "--verify",
            "--quiet",
            name,
        ]
        .map(str::to_owned);
        match git(&args) {
            Ok(_) => return Ok(name.to_owned()),
            Err(GitError::Failed { .. }) => {}
            Err(error) => return Err(error),
        }
    }
    Err(GitError::NoBase {
        tried: tried.map(str::to_owned).to_vec(),
    })
}

type Run<'a> = dyn FnMut(&[&str]) -> Result<Vec<u8>, GitError> + 'a;

fn text(output: &[u8]) -> String {
    lossy(output).trim().to_owned()
}

/// What to diff the working tree against, and the spec that gives it. With no `HEAD` it is the
/// empty tree. With a branch spec whose base does not resolve it is `HEAD`, and a notice says so.
fn revision(
    run: &mut Run<'_>,
    spec: &Spec,
    notices: &mut Vec<Notice>,
) -> Result<(String, Spec), GitError> {
    let has_head = match run(&["rev-parse", "--verify", "--quiet", "HEAD"]) {
        Ok(_) => true,
        Err(GitError::Failed { .. }) => false,
        Err(error) => return Err(error),
    };
    let merge_base = match spec {
        Spec::Branch { base } if has_head && !base.starts_with('-') => {
            match run(&["merge-base", base, "HEAD"]) {
                Ok(output) => Some(text(&output)).filter(|sha| !sha.is_empty()),
                Err(GitError::Failed { .. }) => None,
                Err(error) => return Err(error),
            }
        }
        _ => None,
    };
    if let Some(sha) = merge_base {
        return Ok((sha, spec.clone()));
    }
    if let Spec::Branch { base } = spec {
        notices.push(Notice::BaseMissing { base: base.clone() });
    }
    if has_head {
        return Ok(("HEAD".to_owned(), Spec::WorkTree));
    }
    let tree = run(&["hash-object", "-t", "tree", "/dev/null"])?;
    Ok((text(&tree), Spec::WorkTree))
}

/// The diff of the working tree for `spec`, with the untracked files as added files. `git` answers
/// every call to it, so tests need no repository. Untracked files are read from `root`.
pub fn load(
    root: &Path,
    spec: &Spec,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Diff, GitError> {
    let root_arg = root.to_string_lossy().into_owned();
    let mut run = |args: &[&str]| {
        let args = ["-C", &root_arg]
            .into_iter()
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        git(&args)
    };
    let mut notices = Vec::new();
    let (rev, spec) = revision(&mut run, spec, &mut notices)?;
    let patch = run(&[
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--submodule=short",
        "-M",
        "-U3",
        &rev,
        "--",
    ])?;
    let mut files = parse(&patch, MAX_PATCH);
    let mut budget = MAX_PATCH.saturating_sub(patch.len());
    let mut capped = patch.len() > MAX_PATCH;
    match run(&["ls-files", "--others", "--exclude-standard", "-z"]) {
        Ok(listing) => files.extend(
            listing
                .split(|&byte| byte == 0)
                .filter_map(|path| RelPath::parse(&lossy(path)))
                .map(|path| untracked(root, &path, &mut budget, &mut capped)),
        ),
        Err(_) => notices.push(Notice::UntrackedNotShown),
    }
    if capped {
        notices.push(Notice::PatchCapped);
    }
    files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    Ok(Diff {
        files,
        spec,
        notices,
    })
}

/// An untracked file as one hunk of added rows. A file that cannot be read, is not a plain file or
/// link, is over 1 MiB or holds a NUL byte is listed without rows. A file that would push the
/// total over the cap is `TooLarge`.
fn untracked(root: &Path, path: &RelPath, budget: &mut usize, capped: &mut bool) -> DiffFile {
    let full = root.join(path.as_str());
    let listed = |change| DiffFile::listed(path.clone(), change);
    let Ok(meta) = std::fs::symlink_metadata(&full) else {
        return listed(Change::Untracked);
    };
    let content = if meta.is_symlink() {
        std::fs::read_link(&full).map(|target| target.into_os_string().into_encoded_bytes())
    } else if meta.is_file() && meta.len() > MAX_UNTRACKED {
        return listed(Change::TooLarge);
    } else if meta.is_file() {
        std::fs::read(&full)
    } else {
        return listed(Change::Untracked);
    };
    let Ok(content) = content else {
        return listed(Change::Untracked);
    };
    if content.contains(&0) {
        return listed(Change::Binary);
    }
    if content.len() > *budget {
        *capped = true;
        return listed(Change::TooLarge);
    }
    *budget -= content.len();
    DiffFile {
        hunks: all_added(&content),
        ..listed(Change::Untracked)
    }
}

/// The hunk `git` would print for a new file: every line added.
fn all_added(content: &[u8]) -> Vec<Hunk> {
    let rows = content
        .split_inclusive(|&byte| byte == b'\n')
        .zip(1..)
        .map(|(line, number)| {
            let body = line.strip_suffix(b"\n");
            Row {
                kind: RowKind::Added,
                old: None,
                new: Some(number),
                text: row_text(body.unwrap_or(line)),
                no_newline: body.is_none(),
            }
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Vec::new();
    }
    let header = format!("@@ -0,0 +1,{} @@", rows.len());
    vec![Hunk { header, rows }]
}

impl Diff {
    /// The file a comment on `path` belongs to: the one with that path, or the one renamed from it
    /// when an agent cited the old name.
    pub fn file_index(&self, path: &RelPath) -> Option<usize> {
        self.files
            .iter()
            .position(|file| file.path == *path || file.old_path.as_ref() == Some(path))
    }
}

/// Where a thread lands in the diff on screen. Computed on every load and never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The anchored text is on `line`, on the anchor's side. A file comment has no line.
    Matched { line: Option<u32> },
    /// The file is in the diff but its anchored text is not. `near` is the line on the anchor's
    /// side, in a hunk, closest to where the comment was written.
    Outdated { near: Option<u32> },
    /// The file is not in the diff, or the comment was written against the other spec.
    NotInDiff,
}

/// Place one thread (ADR 0008). A comment written against another spec is `NotInDiff`. Otherwise,
/// in the thread's file: the row holding the anchored text nearest the anchored line, which is the
/// anchored line itself while it still holds that text, else `Outdated`. A range is placed by its
/// first line.
pub fn place(thread: &Thread, diff: &Diff) -> Placement {
    let anchor = &thread.anchor;
    let file = diff
        .file_index(&anchor.path)
        .and_then(|index| diff.files.get(index));
    let Some(file) = file.filter(|_| anchor.spec == diff.spec) else {
        return Placement::NotInDiff;
    };
    let (side, line, text) = match &anchor.target {
        AnchorTarget::File => return Placement::Matched { line: None },
        AnchorTarget::Line { side, line, text }
        | AnchorTarget::Range {
            side,
            start: line,
            text,
            ..
        } => (*side, *line, text.trim_end_matches('\r')),
    };
    let distance = |(number, _): &(u32, &Row)| (number.abs_diff(line), *number);
    let same = file
        .rows(side)
        .filter(|(_, row)| row.text == text)
        .min_by_key(distance);
    match same {
        Some((number, _)) => Placement::Matched { line: Some(number) },
        None => Placement::Outdated {
            near: file
                .rows(side)
                .min_by_key(distance)
                .map(|(number, _)| number),
        },
    }
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

    const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// A `git` that answers from fields and records each call, without `-C <root>`.
    struct Fake {
        error: Option<GitError>,
        head: bool,
        merge_base: Option<&'static str>,
        refs: Vec<&'static str>,
        patch: Vec<u8>,
        others: Option<Vec<u8>>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(patch: &[u8]) -> Self {
            Self {
                error: None,
                head: true,
                merge_base: Some("abc123"),
                refs: vec!["main"],
                patch: patch.to_vec(),
                others: Some(Vec::new()),
                calls: std::cell::RefCell::default(),
            }
        }

        fn call(&self, args: &[String]) -> Result<Vec<u8>, GitError> {
            let failed = || GitError::Failed {
                args: args.join(" "),
                stderr: String::new(),
            };
            let args = args.get(2..).unwrap_or_default();
            self.calls.borrow_mut().push(args.join(" "));
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            match args.as_slice() {
                ["rev-parse", "--verify", "--quiet", "HEAD"] if self.head => Ok(b"abc\n".to_vec()),
                ["rev-parse", "--verify", "--quiet", name] if self.refs.contains(name) => {
                    Ok(b"abc\n".to_vec())
                }
                ["merge-base", ..] => self
                    .merge_base
                    .map(|sha| format!("{sha}\n").into_bytes())
                    .ok_or_else(failed),
                ["hash-object", "-t", "tree", "/dev/null"] => Ok(format!("{TREE}\n").into_bytes()),
                ["-c", _, "diff", ..] => Ok(self.patch.clone()),
                ["ls-files", ..] => self.others.clone().ok_or_else(failed),
                _ => Err(failed()),
            }
        }

        fn load(&self, spec: &Spec) -> Result<Diff, GitError> {
            load(Path::new("/repo"), spec, |args| self.call(args))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn branch(base: &str) -> Spec {
        Spec::Branch { base: base.into() }
    }

    fn paths(diff: &Diff) -> Vec<&str> {
        diff.files.iter().map(|file| file.path.as_str()).collect()
    }

    const DIFF_ARGS: &str = "-c core.quotePath=false diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ --submodule=short -M -U3";

    #[test]
    fn the_working_tree_spec_diffs_against_head_with_the_fixed_flags() {
        let fake = Fake::new(b"");
        let diff = fake.load(&Spec::WorkTree).unwrap();
        assert_eq!(diff.spec, Spec::WorkTree);
        assert_eq!(
            fake.calls(),
            [
                "rev-parse --verify --quiet HEAD".to_owned(),
                format!("{DIFF_ARGS} HEAD --"),
                "ls-files --others --exclude-standard -z".to_owned(),
            ]
        );
    }

    #[test]
    fn the_branch_spec_diffs_the_working_tree_against_the_merge_base() {
        let fake = Fake::new(b"");
        let diff = fake.load(&branch("main")).unwrap();
        assert_eq!(diff.spec, branch("main"));
        assert!(diff.notices.is_empty());
        let calls = fake.calls();
        assert!(calls.contains(&"merge-base main HEAD".to_owned()));
        assert!(calls.contains(&format!("{DIFF_ARGS} abc123 --")));
    }

    #[test]
    fn not_a_repository_and_no_git_are_errors() {
        let mut fake = Fake::new(b"");
        fake.error = Some(GitError::NotARepo);
        assert_eq!(fake.load(&Spec::WorkTree), Err(GitError::NotARepo));
        fake.error = Some(GitError::NotInstalled);
        assert_eq!(fake.load(&branch("main")), Err(GitError::NotInstalled));
    }

    #[test]
    fn a_failed_diff_is_an_error_with_the_stderr() {
        let mut fake = Fake::new(b"");
        fake.error = Some(GitError::Failed {
            args: "diff".into(),
            stderr: "fatal: bad object".into(),
        });
        assert!(matches!(
            fake.load(&Spec::WorkTree),
            Err(GitError::Failed { .. })
        ));
    }

    #[test]
    fn without_commits_the_diff_is_against_the_empty_tree() {
        let mut fake = Fake::new(b"");
        fake.head = false;
        let diff = fake.load(&Spec::WorkTree).unwrap();
        assert!(diff.notices.is_empty());
        assert!(fake.calls().contains(&format!("{DIFF_ARGS} {TREE} --")));
    }

    #[test]
    fn without_commits_a_branch_spec_falls_back_with_a_notice() {
        let mut fake = Fake::new(b"");
        fake.head = false;
        let diff = fake.load(&branch("main")).unwrap();
        assert_eq!(diff.spec, Spec::WorkTree);
        assert_eq!(
            diff.notices,
            [Notice::BaseMissing {
                base: "main".into()
            }]
        );
        assert!(fake.calls().contains(&format!("{DIFF_ARGS} {TREE} --")));
    }

    #[test]
    fn a_missing_base_falls_back_to_the_working_tree_with_a_notice() {
        let mut fake = Fake::new(MIXED);
        fake.merge_base = None;
        let diff = fake.load(&branch("nope")).unwrap();
        assert_eq!(diff.spec, Spec::WorkTree);
        assert_eq!(
            diff.notices,
            [Notice::BaseMissing {
                base: "nope".into()
            }]
        );
        assert!(fake.calls().contains(&format!("{DIFF_ARGS} HEAD --")));
        assert!(!diff.files.is_empty());
    }

    #[test]
    fn a_base_that_looks_like_an_option_never_reaches_git() {
        let fake = Fake::new(b"");
        let diff = fake.load(&branch("--output=/x")).unwrap();
        assert_eq!(diff.spec, Spec::WorkTree);
        assert!(!fake.calls().iter().any(|call| call.contains("merge-base")));
    }

    #[test]
    fn an_empty_diff_has_no_files_and_no_notice() {
        let diff = Fake::new(b"").load(&Spec::WorkTree).unwrap();
        assert!(diff.files.is_empty());
        assert!(diff.notices.is_empty());
    }

    #[test]
    fn a_failed_ls_files_keeps_the_diff_and_adds_a_notice() {
        let mut fake = Fake::new(MIXED);
        fake.others = None;
        let diff = fake.load(&Spec::WorkTree).unwrap();
        assert_eq!(diff.notices, [Notice::UntrackedNotShown]);
        assert_eq!(diff.files.len(), 11);
    }

    #[test]
    fn binary_rename_submodule_and_crlf_files_load_from_the_patch() {
        let diff = Fake::new(MIXED).load(&Spec::WorkTree).unwrap();
        let change = |path: &str| find(&diff.files, path).change;
        assert_eq!(change("bin.dat"), Change::Binary);
        assert_eq!(change("new_name.txt"), Change::Renamed);
        assert_eq!(change("subm"), Change::Submodule);
        assert_eq!(
            texts(find(&diff.files, "crlf.txt"))[2],
            (RowKind::Added, "B")
        );
    }

    #[test]
    fn a_patch_over_the_cap_collapses_the_files_past_it() {
        let big = |name: &str| {
            let line = "x".repeat(2 * 1024 * 1024);
            format!(
                "diff --git a/{name} b/{name}\nnew file mode 100644\n--- /dev/null\n+++ b/{name}\n@@ -0,0 +1 @@\n+{line}\n"
            )
        };
        let patch = [big("one"), big("two"), big("three")].concat();
        let diff = Fake::new(patch.as_bytes()).load(&Spec::WorkTree).unwrap();
        let changes = diff
            .files
            .iter()
            .map(|file| file.change)
            .collect::<Vec<_>>();
        assert_eq!(changes, [Change::Added, Change::TooLarge, Change::TooLarge]);
        assert_eq!(diff.notices, [Notice::PatchCapped]);
        assert_eq!(paths(&diff), ["one", "three", "two"]);
    }

    fn temp_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("herdr-review-diff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn untracked_files_are_added_files_listed_with_the_tracked_ones() {
        let root = temp_repo("untracked");
        std::fs::write(root.join("z.txt"), b"one\r\ntwo").unwrap();
        std::fs::write(root.join("empty"), b"").unwrap();
        std::fs::write(root.join("data.bin"), b"a\0b").unwrap();
        std::fs::write(root.join("big"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        std::fs::create_dir(root.join("nested")).unwrap();
        std::os::unix::fs::symlink("target/file", root.join("link")).unwrap();
        let mut fake = Fake::new(MIXED);
        fake.others = Some(b"z.txt\0empty\0data.bin\0big\0nested/\0link\0gone\0".to_vec());
        let diff = load(&root, &Spec::WorkTree, |args| fake.call(args)).unwrap();
        let change = |path: &str| find(&diff.files, path).change;
        assert_eq!(change("data.bin"), Change::Binary);
        assert_eq!(change("big"), Change::TooLarge);
        assert_eq!(change("empty"), Change::Untracked);
        assert_eq!(change("nested/"), Change::Untracked);
        assert_eq!(change("gone"), Change::Untracked);
        assert!(find(&diff.files, "gone").hunks.is_empty());
        let z = find(&diff.files, "z.txt");
        assert_eq!(z.change, Change::Untracked);
        assert_eq!(z.hunks[0].header, "@@ -0,0 +1,2 @@");
        let rows = &z.hunks[0].rows;
        assert_eq!(
            (rows[0].new, rows[0].text.as_str(), rows[0].no_newline),
            (Some(1), "one", false)
        );
        assert_eq!(
            (rows[1].new, rows[1].text.as_str(), rows[1].no_newline),
            (Some(2), "two", true)
        );
        assert_eq!(
            texts(find(&diff.files, "link")),
            [(RowKind::Added, "target/file")]
        );
        assert!(find(&diff.files, "empty").hunks.is_empty());
        assert!(diff.notices.is_empty());
        let order = paths(&diff);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_untracked_file_past_the_total_cap_is_too_large() {
        let root = temp_repo("budget");
        std::fs::write(root.join("late.txt"), vec![b'y'; 1000]).unwrap();
        let name = "first";
        let line = "x".repeat(MAX_PATCH - 200);
        let patch = format!(
            "diff --git a/{name} b/{name}\nnew file mode 100644\n--- /dev/null\n+++ b/{name}\n@@ -0,0 +1 @@\n+{line}\n"
        );
        assert!(patch.len() < MAX_PATCH);
        let mut fake = Fake::new(patch.as_bytes());
        fake.others = Some(b"late.txt\0".to_vec());
        let diff = load(&root, &Spec::WorkTree, |args| fake.call(args)).unwrap();
        assert_eq!(find(&diff.files, "first").change, Change::Added);
        assert_eq!(find(&diff.files, "late.txt").change, Change::TooLarge);
        assert_eq!(diff.notices, [Notice::PatchCapped]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_default_base_is_the_first_ref_that_resolves() {
        let mut fake = Fake::new(b"");
        fake.refs = vec!["origin/HEAD", "main"];
        let root = Path::new("/repo");
        assert_eq!(
            default_base(root, |args| fake.call(args)).unwrap(),
            "origin/HEAD"
        );
        fake.refs = vec!["master"];
        assert_eq!(
            default_base(root, |args| fake.call(args)).unwrap(),
            "master"
        );
        fake.refs = vec!["main", "master"];
        assert_eq!(default_base(root, |args| fake.call(args)).unwrap(), "main");
        fake.refs = vec![];
        assert_eq!(
            default_base(root, |args| fake.call(args)),
            Err(GitError::NoBase {
                tried: vec!["origin/HEAD".into(), "main".into(), "master".into()]
            })
        );
        fake.error = Some(GitError::NotInstalled);
        assert_eq!(
            default_base(root, |args| fake.call(args)),
            Err(GitError::NotInstalled)
        );
    }

    use crate::store::{Anchor, Author, Comment, CommentId, Status};

    const A_RS: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,5 +1,6 @@
 one
 two
-three
+THREE
+three and a half
 four
 five
";

    fn diff_of(patch: &str, spec: Spec) -> Diff {
        Diff {
            files: parse(patch.as_bytes(), MAX_PATCH),
            spec,
            notices: Vec::new(),
        }
    }

    fn thread(path: &str, spec: Spec, target: AnchorTarget) -> Thread {
        Thread {
            root: Comment {
                id: CommentId::parse("u1").unwrap(),
                parent: None,
                author: Author::User,
                body: "fix".into(),
                sent_batch: None,
                edited_since_sent: false,
            },
            anchor: Anchor {
                path: RelPath::parse(path).unwrap(),
                old_path: None,
                target,
                spec,
            },
            replies: Vec::new(),
            status: Status::Open,
            is_new: false,
            reopened: false,
            unsent: true,
        }
    }

    fn line(side: Side, line: u32, text: &str) -> AnchorTarget {
        AnchorTarget::Line {
            side,
            line,
            text: text.into(),
        }
    }

    fn placed(target: AnchorTarget) -> Placement {
        let diff = diff_of(A_RS, Spec::WorkTree);
        place(&thread("a.rs", Spec::WorkTree, target), &diff)
    }

    #[test]
    fn a_line_with_the_same_text_at_the_same_place_is_matched_there() {
        assert_eq!(
            placed(line(Side::New, 3, "THREE")),
            Placement::Matched { line: Some(3) }
        );
        assert_eq!(
            placed(line(Side::Old, 3, "three")),
            Placement::Matched { line: Some(3) }
        );
    }

    #[test]
    fn a_line_that_moved_is_matched_at_its_new_line() {
        assert_eq!(
            placed(line(Side::New, 2, "four")),
            Placement::Matched { line: Some(5) }
        );
    }

    #[test]
    fn of_two_lines_with_the_same_text_the_nearer_one_wins_and_a_tie_goes_to_the_earlier() {
        let patch = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,5 +1,5 @@
 x
 a
 x
 b
 x
";
        let diff = diff_of(patch, Spec::WorkTree);
        let at = |n| {
            place(
                &thread("a.rs", Spec::WorkTree, line(Side::New, n, "x")),
                &diff,
            )
        };
        assert_eq!(at(1), Placement::Matched { line: Some(1) });
        assert_eq!(at(4), Placement::Matched { line: Some(3) });
        assert_eq!(at(2), Placement::Matched { line: Some(1) });
        assert_eq!(at(40), Placement::Matched { line: Some(5) });
    }

    #[test]
    fn text_that_is_gone_is_outdated_near_where_it_was() {
        assert_eq!(
            placed(line(Side::New, 3, "the old text")),
            Placement::Outdated { near: Some(3) }
        );
        assert_eq!(
            placed(line(Side::New, 40, "the old text")),
            Placement::Outdated { near: Some(6) }
        );
    }

    #[test]
    fn a_side_only_sees_its_own_rows() {
        // THREE was added, so the old side never held it.
        assert_eq!(
            placed(line(Side::Old, 3, "THREE")),
            Placement::Outdated { near: Some(3) }
        );
        assert_eq!(
            placed(line(Side::New, 3, "three")),
            Placement::Outdated { near: Some(3) }
        );
    }

    #[test]
    fn a_file_that_is_not_in_the_diff_is_not_in_the_diff() {
        let diff = diff_of(A_RS, Spec::WorkTree);
        let other = thread("b.rs", Spec::WorkTree, line(Side::New, 3, "THREE"));
        assert_eq!(place(&other, &diff), Placement::NotInDiff);
        let empty = diff_of("", Spec::WorkTree);
        let same = thread("a.rs", Spec::WorkTree, line(Side::New, 3, "THREE"));
        assert_eq!(place(&same, &empty), Placement::NotInDiff);
    }

    #[test]
    fn a_thread_written_against_the_other_spec_is_not_in_the_diff() {
        let branch = Spec::Branch {
            base: "main".into(),
        };
        let thread = thread("a.rs", branch.clone(), line(Side::New, 3, "THREE"));
        let on_tree = diff_of(A_RS, Spec::WorkTree);
        assert_eq!(place(&thread, &on_tree), Placement::NotInDiff);
        let on_branch = diff_of(A_RS, branch);
        assert_eq!(
            place(&thread, &on_branch),
            Placement::Matched { line: Some(3) }
        );
        let other_base = diff_of(
            A_RS,
            Spec::Branch {
                base: "develop".into(),
            },
        );
        assert_eq!(place(&thread, &other_base), Placement::NotInDiff);
    }

    #[test]
    fn a_range_is_placed_by_its_first_line() {
        let range = |text: &str| AnchorTarget::Range {
            side: Side::New,
            start: 3,
            end: 5,
            text: text.into(),
        };
        assert_eq!(placed(range("THREE")), Placement::Matched { line: Some(3) });
        assert_eq!(
            placed(range("something else")),
            Placement::Outdated { near: Some(3) }
        );
    }

    #[test]
    fn editing_the_line_above_a_commented_line_leaves_the_thread_matched() {
        let target = line(Side::New, 5, "four");
        assert_eq!(placed(target.clone()), Placement::Matched { line: Some(5) });
        let edited = A_RS.replace("+three and a half", "+an edit");
        let diff = diff_of(&edited, Spec::WorkTree);
        assert_ne!(edited, A_RS);
        assert_eq!(
            place(&thread("a.rs", Spec::WorkTree, target), &diff),
            Placement::Matched { line: Some(5) }
        );
    }

    #[test]
    fn a_file_comment_is_matched_while_the_file_is_in_the_diff() {
        let diff = diff_of(A_RS, Spec::WorkTree);
        let on = |path| place(&thread(path, Spec::WorkTree, AnchorTarget::File), &diff);
        assert_eq!(on("a.rs"), Placement::Matched { line: None });
        assert_eq!(on("b.rs"), Placement::NotInDiff);
    }

    #[test]
    fn a_renamed_file_is_found_by_either_path() {
        let diff = Diff {
            files: parse(MIXED, MAX_PATCH),
            spec: Spec::WorkTree,
            notices: Vec::new(),
        };
        let on =
            |path, side, n, text| place(&thread(path, Spec::WorkTree, line(side, n, text)), &diff);
        assert_eq!(
            on("new_name.txt", Side::New, 5, "CHANGED"),
            Placement::Matched { line: Some(5) }
        );
        assert_eq!(
            on("old_name.txt", Side::Old, 5, "l5"),
            Placement::Matched { line: Some(5) }
        );
    }

    #[test]
    fn carriage_returns_in_the_stored_text_do_not_matter() {
        assert_eq!(
            placed(line(Side::New, 3, "THREE\r")),
            Placement::Matched { line: Some(3) }
        );
    }

    #[test]
    fn a_line_comment_on_a_file_with_no_rows_is_outdated_with_no_line() {
        let diff = Diff {
            files: parse(MIXED, MAX_PATCH),
            spec: Spec::WorkTree,
            notices: Vec::new(),
        };
        let binary = thread("bin.dat", Spec::WorkTree, line(Side::New, 1, "x"));
        assert_eq!(place(&binary, &diff), Placement::Outdated { near: None });
    }
}
