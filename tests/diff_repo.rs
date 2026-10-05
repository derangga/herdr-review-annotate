//! `diff::load` against a real `git` in a temporary repository. These check what `git` does, so
//! the unit tests can stand in for it.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

use herdr_review::diff::{Change, Diff, DiffFile, GitError, Notice, load, run_git_bytes};
use herdr_review::store::Spec;

struct Repo(PathBuf);

impl Repo {
    /// A repository on `main` whose local config does everything it can to change `git diff`.
    fn new(name: &str) -> Self {
        let dir = Self::dir(name);
        let repo = Self(dir);
        repo.git(&["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("diff.noprefix", "true"),
            ("diff.mnemonicPrefix", "true"),
            ("diff.renames", "false"),
            ("diff.submodule", "log"),
            ("core.quotePath", "true"),
            ("color.ui", "always"),
            ("commit.gpgsign", "false"),
        ] {
            repo.git(&["config", key, value]);
        }
        repo
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("herdr-review-repo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        std::fs::write(self.0.join(path), content).unwrap();
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    fn load(&self, spec: &Spec) -> Diff {
        load(&self.0, spec, run_git_bytes).unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn find<'a>(diff: &'a Diff, path: &str) -> &'a DiffFile {
    diff.files
        .iter()
        .find(|file| file.path.as_str() == path)
        .unwrap()
}

fn paths(diff: &Diff) -> Vec<&str> {
    diff.files.iter().map(|file| file.path.as_str()).collect()
}

fn branch(base: &str) -> Spec {
    Spec::Branch { base: base.into() }
}

#[test]
fn a_directory_that_is_not_a_repository_is_an_error() {
    let dir = Repo::dir("not-a-repo");
    let result = load(&dir, &Spec::WorkTree, run_git_bytes);
    assert_eq!(result, Err(GitError::NotARepo));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_repository_with_no_commits_diffs_against_the_empty_tree() {
    let repo = Repo::new("no-commits");
    repo.write("staged.txt", "one\n");
    repo.git(&["add", "staged.txt"]);
    repo.write("loose.txt", "two\n");
    let diff = repo.load(&Spec::WorkTree);
    assert!(diff.notices.is_empty());
    assert_eq!(paths(&diff), ["loose.txt", "staged.txt"]);
    assert_eq!(find(&diff, "staged.txt").change, Change::Added);
    assert_eq!(find(&diff, "loose.txt").change, Change::Untracked);
}

#[test]
fn an_untracked_file_is_rendered_as_added_lines() {
    let repo = Repo::new("untracked");
    repo.write("a.txt", "a\n");
    repo.commit("base");
    repo.write("new file.txt", "x\ny\n");
    let diff = repo.load(&Spec::WorkTree);
    assert_eq!(paths(&diff), ["new file.txt"]);
    let file = find(&diff, "new file.txt");
    assert_eq!(file.change, Change::Untracked);
    let rows = file.hunks[0]
        .rows
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rows, ["x", "y"]);
}

#[test]
fn an_untracked_file_over_one_mebibyte_is_listed_and_not_rendered() {
    let repo = Repo::new("big-untracked");
    repo.write("a.txt", "a\n");
    repo.commit("base");
    repo.write("big.txt", vec![b'x'; 1024 * 1024 + 1]);
    repo.write("exactly.txt", vec![b'y'; 1024 * 1024]);
    let diff = repo.load(&Spec::WorkTree);
    assert_eq!(find(&diff, "big.txt").change, Change::TooLarge);
    assert!(find(&diff, "big.txt").hunks.is_empty());
    assert_eq!(find(&diff, "exactly.txt").change, Change::Untracked);
    assert!(diff.notices.is_empty());
}

#[test]
fn a_base_that_does_not_exist_falls_back_to_the_working_tree() {
    let repo = Repo::new("no-base");
    repo.write("a.txt", "a\n");
    repo.commit("base");
    repo.write("a.txt", "b\n");
    let diff = repo.load(&branch("no-such-branch"));
    assert_eq!(diff.spec, Spec::WorkTree);
    assert_eq!(
        diff.notices,
        [Notice::BaseMissing {
            base: "no-such-branch".into()
        }]
    );
    assert_eq!(find(&diff, "a.txt").change, Change::Modified);
}

#[test]
fn a_base_equal_to_head_shows_the_uncommitted_changes_as_the_working_tree_does() {
    let repo = Repo::new("base-is-head");
    repo.write("a.txt", "a\n");
    repo.commit("base");
    repo.write("a.txt", "b\n");
    repo.write("loose.txt", "c\n");
    let on_branch = repo.load(&branch("main"));
    let on_tree = repo.load(&Spec::WorkTree);
    assert_eq!(on_branch.spec, branch("main"));
    assert!(on_branch.notices.is_empty());
    assert_eq!(on_branch.files, on_tree.files);
    assert_eq!(paths(&on_branch), ["a.txt", "loose.txt"]);
}

#[test]
fn the_branch_spec_shows_committed_and_uncommitted_changes_the_working_tree_spec_shows_only_the_latter()
 {
    let repo = Repo::new("branch-spec");
    repo.write("a.txt", "a\n");
    repo.commit("base");
    repo.git(&["checkout", "-q", "-b", "feature"]);
    repo.write("committed.txt", "c\n");
    repo.commit("on the branch");
    repo.write("a.txt", "edited\n");
    let on_branch = repo.load(&branch("main"));
    assert_eq!(on_branch.spec, branch("main"));
    assert_eq!(paths(&on_branch), ["a.txt", "committed.txt"]);
    assert_eq!(find(&on_branch, "a.txt").change, Change::Modified);
    assert_eq!(find(&on_branch, "committed.txt").change, Change::Added);
    assert_eq!(paths(&repo.load(&Spec::WorkTree)), ["a.txt"]);
}

#[test]
fn a_rename_with_edits_is_one_renamed_file_whatever_the_local_config_says() {
    let repo = Repo::new("rename");
    repo.write("old.txt", "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n");
    repo.commit("base");
    repo.git(&["mv", "old.txt", "új név.txt"]);
    repo.write("új név.txt", "l1\nl2\nl3\nCHANGED\nl5\nl6\nl7\nl8\n");
    let diff = repo.load(&Spec::WorkTree);
    assert_eq!(paths(&diff), ["új név.txt"]);
    let file = find(&diff, "új név.txt");
    assert_eq!(file.change, Change::Renamed);
    assert_eq!(file.old_path.as_ref().unwrap().as_str(), "old.txt");
    assert!(!file.hunks.is_empty());
}

#[test]
fn a_file_deleted_and_recreated_is_whatever_git_reports() {
    let repo = Repo::new("recreated");
    repo.write("a.txt", "old\n");
    repo.commit("base");
    std::fs::remove_file(repo.0.join("a.txt")).unwrap();
    repo.write("a.txt", "new\n");
    let diff = repo.load(&Spec::WorkTree);
    assert_eq!(paths(&diff), ["a.txt"]);
    assert_eq!(find(&diff, "a.txt").change, Change::Modified);
}
