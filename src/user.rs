//! `archive` and `view`: the two commands that are not a `comment` and that the panel runs for the
//! user (design/graphs/commands.md). `--as user` never calls `herdr` or `hc`, and neither do these.

use crate::cli::{Failure, Output};
use crate::diff::{GitError, RepoRoot, load as load_diff};
use crate::env::Env;
use crate::json::{ArchiveJson, SCHEMA};
use crate::meta::{load as load_meta, locate, save};
use crate::state::{Query, choose};
use crate::store::{RelPath, archive as archive_store};

use std::path::{Path, PathBuf};

fn root_and_dir(
    repo: Option<&Path>,
    env: &Env,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
) -> Result<(RepoRoot, PathBuf), Failure> {
    let root = RepoRoot::resolve(repo, &env.cwd, git)?;
    let base = env.state_base().ok_or(Failure::NoStateDir)?;
    let dir = locate(&base, root.path());
    Ok((root, dir))
}

/// `archive [--json]`: move the resolved threads that are not new to `archive.jsonl`.
pub fn archive(
    repo: Option<&Path>,
    json: bool,
    env: &Env,
    now: &str,
    git: impl FnMut(&[String]) -> Result<String, GitError>,
) -> Result<Output, Failure> {
    let (_, dir) = root_and_dir(repo, env, git)?;
    let done = archive_store(&dir, now)?;
    let stdout = if json {
        let reply = ArchiveJson {
            schema: SCHEMA,
            archived: done.threads,
            unsent: done.unsent,
        };
        format!("{}\n", serde_json::to_string(&reply).unwrap_or_default())
    } else if done.threads == 0 {
        "nothing to archive\n".to_owned()
    } else {
        let noun = if done.threads == 1 {
            "thread"
        } else {
            "threads"
        };
        let never = if done.unsent > 0 {
            format!(", {} never sent", done.unsent)
        } else {
            String::new()
        };
        format!("archived {} {noun}{never}\n", done.threads)
    };
    Ok(Output {
        stdout,
        ..Output::default()
    })
}

/// `view --path P --set|--unset`: touch the one entry of `viewed` in `meta.json`. A mark that is
/// already as asked writes nothing. `--set` stores the fingerprint of the file in the diff of the
/// spec, so a path the diff does not have is refused.
pub fn view(
    query: &Query,
    path: &str,
    set: bool,
    env: &Env,
    mut git: impl FnMut(&[String]) -> Result<Vec<u8>, GitError>,
) -> Result<Output, Failure> {
    let (root, dir) = {
        let mut text =
            |args: &[String]| git(args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        root_and_dir(query.repo.as_deref(), env, &mut text)?
    };
    let meta = load_meta(&dir).0;
    let change = if set {
        let (spec, _, _) = choose(query, &meta, root.path(), &mut git)?;
        let diff = load_diff(root.path(), &spec, &mut git)?;
        let file = RelPath::parse(path)
            .and_then(|wanted| diff.files.into_iter().find(|file| file.path == wanted))
            .ok_or_else(|| Failure::NotInDiff(path.to_owned()))?;
        let fingerprint = file.fingerprint();
        (meta.viewed.get(path) != Some(&fingerprint)).then_some(Some(fingerprint))
    } else {
        meta.viewed.contains_key(path).then_some(None)
    };
    if let Some(fingerprint) = change {
        save(&dir, root.path(), |meta| match fingerprint {
            Some(fingerprint) => {
                meta.viewed.insert(path.to_owned(), fingerprint);
            }
            None => {
                meta.viewed.remove(path);
            }
        })?;
    }
    Ok(Output::default())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;
    use crate::json::StateJson;
    use crate::store::{Add, Author, CommentId, Event, Kind, Side, Spec, state_dir, write};

    const PATCH: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,2 @@
 keep();
-old();
+new();
";

    struct World {
        env: Env,
        root: PathBuf,
        dir: PathBuf,
        top: Result<(), GitError>,
    }

    impl World {
        fn new(name: &str) -> Self {
            let home = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("herdr-review-user-{name}-{}", std::process::id()));
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
                top: Ok(()),
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
                [_, _, "rev-parse", "--verify", "--quiet", "HEAD" | "main"] => {
                    Ok(b"abc\n".to_vec())
                }
                [_, _, "merge-base", ..] => Ok(b"mb123\n".to_vec()),
                [_, _, "-c", _, "diff", ..] => Ok(PATCH.as_bytes().to_vec()),
                [_, _, "ls-files", ..] => Ok(Vec::new()),
                _ => Err(failed()),
            }
        }

        fn text_git(&self, args: &[String]) -> Result<String, GitError> {
            self.git(args)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        }

        fn view(&self, line: &str, set: bool) -> Result<Output, Failure> {
            let mut args = ["view", "--set"].map(str::to_owned).to_vec();
            args.extend(line.split_whitespace().map(str::to_owned));
            let Ok(crate::cli::Command::View { query, path, .. }) = crate::cli::parse(&args) else {
                panic!("{line}");
            };
            view(&query, &path, set, &self.env, |args| self.git(args))
        }

        fn state(&self) -> StateJson {
            let query = Query::default();
            let output = crate::state::state(&query, &self.env, |args| self.git(args)).unwrap();
            serde_json::from_str(&output.stdout).unwrap()
        }

        fn archive(&self, json: bool) -> Result<Output, Failure> {
            archive(None, json, &self.env, "t", |args| self.text_git(args))
        }

        fn meta(&self) -> Option<String> {
            std::fs::read_to_string(self.dir.join("meta.json")).ok()
        }

        /// A user thread `id`, resolved by `by`.
        fn resolved(&self, id: &str, by: &Author) {
            let add = Event {
                at: "t".into(),
                by: Author::User,
                kind: Kind::Add(Add {
                    id: CommentId::parse(id).unwrap(),
                    parent: None,
                    path: RelPath::parse("src/a.rs"),
                    old_path: None,
                    side: Some(Side::New),
                    line: Some(1),
                    end_line: None,
                    line_text: Some("keep();".into()),
                    spec: Some(Spec::WorkTree),
                    body: "b".into(),
                }),
            };
            let resolve = Event {
                at: "t".into(),
                by: by.clone(),
                kind: Kind::Resolve {
                    id: CommentId::parse(id).unwrap(),
                },
            };
            write(&self.dir, "t", |_, _| Ok::<_, ()>((vec![add, resolve], ()))).unwrap();
        }
    }

    #[test]
    fn archive_with_nothing_to_move_writes_nothing_and_prints_zeros() {
        let world = World::new("archive-none");
        let json = world.archive(true).unwrap();
        assert_eq!(json.stdout, "{\"schema\":1,\"archived\":0,\"unsent\":0}\n");
        assert_eq!(world.archive(false).unwrap().stdout, "nothing to archive\n");
        assert!(!world.dir.join("review.jsonl").exists());
        assert!(!world.dir.join("archive.jsonl").exists());
    }

    #[test]
    fn archive_moves_the_resolved_threads_and_counts_the_unsent_ones() {
        let world = World::new("archive");
        world.resolved("u1", &Author::User);
        world.resolved("u2", &Author::User);
        let agent = Author::Agent(Some("claude".into()));
        world.resolved("u3", &agent);
        assert_eq!(world.state().files[0].new, 1);
        // The count is what the archive then moves.
        assert_eq!(world.state().archivable, 2);
        let json = world.archive(true).unwrap();
        // The thread an agent resolved is new and stays.
        assert_eq!(json.stdout, "{\"schema\":1,\"archived\":2,\"unsent\":2}\n");
        let state = world.state();
        assert_eq!((state.files[0].open, state.files[0].new), (0, 1));
        assert_eq!(state.archivable, 0);
        assert!(world.dir.join("archive.jsonl").exists());
        assert_eq!(world.archive(false).unwrap().stdout, "nothing to archive\n");
        world.resolved("u4", &Author::User);
        assert_eq!(world.state().archivable, 1);
        assert_eq!(
            world.archive(false).unwrap().stdout,
            "archived 1 thread, 1 never sent\n"
        );
    }

    #[test]
    fn archive_fails_with_exit_1_outside_a_repo_and_on_a_log_it_cannot_read() {
        let mut world = World::new("archive-fail");
        std::fs::create_dir_all(world.dir.join("review.jsonl")).unwrap();
        let output = world.archive(true).unwrap_err().output();
        assert_eq!(output.code, 1);
        assert!(output.stderr.contains("review.jsonl"));
        world.top = Err(GitError::NotARepo);
        let output = archive(None, true, &world.env, "t", |args| world.text_git(args))
            .unwrap_err()
            .output();
        assert_eq!(
            (output.code, output.stderr.as_str()),
            (1, "not a git repository\n")
        );
    }

    #[test]
    fn view_set_marks_the_file_in_the_diff_and_unset_takes_the_mark_off() {
        let world = World::new("view");
        assert!(!world.state().files[0].viewed);
        let output = world.view("--path src/a.rs", true).unwrap();
        assert_eq!((output.code, output.stdout.as_str()), (0, ""));
        assert!(world.state().files[0].viewed);
        let marked = world.meta().unwrap();
        // The same mark again rewrites nothing.
        world.view("--path src/a.rs", true).unwrap();
        assert_eq!(world.meta().unwrap(), marked);
        world
            .view("--path src/a.rs --spec branch --base main", true)
            .unwrap();
        world.view("--path src/a.rs", false).unwrap();
        assert!(!world.state().files[0].viewed);
        assert!(!world.meta().unwrap().contains("src/a.rs"));
    }

    #[test]
    fn view_touches_one_entry_and_leaves_the_other_marks_and_fields() {
        let world = World::new("view-one");
        std::fs::create_dir_all(&world.dir).unwrap();
        std::fs::write(
            world.dir.join("meta.json"),
            r#"{"root":"/x","base":"dev","viewed":{"gone.rs":"ff"}}"#,
        )
        .unwrap();
        world.view("--path src/a.rs", true).unwrap();
        let meta = crate::meta::load(&world.dir).0;
        assert_eq!(meta.viewed.len(), 2);
        assert_eq!(meta.viewed["gone.rs"], "ff");
        assert_eq!(meta.base.as_deref(), Some("dev"));
        world.view("--path src/a.rs", false).unwrap();
        assert_eq!(crate::meta::load(&world.dir).0.viewed.len(), 1);
    }

    #[test]
    fn view_set_of_a_file_not_in_the_diff_exits_2_and_unset_of_an_unmarked_path_writes_nothing() {
        let world = World::new("view-missing");
        for path in ["nope.rs", "/abs.rs", "../up.rs"] {
            let output = world
                .view(&format!("--path {path}"), true)
                .unwrap_err()
                .output();
            assert_eq!(output.code, 2, "{path}");
            assert_eq!(output.stderr, format!("not in diff: {path}\n"));
        }
        assert_eq!(world.view("--path src/a.rs", false).unwrap().code, 0);
        assert!(world.meta().is_none() && !world.dir.exists());
    }

    #[test]
    fn view_fails_with_exit_1_outside_a_repo() {
        let mut world = World::new("view-no-repo");
        world.top = Err(GitError::NotARepo);
        let output = world.view("--path src/a.rs", true).unwrap_err().output();
        assert_eq!(
            (output.code, output.stderr.as_str()),
            (1, "not a git repository\n")
        );
    }
}
