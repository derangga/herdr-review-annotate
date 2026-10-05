//! The pane against a real `herdr-review comment` process: what the agent writes shows up on its
//! own, with no key pressed.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::cell::Cell;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use herdr_review::actions;
use herdr_review::diff::run_git_bytes;
use herdr_review::env::Env;
use herdr_review::store::{Anchor, AnchorTarget, RelPath, Side, Spec, state_dir};
use herdr_review::tui::{App, render, run_loop};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn a_reply_from_the_comment_command_appears_within_a_second_with_no_key_pressed() {
    let home = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("herdr-review-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let root = home.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let init = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["init", "-q"])
        .status();
    assert!(init.unwrap().success());
    std::fs::write(root.join("a.rs"), "let x = 1;\n").unwrap();

    let env = Env::new(
        [("HOME".to_owned(), home.display().to_string())],
        root.clone(),
    );
    let dir = state_dir(&env.state_base().unwrap(), &root);
    let anchor = Anchor {
        path: RelPath::parse("a.rs").unwrap(),
        old_path: None,
        target: AnchorTarget::Line {
            side: Side::New,
            line: 1,
            text: "let x = 1;".into(),
        },
        spec: Spec::WorkTree,
    };
    actions::comment(&dir, "2026-10-05T00:00:00Z", &anchor, "fix").unwrap();

    let mut git = run_git_bytes;
    let mut app = App::new(env, Some(root.clone()));
    app.load(&mut git);
    assert_eq!(app.review.threads.len(), 1);
    assert!(app.review.threads[0].replies.is_empty());

    // The agent replies from its own process while the pane runs.
    let finished = Arc::new(Mutex::new(None));
    let agent = {
        let (finished, root, home) = (Arc::clone(&finished), root.clone(), home.clone());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-review"))
                .args(["comment", "reply", "--repo"])
                .arg(&root)
                .args(["--name", "claude", "u1", "-"])
                .env("HOME", &home)
                .env_remove("XDG_STATE_HOME")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"ack from the agent")
                .unwrap();
            assert!(child.wait().unwrap().success());
            *finished.lock().unwrap() = Some(Instant::now());
        })
    };

    let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
    let (stop, started) = (Cell::new(false), Instant::now());
    run_loop(
        &mut app,
        &mut terminal,
        &mut git,
        |timeout| {
            std::thread::sleep(timeout);
            stop.set(finished.lock().unwrap().is_some());
            Ok(None)
        },
        || stop.get() || started.elapsed() > Duration::from_secs(10),
    );
    agent.join().unwrap();
    let finished_at = finished.lock().unwrap().unwrap();
    assert!(
        finished_at.elapsed() < Duration::from_secs(1),
        "the pane took {:?} to pick up the reply",
        finished_at.elapsed()
    );
    assert_eq!(app.review.threads[0].replies.len(), 1);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(screen.contains("ack from the agent"), "{screen}");
    assert!(screen.contains("agent:claude"), "{screen}");
    let _ = std::fs::remove_dir_all(&home);
}
