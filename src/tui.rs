//! The review pane: the terminal guard, the loop, and what start-up reads.
//!
//! Everything below `run` takes its terminal, its input, its `git` and its clock as parameters, so
//! tests drive it with a `TestBackend` and a list of events.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    KeyCode, KeyEvent, KeyEventKind, MouseEvent,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::actions;
use crate::cards::{Look, place};
use crate::comment::CommandError;
use crate::diff::{Diff, GitError, RepoRoot, default_base, load, run_git_bytes};
use crate::editor::{Editor, Outcome};
use crate::env::Env;
use crate::herdr::run_herdr_output;
use crate::keymap::{Action, Keymap};
use crate::meta::{Meta, Target, locate, save};
use crate::send::{Scope, SendError, TargetError, resolve_target, save_target, send};
use crate::store::{
    Anchor, AnchorTarget, Comment, CommentId, PaneId, Review, Spec, StoreError, Thread, Warning,
    WriteError, archive, log_len, read,
};
use crate::syntax::Cache;
use crate::termination::Termination;
use crate::theme::Theme;
use crate::view::{
    View, areas, draw, editor_rect, key_style, note_box, popup_block, sidebar_config,
};
use crate::width::{string_width, truncate_to_width};

/// How long the loop waits for a key before it checks the store and the signal flag.
pub(crate) const TICK: Duration = Duration::from_millis(250);

/// `git` as the pane sees it: the arguments in, the stdout bytes out.
pub type Git<'a> = dyn FnMut(&[String]) -> Result<Vec<u8>, GitError> + 'a;

/// Remove terminal control characters while retaining useful whitespace. A tab becomes four
/// spaces. ESC and the C1 controls go, so a file name cannot drive the terminal.
pub fn sanitize_terminal_text(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '\t' {
            clean.push_str("    ");
        } else if !((character <= '\u{0008}')
            || matches!(character, '\u{000b}' | '\u{000c}')
            || ('\u{000e}'..='\u{001f}').contains(&character)
            || ('\u{007f}'..='\u{009f}').contains(&character))
        {
            clean.push(character);
        }
    }
    clean
}

/// What fills the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Start-up failed. The message says why, and `reload` and `quit` still work.
    Message(String),
    Review,
}

/// What the editor is writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    /// A new thread at an anchor captured when the key was pressed.
    Comment(Anchor),
    /// A reply to the thread with this root.
    Reply(CommentId),
    /// New text for this comment of the user's.
    Edit(CommentId),
}

/// The editor, and what its text becomes.
#[derive(Debug)]
pub struct Compose {
    pub draft: Draft,
    pub editor: Editor,
    /// What the top border of the editor says first: `Draft note - `, `Reply to u3`, `Edit u3`.
    title: String,
    /// Where a new comment points, after the title. It is cut from the left when it is long.
    place: String,
}

/// How the pane calls the Herdr CLI: the arguments in, stdout or stderr out. Tests replace it.
pub struct HerdrCall(Box<Call>);

type Call = dyn FnMut(&[String]) -> Result<String, String>;

impl HerdrCall {
    pub fn new(call: impl FnMut(&[String]) -> Result<String, String> + 'static) -> Self {
        Self(Box::new(call))
    }

    pub(crate) fn call(&mut self, args: &[String]) -> Result<String, String> {
        (self.0)(args)
    }
}

impl std::fmt::Debug for HerdrCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HerdrCall")
    }
}

/// A send waiting for the loop to draw "sending" and then make the Herdr call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    scope: Scope,
    /// The quit prompt chose "send": leave once the send went through.
    quit_after: bool,
}

/// A question the pane asks before it goes on. It takes every key until it is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    /// Several agents match. The choice is saved, and the send that asked runs again.
    Pick {
        found: Vec<Target>,
        selected: usize,
        request: Request,
    },
    /// Quitting with unsent comments: send, keep, or stay.
    Quit,
    /// Archiving the resolved threads: archive, or stay.
    Archive,
}

/// What a message on the status line is: something that happened, or something that went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Notice,
    Failure,
}

/// The keys the status line names, in the order they are drawn. They drop off from the left when
/// the line is narrow.
const FOOTER: [(Action, &str); 6] = [
    (Action::ToggleSidebar, "sidebar"),
    (Action::Send, "send"),
    (Action::Resolve, "resolve"),
    (Action::Reload, "reload"),
    (Action::Help, "help"),
    (Action::Quit, "quit"),
];

/// The time to write into an event, as RFC 3339.
fn real_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Why a write did not happen, in words for the status line.
fn failure(error: &WriteError<CommandError>) -> String {
    match error {
        WriteError::Store(StoreError::Busy) => "review is busy, press again".to_owned(),
        WriteError::Store(StoreError::Io { path, kind }) => {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            format!("could not write {name}: {kind}")
        }
        WriteError::Build(error) => error.to_string(),
    }
}

/// The state of the pane.
#[derive(Debug)]
pub struct App {
    pub env: Env,
    repo: Option<PathBuf>,
    pub keymap: Keymap,
    pub theme: Theme,
    /// The syntax tokens of the files that have been on screen. A reload against the same
    /// revision keeps the tokens of the files whose hunks did not change.
    pub syntax: Cache,
    pub root: Option<RepoRoot>,
    dir: Option<PathBuf>,
    pub meta: Meta,
    pub review: Review,
    /// The length of `review.jsonl` when it was last read.
    seen_len: u64,
    pub diff: Option<Diff>,
    pub view: View,
    pub screen: Screen,
    /// What the last action said, in place of the state on the status line.
    pub status: Option<(Tone, String)>,
    /// Shown on one line under the status line until the next action.
    pub warnings: Vec<Warning>,
    pane_saved: bool,
    /// The agent the next send goes to, as last resolved. `None` shows as "no agent".
    pub target: Option<Target>,
    pub herdr: HerdrCall,
    pub prompt: Option<Prompt>,
    pending: Option<Request>,
    pub quit: bool,
    /// The editor, while one is open. The diff is not reloaded while it is.
    pub compose: Option<Compose>,
    /// The clock, which tests replace.
    pub now: fn() -> String,
}

impl App {
    /// Read the keymap, the theme and whether the sidebar starts open. Nothing else is read until
    /// `load`.
    pub fn new(env: Env, repo: Option<PathBuf>) -> Self {
        let config = env
            .get("HERDR_PLUGIN_CONFIG_DIR")
            .map(|dir| Path::new(dir).join("config.toml"));
        let keymap = Keymap::load(config.as_deref());
        let (theme, theme_warnings) = Theme::load(config.as_deref());
        let (sidebar, sidebar_warnings) = sidebar_config(config.as_deref());
        let mut warnings = keymap.warnings.clone();
        warnings.extend(theme_warnings);
        warnings.extend(sidebar_warnings);
        let mut app = Self {
            warnings,
            env,
            repo,
            keymap,
            theme,
            syntax: Cache::default(),
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
            target: None,
            herdr: HerdrCall::new(run_herdr_output),
            prompt: None,
            pending: None,
            quit: false,
            compose: None,
            now: real_now,
        };
        app.view.sidebar = sidebar.open;
        app.view.icons = sidebar.icons;
        app
    }

    /// Say on the status line that something happened.
    fn notice(&mut self, text: impl Into<String>) {
        self.status = Some((Tone::Notice, text.into()));
    }

    /// Say on the status line that something went wrong.
    fn fail(&mut self, text: impl Into<String>) {
        self.status = Some((Tone::Failure, text.into()));
    }

    /// The words of the message on the status line.
    pub fn message(&self) -> Option<&str> {
        self.status.as_ref().map(|(_, text)| text.as_str())
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
        let spot = self.diff.as_ref().and_then(|old| self.view.spot(old));
        let now = (self.now)();
        let look = Look {
            theme: &self.theme,
            keymap: &self.keymap,
            now: &now,
        };
        self.view.rebuild(&diff, &self.review, spot, &look);
        match &self.diff {
            Some(old) if old.rev == diff.rev => {
                let new = diff
                    .files
                    .iter()
                    .map(|file| (file.path.as_str(), file))
                    .collect::<HashMap<_, _>>();
                let unchanged = old
                    .files
                    .iter()
                    .filter(|file| new.get(file.path.as_str()) == Some(file))
                    .map(|file| file.path.as_str())
                    .collect::<HashSet<_>>();
                self.syntax.retain(|path| unchanged.contains(path));
            }
            _ => self.syntax.clear(),
        }
        self.diff = Some(diff);
        Ok(())
    }

    /// Lay the diff on screen out again, for threads that changed while the diff could not be
    /// reloaded, or for a pane that was resized.
    fn rebuild_view(&mut self) {
        if let Some(diff) = &self.diff {
            let spot = self.view.spot(diff);
            let now = (self.now)();
            let look = Look {
                theme: &self.theme,
                keymap: &self.keymap,
                now: &now,
            };
            self.view.rebuild(diff, &self.review, spot, &look);
        }
    }

    /// Highlight the files that have a row on screen and were not highlighted yet. It runs before
    /// each frame, so a file is read when it scrolls into view and not before.
    pub fn highlight(&mut self, git: &mut Git) {
        let (Some(diff), Some(root)) = (&self.diff, &self.root) else {
            return;
        };
        for file in self.view.visible_files() {
            if let Some(file) = diff.files.get(file) {
                self.syntax.ensure(root.path(), &diff.rev, file, git);
            }
        }
    }

    /// Tell the view how big the pane is. Cards wrap to the stream's width, so a new width lays
    /// them out again.
    pub fn resize(&mut self, area: Rect) {
        self.view.resize(area);
        if self.view.needs_rebuild() {
            self.rebuild_view();
        }
    }

    /// Start-up and reload: the root, the store, the diff. A failure with no diff on screen is the
    /// message screen. With a diff on screen it is the status line, and the diff stays.
    pub fn load(&mut self, git: &mut Git) {
        let loaded = self.open(git).and_then(|(root, dir)| {
            self.read_store(&dir)?;
            self.read_diff(&root, git)?;
            if !self.pane_saved {
                self.refresh_target(&root, &dir);
            }
            self.save_pane(&root, &dir);
            Ok(())
        });
        match loaded {
            Ok(()) => {
                self.screen = Screen::Review;
                self.status = None;
            }
            Err(message) if self.diff.is_some() => {
                self.fail(message);
                self.rebuild_view();
            }
            Err(message) => self.screen = Screen::Message(message),
        }
    }

    /// Find the agent for the status line, once at start and after each send. Whatever it finds is
    /// saved, so the `send` action, which never has this pane's environment, reaches the same agent.
    fn refresh_target(&mut self, root: &RepoRoot, dir: &Path) {
        let found = resolve_target(
            &self.env,
            &self.meta,
            |args| self.herdr.call(args),
            root.path(),
        );
        self.target = found.ok();
        if let Some(target) = self.target.clone() {
            if save_target(dir, root.path(), &self.meta, &target).is_err() {
                self.warn(Warning::TargetNotSaved);
            }
            self.meta.target = Some(target);
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
    /// is the moment its fix landed. It waits while the editor is open or a range is being
    /// selected. A failed read is a warning and the next tick tries again. True when it read the
    /// log or warned, whether or not the diff reload worked, since the screen may differ.
    pub fn check_store(&mut self, git: &mut Git) -> bool {
        if self.view.select.is_some() {
            return false;
        }
        let (Screen::Review, Some(dir), Some(root), None) = (
            &self.screen,
            self.dir.clone(),
            self.root.clone(),
            &self.compose,
        ) else {
            return false;
        };
        match log_len(&dir) {
            Ok(len) if len == self.seen_len => return false,
            Ok(_) => {}
            Err(error) => {
                self.warn(Warning::Config(error.to_string()));
                return true;
            }
        }
        match self.read_store(&dir) {
            Ok(()) => {
                if let Err(message) = self.read_diff(&root, git) {
                    self.fail(message);
                    self.rebuild_view();
                }
            }
            Err(message) => self.warn(Warning::Config(message)),
        }
        true
    }

    /// Apply a key's action. Any action clears the warning line and the last reload error.
    pub fn handle(&mut self, action: Action, git: &mut Git) {
        self.warnings.clear();
        self.status = None;
        match action {
            Action::Quit => self.quit_or_ask(),
            Action::Send => self.request(Scope::Unsent, false),
            Action::Resend => self.resend(),
            Action::Reload => self.load(git),
            Action::SwitchSpec => self.switch_spec(git),
            Action::ToggleLayout => {
                self.view.toggle_layout();
                self.rebuild_view();
            }
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::Comment => self.start_comment(),
            Action::SelectRange => self.view.toggle_select(),
            Action::Reply => self.start_reply(),
            Action::Edit => self.start_edit(),
            Action::Delete => self.delete(),
            Action::Resolve => self.toggle_resolve(),
            Action::Archive => self.ask_archive(),
            _ => {
                self.view.apply(action);
            }
        }
    }

    /// `toggle_sidebar`: show or hide the sidebar and lay the stream out at its new width. A pane
    /// too narrow for a sidebar keeps the choice for when it is wider, and says so either way,
    /// since nothing on screen moves.
    fn toggle_sidebar(&mut self) {
        self.view.toggle_sidebar();
        if self.view.needs_rebuild() {
            self.rebuild_view();
        }
        if self.view.too_narrow() {
            self.notice("the pane is too narrow to show the sidebar");
        }
    }

    fn unsent(&self) -> usize {
        self.review.threads.iter().filter(|t| t.unsent).count()
    }

    /// `quit`: leave, or with unsent comments ask what to do with them first.
    fn quit_or_ask(&mut self) {
        if self.unsent() == 0 {
            self.quit = true;
        } else {
            self.prompt = Some(Prompt::Quit);
        }
    }

    /// Queue a send. The loop draws "sending" and then runs it, because the Herdr call blocks.
    fn request(&mut self, scope: Scope, quit_after: bool) {
        if self.dir.is_some() {
            self.notice("sending");
            self.pending = Some(Request { scope, quit_after });
        }
    }

    /// `resend`: send the thread under the cursor again, with the text it has now.
    fn resend(&mut self) {
        let root = self
            .view
            .focused()
            .and_then(|thread| self.view.thread_id(thread))
            .cloned();
        match root.and_then(|root| self.review.thread(&root)) {
            None => self.notice("no thread here to resend"),
            Some(thread) if !thread.is_open() => {
                self.notice("a resolved thread is not resent, reopen it first");
            }
            Some(thread) => {
                let scope = Scope::Thread(thread.root.id.clone());
                self.request(scope, false);
            }
        }
    }

    /// Run the queued send. `draw` shows the frame first, which says "sending".
    pub fn run_pending(&mut self, draw: impl FnOnce(&Self)) {
        let Some(request) = self.pending.take() else {
            return;
        };
        draw(self);
        self.run_send(request);
    }

    fn run_send(&mut self, request: Request) {
        let (Some(root), Some(dir)) = (self.root.clone(), self.dir.clone()) else {
            return;
        };
        let now = (self.now)();
        let result = send(&dir, root.path(), &self.env, &now, &request.scope, |args| {
            self.herdr.call(args)
        });
        self.status = None;
        match result {
            Ok(sent) => {
                for warning in sent.warnings {
                    self.warn(warning);
                }
                if let Some(target) = sent.target {
                    self.meta.target = Some(target.clone());
                    self.target = Some(target);
                }
                self.notice(sent.outcome.to_string());
                self.refold();
                self.quit |= request.quit_after;
            }
            Err(SendError::Target(TargetError::Ambiguous(found))) => {
                self.prompt = Some(Prompt::Pick {
                    found,
                    selected: 0,
                    request,
                });
            }
            Err(error) => {
                let message = error.to_string();
                if matches!(error, SendError::Target(_)) {
                    self.target = None;
                }
                let args = [
                    "notification",
                    "show",
                    "review: not sent",
                    "--body",
                    &message,
                ];
                let _ = self.herdr.call(&args.map(str::to_owned));
                self.fail(message);
            }
        }
    }

    /// A key while a prompt is open.
    fn prompt_key(&mut self, key: KeyEvent) {
        match (self.prompt.take(), key.code) {
            (Some(Prompt::Quit), KeyCode::Char('s')) => self.request(Scope::Unsent, true),
            (Some(Prompt::Quit), KeyCode::Char('k')) => self.quit = true,
            (Some(Prompt::Quit | Prompt::Archive), KeyCode::Esc | KeyCode::Char('n')) => {}
            (Some(Prompt::Archive), KeyCode::Char('y')) => self.run_archive(),
            (
                Some(Prompt::Pick {
                    found,
                    selected,
                    request,
                }),
                code,
            ) => match code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.choose(&found, selected, request),
                KeyCode::Up | KeyCode::Char('k') => {
                    self.prompt = Some(Prompt::Pick {
                        selected: selected.saturating_sub(1),
                        found,
                        request,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.prompt = Some(Prompt::Pick {
                        selected: (selected + 1).min(found.len().saturating_sub(1)),
                        found,
                        request,
                    });
                }
                _ => {
                    self.prompt = Some(Prompt::Pick {
                        found,
                        selected,
                        request,
                    });
                }
            },
            (prompt, _) => self.prompt = prompt,
        }
    }

    /// Remember the agent the user picked, then run the send that asked.
    fn choose(&mut self, found: &[Target], selected: usize, request: Request) {
        let (Some(root), Some(dir), Some(chosen)) =
            (self.root.clone(), self.dir.clone(), found.get(selected))
        else {
            return;
        };
        if save_target(&dir, root.path(), &self.meta, chosen).is_err() {
            self.warn(Warning::TargetNotSaved);
        }
        self.meta.target = Some(chosen.clone());
        self.target = Some(chosen.clone());
        self.request(request.scope, request.quit_after);
    }

    /// Read the log again and lay the stream out, after this pane wrote to it.
    fn refold(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        match self.read_store(&dir) {
            Ok(()) => self.rebuild_view(),
            Err(message) => self.warn(Warning::Config(message)),
        }
    }

    /// The thread the cursor is on, and the comment of it, looked up in the review.
    fn focused_comment(&self) -> Option<(&Thread, &Comment)> {
        let (thread, comment) = self.view.focused_comment()?;
        let thread = self.review.thread(self.view.thread_id(thread)?)?;
        Some((thread, thread.comments().nth(comment)?))
    }

    /// `comment`: open the editor for a new thread. What it points at is read now, from the rows
    /// under the cursor or the selected range, and kept until the text is saved.
    fn start_comment(&mut self) {
        let Some(diff) = &self.diff else { return };
        match self.view.capture(diff) {
            Ok(anchor) => {
                self.view.select = None;
                self.compose = Some(Compose {
                    title: "Draft note - ".to_owned(),
                    place: place(&anchor),
                    draft: Draft::Comment(anchor),
                    editor: Editor::default(),
                });
            }
            Err(why) => self.notice(why),
        }
    }

    /// `reply`: open the editor for a reply to the thread under the cursor.
    fn start_reply(&mut self) {
        let root = self
            .view
            .focused()
            .and_then(|thread| self.view.thread_id(thread))
            .cloned();
        match root {
            Some(root) => {
                let title = format!("Reply to {root}");
                self.compose = Some(Compose {
                    draft: Draft::Reply(root),
                    editor: Editor::default(),
                    title,
                    place: String::new(),
                });
            }
            None => self.notice("no thread here to reply to"),
        }
    }

    /// The user's own comment under the cursor, or why there is none.
    fn own_comment(&self) -> Result<(CommentId, String), String> {
        let Some((_, comment)) = self.focused_comment() else {
            return Err("no comment here".to_owned());
        };
        if comment.author.is_user() {
            Ok((comment.id.clone(), comment.body.clone()))
        } else {
            Err(format!(
                "{} is the agent's, you can only change your own comments",
                comment.id
            ))
        }
    }

    /// `edit`: open the editor on the text of one of the user's comments.
    fn start_edit(&mut self) {
        match self.own_comment() {
            Ok((id, body)) => {
                let title = format!("Edit {id}");
                self.compose = Some(Compose {
                    draft: Draft::Edit(id),
                    editor: Editor::with_text(&body),
                    title,
                    place: String::new(),
                });
            }
            Err(why) => self.notice(why),
        }
    }

    /// `delete`: remove one of the user's comments. A root takes its thread with it.
    fn delete(&mut self) {
        let (id, _) = match self.own_comment() {
            Ok(own) => own,
            Err(why) => return self.notice(why),
        };
        let Some(dir) = self.dir.clone() else { return };
        match actions::delete(&dir, &(self.now)(), &id) {
            Ok(()) => {
                self.notice(format!("deleted {id}"));
                self.refold();
            }
            Err(error) => self.fail(failure(&error)),
        }
    }

    /// `resolve`: resolve the thread under the cursor, or reopen it when it is resolved.
    fn toggle_resolve(&mut self) {
        let root = self
            .view
            .focused()
            .and_then(|thread| self.view.thread_id(thread))
            .cloned();
        let (Some(root), Some(dir)) = (root, self.dir.clone()) else {
            self.notice("no thread here");
            return;
        };
        match actions::toggle(&dir, &(self.now)(), &root) {
            Ok(resolved) => {
                let what = if resolved { "resolved" } else { "reopened" };
                self.notice(format!("{what} {root}"));
                self.refold();
            }
            Err(error) => self.fail(failure(&error)),
        }
    }

    /// How many resolved threads `archive` takes, and how many of them were never sent.
    fn archivable(&self) -> (usize, usize) {
        let taken = self.review.threads.iter().filter(|t| t.archivable());
        (taken.clone().count(), taken.filter(|t| t.unsent).count())
    }

    /// `archive`: ask before the resolved threads leave the pane. With none, say so.
    fn ask_archive(&mut self) {
        if self.archivable().0 == 0 {
            self.notice("nothing to archive");
        } else {
            self.prompt = Some(Prompt::Archive);
        }
    }

    /// The archive prompt said yes: move the resolved threads to `archive.jsonl`. The store reads
    /// the log again under its lock, so the count is what it moved and not what the prompt said.
    fn run_archive(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        match archive(&dir, &(self.now)()) {
            Ok(done) if done.threads == 0 => self.notice("nothing to archive"),
            Ok(done) => {
                self.notice(format!("archived {}", done.threads));
                self.refold();
            }
            Err(error) => self.fail(failure(&WriteError::Store(error))),
        }
    }

    /// Write what the editor holds. When it is written the editor closes and the cursor goes to the
    /// thread. When it is not, the editor stays open with its text and says why.
    fn commit(&mut self) {
        let (Some(compose), Some(dir)) = (&self.compose, self.dir.clone()) else {
            return;
        };
        let (text, now) = (compose.editor.text(), (self.now)());
        let written = match &compose.draft {
            Draft::Comment(anchor) => actions::comment(&dir, &now, anchor, &text).map(Some),
            Draft::Reply(root) => {
                actions::reply(&dir, &now, root, &text).map(|_| Some(root.clone()))
            }
            Draft::Edit(id) => actions::edit(&dir, &now, id, &text).map(|()| None),
        };
        match written {
            Ok(focus) => {
                self.compose = None;
                self.refold();
                if let Some(id) = focus {
                    self.view.focus_thread(&id);
                }
            }
            Err(error) => {
                let message = failure(&error);
                self.fail(message.clone());
                if let Some(compose) = &mut self.compose {
                    compose.editor.fail(message);
                }
            }
        }
    }

    /// A thread an agent resolved is new until the cursor reaches it. Then one `seen` event is
    /// written, so the marker stays cleared after a restart. A failed write is a warning, and the
    /// next key tries again.
    pub fn mark_seen(&mut self) {
        if self.compose.is_some() {
            return;
        }
        let root = self
            .view
            .focused()
            .and_then(|thread| self.view.thread_id(thread))
            .cloned();
        let Some(root) = root.filter(|root| self.review.thread(root).is_some_and(|t| t.is_new))
        else {
            return;
        };
        let Some(dir) = self.dir.clone() else { return };
        match actions::seen(&dir, &(self.now)(), &root) {
            Ok(()) => self.refold(),
            Err(error) => self.warn(Warning::Config(failure(&error))),
        }
    }

    /// Apply a key press: to the editor when one is open, else to the help overlay or the keymap.
    pub fn key(&mut self, key: KeyEvent, git: &mut Git) {
        if self.compose.is_some() {
            self.compose_key(key);
        } else if self.prompt.is_some() {
            self.prompt_key(key);
        } else if self.view.help {
            self.view.help = false;
        } else if key.code == KeyCode::Esc && self.view.select.is_some() {
            self.warnings.clear();
            self.status = None;
            self.view.select = None;
        } else if let Some(action) = self.keymap.action(&key) {
            self.handle(action, git);
        }
    }

    /// A mouse event over the review. A click on the `[+]` of a line opens the comment editor
    /// there, as `comment` does.
    pub fn mouse(&mut self, mouse: MouseEvent) {
        if self.view.mouse(mouse) {
            self.warnings.clear();
            self.status = None;
            self.start_comment();
        }
    }

    /// A key while the editor is open.
    fn compose_key(&mut self, key: KeyEvent) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        match compose.editor.handle_key(key) {
            Outcome::Continue => {}
            Outcome::Cancel => self.compose = None,
            Outcome::Save(_) => self.commit(),
        }
    }

    /// The pane is going away with the editor open: write what was typed, so it is not lost.
    pub fn save_draft(&mut self) {
        if self
            .compose
            .as_ref()
            .is_some_and(|compose| !compose.editor.text().is_empty())
        {
            self.commit();
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
                            self.fail(error.to_string());
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
            self.fail(message);
        }
    }

    /// The left of the status line: the diff on screen as a chip, the target agent, and a chip
    /// with the number of unsent threads when there are any.
    fn state(&self) -> Vec<Span<'static>> {
        let theme = &self.theme;
        let chip = |text: String, bg| Span::styled(text, Style::new().fg(theme.base).bg(bg));
        let spec = self
            .diff
            .as_ref()
            .map_or_else(|| self.requested_spec(), |diff| diff.spec.clone());
        let spec = match spec {
            Spec::Branch { base } => format!("vs {base}"),
            Spec::WorkTree => "working tree".to_owned(),
        };
        let spec = sanitize_terminal_text(&spec).to_uppercase();
        let agent = self.target.as_ref().map_or_else(
            || Span::styled("\u{2192} no agent", Style::new().fg(theme.removed)),
            |target| {
                let text = format!("\u{2192} {} {}", target.agent, target.pane);
                Span::styled(sanitize_terminal_text(&text), Style::new().fg(theme.text))
            },
        );
        let mut spans = vec![chip(format!(" {spec} "), theme.accent), " ".into(), agent];
        let unsent = self.unsent();
        if unsent > 0 {
            spans.push("  ".into());
            spans.push(chip(format!(" {unsent} unsent "), theme.warning));
        }
        spans
    }

    /// The left of the status line in visual mode: the chip, then the place a comment would point
    /// at and how many lines it covers, or why it could not point anywhere.
    fn visual_state(&self) -> Vec<Span<'static>> {
        let theme = &self.theme;
        let mut spans = vec![Span::styled(
            " VISUAL ",
            Style::new().fg(theme.base).bg(theme.visual),
        )];
        match self.diff.as_ref().map(|diff| self.view.capture(diff)) {
            Some(Ok(anchor)) => {
                let lines = match &anchor.target {
                    AnchorTarget::File => None,
                    AnchorTarget::Line { .. } => Some(1),
                    AnchorTarget::Range { start, end, .. } => Some(end - start + 1),
                };
                let count = match lines {
                    None => String::new(),
                    Some(1) => " (1 line)".to_owned(),
                    Some(n) => format!(" ({n} lines)"),
                };
                spans.push(Span::styled(
                    format!(" {}{count}", place(&anchor)),
                    Style::new().fg(theme.text),
                ));
            }
            Some(Err(why)) => spans.push(Span::styled(
                format!(" {why}"),
                Style::new().fg(theme.removed),
            )),
            None => {}
        }
        spans
    }

    /// The status line, `width` cells wide: the state, or the message that takes its place, on the
    /// left and the keys against the right edge. Keys that do not fit beside the left part drop off
    /// from the left.
    fn status_line(&self, width: usize) -> Line<'static> {
        let theme = &self.theme;
        let spans = match &self.status {
            Some((tone, text)) => {
                let colour = match tone {
                    Tone::Notice => theme.warning,
                    Tone::Failure => theme.removed,
                };
                let text = format!(" {}", sanitize_terminal_text(text));
                vec![Span::styled(
                    truncate_to_width(&text, width),
                    Style::new().fg(colour),
                )]
            }
            None if self.view.select.is_some() => self.visual_state(),
            None => self.state(),
        };
        let label = |action| self.keymap.label(action);
        let keys = if self.view.select.is_some() {
            vec![
                (label(Action::Comment), "comment"),
                (format!("{}/esc", label(Action::SelectRange)), "cancel"),
            ]
        } else {
            FOOTER
                .iter()
                .map(|(action, what)| (label(*action), *what))
                .collect()
        };
        status_bar(spans, keys, width, theme)
    }
}

/// A status line `width` cells wide: `spans` on the left and `keys`, each a key and what it does,
/// against the right edge. Keys that do not fit beside the left part drop off from the left.
pub(crate) fn status_bar(
    mut spans: Vec<Span<'static>>,
    keys: Vec<(String, &str)>,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let used = spans
        .iter()
        .map(|span| string_width(&span.content))
        .sum::<usize>();
    let keys = keys
        .into_iter()
        .map(|(key, what)| {
            let key = sanitize_terminal_text(&key);
            (string_width(&key) + 1 + what.len(), key, what)
        })
        .collect::<Vec<_>>();
    let room = width.saturating_sub(used + 2);
    let fits = |from: usize| {
        let shown = keys.iter().skip(from);
        shown.clone().map(|key| key.0).sum::<usize>() + 2 * shown.count().saturating_sub(1) <= room
    };
    let Some(from) = (0..keys.len()).find(|from| fits(*from)) else {
        return Line::from(spans);
    };
    let keys_width = keys.iter().skip(from).map(|key| key.0).sum::<usize>()
        + 2 * (keys.len() - from).saturating_sub(1);
    spans.push(" ".repeat(width - used - keys_width).into());
    for (at, (_, key, what)) in keys.into_iter().enumerate().skip(from) {
        if at > from {
            spans.push("  ".into());
        }
        let bold = Style::new().fg(theme.accent).add_modifier(Modifier::BOLD);
        spans.push(Span::styled(key, bold));
        spans.push(Span::styled(format!(" {what}"), theme.dim()));
    }
    Line::from(spans)
}

/// The text of a screen that has nothing else to show, with the keys that still work under it.
pub(crate) fn draw_notice(
    frame: &mut Frame,
    area: Rect,
    text: &str,
    keymap: &Keymap,
    theme: &Theme,
) {
    let hint = format!(
        "{} reload, {} quit",
        keymap.label(Action::Reload),
        keymap.label(Action::Quit)
    );
    let text = Text::from(vec![
        Line::from(sanitize_terminal_text(text)),
        Line::default(),
        Line::styled(hint, theme.dim()),
    ]);
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
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
        Screen::Message(message) => draw_notice(frame, body, message, &app.keymap, &app.theme),
        Screen::Review => {
            // The rows a new comment points at, marked while its editor is open.
            let mark = match (&app.compose, &app.diff) {
                (Some(compose), Some(diff)) => Some(match &compose.draft {
                    Draft::Comment(anchor) => app.view.rows_of(diff, anchor),
                    Draft::Reply(_) | Draft::Edit(_) => None,
                }),
                _ => None,
            };
            if let Some(diff) = &app.diff {
                let rows = mark.flatten();
                draw(
                    frame,
                    &app.view,
                    diff,
                    &app.keymap,
                    &app.theme,
                    rows,
                    &app.syntax,
                );
            }
            if let (Some(compose), Some(rows)) = (&app.compose, mark) {
                draw_compose(frame, app, compose, rows);
            }
        }
    }
    if let Some(prompt) = &app.prompt {
        draw_prompt(frame, prompt, app);
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
    let bar = if app.view.select.is_some() {
        app.theme.selection
    } else {
        app.theme.header
    };
    frame.render_widget(
        Paragraph::new(app.status_line(width)).style(Style::new().fg(app.theme.text).bg(bar)),
        status,
    );
    app.theme.paint(frame.buffer_mut());
}

/// The editor, where `note_box` puts a note on its side of the diff, under the cursor or under the
/// last of `rows`, the rows a new comment points at, whichever is lower.
fn draw_compose(frame: &mut Frame, app: &App, compose: &Compose, rows: Option<(usize, usize)>) {
    let stream = areas(frame.area(), app.view.sidebar).stream;
    // The box goes where the card of the saved comment will be.
    let anchor = match &compose.draft {
        Draft::Comment(anchor) => Some(anchor),
        Draft::Reply(id) | Draft::Edit(id) => {
            let holds = |thread: &&Thread| thread.comments().any(|comment| comment.id == *id);
            app.review.threads.iter().find(holds).map(|t| &t.anchor)
        }
    };
    let side = anchor.and_then(Anchor::side);
    let (left, wide) = note_box(usize::from(stream.width), app.view.layout(), side);
    let slot = Rect {
        x: stream.x + u16::try_from(left).unwrap_or(0),
        width: u16::try_from(wide).unwrap_or(stream.width),
        ..stream
    };
    let under = rows.map_or(app.view.cursor, |(_, high)| high.max(app.view.cursor));
    let at = under.saturating_sub(app.view.scroll);
    let height = compose
        .editor
        .height(slot.width, (stream.height * 2 / 3).max(4));
    compose.editor.draw(
        frame,
        editor_rect(slot, at, height),
        &compose.title,
        &compose.place,
        &app.theme,
    );
}

/// The question the pane is waiting on, in a box over the middle of the pane.
fn draw_prompt(frame: &mut Frame, prompt: &Prompt, app: &App) {
    let theme = &app.theme;
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    // A choice: its key in `style`, then what it does.
    let choice = |key: &'static str, what: &'static str, style: Style| {
        Line::from(vec![Span::styled(key, style), Span::raw(what)])
    };
    let accent = key_style(theme);
    let (title, lines) = match prompt {
        Prompt::Quit => {
            let unsent = app.unsent();
            (
                format!(" {unsent} unsent comment{} ", plural(unsent)),
                vec![
                    choice("[s]", " send, then quit", accent),
                    choice("[k]", " quit and keep them unsent", accent),
                    choice("[esc]", " stay", accent),
                ],
            )
        }
        Prompt::Archive => {
            let (threads, unsent) = app.archivable();
            let never_sent = if unsent == 0 {
                String::new()
            } else {
                format!(" ({unsent} never sent)")
            };
            (
                format!(
                    " archive {threads} resolved thread{}{never_sent}? ",
                    plural(threads)
                ),
                vec![
                    choice("[y]", " yes", accent.fg(theme.success)),
                    choice("[n]", " no", accent.fg(theme.removed)),
                ],
            )
        }
        Prompt::Pick {
            found, selected, ..
        } => (
            " several agents match, pick one ".to_owned(),
            found
                .iter()
                .enumerate()
                .map(|(at, target)| {
                    let text = format!("> {} {}", target.agent, target.pane);
                    let line = Line::from(sanitize_terminal_text(&text));
                    if at == *selected {
                        line.style(Style::new().add_modifier(Modifier::REVERSED))
                    } else {
                        line
                    }
                })
                .collect(),
        ),
    };
    draw_popup(frame, theme, title, lines);
}

/// `lines` in a box over the middle of the pane, under `title` on its top border.
pub(crate) fn draw_popup(
    frame: &mut Frame,
    theme: &Theme,
    title: String,
    lines: Vec<Line<'static>>,
) {
    let area = frame.area();
    // Wide enough for the title between the corners, and no narrower than 44.
    let wanted = u16::try_from(string_width(&title) + 2).unwrap_or(u16::MAX);
    let width = area.width.saturating_sub(4).min(wanted.max(44));
    let height = (u16::try_from(lines.len()).unwrap_or(0) + 2).min(area.height);
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(theme, title))
            .style(Style::new().bg(theme.popup)),
        popup,
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
    // A frame is drawn only after something that can change it: an event, a new terminal size,
    // a read of the log, or a send.
    let mut dirty = true;
    let mut drawn_size = None;
    loop {
        if terminated() {
            app.save_draft();
            return Exit::Terminated;
        }
        let size = terminal.size().ok();
        if let Some(size) = size {
            app.resize(Rect::new(0, 0, size.width, size.height));
        }
        if dirty || size != drawn_size {
            app.highlight(git);
            if terminal.draw(|frame| render(frame, app)).is_err() {
                app.save_draft();
                return Exit::Io;
            }
            (dirty, drawn_size) = (false, size);
        }
        let polled = poll(TICK);
        dirty |= matches!(polled, Ok(Some(_)));
        let input = matches!(polled, Ok(Some(Event::Key(_) | Event::Mouse(_))));
        match polled {
            Err(_) => {
                app.save_draft();
                return Exit::Io;
            }
            Ok(Some(Event::Key(key))) if key.kind != KeyEventKind::Release => app.key(key, git),
            Ok(Some(Event::Mouse(mouse)))
                if app.screen == Screen::Review
                    && !app.view.help
                    && app.compose.is_none()
                    && app.prompt.is_none() =>
            {
                app.mouse(mouse);
            }
            Ok(Some(Event::FocusGained)) if app.compose.is_none() && app.view.select.is_none() => {
                app.load(git);
            }
            Ok(_) => {}
        }
        if input {
            app.mark_seen();
        }
        app.run_pending(|app| {
            dirty = true;
            let _ = terminal.draw(|frame| render(frame, app));
        });
        if app.quit {
            return Exit::Quit;
        }
        dirty |= app.check_store(git);
    }
}

/// Runs `restore` once when dropped, also while a panic unwinds.
pub(crate) struct Guard<F: FnMut()>(pub(crate) Option<F>);

impl<F: FnMut()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(mut restore) = self.0.take() {
            restore();
        }
    }
}

/// Raw mode off, main screen back, cursor shown, mouse and focus reports off.
pub(crate) fn restore_terminal() {
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
pub(crate) fn restoring_panic_hook(restore: fn()) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

pub(crate) fn enter_terminal() -> io::Result<()> {
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
mod tests;
