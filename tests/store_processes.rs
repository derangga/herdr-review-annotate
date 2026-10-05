//! The store with several processes. Each test starts copies of this test binary, which run the
//! `child_*` tests below when their environment variable is set and do nothing otherwise.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use herdr_review::store::{Add, Author, Event, Kind, RelPath, Spec, StoreError, WriteError, read, write};

const DIR: &str = "REVIEW_TEST_DIR";
const APPENDS: usize = 200;
const PROCESSES: usize = 8;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("herdr-review-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn child(test: &str, dir: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(DIR, dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

fn agent_comment(by: &Author, id: herdr_review::store::CommentId, now: &str) -> Event {
    Event {
        at: now.into(),
        by: by.clone(),
        kind: Kind::Add(Add {
            id,
            parent: None,
            path: RelPath::parse("a.rs"),
            old_path: None,
            side: None,
            line: None,
            end_line: None,
            line_text: None,
            spec: Some(Spec::WorkTree),
            body: "x".into(),
        }),
    }
}

#[test]
fn child_appends() {
    let Some(dir) = std::env::var_os(DIR) else { return };
    let by = Author::Agent(Some("child".into()));
    for _ in 0..APPENDS {
        let result = write(Path::new(&dir), "t", |review, now| {
            let id = review.ids.clone().comment(&by);
            Ok::<_, ()>((vec![agent_comment(&by, id, now)], ()))
        });
        assert!(result.is_ok(), "{result:?}");
    }
}

#[test]
fn child_holds_the_lock() {
    let Some(dir) = std::env::var_os(DIR) else { return };
    let result = write(Path::new(&dir), "t", |_, _| {
        #[allow(clippy::print_stdout, reason = "tells the parent test the lock is held")]
        {
            println!("locked");
        }
        std::thread::sleep(Duration::from_secs(60));
        Ok::<_, ()>((Vec::new(), ()))
    });
    assert!(result.is_ok());
}

#[test]
fn eight_processes_appending_200_events_each_leave_1600_valid_lines_and_no_duplicate_id() {
    let dir = temp_dir("append");
    let children = (0..PROCESSES)
        .map(|_| child("child_appends", &dir).spawn().unwrap())
        .collect::<Vec<_>>();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let text = std::fs::read_to_string(dir.join("review.jsonl")).unwrap();
    let ids = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["id"].to_string())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(text.lines().count(), PROCESSES * APPENDS);
    assert_eq!(ids.len(), PROCESSES * APPENDS);
    let review = read(&dir).unwrap();
    assert_eq!((review.threads.len(), review.skipped_lines), (PROCESSES * APPENDS, 0));
}

fn wait_for_lock(holder: &mut Child) {
    let stdout = holder.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines() {
        if line.unwrap().ends_with("locked") {
            return;
        }
    }
    panic!("the holder never took the lock");
}

#[test]
fn a_lock_held_by_another_process_gives_busy_and_killing_the_holder_frees_it() {
    let dir = temp_dir("holder");
    let mut holder = child("child_holds_the_lock", &dir).spawn().unwrap();
    wait_for_lock(&mut holder);
    let busy = write(&dir, "t", |_, _| Ok::<_, ()>((Vec::new(), ())));
    assert_eq!(busy, Err(WriteError::Store(StoreError::Busy)));
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert!(write(&dir, "t", |_, _| Ok::<_, ()>((Vec::new(), ()))).is_ok());
}
