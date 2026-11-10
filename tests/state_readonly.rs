//! `state` and `file` against a real `git` and the real binary: they read a review and write
//! nothing. The panel watches the repo root and the state directory, so one write would make it
//! read again without end.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use herdr_review::json::{FileJson, StateJson};
use herdr_review::store::state_dir;

struct World {
    home: PathBuf,
    root: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let home = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "herdr-review-readonly-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
            home,
        }
    }

    fn state_base(&self) -> PathBuf {
        self.home.join("state")
    }

    fn dir(&self) -> PathBuf {
        state_dir(&self.state_base().join("herdr-review"), &self.root)
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            // The machine's own git config stays out: a global core.fsmonitor starts a daemon on macOS that
            // makes and removes files under .git while the snapshots walk it.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.root.join(path), text).unwrap();
    }

    /// A commit, then an unstaged change, a file touched so that its stat is stale and `git diff`
    /// would refresh the index, and an untracked file.
    fn repo(&self) {
        self.git(&["init", "-q", "-b", "main"]);
        self.git(&["config", "commit.gpgsign", "false"]);
        // The commit would otherwise start `git maintenance run --auto` in the background, which holds
        // .git/objects/maintenance.lock while the snapshots walk the repository.
        self.git(&["config", "maintenance.auto", "false"]);
        self.git(&["config", "gc.auto", "0"]);
        self.write("a.txt", "one\ntwo\nthree\n");
        self.write("b.txt", "same\n");
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", "base"]);
        self.write("a.txt", "one\n2\nthree\n");
        self.write("b.txt", "same\n");
        let hour = std::time::Duration::from_secs(3600);
        let stale = std::time::SystemTime::now() - hour;
        let file = std::fs::File::options()
            .write(true)
            .open(self.root.join("b.txt"))
            .unwrap();
        file.set_modified(stale).unwrap();
        self.write("new.txt", "fresh\n");
    }

    /// A review with one open thread on a line of `a.txt`, and a `meta.json` that marks it viewed.
    fn review(&self) {
        let dir = self.dir();
        std::fs::create_dir_all(&dir).unwrap();
        let add = r#"{"at":"2026-11-09T10:00:00Z","by":"user","kind":"add","id":"u1","path":"a.txt","side":"new","line":2,"line_text":"2","spec":"worktree","body":"why"}"#;
        std::fs::write(dir.join("review.jsonl"), format!("{add}\n")).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            r#"{"spec":"worktree","viewed":{"b.txt":"0000000000000000"}}"#,
        )
        .unwrap();
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_herdr-review"))
            .args(args)
            .arg("--repo")
            .arg(&self.root)
            .arg("--json")
            .env("XDG_STATE_HOME", self.state_base())
            .env_remove("HERDR_PANE_ID")
            .current_dir(&self.home)
            .output()
            .unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Every file under `dir` with its bytes, the `.git` folder and the state directory included.
fn snapshot(dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            snapshot(&path, into);
        } else {
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .unwrap();
            into.insert(path, bytes);
        }
    }
}

/// The paths that were added, removed or changed between two snapshots.
fn changed(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> Vec<PathBuf> {
    let paths = before.keys().chain(after.keys());
    let paths = paths.collect::<std::collections::BTreeSet<_>>();
    let same = |path: &&PathBuf| before.get(*path) == after.get(*path);
    paths
        .into_iter()
        .filter(|path| !same(path))
        .cloned()
        .collect()
}

fn everything(world: &World) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    snapshot(&world.home, &mut files);
    files
}

#[test]
fn state_and_file_leave_the_review_the_work_tree_and_the_index_as_they_were() {
    let world = World::new("kept");
    world.repo();
    world.review();
    let before = everything(&world);
    let index = world.root.join(".git/index");
    assert!(before.contains_key(&index));
    for args in [
        &["state"][..],
        &["file", "--path", "a.txt"],
        &["file", "--path", "new.txt"],
        &["state", "--spec", "branch", "--base", "main"],
    ] {
        let output = world.run(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
        assert_eq!(
            changed(&before, &everything(&world)),
            Vec::<PathBuf>::new(),
            "{args:?} wrote something"
        );
    }
    // What they printed is the review they read.
    let state: StateJson = serde_json::from_slice(&world.run(&["state"]).stdout).unwrap();
    let a = state.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!((a.add, a.del, a.open, a.unsent), (1, 1, 1, 1));
    let paths = state
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["a.txt", "new.txt"]);
    let file: FileJson =
        serde_json::from_slice(&world.run(&["file", "--path", "a.txt"]).stdout).unwrap();
    assert_eq!(file.threads[0].placement.row, Some(3));
}

#[test]
fn state_and_file_create_no_state_directory() {
    let world = World::new("no-store");
    world.repo();
    let before = everything(&world);
    for args in [&["state"][..], &["file", "--path", "a.txt"]] {
        let output = world.run(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
    }
    assert_eq!(changed(&before, &everything(&world)), Vec::<PathBuf>::new());
    assert!(!world.state_base().exists());
}
