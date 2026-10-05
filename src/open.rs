//! The `open` action: put the review pane to the right of the agent pane.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

use crate::diff::{GitError, RepoRoot, run_git};
use crate::env::Env;
use crate::herdr::{notify, run_herdr_output};
use crate::meta::{Meta, load, locate, save};
use crate::send::resolve_target;
use crate::store::Spec;

/// What the action context says about the pane the user was in.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub pane: Option<String>,
    pub cwd: Option<PathBuf>,
}

pub fn parse_context(json: Option<&str>) -> Context {
    let value = json.and_then(|json| serde_json::from_str::<Value>(json).ok());
    let field = |name: &str| {
        value
            .as_ref()
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    Context {
        pane: field("focused_pane_id"),
        cwd: field("focused_pane_cwd").map(PathBuf::from),
    }
}

/// The `terminal_id` of the agent in `pane`, from one `herdr agent get` result.
fn terminal_id(agent_get: &Result<String, String>) -> Option<String> {
    let value = serde_json::from_str::<Value>(agent_get.as_ref().ok()?).ok()?;
    value
        .pointer("/result/agent/terminal_id")?
        .as_str()
        .map(str::to_owned)
}

/// The arguments of `herdr plugin pane open` for the review pane (PLAN.md section 5, `open` step 4).
fn pane_open_args(root: &str, pane: Option<&str>, terminal: Option<&str>) -> Vec<String> {
    let mut args = [
        "plugin",
        "pane",
        "open",
        "--plugin",
        "review",
        "--entrypoint",
        "tui",
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend(["--placement", "split", "--direction", "right"].map(str::to_owned));
    if let Some(pane) = pane {
        args.extend(["--target-pane".to_owned(), pane.to_owned()]);
        args.push("--env".to_owned());
        args.push(format!("REVIEW_DELIVER_TO={pane}"));
    }
    if let Some(terminal) = terminal {
        args.push("--env".to_owned());
        args.push(format!("REVIEW_DELIVER_TERM={terminal}"));
    }
    args.extend(["--cwd".to_owned(), root.to_owned(), "--focus".to_owned()]);
    args
}

/// An agent that can take a review pane beside it.
struct Beside {
    pane: String,
    terminal: String,
}

/// The agent in `pane`, when `herdr agent get` accepts it and names its terminal.
fn agent_in(
    pane: &str,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> Option<Beside> {
    let got = herdr(&["agent".to_owned(), "get".to_owned(), pane.to_owned()]);
    let terminal = terminal_id(&got)?;
    Some(Beside {
        pane: pane.to_owned(),
        terminal,
    })
}

/// Focus the review pane `meta.json` names, when it still exists. Any failure means "open one".
fn focus_existing(
    meta: &Meta,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> bool {
    let Some(pane) = &meta.review_pane else {
        return false;
    };
    let pane = pane.as_str().to_owned();
    herdr(&["pane".to_owned(), "get".to_owned(), pane.clone()]).is_ok()
        && herdr(&[
            "plugin".to_owned(),
            "pane".to_owned(),
            "focus".to_owned(),
            pane,
        ])
        .is_ok()
}

/// What `open` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The review pane was already open and now has focus.
    Focused,
    Opened,
}

/// Open the review pane for the repository of the action's pane or the current directory, or focus
/// the one that is open (PLAN.md section 5, `open`, and the graph in 12.4).
///
/// The agent is the focused pane when it hosts an agent, else `$HERDR_PANE_ID` when Herdr accepts
/// it, else whatever target resolution finds. With none, or several, the pane opens without one
/// and the picker runs at the first send.
pub fn open(
    env: &Env,
    repo: Option<&Path>,
    base: Option<&str>,
    mut git: impl FnMut(&[String]) -> Result<String, GitError>,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<Opened, String> {
    let context = parse_context(env.get("HERDR_PLUGIN_CONTEXT_JSON"));
    let cwd = context.cwd.clone().unwrap_or_else(|| env.cwd.clone());
    let root = RepoRoot::resolve(repo, &cwd, &mut git).map_err(|error| error.to_string())?;
    let state = env
        .state_base()
        .ok_or("cannot find a state directory, set HOME or XDG_STATE_HOME")?;
    let dir = locate(&state, root.path());
    let (meta, _) = load(&dir);
    if let Some(base) = base.filter(|base| !base.is_empty() && !base.starts_with('-')) {
        // A failed save only loses the choice, so it does not stop the pane from opening.
        let _ = save(&dir, root.path(), |meta| {
            meta.spec = Some(Spec::Branch {
                base: base.to_owned(),
            });
            meta.base = Some(base.to_owned());
        });
    }
    if focus_existing(&meta, &mut herdr) {
        return Ok(Opened::Focused);
    }
    let focused = context.pane.as_deref();
    let own = env.get("HERDR_PANE_ID");
    let beside = [focused, own]
        .into_iter()
        .flatten()
        .find_map(|pane| agent_in(pane, &mut herdr))
        .or_else(|| {
            resolve_target(env, &meta, &mut herdr, root.path())
                .ok()
                .map(|target| Beside {
                    pane: target.pane.to_string(),
                    terminal: target.terminal.to_string(),
                })
        });
    herdr(&pane_open_args(
        &root.path().to_string_lossy(),
        beside.as_ref().map(|agent| agent.pane.as_str()),
        beside.as_ref().map(|agent| agent.terminal.as_str()),
    ))
    .map(|_| Opened::Opened)
}

/// The `open` action's edge: a failure is a notification and exit 1, because an action has no
/// terminal.
fn report(result: &Result<Opened, String>, mut notify: impl FnMut(&str, Option<&str>)) -> ExitCode {
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(message) => {
            notify("review: cannot open", Some(message));
            ExitCode::FAILURE
        }
    }
}

pub fn run(env: &Env, repo: Option<&Path>, base: Option<&str>) -> ExitCode {
    let result = open(env, repo, base, run_git, run_herdr_output);
    report(&result, notify)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn context_fields_are_optional() {
        assert_eq!(parse_context(None), Context::default());
        assert_eq!(parse_context(Some("not json")), Context::default());
        assert_eq!(
            parse_context(Some(
                r#"{"focused_pane_id":"w1:p2","focused_pane_cwd":"/repo"}"#
            )),
            Context {
                pane: Some("w1:p2".into()),
                cwd: Some("/repo".into())
            }
        );
    }

    #[test]
    fn terminal_id_comes_from_agent_get() {
        let record = r#"{"result":{"agent":{"terminal_id":"term_1"}}}"#.to_owned();
        assert_eq!(terminal_id(&Ok(record)), Some("term_1".to_owned()));
        assert_eq!(terminal_id(&Err("agent_not_found".into())), None);
    }

    #[test]
    fn the_pane_opens_to_the_right_of_the_agent_with_its_ids() {
        let args = pane_open_args("/repo", Some("w1:p2"), Some("term_1")).join(" ");
        assert_eq!(
            args,
            "plugin pane open --plugin review --entrypoint tui --placement split --direction right \
             --target-pane w1:p2 --env REVIEW_DELIVER_TO=w1:p2 --env REVIEW_DELIVER_TERM=term_1 \
             --cwd /repo --focus"
        );
    }

    use std::cell::RefCell;

    use crate::meta::Target;
    use crate::store::{PaneId, TerminalId};

    /// A temporary home with a repository root, the `git` that finds it, and a Herdr that records
    /// every call and answers from a script.
    struct World {
        home: PathBuf,
        root: PathBuf,
        calls: RefCell<Vec<String>>,
        /// Panes `agent get` accepts, with their terminal ids.
        agents: Vec<(&'static str, &'static str)>,
        /// Panes `pane get` accepts.
        panes: Vec<&'static str>,
        list: String,
        focus_fails: bool,
        open_fails: bool,
        not_a_repo: bool,
    }

    impl World {
        fn new(name: &str) -> Self {
            let home = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("herdr-review-open-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            let root = home.join("repo");
            std::fs::create_dir_all(&root).unwrap();
            Self {
                root: root.canonicalize().unwrap(),
                home,
                calls: RefCell::default(),
                agents: Vec::new(),
                panes: Vec::new(),
                list: r#"{"result":{"agents":[]}}"#.into(),
                focus_fails: false,
                open_fails: false,
                not_a_repo: false,
            }
        }

        fn env(&self, vars: &[(&str, &str)]) -> Env {
            let home = self.home.display().to_string();
            let vars = vars
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .chain([("HOME".to_owned(), home)]);
            Env::new(vars, self.root.clone())
        }

        fn dir(&self) -> PathBuf {
            let base = self.env(&[]).state_base().unwrap();
            locate(&base, &self.root)
        }

        fn git(&self, args: &[String]) -> Result<String, GitError> {
            self.calls
                .borrow_mut()
                .push(format!("git {}", args.join(" ")));
            if self.not_a_repo {
                return Err(GitError::NotARepo);
            }
            Ok(format!("{}\n", self.root.display()))
        }

        fn herdr(&self, args: &[String]) -> Result<String, String> {
            let call = args.join(" ");
            self.calls.borrow_mut().push(call.clone());
            if call == "agent list" {
                return Ok(self.list.clone());
            }
            if let Some(pane) = call.strip_prefix("agent get ") {
                let found = self.agents.iter().find(|(name, _)| *name == pane);
                return found.map_or(Err("agent_not_found".into()), |(_, term)| {
                    Ok(format!(
                        r#"{{"result":{{"agent":{{"agent":"claude","agent_status":"idle","terminal_id":"{term}","pane_id":"{pane}","cwd":"{}"}}}}}}"#,
                        self.root.display()
                    ))
                });
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

        fn run(&self, env: &Env, base: Option<&str>) -> Result<Opened, String> {
            open(env, None, base, |a| self.git(a), |a| self.herdr(a))
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn opened(&self) -> Vec<String> {
            let calls = self.calls();
            calls
                .into_iter()
                .filter(|c| c.starts_with("plugin pane open "))
                .collect()
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn context(pane: &str, cwd: &str) -> String {
        format!(r#"{{"focused_pane_id":"{pane}","focused_pane_cwd":"{cwd}"}}"#)
    }

    #[test]
    fn the_pane_opens_to_the_right_of_the_focused_agent_with_its_ids() {
        let mut world = World::new("focused");
        world.agents = vec![("w1:p2", "term_1")];
        let json = context("w1:p2", &world.root.display().to_string());
        let env = world.env(&[("HERDR_PLUGIN_CONTEXT_JSON", &json)]);
        assert_eq!(world.run(&env, None), Ok(Opened::Opened));
        let opened = world.opened();
        assert_eq!(opened.len(), 1);
        assert!(opened[0].contains("--target-pane w1:p2"));
        assert!(opened[0].contains("--env REVIEW_DELIVER_TO=w1:p2"));
        assert!(opened[0].contains("--env REVIEW_DELIVER_TERM=term_1"));
        assert!(opened[0].ends_with(&format!("--cwd {} --focus", world.root.display())));
    }

    #[test]
    fn without_a_context_the_current_directory_names_the_repository() {
        let world = World::new("no-context");
        world.run(&world.env(&[]), None).unwrap();
        let git = &world.calls()[0];
        assert_eq!(
            git,
            &format!("git -C {} rev-parse --show-toplevel", world.root.display())
        );
        let json = context("w1:p2", "/elsewhere");
        let env = world.env(&[("HERDR_PLUGIN_CONTEXT_JSON", &json)]);
        world.run(&env, None).unwrap();
        assert!(
            world
                .calls()
                .iter()
                .any(|c| c.starts_with("git -C /elsewhere "))
        );
    }

    #[test]
    fn a_pane_that_is_not_an_agent_falls_back_to_the_panes_own_id() {
        let mut world = World::new("own-id");
        world.agents = vec![("w1:p5", "term_5")];
        let json = context("w1:p9", &world.root.display().to_string());
        let env = world.env(&[
            ("HERDR_PLUGIN_CONTEXT_JSON", &json),
            ("HERDR_PANE_ID", "w1:p5"),
        ]);
        world.run(&env, None).unwrap();
        assert!(world.opened()[0].contains("--target-pane w1:p5"));
    }

    #[test]
    fn a_second_open_focuses_the_review_pane_that_is_already_open() {
        let mut world = World::new("second");
        world.panes = vec!["w1:p7"];
        let pane = PaneId::parse("w1:p7").unwrap();
        save(&world.dir(), &world.root, |meta| {
            meta.review_pane = Some(pane);
        })
        .unwrap();
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Focused));
        let calls = world.calls();
        assert!(calls.contains(&"plugin pane focus w1:p7".to_owned()));
        assert!(world.opened().is_empty());
    }

    #[test]
    fn a_review_pane_that_is_gone_or_cannot_be_focused_is_opened_again() {
        let mut world = World::new("stale");
        let pane = PaneId::parse("w1:p7").unwrap();
        save(&world.dir(), &world.root, |meta| {
            meta.review_pane = Some(pane);
        })
        .unwrap();
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Opened));
        assert!(
            !world
                .calls()
                .iter()
                .any(|c| c.starts_with("plugin pane focus"))
        );
        world.panes = vec!["w1:p7"];
        world.focus_fails = true;
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Opened));
        assert_eq!(world.opened().len(), 2);
    }

    #[test]
    fn with_no_agent_found_the_pane_still_opens_without_a_target() {
        let world = World::new("no-agent");
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Opened));
        let opened = world.opened();
        assert_eq!(opened.len(), 1);
        assert!(!opened[0].contains("--target-pane") && !opened[0].contains("REVIEW_DELIVER"));
    }

    #[test]
    fn several_agents_open_the_pane_without_a_target_too() {
        let mut world = World::new("several");
        let agent = |pane: &str, term: &str| {
            format!(
                r#"{{"agent":"claude","cwd":"{}","pane_id":"{pane}","terminal_id":"{term}","workspace_id":"w1"}}"#,
                world.root.display()
            )
        };
        world.list = format!(
            r#"{{"result":{{"agents":[{},{}]}}}}"#,
            agent("w1:p1", "term_1"),
            agent("w1:p2", "term_2")
        );
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Opened));
        assert!(!world.opened()[0].contains("--target-pane"));
    }

    #[test]
    fn a_single_agent_found_by_cwd_is_the_target_when_the_focus_is_elsewhere() {
        let mut world = World::new("by-cwd");
        world.list = format!(
            r#"{{"result":{{"agents":[{{"agent":"claude","cwd":"{}","pane_id":"w1:p3","terminal_id":"term_3","workspace_id":"w1"}}]}}}}"#,
            world.root.display()
        );
        world.run(&world.env(&[]), None).unwrap();
        let opened = world.opened();
        assert!(opened[0].contains("--target-pane w1:p3"));
        assert!(opened[0].contains("REVIEW_DELIVER_TERM=term_3"));
    }

    #[test]
    fn an_unreadable_meta_file_does_not_stop_the_pane() {
        let world = World::new("meta");
        std::fs::create_dir_all(world.dir()).unwrap();
        std::fs::write(world.dir().join("meta.json"), "{not json").unwrap();
        assert_eq!(world.run(&world.env(&[]), None), Ok(Opened::Opened));
    }

    #[test]
    fn outside_a_repository_the_action_notifies_and_exits_1() {
        let mut world = World::new("not-a-repo");
        world.not_a_repo = true;
        let result = world.run(&world.env(&[]), None);
        assert!(result.is_err());
        let mut notes = Vec::new();
        let code = report(&result, |title, body| {
            notes.push((title.to_owned(), body.map(str::to_owned)));
        });
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "review: cannot open");
        assert!(
            notes[0]
                .1
                .as_deref()
                .unwrap()
                .contains("not a git repository")
        );
        assert!(world.opened().is_empty());
    }

    #[test]
    fn a_herdr_error_on_open_is_reported() {
        let mut world = World::new("open-fails");
        world.open_fails = true;
        let result = world.run(&world.env(&[]), None);
        assert_eq!(result, Err("herdr said no".to_owned()));
        assert_eq!(report(&result, |_, _| {}), ExitCode::FAILURE);
        assert_eq!(
            report(&Ok(Opened::Opened), |_, _| panic!("notified")),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn a_base_is_saved_as_the_branch_spec_and_a_dash_is_ignored() {
        let world = World::new("base");
        world.run(&world.env(&[]), Some("--upload-pack=x")).unwrap();
        assert_eq!(load(&world.dir()).0.spec, None);
        world.run(&world.env(&[]), Some("develop")).unwrap();
        let meta = load(&world.dir()).0;
        assert_eq!(
            meta.spec,
            Some(Spec::Branch {
                base: "develop".into()
            })
        );
        assert_eq!(meta.base.as_deref(), Some("develop"));
    }

    #[test]
    fn a_stored_target_is_used_when_the_focus_is_not_an_agent() {
        let mut world = World::new("stored");
        world.agents = vec![("w1:p4", "term_4")];
        let target = Target {
            pane: PaneId::parse("w1:p4").unwrap(),
            terminal: TerminalId::parse("term_4").unwrap(),
            agent: "claude".into(),
        };
        save(&world.dir(), &world.root, |meta| meta.target = Some(target)).unwrap();
        world.run(&world.env(&[]), None).unwrap();
        assert!(world.opened()[0].contains("--target-pane w1:p4"));
    }
}
