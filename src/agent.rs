//! Who is running a `comment` command: `--name`, else the Herdr agent that owns this shell.

use serde_json::Value;

use crate::env::Env;
use crate::store::Author;

const MAX_NAME: usize = 64;

/// A name fit for the log and for a prompt: no control characters, at most 64 of them.
fn clean(name: &str) -> Option<String> {
    let name = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The agent name in one `herdr agent list` result: the entry whose `pane_id` is `pane`, else the
/// only entry whose `cwd` is `cwd`.
fn from_list(list: &str, pane: Option<&str>, cwd: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(list).ok()?;
    let agents = value.pointer("/result/agents")?.as_array()?;
    let field = |agent: &Value, name| agent.get(name).and_then(Value::as_str).map(str::to_owned);
    let by_pane = pane.and_then(|pane| {
        agents
            .iter()
            .find(|agent| field(agent, "pane_id").as_deref() == Some(pane))
    });
    let mut here = agents
        .iter()
        .filter(|agent| field(agent, "cwd").as_deref() == Some(cwd));
    let found = by_pane.or_else(|| here.next().filter(|_| here.next().is_none()))?;
    clean(&field(found, "agent")?)
}

/// `--name` wins. Otherwise ask Herdr. Outside Herdr, with no single match, or when Herdr fails,
/// the author is a plain agent, because a failed Herdr call never fails the command.
pub fn author(
    name: Option<&str>,
    env: &Env,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Author {
    if let Some(name) = name.and_then(clean) {
        return Author::Agent(Some(name));
    }
    let list = herdr(&["agent".to_owned(), "list".to_owned()]);
    let name = list
        .ok()
        .and_then(|list| from_list(&list, env.get("HERDR_PANE_ID"), &env.cwd.to_string_lossy()));
    Author::Agent(name)
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"id":"x","result":{"type":"agent_list","agents":[
        {"pane_id":"w1:p1","agent":"claude","cwd":"/repo"},
        {"pane_id":"w1:p2","agent":"codex","cwd":"/repo"},
        {"pane_id":"w1:p3","agent":"pi","cwd":"/other"}]}}"#;

    fn env(vars: &[(&str, &str)], cwd: &str) -> Env {
        Env::new(
            vars.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())),
            cwd.into(),
        )
    }

    fn agent(name: &str) -> Author {
        Author::Agent(Some(name.to_owned()))
    }

    #[test]
    fn the_name_flag_wins_and_asks_nothing() {
        let author = author(Some("reviewer"), &env(&[], "/repo"), |_| {
            panic!("asked herdr")
        });
        assert_eq!(author, agent("reviewer"));
    }

    #[test]
    fn the_pane_that_owns_the_shell_names_the_agent() {
        let env = env(&[("HERDR_PANE_ID", "w1:p2")], "/repo");
        assert_eq!(author(None, &env, |_| Ok(LIST.into())), agent("codex"));
    }

    #[test]
    fn a_single_agent_in_the_current_directory_names_the_agent() {
        let env = env(&[], "/other");
        assert_eq!(author(None, &env, |_| Ok(LIST.into())), agent("pi"));
    }

    #[test]
    fn several_or_no_matches_give_a_plain_agent() {
        assert_eq!(
            author(None, &env(&[], "/repo"), |_| Ok(LIST.into())),
            Author::Agent(None)
        );
        assert_eq!(
            author(None, &env(&[], "/none"), |_| Ok(LIST.into())),
            Author::Agent(None)
        );
        let unknown_pane = env(&[("HERDR_PANE_ID", "w9:p9")], "/none");
        assert_eq!(
            author(None, &unknown_pane, |_| Ok(LIST.into())),
            Author::Agent(None)
        );
    }

    #[test]
    fn a_failing_or_odd_herdr_gives_a_plain_agent() {
        let env = env(&[("HERDR_PANE_ID", "w1:p1")], "/repo");
        for reply in [
            Err("gone".to_owned()),
            Ok("not json".to_owned()),
            Ok("{}".to_owned()),
            Ok(r#"{"result":{"agents":[{"pane_id":"w1:p1","agent":""}]}}"#.to_owned()),
        ] {
            assert_eq!(author(None, &env, |_| reply.clone()), Author::Agent(None));
        }
    }

    #[test]
    fn names_lose_control_characters_and_length() {
        assert_eq!(clean("  cl\u{1b}[31maude\n"), Some("cl[31maude".into()));
        assert_eq!(clean("\n"), None);
        assert_eq!(clean(&"x".repeat(100)).map(|n| n.len()), Some(64));
    }
}
