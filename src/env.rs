//! The process environment, read once in `main.rs` and passed down as a value.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::store::state_base;

/// The `HERDR_*` and `REVIEW_*` variables, the two that locate the state directory, the one that
/// locates Claude Code's config, and the working directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

impl Env {
    pub fn new(vars: impl IntoIterator<Item = (String, String)>, cwd: PathBuf) -> Self {
        let vars = vars
            .into_iter()
            .filter(|(name, _)| {
                name.starts_with("HERDR_")
                    || name.starts_with("REVIEW_")
                    || matches!(
                        name.as_str(),
                        "HOME" | "XDG_STATE_HOME" | "CLAUDE_CONFIG_DIR"
                    )
            })
            .collect();
        Self { vars, cwd }
    }

    /// A variable's value. An empty value is the same as a missing one.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// The `HERDR_*` and `REVIEW_*` variables in name order.
    pub fn herdr_and_review(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars
            .iter()
            .filter(|(name, _)| name.starts_with("HERDR_") || name.starts_with("REVIEW_"))
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Where all reviews are kept: `${XDG_STATE_HOME:-~/.local/state}/herdr-review`.
    pub fn state_base(&self) -> Option<PathBuf> {
        let path = |name| self.get(name).map(PathBuf::from);
        state_base(path("XDG_STATE_HOME").as_deref(), path("HOME").as_deref())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> Env {
        Env::new(
            vars.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            "/cwd".into(),
        )
    }

    #[test]
    fn only_the_variables_the_program_reads_are_kept() {
        let env = env(&[
            ("PATH", "/bin"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("REVIEW_X", ""),
            ("CLAUDE_CONFIG_DIR", "/c"),
        ]);
        assert_eq!(env.get("PATH"), None);
        assert_eq!(env.get("HERDR_PANE_ID"), Some("w1:p1"));
        assert_eq!(env.get("REVIEW_X"), None);
        assert_eq!(env.get("CLAUDE_CONFIG_DIR"), Some("/c"));
        assert_eq!(env.herdr_and_review().count(), 2);
    }

    #[test]
    fn the_state_base_follows_xdg_then_home() {
        let xdg = env(&[("XDG_STATE_HOME", "/x"), ("HOME", "/h")]);
        assert_eq!(xdg.state_base().unwrap(), PathBuf::from("/x/herdr-review"));
        let home = env(&[("HOME", "/h")]);
        assert_eq!(
            home.state_base().unwrap(),
            PathBuf::from("/h/.local/state/herdr-review")
        );
        assert_eq!(env(&[]).state_base(), None);
    }
}
