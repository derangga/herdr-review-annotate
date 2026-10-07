use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::diff::Change;
use crate::store::{
    Add, AnchorTarget, Author, CommentId, Event as LogEvent, Kind, RelPath, Side, state_dir,
};

const PATCH: &[u8] = b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

/// The Herdr the pane talks to in a test: the agents it lists, and every call it got.
#[derive(Default)]
struct FakeHerdr {
    /// `(pane, terminal, status)`, all named claude and working in the repository root.
    agents: Vec<(&'static str, &'static str, &'static str)>,
    cwd: String,
    calls: Vec<String>,
    prompt_fails: bool,
}

impl FakeHerdr {
    fn agent(&self, (pane, term, status): (&str, &str, &str)) -> String {
        format!(
            r#"{{"agent":"claude","agent_status":"{status}","cwd":"{}","pane_id":"{pane}","terminal_id":"{term}","workspace_id":"w1"}}"#,
            self.cwd
        )
    }

    fn answer(&mut self, args: &[String]) -> Result<String, String> {
        let call = args.join(" ");
        self.calls.push(call.clone());
        if call == "agent list" {
            let agents = self
                .agents
                .iter()
                .map(|a| self.agent(*a))
                .collect::<Vec<_>>();
            return Ok(format!(
                r#"{{"result":{{"agents":[{}]}}}}"#,
                agents.join(",")
            ));
        }
        if let Some(pane) = call.strip_prefix("agent get ") {
            let found = self.agents.iter().find(|a| a.0 == pane);
            return found.map_or(Err("pane_not_found".into()), |a| {
                Ok(format!(r#"{{"result":{{"agent":{}}}}}"#, self.agent(*a)))
            });
        }
        if call.starts_with("agent prompt ") && self.prompt_fails {
            return Err("socket gone".into());
        }
        Ok("{}".into())
    }
}

/// A home directory with a repository root and a config directory, and a `git` to go with it.
struct Fixture {
    home: PathBuf,
    root: PathBuf,
    env: Env,
    repo_ok: Cell<bool>,
    diff_error: RefCell<Option<GitError>>,
    merge_base_missing: Cell<bool>,
    no_refs: Cell<bool>,
    patch: RefCell<Vec<u8>>,
    diffs: Cell<usize>,
    /// How many `git show` calls highlighting made.
    shows: Cell<usize>,
    herdr: Rc<RefCell<FakeHerdr>>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let home = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("herdr-review-tui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(home.join("config")).unwrap();
        let env = Env::new(
            [
                ("HOME".to_owned(), home.display().to_string()),
                ("HERDR_PANE_ID".to_owned(), "w1:p9".to_owned()),
                (
                    "HERDR_PLUGIN_CONFIG_DIR".to_owned(),
                    home.join("config").display().to_string(),
                ),
            ],
            "/work".into(),
        );
        let root = root.canonicalize().unwrap();
        let herdr = FakeHerdr {
            cwd: root.display().to_string(),
            ..FakeHerdr::default()
        };
        Self {
            root,
            herdr: Rc::new(RefCell::new(herdr)),
            home,
            env,
            repo_ok: Cell::new(true),
            diff_error: RefCell::new(None),
            merge_base_missing: Cell::new(false),
            no_refs: Cell::new(false),
            patch: RefCell::new(PATCH.to_vec()),
            diffs: Cell::new(0),
            shows: Cell::new(0),
        }
    }

    fn dir(&self) -> PathBuf {
        state_dir(&self.env.state_base().unwrap(), &self.root)
    }

    fn app(&self) -> App {
        let mut app = App::new(self.env.clone(), None);
        app.now = || "2026-10-05T00:00:00Z".to_owned();
        let herdr = Rc::clone(&self.herdr);
        app.herdr = HerdrCall::new(move |args| herdr.borrow_mut().answer(args));
        app
    }

    /// The agents the fake Herdr lists. Set them before the pane starts.
    fn agents(&self, agents: &[(&'static str, &'static str, &'static str)]) {
        self.herdr.borrow_mut().agents = agents.to_vec();
    }

    fn calls_starting(&self, start: &str) -> Vec<String> {
        let calls = self.herdr.borrow().calls.clone();
        calls.into_iter().filter(|c| c.starts_with(start)).collect()
    }

    fn prompts(&self) -> Vec<String> {
        self.calls_starting("agent prompt ")
    }

    fn git(&self, args: &[String]) -> Result<Vec<u8>, GitError> {
        let failed = || GitError::Failed {
            args: args.join(" "),
            stderr: String::new(),
        };
        let args = args.iter().skip(2).map(String::as_str).collect::<Vec<_>>();
        match args.as_slice() {
            ["rev-parse", "--show-toplevel"] if self.repo_ok.get() => {
                Ok(format!("{}\n", self.root.display()).into_bytes())
            }
            ["rev-parse", "--show-toplevel"] => Err(GitError::NotARepo),
            ["merge-base", ..] if self.merge_base_missing.get() => Err(failed()),
            ["rev-parse", "--verify", "--quiet", name] if self.no_refs.get() && *name != "HEAD" => {
                Err(failed())
            }
            ["rev-parse" | "merge-base", ..] => Ok(b"abc\n".to_vec()),
            ["-c", _, "diff", ..] => {
                self.diffs.set(self.diffs.get() + 1);
                match self.diff_error.borrow().clone() {
                    Some(error) => Err(error),
                    None => Ok(self.patch.borrow().clone()),
                }
            }
            ["ls-files", ..] => Ok(Vec::new()),
            ["show", ..] => {
                self.shows.set(self.shows.get() + 1);
                Err(failed())
            }
            _ => Err(failed()),
        }
    }

    /// Run `body` with the fixture's `git` as the closure the pane takes.
    fn with_git<T>(&self, body: impl FnOnce(&mut Git) -> T) -> T {
        body(&mut |args: &[String]| self.git(args))
    }

    fn started(&self) -> App {
        let mut app = self.app();
        self.with_git(|git| app.load(git));
        assert_eq!(app.screen, Screen::Review);
        app
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(self.dir(), std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn terminal() -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(80, 12)).unwrap()
}

fn screen_of(app: &App) -> String {
    let mut terminal = terminal();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(c: char) -> Event {
    let mods = if c.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    Event::Key(KeyEvent::new(KeyCode::Char(c), mods))
}

/// Run the loop over `events`, one per tick, then stop as if signalled. `before` runs ahead of
/// each tick with its index.
fn drive(
    fixture: &Fixture,
    app: &mut App,
    events: Vec<Option<Event>>,
    mut before: impl FnMut(usize),
) -> Exit {
    let count = events.len();
    let mut events = events.into_iter();
    let polls = Cell::new(0);
    let poll = |_| {
        before(polls.get());
        polls.set(polls.get() + 1);
        Ok(events.next().flatten())
    };
    fixture.with_git(|git| run_loop(app, &mut terminal(), git, poll, || polls.get() >= count))
}

fn add_event(id: &str) -> LogEvent {
    LogEvent {
        at: "2026-10-05T00:00:00Z".into(),
        by: Author::User,
        kind: Kind::Add(Add {
            id: CommentId::parse(id).unwrap(),
            parent: None,
            path: RelPath::parse("a.rs"),
            old_path: None,
            side: Some(Side::New),
            line: Some(1),
            end_line: None,
            line_text: Some("new".into()),
            spec: Some(Spec::WorkTree),
            body: "fix".into(),
        }),
    }
}

fn log_line(event: &LogEvent) -> String {
    format!("{}\n", serde_json::to_string(event).unwrap())
}

#[test]
fn an_invalid_config_still_starts_with_the_default_keys_and_a_warning() {
    let fixture = Fixture::new("bad-config");
    std::fs::write(fixture.home.join("config/config.toml"), "[keys\nsend = ").unwrap();
    let app = fixture.started();
    assert_eq!(app.keymap.warnings.len(), 1);
    assert!(screen_of(&app).contains("not valid TOML"));
    assert_eq!(app.keymap.label(Action::Quit), "q");
}

#[test]
fn not_a_repository_is_a_message_screen_that_says_how_to_leave() {
    let fixture = Fixture::new("not-a-repo");
    fixture.repo_ok.set(false);
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    assert_eq!(app.screen, Screen::Message("not a git repository".into()));
    let screen = screen_of(&app);
    assert!(screen.contains("not a git repository"));
    assert!(screen.contains("R reload, q quit"));
}

#[test]
fn a_store_that_cannot_be_read_is_a_message_with_its_path() {
    let fixture = Fixture::new("store-io");
    std::fs::create_dir_all(fixture.dir().join("review.jsonl")).unwrap();
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    let Screen::Message(message) = &app.screen else {
        panic!("{:?}", app.screen);
    };
    assert!(message.contains("review.jsonl"), "{message}");
}

#[test]
fn git_missing_and_a_failed_diff_are_messages_with_the_reason() {
    let fixture = Fixture::new("git-errors");
    *fixture.diff_error.borrow_mut() = Some(GitError::NotInstalled);
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    assert_eq!(app.screen, Screen::Message("git is not installed".into()));
    *fixture.diff_error.borrow_mut() = Some(GitError::Failed {
        args: "diff".into(),
        stderr: "fatal: bad object".into(),
    });
    fixture.with_git(|git| app.load(git));
    let Screen::Message(message) = &app.screen else {
        panic!("{:?}", app.screen);
    };
    assert!(message.contains("fatal: bad object"));
}

#[test]
fn a_missing_base_falls_back_to_the_working_tree_and_says_so() {
    let fixture = Fixture::new("no-base");
    let spec = Spec::Branch {
        base: "nope".into(),
    };
    save(&fixture.dir(), &fixture.root, |meta| meta.spec = Some(spec)).unwrap();
    fixture.merge_base_missing.set(true);
    let app = fixture.started();
    assert_eq!(app.diff.as_ref().unwrap().spec, Spec::WorkTree);
    assert!(screen_of(&app).contains("base nope not found"));
}

#[test]
fn the_pane_is_recorded_in_meta_and_a_failed_save_is_a_warning() {
    let fixture = Fixture::new("save-pane");
    let app = fixture.started();
    let (meta, _) = crate::meta::load(&fixture.dir());
    assert_eq!(meta.review_pane, PaneId::parse("w1:p9"));
    assert!(app.warnings.is_empty());

    let blocked = Fixture::new("save-pane-blocked");
    std::fs::create_dir_all(blocked.dir()).unwrap();
    std::fs::set_permissions(blocked.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let app = blocked.started();
    assert!(screen_of(&app).contains("could not save meta.json"));
}

#[test]
fn the_status_line_says_no_agent_until_a_target_is_known() {
    let fixture = Fixture::new("status");
    let app = fixture.started();
    assert!(screen_of(&app).contains(" WORKING TREE  \u{2192} no agent  "));
    fixture.agents(&[("w1:p2", "term_1", "idle")]);
    let app = fixture.started();
    assert!(screen_of(&app).contains(" WORKING TREE  \u{2192} claude w1:p2  "));
    // What was found is kept for the `send` action.
    let saved = crate::meta::load(&fixture.dir()).0.target.unwrap();
    assert_eq!(saved.pane.as_str(), "w1:p2");
}

/// The status line of the pane drawn at `width`: its text and its cells.
fn status_row(app: &mut App, width: u16) -> (String, Vec<ratatui::buffer::Cell>) {
    let (rows, buffer) = drawn(app, width);
    let cells = buffer.content.chunks(usize::from(width)).last().unwrap();
    (rows.last().unwrap().clone(), cells.to_vec())
}

/// The cells of the status line that draw `text`.
fn cells_of<'a>(
    (row, cells): &'a (String, Vec<ratatui::buffer::Cell>),
    text: &str,
) -> &'a [ratatui::buffer::Cell] {
    let at = row
        .find(text)
        .unwrap_or_else(|| panic!("no '{text}' in '{row}'"));
    let start = row[..at].chars().count();
    &cells[start..start + text.chars().count()]
}

#[test]
fn the_spec_is_a_chip_in_upper_case_on_the_accent_colour() {
    let fixture = Fixture::new("chip-spec");
    let mut app = fixture.started();
    let theme = app.theme;
    let line = status_row(&mut app, 80);
    assert!(line.0.starts_with(" WORKING TREE  "), "{}", line.0);
    for cell in cells_of(&line, " WORKING TREE ") {
        assert_eq!((cell.fg, cell.bg), (theme.base, theme.accent));
    }
    // The cell after the chip's padding is the bar again.
    assert_eq!(line.1[14].bg, theme.header);
    fixture.with_git(|git| app.handle(Action::SwitchSpec, git));
    let line = status_row(&mut app, 80);
    assert!(line.0.starts_with(" VS ORIGIN/HEAD  "), "{}", line.0);
    for cell in cells_of(&line, " VS ORIGIN/HEAD ") {
        assert_eq!((cell.fg, cell.bg), (theme.base, theme.accent));
    }
}

#[test]
fn the_agent_is_in_the_text_colour_and_no_agent_is_in_the_removed_colour() {
    let fixture = Fixture::new("chip-agent");
    let mut app = fixture.started();
    let theme = app.theme;
    let line = status_row(&mut app, 80);
    for cell in cells_of(&line, "\u{2192} no agent") {
        assert_eq!((cell.fg, cell.bg), (theme.removed, theme.header));
    }
    fixture.agents(&[("w1:p2", "term_1", "idle")]);
    let mut app = fixture.started();
    let line = status_row(&mut app, 80);
    for cell in cells_of(&line, "\u{2192} claude w1:p2") {
        assert_eq!((cell.fg, cell.bg), (theme.text, theme.header));
    }
}

#[test]
fn the_unsent_chip_is_on_the_warning_colour_and_absent_at_zero() {
    let fixture = Fixture::new("chip-unsent");
    let mut app = fixture.started();
    assert!(!status_row(&mut app, 80).0.contains("unsent"));
    std::fs::write(
        fixture.dir().join("review.jsonl"),
        log_line(&add_event("u1")),
    )
    .unwrap();
    let mut app = fixture.started();
    let theme = app.theme;
    let line = status_row(&mut app, 80);
    assert!(line.0.contains("no agent   1 unsent  "), "{}", line.0);
    for cell in cells_of(&line, " 1 unsent ") {
        assert_eq!((cell.fg, cell.bg), (theme.base, theme.warning));
    }
}

#[test]
fn the_keys_are_against_the_right_edge_with_bold_accent_keys_and_subtle_labels() {
    let fixture = Fixture::new("footer-keys");
    let mut app = fixture.started();
    let theme = app.theme;
    let line = status_row(&mut app, 84);
    assert!(
        line.0
            .ends_with("  S send  x resolve  R reload  ? help  q quit"),
        "{}",
        line.0
    );
    assert!(!line.0.contains("panel"));
    // The sidebar key comes first, so it is the first to drop.
    assert!(line.0.contains("  f sidebar  S send"), "{}", line.0);
    assert_eq!(cells_of(&line, "f ")[0].fg, theme.accent);
    let narrow = status_row(&mut app, 80).0;
    assert!(
        narrow.contains("   S send") && !narrow.contains("sidebar"),
        "{narrow}"
    );
    for key in ["S", "x", "R", "?", "q"] {
        let what = format!("{key} ");
        let cell = &cells_of(&line, &what)[0];
        assert_eq!(cell.fg, theme.accent, "{key}");
        assert!(cell.modifier.contains(Modifier::BOLD), "{key}");
    }
    for label in [" send", " resolve", " reload", " help", " quit"] {
        for cell in cells_of(&line, label) {
            assert_eq!(cell.fg, theme.subtle, "{label}");
            assert!(!cell.modifier.contains(Modifier::BOLD), "{label}");
        }
    }
}

#[test]
fn keys_drop_off_from_the_left_when_the_line_is_narrow_and_help_and_quit_go_last() {
    let fixture = Fixture::new("footer-narrow");
    fixture.agents(&[("w1:p2", "term_1", "idle")]);
    std::fs::create_dir_all(fixture.dir()).unwrap();
    std::fs::write(
        fixture.dir().join("review.jsonl"),
        log_line(&add_event("u1")),
    )
    .unwrap();
    let mut app = fixture.started();
    // The state is 41 cells with the unsent chip, so the five keys do not fit in 80.
    let line = status_row(&mut app, 80).0;
    assert!(
        line.ends_with(" 1 unsent     x resolve  R reload  ? help  q quit"),
        "{line}"
    );
    assert!(!line.contains("send"), "{line}");
    let line = status_row(&mut app, 60).0;
    assert!(line.ends_with(" 1 unsent      ? help  q quit"), "{line}");
    assert!(!line.contains("reload"), "{line}");
    let line = status_row(&mut app, 49).0;
    assert!(line.ends_with(" 1 unsent   q quit"), "{line}");
    assert!(!line.contains("help"), "{line}");
    // With no room beside the state no key is drawn.
    let line = status_row(&mut app, 40).0;
    assert!(
        line.starts_with(" WORKING TREE  \u{2192} claude w1:p2   1 unsent"),
        "{line}"
    );
    assert!(!line.contains("quit"), "{line}");
}

#[test]
fn a_notice_and_a_failure_take_the_place_of_the_state_and_the_keys_stay() {
    let fixture = Fixture::new("status-tone");
    let mut app = fixture.started();
    let theme = app.theme;
    app.notice("deleted u2");
    let line = status_row(&mut app, 80);
    assert!(line.0.starts_with(" deleted u2  "), "{}", line.0);
    assert!(
        line.0
            .ends_with("S send  x resolve  R reload  ? help  q quit")
    );
    assert!(!line.0.contains("WORKING TREE"));
    for cell in cells_of(&line, "deleted u2") {
        assert_eq!((cell.fg, cell.bg), (theme.warning, theme.header));
    }
    app.fail("review is busy, press again");
    let line = status_row(&mut app, 80);
    assert!(
        line.0
            .ends_with("S send  x resolve  R reload  ? help  q quit")
    );
    for cell in cells_of(&line, "review is busy, press again") {
        assert_eq!((cell.fg, cell.bg), (theme.removed, theme.header));
    }
}

#[test]
fn each_message_says_whether_it_is_a_notice_or_a_failure() {
    let fixture = Fixture::new("status-kinds");
    let mut app = fixture.started();
    fixture.with_git(|git| app.handle(Action::Resolve, git));
    assert_eq!(app.status, Some((Tone::Notice, "no thread here".into())));
    *fixture.diff_error.borrow_mut() = Some(GitError::Failed {
        args: "diff".into(),
        stderr: "fatal: index locked".into(),
    });
    fixture.with_git(|git| app.handle(Action::Reload, git));
    assert_eq!(app.status.as_ref().unwrap().0, Tone::Failure);
    fixture.with_git(|git| app.handle(Action::Send, git));
    assert_eq!(app.status, Some((Tone::Notice, "sending".into())));
}

#[test]
fn the_status_line_is_a_bar_in_the_header_colour_with_nothing_reversed() {
    let fixture = Fixture::new("status-bar");
    let mut app = fixture.started();
    let theme = app.theme;
    for width in [40, 80, 130] {
        let (row, cells) = status_row(&mut app, width);
        for (cell, symbol) in cells.iter().zip(row.chars()) {
            assert!(!cell.modifier.contains(Modifier::REVERSED), "{row}");
            if symbol == ' ' && cell.bg != theme.accent {
                assert_eq!(cell.bg, theme.header, "{row}");
            }
        }
        assert_eq!(cells.first().unwrap().bg, theme.accent);
        assert_eq!(cells.last().unwrap().bg, theme.header);
    }
}

#[test]
fn a_message_is_drawn_without_control_characters() {
    let fixture = Fixture::new("sanitize");
    *fixture.diff_error.borrow_mut() = Some(GitError::Failed {
        args: String::new(),
        stderr: "bad \u{1b}[31mname\u{9b}".into(),
    });
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    let screen = screen_of(&app);
    assert!(screen.contains("bad [31mname"));
    assert!(!screen.chars().any(|c| c != '\n' && c.is_control()));
    assert_eq!(sanitize_terminal_text("a\tb\u{7f}"), "a    b");
}

#[test]
fn every_control_range_is_dropped_and_a_newline_is_kept() {
    assert_eq!(
        sanitize_terminal_text("\u{0}\u{8}\n\u{b}\u{c}\u{e}\u{1f}\u{7f}\u{9f}x"),
        "\nx"
    );
}

#[test]
fn a_termination_signal_ends_the_loop_before_anything_is_read() {
    let fixture = Fixture::new("terminated");
    let mut app = fixture.started();
    let exit = fixture.with_git(|git| {
        run_loop(
            &mut app,
            &mut terminal(),
            git,
            |_| panic!("polled after the signal"),
            || true,
        )
    });
    assert_eq!(exit, Exit::Terminated);
}

#[test]
fn a_failed_poll_ends_the_loop() {
    let fixture = Fixture::new("poll-io");
    let mut app = fixture.started();
    let exit = fixture.with_git(|git| {
        run_loop(
            &mut app,
            &mut terminal(),
            git,
            |_| Err(io::Error::other("tty gone")),
            || false,
        )
    });
    assert_eq!(exit, Exit::Io);
}

#[test]
fn a_key_with_no_binding_is_ignored_and_quit_ends_the_loop() {
    let fixture = Fixture::new("ignored");
    let mut app = fixture.started();
    app.warnings.push(Warning::Config("kept".into()));
    let exit = drive(
        &fixture,
        &mut app,
        vec![Some(key('z')), Some(Event::FocusLost)],
        |_| {},
    );
    assert_eq!(exit, Exit::Terminated);
    // An ignored key is not an action, so it leaves the warning line alone.
    assert_eq!(app.warnings, [Warning::Config("kept".into())]);
    let exit = drive(
        &fixture,
        &mut app,
        vec![Some(key('z')), Some(key('q'))],
        |_| {},
    );
    assert_eq!(exit, Exit::Quit);
}

#[test]
fn a_change_to_the_log_is_read_and_the_diff_is_reloaded() {
    let fixture = Fixture::new("store-change");
    let mut app = fixture.started();
    assert_eq!((app.review.threads.len(), fixture.diffs.get()), (0, 1));
    let path = fixture.dir().join("review.jsonl");
    let line = log_line(&add_event("u1"));
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            std::fs::write(&path, &line).unwrap();
        }
    });
    assert_eq!(app.review.threads.len(), 1);
    assert_eq!(fixture.diffs.get(), 2);
}

#[test]
fn a_thread_in_the_log_is_drawn_as_a_card_wrapped_to_the_panes_width() {
    let fixture = Fixture::new("cards");
    std::fs::create_dir_all(fixture.dir()).unwrap();
    let mut event = add_event("u1");
    if let Kind::Add(add) = &mut event.kind {
        add.body = "word ".repeat(20);
    }
    std::fs::write(fixture.dir().join("review.jsonl"), log_line(&event)).unwrap();
    let mut app = fixture.started();
    drive(&fixture, &mut app, vec![None], |_| {});
    // The file header, the hunk and two lines, and a box of two borders, an empty row and two
    // body rows.
    assert_eq!(app.view.stream.len(), 9);
    assert!(screen_of(&app).contains("● Your note"));
    // A narrower pane wraps the body into more rows, and the loop lays the stream out again.
    let mut git = |args: &[String]| fixture.git(args);
    let mut narrow = Terminal::new(TestBackend::new(30, 8)).unwrap();
    let polls = Cell::new(0);
    run_loop(
        &mut app,
        &mut narrow,
        &mut git,
        |_| {
            polls.set(polls.get() + 1);
            Ok(None)
        },
        || polls.get() >= 1,
    );
    // Five words fit a row of the 28 column box, so the body is four rows under the empty one.
    assert_eq!(app.view.stream.len(), 11);
}

/// A test terminal whose size the test changes while the loop runs, with no event to say so.
/// It draws into a backend big enough for any size it reports.
struct Resizing {
    inner: TestBackend,
    size: Rc<Cell<ratatui::layout::Size>>,
}

impl Backend for Resizing {
    type Error = std::convert::Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        self.inner.draw(content)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<ratatui::layout::Position>>(
        &mut self,
        position: P,
    ) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<ratatui::layout::Size, Self::Error> {
        Ok(self.size.get())
    }

    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

/// How many frames `run_loop` draws on `terminal` while it polls `events`, with `before` run
/// ahead of each poll.
fn frames<B: Backend>(
    fixture: &Fixture,
    app: &mut App,
    terminal: &mut Terminal<B>,
    events: Vec<Option<Event>>,
    mut before: impl FnMut(usize),
) -> usize {
    // Every file is highlighted first, so no frame counted is one that highlighting draws.
    fixture.with_git(|git| {
        let (diff, root) = (app.diff.as_ref().unwrap(), app.root.as_ref().unwrap());
        for file in &diff.files {
            app.syntax.ensure(root.path(), &diff.rev, file, git);
        }
    });
    let count = events.len();
    let mut events = events.into_iter();
    let polls = Cell::new(0);
    let poll = |_| {
        before(polls.get());
        polls.set(polls.get() + 1);
        Ok(events.next().flatten())
    };
    fixture.with_git(|git| run_loop(app, terminal, git, poll, || polls.get() >= count));
    // A frame's count is the number drawn before it.
    terminal.draw(|_| {}).unwrap().count
}

#[test]
fn idle_ticks_draw_one_frame() {
    let fixture = Fixture::new("idle");
    let mut app = fixture.started();
    let drawn = frames(&fixture, &mut app, &mut terminal(), vec![None; 10], |_| {});
    assert_eq!(drawn, 1);
}

#[test]
fn a_key_between_idle_ticks_draws_one_more_frame() {
    let fixture = Fixture::new("idle-key");
    let mut app = fixture.started();
    let events = vec![None, None, Some(key('j')), None, None];
    let drawn = frames(&fixture, &mut app, &mut terminal(), events, |_| {});
    assert_eq!(drawn, 2);
}

#[test]
fn a_write_to_the_log_from_outside_draws_one_more_frame() {
    let fixture = Fixture::new("idle-log");
    let mut app = fixture.started();
    let path = fixture.dir().join("review.jsonl");
    let line = log_line(&add_event("u1"));
    let drawn = frames(
        &fixture,
        &mut app,
        &mut terminal(),
        vec![None; 10],
        |tick| {
            if tick == 3 {
                std::fs::write(&path, &line).unwrap();
            }
        },
    );
    assert_eq!(app.review.threads.len(), 1);
    assert_eq!(drawn, 2);
}

#[test]
fn a_new_terminal_size_draws_one_more_frame() {
    let fixture = Fixture::new("idle-resize");
    let mut app = fixture.started();
    let size = Rc::new(Cell::new(ratatui::layout::Size::new(80, 12)));
    let backend = Resizing {
        inner: TestBackend::new(200, 60),
        size: Rc::clone(&size),
    };
    let mut terminal = Terminal::new(backend).unwrap();
    let drawn = frames(&fixture, &mut app, &mut terminal, vec![None; 10], |tick| {
        if tick == 3 {
            size.set(ratatui::layout::Size::new(100, 20));
        }
    });
    assert_eq!(drawn, 2);
}

#[test]
fn a_log_that_cannot_be_read_is_a_warning_and_the_next_tick_tries_again() {
    let fixture = Fixture::new("store-retry");
    let mut app = fixture.started();
    let path = fixture.dir().join("review.jsonl");
    let line = log_line(&add_event("u1"));
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::remove_dir(&path).unwrap();
            std::fs::write(&path, &line).unwrap();
        }
    });
    assert_eq!(app.review.threads.len(), 1);
    assert!(
        app.warnings
            .iter()
            .any(|w| w.to_string().contains("review.jsonl"))
    );
    assert_eq!(app.screen, Screen::Review);
}

#[test]
fn the_message_screen_responds_to_reload_and_quit() {
    let fixture = Fixture::new("message-keys");
    fixture.repo_ok.set(false);
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    assert!(matches!(app.screen, Screen::Message(_)));
    // Still not a repository: reload keeps the message.
    drive(&fixture, &mut app, vec![Some(key('R'))], |_| {});
    assert!(matches!(app.screen, Screen::Message(_)));
    // Fixed: reload shows the review.
    drive(&fixture, &mut app, vec![Some(key('R'))], |_| {
        fixture.repo_ok.set(true);
    });
    assert_eq!(app.screen, Screen::Review);
    assert_eq!(app.diff.as_ref().unwrap().files[0].change, Change::Modified);
    // And quit leaves from the message screen too.
    fixture.repo_ok.set(false);
    let mut broken = fixture.app();
    fixture.with_git(|git| broken.load(git));
    assert_eq!(
        drive(&fixture, &mut broken, vec![Some(key('q'))], |_| {}),
        Exit::Quit
    );
}

#[test]
fn a_failed_reload_keeps_the_diff_and_shows_the_error() {
    let fixture = Fixture::new("reload-fails");
    let mut app = fixture.started();
    *fixture.diff_error.borrow_mut() = Some(GitError::Failed {
        args: "diff".into(),
        stderr: "fatal: index locked".into(),
    });
    drive(&fixture, &mut app, vec![Some(key('R'))], |_| {});
    assert_eq!(app.screen, Screen::Review);
    assert_eq!(app.diff.as_ref().unwrap().files.len(), 1);
    assert!(screen_of(&app).contains("fatal: index locked"));
    *fixture.diff_error.borrow_mut() = None;
    drive(&fixture, &mut app, vec![Some(key('R'))], |_| {});
    assert_eq!(app.status, None);
}

#[test]
fn regaining_focus_reloads_the_diff() {
    let fixture = Fixture::new("focus");
    let mut app = fixture.started();
    drive(&fixture, &mut app, vec![Some(Event::FocusGained)], |_| {});
    assert_eq!(fixture.diffs.get(), 2);
}

#[test]
fn a_panic_in_the_loop_restores_the_terminal_once() {
    let fixture = Fixture::new("panic");
    let mut app = fixture.started();
    let restored = Cell::new(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _guard = Guard(Some(|| restored.set(restored.get() + 1)));
        fixture
            .with_git(|git| run_loop(&mut app, &mut terminal(), git, |_| panic!("boom"), || false))
    }));
    assert!(result.is_err());
    assert_eq!(restored.get(), 1);
}

static HOOK_RAN: AtomicBool = AtomicBool::new(false);

fn mark_restored() {
    HOOK_RAN.store(true, Ordering::SeqCst);
}

#[test]
fn the_panic_hook_restores_the_terminal_before_the_message() {
    restoring_panic_hook(mark_restored);
    let result = catch_unwind(|| panic!("restore first"));
    let _ = std::panic::take_hook();
    assert!(result.is_err());
    assert!(HOOK_RAN.load(Ordering::SeqCst));
}

const TWO_FILES: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 a1
-a2
+A2
 a3
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-b
+B
";

fn started_with(fixture: &Fixture, patch: &str) -> App {
    *fixture.patch.borrow_mut() = patch.as_bytes().to_vec();
    fixture.started()
}

#[test]
fn keys_move_the_cursor_through_the_loop_and_the_screen_follows() {
    let fixture = Fixture::new("navigate");
    let mut app = started_with(&fixture, TWO_FILES);
    let events = vec![
        Some(key('j')),
        Some(key('j')),
        Some(key(']')),
        Some(key('k')),
    ];
    drive(&fixture, &mut app, events, |_| {});
    // Down twice to row 2, the next hunk header is row 7, and up once.
    assert_eq!(app.view.cursor, 6);
    let screen = screen_of(&app);
    assert!(screen.contains("M a.rs"));
    assert!(screen.contains("@@ -1 +1 @@"));
}

#[test]
fn the_mouse_reaches_the_view_through_the_loop() {
    let fixture = Fixture::new("mouse");
    let mut app = started_with(&fixture, TWO_FILES);
    let click = Event::Mouse(MouseEvent {
        kind: event::MouseEventKind::Down(event::MouseButton::Left),
        column: 40,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    drive(&fixture, &mut app, vec![Some(click)], |_| {});
    assert_eq!(app.view.cursor, 3);
}

#[test]
fn rebinding_a_key_changes_the_footer_and_the_help_overlay() {
    let fixture = Fixture::new("rebind");
    let app = started_with(&fixture, TWO_FILES);
    assert!(screen_of(&app).contains("R reload"));
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nreload = \"r\"\nhelp = \"F1\"\n",
    )
    .unwrap();
    let mut tall = App::new(fixture.env.clone(), None);
    fixture.with_git(|git| tall.load(git));
    let screen = screen_of(&tall);
    assert!(screen.contains("r reload"), "{screen}");
    assert!(screen.contains("f1 help"), "{screen}");
    assert!(screen.contains("no key left for reply"), "{screen}");
    // The overlay opens on its key, is drawn from the same map, and closes on any key.
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    let mut git = |args: &[String]| fixture.git(args);
    let f1 = Event::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    let events = [Some(f1), None];
    let mut events = events.into_iter();
    let polls = Cell::new(0);
    run_loop(
        &mut tall,
        &mut terminal,
        &mut git,
        |_| {
            polls.set(polls.get() + 1);
            Ok(events.next().flatten())
        },
        || polls.get() >= 1,
    );
    assert!(tall.view.help);
    terminal.draw(|frame| render(frame, &tall)).unwrap();
    let buffer = terminal.backend().buffer();
    let text = buffer
        .content
        .chunks(80)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("r                reload the diff"), "{text}");
    assert!(
        text.contains("-                reply to the thread"),
        "{text}"
    );
    drive(&fixture, &mut tall, vec![Some(key('q'))], |_| {});
    assert!(!tall.view.help);
    assert!(
        !tall.quit,
        "the key that closes the overlay does nothing else"
    );
}

#[test]
fn a_file_name_with_an_escape_byte_reaches_the_backend_without_it() {
    let fixture = Fixture::new("escape-name");
    let patch = "diff --git \"a/e\\033[2Jvil.rs\" \"b/e\\033[2Jvil.rs\"\n--- \"a/e\\033[2Jvil.rs\"\n+++ \"b/e\\033[2Jvil.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
    let app = started_with(&fixture, patch);
    assert!(
        app.diff.as_ref().unwrap().files[0]
            .path
            .as_str()
            .contains('\u{1b}')
    );
    let screen = screen_of(&app);
    assert!(screen.contains("e[2Jvil.rs"));
    assert!(!screen.chars().any(|c| c != '\n' && c.is_control()));
}

#[test]
fn switching_the_spec_shows_the_other_diff_and_remembers_it() {
    let fixture = Fixture::new("switch-spec");
    let mut app = started_with(&fixture, TWO_FILES);
    assert!(screen_of(&app).contains("WORKING TREE"));
    drive(&fixture, &mut app, vec![Some(key('b'))], |_| {});
    assert_eq!(
        app.diff.as_ref().unwrap().spec,
        Spec::Branch {
            base: "origin/HEAD".into()
        }
    );
    assert!(screen_of(&app).contains("VS ORIGIN/HEAD"));
    let (meta, _) = crate::meta::load(&fixture.dir());
    assert_eq!(
        meta.spec,
        Some(Spec::Branch {
            base: "origin/HEAD".into()
        })
    );
    assert_eq!(fixture.diffs.get(), 2);
    drive(&fixture, &mut app, vec![Some(key('b'))], |_| {});
    assert_eq!(app.diff.as_ref().unwrap().spec, Spec::WorkTree);
    let (meta, _) = crate::meta::load(&fixture.dir());
    assert_eq!(meta.spec, Some(Spec::WorkTree));
    // The base is remembered for the next switch.
    assert_eq!(meta.base.as_deref(), Some("origin/HEAD"));
}

#[test]
fn switching_to_a_branch_with_no_base_says_so_and_keeps_the_diff() {
    let fixture = Fixture::new("switch-no-base");
    let mut app = started_with(&fixture, TWO_FILES);
    fixture.no_refs.set(true);
    drive(&fixture, &mut app, vec![Some(key('b'))], |_| {});
    assert_eq!(app.diff.as_ref().unwrap().spec, Spec::WorkTree);
    assert!(screen_of(&app).contains("no base branch found, tried origin/HEAD, main, master"));
}

#[test]
fn a_reload_keeps_the_cursor_on_the_same_line() {
    let fixture = Fixture::new("keep-cursor");
    let mut app = started_with(&fixture, TWO_FILES);
    drive(
        &fixture,
        &mut app,
        vec![Some(key('j')), Some(key('j')), Some(key('j'))],
        |_| {},
    );
    assert_eq!(app.view.cursor, 3);
    drive(&fixture, &mut app, vec![Some(key('R'))], |_| {});
    assert_eq!(app.view.cursor, 3);
    assert_eq!(fixture.diffs.get(), 2);
}

#[test]
fn an_empty_diff_is_a_message_and_the_keys_still_work() {
    let fixture = Fixture::new("empty");
    let mut app = started_with(&fixture, "");
    assert!(screen_of(&app).contains("No changes in the working tree."));
    let events = vec![Some(key('j')), Some(key(']')), Some(key('q'))];
    assert_eq!(drive(&fixture, &mut app, events, |_| {}), Exit::Quit);
}

// The rows of `PATCH`, which a test starts on: the a.rs header 0, the hunk 1, the removed
// `old` 2 and the added `new` 3. `TWO_FILES` has a.rs at 0 to 5 and b.rs at 6 to 9.

fn opened(fixture: &Fixture, patch: &str) -> App {
    let mut app = started_with(fixture, patch);
    app.resize(Rect::new(0, 0, 80, 12));
    app
}

fn press(fixture: &Fixture, app: &mut App, events: impl IntoIterator<Item = Event>) {
    for event in events {
        if let Event::Key(key) = event {
            fixture.with_git(|git| app.key(key, git));
            app.run_pending(|_| {});
        }
    }
}

fn chars(text: &str) -> Vec<Event> {
    text.chars().map(key).collect()
}

fn ctrl_s() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
}

fn esc() -> Event {
    Event::Key(KeyEvent::from(KeyCode::Esc))
}

fn thread_event(id: &str, by: Author, body: &str) -> LogEvent {
    let mut event = add_event(id);
    event.by = by;
    if let Kind::Add(add) = &mut event.kind {
        add.body = body.into();
    }
    event
}

fn agent() -> Author {
    Author::Agent(Some("claude".into()))
}

fn write_log(fixture: &Fixture, events: &[LogEvent]) {
    std::fs::create_dir_all(fixture.dir()).unwrap();
    let lines = events.iter().map(log_line).collect::<String>();
    std::fs::write(fixture.dir().join("review.jsonl"), lines).unwrap();
}

fn line_anchor(side: Side, line: u32, text: &str) -> Anchor {
    Anchor {
        path: RelPath::parse("a.rs").unwrap(),
        old_path: None,
        target: AnchorTarget::Line {
            side,
            line,
            text: text.into(),
        },
        spec: Spec::WorkTree,
    }
}

/// `PATCH` as text, so a test can start on it.
fn patch_text() -> &'static str {
    std::str::from_utf8(PATCH).unwrap()
}

#[test]
fn a_comment_on_a_line_is_written_and_drawn_as_a_card_under_it() {
    let fixture = Fixture::new("comment-line");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('j'), key('j'), key('c')]);
    assert!(screen_of(&app).contains("╭ Draft note - a.rs L1 ─"));
    press(&fixture, &mut app, chars("fix this"));
    press(&fixture, &mut app, [ctrl_s()]);
    assert!(app.compose.is_none());
    assert_eq!(app.review.threads.len(), 1);
    let thread = &app.review.threads[0];
    assert_eq!(thread.anchor, line_anchor(Side::Old, 1, "old"));
    assert_eq!(thread.root.body, "fix this");
    assert!(thread.unsent);
    // The card is under the removed row, and the cursor is on it.
    assert_eq!(app.view.cursor, 3);
    assert_eq!(app.view.focused(), Some(0));
    let screen = screen_of(&app);
    assert!(
        screen.contains("● Your note · now · a.rs L1 [unsent]") && screen.contains("fix this"),
        "{screen}"
    );
}

#[test]
fn a_line_a_range_and_a_file_are_present_after_the_pane_is_closed_and_opened_again() {
    let fixture = Fixture::new("comment-kinds");
    let mut app = opened(&fixture, TWO_FILES);
    // Row 2 is a1 and row 5 is a3. The removed row between them has no new line.
    press(&fixture, &mut app, [key('j'), key('j'), key('v')]);
    press(&fixture, &mut app, [key('j'), key('j'), key('j'), key('c')]);
    press(&fixture, &mut app, chars("this block"));
    press(&fixture, &mut app, [ctrl_s()]);
    // The file comment, from its header.
    press(&fixture, &mut app, "k".repeat(20).chars().map(key));
    assert_eq!(app.view.cursor, 0);
    press(&fixture, &mut app, [key('c')]);
    press(&fixture, &mut app, chars("the file"));
    press(&fixture, &mut app, [ctrl_s()]);
    // A line, on the added row of b.rs.
    press(&fixture, &mut app, "j".repeat(40).chars().map(key));
    press(&fixture, &mut app, [key('c')]);
    press(&fixture, &mut app, chars("one line"));
    press(&fixture, &mut app, [ctrl_s()]);
    drop(app);
    let mut reopened = fixture.started();
    reopened.resize(Rect::new(0, 0, 80, 12));
    let threads = &reopened.review.threads;
    assert_eq!(threads.len(), 3);
    assert_eq!(
        threads[0].anchor.target,
        AnchorTarget::Range {
            side: Side::New,
            start: 1,
            end: 3,
            text: "a1".into()
        }
    );
    assert_eq!(threads[1].anchor.target, AnchorTarget::File);
    assert_eq!(threads[2].anchor.path.as_str(), "b.rs");
    let bodies = threads
        .iter()
        .map(|thread| thread.root.body.as_str())
        .collect::<Vec<_>>();
    assert_eq!(bodies, ["this block", "the file", "one line"]);
    let screen = screen_of(&reopened);
    assert!(screen.contains("this block"), "{screen}");
}

#[test]
fn a_hunk_header_cannot_be_commented_and_says_what_can() {
    let fixture = Fixture::new("comment-hunk");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('j'), key('c')]);
    assert!(app.compose.is_none());
    assert!(screen_of(&app).contains("comment on a line or a file header"));
}

#[test]
fn cancelling_the_editor_writes_nothing() {
    let fixture = Fixture::new("cancel");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('c')]);
    press(&fixture, &mut app, chars("never mind"));
    press(&fixture, &mut app, [esc()]);
    assert!(app.compose.is_none());
    assert!(!fixture.dir().join("review.jsonl").exists());
    assert!(app.review.threads.is_empty());
}

/// The pane drawn at `width` by 12, as the rows of the screen and the buffer behind them.
fn drawn(app: &mut App, width: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
    drawn_tall(app, width, 12)
}

/// The rows without the sidebar's 20 columns, so its filter box is not taken for a card.
fn stream_columns(rows: &[String]) -> Vec<String> {
    rows.iter()
        .map(|row| row.chars().skip(20).collect())
        .collect()
}

fn drawn_tall(app: &mut App, width: u16, height: u16) -> (Vec<String>, ratatui::buffer::Buffer) {
    app.resize(Rect::new(0, 0, width, height));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows = buffer
        .content
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect();
    (rows, buffer)
}

#[test]
fn the_editor_is_a_rounded_box_where_a_note_goes_under_the_marked_row() {
    let fixture = Fixture::new("editor-box");
    let mut app = opened(&fixture, patch_text());
    // Unified: the stream is columns 20..80, the box is four cells in, and row 3 is `+new`.
    press(&fixture, &mut app, [key('j'), key('j'), key('j'), key('c')]);
    let (rows, buffer) = drawn(&mut app, 80);
    let stream = |row: &str| row.chars().skip(20).collect::<String>();
    assert!(
        stream(&rows[4]).starts_with("    ╭ Draft note - a.rs R1 ─"),
        "{}",
        rows[4]
    );
    assert!(rows[4].ends_with("─╮"), "{}", rows[4]);
    // An empty row, then the text a cell clear of the side.
    assert_eq!(stream(&rows[5]), format!("    │{}│", " ".repeat(54)));
    assert!(
        stream(&rows[6]).starts_with("    │ Write a note…"),
        "{}",
        rows[6]
    );
    assert!(rows[6].ends_with('│'), "{}", rows[6]);
    assert!(stream(&rows[7]).starts_with("    ╰─"), "{}", rows[7]);
    assert!(rows[7].ends_with(" ^S save  Esc cancel ╯"), "{}", rows[7]);
    assert_eq!(buffer[(24, 4)].fg, app.theme.warning);
    // The commented row keeps a bar and a tint while the editor is open.
    assert_eq!(buffer[(20, 3)].symbol(), "▌");
    assert_eq!(buffer[(20, 3)].fg, app.theme.warning);
    assert_eq!(buffer[(60, 3)].bg, app.theme.selection);
    assert_ne!(buffer[(60, 2)].bg, app.theme.selection);
    // Side by side: the sidebar is 32 columns, and `-old` and `+new` share row 2. The comment
    // is on the new side, so the box is the new half, which starts 49 cells into the stream.
    let (rows, buffer) = drawn(&mut app, 130);
    let half = |row: &str| row.chars().skip(32 + 49).collect::<String>();
    assert!(
        half(&rows[3]).starts_with("╭ Draft note - a.rs R1 ─"),
        "{}",
        rows[3]
    );
    assert!(rows[3].ends_with("─╮"), "{}", rows[3]);
    assert_eq!(buffer[(32 + 48, 3)].symbol(), " ");
    assert!(rows[6].ends_with(" ^S save  Esc cancel ╯"), "{}", rows[6]);
    assert_eq!(buffer[(100, 2)].bg, app.theme.selection);
    press(&fixture, &mut app, [esc()]);
    let (rows, buffer) = drawn(&mut app, 130);
    assert!(!rows.join("\n").contains("Draft note"));
    assert_ne!(buffer[(100, 2)].bg, app.theme.selection);
}

#[test]
fn the_editor_title_names_a_range_and_a_file_and_the_box_sits_under_the_range() {
    let fixture = Fixture::new("editor-titles");
    let patch =
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,4 @@\n a\n+b\n+c\n d\n";
    let mut app = opened(&fixture, patch);
    // Rows: the header, the hunk, ` a`, `+b`, `+c`, ` d`. The range is selected upwards.
    press(&fixture, &mut app, chars("jjjjvkc"));
    assert_eq!(app.view.cursor, 3);
    let (rows, buffer) = drawn(&mut app, 80);
    assert!(
        rows[5].contains("╭ Draft note - a.rs R2-3 ─"),
        "{}",
        rows[5]
    );
    for y in [3, 4] {
        assert_eq!(buffer[(20, y)].symbol(), "▌", "row {y}");
        assert_eq!(buffer[(60, y)].bg, app.theme.selection, "row {y}");
    }
    press(&fixture, &mut app, [esc()]);
    press(&fixture, &mut app, chars("kkkc"));
    let (rows, buffer) = drawn(&mut app, 80);
    assert!(rows[1].contains("╭ Draft note - a.rs ─"), "{}", rows[1]);
    assert_eq!(buffer[(60, 0)].bg, app.theme.selection);
}

#[test]
fn a_long_draft_stops_growing_at_two_thirds_of_the_stream() {
    let fixture = Fixture::new("editor-cap");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('c')]);
    for _ in 0..9 {
        press(&fixture, &mut app, chars("line"));
        press(
            &fixture,
            &mut app,
            [Event::Key(KeyEvent::from(KeyCode::Enter))],
        );
    }
    // The stream is 10 rows tall, so the box is 6: two borders and the last four lines.
    let (rows, _) = drawn(&mut app, 80);
    let rows = stream_columns(&rows);
    let top = rows.iter().position(|row| row.contains('╭')).unwrap();
    let bottom = rows.iter().position(|row| row.contains('╰')).unwrap();
    assert_eq!(bottom - top, 5, "{}", rows.join("\n"));
}

#[test]
fn a_failed_save_leaves_the_editor_open_with_its_text() {
    let fixture = Fixture::new("save-fails");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('c')]);
    press(&fixture, &mut app, chars("keep me"));
    std::fs::set_permissions(fixture.dir(), std::fs::Permissions::from_mode(0o500)).unwrap();
    press(&fixture, &mut app, [ctrl_s()]);
    let screen = screen_of(&app);
    assert!(screen.contains("review.jsonl"), "{screen}");
    assert!(screen.contains("keep me"), "{screen}");
    assert!(app.compose.is_some());
    // The text goes out once the disk allows it.
    std::fs::set_permissions(fixture.dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
    press(&fixture, &mut app, [ctrl_s()]);
    assert!(app.compose.is_none());
    assert_eq!(app.review.threads[0].root.body, "keep me");
}

#[test]
fn a_busy_review_keeps_the_editor_and_says_to_press_again() {
    let fixture = Fixture::new("busy-editor");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('c')]);
    press(&fixture, &mut app, chars("keep me"));
    let held = crate::store::lock(&fixture.dir()).unwrap();
    press(&fixture, &mut app, [ctrl_s()]);
    assert!(screen_of(&app).contains("review is busy, press again"));
    assert!(app.compose.is_some());
    drop(held);
    press(&fixture, &mut app, [ctrl_s()]);
    assert!(app.compose.is_none());
    assert_eq!(app.review.threads.len(), 1);
}

#[test]
fn a_termination_signal_with_a_draft_writes_one_comment_at_the_captured_anchor() {
    let fixture = Fixture::new("terminated-draft");
    let mut app = opened(&fixture, patch_text());
    // Rows 2 and 3 are `old` and `new`. The comment is on the removed row.
    let events = vec![
        Some(key('j')),
        Some(key('j')),
        Some(key('c')),
        Some(key('h')),
        Some(key('i')),
        // The agent writes while the editor is open, and the pane regains focus. Neither
        // reloads the diff.
        Some(Event::FocusGained),
        None,
    ];
    let patch = fixture.patch.clone();
    let exit = drive(&fixture, &mut app, events, |tick| {
        if tick == 5 {
            let agent_event = thread_event("a1", agent(), "from the agent");
            write_log(&fixture, &[agent_event]);
            *patch.borrow_mut() =
                b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1,2 @@\n+first\n new\n"
                    .to_vec();
        }
    });
    assert_eq!(exit, Exit::Terminated);
    assert_eq!(
        fixture.diffs.get(),
        1,
        "the diff was reloaded under the editor"
    );
    assert!(app.compose.is_none());
    let log = std::fs::read_to_string(fixture.dir().join("review.jsonl")).unwrap();
    let added = log
        .lines()
        .filter_map(|line| serde_json::from_str::<LogEvent>(line).ok())
        .filter_map(|event| match event.kind {
            Kind::Add(add) if event.by == Author::User => Some(add),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].body, "hi");
    assert_eq!(added[0].side, Some(Side::Old));
    assert_eq!(added[0].line, Some(1));
    assert_eq!(added[0].line_text.as_deref(), Some("old"));
}

#[test]
fn a_termination_signal_with_no_text_writes_nothing() {
    let fixture = Fixture::new("terminated-empty");
    let mut app = opened(&fixture, patch_text());
    let events = vec![Some(key('c')), Some(key(' ')), None];
    drive(&fixture, &mut app, events, |_| {});
    assert!(!fixture.dir().join("review.jsonl").exists());
}

/// Two threads on the added row of `PATCH`: the user's `u1`, then the agent's `a1`. The cursor
/// starts on `u1`'s card, which is row 4.
fn with_two_threads(name: &str) -> (Fixture, App) {
    let fixture = Fixture::new(name);
    write_log(
        &fixture,
        &[
            thread_event("u1", Author::User, "fix"),
            thread_event("a1", agent(), "agent note"),
        ],
    );
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('n')]);
    assert_eq!(app.view.cursor, 4);
    (fixture, app)
}

#[test]
fn a_reply_joins_the_thread_under_the_cursor() {
    let (fixture, mut app) = with_two_threads("reply");
    press(&fixture, &mut app, [key('r')]);
    assert!(screen_of(&app).contains("Reply to u1"));
    press(&fixture, &mut app, chars("thanks"));
    press(&fixture, &mut app, [ctrl_s()]);
    let thread = &app.review.threads[0];
    assert_eq!(thread.replies.len(), 1);
    assert_eq!(thread.replies[0].body, "thanks");
    assert_eq!(thread.replies[0].id.as_str(), "u2");
    assert!(screen_of(&app).contains("↳ user: thanks"));
    assert_eq!(app.view.focused(), Some(0));
}

#[test]
fn a_reply_needs_a_thread_under_the_cursor() {
    let fixture = Fixture::new("reply-nothing");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('r')]);
    assert!(app.compose.is_none());
    assert!(screen_of(&app).contains("no thread here to reply to"));
}

#[test]
fn an_edit_opens_the_comment_and_saves_the_new_text() {
    let (fixture, mut app) = with_two_threads("edit");
    press(&fixture, &mut app, [key('e')]);
    let screen = screen_of(&app);
    assert!(
        screen.contains("Edit u1") && screen.contains("fix"),
        "{screen}"
    );
    press(&fixture, &mut app, chars(" it"));
    press(&fixture, &mut app, [ctrl_s()]);
    assert_eq!(app.review.threads[0].root.body, "fix it");
    assert!(app.compose.is_none());
    // The comment was never sent, so it is not marked as edited since.
    assert!(!app.review.threads[0].root.edited_since_sent);
    assert_eq!(app.review.threads.len(), 2);
}

#[test]
fn delete_removes_the_users_comment_and_a_root_takes_its_thread() {
    let (fixture, mut app) = with_two_threads("delete");
    press(&fixture, &mut app, [key('r')]);
    press(&fixture, &mut app, chars("a reply"));
    press(&fixture, &mut app, [ctrl_s()]);
    // On the reply's line, d removes only the reply.
    let reply_row = app.view.cursor + 3;
    press(&fixture, &mut app, chars("jjj"));
    assert_eq!(app.view.cursor, reply_row);
    press(&fixture, &mut app, [key('d')]);
    assert!(screen_of(&app).contains("deleted u2"));
    assert!(app.review.threads[0].replies.is_empty());
    // On the root's line, d removes the thread.
    press(&fixture, &mut app, chars("kkk"));
    press(&fixture, &mut app, [key('d')]);
    assert_eq!(app.review.threads.len(), 1);
    assert_eq!(app.review.threads[0].root.id.as_str(), "a1");
}

#[test]
fn x_resolves_the_thread_and_reopens_it() {
    let (fixture, mut app) = with_two_threads("resolve");
    press(&fixture, &mut app, [key('x')]);
    assert!(!app.review.threads[0].is_open());
    assert!(screen_of(&app).contains("resolved u1"));
    // The card is one line now, and the cursor is still in the thread.
    assert!(screen_of(&app).contains("✓ u1 resolved by user: fix"));
    press(&fixture, &mut app, [key('x')]);
    assert!(app.review.threads[0].is_open());
    assert!(screen_of(&app).contains("reopened u1"));
}

#[test]
fn either_side_may_resolve_but_the_agents_words_cannot_be_edited_or_deleted() {
    let (fixture, mut app) = with_two_threads("rights");
    // u1's card is rows 4 to 5, and a1's starts at 6.
    press(&fixture, &mut app, [key('n')]);
    assert_eq!(app.view.focused(), Some(1));
    let before = app.review.clone();
    for action in ['e', 'd'] {
        press(&fixture, &mut app, [key(action)]);
        assert!(app.compose.is_none());
        let screen = screen_of(&app);
        assert!(screen.contains("a1 is the agent's"), "{screen}");
        assert_eq!(app.review, before);
    }
    press(&fixture, &mut app, [key('x')]);
    assert!(!app.review.threads[1].is_open());
}

#[test]
fn resolving_and_deleting_report_a_busy_review_on_the_status_line() {
    let (fixture, mut app) = with_two_threads("busy-actions");
    let held = crate::store::lock(&fixture.dir()).unwrap();
    for action in ['x', 'd'] {
        press(&fixture, &mut app, [key(action)]);
        assert!(
            screen_of(&app).contains("review is busy, press again"),
            "{action}"
        );
    }
    drop(held);
    assert!(app.review.threads[0].is_open());
    assert_eq!(app.review.threads.len(), 2);
}

#[test]
fn a_selected_range_is_dropped_by_a_second_press_and_by_writing() {
    let fixture = Fixture::new("select");
    let mut app = opened(&fixture, TWO_FILES);
    press(&fixture, &mut app, [key('j'), key('j'), key('v')]);
    assert_eq!(app.view.select.map(|select| select.row), Some(2));
    press(&fixture, &mut app, [key('v')]);
    assert_eq!(app.view.select, None);
    press(&fixture, &mut app, [key('v'), key('c')]);
    assert_eq!(app.view.select, None);
    assert!(app.compose.is_some());
}

fn agent_reply_and_resolve(thread: &str, id: &str, body: &str) -> Vec<LogEvent> {
    let mut reply = thread_event(id, agent(), body);
    if let Kind::Add(add) = &mut reply.kind {
        *add = Add {
            id: CommentId::parse(id).unwrap(),
            parent: CommentId::parse(thread),
            path: None,
            old_path: None,
            side: None,
            line: None,
            end_line: None,
            line_text: None,
            spec: None,
            body: body.into(),
        };
    }
    let resolve = LogEvent {
        at: "2026-10-05T00:00:00Z".into(),
        by: agent(),
        kind: Kind::Resolve {
            id: CommentId::parse(thread).unwrap(),
        },
    };
    vec![reply, resolve]
}

fn seen_events(fixture: &Fixture) -> usize {
    std::fs::read_to_string(fixture.dir().join("review.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("\"kind\":\"seen\""))
        .count()
}

#[test]
fn an_agent_resolve_shows_new_until_the_cursor_reaches_it_and_stays_cleared_after_a_restart() {
    let fixture = Fixture::new("new-marker");
    let mut events = vec![thread_event("u1", Author::User, "fix")];
    events.extend(agent_reply_and_resolve("u1", "a1", "Added with_capacity"));
    write_log(&fixture, &events);
    let mut app = opened(&fixture, patch_text());
    assert!(app.review.threads[0].is_new);
    assert!(screen_of(&app).contains("✓ u1 [new] resolved by agent:claude"));
    // Rows 1 and 2 are not the thread. Row 3 is the line its card hangs under.
    drive(
        &fixture,
        &mut app,
        vec![Some(key('j')), Some(key('j'))],
        |_| {},
    );
    assert!(app.review.threads[0].is_new);
    assert_eq!(seen_events(&fixture), 0);
    drive(&fixture, &mut app, vec![Some(key('j'))], |_| {});
    assert!(!app.review.threads[0].is_new);
    assert_eq!(seen_events(&fixture), 1);
    assert!(!screen_of(&app).contains("[new]"));
    // Moving around the thread, away and back, writes nothing more.
    let wander = [key('j'), key('k'), key('k'), key('j'), key('n')];
    drive(&fixture, &mut app, wander.map(Some).to_vec(), |_| {});
    assert_eq!(seen_events(&fixture), 1);
    // A restart reads the seen event.
    let restarted = opened(&fixture, patch_text());
    assert!(!restarted.review.threads[0].is_new);
    assert!(!screen_of(&restarted).contains("[new]"));
}

#[test]
fn jumping_to_the_card_with_next_thread_reaches_it() {
    let fixture = Fixture::new("new-by-jump");
    let mut events = vec![thread_event("u1", Author::User, "fix")];
    events.extend(agent_reply_and_resolve("u1", "a1", "done"));
    write_log(&fixture, &events);
    let mut app = opened(&fixture, patch_text());
    drive(&fixture, &mut app, vec![Some(key('n'))], |_| {});
    assert_eq!(app.view.cursor, 4);
    assert!(!app.review.threads[0].is_new);
    assert_eq!(seen_events(&fixture), 1);
}

#[test]
fn a_resolve_written_by_another_process_while_the_pane_is_open_shows_new_with_no_key_pressed() {
    let fixture = Fixture::new("new-live");
    write_log(&fixture, &[thread_event("u1", Author::User, "fix")]);
    let mut app = opened(&fixture, patch_text());
    assert!(!screen_of(&app).contains("[new]"));
    let mut events = vec![thread_event("u1", Author::User, "fix")];
    events.extend(agent_reply_and_resolve("u1", "a1", "done"));
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 1 {
            write_log(&fixture, &events);
        }
    });
    assert!(app.review.threads[0].is_new);
    assert!(screen_of(&app).contains("✓ u1 [new] resolved by agent:claude: done"));
    // No key was pressed, so nothing was marked seen.
    assert_eq!(seen_events(&fixture), 0);
}

#[test]
fn a_shorter_log_is_read_again_from_the_start() {
    let fixture = Fixture::new("shorter");
    write_log(
        &fixture,
        &[
            thread_event("u1", Author::User, "first"),
            thread_event("u2", Author::User, "second"),
        ],
    );
    let mut app = opened(&fixture, patch_text());
    assert_eq!(app.review.threads.len(), 2);
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            write_log(&fixture, &[thread_event("u1", Author::User, "first")]);
        }
    });
    assert_eq!(app.review.threads.len(), 1);
    assert_eq!(app.review.threads[0].root.body, "first");
    assert_eq!(fixture.diffs.get(), 2);
    // A log that is gone entirely is an empty review.
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            std::fs::remove_file(fixture.dir().join("review.jsonl")).unwrap();
        }
    });
    assert!(app.review.threads.is_empty());
}

#[test]
fn a_seen_event_that_cannot_be_written_is_a_warning_and_the_next_key_tries_again() {
    let fixture = Fixture::new("seen-busy");
    let mut events = vec![thread_event("u1", Author::User, "fix")];
    events.extend(agent_reply_and_resolve("u1", "a1", "done"));
    write_log(&fixture, &events);
    let mut app = opened(&fixture, patch_text());
    let held = crate::store::lock(&fixture.dir()).unwrap();
    let landing = vec![Some(key('j')), Some(key('j')), Some(key('j'))];
    drive(&fixture, &mut app, landing, |_| {});
    assert!(app.review.threads[0].is_new);
    assert_eq!(seen_events(&fixture), 0);
    assert!(screen_of(&app).contains("review is busy, press again"));
    // The cursor is still on the thread, and the next key writes the event.
    drop(held);
    drive(&fixture, &mut app, vec![Some(key('n'))], |_| {});
    assert_eq!(seen_events(&fixture), 1);
    assert!(!app.review.threads[0].is_new);
}

#[test]
fn editing_the_commented_line_tags_the_thread_outdated_and_committing_moves_it_to_the_block() {
    let fixture = Fixture::new("outdated-then-committed");
    write_log(&fixture, &[thread_event("u1", Author::User, "fix")]);
    let mut app = opened(&fixture, patch_text());
    let screen = screen_of(&app);
    assert!(
        screen.contains("● Your note") && !screen.contains("outdated"),
        "{screen}"
    );
    // The line is edited in an editor, and R reloads.
    *fixture.patch.borrow_mut() =
        b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+edited\n".to_vec();
    press(&fixture, &mut app, [key('R')]);
    let screen = screen_of(&app);
    assert!(screen.contains("a.rs R1 [outdated]"), "{screen}");
    assert!(screen.contains("was: new"), "{screen}");
    // Everything is committed: the diff is empty, and the thread is still listed.
    fixture.patch.borrow_mut().clear();
    press(&fixture, &mut app, [key('R')]);
    let screen = screen_of(&app);
    assert!(screen.contains("Comments not in this diff (1)"), "{screen}");
    assert!(screen.contains("· a.rs R1 "), "{screen}");
    assert!(
        screen.contains("No changes in the working tree."),
        "{screen}"
    );
    // And the user can still act on it from there.
    press(&fixture, &mut app, [key('n'), key('x')]);
    assert!(!app.review.threads[0].is_open());
}

// Send, resend, the picker and the quit prompt.

fn two_user_threads(
    name: &str,
    agents: &[(&'static str, &'static str, &'static str)],
) -> (Fixture, App) {
    let fixture = Fixture::new(name);
    fixture.agents(agents);
    write_log(
        &fixture,
        &[
            thread_event("u1", Author::User, "fix"),
            thread_event("u2", Author::User, "other"),
        ],
    );
    let app = opened(&fixture, patch_text());
    (fixture, app)
}

const IDLE: (&str, &str, &str) = ("w1:p1", "term_1", "idle");

#[test]
fn send_delivers_the_unsent_comments_and_a_second_send_says_nothing() {
    let (fixture, mut app) = two_user_threads("send", &[IDLE]);
    assert!(screen_of(&app).contains("\u{2192} claude w1:p1   2 unsent "));
    press(&fixture, &mut app, [key('S')]);
    let prompts = fixture.prompts();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].starts_with("agent prompt w1:p1 "));
    assert!(prompts[0].contains("[u1]") && prompts[0].contains("[u2]"));
    assert_eq!(app.message(), Some("sent 2 to claude"));
    assert!(screen_of(&app).contains("sent 2 to claude"));
    assert_eq!(app.unsent(), 0);
    press(&fixture, &mut app, [key('S')]);
    assert_eq!(app.message(), Some("nothing to send"));
    assert_eq!(fixture.prompts().len(), 1);
    press(&fixture, &mut app, [key('j')]);
    assert!(!screen_of(&app).contains("unsent"));
}

#[test]
fn the_sending_frame_is_drawn_before_the_herdr_call() {
    let (fixture, mut app) = two_user_threads("sending", &[IDLE]);
    fixture.with_git(|git| app.key(key_event('S'), git));
    let (mut frame, mut prompts_then) = (String::new(), usize::MAX);
    app.run_pending(|app| {
        frame = screen_of(app);
        prompts_then = fixture.prompts().len();
    });
    assert!(frame.contains("sending"), "{frame}");
    assert_eq!(prompts_then, 0);
    assert_eq!(fixture.prompts().len(), 1);
}

fn key_event(c: char) -> KeyEvent {
    match key(c) {
        Event::Key(key) => key,
        _ => unreachable!(),
    }
}

#[test]
fn resend_sends_the_focused_thread_alone() {
    let (fixture, mut app) = two_user_threads("resend", &[IDLE]);
    press(&fixture, &mut app, [key('S'), key('n')]);
    assert_eq!(app.view.focused(), Some(0));
    press(&fixture, &mut app, [key('s')]);
    let prompts = fixture.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("[u1]") && !prompts[1].contains("[u2]"));
    assert_eq!(app.message(), Some("sent 1 to claude"));
}

#[test]
fn resend_needs_an_open_thread_under_the_cursor() {
    let (fixture, mut app) = two_user_threads("resend-none", &[IDLE]);
    press(&fixture, &mut app, [key('s')]);
    assert_eq!(app.message(), Some("no thread here to resend"));
    press(&fixture, &mut app, [key('n'), key('x'), key('s')]);
    assert!(app.message().unwrap().contains("resolved thread"));
    assert!(fixture.prompts().is_empty());
}

#[test]
fn several_agents_show_the_picker_once_and_the_choice_is_remembered() {
    let two = [IDLE, ("w1:p2", "term_2", "idle")];
    let (fixture, mut app) = two_user_threads("picker", &two);
    press(&fixture, &mut app, [key('S')]);
    assert!(fixture.prompts().is_empty());
    let screen = screen_of(&app);
    assert!(screen.contains("several agents match") && screen.contains("> claude w1:p2"));
    press(&fixture, &mut app, [key('j')]);
    press(
        &fixture,
        &mut app,
        [Event::Key(KeyEvent::from(KeyCode::Enter))],
    );
    assert!(app.prompt.is_none());
    assert!(fixture.prompts()[0].starts_with("agent prompt w1:p2 "));
    let saved = crate::meta::load(&fixture.dir()).0.target.unwrap();
    assert_eq!(saved.pane.as_str(), "w1:p2");
    assert_eq!(app.target.as_ref().map(|t| t.pane.as_str()), Some("w1:p2"));
    // A new comment goes to the same agent without asking.
    let anchor = line_anchor(Side::New, 1, "new");
    actions::comment(&fixture.dir(), "t", &anchor, "later").unwrap();
    fixture.with_git(|git| app.check_store(git));
    press(&fixture, &mut app, [key('S')]);
    assert!(app.prompt.is_none());
    assert!(fixture.prompts()[1].starts_with("agent prompt w1:p2 "));
}

#[test]
fn escape_closes_the_picker_and_sends_nothing() {
    let two = [IDLE, ("w1:p2", "term_2", "idle")];
    let (fixture, mut app) = two_user_threads("picker-esc", &two);
    press(&fixture, &mut app, [key('S'), esc()]);
    assert!(app.prompt.is_none() && fixture.prompts().is_empty());
    assert_eq!(app.unsent(), 2);
}

#[test]
fn quitting_with_nothing_unsent_quits() {
    let (fixture, mut app) = two_user_threads("quit-clean", &[IDLE]);
    press(&fixture, &mut app, [key('S'), key('q')]);
    assert!(app.quit && app.prompt.is_none());
}

#[test]
fn quitting_with_unsent_comments_asks_and_stay_keeps_the_pane_open() {
    let (fixture, mut app) = two_user_threads("quit-stay", &[IDLE]);
    press(&fixture, &mut app, [key('q')]);
    assert!(!app.quit);
    let screen = screen_of(&app);
    assert!(
        screen.contains("2 unsent comments") && screen.contains("[s] send, then quit"),
        "{screen}"
    );
    press(&fixture, &mut app, [key('x')]);
    assert!(app.prompt.is_some(), "an unrelated key answers nothing");
    press(&fixture, &mut app, [esc()]);
    assert!(!app.quit && app.prompt.is_none());
    assert_eq!(app.unsent(), 2);
}

#[test]
fn quitting_with_keep_leaves_the_comments_unsent() {
    let (fixture, mut app) = two_user_threads("quit-keep", &[IDLE]);
    press(&fixture, &mut app, [key('q'), key('k')]);
    assert!(app.quit);
    assert!(fixture.prompts().is_empty());
    assert_eq!(
        read(&fixture.dir())
            .unwrap()
            .threads
            .iter()
            .filter(|t| t.unsent)
            .count(),
        2
    );
}

#[test]
fn quitting_with_send_delivers_first_and_a_refusal_keeps_the_pane_open() {
    let (fixture, mut app) = two_user_threads("quit-send", &[IDLE]);
    press(&fixture, &mut app, [key('q'), key('s')]);
    assert!(app.quit);
    assert_eq!(fixture.prompts().len(), 1);
    let (fixture, mut app) = two_user_threads("quit-refused", &[("w1:p1", "term_1", "blocked")]);
    press(&fixture, &mut app, [key('q'), key('s')]);
    assert!(!app.quit, "the comments were not delivered");
    assert!(app.message().unwrap().contains("waiting on a prompt"));
    assert_eq!(app.unsent(), 2);
}

#[test]
fn a_refusal_is_shown_and_notified_and_marks_nothing() {
    let (fixture, mut app) = two_user_threads("refused", &[("w1:p1", "term_1", "blocked")]);
    press(&fixture, &mut app, [key('S')]);
    let (tone, status) = app.status.clone().unwrap();
    assert_eq!(tone, Tone::Failure);
    assert!(status.contains("waiting on a prompt") && status.contains("still unsent"));
    assert!(screen_of(&app).contains("waiting on a prompt"));
    let notes = fixture.calls_starting("notification show review: not sent");
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("waiting on a prompt"));
    assert!(fixture.prompts().is_empty());
    assert_eq!(app.unsent(), 2);
}

#[test]
fn no_agent_is_a_refusal_with_the_reason() {
    let (fixture, mut app) = two_user_threads("no-agent", &[]);
    press(&fixture, &mut app, [key('S')]);
    assert_eq!(app.message(), Some("No agent found for this review."));
    assert!(app.target.is_none());
}

#[test]
fn an_edited_sent_comment_says_so_and_goes_out_on_resend_only() {
    let (fixture, mut app) = two_user_threads("edited", &[IDLE]);
    press(&fixture, &mut app, [key('S')]);
    let id = CommentId::parse("u1").unwrap();
    actions::edit(&fixture.dir(), "t", &id, "changed text").unwrap();
    fixture.with_git(|git| app.check_store(git));
    assert!(screen_of(&app).contains("edited since sent"));
    press(&fixture, &mut app, [key('S')]);
    assert_eq!(app.message(), Some("nothing to send"));
    assert_eq!(fixture.prompts().len(), 1);
    app.view.focus_thread(&id);
    press(&fixture, &mut app, [key('s')]);
    let prompts = fixture.prompts();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("changed text"));
    assert!(!screen_of(&app).contains("edited since sent"));
}

#[test]
fn the_loop_runs_a_queued_send_and_a_prompt_blocks_the_mouse() {
    let (fixture, mut app) = two_user_threads("loop-send", &[IDLE]);
    drive(&fixture, &mut app, vec![Some(key('S')), None], |_| {});
    assert_eq!(fixture.prompts().len(), 1);
    assert_eq!(app.message(), Some("sent 2 to claude"));
}

#[test]
fn the_layout_key_switches_between_unified_and_side_by_side_and_back() {
    let fixture = Fixture::new("layout-key");
    let mut app = opened(&fixture, patch_text());
    assert_eq!(app.view.layout(), crate::view::DiffLayout::Unified);
    press(&fixture, &mut app, [key('t')]);
    assert_eq!(app.view.layout(), crate::view::DiffLayout::Split);
    let screen = screen_of(&app);
    let row = screen.lines().find(|row| row.contains("old")).unwrap();
    assert!(row.contains("new"), "old and new share a row: {row}");
    press(&fixture, &mut app, [key('t')]);
    assert_eq!(app.view.layout(), crate::view::DiffLayout::Unified);
    assert!(
        !screen_of(&app)
            .lines()
            .any(|row| row.contains("old") && row.contains("new"))
    );
}

fn act(fixture: &Fixture, app: &mut App, action: Action) {
    fixture.with_git(|git| app.handle(action, git));
}

/// The column the open card's box starts at and the one it ends at, in the pane at `width`.
fn card_span(app: &mut App, width: u16) -> (usize, usize) {
    let (rows, _) = drawn(app, width);
    let top = rows.iter().find(|row| row.contains("Your note")).unwrap();
    let cells = top.chars().collect::<Vec<_>>();
    let start = cells.iter().position(|c| *c == '╭').unwrap();
    (start, cells.iter().position(|c| *c == '╮').unwrap())
}

#[test]
fn the_sidebar_key_hides_and_shows_it_and_the_cards_follow_the_stream_s_width() {
    // The sidebar is 20 columns of an 80 column pane, where a box is four cells into the
    // stream. It is 32 of a 130 column one, where the box is the new half of the stream:
    // 49 cells into a stream of 98, and 65 into one of 130.
    for (width, shown, hidden) in [(80, 24, 4), (130, 81, 65)] {
        let fixture = Fixture::new(&format!("sidebar-toggle-{width}"));
        write_log(&fixture, &[thread_event("u1", Author::User, "fix")]);
        let mut app = opened(&fixture, patch_text());
        let last = usize::from(width) - 1;
        assert_eq!(card_span(&mut app, width), (shown, last));
        assert!(
            drawn(&mut app, width)
                .0
                .join("\n")
                .contains(&format!("•M {} a.rs", crate::icons::icon("a.rs")))
        );
        act(&fixture, &mut app, Action::ToggleSidebar);
        assert!(!app.view.sidebar_drawn());
        assert_eq!(card_span(&mut app, width), (hidden, last));
        let (rows, _) = drawn(&mut app, width);
        assert!(rows[0].starts_with("M a.rs"), "{}", rows[0]);
        assert!(
            !rows
                .join("\n")
                .contains(&format!("•M {} a.rs", crate::icons::icon("a.rs")))
        );
        act(&fixture, &mut app, Action::ToggleSidebar);
        assert!(app.view.sidebar_drawn());
        assert_eq!(card_span(&mut app, width), (shown, last));
        assert_eq!(app.message(), None);
    }
}

#[test]
fn the_editor_box_follows_the_stream_while_the_sidebar_is_hidden() {
    let fixture = Fixture::new("sidebar-editor");
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, chars("fjjjc"));
    let (rows, buffer) = drawn(&mut app, 80);
    // Four cells into a stream that starts at the pane's first column.
    assert!(
        rows[4].starts_with("    ╭ Draft note - a.rs R1 ─"),
        "{}",
        rows[4]
    );
    assert!(rows[4].ends_with("─╮"), "{}", rows[4]);
    assert!(rows[7].starts_with("    ╰─"), "{}", rows[7]);
    // The mark of the commented row is in the pane's first column.
    assert_eq!(buffer[(0, 3)].symbol(), "▌");
}

#[test]
fn hiding_the_sidebar_moves_the_focus_to_the_stream_and_switch_panel_does_nothing() {
    let fixture = Fixture::new("sidebar-focus");
    let mut app = opened(&fixture, patch_text());
    act(&fixture, &mut app, Action::SwitchPanel);
    assert_eq!(app.view.panel, crate::view::Panel::Sidebar);
    act(&fixture, &mut app, Action::ToggleSidebar);
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    act(&fixture, &mut app, Action::SwitchPanel);
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    // With the focus on the stream, down moves one row and not one file.
    act(&fixture, &mut app, Action::Down);
    assert_eq!(app.view.cursor, 1);
    act(&fixture, &mut app, Action::ToggleSidebar);
    act(&fixture, &mut app, Action::SwitchPanel);
    assert_eq!(app.view.panel, crate::view::Panel::Sidebar);
}

#[test]
fn a_pane_under_50_columns_keeps_the_focus_on_the_stream_and_says_it_is_too_narrow() {
    let fixture = Fixture::new("sidebar-narrow");
    let mut app = opened(&fixture, patch_text());
    act(&fixture, &mut app, Action::SwitchPanel);
    // The pane shrinks with the sidebar focused.
    app.resize(Rect::new(0, 0, 49, 12));
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    act(&fixture, &mut app, Action::SwitchPanel);
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    act(&fixture, &mut app, Action::ToggleSidebar);
    assert!(!app.view.sidebar, "the key still flips the state");
    let said = Some((
        Tone::Notice,
        "the pane is too narrow to show the sidebar".to_owned(),
    ));
    assert_eq!(app.status, said);
    assert!(drawn(&mut app, 49).0[11].contains("too narrow to show the sidebar"));
    act(&fixture, &mut app, Action::ToggleSidebar);
    assert!(app.view.sidebar);
    assert_eq!(app.status, said);
    // Wide again, the sidebar is back, since the state is the one the key left.
    app.resize(Rect::new(0, 0, 80, 12));
    assert!(app.view.sidebar_drawn());
}

#[test]
fn a_click_and_the_plus_land_on_their_row_and_column_while_the_sidebar_is_hidden() {
    let fixture = Fixture::new("sidebar-mouse");
    let mut app = opened(&fixture, patch_text());
    act(&fixture, &mut app, Action::ToggleSidebar);
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    let click = event::MouseEventKind::Down(event::MouseButton::Left);
    // The stream starts at column 0. A click in what was the sidebar moves the cursor.
    app.mouse(mouse(click, 10, 2));
    assert_eq!(app.view.cursor, 2);
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    assert!(app.compose.is_none());
    app.mouse(mouse(event::MouseEventKind::Moved, 10, 3));
    let (rows, _) = drawn(&mut app, 80);
    assert!(rows[3].starts_with("[+]"), "{}", rows[3]);
    app.mouse(mouse(click, 1, 3));
    let compose = app.compose.as_ref().unwrap();
    assert!(matches!(&compose.draft, Draft::Comment(anchor)
        if anchor.target == AnchorTarget::Line { side: Side::New, line: 1, text: "new".into() }));
}

#[test]
fn the_sidebar_key_can_be_rebound_and_the_help_overlay_lists_it() {
    let fixture = Fixture::new("sidebar-rebind");
    let mut app = opened(&fixture, patch_text());
    app.view.help = true;
    app.resize(Rect::new(0, 0, 80, 30));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text.contains("f                show or hide the sidebar"));
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\ntoggle_sidebar = \"g\"\n",
    )
    .unwrap();
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('f')]);
    assert!(app.view.sidebar_drawn());
    press(&fixture, &mut app, [key('g')]);
    assert!(!app.view.sidebar_drawn());
}

/// A pane started with `config` as its `config.toml`.
fn configured(fixture: &Fixture, config: &str) -> App {
    std::fs::write(fixture.home.join("config/config.toml"), config).unwrap();
    opened(fixture, patch_text())
}

#[test]
fn the_config_starts_the_pane_with_the_sidebar_hidden_and_the_key_shows_it() {
    let fixture = Fixture::new("sidebar-closed");
    let mut app = configured(&fixture, "[sidebar]\nopen = false\n");
    assert!(!app.view.sidebar_drawn());
    assert!(app.warnings.is_empty(), "{:?}", app.warnings);
    assert!(drawn(&mut app, 80).0[0].starts_with("M a.rs"));
    press(&fixture, &mut app, [key('f')]);
    assert!(app.view.sidebar_drawn());
    // The key does not write the file, so the next start is hidden again.
    assert!(!opened(&fixture, patch_text()).view.sidebar_drawn());
}

#[test]
fn the_sidebar_starts_shown_when_the_config_says_so_or_says_nothing() {
    let fixture = Fixture::new("sidebar-open");
    assert!(opened(&fixture, patch_text()).view.sidebar_drawn());
    for config in ["[sidebar]\nopen = true\n", "[sidebar]\n", "[keys]\n"] {
        let app = configured(&fixture, config);
        assert!(app.view.sidebar_drawn(), "{config}");
        assert!(app.warnings.is_empty(), "{config}");
    }
}

#[test]
fn a_sidebar_table_of_the_wrong_shape_warns_and_starts_shown() {
    let fixture = Fixture::new("sidebar-bad");
    for (config, warning) in [
        (
            "[sidebar]\nopen = \"no\"\n",
            "[sidebar] open is not true or false, showing the sidebar",
        ),
        (
            "sidebar = false\n",
            "[sidebar] is not a table, showing the sidebar",
        ),
    ] {
        let app = configured(&fixture, config);
        assert!(app.view.sidebar_drawn(), "{config}");
        assert_eq!(app.warnings, [Warning::Config(warning.to_owned())]);
        assert!(screen_of(&app).contains(warning), "{config}");
    }
}

/// The sidebar's rows of an 80 column pane started with `config`, cut to the sidebar's width.
fn sidebar_rows(fixture: &Fixture, config: &str) -> (App, Vec<String>) {
    let mut app = configured(fixture, config);
    let rows = drawn(&mut app, 80)
        .0
        .iter()
        .map(|row| row.chars().take(20).collect())
        .collect();
    (app, rows)
}

fn first_file_row(rows: &[String]) -> &str {
    rows.iter()
        .find(|row| row.contains("a.rs"))
        .map_or("", String::as_str)
}

#[test]
fn icons_true_draws_the_icon_after_the_letter() {
    let fixture = Fixture::new("sidebar-icons");
    let (app, rows) = sidebar_rows(&fixture, "[sidebar]\nicons = true\n");
    assert!(app.warnings.is_empty(), "{:?}", app.warnings);
    let glyph = crate::icons::icon("a.rs");
    assert!(
        first_file_row(&rows).starts_with(&format!(" M {glyph} a.rs")),
        "{rows:?}"
    );
}

#[test]
fn icons_are_on_without_a_config_and_icons_false_turns_them_off() {
    let fixture = Fixture::new("sidebar-no-icons");
    let glyph = crate::icons::icon("a.rs");
    for config in ["", "[keys]\n", "[sidebar]\n"] {
        let (app, rows) = sidebar_rows(&fixture, config);
        assert!(app.warnings.is_empty(), "{config}");
        assert!(
            first_file_row(&rows).starts_with(&format!(" M {glyph} a.rs")),
            "{config} {rows:?}"
        );
    }
    let (app, rows) = sidebar_rows(&fixture, "[sidebar]\nicons = false\n");
    assert!(app.warnings.is_empty());
    assert!(first_file_row(&rows).starts_with(" M a.rs"), "{rows:?}");
}

#[test]
fn icons_that_is_not_a_boolean_warns_and_leaves_open_alone() {
    let fixture = Fixture::new("sidebar-icons-bad");
    let warning = "[sidebar] icons is not true or false, showing icons";
    let (app, rows) = sidebar_rows(&fixture, "[sidebar]\nicons = \"yes\"\n");
    assert_eq!(app.warnings, [Warning::Config(warning.to_owned())]);
    assert!(app.view.sidebar_drawn());
    let glyph = crate::icons::icon("a.rs");
    assert!(
        first_file_row(&rows).starts_with(&format!(" M {glyph} a.rs")),
        "{rows:?}"
    );
    let app = configured(&fixture, "[sidebar]\nopen = false\nicons = \"yes\"\n");
    assert_eq!(app.warnings, [Warning::Config(warning.to_owned())]);
    assert!(!app.view.sidebar_drawn());
}

#[test]
fn open_that_is_not_a_boolean_warns_and_leaves_icons_alone() {
    let fixture = Fixture::new("sidebar-open-bad");
    let (app, rows) = sidebar_rows(&fixture, "[sidebar]\nopen = \"no\"\nicons = false\n");
    assert_eq!(
        app.warnings,
        [Warning::Config(
            "[sidebar] open is not true or false, showing the sidebar".to_owned()
        )]
    );
    assert!(first_file_row(&rows).starts_with(" M a.rs"), "{rows:?}");
}

#[test]
fn a_bad_sidebar_table_is_one_warning_and_both_defaults() {
    let fixture = Fixture::new("sidebar-table-bad");
    let (app, rows) = sidebar_rows(&fixture, "sidebar = false\n");
    assert_eq!(app.warnings.len(), 1, "{:?}", app.warnings);
    assert!(app.view.sidebar_drawn());
    let glyph = crate::icons::icon("a.rs");
    assert!(
        first_file_row(&rows).starts_with(&format!(" M {glyph} a.rs")),
        "{rows:?}"
    );
}

#[test]
fn a_config_that_is_missing_or_not_toml_adds_no_warning_about_the_sidebar() {
    let fixture = Fixture::new("sidebar-no-file");
    let app = opened(&fixture, patch_text());
    assert!(app.view.sidebar_drawn() && app.warnings.is_empty());
    let app = configured(&fixture, "[sidebar\nopen = ");
    assert!(app.view.sidebar_drawn());
    assert_eq!(app.warnings.len(), 1, "{:?}", app.warnings);
    assert!(app.warnings[0].to_string().contains("not valid TOML"));
}

fn user_event(kind: Kind) -> LogEvent {
    LogEvent {
        at: "2026-10-05T00:00:00Z".into(),
        by: Author::User,
        kind,
    }
}

fn cid(text: &str) -> CommentId {
    CommentId::parse(text).unwrap()
}

/// Four threads of the user's: u1 sent and resolved, u2 resolved and never sent, u3 open, and
/// u4 resolved by the agent, which the user has not looked at.
fn with_resolved_threads(name: &str) -> (Fixture, App) {
    let fixture = Fixture::new(name);
    let mut events = ["u1", "u2", "u3", "u4"]
        .map(|id| thread_event(id, Author::User, id))
        .to_vec();
    events.push(user_event(Kind::Sent {
        ids: vec![cid("u1"), cid("u3"), cid("u4")],
        batch: crate::store::BatchId::parse("b1").unwrap(),
    }));
    events.push(user_event(Kind::Resolve { id: cid("u1") }));
    events.push(user_event(Kind::Resolve { id: cid("u2") }));
    let mut by_agent = user_event(Kind::Resolve { id: cid("u4") });
    by_agent.by = agent();
    events.push(by_agent);
    write_log(&fixture, &events);
    let app = opened(&fixture, patch_text());
    (fixture, app)
}

fn thread_ids(app: &App) -> Vec<&str> {
    let threads = app.review.threads.iter();
    threads.map(|thread| thread.root.id.as_str()).collect()
}

fn log_of(fixture: &Fixture, file: &str) -> String {
    std::fs::read_to_string(fixture.dir().join(file)).unwrap_or_default()
}

#[test]
fn archive_asks_first_and_then_moves_the_resolved_threads_out_of_the_pane() {
    let (fixture, mut app) = with_resolved_threads("archive");
    assert_eq!(app.unsent(), 1);
    press(&fixture, &mut app, [key('A')]);
    assert_eq!(app.prompt, Some(Prompt::Archive));
    let screen = screen_of(&app);
    assert!(
        screen.contains(" archive 2 resolved threads (1 never sent)? "),
        "{screen}"
    );
    assert!(screen.contains("[y] yes") && screen.contains("[n] no"));
    press(&fixture, &mut app, [key('x')]);
    assert!(app.prompt.is_some(), "an unrelated key answers nothing");
    press(&fixture, &mut app, [key('a')]);
    assert!(app.prompt.is_some(), "the old key answers nothing");
    assert_eq!(log_of(&fixture, "archive.jsonl"), "");
    press(&fixture, &mut app, [key('n')]);
    assert_eq!(app.prompt, None, "n stays");
    assert_eq!(log_of(&fixture, "archive.jsonl"), "");
    press(&fixture, &mut app, [key('A'), key('y')]);
    assert_eq!(app.prompt, None);
    assert_eq!(app.status, Some((Tone::Notice, "archived 2".to_owned())));
    // The open thread stays, and so does the one whose resolve is still new.
    assert_eq!(thread_ids(&app), ["u3", "u4"]);
    assert_eq!(app.unsent(), 0);
    let screen = screen_of(&app);
    assert!(screen.contains("archived 2"), "{screen}");
    assert!(!screen.contains("✓ u1") && !screen.contains("✓ u2"));
    assert!(screen.contains("✓ u4 [new]"), "{screen}");
    let moved = log_of(&fixture, "archive.jsonl");
    assert!(moved.contains("\"id\":\"u1\"") && moved.contains("\"id\":\"u2\""));
    assert!(!moved.contains("\"id\":\"u3\"") && !moved.contains("\"id\":\"u4\""));
    let left = log_of(&fixture, "review.jsonl");
    assert!(!left.contains("\"id\":\"u1\"") && !left.contains("\"id\":\"u2\""));
}

#[test]
fn prompts_draw_an_accent_box_with_bold_keys_and_a_green_yes_and_red_no() {
    let (fixture, mut app) = with_resolved_threads("archive-colours");
    press(&fixture, &mut app, [key('A')]);
    let mut terminal = terminal();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let cell_of = |text: &str| {
        let screen = buffer
            .content
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        // Every symbol here is one char, so a char count is a cell index.
        let at = screen.find(text).unwrap();
        buffer.content[screen[..at].chars().count()].clone()
    };
    let theme = &app.theme;
    assert_eq!(cell_of("┌").fg, theme.accent);
    assert_eq!(cell_of("archive 2").fg, theme.accent);
    let yes = cell_of("[y]");
    assert_eq!(yes.fg, theme.success);
    assert!(yes.modifier.contains(Modifier::BOLD));
    let no = cell_of("[n]");
    assert_eq!(no.fg, theme.removed);
    assert!(no.modifier.contains(Modifier::BOLD));
}

#[test]
fn escape_at_the_archive_prompt_writes_nothing() {
    let fixture = Fixture::new("archive-esc");
    write_log(
        &fixture,
        &[
            thread_event("u1", Author::User, "fix"),
            user_event(Kind::Sent {
                ids: vec![cid("u1")],
                batch: crate::store::BatchId::parse("b1").unwrap(),
            }),
            user_event(Kind::Resolve { id: cid("u1") }),
        ],
    );
    let mut app = opened(&fixture, patch_text());
    let before = log_of(&fixture, "review.jsonl");
    press(&fixture, &mut app, [key('A')]);
    // One thread, and it was sent, so the question has no bracket.
    assert!(screen_of(&app).contains(" archive 1 resolved thread? "));
    press(&fixture, &mut app, [esc()]);
    assert_eq!(app.prompt, None);
    assert_eq!(log_of(&fixture, "review.jsonl"), before);
    assert!(!fixture.dir().join("archive.jsonl").exists());
    assert_eq!(thread_ids(&app), ["u1"]);
}

#[test]
fn with_nothing_to_archive_the_status_line_says_so_and_no_prompt_opens() {
    let (fixture, mut app) = two_user_threads("archive-nothing", &[IDLE]);
    press(&fixture, &mut app, [key('A')]);
    assert_eq!(app.prompt, None);
    let said = Some((Tone::Notice, "nothing to archive".to_owned()));
    assert_eq!(app.status, said);
    assert!(screen_of(&app).contains("nothing to archive"));
    // A resolve that is still new is not something to archive either.
    let (fixture, mut app) = with_resolved_threads("archive-new-only");
    press(&fixture, &mut app, [key('A'), key('y'), key('A')]);
    assert_eq!(app.prompt, None);
    assert_eq!(app.status, said);
    assert_eq!(thread_ids(&app), ["u3", "u4"]);
}

#[test]
fn a_comment_written_after_an_archive_takes_the_next_id_also_after_a_restart() {
    let (fixture, mut app) = with_resolved_threads("archive-ids");
    press(&fixture, &mut app, [key('A'), key('y')]);
    // The cursor is on the file header, which a comment may point at.
    press(&fixture, &mut app, [key('c'), key('x'), ctrl_s()]);
    assert_eq!(thread_ids(&app), ["u3", "u4", "u5"]);
    let mut restarted = opened(&fixture, patch_text());
    press(&fixture, &mut restarted, [key('c'), key('x'), ctrl_s()]);
    assert_eq!(thread_ids(&restarted), ["u3", "u4", "u5", "u6"]);
    // Batch b1 went to the archive with u1, and the next send is still b2.
    fixture.agents(&[IDLE]);
    press(&fixture, &mut restarted, [key('S')]);
    assert!(log_of(&fixture, "review.jsonl").contains("\"batch\":\"b2\""));
}

#[test]
fn an_archive_of_a_busy_review_says_so_and_changes_neither_file() {
    let (fixture, mut app) = with_resolved_threads("archive-busy");
    let before = log_of(&fixture, "review.jsonl");
    let held = crate::store::lock(&fixture.dir()).unwrap();
    press(&fixture, &mut app, [key('A'), key('y')]);
    drop(held);
    let said = Some((Tone::Failure, "review is busy, press again".to_owned()));
    assert_eq!(app.status, said);
    assert_eq!(log_of(&fixture, "review.jsonl"), before);
    assert!(!fixture.dir().join("archive.jsonl").exists());
    assert_eq!(thread_ids(&app).len(), 4);
}

#[test]
fn an_archive_by_another_pane_is_picked_up_because_the_log_got_shorter() {
    let (fixture, mut app) = with_resolved_threads("archive-other-pane");
    let before = std::fs::metadata(fixture.dir().join("review.jsonl")).unwrap();
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            archive(&fixture.dir(), "2026-10-05T00:00:00Z").unwrap();
        }
    });
    let after = std::fs::metadata(fixture.dir().join("review.jsonl")).unwrap();
    assert!(after.len() < before.len());
    assert_eq!(thread_ids(&app), ["u3", "u4"]);
    assert!(app.warnings.is_empty(), "{:?}", app.warnings);
    assert!(!screen_of(&app).contains("✓ u1"));
}

#[test]
fn the_archive_key_can_be_rebound_and_the_help_overlay_lists_it() {
    let (fixture, mut app) = with_resolved_threads("archive-rebind");
    app.view.help = true;
    app.resize(Rect::new(0, 0, 80, 30));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text.contains("A                archive the resolved threads"));
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\narchive = \"z\"\n",
    )
    .unwrap();
    let mut app = opened(&fixture, patch_text());
    press(&fixture, &mut app, [key('A')]);
    assert_eq!(app.prompt, None);
    press(&fixture, &mut app, [key('z')]);
    assert_eq!(app.prompt, Some(Prompt::Archive));
}

#[test]
fn clicking_the_plus_of_a_hovered_line_opens_the_editor_on_that_line() {
    let fixture = Fixture::new("plus-click");
    let mut app = opened(&fixture, patch_text());
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    // The stream starts at column 20. Row 3 is the added line, a.rs line 1 on the new side.
    app.mouse(mouse(event::MouseEventKind::Moved, 30, 3));
    assert!(screen_of(&app).lines().nth(3).unwrap().contains("[+]"));
    app.mouse(mouse(
        event::MouseEventKind::Down(event::MouseButton::Left),
        21,
        3,
    ));
    let compose = app.compose.as_ref().unwrap();
    assert!(matches!(&compose.draft, Draft::Comment(anchor)
        if anchor.target == AnchorTarget::Line { side: Side::New, line: 1, text: "new".into() }));
    assert_eq!(
        (compose.title.as_str(), compose.place.as_str()),
        ("Draft note - ", "a.rs R1")
    );
}

#[test]
fn a_click_off_the_plus_does_not_open_the_editor() {
    let fixture = Fixture::new("plus-miss");
    let mut app = opened(&fixture, patch_text());
    app.mouse(MouseEvent {
        kind: event::MouseEventKind::Down(event::MouseButton::Left),
        column: 40,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.compose.is_none());
    assert_eq!(app.view.cursor, 3);
}

#[test]
fn the_pane_is_painted_in_the_flavor_the_config_names() {
    let fixture = Fixture::new("theme");
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[theme]\nname = \"catppuccin-latte\"\n",
    )
    .unwrap();
    let app = fixture.started();
    let latte = Theme::named("catppuccin-latte").unwrap();
    assert_eq!(app.theme, latte);
    assert!(app.warnings.is_empty(), "{:?}", app.warnings);
    let mut terminal = terminal();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    // Rows of the stream: the file header under the cursor, the hunk header, `-old`, `+new`.
    assert_eq!(buffer[(40, 0)].bg, latte.cursor);
    assert_eq!(buffer[(40, 2)].bg, latte.removed_bg);
    assert_eq!(buffer[(40, 3)].bg, latte.added_bg);
    // An empty row of the stream, the sidebar, and the warning line have the base behind them.
    for at in [(40, 8), (2, 8), (40, 10)] {
        assert_eq!(buffer[at].bg, latte.base, "{at:?}");
        assert_eq!(buffer[at].fg, latte.text, "{at:?}");
    }
}

#[test]
fn a_card_s_footer_names_the_keys_the_config_gives_and_an_agent_s_offers_only_reply() {
    let fixture = Fixture::new("card-footer");
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nreply = \"ctrl+r\"\ndelete = \"shift+x\"\n",
    )
    .unwrap();
    write_log(
        &fixture,
        &[
            thread_event("u1", Author::User, "mine"),
            thread_event("a1", agent(), "theirs"),
        ],
    );
    let mut app = opened(&fixture, patch_text());
    // Four rows of the diff and two boxes of four rows each.
    let (rows, _) = drawn_tall(&mut app, 80, 16);
    let rows = stream_columns(&rows);
    let footers = rows
        .iter()
        .filter(|row| row.contains('╰'))
        .collect::<Vec<_>>();
    assert_eq!(footers.len(), 2, "{}", rows.join("\n"));
    assert!(
        footers[0].ends_with("─ ctrl+r reply  e edit  X delete ╯"),
        "{}",
        footers[0]
    );
    assert!(footers[1].ends_with("─ ctrl+r reply ╯"), "{}", footers[1]);
    assert!(
        rows.join("\n").contains("● claude · "),
        "{}",
        rows.join("\n")
    );
    // The configured key is the one that works.
    press(
        &fixture,
        &mut app,
        [
            key('n'),
            Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        ],
    );
    assert!(app.compose.is_some());
}

#[test]
fn a_code_row_is_drawn_in_token_colours_over_its_tint_once_its_file_is_highlighted() {
    let fixture = Fixture::new("syntax");
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-let n = 41;\n+let n = 42;\n";
    std::fs::write(fixture.root.join("a.rs"), "let n = 42;\n").unwrap();
    let mut app = opened(&fixture, patch);
    // The stream starts at column 20, and the text of a unified row 12 cells further.
    let (keyword, number) = ((32, 3), (40, 3));
    let (rows, buffer) = drawn(&mut app, 80);
    assert!(rows[3].trim_end().ends_with("+let n = 42;"), "{}", rows[3]);
    // Before the file is highlighted the row is one colour, the green of an added row.
    assert_eq!(buffer[keyword].fg, app.theme.added);
    assert_eq!(buffer[number].fg, app.theme.added);
    fixture.with_git(|git| app.highlight(git, &mut || true));
    let (after, buffer) = drawn(&mut app, 80);
    assert_eq!(after, rows, "highlighting changes colours and no character");
    let highlighted = cfg!(feature = "syntax");
    let expect = |token, plain| if highlighted { token } else { plain };
    assert_eq!(
        buffer[keyword].fg,
        expect(app.theme.keyword, app.theme.added)
    );
    assert_eq!(buffer[number].fg, expect(app.theme.number, app.theme.added));
    // The sign keeps the colour of its kind, and the tint stays behind all of it.
    assert_eq!(buffer[(31, 3)].fg, app.theme.added);
    for x in [31, 32, 40, 70] {
        assert_eq!(buffer[(x, 3)].bg, app.theme.added_bg, "column {x}");
    }
    // The removed row comes from the old side, which this git cannot show: its hunk is
    // highlighted as a snippet.
    assert_eq!(
        buffer[(40, 2)].fg,
        expect(app.theme.number, app.theme.removed)
    );
    assert_eq!(buffer[(40, 2)].bg, app.theme.removed_bg);
    // Side by side, the new half starts after the 32 column sidebar and the 49 column old half.
    let (rows, buffer) = drawn(&mut app, 130);
    let column = rows[2].chars().position(|c| c == '+').unwrap();
    let at = (u16::try_from(column).unwrap() + 2, 2);
    assert_eq!(buffer[at].symbol(), "l");
    assert_eq!(buffer[at].fg, expect(app.theme.keyword, app.theme.added));
    assert_eq!(buffer[at].bg, app.theme.added_bg);
    // A reload that finds the file as it was keeps its tokens.
    press(&fixture, &mut app, [key('R')]);
    assert!(app.syntax.file("a.rs").is_some());
}

#[cfg(feature = "syntax")]
const A_RS: &str =
    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-let n = 41;\n+let n = 42;\n";

#[cfg(feature = "syntax")]
/// A pane on `A_RS` with `a.rs` highlighted once, which asks `git show` for its old side.
fn highlighted(fixture: &Fixture) -> App {
    std::fs::write(fixture.root.join("a.rs"), "let n = 42;\n").unwrap();
    let mut app = opened(fixture, A_RS);
    fixture.with_git(|git| app.highlight(git, &mut || true));
    assert_eq!(fixture.shows.get(), 1);
    app
}

#[test]
fn the_loop_does_not_wait_for_a_key_while_a_file_on_screen_is_not_highlighted() {
    let fixture = Fixture::new("syntax-poll");
    let mut app = fixture.started();
    let waits = RefCell::new(Vec::new());
    let poll = |wait| {
        waits.borrow_mut().push(wait);
        Ok(None)
    };
    fixture.with_git(|git| {
        run_loop(&mut app, &mut terminal(), git, poll, || {
            waits.borrow().len() >= 200
        })
    });
    let waits = waits.into_inner();
    assert_eq!(waits.first(), Some(&Duration::ZERO));
    assert_eq!(waits.last(), Some(&TICK));
    assert!(!app.highlight_pending());
}

#[cfg(feature = "syntax")]
#[test]
fn a_file_is_highlighted_only_once_no_key_is_waiting() {
    let fixture = Fixture::new("syntax-idle");
    std::fs::write(fixture.root.join("a.rs"), "let n = 42;\n").unwrap();
    let mut app = opened(&fixture, A_RS);
    // The frame of each key is drawn with the file not read yet.
    drive(&fixture, &mut app, vec![Some(key('j')); 5], |_| {});
    assert_eq!(fixture.shows.get(), 0);
    assert!(app.highlight_pending());
    drive(&fixture, &mut app, vec![None; 200], |_| {});
    assert_eq!(fixture.shows.get(), 1);
    assert!(!app.highlight_pending());
}

#[cfg(feature = "syntax")]
#[test]
fn a_reload_with_the_file_unchanged_reads_it_no_more() {
    let fixture = Fixture::new("syntax-kept");
    let mut app = highlighted(&fixture);
    press(&fixture, &mut app, [key('R')]);
    fixture.with_git(|git| app.highlight(git, &mut || true));
    assert_eq!(fixture.shows.get(), 1);
}

#[cfg(feature = "syntax")]
#[test]
fn a_reload_after_the_file_changed_highlights_it_again() {
    let fixture = Fixture::new("syntax-changed");
    let mut app = highlighted(&fixture);
    std::fs::write(fixture.root.join("a.rs"), "let n = 43;\n").unwrap();
    *fixture.patch.borrow_mut() = A_RS.replace("42", "43").into_bytes();
    press(&fixture, &mut app, [key('R')]);
    fixture.with_git(|git| app.highlight(git, &mut || true));
    assert_eq!(fixture.shows.get(), 2);
}

#[cfg(feature = "syntax")]
#[test]
fn a_reload_against_another_revision_highlights_every_file_again() {
    let fixture = Fixture::new("syntax-spec");
    let mut app = highlighted(&fixture);
    let rev = app.diff.as_ref().unwrap().rev.clone();
    press(&fixture, &mut app, [key('b')]);
    assert_ne!(app.diff.as_ref().unwrap().rev, rev);
    fixture.with_git(|git| app.highlight(git, &mut || true));
    assert_eq!(fixture.shows.get(), 2);
}

#[test]
fn an_unknown_theme_is_a_warning_and_the_pane_starts_in_mocha() {
    let fixture = Fixture::new("theme-unknown");
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nsend = \"ctrl+s\"\n[theme]\nname = \"nord\"\n",
    )
    .unwrap();
    let app = fixture.started();
    assert_eq!(app.theme, Theme::default());
    assert_eq!(app.keymap.label(Action::Send), "ctrl+s");
    assert!(
        screen_of(&app).contains("unknown theme 'nord', using catppuccin-mocha"),
        "{}",
        screen_of(&app)
    );
}

#[test]
fn visual_mode_names_the_range_and_hides_the_state_until_it_ends() {
    let fixture = Fixture::new("visual-footer");
    let mut app = opened(&fixture, TWO_FILES);
    assert!(screen_of(&app).contains("WORKING TREE"));
    press(&fixture, &mut app, [key('j'), key('j'), key('v')]);
    let screen = screen_of(&app);
    assert!(screen.contains(" VISUAL  a.rs R1 (1 line)"), "{screen}");
    assert!(screen.contains("c comment  v/esc cancel"), "{screen}");
    assert!(!screen.contains("WORKING TREE") && !screen.contains("no agent"));
    // The range grows with the cursor: a1, a2 removed, A2 added and a3 are new lines 1 to 3.
    press(&fixture, &mut app, [key('j'), key('j'), key('j')]);
    assert!(
        screen_of(&app).contains(" VISUAL  a.rs R1-3 (3 lines)"),
        "{}",
        screen_of(&app)
    );
    press(&fixture, &mut app, [key('v')]);
    let screen = screen_of(&app);
    assert!(screen.contains("WORKING TREE") && screen.contains("no agent"));
    assert!(!screen.contains("VISUAL"));
}

#[test]
fn the_visual_status_line_is_tinted_across_the_whole_width() {
    let fixture = Fixture::new("visual-tint");
    let mut app = opened(&fixture, TWO_FILES);
    press(&fixture, &mut app, [key('v')]);
    let theme = Theme::default();
    let (_, row) = status_row(&mut app, 80);
    assert_eq!(row.len(), 80);
    let chip = " VISUAL ".len();
    for (at, cell) in row.iter().enumerate() {
        let want = if at < chip {
            theme.visual
        } else {
            theme.selection
        };
        assert_eq!(cell.bg, want, "cell {at}");
    }
    assert_eq!(row[1].fg, theme.base);
    // A notice takes the left side and is drawn on the tint as well.
    app.notice("select lines to comment on");
    let (text, row) = status_row(&mut app, 80);
    assert!(text.contains("select lines to comment on"));
    assert!(row.iter().all(|cell| cell.bg == theme.selection));
    // Outside visual mode the bar is the header colour again.
    app.view.select = None;
    assert!(
        status_row(&mut app, 80)
            .1
            .iter()
            .all(|cell| cell.bg == theme.header)
    );
}

#[test]
fn a_range_across_two_files_shows_why_in_the_footer() {
    let fixture = Fixture::new("visual-two-files");
    let mut app = opened(&fixture, TWO_FILES);
    press(&fixture, &mut app, [key('j'), key('j'), key('v')]);
    press(&fixture, &mut app, (0..6).map(|_| key('j')));
    let line = status_row(&mut app, 80);
    let why = "a range stays inside one file";
    assert!(
        cells_of(&line, why)
            .iter()
            .all(|cell| cell.fg == Theme::default().removed)
    );
    // Pressing `c` leaves the editor closed and the mode on.
    press(&fixture, &mut app, [key('c')]);
    assert!(app.compose.is_none() && app.view.select.is_some());
}

#[test]
fn the_visual_footer_keys_follow_a_rebound_comment_and_select_range() {
    let fixture = Fixture::new("visual-rebind");
    let mut app = configured(&fixture, "[keys]\ncomment = \"m\"\nselect_range = \"w\"\n");
    press(&fixture, &mut app, [key('w')]);
    let screen = screen_of(&app);
    assert!(screen.contains("VISUAL"), "{screen}");
    assert!(screen.contains("m comment  w/esc cancel"), "{screen}");
    assert!(!screen.contains("c comment") && !screen.contains("v/esc"));
}

#[test]
fn escape_leaves_visual_mode_and_does_nothing_outside_it() {
    let fixture = Fixture::new("visual-escape");
    let mut app = opened(&fixture, TWO_FILES);
    press(&fixture, &mut app, [key('j'), key('j')]);
    let before = app.view.clone();
    press(&fixture, &mut app, [esc()]);
    assert_eq!(app.view, before);
    assert!(app.compose.is_none() && app.prompt.is_none() && !app.quit);
    press(&fixture, &mut app, [key('v'), key('j')]);
    assert!(app.view.select.is_some());
    press(&fixture, &mut app, [esc()]);
    assert_eq!(app.view.select, None);
    assert_eq!(app.view.cursor, 3);
    assert!(!screen_of(&app).contains("VISUAL"));
}

#[test]
fn a_write_from_another_process_waits_for_visual_mode_to_end() {
    let fixture = Fixture::new("visual-pickup");
    let mut app = opened(&fixture, TWO_FILES);
    press(&fixture, &mut app, [key('v')]);
    let line = log_line(&add_event("u1"));
    let path = fixture.dir().join("review.jsonl");
    drive(&fixture, &mut app, vec![None, None], |tick| {
        if tick == 0 {
            std::fs::write(&path, &line).unwrap();
        }
    });
    assert_eq!(app.review.threads.len(), 0);
    assert_eq!(fixture.diffs.get(), 1);
    // Focus coming back does not reload either.
    drive(&fixture, &mut app, vec![Some(Event::FocusGained)], |_| {});
    assert_eq!(fixture.diffs.get(), 1);
    press(&fixture, &mut app, [esc()]);
    drive(&fixture, &mut app, vec![None], |_| {});
    assert_eq!(app.review.threads.len(), 1);
    assert_eq!(fixture.diffs.get(), 2);
}

// The file filter. `FILTER_FILES` are files 0 to 3 of the diff: README.md, src/tui.rs, src/view.rs and
// src/view/tests.rs, each with one hunk of two lines, so a file is four rows of the stream.

const FILTER_FILES: [&str; 4] = [
    "README.md",
    "src/tui.rs",
    "src/view.rs",
    "src/view/tests.rs",
];

fn files_patch(paths: &[&str]) -> String {
    use std::fmt::Write as _;
    let mut patch = String::new();
    for path in paths {
        let _ = write!(
            patch,
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-old\n+new\n"
        );
    }
    patch
}

fn filter_pane(fixture: &Fixture) -> App {
    opened(fixture, &files_patch(&FILTER_FILES))
}

/// The sidebar as the names it lists: a heading as drawn, a file as ` name` after its icon, and
/// the filter's box as its line of text (`> vw 2/4`) with its borders left out. The status line
/// is left out.
fn sidebar(app: &mut App) -> Vec<String> {
    let (rows, _) = drawn(app, 80);
    rows.iter()
        .take(rows.len() - 1)
        .map(|row| row.chars().take(19).collect::<String>())
        .filter(|row| !row.starts_with(['╭', '╰']))
        .map(|row| row.trim_end().to_owned())
        .filter(|row| !row.is_empty())
        .map(|row| match row.split_whitespace().nth(2) {
            Some(name) if row.starts_with(" M") => format!(" {name}"),
            _ if row.starts_with('│') => row
                .trim_matches('│')
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            _ => row,
        })
        .collect()
}

fn file_under_cursor(app: &App) -> usize {
    app.view.stream.file_at(app.view.cursor)
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn enter() -> Event {
    Event::Key(KeyEvent::from(KeyCode::Enter))
}

fn backspace() -> Event {
    Event::Key(KeyEvent::from(KeyCode::Backspace))
}

#[test]
fn typing_narrows_the_sidebar_in_the_diffs_order_and_the_stream_does_not_move() {
    let fixture = Fixture::new("filter-type");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("jjj"));
    let (cursor, scroll) = (app.view.cursor, app.view.scroll);
    assert_eq!(
        sidebar(&mut app),
        [
            "> filter (/) 4/4",
            "./",
            " README.…",
            "src/",
            " tui.rs",
            " view.rs",
            "src/view/",
            " tests.rs"
        ]
    );
    press(&fixture, &mut app, chars("/vw"));
    assert_eq!(app.view.panel, crate::view::Panel::Sidebar);
    assert_eq!(
        sidebar(&mut app),
        ["> vw 2/4", "src/", " view.rs", "src/view/", " tests.rs"]
    );
    assert_eq!((app.view.cursor, app.view.scroll), (cursor, scroll));
    // A character is a literal one: a space matches nothing here.
    press(&fixture, &mut app, [key(' ')]);
    assert_eq!(sidebar(&mut app), ["> vw 0/4", "no match"]);
}

#[test]
fn backspace_widens_the_list_and_ctrl_u_empties_the_query() {
    let fixture = Fixture::new("filter-erase");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/vw"));
    press(&fixture, &mut app, [backspace(), backspace()]);
    assert_eq!(sidebar(&mut app).len(), 8, "the query row and every file");
    // On an empty query backspace does nothing.
    press(&fixture, &mut app, [backspace()]);
    assert_eq!(app.view.filter.as_ref().unwrap().query, "");
    press(&fixture, &mut app, chars("src"));
    assert_eq!(sidebar(&mut app)[0], "> src 3/4");
    press(&fixture, &mut app, [ctrl('u')]);
    assert_eq!(app.view.filter.as_ref().unwrap().query, "");
    assert_eq!(sidebar(&mut app).len(), 8);
}

#[test]
fn enter_applies_the_filter_focuses_the_sidebar_and_moves_to_the_first_match() {
    let fixture = Fixture::new("filter-apply");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/vw"));
    press(&fixture, &mut app, [enter()]);
    let filter = app.view.filter.as_ref().unwrap();
    assert!(!filter.typing);
    assert_eq!(app.view.panel, crate::view::Panel::Sidebar);
    assert_eq!(file_under_cursor(&app), 2);
    // The query row stays, with no cursor cell after it.
    assert_eq!(sidebar(&mut app)[0], "> vw 2/4");
    // The keys are the keymap's again: `j` is `down`, not a character of the query.
    press(&fixture, &mut app, [key('j')]);
    assert_eq!(app.view.filter.as_ref().unwrap().query, "vw");
    assert_eq!(file_under_cursor(&app), 3);
}

#[test]
fn enter_leaves_the_cursor_when_its_file_already_matches() {
    let fixture = Fixture::new("filter-keep");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("jjjj"));
    assert_eq!(file_under_cursor(&app), 1);
    press(&fixture, &mut app, chars("/s"));
    let cursor = app.view.cursor;
    press(&fixture, &mut app, [enter()]);
    assert_eq!(app.view.cursor, cursor);
}

#[test]
fn enter_with_no_match_is_ignored_and_the_list_says_no_match() {
    let fixture = Fixture::new("filter-nomatch");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/zzz"));
    assert_eq!(sidebar(&mut app), ["> zzz 0/4", "no match"]);
    press(&fixture, &mut app, [enter()]);
    assert!(app.view.filter.as_ref().unwrap().typing);
    press(
        &fixture,
        &mut app,
        [backspace(), backspace(), backspace(), key('v')],
    );
    assert_eq!(sidebar(&mut app)[0], "> v 2/4");
}

#[test]
fn enter_on_an_empty_query_clears_the_filter() {
    let fixture = Fixture::new("filter-empty");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/"));
    assert!(app.view.filter.is_some());
    press(&fixture, &mut app, [enter()]);
    assert!(app.view.filter.is_none());
    assert_eq!(sidebar(&mut app).len(), 8);
}

#[test]
fn esc_while_typing_clears_the_filter_and_leaves_the_cursor() {
    let fixture = Fixture::new("filter-esc-typing");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("jjj/vw"));
    let cursor = app.view.cursor;
    press(&fixture, &mut app, [esc()]);
    assert!(app.view.filter.is_none());
    assert_eq!(app.view.cursor, cursor);
    assert_eq!(sidebar(&mut app).len(), 8);
}

#[test]
fn esc_with_a_filter_applied_clears_it_from_the_stream_and_after_a_selection() {
    let fixture = Fixture::new("filter-esc-applied");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/vw"));
    press(
        &fixture,
        &mut app,
        [enter(), Event::Key(KeyEvent::from(KeyCode::Tab))],
    );
    assert_eq!(app.view.panel, crate::view::Panel::Stream);
    press(&fixture, &mut app, [esc()]);
    assert!(app.view.filter.is_none());
    assert_eq!(sidebar(&mut app).len(), 8);

    press(&fixture, &mut app, chars("/vw"));
    press(&fixture, &mut app, [enter()]);
    app.view.panel = crate::view::Panel::Stream;
    press(&fixture, &mut app, [key('v')]);
    assert!(app.view.select.is_some());
    press(&fixture, &mut app, [esc()]);
    assert!(app.view.select.is_none());
    assert!(app.view.filter.is_some());
    press(&fixture, &mut app, [esc()]);
    assert!(app.view.filter.is_none());
}

#[test]
fn the_sidebar_keys_visit_only_the_matches_from_a_file_that_does_not_match_too() {
    let fixture = Fixture::new("filter-keys");
    let mut app = filter_pane(&fixture);
    // `t` matches src/tui.rs (1) and src/view/tests.rs (3).
    press(&fixture, &mut app, chars("/t"));
    press(&fixture, &mut app, [enter()]);
    assert_eq!(file_under_cursor(&app), 1);
    press(&fixture, &mut app, [key('j')]);
    assert_eq!(file_under_cursor(&app), 3);
    press(&fixture, &mut app, [key('j')]);
    assert_eq!(file_under_cursor(&app), 3);
    press(&fixture, &mut app, [key('k')]);
    assert_eq!(file_under_cursor(&app), 1);
    press(&fixture, &mut app, [key('k')]);
    assert_eq!(file_under_cursor(&app), 1);
    // The cursor on file 2, which does not match: no row is highlighted.
    let on = |app: &mut App, file: usize| {
        app.view.cursor = app.view.stream.file_start(file).unwrap();
    };
    on(&mut app, 2);
    press(&fixture, &mut app, [key('j')]);
    assert_eq!(file_under_cursor(&app), 3);
    on(&mut app, 2);
    press(&fixture, &mut app, [key('k')]);
    assert_eq!(file_under_cursor(&app), 1);
    on(&mut app, 2);
    press(&fixture, &mut app, [ctrl('d')]);
    assert_eq!(file_under_cursor(&app), 3);
    on(&mut app, 2);
    press(&fixture, &mut app, [ctrl('u')]);
    assert_eq!(file_under_cursor(&app), 1);
    // Nothing matches before file 0, or after file 3 that is not already a match.
    on(&mut app, 0);
    press(&fixture, &mut app, [key('k')]);
    assert_eq!(file_under_cursor(&app), 0);
}

#[test]
fn a_click_on_a_narrowed_row_selects_the_file_drawn_there() {
    let fixture = Fixture::new("filter-click");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/vw"));
    press(&fixture, &mut app, [enter()]);
    assert_eq!(
        sidebar(&mut app),
        ["> vw 2/4", "src/", " view.rs", "src/view/", " tests.rs"]
    );
    let click = |column, row| MouseEvent {
        kind: event::MouseEventKind::Down(event::MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    // Rows 0 to 2 are the box, 3 is the heading, 4 `view.rs` and 6 `tests.rs`.
    app.mouse(click(3, 6));
    assert_eq!(file_under_cursor(&app), 3);
    app.mouse(click(3, 4));
    assert_eq!(file_under_cursor(&app), 2);
    app.mouse(click(3, 1));
    assert_eq!(file_under_cursor(&app), 2);
    assert_eq!(app.view.filter.as_ref().unwrap().query, "vw");
}

#[test]
fn the_filter_key_shows_a_hidden_sidebar_and_a_narrow_pane_opens_nothing() {
    let fixture = Fixture::new("filter-hidden");
    let mut app = filter_pane(&fixture);
    act(&fixture, &mut app, Action::ToggleSidebar);
    assert!(!app.view.sidebar_drawn());
    press(&fixture, &mut app, [key('/')]);
    assert!(app.view.sidebar_drawn());
    assert_eq!(app.view.panel, crate::view::Panel::Sidebar);
    assert!(app.view.filter.as_ref().unwrap().typing);

    let mut app = filter_pane(&fixture);
    app.resize(Rect::new(0, 0, 49, 12));
    press(&fixture, &mut app, [key('/')]);
    assert!(app.view.filter.is_none());
    assert_eq!(
        app.status,
        Some((
            Tone::Notice,
            "the pane is too narrow to show the sidebar".to_owned()
        ))
    );
}

#[test]
fn the_filter_key_does_nothing_on_the_start_up_message_screen() {
    let fixture = Fixture::new("filter-message");
    fixture.repo_ok.set(false);
    let mut app = fixture.app();
    fixture.with_git(|git| app.load(git));
    act(&fixture, &mut app, Action::Filter);
    assert!(app.view.filter.is_none());
    assert_eq!(app.status, None);
}

#[test]
fn the_filter_key_reopens_an_applied_query_for_more_typing() {
    let fixture = Fixture::new("filter-reopen");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/v"));
    press(&fixture, &mut app, [enter()]);
    press(&fixture, &mut app, [key('/')]);
    assert!(app.view.filter.as_ref().unwrap().typing);
    assert_eq!(app.view.filter.as_ref().unwrap().query, "v");
    press(&fixture, &mut app, [key('w')]);
    assert_eq!(sidebar(&mut app)[0], "> vw 2/4");
}

#[test]
fn a_reload_keeps_the_query_and_a_reload_that_matches_nothing_says_no_match() {
    let fixture = Fixture::new("filter-reload");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/vw"));
    press(&fixture, &mut app, [enter()]);
    *fixture.patch.borrow_mut() = files_patch(&["README.md", "src/view.rs"]).into_bytes();
    press(&fixture, &mut app, [key('R')]);
    assert_eq!(sidebar(&mut app), ["> vw 1/2", "src/", " view.rs"]);
    *fixture.patch.borrow_mut() = files_patch(&["README.md"]).into_bytes();
    press(&fixture, &mut app, [key('R')]);
    assert_eq!(sidebar(&mut app), ["> vw 0/1", "no match"]);
    assert!(app.view.filter.is_some());
}

#[test]
fn the_filter_key_can_be_rebound_the_help_lists_it_and_the_footer_does_not() {
    let fixture = Fixture::new("filter-rebind");
    let mut app = filter_pane(&fixture);
    app.view.help = true;
    app.resize(Rect::new(0, 0, 80, 30));
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    assert!(text.contains("/                filter the files in the sidebar"));
    app.view.help = false;
    assert!(!drawn(&mut app, 80).0[11].contains("filter"));

    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nfilter = \"g\"\n",
    )
    .unwrap();
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, [key('/')]);
    assert!(app.view.filter.is_none());
    press(&fixture, &mut app, [key('g')]);
    assert!(app.view.filter.is_some());
}

#[test]
fn while_typing_a_key_bound_to_an_action_goes_into_the_query() {
    let fixture = Fixture::new("filter-bound-keys");
    let mut app = filter_pane(&fixture);
    press(&fixture, &mut app, chars("/qjf"));
    assert_eq!(app.view.filter.as_ref().unwrap().query, "qjf");
    assert!(!app.quit);
    assert!(app.view.sidebar);
}

#[test]
fn the_box_is_always_drawn_and_its_hint_names_the_key_of_the_filter() {
    let fixture = Fixture::new("filter-hint");
    let mut app = filter_pane(&fixture);
    assert_eq!(sidebar(&mut app)[0], "> filter (/) 4/4");
    assert!(app.view.filter.is_none());
    press(&fixture, &mut app, [key('/')]);
    assert_eq!(
        sidebar(&mut app)[0],
        "> 4/4",
        "the hint goes when typing starts"
    );
    press(&fixture, &mut app, [esc()]);
    assert_eq!(sidebar(&mut app)[0], "> filter (/) 4/4");

    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nfilter = \"g\"\n",
    )
    .unwrap();
    let mut app = filter_pane(&fixture);
    assert_eq!(sidebar(&mut app)[0], "> filter (g) 4/4");
    std::fs::write(
        fixture.home.join("config/config.toml"),
        "[keys]\nfilter = \"\"\n",
    )
    .unwrap();
    let mut app = filter_pane(&fixture);
    assert_eq!(sidebar(&mut app)[0], "> filter 4/4");
}
