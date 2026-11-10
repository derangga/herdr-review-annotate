//! The process environment, read once in `main.rs` and passed down as a value.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::store::state_base;

/// The `HERDR_*` and `REVIEW_*` variables, `HC_TERMINAL_ID`, the two that locate the state directory,
/// the one that locates Claude Code's config, the working directory and the running binary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// The running binary, `std::env::current_exe()`. Empty when the system cannot say.
    pub exe: PathBuf,
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
                        "HOME" | "XDG_STATE_HOME" | "CLAUDE_CONFIG_DIR" | "HC_TERMINAL_ID"
                    )
            })
            .collect();
        Self {
            vars,
            cwd,
            exe: PathBuf::new(),
        }
    }

    /// The same environment with the running binary's path.
    #[must_use]
    pub fn with_exe(mut self, exe: PathBuf) -> Self {
        self.exe = exe;
        self
    }

    /// A variable's value. An empty value is the same as a missing one.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    /// The hc terminal that owns this shell, when hc is the nearest host. A Herdr pane started inside
    /// an hc terminal inherits `HC_TERMINAL_ID`, and hc terminals carry no `HERDR_*` variable, so
    /// `HERDR_PANE_ID` means Herdr is nearer and wins.
    pub fn hc_terminal(&self) -> Option<&str> {
        self.get("HC_TERMINAL_ID")
            .filter(|_| self.get("HERDR_PANE_ID").is_none())
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
            ("HC_TERMINAL_ID", "t7"),
        ]);
        assert_eq!(env.get("PATH"), None);
        assert_eq!(env.get("HERDR_PANE_ID"), Some("w1:p1"));
        assert_eq!(env.get("REVIEW_X"), None);
        assert_eq!(env.get("CLAUDE_CONFIG_DIR"), Some("/c"));
        assert_eq!(env.get("HC_TERMINAL_ID"), Some("t7"));
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
