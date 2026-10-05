//! `git`: the runner, and the repository root.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
}
