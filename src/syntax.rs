//! Syntax colours for the code rows of a diff.
//!
//! `highlight` turns the text of a file into tokens per line, and it is the only place that knows
//! which engine does it. `Cache` keeps the tokens of the lines the diff shows, per file, and reads
//! each side of a file the first time the file comes on screen. A build without the `syntax`
//! feature knows no language, so every row draws plain.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use crate::diff::{DiffFile, Row, RowKind};
use crate::store::Side;
use crate::theme::Token;
use crate::tui::Git;

/// A token of one line: the bytes of the line it covers, and what it is.
pub type Span = (Range<usize>, Token);

/// A side of a file larger than this is not read. Its hunks are highlighted on their own.
const MAX_FILE: usize = 1024 * 1024;

#[cfg(feature = "syntax")]
mod engine {
    use std::sync::LazyLock;

    use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
    use syntect::util::LinesWithEndings;

    use super::Span;
    use crate::theme::{SCOPES, Token};

    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

    /// The theme's scope table, with each scope parsed once.
    static TABLE: LazyLock<Vec<(Scope, Token)>> = LazyLock::new(|| {
        SCOPES
            .iter()
            .filter_map(|(scope, token)| Some((Scope::new(scope).ok()?, *token)))
            .collect()
    });

    /// A language the engine can split into tokens.
    #[derive(Debug, Clone, Copy)]
    pub struct Language(&'static SyntaxReference);

    /// The language of the file at `path`, from its name and then its extension.
    pub fn language(path: &str) -> Option<Language> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let extension = name
            .rsplit_once('.')
            .map_or(name, |(_, extension)| extension);
        SYNTAXES
            .find_syntax_by_extension(name)
            .or_else(|| SYNTAXES.find_syntax_by_extension(extension))
            .map(Language)
    }

    /// The token of the innermost scope the theme has a colour for.
    fn token_of(stack: &ScopeStack) -> Option<Token> {
        stack.as_slice().iter().rev().find_map(|scope| {
            let found = TABLE.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope));
            found.map(|(_, token)| *token)
        })
    }

    /// The tokens of each line of `text`, which is a whole file or a run of lines from one. A
    /// line the engine cannot parse has no tokens.
    pub fn highlight(text: &str, language: Language) -> Vec<Vec<Span>> {
        let mut state = ParseState::new(language.0);
        let mut stack = ScopeStack::new();
        let mut lines = Vec::new();
        for line in LinesWithEndings::from(text) {
            let mut spans: Vec<Span> = Vec::new();
            let Ok(ops) = state.parse_line(line, &SYNTAXES) else {
                lines.push(spans);
                continue;
            };
            let end = line.trim_end_matches(['\n', '\r']).len();
            let mut emit = |stack: &ScopeStack, from: usize, to: usize| {
                let to = to.min(end);
                let Some(token) = token_of(stack).filter(|_| from < to) else {
                    return;
                };
                match spans.last_mut() {
                    Some((last, same)) if last.end == from && *same == token => last.end = to,
                    _ => spans.push((from..to, token)),
                }
            };
            let mut start = 0;
            for (at, op) in &ops {
                emit(&stack, start, *at);
                start = *at;
                let _ = stack.apply(op);
            }
            emit(&stack, start, line.len());
            lines.push(spans);
        }
        lines
    }
}

#[cfg(not(feature = "syntax"))]
mod engine {
    use super::Span;

    /// No language is known without the `syntax` feature.
    #[derive(Debug, Clone, Copy)]
    pub enum Language {}

    pub fn language(_path: &str) -> Option<Language> {
        None
    }

    pub fn highlight(_text: &str, language: Language) -> Vec<Vec<Span>> {
        match language {}
    }
}

pub use engine::{Language, highlight, language};

/// The tokens of the lines of one file that the diff shows. A line that is not here draws plain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTokens {
    old: HashMap<u32, Vec<Span>>,
    new: HashMap<u32, Vec<Span>>,
}

impl FileTokens {
    /// The tokens of line `line` on `side`, or `None` when the line was not highlighted.
    pub fn line(&self, side: Side, line: u32) -> Option<&[Span]> {
        let lines = match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        };
        lines.get(&line).map(Vec::as_slice)
    }
}

/// The side whose text a row is read from. A context row is the same on both, so it is new.
pub const fn side_of(row: &Row) -> Side {
    match row.kind {
        RowKind::Removed => Side::Old,
        RowKind::Added | RowKind::Context => Side::New,
    }
}

/// The tokens of the rows of `file` on `side`. With the text of the whole file, each row takes the
/// tokens of its line, when the line still reads as the row does. Without it, the rows of each
/// hunk on that side are highlighted as a snippet, which cannot know that the hunk starts inside
/// a block comment or a string.
fn side_tokens(
    file: &DiffFile,
    side: Side,
    text: Option<&str>,
    language: Language,
) -> HashMap<u32, Vec<Span>> {
    let mut tokens = HashMap::new();
    if let Some(text) = text {
        let rows = file.hunks.iter().flat_map(|hunk| &hunk.rows);
        let needed = rows.filter_map(|row| row.line(side).filter(|_| side_of(row) == side));
        let last = needed.max().unwrap_or(0) as usize;
        // The parse has to start at line 1, but it can stop after the last line a hunk shows.
        // ponytail: a hunk at the end of a 1 MiB file still parses the whole file on the UI
        // thread. Move the parse to a worker thread if that stall shows up.
        let end = text
            .match_indices('\n')
            .nth(last.saturating_sub(1))
            .map_or(text.len(), |(at, _)| at + 1);
        let prefix = text.get(..end).unwrap_or(text);
        let mut lines = highlight(prefix, language);
        let source = prefix.split('\n').collect::<Vec<_>>();
        for row in file.hunks.iter().flat_map(|hunk| &hunk.rows) {
            let Some(line) = row.line(side).filter(|_| side_of(row) == side) else {
                continue;
            };
            let index = (line as usize).saturating_sub(1);
            if source.get(index) == Some(&row.text.as_str()) {
                let spans = lines.get_mut(index).map(std::mem::take);
                tokens.insert(line, spans.unwrap_or_default());
            }
        }
        return tokens;
    }
    for hunk in &file.hunks {
        let rows = hunk
            .rows
            .iter()
            .filter_map(|row| Some((row, row.line(side)?)))
            .collect::<Vec<_>>();
        let mut snippet = String::new();
        for (row, _) in &rows {
            snippet.push_str(&row.text);
            snippet.push('\n');
        }
        for ((row, line), spans) in rows.iter().zip(highlight(&snippet, language)) {
            if side_of(row) == side {
                tokens.insert(*line, spans);
            }
        }
    }
    tokens
}

/// The new side of `file` from the work tree, and its old side from `git show <rev>:<path>`. A
/// side with no row to colour is not read. A side that cannot be read, is not UTF-8 or is over
/// 1 MiB is `None`.
fn tokenise(
    root: &Path,
    rev: &str,
    file: &DiffFile,
    language: Language,
    git: &mut Git,
) -> FileTokens {
    let has = |side| {
        let mut rows = file.hunks.iter().flat_map(|hunk| &hunk.rows);
        rows.any(|row| side_of(row) == side)
    };
    let small = |text: &String| text.len() <= MAX_FILE;
    let mut tokens = FileTokens::default();
    if has(Side::New) {
        let path = root.join(file.path.as_str());
        let fits = std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_FILE as u64);
        let text = fits.then(|| std::fs::read_to_string(&path).ok()).flatten();
        tokens.new = side_tokens(file, Side::New, text.as_deref(), language);
    }
    if has(Side::Old) {
        let path = file.old_path.as_ref().unwrap_or(&file.path);
        let args = [
            "-C".to_owned(),
            root.to_string_lossy().into_owned(),
            "show".to_owned(),
            format!("{rev}:{}", path.as_str()),
        ];
        // ponytail: git hands over the whole blob before its size is known. Ask `cat-file -s`
        // first if large old sides turn out to cost time.
        let text = git(&args)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(small);
        tokens.old = side_tokens(file, Side::Old, text.as_deref(), language);
    }
    tokens
}

/// The tokens of every file that has been on screen since the diff was loaded.
#[derive(Debug, Default)]
pub struct Cache {
    files: HashMap<String, FileTokens>,
}

impl Cache {
    /// Forget everything. The diff was loaded against another revision, so every file may read
    /// differently.
    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Keep the tokens of the files whose path `keep` accepts, and forget the rest.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.files.retain(|path, _| keep(path));
    }

    /// The tokens of the file at `path`, when it has been highlighted.
    pub fn file(&self, path: &str) -> Option<&FileTokens> {
        self.files.get(path)
    }

    /// Highlight `file` unless it has been. `rev` is the revision the diff compares against. A
    /// file in a language the engine does not know is remembered as having no tokens.
    pub fn ensure(&mut self, root: &Path, rev: &str, file: &DiffFile, git: &mut Git) {
        if self.files.contains_key(file.path.as_str()) {
            return;
        }
        let tokens = language(file.path.as_str())
            .map(|language| tokenise(root, rev, file, language, git))
            .unwrap_or_default();
        self.files.insert(file.path.as_str().to_owned(), tokens);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_extension_has_no_language_and_no_tokens() {
        assert!(language("notes.zzz").is_none());
        assert!(language("no_extension_at_all_xyz").is_none());
        let file = crate::diff::parse(
            b"diff --git a/notes.zzz b/notes.zzz\n--- a/notes.zzz\n+++ b/notes.zzz\n@@ -1 +1 @@\n-let a = 1;\n+let a = 2;\n",
            crate::diff::MAX_PATCH,
        )
        .remove(0);
        let mut cache = Cache::default();
        let mut calls = 0;
        cache.ensure(Path::new("/nonexistent"), "HEAD", &file, &mut |_| {
            calls += 1;
            Ok(Vec::new())
        });
        assert_eq!(calls, 0, "a file with no language is not read");
        let tokens = cache.file("notes.zzz").unwrap();
        assert_eq!(tokens.line(Side::New, 1), None);
        assert_eq!(tokens.line(Side::Old, 1), None);
    }
}

#[cfg(all(test, feature = "syntax"))]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod engine_tests {
    use super::*;
    use crate::diff::{GitError, MAX_PATCH, parse};

    /// Lines 2 and 5 sit inside a block comment and a template string that open on other lines.
    const NEW: &str =
        "/* start\n still a comment\n end */\nconst s = `a\nb and more\nc`;\nlet n = 42;\n";
    const OLD: &str = "/* start\n was a comment\n end */\nconst s = `a\nb\nc`;\nlet n = 42;\n";
    const PATCH: &[u8] = b"diff --git a/app.js b/app.js\n--- a/app.js\n+++ b/app.js\n@@ -2 +2 @@\n- was a comment\n+ still a comment\n@@ -5 +5 @@\n-b\n+b and more\n";

    fn tokens_of(line: &[Span]) -> Vec<Token> {
        line.iter().map(|(_, token)| *token).collect()
    }

    /// A repository root holding `app.js` with `text`, or nothing when `text` is `None`.
    fn root(name: &str, text: Option<&str>) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("herdr-review-syntax-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        if let Some(text) = text {
            std::fs::write(root.join("app.js"), text).unwrap();
        }
        root
    }

    fn file() -> DiffFile {
        parse(PATCH, MAX_PATCH).remove(0)
    }

    #[test]
    fn the_language_comes_from_the_file_name() {
        for path in [
            "src/main.rs",
            "web/app.ts",
            "web/view.tsx",
            "Cargo.toml",
            "a/b/c.py",
            "Makefile",
        ] {
            assert!(language(path).is_some(), "{path}");
        }
    }

    #[test]
    fn a_line_is_split_into_the_tokens_the_theme_has_colours_for() {
        let rust = language("a.rs").unwrap();
        let lines = highlight("fn main() { let n = 42; } // done\n", rust);
        let line = &lines[0];
        let text = "fn main() { let n = 42; } // done";
        let at = |needle: &str| {
            let start = text.find(needle).unwrap();
            line.iter()
                .find(|(range, _)| range.start <= start && start < range.end)
                .map(|(_, token)| *token)
        };
        assert_eq!(at("fn"), Some(Token::Keyword));
        assert_eq!(at("main"), Some(Token::Function));
        assert_eq!(at("42"), Some(Token::Number));
        assert_eq!(at("// done"), Some(Token::Comment));
        // No token reaches past the text, and they come in order without overlap.
        let mut end = 0;
        for (range, _) in line {
            assert!(range.start >= end && range.end <= text.len(), "{range:?}");
            end = range.end;
        }
    }

    #[test]
    fn a_block_comment_and_a_template_string_are_coloured_from_the_whole_file() {
        let root = root("whole", Some(NEW));
        let mut cache = Cache::default();
        cache.ensure(&root, "HEAD", &file(), &mut |_| Ok(OLD.as_bytes().to_vec()));
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(
            tokens_of(tokens.line(Side::New, 2).unwrap()),
            [Token::Comment]
        );
        assert_eq!(
            tokens_of(tokens.line(Side::New, 5).unwrap()),
            [Token::String]
        );
        assert_eq!(
            tokens_of(tokens.line(Side::Old, 2).unwrap()),
            [Token::Comment]
        );
        assert_eq!(
            tokens_of(tokens.line(Side::Old, 5).unwrap()),
            [Token::String]
        );
        // The whole line is covered, to its last byte.
        let line = tokens.line(Side::New, 2).unwrap();
        assert_eq!(line[0].0, 0.." still a comment".len());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_side_that_cannot_be_read_is_highlighted_hunk_by_hunk() {
        // No file in the work tree, and `git show` fails.
        let root = root("snippet", None);
        let mut cache = Cache::default();
        cache.ensure(&root, "HEAD", &file(), &mut |args| {
            Err(GitError::Failed {
                args: args.join(" "),
                stderr: String::new(),
            })
        });
        let tokens = cache.file("app.js").unwrap();
        // Alone, the lines are not a comment and not a string, but they are still highlighted.
        for side in [Side::New, Side::Old] {
            for line in [2, 5] {
                let found = tokens_of(tokens.line(side, line).unwrap());
                assert!(
                    !found.contains(&Token::Comment),
                    "{side:?} {line} {found:?}"
                );
                assert!(!found.contains(&Token::String), "{side:?} {line} {found:?}");
            }
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_over_a_mebibyte_falls_back_to_its_hunks() {
        let big = format!("{NEW}{}", "// pad\n".repeat(MAX_FILE / 7 + 1));
        let root = root("big", Some(&big));
        let mut cache = Cache::default();
        cache.ensure(&root, "HEAD", &file(), &mut |_| {
            Ok(big.clone().into_bytes())
        });
        let tokens = cache.file("app.js").unwrap();
        for side in [Side::New, Side::Old] {
            assert_ne!(tokens_of(tokens.line(side, 2).unwrap()), [Token::Comment]);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_line_that_no_longer_reads_as_its_row_draws_plain() {
        // The file changed after the diff was taken: line 2 is other text now.
        let root = root(
            "stale",
            Some(&NEW.replace("still a comment", "edited since")),
        );
        let mut cache = Cache::default();
        cache.ensure(&root, "HEAD", &file(), &mut |_| Ok(OLD.as_bytes().to_vec()));
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(tokens.line(Side::New, 2), None);
        assert!(tokens.line(Side::New, 5).is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_old_side_of_a_renamed_file_is_read_under_its_old_name() {
        let patch = b"diff --git a/old.js b/app.js\nsimilarity index 80%\nrename from old.js\nrename to app.js\n--- a/old.js\n+++ b/app.js\n@@ -2 +2 @@\n- was a comment\n+ still a comment\n";
        let file = parse(patch, MAX_PATCH).remove(0);
        let root = root("renamed", Some(NEW));
        let mut asked = Vec::new();
        let mut cache = Cache::default();
        cache.ensure(&root, "abc123", &file, &mut |args| {
            asked.push(args.join(" "));
            Ok(OLD.as_bytes().to_vec())
        });
        assert_eq!(asked, [format!("-C {} show abc123:old.js", root.display())]);
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(
            tokens_of(tokens.line(Side::Old, 2).unwrap()),
            [Token::Comment]
        );
        assert_eq!(
            tokens_of(tokens.line(Side::New, 2).unwrap()),
            [Token::Comment]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// 200 lines of JavaScript that open with a block comment, with line `n` read as `edit` when
    /// given, and a newline after the last line unless `open_end`.
    fn long(edit: Option<(u32, &str)>, open_end: bool) -> String {
        let lines = (1..=200u32).map(|n| match (n, edit) {
            (n, Some((at, text))) if n == at => text.to_owned(),
            (1, _) => "/* start".to_owned(),
            (2, _) => " inside".to_owned(),
            (3, _) => " end */".to_owned(),
            (n, _) => format!("let v{n} = {n};"),
        });
        let text = lines.collect::<Vec<_>>().join("\n");
        if open_end { text } else { text + "\n" }
    }

    /// A one-row change of line `n` per hunk, from `old` to `new`, with `tail` after each row.
    fn patch(hunks: &[(u32, &str, &str)], tail: &str) -> DiffFile {
        let head = "diff --git a/app.js b/app.js\n--- a/app.js\n+++ b/app.js\n";
        let body = hunks
            .iter()
            .map(|(n, old, new)| format!("@@ -{n} +{n} @@\n-{old}\n{tail}+{new}\n{tail}"));
        let patch = std::iter::once(head.to_owned())
            .chain(body)
            .collect::<String>();
        parse(patch.as_bytes(), MAX_PATCH).remove(0)
    }

    /// The tokens `highlight` gives line `n` of the whole of `text`.
    fn whole(text: &str, n: u32) -> Vec<Span> {
        highlight(text, language("app.js").unwrap()).remove(n as usize - 1)
    }

    #[test]
    fn a_hunk_near_the_top_gets_the_tokens_of_the_whole_file() {
        let (old, new) = (long(None, false), long(Some((2, " inside now")), false));
        let root = root("top", Some(&new));
        let mut cache = Cache::default();
        let file = patch(&[(2, " inside", " inside now")], "");
        cache.ensure(&root, "HEAD", &file, &mut |_| Ok(old.as_bytes().to_vec()));
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(tokens.line(Side::New, 2).unwrap(), whole(&new, 2));
        assert_eq!(tokens.line(Side::Old, 2).unwrap(), whole(&old, 2));
        assert_eq!(tokens_of(&whole(&new, 2)), [Token::Comment]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_last_hunk_of_a_file_gets_its_tokens() {
        let old = long(None, false);
        let new = long(Some((190, "let changed = 190;")), false);
        let new = new.replace(" inside\n", " inside now\n");
        let root = root("bottom", Some(&new));
        let mut cache = Cache::default();
        let hunks = [
            (2, " inside", " inside now"),
            (190, "let v190 = 190;", "let changed = 190;"),
        ];
        cache.ensure(&root, "HEAD", &patch(&hunks, ""), &mut |_| {
            Ok(old.as_bytes().to_vec())
        });
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(tokens.line(Side::New, 190).unwrap(), whole(&new, 190));
        assert_eq!(tokens.line(Side::Old, 190).unwrap(), whole(&old, 190));
        assert!(tokens_of(&whole(&new, 190)).contains(&Token::Keyword));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_last_line_with_no_newline_gets_its_tokens() {
        let old = long(None, true);
        let new = long(Some((200, "let last = 200;")), true);
        let root = root("open-end", Some(&new));
        let mut cache = Cache::default();
        let file = patch(
            &[(200, "let v200 = 200;", "let last = 200;")],
            "\\ No newline at end of file\n",
        );
        cache.ensure(&root, "HEAD", &file, &mut |_| Ok(old.as_bytes().to_vec()));
        let tokens = cache.file("app.js").unwrap();
        assert_eq!(tokens.line(Side::New, 200).unwrap(), whole(&new, 200));
        assert_eq!(tokens.line(Side::Old, 200).unwrap(), whole(&old, 200));
        assert!(tokens_of(&whole(&new, 200)).contains(&Token::Keyword));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_file_is_read_once_until_the_cache_is_cleared_and_only_the_sides_it_needs() {
        let root = root("cache", Some(NEW));
        let mut calls = 0;
        let mut cache = Cache::default();
        let mut git = |_: &[String]| {
            calls += 1;
            Ok(OLD.as_bytes().to_vec())
        };
        cache.ensure(&root, "HEAD", &file(), &mut git);
        cache.ensure(&root, "HEAD", &file(), &mut git);
        cache.clear();
        assert!(cache.file("app.js").is_none());
        cache.ensure(&root, "HEAD", &file(), &mut git);
        // A file with only added rows has no old side to ask git for.
        let added = parse(
            b"diff --git a/new.js b/new.js\nnew file mode 100644\n--- /dev/null\n+++ b/new.js\n@@ -0,0 +1 @@\n+let n = 42;\n",
            MAX_PATCH,
        )
        .remove(0);
        cache.ensure(&root, "HEAD", &added, &mut git);
        assert_eq!(calls, 2);
        let _ = std::fs::remove_dir_all(root);
    }
}
