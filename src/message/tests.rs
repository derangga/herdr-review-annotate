use std::path::PathBuf;

use super::*;

const SESSION: &str = "0b9c6f1e-1111-4222-8333-444455556666";

/// A temporary `HOME`, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "herdr-review-message-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn env(&self, config: Option<&str>) -> Env {
        let mut vars = vec![("HOME".to_owned(), self.0.display().to_string())];
        if let Some(config) = config {
            vars.push(("CLAUDE_CONFIG_DIR".to_owned(), config.to_owned()));
        }
        Env::new(vars, self.0.clone())
    }

    /// Write `lines` as the transcript of `session` under `<config>/projects/p`.
    fn transcript(&self, config: &str, session: &str, lines: &[String]) -> PathBuf {
        let dir = if config.starts_with('/') {
            PathBuf::from(config)
        } else {
            self.0.join(config)
        };
        let dir = dir.join("projects").join("p");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session}.jsonl"));
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assistant(id: &str, block: &str) -> String {
    format!(
        r#"{{"type":"assistant","isSidechain":false,"uuid":"u","message":{{"id":"{id}","model":"claude-x","content":[{block}]}}}}"#
    )
}

fn text(id: &str, text: &str) -> String {
    assistant(id, &format!(r#"{{"type":"text","text":{}}}"#, json(text)))
}

fn json(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

fn tool_use(id: &str) -> String {
    assistant(
        id,
        r#"{"type":"tool_use","id":"t","name":"Bash","input":{}}"#,
    )
}

fn thinking(id: &str) -> String {
    assistant(id, r#"{"type":"thinking","thinking":"hmm"}"#)
}

#[allow(clippy::unnecessary_wraps, reason = "the shape `herdr` returns")]
fn agent(name: &str, session: &str) -> Result<String, String> {
    Ok(format!(
        r#"{{"id":"cli:agent:get","result":{{"agent":{{"agent":"{name}","agent_status":"idle","cwd":"/work","pane_id":"w1:p2","terminal_id":"term_1","agent_session":{session}}},"type":"agent_info"}}}}"#
    ))
}

fn id_session(id: &str) -> String {
    format!(r#"{{"agent":"claude","kind":"id","source":"herdr:claude","value":"{id}"}}"#)
}

fn newest(home: &Home, lines: &[String]) -> Result<AgentMessage, MessageError> {
    let path = home.transcript(".claude", SESSION, lines);
    read_message(&path)
}

#[test]
fn an_agent_that_is_not_claude_is_unsupported() {
    let home = Home::new("unsupported");
    let got = agent("codex", &id_session(SESSION));
    assert_eq!(
        load(&got, &home.env(None)),
        Err(MessageError::Unsupported("codex".into()))
    );
}

#[test]
fn an_agent_without_a_session_has_no_transcript_to_find() {
    let home = Home::new("no-session");
    assert_eq!(
        load(&agent("claude", "null"), &home.env(None)),
        Err(MessageError::NoSession)
    );
    let odd = r#"{"agent":"claude","kind":"other","value":"x"}"#;
    assert_eq!(
        load(&agent("claude", odd), &home.env(None)),
        Err(MessageError::NoSession)
    );
}

#[test]
fn a_pane_that_is_not_an_agent_is_refused() {
    let home = Home::new("not-an-agent");
    let gone = Err(
        r#"{"error":{"code":"agent_not_found","message":"agent target w1:p2 not found"}}"#
            .to_owned(),
    );
    assert_eq!(load(&gone, &home.env(None)), Err(MessageError::NotAnAgent));
    let bare =
        Ok(r#"{"result":{"agent":{"agent_status":"unknown","pane_id":"w1:p2"}}}"#.to_owned());
    assert_eq!(load(&bare, &home.env(None)), Err(MessageError::NotAnAgent));
    let broken = Err("connection refused".to_owned());
    assert!(matches!(
        load(&broken, &home.env(None)),
        Err(MessageError::Herdr(_))
    ));
}

#[test]
fn the_agent_record_is_read_from_agent_get() {
    let got = agent("claude", &id_session(SESSION));
    let agent = parse_agent(&got).unwrap();
    assert_eq!(agent.pane.as_str(), "w1:p2");
    assert_eq!(agent.terminal.as_str(), "term_1");
    assert_eq!(agent.cwd, PathBuf::from("/work"));
    assert_eq!(agent.status, AgentStatus::Idle);
    assert_eq!(agent.session, Some(Session::Id(SESSION.into())));
    assert_eq!(agent.name, "claude");
}

#[test]
fn a_path_session_is_used_when_the_file_exists() {
    let home = Home::new("path");
    let path = home.transcript("elsewhere", "any", &[text("m1", "hello")]);
    let session = format!(r#"{{"kind":"path","value":"{}"}}"#, path.display());
    let (_, message) = load(&agent("claude", &session), &home.env(None)).unwrap();
    assert_eq!(message.lines, ["hello"]);
    assert_eq!(message.transcript, path);

    let missing = format!(
        r#"{{"kind":"path","value":"{}/nope.jsonl"}}"#,
        home.0.display()
    );
    assert!(matches!(
        load(&agent("claude", &missing), &home.env(None)),
        Err(MessageError::NoTranscript { .. })
    ));
}

#[test]
fn the_id_is_found_under_the_config_dir_then_the_default_then_any_claude_dir() {
    let home = Home::new("roots");
    let got = agent("claude", &id_session(SESSION));
    let line = |from: &str| [text("m1", from)];

    home.transcript(".claude-work", SESSION, &line("work"));
    let (_, message) = load(&got, &home.env(None)).unwrap();
    assert_eq!(message.lines, ["work"], "~/.claude-work");

    home.transcript(".claude", SESSION, &line("default"));
    let (_, message) = load(&got, &home.env(None)).unwrap();
    assert_eq!(message.lines, ["default"], "~/.claude");

    let other = home.0.join("elsewhere");
    home.transcript(&other.display().to_string(), SESSION, &line("config"));
    let env = home.env(Some(&other.display().to_string()));
    let (_, message) = load(&got, &env).unwrap();
    assert_eq!(message.lines, ["config"], "CLAUDE_CONFIG_DIR");
}

#[test]
fn a_session_with_no_file_has_no_transcript() {
    let home = Home::new("no-file");
    home.transcript(".claude", "another-session", &[text("m1", "x")]);
    assert_eq!(
        load(&agent("claude", &id_session(SESSION)), &home.env(None)),
        Err(MessageError::NoTranscript { id: SESSION.into() })
    );
}

#[test]
fn an_id_that_could_leave_the_projects_directory_finds_nothing() {
    let home = Home::new("escape");
    home.transcript(".claude", "x", &[text("m1", "x")]);
    let session = id_session("../p/x");
    assert!(matches!(
        load(&agent("claude", &session), &home.env(None)),
        Err(MessageError::NoTranscript { .. })
    ));
}

#[test]
fn a_line_that_is_not_json_is_skipped() {
    let home = Home::new("bad-line");
    let lines = [
        text("m1", "one"),
        "{not json".to_owned(),
        String::new(),
        r#"{"type":"assistant","message":"odd"}"#.to_owned(),
    ];
    assert_eq!(newest(&home, &lines).unwrap().lines, ["one"]);
}

#[test]
fn sidechain_synthetic_and_non_assistant_lines_are_not_messages() {
    let home = Home::new("skipped");
    let sidechain =
        text("m2", "sub-agent").replace(r#""isSidechain":false"#, r#""isSidechain":true"#);
    let synthetic = text("m3", "No response requested.").replace("claude-x", "<synthetic>");
    let user = r#"{"type":"user","message":{"role":"user","content":"a prompt"}}"#.to_owned();
    let other =
        r#"{"type":"attachment","message":{"id":"m5","content":[{"type":"text","text":"x"}]}}"#
            .to_owned();
    let lines = [
        text("m1", "the real one"),
        sidechain,
        synthetic,
        user,
        other,
    ];
    let message = newest(&home, &lines).unwrap();
    assert_eq!(
        (message.id.as_str(), &message.lines[..]),
        ("m1", &["the real one".to_owned()][..])
    );
}

#[test]
fn thinking_and_tool_use_blocks_are_dropped() {
    let home = Home::new("blocks");
    let lines = [thinking("m1"), text("m1", "Done."), tool_use("m1")];
    assert_eq!(newest(&home, &lines).unwrap().lines, ["Done."]);
}

#[test]
fn one_message_id_over_several_lines_is_joined_by_a_blank_line() {
    let home = Home::new("joined");
    let lines = [
        text("m0", "earlier"),
        text("m1", "first\nsecond\n"),
        tool_use("m1"),
        text("m1", "\n\nthird"),
    ];
    let message = newest(&home, &lines).unwrap();
    assert_eq!(message.lines, ["first", "second", "", "third"]);
}

#[test]
fn the_last_id_with_text_wins_over_a_later_line_with_only_tool_use() {
    let home = Home::new("tool-only");
    let lines = [text("m1", "older"), text("m2", "newer"), tool_use("m3")];
    assert_eq!(newest(&home, &lines).unwrap().lines, ["newer"]);
}

#[test]
fn blank_text_and_an_empty_transcript_have_no_message() {
    let home = Home::new("none");
    let lines = [thinking("m1"), text("m1", "  \n"), tool_use("m1")];
    let path = home.transcript(".claude", SESSION, &lines);
    assert_eq!(
        read_message(&path),
        Err(MessageError::NoMessage { path: path.clone() })
    );
    let empty = home.transcript(".claude", "empty", &[]);
    assert!(matches!(
        read_message(&empty),
        Err(MessageError::NoMessage { .. })
    ));
}

#[test]
fn a_transcript_that_cannot_be_read_is_an_io_error() {
    let home = Home::new("io");
    let path = home.0.join("missing.jsonl");
    assert_eq!(
        read_message(&path),
        Err(MessageError::Io {
            path: path.clone(),
            kind: ErrorKind::NotFound
        })
    );
}

fn comment(start: u32, end: u32, body: &str) -> MessageComment {
    MessageComment {
        start,
        end,
        body: body.to_owned(),
    }
}

fn numbered(count: usize) -> Vec<String> {
    (1..=count).map(|n| format!("text {n}")).collect()
}

#[test]
fn the_prompt_quotes_the_lines_and_indents_the_comment_under_them() {
    let lines = numbered(40);
    let prompt = format_prompt(
        &lines,
        &[
            comment(2, 3, "Why not both?"),
            comment(30, 30, "Fine for now."),
        ],
    );
    assert_eq!(
        prompt,
        "Feedback on your last message. Address each point.\n\
         \n\
         - lines 2-3:\n\
         \x20 > text 2\n\
         \x20 > text 3\n\
         \x20 Why not both?\n\
         - line 30:\n\
         \x20 > text 30\n\
         \x20 Fine for now."
    );
}

#[test]
fn comments_go_in_line_order_and_then_in_the_order_written() {
    let lines = numbered(10);
    let prompt = format_prompt(
        &lines,
        &[
            comment(5, 5, "late"),
            comment(2, 9, "wide"),
            comment(2, 2, "narrow, written second"),
            comment(1, 1, "first"),
        ],
    );
    let items = prompt
        .lines()
        .filter(|line| line.starts_with("- "))
        .collect::<Vec<_>>();
    assert_eq!(
        items,
        ["- line 1:", "- lines 2-9:", "- line 2:", "- line 5:"]
    );
}

#[test]
fn a_range_of_more_than_six_lines_is_cut_to_five_and_a_count() {
    let lines = numbered(20);
    let quote = |start, end| {
        format_prompt(&lines, &[comment(start, end, "x")])
            .lines()
            .filter(|line| line.starts_with("  >"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(quote(3, 8).len(), 6, "six lines are quoted whole");
    let cut = quote(3, 9);
    assert_eq!(
        cut,
        [
            "  > text 3",
            "  > text 4",
            "  > text 5",
            "  > text 6",
            "  > text 7",
            "  > … (2 more lines)"
        ]
    );
    assert_eq!(quote(1, 20).last().unwrap(), "  > … (15 more lines)");
}

#[test]
fn a_comment_of_several_lines_is_indented_on_each_and_blank_lines_stay_blank() {
    let lines = numbered(3);
    let prompt = format_prompt(&lines, &[comment(1, 1, "first\n\n  nested\nlast")]);
    let body = prompt.lines().skip(4).collect::<Vec<_>>();
    assert_eq!(body, ["  first", "", "    nested", "  last"]);
}

#[test]
fn an_empty_line_is_quoted_without_a_trailing_space() {
    let lines = ["a".to_owned(), String::new(), "b".to_owned()];
    let prompt = format_prompt(&lines, &[comment(1, 3, "x")]);
    assert!(prompt.contains("  > a\n  >\n  > b\n"), "{prompt:?}");
    assert!(prompt.lines().all(|line| line == line.trim_end()));
}

#[test]
fn a_range_past_the_end_of_the_message_quotes_what_is_there() {
    let lines = numbered(2);
    let prompt = format_prompt(&lines, &[comment(2, 5, "x"), comment(9, 9, "y")]);
    assert!(
        prompt.contains("- lines 2-5:\n  > text 2\n  x"),
        "{prompt:?}"
    );
    assert!(prompt.contains("- line 9:\n  y"), "{prompt:?}");
}
