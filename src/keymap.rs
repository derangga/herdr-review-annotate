//! The pane's keys: the defaults from PLAN.md section 7 and the `[keys]` table of `config.toml`.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::store::Warning;

/// What a key does in the pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    PrevHunk,
    NextHunk,
    PrevThread,
    NextThread,
    SwitchPanel,
    ToggleSidebar,
    Comment,
    SelectRange,
    Reply,
    Edit,
    Delete,
    Resolve,
    Send,
    Resend,
    Reload,
    SwitchSpec,
    ToggleLayout,
    Help,
    Quit,
}

impl Action {
    /// In the order of the table in PLAN.md section 7, which is the order of the help overlay.
    pub const ALL: [Self; 23] = [
        Self::Up,
        Self::Down,
        Self::PageUp,
        Self::PageDown,
        Self::PrevHunk,
        Self::NextHunk,
        Self::PrevThread,
        Self::NextThread,
        Self::SwitchPanel,
        Self::ToggleSidebar,
        Self::Comment,
        Self::SelectRange,
        Self::Reply,
        Self::Edit,
        Self::Delete,
        Self::Resolve,
        Self::Send,
        Self::Resend,
        Self::Reload,
        Self::SwitchSpec,
        Self::ToggleLayout,
        Self::Help,
        Self::Quit,
    ];

    /// The name used in `[keys]`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::PageUp => "page_up",
            Self::PageDown => "page_down",
            Self::PrevHunk => "prev_hunk",
            Self::NextHunk => "next_hunk",
            Self::PrevThread => "prev_thread",
            Self::NextThread => "next_thread",
            Self::SwitchPanel => "switch_panel",
            Self::ToggleSidebar => "toggle_sidebar",
            Self::Comment => "comment",
            Self::SelectRange => "select_range",
            Self::Reply => "reply",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Resolve => "resolve",
            Self::Send => "send",
            Self::Resend => "resend",
            Self::Reload => "reload",
            Self::SwitchSpec => "switch_spec",
            Self::ToggleLayout => "toggle_layout",
            Self::Help => "help",
            Self::Quit => "quit",
        }
    }

    /// What the action does, for the help overlay.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Up => "move up one row",
            Self::Down => "move down one row",
            Self::PageUp => "scroll up one page",
            Self::PageDown => "scroll down one page",
            Self::PrevHunk => "previous hunk",
            Self::NextHunk => "next hunk",
            Self::PrevThread => "previous thread",
            Self::NextThread => "next thread",
            Self::SwitchPanel => "switch between sidebar and stream",
            Self::ToggleSidebar => "show or hide the sidebar",
            Self::Comment => "comment on the line, range or file",
            Self::SelectRange => "start a range",
            Self::Reply => "reply to the thread",
            Self::Edit => "edit your comment",
            Self::Delete => "delete your comment",
            Self::Resolve => "resolve or reopen the thread",
            Self::Send => "send unsent comments",
            Self::Resend => "resend the thread",
            Self::Reload => "reload the diff",
            Self::SwitchSpec => "switch diff spec",
            Self::ToggleLayout => "side by side or unified",
            Self::Help => "show this help",
            Self::Quit => "quit",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }

    const fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::Up => &["k", "up"],
            Self::Down => &["j", "down"],
            Self::PageUp => &["pageup"],
            Self::PageDown => &["pagedown"],
            Self::PrevHunk => &["["],
            Self::NextHunk => &["]"],
            Self::PrevThread => &["shift+n"],
            Self::NextThread => &["n"],
            Self::SwitchPanel => &["tab"],
            Self::ToggleSidebar => &["f"],
            Self::Comment => &["c"],
            Self::SelectRange => &["v"],
            Self::Reply => &["r"],
            Self::Edit => &["e"],
            Self::Delete => &["d"],
            Self::Resolve => &["x"],
            Self::Send => &["shift+s"],
            Self::Resend => &["s"],
            Self::Reload => &["shift+r"],
            Self::SwitchSpec => &["b"],
            Self::ToggleLayout => &["t"],
            Self::Help => &["?"],
            Self::Quit => &["q"],
        }
    }
}

/// One key with its modifiers. `shift+s` and `S` are the same key, and so are `shift+tab` and
/// `backtab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

impl Key {
    fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let mods = mods & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let (code, mods) = match code {
            KeyCode::Char(c) if mods.contains(KeyModifiers::SHIFT) => (
                KeyCode::Char(c.to_uppercase().next().unwrap_or(c)),
                mods - KeyModifiers::SHIFT,
            ),
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => {
                (KeyCode::BackTab, mods - KeyModifiers::SHIFT)
            }
            KeyCode::BackTab => (code, mods - KeyModifiers::SHIFT),
            _ => (code, mods),
        };
        Self { code, mods }
    }

    pub fn from_event(event: &KeyEvent) -> Self {
        Self::new(event.code, event.modifiers)
    }
}

/// What a key prints as in the footer and the help overlay: `S`, `ctrl+n`, `pageup`.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(KeyModifiers::CONTROL) {
            f.write_str("ctrl+")?;
        }
        if self.mods.contains(KeyModifiers::ALT) {
            f.write_str("alt+")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::F(n) => write!(f, "f{n}"),
            KeyCode::BackTab => f.write_str("shift+tab"),
            code => f.write_str(&code.to_string().to_lowercase().replace(' ', "")),
        }
    }
}

impl FromStr for Key {
    type Err = String;

    /// `ctrl+`, `alt+` and `shift+` in front of a character or a name such as `enter`.
    fn from_str(text: &str) -> Result<Self, String> {
        let mut mods = KeyModifiers::empty();
        let mut rest = text;
        loop {
            let lower = rest.to_ascii_lowercase();
            let Some((flag, len)) = [
                ("ctrl+", KeyModifiers::CONTROL),
                ("alt+", KeyModifiers::ALT),
                ("shift+", KeyModifiers::SHIFT),
            ]
            .into_iter()
            .find(|(prefix, _)| lower.starts_with(prefix))
            .map(|(prefix, flag)| (flag, prefix.len())) else {
                break;
            };
            mods |= flag;
            rest = rest.get(len..).unwrap_or_default();
        }
        let mut chars = rest.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCode::Char(c),
            (Some(_), Some(_)) => {
                named(&rest.to_ascii_lowercase()).ok_or_else(|| format!("unknown key '{text}'"))?
            }
            (None, _) => return Err(format!("unknown key '{text}'")),
        };
        Ok(Self::new(code, mods))
    }
}

fn named(name: &str) -> Option<KeyCode> {
    Some(match name {
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ => {
            let n = name.strip_prefix('f')?.parse().ok()?;
            KeyCode::F((1..=12).contains(&n).then_some(n)?)
        }
    })
}

/// The effective keys, and what was wrong with the file they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    keys: HashMap<Key, Action>,
    pub warnings: Vec<Warning>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::from_config(&HashMap::new(), Vec::new())
    }
}

impl Keymap {
    /// Read `config.toml` at `path`. A missing file or no path is the defaults. A file that cannot
    /// be read or is not valid TOML is the defaults and a warning.
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => Self::with_warning(format!(
                "{}: {error}, using the default keys",
                path.display()
            )),
        }
    }

    fn with_warning(message: String) -> Self {
        Self {
            warnings: vec![Warning::Config(message)],
            ..Self::default()
        }
    }

    /// The keymap for the text of a `config.toml`.
    pub fn from_toml(text: &str) -> Self {
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(error) => {
                let reason = error.message().to_owned();
                return Self::with_warning(format!(
                    "config.toml is not valid TOML ({reason}), using the default keys"
                ));
            }
        };
        let mut warnings = Vec::new();
        let mut configured = HashMap::new();
        let keys = match table.get("keys") {
            Some(toml::Value::Table(keys)) => keys.iter().collect::<Vec<_>>(),
            Some(_) => {
                warnings.push(warn("[keys] is not a table, ignored".into()));
                Vec::new()
            }
            None => Vec::new(),
        };
        for (name, value) in keys {
            let Some(action) = Action::from_name(name) else {
                warnings.push(warn(format!("unknown action '{name}' in [keys], ignored")));
                continue;
            };
            let specs = match value {
                toml::Value::String(spec) => vec![spec.as_str()],
                toml::Value::Array(items) => {
                    let strings = items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .collect::<Vec<_>>();
                    if strings.len() != items.len() {
                        warnings.push(warn(format!("{name}: every key must be a string")));
                    }
                    strings
                }
                _ => {
                    warnings.push(warn(format!(
                        "{name}: expected a key or a list of keys, ignored"
                    )));
                    continue;
                }
            };
            let specs = specs
                .into_iter()
                .filter(|spec| !spec.is_empty())
                .collect::<Vec<_>>();
            let mut parsed = Vec::new();
            for spec in &specs {
                match spec.parse::<Key>() {
                    Ok(key) => parsed.push(key),
                    Err(message) => warnings.push(warn(format!("{name}: {message}, skipped"))),
                }
            }
            // A binding whose every key is bad keeps the defaults. An empty one unbinds.
            if specs.is_empty() || !parsed.is_empty() {
                configured.insert(action, parsed);
            }
        }
        Self::from_config(&configured, warnings)
    }

    /// Defaults for the actions not in `configured`, then the configured keys. A key two actions
    /// want goes to the configured one, or to the earlier in `Action::ALL` when both are.
    fn from_config(configured: &HashMap<Action, Vec<Key>>, mut warnings: Vec<Warning>) -> Self {
        let mut keys = HashMap::new();
        let mut keyless = Vec::new();
        for action in Action::ALL {
            if let Some(own) = configured.get(&action) {
                for key in own {
                    keys.entry(*key).or_insert(action);
                }
            }
        }
        for action in Action::ALL {
            if configured.contains_key(&action) {
                continue;
            }
            let mut any = false;
            for key in action
                .defaults()
                .iter()
                .filter_map(|spec| spec.parse::<Key>().ok())
            {
                any |= *keys.entry(key).or_insert(action) == action;
            }
            if !any {
                keyless.push(action);
            }
        }
        for (action, own) in configured {
            let lost = !own.is_empty() && own.iter().all(|key| keys.get(key) != Some(action));
            if lost {
                keyless.push(*action);
            }
        }
        keyless.sort();
        if !keyless.is_empty() {
            let names = keyless
                .iter()
                .map(|action| action.name())
                .collect::<Vec<_>>();
            warnings.push(warn(format!("no key left for {}", names.join(", "))));
        }
        Self { keys, warnings }
    }

    pub fn action(&self, event: &KeyEvent) -> Option<Action> {
        self.keys.get(&Key::from_event(event)).copied()
    }

    /// The keys of `action`, in a stable order, for the footer and the help overlay.
    pub fn keys(&self, action: Action) -> Vec<Key> {
        let mut keys = self
            .keys
            .iter()
            .filter(|(_, bound)| **bound == action)
            .map(|(key, _)| *key)
            .collect::<Vec<_>>();
        keys.sort_by_key(ToString::to_string);
        keys
    }

    /// `S` or `ctrl+s, S` for the footer, or `-` when the action has no key.
    pub fn label(&self, action: Action) -> String {
        let keys = self
            .keys(action)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if keys.is_empty() {
            "-".to_owned()
        } else {
            keys.join(", ")
        }
    }
}

fn warn(message: String) -> Warning {
    Warning::Config(message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn char_key(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn warnings(keymap: &Keymap) -> Vec<String> {
        keymap.warnings.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn the_defaults_are_the_table_in_the_plan() {
        let keymap = Keymap::default();
        assert!(keymap.warnings.is_empty());
        for (code, mods, action) in [
            (KeyCode::Char('k'), KeyModifiers::NONE, Action::Up),
            (KeyCode::Up, KeyModifiers::NONE, Action::Up),
            (KeyCode::Char('j'), KeyModifiers::NONE, Action::Down),
            (KeyCode::PageUp, KeyModifiers::NONE, Action::PageUp),
            (KeyCode::Char('['), KeyModifiers::NONE, Action::PrevHunk),
            (KeyCode::Char(']'), KeyModifiers::NONE, Action::NextHunk),
            (KeyCode::Char('n'), KeyModifiers::NONE, Action::NextThread),
            (KeyCode::Char('N'), KeyModifiers::SHIFT, Action::PrevThread),
            (KeyCode::Tab, KeyModifiers::NONE, Action::SwitchPanel),
            (
                KeyCode::Char('f'),
                KeyModifiers::NONE,
                Action::ToggleSidebar,
            ),
            (KeyCode::Char('s'), KeyModifiers::NONE, Action::Resend),
            (KeyCode::Char('S'), KeyModifiers::SHIFT, Action::Send),
            (KeyCode::Char('R'), KeyModifiers::SHIFT, Action::Reload),
            (KeyCode::Char('?'), KeyModifiers::NONE, Action::Help),
            (KeyCode::Char('q'), KeyModifiers::NONE, Action::Quit),
        ] {
            assert_eq!(keymap.action(&press(code, mods)), Some(action), "{code:?}");
        }
        assert_eq!(keymap.action(&char_key('z')), None);
        for action in Action::ALL {
            assert!(!keymap.keys(action).is_empty(), "{}", action.name());
        }
    }

    #[test]
    fn shift_s_and_a_capital_s_are_the_same_key() {
        assert_eq!("shift+s".parse::<Key>(), "S".parse::<Key>());
        assert_eq!("Shift+S".parse::<Key>(), "S".parse::<Key>());
        let keymap = Keymap::from_toml("[keys]\nsend = \"S\"\nresend = \"shift+s\"\n");
        let from_terminal = press(KeyCode::Char('S'), KeyModifiers::SHIFT);
        let without_modifier = press(KeyCode::Char('S'), KeyModifiers::NONE);
        assert_eq!(
            keymap.action(&from_terminal),
            keymap.action(&without_modifier)
        );
        assert_eq!("shift+tab".parse::<Key>(), "backtab".parse::<Key>());
    }

    #[test]
    fn an_override_replaces_the_default_keys_of_that_action() {
        let keymap = Keymap::from_toml("[keys]\nsend = \"ctrl+s\"\n");
        assert!(keymap.warnings.is_empty());
        let ctrl_s = press(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(keymap.action(&ctrl_s), Some(Action::Send));
        assert_eq!(
            keymap.action(&press(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            None
        );
        assert_eq!(keymap.action(&char_key('s')), Some(Action::Resend));
        assert_eq!(keymap.label(Action::Send), "ctrl+s");
    }

    #[test]
    fn a_list_binds_several_keys() {
        let keymap = Keymap::from_toml("[keys]\nnext_hunk = [\"]\", \"ctrl+n\"]\n");
        assert!(keymap.warnings.is_empty());
        assert_eq!(keymap.action(&char_key(']')), Some(Action::NextHunk));
        let ctrl_n = press(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(keymap.action(&ctrl_n), Some(Action::NextHunk));
        assert_eq!(keymap.action(&char_key('n')), Some(Action::NextThread));
        assert_eq!(keymap.label(Action::NextHunk), "], ctrl+n");
    }

    #[test]
    fn an_empty_string_unbinds_without_a_warning() {
        let keymap = Keymap::from_toml("[keys]\nswitch_spec = \"\"\nhelp = []\n");
        assert!(keymap.warnings.is_empty());
        assert_eq!(keymap.action(&char_key('b')), None);
        assert_eq!(keymap.action(&char_key('?')), None);
        assert_eq!(keymap.label(Action::SwitchSpec), "-");
    }

    #[test]
    fn a_configured_key_beats_another_actions_default_which_is_reported() {
        let keymap = Keymap::from_toml("[keys]\nreload = \"r\"\n");
        assert_eq!(keymap.action(&char_key('r')), Some(Action::Reload));
        assert_eq!(
            keymap.action(&press(KeyCode::Char('R'), KeyModifiers::SHIFT)),
            None
        );
        assert!(keymap.keys(Action::Reply).is_empty());
        assert_eq!(warnings(&keymap), ["no key left for reply"]);
        assert_eq!(keymap.label(Action::Reload), "r");
        assert_eq!(keymap.label(Action::Reply), "-");
    }

    #[test]
    fn the_warning_names_every_action_left_with_no_key() {
        let keymap = Keymap::from_toml("[keys]\nsend = \"r\"\nreload = \"d\"\n");
        assert_eq!(warnings(&keymap), ["no key left for reply, delete"]);
    }

    #[test]
    fn an_action_that_keeps_one_of_its_keys_is_not_reported() {
        let keymap = Keymap::from_toml("[keys]\nreload = \"j\"\n");
        assert_eq!(keymap.action(&char_key('j')), Some(Action::Reload));
        assert_eq!(
            keymap.action(&press(KeyCode::Down, KeyModifiers::NONE)),
            Some(Action::Down)
        );
        assert!(keymap.warnings.is_empty());
    }

    #[test]
    fn two_configured_actions_on_one_key_go_to_the_earlier_one() {
        let keymap = Keymap::from_toml("[keys]\nreload = \"F5\"\nsend = \"f5\"\n");
        assert_eq!(
            keymap.action(&press(KeyCode::F(5), KeyModifiers::NONE)),
            Some(Action::Send)
        );
        assert_eq!(warnings(&keymap), ["no key left for reload"]);
    }

    #[test]
    fn an_unknown_action_is_skipped_with_a_warning() {
        let keymap = Keymap::from_toml("[keys]\nfly = \"z\"\nquit = \"Q\"\n");
        assert_eq!(warnings(&keymap).len(), 1);
        assert!(warnings(&keymap)[0].contains("unknown action 'fly'"));
        assert_eq!(
            keymap.action(&press(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Some(Action::Quit)
        );
        assert_eq!(keymap.action(&char_key('z')), None);
    }

    #[test]
    fn a_key_that_does_not_parse_is_skipped_with_a_warning() {
        let keymap = Keymap::from_toml(
            "[keys]\nquit = [\"hyper+q\", \"Q\"]\nsend = \"nonsense\"\nhelp = 3\n",
        );
        let warnings = warnings(&keymap);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("hyper+q")));
        // quit keeps the key that parsed. send keeps its defaults, since none of its keys parsed.
        assert_eq!(keymap.action(&char_key('q')), None);
        assert_eq!(
            keymap.action(&press(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Some(Action::Quit)
        );
        assert_eq!(
            keymap.action(&press(KeyCode::Char('S'), KeyModifiers::SHIFT)),
            Some(Action::Send)
        );
        assert_eq!(keymap.action(&char_key('?')), Some(Action::Help));
    }

    #[test]
    fn a_file_that_is_not_toml_is_ignored_as_a_whole() {
        let keymap = Keymap::from_toml("[keys\nsend = ");
        assert_eq!(warnings(&keymap).len(), 1);
        assert!(warnings(&keymap)[0].contains("not valid TOML"));
        assert_eq!(keymap.keys, Keymap::default().keys);
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_an_unreadable_one_warns() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let missing = dir.join(format!(
            "herdr-review-no-config-{}.toml",
            std::process::id()
        ));
        assert_eq!(Keymap::load(Some(&missing)), Keymap::default());
        assert_eq!(Keymap::load(None), Keymap::default());
        let unreadable = Keymap::load(Some(&dir));
        assert_eq!(unreadable.warnings.len(), 1);
        let file = dir.join(format!("herdr-review-config-{}.toml", std::process::id()));
        std::fs::write(&file, "[keys]\nquit = \"Q\"\n").unwrap();
        let loaded = Keymap::load(Some(&file));
        let _ = std::fs::remove_file(&file);
        assert_eq!(
            loaded.action(&press(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Some(Action::Quit)
        );
    }

    #[test]
    fn other_tables_and_keys_in_the_file_are_ignored() {
        let keymap = Keymap::from_toml("theme = \"dark\"\n[ui]\nwide = true\n");
        assert_eq!(keymap, Keymap::default());
    }

    #[test]
    fn keys_print_the_way_they_are_spelled() {
        let text = |spec: &str| spec.parse::<Key>().unwrap().to_string();
        assert_eq!(text("shift+s"), "S");
        assert_eq!(text("ctrl+n"), "ctrl+n");
        assert_eq!(text("pageup"), "pageup");
        assert_eq!(text("space"), "space");
        assert_eq!(text("f5"), "f5");
        assert_eq!(text("ctrl++"), "ctrl++");
    }
}
