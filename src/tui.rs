//! The review pane: the terminal guard, the loop, and what start-up reads.
//!
//! Everything below `run` takes its terminal, its input, its `git` and its clock as parameters, so
//! tests drive it with a `TestBackend` and a list of events.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::diff::{Diff, GitError, RepoRoot, default_base, load, run_git_bytes};
use crate::env::Env;
use crate::keymap::{Action, Keymap};
use crate::meta::{Meta, locate, save};
use crate::store::{PaneId, Review, Spec, Warning, log_len, read};
use crate::termination::Termination;
use crate::view::{View, draw};
use crate::width::truncate_to_width;

/// How long the loop waits for a key before it checks the store and the signal flag.
const TICK: Duration = Duration::from_millis(250);

/// `git` as the pane sees it: the arguments in, the stdout bytes out.
pub type Git<'a> = dyn FnMut(&[String]) -> Result<Vec<u8>, GitError> + 'a;

/// Remove terminal control characters while retaining useful whitespace. A tab becomes four
/// spaces. ESC and the C1 controls go, so a file name cannot drive the terminal.
pub fn sanitize_terminal_text(text: &str) -> String {
    text.chars()
        .flat_map(|character| {
            if character == '\t' {
                "    ".chars().collect::<Vec<_>>()
            } else if (character <= '\u{0008}')
                || matches!(character, '\u{000b}' | '\u{000c}')
                || ('\u{000e}'..='\u{001f}').contains(&character)
                || ('\u{007f}'..='\u{009f}').contains(&character)
            {
                Vec::new()
            } else {
                vec![character]
            }
        })
        .collect()
}

/// What fills the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Start-up failed. The message says why, and `reload` and `quit` still work.
    Message(String),
    Review,
}

/// The state of the pane.
#[derive(Debug)]
pub struct App {
    pub env: Env,
    repo: Option<PathBuf>,
    pub keymap: Keymap,
    pub root: Option<RepoRoot>,
    dir: Option<PathBuf>,
    pub meta: Meta,
    pub review: Review,
    /// The length of `review.jsonl` when it was last read.
    seen_len: u64,
    pub diff: Option<Diff>,
    pub view: View,
    pub screen: Screen,
    /// Why the last reload failed. The previous diff stays on screen.
    pub status: Option<String>,
    /// Shown on one line under the status line until the next action.
    pub warnings: Vec<Warning>,
    pane_saved: bool,
    pub quit: bool,
}

impl App {
    /// Read the keymap. Nothing else is read until `load`.
    pub fn new(env: Env, repo: Option<PathBuf>) -> Self {
        let config = env
            .get("HERDR_PLUGIN_CONFIG_DIR")
            .map(|dir| Path::new(dir).join("config.toml"));
        let keymap = Keymap::load(config.as_deref());
        Self {
            warnings: keymap.warnings.clone(),
            env,
            repo,
            keymap,
            root: None,
            dir: None,
            meta: Meta::default(),
            review: Review::default(),
            seen_len: 0,
            diff: None,
            view: View::default(),
            screen: Screen::Message("loading".to_owned()),
            status: None,
            pane_saved: false,
            quit: false,
        }
    }

    fn warn(&mut self, warning: Warning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    /// Find the root and the state directory, and read the meta file, once.
    fn open(&mut self, git: &mut Git) -> Result<(RepoRoot, PathBuf), String> {
        if let (Some(root), Some(dir)) = (&self.root, &self.dir) {
            return Ok((root.clone(), dir.clone()));
        }
        let mut text =
            |args: &[String]| git(args).map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let root = RepoRoot::resolve(self.repo.as_deref(), &self.env.cwd, &mut text)
            .map_err(|error| error.to_string())?;
        let base = self
            .env
            .state_base()
            .ok_or("no place to keep the review: HOME is not set")?;
        let dir = locate(&base, root.path());
        let (meta, warning) = crate::meta::load(&dir);
        self.meta = meta;
        self.warnings.extend(warning);
        self.root = Some(root.clone());
        self.dir = Some(dir.clone());
        Ok((root, dir))
    }

    /// Read `review.jsonl` and fold it.
    fn read_store(&mut self, dir: &Path) -> Result<(), String> {
        let len = log_len(dir).map_err(|error| error.to_string())?;
        let review = read(dir).map_err(|error| error.to_string())?;
        if review.skipped_lines > 0 {
            self.warn(Warning::SkippedLine(review.skipped_lines));
        }
        self.review = review;
        self.seen_len = len;
        Ok(())
    }

    fn requested_spec(&self) -> Spec {
        self.meta.spec.clone().unwrap_or(Spec::WorkTree)
    }

    fn read_diff(&mut self, root: &RepoRoot, git: &mut Git) -> Result<(), String> {
        let spec = self.requested_spec();
        let diff = load(root.path(), &spec, git).map_err(|error| error.to_string())?;
        let anchor = self.diff.as_ref().and_then(|old| self.view.anchor(old));
        self.view.rebuild(&diff, &self.review, anchor);
        self.diff = Some(diff);
        Ok(())
    }

    /// Start-up and reload: the root, the store, the diff. A failure with no diff on screen is the
    /// message screen. With a diff on screen it is the status line, and the diff stays.
    pub fn load(&mut self, git: &mut Git) {
        let loaded = self.open(git).and_then(|(root, dir)| {
            self.read_store(&dir)?;
            self.read_diff(&root, git)?;
            self.save_pane(&root, &dir);
            Ok(())
        });
        match loaded {
            Ok(()) => {
                self.screen = Screen::Review;
                self.status = None;
            }
            Err(message) if self.diff.is_some() => self.status = Some(message),
            Err(message) => self.screen = Screen::Message(message),
        }
    }

    /// Record this pane in `meta.json` once, so `open` can focus it instead of splitting again.
    fn save_pane(&mut self, root: &RepoRoot, dir: &Path) {
        if std::mem::replace(&mut self.pane_saved, true) {
            return;
        }
        let Some(pane) = self.env.get("HERDR_PANE_ID").and_then(PaneId::parse) else {
            return;
        };
        let saved = save(dir, root.path(), |meta| meta.review_pane = Some(pane));
        if let Err(error) = saved {
            self.warn(Warning::Config(format!(
                "could not save meta.json: {error}"
            )));
        }
    }

    /// Once per tick: when the log changed, read it and reload the diff, since an agent replying
    /// is the moment its fix landed. A failed read is a warning and the next tick tries again.
    pub fn check_store(&mut self, git: &mut Git) {
        let (Screen::Review, Some(dir), Some(root)) =
            (&self.screen, self.dir.clone(), self.root.clone())
        else {
            return;
        };
        match log_len(&dir) {
            Ok(len) if len == self.seen_len => return,
            Ok(_) => {}
            Err(error) => return self.warn(Warning::Config(error.to_string())),
        }
        match self.read_store(&dir) {
            Ok(()) => {
                if let Err(message) = self.read_diff(&root, git) {
                    self.status = Some(message);
                }
            }
            Err(message) => self.warn(Warning::Config(message)),
        }
    }

    /// Apply a key's action. Any action clears the warning line and the last reload error.
    pub fn handle(&mut self, action: Action, git: &mut Git) {
        self.warnings.clear();
        self.status = None;
        match action {
            Action::Quit => self.quit = true,
            Action::Reload => self.load(git),
            Action::SwitchSpec => self.switch_spec(git),
            _ => {
                self.view.apply(action);
            }
        }
    }

    /// Show the other diff: the working tree against `HEAD`, or against the base. The choice is
    /// kept in `meta.json`. With no base to compare with, the status line says so.
    fn switch_spec(&mut self, git: &mut Git) {
        let (Some(root), Some(dir)) = (self.root.clone(), self.dir.clone()) else {
            return;
        };
        let (spec, base) = match self.requested_spec() {
            Spec::Branch { base } => (Spec::WorkTree, Some(base)),
            Spec::WorkTree => {
                let base = match self.meta.base.clone() {
                    Some(base) => base,
                    None => match default_base(root.path(), &mut *git) {
                        Ok(base) => base,
                        Err(error) => {
                            self.status = Some(error.to_string());
                            return;
                        }
                    },
                };
                (Spec::Branch { base: base.clone() }, Some(base))
            }
        };
        let saved = save(&dir, root.path(), |meta| {
            meta.spec = Some(spec.clone());
            meta.base.clone_from(&base);
        });
        if let Err(error) = saved {
            self.warn(Warning::Config(format!(
                "could not save meta.json: {error}"
            )));
        }
        self.meta.spec = Some(spec);
        self.meta.base = base;
        if let Err(message) = self.read_diff(&root, git) {
            self.status = Some(message);
        }
    }

    /// The status line: which diff is shown, the target agent, and the keys that matter most.
    fn status_line(&self) -> String {
        let spec = self
            .diff
            .as_ref()
            .map_or_else(|| self.requested_spec(), |diff| diff.spec.clone());
        let spec = match spec {
            Spec::Branch { base } => format!("vs {base}"),
            Spec::WorkTree => "working tree".to_owned(),
        };
        let target = self.meta.target.as_ref().map_or_else(
            || "no agent".to_owned(),
            |target| format!("> {} {}", target.agent, target.pane),
        );
        let hints = [
            (Action::SwitchPanel, "panel"),
            (Action::SwitchSpec, "spec"),
            (Action::Reload, "reload"),
            (Action::Help, "help"),
            (Action::Quit, "quit"),
        ]
        .map(|(action, what)| format!("{} {what}", self.keymap.label(action)))
        .join("  ");
        format!("{spec}  {target}   {hints}")
    }
}

/// Draw the pane. Every string that came from the store or from `git` passes through
/// `sanitize_terminal_text` before it reaches the buffer.
pub fn render(frame: &mut Frame, app: &App) {
    let [body, warnings, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    match &app.screen {
        Screen::Message(message) => {
            let hint = format!(
                "{} reload, {} quit",
                app.keymap.label(Action::Reload),
                app.keymap.label(Action::Quit)
            );
            let text = Text::from(vec![
                Line::from(sanitize_terminal_text(message)),
                Line::default(),
                Line::styled(hint, Style::new().add_modifier(Modifier::DIM)),
            ]);
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), body);
        }
        Screen::Review => {
            if let Some(diff) = &app.diff {
                draw(frame, &app.view, diff, &app.keymap);
            }
        }
    }
    let width = usize::from(frame.area().width);
    let notices = app
        .diff
        .iter()
        .flat_map(|diff| &diff.notices)
        .map(ToString::to_string);
    let warning = app
        .warnings
        .iter()
        .map(ToString::to_string)
        .chain(notices)
        .collect::<Vec<_>>()
        .join("; ");
    frame.render_widget(
        Paragraph::new(truncate_to_width(&sanitize_terminal_text(&warning), width))
            .style(Style::new().add_modifier(Modifier::BOLD)),
        warnings,
    );
    let line = app
        .status
        .as_deref()
        .map_or_else(|| app.status_line(), ToOwned::to_owned);
    frame.render_widget(
        Paragraph::new(truncate_to_width(&sanitize_terminal_text(&line), width))
            .style(Style::new().add_modifier(Modifier::REVERSED)),
        status,
    );
}

/// Why the loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Quit,
    /// SIGTERM or SIGHUP.
    Terminated,
    /// The terminal failed.
    Io,
}

/// Draw, wait up to one tick for an event, apply it, check the store. `poll` returns the next event
/// or `None` when the tick passed, and `terminated` is the signal flag.
pub fn run_loop<B: Backend>(
    app: &mut App,
    terminal: &mut Terminal<B>,
    git: &mut Git,
    mut poll: impl FnMut(Duration) -> io::Result<Option<Event>>,
    terminated: impl Fn() -> bool,
) -> Exit {
    loop {
        if terminated() {
            return Exit::Terminated;
        }
        if let Ok(size) = terminal.size() {
            app.view.resize(Rect::new(0, 0, size.width, size.height));
        }
        if terminal.draw(|frame| render(frame, app)).is_err() {
            return Exit::Io;
        }
        match poll(TICK) {
            Err(_) => return Exit::Io,
            Ok(Some(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                if app.view.help {
                    app.view.help = false;
                } else if let Some(action) = app.keymap.action(&key) {
                    app.handle(action, git);
                }
            }
            Ok(Some(Event::Mouse(mouse))) if app.screen == Screen::Review && !app.view.help => {
                app.view.mouse(mouse);
            }
            Ok(Some(Event::FocusGained)) => app.load(git),
            Ok(_) => {}
        }
        if app.quit {
            return Exit::Quit;
        }
        app.check_store(git);
    }
}

/// Runs `restore` once when dropped, also while a panic unwinds.
struct Guard<F: FnMut()>(Option<F>);

impl<F: FnMut()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(mut restore) = self.0.take() {
            restore();
        }
    }
}

/// Raw mode off, main screen back, cursor shown, mouse and focus reports off.
fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableFocusChange,
        LeaveAlternateScreen,
        ratatui::crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}

/// Restore the terminal before the panic message prints, so it is not drawn on the alternate
/// screen and lost.
fn restoring_panic_hook(restore: fn()) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

fn enter_terminal() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange
    )
}

/// The `tui` entry point: the process owns the terminal until it returns.
pub fn run(env: &Env, repo: Option<&Path>) -> ExitCode {
    let termination = Termination::install();
    restoring_panic_hook(restore_terminal);
    let _guard = Guard(Some(restore_terminal));
    if enter_terminal().is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(mut terminal) = Terminal::new(CrosstermBackend::new(io::stdout())) else {
        return ExitCode::FAILURE;
    };
    let mut app = App::new(env.clone(), repo.map(Path::to_path_buf));
    let mut git = run_git_bytes;
    app.load(&mut git);
    let poll = |timeout| {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    };
    match run_loop(&mut app, &mut terminal, &mut git, poll, || {
        termination.requested()
    }) {
        Exit::Quit | Exit::Terminated => ExitCode::SUCCESS,
        Exit::Io => ExitCode::FAILURE,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::os::unix::fs::PermissionsExt;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, Ordering};

    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::diff::Change;
    use crate::store::{Add, Author, CommentId, Event as LogEvent, Kind, RelPath, Side, state_dir};

    const PATCH: &[u8] =
        b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

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
            Self {
                root: root.canonicalize().unwrap(),
                home,
                env,
                repo_ok: Cell::new(true),
                diff_error: RefCell::new(None),
                merge_base_missing: Cell::new(false),
                no_refs: Cell::new(false),
                patch: RefCell::new(PATCH.to_vec()),
                diffs: Cell::new(0),
            }
        }

        fn dir(&self) -> PathBuf {
            state_dir(&self.env.state_base().unwrap(), &self.root)
        }

        fn app(&self) -> App {
            App::new(self.env.clone(), None)
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
                ["rev-parse", "--verify", "--quiet", name]
                    if self.no_refs.get() && *name != "HEAD" =>
                {
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
        Terminal::new(TestBackend::new(80, 8)).unwrap()
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
        assert!(screen_of(&app).contains("working tree  no agent"));
        let target = crate::meta::Target {
            pane: PaneId::parse("w1:p2").unwrap(),
            terminal: crate::store::TerminalId::parse("term_1").unwrap(),
            agent: "claude".into(),
        };
        save(&fixture.dir(), &fixture.root, |meta| {
            meta.target = Some(target);
        })
        .unwrap();
        let app = fixture.started();
        assert!(screen_of(&app).contains("working tree  > claude w1:p2"));
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
            fixture.with_git(|git| {
                run_loop(&mut app, &mut terminal(), git, |_| panic!("boom"), || false)
            })
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
        let click = Event::Mouse(event::MouseEvent {
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
        assert!(screen_of(&app).contains("working tree"));
        drive(&fixture, &mut app, vec![Some(key('b'))], |_| {});
        assert_eq!(
            app.diff.as_ref().unwrap().spec,
            Spec::Branch {
                base: "origin/HEAD".into()
            }
        );
        assert!(screen_of(&app).contains("vs origin/HEAD"));
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
}
