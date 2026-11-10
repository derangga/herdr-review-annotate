use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::*;

const SESSION: &str = "0b9c6f1e-1111-4222-8333-444455556666";

/// A temporary `HOME`, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("herdr-review-mjson-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn env(&self) -> Env {
        Env::new(
            [("HOME".to_owned(), self.0.display().to_string())],
            self.0.clone(),
        )
    }

    /// A transcript under `.claude/projects/<project>/<name>.jsonl`, written `age` seconds ago.
    fn transcript(&self, project: &str, name: &str, id: &str, text: &str, age: u64) {
        let dir = self.0.join(".claude/projects").join(project);
        std::fs::create_dir_all(&dir).unwrap();
        let line = serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "message": {"id": id, "model": "claude-x", "content": [{"type": "text", "text": text}]}
        });
        let path = dir.join(format!("{name}.jsonl"));
        std::fs::write(&path, line.to_string()).unwrap();
        let at = SystemTime::now() - Duration::from_secs(age);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    /// The file `message send` reads.
    fn send_file(&self, text: &str) -> PathBuf {
        let path = self.0.join("send.json");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pane() -> Target {
    Target::Pane("w1:p2".into())
}

fn terminal() -> Target {
    Target::Terminal(TerminalAddress::parse("t7").unwrap())
}

fn agent(name: &str, status: &str, session: &str) -> Result<String, String> {
    Ok(format!(
        r#"{{"id":"cli:agent:get","result":{{"agent":{{"agent":"{name}","agent_status":"{status}","cwd":"/work","pane_id":"w1:p2","terminal_id":"term_1","agent_session":{session}}},"type":"agent_info"}}}}"#
    ))
}

fn claude(status: &str) -> Result<String, String> {
    agent(
        "claude",
        status,
        &format!(r#"{{"kind":"id","value":"{SESSION}"}}"#),
    )
}

/// An hc record for a terminal started in `/work/proj`.
fn record(app: &str, state: &str, status: &str) -> Result<String, HcError> {
    Ok(format!(
        r#"{{"address":"t7","cwd":"/work/proj","state":"{state}","reported":true,"status":{{"app":"{app}","state":"{status}"}}}}"#
    ))
}

fn no_herdr(_: &[String]) -> Result<String, String> {
    Err("herdr must not be called".to_owned())
}

fn no_hc(_: &[String]) -> Result<String, HcError> {
    Err(HcError::default())
}

fn said(failure: Result<Output, Failure>) -> String {
    match failure.unwrap_err() {
        Failure::Message(text) => text,
        Failure::Command(error) => error.to_string(),
        other => panic!("unexpected {other:?}"),
    }
}

const MESSAGE: &str = "# Plan\nStep one\nStep two\nStep three";

#[test]
fn a_herdr_agents_newest_message_is_printed_as_json() {
    let home = Home::new("read-pane");
    home.transcript("p", SESSION, "m1", MESSAGE, 0);
    let out = read(&pane(), &home.env(), |_| claude("working"), no_hc).unwrap();
    let json = serde_json::from_str::<MessageJson>(&out.stdout).unwrap();
    assert_eq!(
        json,
        MessageJson {
            schema: SCHEMA,
            agent: "claude".into(),
            status: "working".into(),
            id: "m1".into(),
            lines: MESSAGE.lines().map(str::to_owned).collect(),
        }
    );
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
}

#[test]
fn a_message_that_cannot_be_read_fails_with_the_reason() {
    let home = Home::new("read-fails");
    let env = home.env();
    let unsupported = read(&pane(), &env, |_| agent("codex", "idle", "null"), no_hc);
    assert_eq!(
        said(unsupported),
        "Message review reads Claude Code only, not codex."
    );
    let no_session = read(&pane(), &env, |_| agent("claude", "idle", "null"), no_hc);
    assert_eq!(said(no_session), "Herdr has no session for this agent yet.");
    let no_file = read(&pane(), &env, |_| claude("idle"), no_hc);
    assert!(said(no_file).starts_with("Cannot find the transcript of session"));
    let gone = Err(r#"{"error":{"code":"agent_not_found","message":"x"}}"#.to_owned());
    assert_eq!(
        said(read(&pane(), &env, |_| gone.clone(), no_hc)),
        "No agent is running in that pane."
    );
    let failure = read(&pane(), &env, |_| Err("connection refused".into()), no_hc).unwrap_err();
    assert_eq!(failure.output().code, 1);
}

#[test]
fn an_hc_terminals_message_is_the_newest_session_of_its_folder() {
    let home = Home::new("read-hc");
    home.transcript("-work-proj", "old", "m1", "old text", 100);
    home.transcript("-work-proj", "new", "m2", "new text", 0);
    home.transcript("-other", "newest", "m3", "elsewhere", 0);
    let out = read(&terminal(), &home.env(), no_herdr, |args| {
        assert_eq!(args, ["get", "t7", "--json"]);
        record("claude-code", "live", "idle")
    })
    .unwrap();
    let json = serde_json::from_str::<MessageJson>(&out.stdout).unwrap();
    assert_eq!(
        (json.agent.as_str(), json.status.as_str(), json.id.as_str()),
        ("claude-code", "idle", "m2")
    );
    assert_eq!(json.lines, ["new text"]);
}

#[test]
fn an_hc_terminal_that_is_not_claude_or_is_gone_or_has_no_session_fails() {
    let home = Home::new("read-hc-fails");
    let env = home.env();
    let read_with = |reply: Result<String, HcError>| {
        said(read(&terminal(), &env, no_herdr, move |_| reply.clone()))
    };
    assert_eq!(
        read_with(record("codex", "live", "idle")),
        "Message review reads Claude Code only, not codex."
    );
    assert_eq!(
        read_with(record("claude-code", "finished", "idle")),
        "No agent is running in that pane."
    );
    let missing = HcError {
        exit: Some(3),
        kind: Some("not_found".into()),
        message: "no t7".into(),
    };
    assert_eq!(read_with(Err(missing)), "No agent is running in that pane.");
    assert_eq!(
        read_with(record("claude-code", "live", "idle")),
        "Cannot find a Claude Code session for /work/proj."
    );
}

const COMMENTS: &str = r#"{"id":"m1","comments":[
    {"start":2,"end":3,"body":"Why two steps?"},
    {"start":1,"end":1,"body":"Rename this"}]}"#;

#[test]
fn comments_are_sent_to_a_herdr_agent_as_the_prompt_of_the_tui() {
    let home = Home::new("send-pane");
    home.transcript("p", SESSION, "m1", MESSAGE, 0);
    let file = home.send_file(COMMENTS);
    let calls = RefCell::new(Vec::<Vec<String>>::new());
    let out = send(
        &pane(),
        &file,
        &home.env(),
        |args| {
            calls.borrow_mut().push(args.to_vec());
            claude("idle")
        },
        no_hc,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<MessageSentJson>(&out.stdout).unwrap(),
        MessageSentJson {
            schema: SCHEMA,
            sent: 2
        }
    );
    let lines = MESSAGE.lines().map(str::to_owned).collect::<Vec<_>>();
    let comment = |start, end, body: &str| MessageComment {
        start,
        end,
        chars: None,
        body: body.into(),
    };
    let expected = format_prompt(
        &lines,
        &[
            comment(2, 3, "Why two steps?"),
            comment(1, 1, "Rename this"),
        ],
    );
    let calls = calls.borrow();
    let prompt = calls.last().unwrap();
    assert_eq!(prompt[..3], ["agent", "prompt", "w1:p2"]);
    assert_eq!(prompt[3], expected);
    assert!(expected.starts_with("Feedback on your last message."));
    assert!(!file.exists(), "the file is deleted once read");
}

#[test]
fn comments_are_sent_to_an_hc_terminal_with_hc_send() {
    let home = Home::new("send-hc");
    home.transcript("-work-proj", "s", "m1", MESSAGE, 0);
    let file = home.send_file(COMMENTS);
    let sent = RefCell::new(Vec::<String>::new());
    let out = send(&terminal(), &file, &home.env(), no_herdr, |args| {
        if args.first().is_some_and(|a| a == "send") {
            sent.borrow_mut().extend(args.iter().cloned());
            return Ok(r#"{"status":"idle"}"#.to_owned());
        }
        record("claude-code", "live", "idle")
    })
    .unwrap();
    assert!(out.stdout.contains(r#""sent":2"#));
    let sent = sent.borrow();
    assert_eq!(sent[..3], ["send", "--json", "t7"]);
    assert!(sent[3].contains("- lines 2-3:\n  > Step one\n  > Step two\n  Why two steps?"));
}

#[test]
fn a_refused_delivery_fails_and_types_nothing() {
    let home = Home::new("send-refused");
    home.transcript("p", SESSION, "m1", MESSAGE, 0);
    let prompts = RefCell::new(0);
    let blocked = send(
        &pane(),
        &home.send_file(COMMENTS),
        &home.env(),
        |args| {
            if args.get(1).is_some_and(|a| a == "prompt") {
                *prompts.borrow_mut() += 1;
            }
            claude("blocked")
        },
        no_hc,
    );
    let text = said(blocked);
    assert!(
        text.starts_with("The agent is waiting on a prompt."),
        "{text}"
    );
    assert!(text.contains("Nothing was sent"));
    assert_eq!(*prompts.borrow(), 0);

    home.transcript("-work-proj", "s", "m1", MESSAGE, 0);
    let hc_blocked = send(
        &terminal(),
        &home.send_file(COMMENTS),
        &home.env(),
        no_herdr,
        |_| record("claude-code", "live", "blocked"),
    );
    assert!(said(hc_blocked).contains("Nothing was sent"));
}

#[test]
fn a_send_file_that_does_not_fit_the_message_is_refused_before_anything_is_sent() {
    let home = Home::new("send-invalid");
    home.transcript("p", SESSION, "m1", MESSAGE, 0);
    let env = home.env();
    let refuse = |text: &str| {
        let file = home.send_file(text);
        let failure = send(&pane(), &file, &env, |_| claude("idle"), no_hc).unwrap_err();
        assert_eq!(failure.output().code, 2, "{text}");
        said(Err(failure))
    };
    assert!(refuse(r#"{"comments":[]}"#).contains("no comments"));
    assert!(refuse(r#"{"comments":[{"start":0,"end":1,"body":"x"}]}"#).contains("comments[0]"));
    assert!(
        refuse(r#"{"comments":[{"start":3,"end":2,"body":"x"}]}"#).contains("not in the message")
    );
    assert!(refuse(r#"{"comments":[{"start":1,"end":5,"body":"x"}]}"#).contains("lines 1-5"));
    assert!(refuse(r#"{"comments":[{"start":1,"end":1,"body":"  "}]}"#).contains("comments[0]"));
    assert!(
        refuse(r#"{"id":"m0","comments":[{"start":1,"end":1,"body":"x"}]}"#).contains("changed")
    );
    assert!(refuse("not json").contains("invalid batch"));
}

#[test]
fn an_unreadable_send_file_is_refused() {
    let home = Home::new("send-nofile");
    let missing = home.0.join("nope.json");
    let failure = send(&pane(), &missing, &home.env(), no_herdr, no_hc).unwrap_err();
    assert_eq!(failure.output().code, 2);
}
