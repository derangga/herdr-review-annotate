use std::cell::RefCell;
use std::path::PathBuf;

use super::*;
use crate::message::{pointer_path, write_pointer};
use crate::store::{PaneId, TerminalId};

const SESSION: &str = "0b9c6f1e-1111-4222-8333-444455556666";

/// A temporary `HOME` holding a transcript, and a Herdr that records every call and answers from
/// a script.
struct World {
    home: PathBuf,
    calls: RefCell<Vec<String>>,
    /// Panes `agent get` accepts, with the agent running there.
    agents: Vec<(&'static str, &'static str)>,
    /// Panes `pane get` accepts.
    panes: Vec<&'static str>,
    focus_fails: bool,
    open_fails: bool,
}

impl World {
    fn new(name: &str, transcript: &[&str]) -> Self {
        let home = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "herdr-review-message-action-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        let projects = home.join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(
            projects.join(format!("{SESSION}.jsonl")),
            transcript.join("\n"),
        )
        .unwrap();
        Self {
            home,
            calls: RefCell::default(),
            agents: vec![("w1:p2", "claude")],
            panes: Vec::new(),
            focus_fails: false,
            open_fails: false,
        }
    }

    fn env(&self, pane: Option<&str>) -> Env {
        let mut vars = vec![("HOME".to_owned(), self.home.display().to_string())];
        if let Some(pane) = pane {
            let json = format!(r#"{{"focused_pane_id":"{pane}","focused_pane_cwd":"/work"}}"#);
            vars.push(("HERDR_PLUGIN_CONTEXT_JSON".to_owned(), json));
        }
        Env::new(vars, self.home.clone())
    }

    fn herdr(&self, args: &[String]) -> Result<String, String> {
        let call = args.join(" ");
        self.calls.borrow_mut().push(call.clone());
        if let Some(pane) = call.strip_prefix("agent get ") {
            let found = self.agents.iter().find(|(name, _)| *name == pane);
            return found.map_or(
                Err(r#"{"error":{"code":"agent_not_found","message":"no agent"}}"#.into()),
                |(_, agent)| {
                    Ok(format!(
                        r#"{{"result":{{"agent":{{"agent":"{agent}","agent_status":"idle","cwd":"/work","pane_id":"{pane}","terminal_id":"term_1","agent_session":{{"kind":"id","value":"{SESSION}"}}}}}}}}"#
                    ))
                },
            );
        }
        if let Some(pane) = call.strip_prefix("pane get ") {
            return if self.panes.contains(&pane) {
                Ok("{}".into())
            } else {
                Err("pane_not_found".into())
            };
        }
        if call.starts_with("plugin pane focus ") && self.focus_fails {
            return Err("focus failed".into());
        }
        if call.starts_with("plugin pane open ") && self.open_fails {
            return Err("herdr said no".into());
        }
        Ok("{}".into())
    }

    fn run(&self, env: &Env) -> Result<Opened, String> {
        open(env, |args| self.herdr(args))
    }

    fn point_at(&self, pane: &str) {
        let path = pointer_path(&self.env(None), &TerminalId::parse("term_1").unwrap()).unwrap();
        write_pointer(&path, &PaneId::parse(pane).unwrap()).unwrap();
    }

    fn opened(&self) -> Vec<String> {
        let calls = self.calls.borrow();
        let opened = calls.iter().filter(|c| c.starts_with("plugin pane open "));
        opened.cloned().collect()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

const REPLY: &str = r#"{"type":"assistant","isSidechain":false,"message":{"id":"m1","model":"claude-x","content":[{"type":"text","text":"Done."}]}}"#;

#[test]
fn the_pane_opens_to_the_right_of_the_agent_with_its_ids() {
    let world = World::new("open", &[REPLY]);
    assert_eq!(world.run(&world.env(Some("w1:p2"))), Ok(Opened::Opened));
    assert_eq!(
        world.opened(),
        [
            "plugin pane open --plugin review --entrypoint message --placement split \
             --direction right --target-pane w1:p2 --env REVIEW_DELIVER_TO=w1:p2 \
             --env REVIEW_DELIVER_TERM=term_1 --cwd /work --focus"
        ]
    );
}

#[test]
fn without_a_focused_pane_the_user_is_told_to_focus_the_agent() {
    let world = World::new("no-context", &[REPLY]);
    let result = world.run(&world.env(None));
    assert_eq!(result, Err("Focus the agent's pane first.".to_owned()));
    assert!(world.calls.borrow().is_empty());
}

#[test]
fn a_pane_that_is_not_an_agent_notifies_and_opens_nothing() {
    let world = World::new("not-an-agent", &[REPLY]);
    let result = world.run(&world.env(Some("w1:p9")));
    assert_eq!(result, Err("No agent is running in that pane.".to_owned()));
    assert!(world.opened().is_empty());
}

#[test]
fn an_agent_that_is_not_claude_notifies_and_opens_nothing() {
    let mut world = World::new("codex", &[REPLY]);
    world.agents = vec![("w1:p2", "codex")];
    let result = world.run(&world.env(Some("w1:p2")));
    assert_eq!(
        result,
        Err("Message review reads Claude Code only, not codex.".to_owned())
    );
    assert!(world.opened().is_empty());
}

#[test]
fn a_second_press_focuses_the_message_pane_that_is_open() {
    let mut world = World::new("second", &[REPLY]);
    world.panes = vec!["w1:p7"];
    world.point_at("w1:p7");
    assert_eq!(world.run(&world.env(Some("w1:p2"))), Ok(Opened::Focused));
    assert!(
        world
            .calls
            .borrow()
            .contains(&"plugin pane focus w1:p7".to_owned())
    );
    assert!(world.opened().is_empty());
}

#[test]
fn a_stale_pane_id_or_a_failed_focus_opens_a_new_pane() {
    let mut world = World::new("stale", &[REPLY]);
    world.point_at("w1:p7");
    assert_eq!(world.run(&world.env(Some("w1:p2"))), Ok(Opened::Opened));
    assert!(
        !world
            .calls
            .borrow()
            .iter()
            .any(|c| c.starts_with("plugin pane focus"))
    );
    world.panes = vec!["w1:p7"];
    world.focus_fails = true;
    assert_eq!(world.run(&world.env(Some("w1:p2"))), Ok(Opened::Opened));
    assert_eq!(world.opened().len(), 2);
}

#[test]
fn a_session_with_no_message_notifies_and_opens_nothing() {
    let world = World::new(
        "no-message",
        &[r#"{"type":"user","message":{"content":"hi"}}"#],
    );
    let result = world.run(&world.env(Some("w1:p2")));
    assert_eq!(result, Err("The session has no message yet.".to_owned()));
    assert!(world.opened().is_empty());
}

#[test]
fn a_herdr_error_on_open_is_reported() {
    let mut world = World::new("open-fails", &[REPLY]);
    world.open_fails = true;
    let result = world.run(&world.env(Some("w1:p2")));
    assert_eq!(result, Err("herdr said no".to_owned()));
}
