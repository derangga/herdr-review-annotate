//! The `open` action: put the review pane to the right of the agent pane.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::Value;

use crate::herdr::{notify, run_herdr_output};

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

fn open(
    context: &Context,
    cwd: PathBuf,
    mut herdr: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<(), String> {
    let root = context.cwd.clone().unwrap_or(cwd);
    let terminal = context.pane.as_deref().and_then(|pane| {
        terminal_id(&herdr(&[
            "agent".to_owned(),
            "get".to_owned(),
            pane.to_owned(),
        ]))
    });
    herdr(&pane_open_args(
        &root.to_string_lossy(),
        context.pane.as_deref(),
        terminal.as_deref(),
    ))
    .map(drop)
}

pub fn run() -> ExitCode {
    let context = parse_context(std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok().as_deref());
    let cwd = std::env::current_dir().unwrap_or_default();
    match open(&context, cwd, run_herdr_output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // An action has no terminal, so the failure goes to a notification.
            notify(&format!("review: {message}"), None);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
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

    #[test]
    fn open_runs_agent_get_then_pane_open() {
        let calls = std::cell::RefCell::new(Vec::new());
        let context = Context {
            pane: Some("w1:p2".into()),
            cwd: Some("/repo".into()),
        };
        let result = open(&context, "/elsewhere".into(), |args| {
            calls.borrow_mut().push(args.join(" "));
            Ok(r#"{"result":{"agent":{"terminal_id":"term_1"}}}"#.to_owned())
        });
        assert!(result.is_ok());
        let calls = calls.into_inner();
        assert_eq!(calls.first().map(String::as_str), Some("agent get w1:p2"));
        assert!(
            calls
                .get(1)
                .is_some_and(|call| call.contains("REVIEW_DELIVER_TERM=term_1"))
        );
    }
}
