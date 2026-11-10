//! The pane's colours: one `Theme` of named roles, filled from a Catppuccin flavor with the user's
//! `[theme.palette]` over it, and the `[theme]` table of `config.toml` that chooses both.
//!
//! No other module names a colour. Drawing code asks the theme for a role, so a flavor or a palette
//! changes every colour at once. The colours are 24-bit, and a terminal without truecolor is not handled.

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

use crate::store::Warning;

/// The flavor a pane uses when `config.toml` names none, or names one that does not exist.
pub const DEFAULT: &str = "catppuccin-mocha";

/// The colours of one Catppuccin flavor that the roles are made from, each as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Palette {
    base: u32,
    mantle: u32,
    surface0: u32,
    surface1: u32,
    surface2: u32,
    overlay1: u32,
    text: u32,
    red: u32,
    green: u32,
    yellow: u32,
    blue: u32,
    mauve: u32,
    peach: u32,
    sky: u32,
}

const MOCHA: Palette = Palette {
    base: 0x1e_1e2e,
    mantle: 0x18_1825,
    surface0: 0x31_3244,
    surface1: 0x45_475a,
    surface2: 0x58_5b70,
    overlay1: 0x7f_849c,
    text: 0xcd_d6f4,
    red: 0xf3_8ba8,
    green: 0xa6_e3a1,
    yellow: 0xf9_e2af,
    blue: 0x89_b4fa,
    mauve: 0xcb_a6f7,
    peach: 0xfa_b387,
    sky: 0x89_dceb,
};

const MACCHIATO: Palette = Palette {
    base: 0x24_273a,
    mantle: 0x1e_2030,
    surface0: 0x36_3a4f,
    surface1: 0x49_4d64,
    surface2: 0x5b_6078,
    overlay1: 0x80_87a2,
    text: 0xca_d3f5,
    red: 0xed_8796,
    green: 0xa6_da95,
    yellow: 0xee_d49f,
    blue: 0x8a_adf4,
    mauve: 0xc6_a0f6,
    peach: 0xf5_a97f,
    sky: 0x91_d7e3,
};

const FRAPPE: Palette = Palette {
    base: 0x30_3446,
    mantle: 0x29_2c3c,
    surface0: 0x41_4559,
    surface1: 0x51_576d,
    surface2: 0x62_6880,
    overlay1: 0x83_8ba7,
    text: 0xc6_d0f5,
    red: 0xe7_8284,
    green: 0xa6_d189,
    yellow: 0xe5_c890,
    blue: 0x8c_aaee,
    mauve: 0xca_9ee6,
    peach: 0xef_9f76,
    sky: 0x99_d1db,
};

const LATTE: Palette = Palette {
    base: 0xef_f1f5,
    mantle: 0xe6_e9ef,
    surface0: 0xcc_d0da,
    surface1: 0xbc_c0cc,
    surface2: 0xac_b0be,
    overlay1: 0x8c_8fa1,
    text: 0x4c_4f69,
    red: 0xd2_0f39,
    green: 0x40_a02b,
    yellow: 0xdf_8e1d,
    blue: 0x1e_66f5,
    mauve: 0x88_39ef,
    peach: 0xfe_640b,
    sky: 0x04_a5e5,
};

impl Palette {
    /// The colour a `[theme.palette]` key sets, or none when the key is not one of the 14.
    fn slot(&mut self, key: &str) -> Option<&mut u32> {
        Some(match key {
            "base" => &mut self.base,
            "mantle" => &mut self.mantle,
            "surface0" => &mut self.surface0,
            "surface1" => &mut self.surface1,
            "surface2" => &mut self.surface2,
            "overlay1" => &mut self.overlay1,
            "text" => &mut self.text,
            "red" => &mut self.red,
            "green" => &mut self.green,
            "yellow" => &mut self.yellow,
            "blue" => &mut self.blue,
            "mauve" => &mut self.mauve,
            "peach" => &mut self.peach,
            "sky" => &mut self.sky,
            _ => return None,
        })
    }

    /// `self` with the keys of a `[theme.palette]` table laid over it. Each key that cannot be
    /// applied is one warning and keeps `self`'s colour.
    fn merged(mut self, table: &toml::Table, warnings: &mut Vec<Warning>) -> Self {
        for (key, value) in table {
            let hex = value.as_str().and_then(hex_of);
            let message = match (self.slot(key), hex) {
                (Some(slot), Some(hex)) => {
                    *slot = hex;
                    continue;
                }
                (Some(_), None) => format!(
                    "[theme.palette] {key} is not a \"#rrggbb\" string, keeping the flavor's colour"
                ),
                (None, _) => format!("[theme.palette] {key} is not a palette colour, ignored"),
            };
            warnings.push(Warning::Config(message));
        }
        self
    }
}

/// `#rrggbb`, six hex digits of either case, and nothing else.
fn hex_of(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// The names `[theme] name` accepts.
const FLAVORS: [(&str, &Palette); 4] = [
    (DEFAULT, &MOCHA),
    ("catppuccin-macchiato", &MACCHIATO),
    ("catppuccin-frappe", &FRAPPE),
    ("catppuccin-latte", &LATTE),
];

/// How much of the green or the red is mixed into the base for the tint of a changed row.
const TINT_PERCENT: u32 = 15;

/// How much of the green or the red is mixed into the base behind a changed word of a row.
const WORD_PERCENT: u32 = 35;

/// How much of the accent is mixed into the base behind a range being selected.
const SELECTION_PERCENT: u32 = 30;

const fn channels(hex: u32) -> (u32, u32, u32) {
    ((hex >> 16) & 0xff, (hex >> 8) & 0xff, hex & 0xff)
}

const fn rgb(hex: u32) -> Color {
    let (r, g, b) = channels(hex);
    Color::Rgb(r as u8, g as u8, b as u8)
}

const fn mix(over: u32, under: u32, percent: u32) -> u8 {
    ((over * percent + under * (100 - percent)) / 100) as u8
}

/// `over` mixed into `under`, `percent` parts in a hundred.
const fn blend(over: u32, under: u32, percent: u32) -> Color {
    let (or, og, ob) = channels(over);
    let (ur, ug, ub) = channels(under);
    Color::Rgb(
        mix(or, ur, percent),
        mix(og, ug, percent),
        mix(ob, ub, percent),
    )
}

/// What a piece of code is, as far as its colour goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Comment,
    String,
    /// A number, and any other constant.
    Number,
    Keyword,
    Operator,
    Function,
    Type,
}

/// The token of a `TextMate` scope and of every scope under it. The first entry that fits wins, so
/// a narrower scope stands above the wider one it belongs to. A scope that is not here has no
/// colour of its own and takes that of the scope around it.
pub const SCOPES: &[(&str, Token)] = &[
    ("comment", Token::Comment),
    ("string", Token::String),
    ("constant", Token::Number),
    ("keyword.operator", Token::Operator),
    ("keyword", Token::Keyword),
    ("storage", Token::Keyword),
    ("variable.language", Token::Keyword),
    ("entity.name.function", Token::Function),
    ("support.function", Token::Function),
    ("variable.function", Token::Function),
    ("entity.name.tag", Token::Function),
    ("entity.name", Token::Type),
    ("entity.other.inherited-class", Token::Type),
    ("entity.other.attribute-name", Token::Type),
    ("support.type", Token::Type),
    ("support.class", Token::Type),
];

/// The token of the scope `scope`, such as `keyword.operator.arithmetic.rust`.
pub fn token_of(scope: &str) -> Option<Token> {
    let under = |prefix: &str| {
        scope
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    };
    SCOPES
        .iter()
        .find(|(prefix, _)| under(prefix))
        .map(|(_, token)| *token)
}

/// Where the herdr-app panel takes its colours: from herdr-app (`App`) or from this palette
/// (`Plugin`). The pane draws the same for both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gui {
    #[default]
    App,
    Plugin,
}

/// What each part of the pane is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Where the panel takes its colours. The pane does not read it.
    pub gui: Gui,
    /// The merged palette the roles below were made from, kept for `theme --json`.
    palette: Palette,
    /// The background of the pane.
    pub base: Color,
    pub text: Color,
    /// Text that matters less: line numbers, hints, a resolved thread.
    pub subtle: Color,
    pub border: Color,
    /// Hunk headers, the `[+]` marker and the user's comments.
    pub accent: Color,
    /// An agent's comments.
    pub agent: Color,
    /// Behind the cursor's row.
    pub cursor: Color,
    /// Behind the rows of a range being selected.
    pub selection: Color,
    /// The chip that says visual mode is on.
    pub visual: Color,
    pub added: Color,
    pub removed: Color,
    /// Behind an added and a removed row.
    pub added_bg: Color,
    pub removed_bg: Color,
    /// Behind the words of an added and of a removed row that its paired row does not have.
    pub added_word: Color,
    pub removed_word: Color,
    /// The half of a split row that has no line.
    pub filler: Color,
    /// Behind a file header and the unchanged-lines row.
    pub header: Color,
    /// Behind the help overlay and the prompts.
    pub popup: Color,
    pub warning: Color,
    pub success: Color,
    /// The colours of code, one per `Token`. A comment is drawn as subtle text.
    pub string: Color,
    pub number: Color,
    pub keyword: Color,
    pub operator: Color,
    pub function: Color,
    pub kind: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::of(&MOCHA)
    }
}

impl Theme {
    const fn of(palette: &Palette) -> Self {
        Self {
            gui: Gui::App,
            palette: *palette,
            base: rgb(palette.base),
            text: rgb(palette.text),
            subtle: rgb(palette.overlay1),
            border: rgb(palette.surface2),
            accent: rgb(palette.blue),
            agent: rgb(palette.mauve),
            cursor: rgb(palette.surface1),
            selection: blend(palette.blue, palette.base, SELECTION_PERCENT),
            visual: rgb(palette.peach),
            added: rgb(palette.green),
            removed: rgb(palette.red),
            added_bg: blend(palette.green, palette.base, TINT_PERCENT),
            removed_bg: blend(palette.red, palette.base, TINT_PERCENT),
            added_word: blend(palette.green, palette.base, WORD_PERCENT),
            removed_word: blend(palette.red, palette.base, WORD_PERCENT),
            filler: rgb(palette.mantle),
            header: rgb(palette.surface0),
            popup: rgb(palette.mantle),
            warning: rgb(palette.yellow),
            success: rgb(palette.green),
            string: rgb(palette.green),
            number: rgb(palette.peach),
            keyword: rgb(palette.mauve),
            operator: rgb(palette.sky),
            function: rgb(palette.blue),
            kind: rgb(palette.yellow),
        }
    }

    /// The flavor called `name`, as `[theme] name` spells it.
    pub fn named(name: &str) -> Option<Self> {
        FLAVORS
            .iter()
            .find(|(flavor, _)| *flavor == name)
            .map(|(_, palette)| Self::of(palette))
    }

    /// Read `[theme]` from the `config.toml` at `path`. A file that is missing, unreadable or not
    /// TOML is the default with no warning here, because the keymap reads the same file and
    /// reports it.
    pub fn load(path: Option<&Path>) -> (Self, Vec<Warning>) {
        path.and_then(|path| std::fs::read_to_string(path).ok())
            .map_or_else(
                || (Self::default(), Vec::new()),
                |text| Self::from_toml(&text),
            )
    }

    /// The theme for the text of a `config.toml`, and what was wrong with its `[theme]` table. A
    /// bad `name` keeps a good `palette`, which then goes over mocha.
    pub fn from_toml(text: &str) -> (Self, Vec<Warning>) {
        let Ok(table) = text.parse::<toml::Table>() else {
            return (Self::default(), Vec::new());
        };
        let section = match table.get("theme") {
            None => return (Self::default(), Vec::new()),
            Some(toml::Value::Table(section)) => section,
            Some(_) => {
                let message = format!("[theme] is not a table, using {DEFAULT}");
                return (Self::default(), vec![Warning::Config(message)]);
            }
        };
        let mut warnings = Vec::new();
        let mut flavor = &MOCHA;
        match section.get("name") {
            None => {}
            Some(toml::Value::String(name)) => {
                match FLAVORS.iter().find(|(flavor, _)| flavor == name) {
                    Some((_, found)) => flavor = found,
                    None => warnings.push(Warning::Config(format!(
                        "unknown theme '{name}', using {DEFAULT}"
                    ))),
                }
            }
            Some(_) => warnings.push(Warning::Config(format!(
                "[theme] name is not a string, using {DEFAULT}"
            ))),
        }
        let palette = match section.get("palette") {
            None => *flavor,
            Some(toml::Value::Table(overrides)) => flavor.merged(overrides, &mut warnings),
            Some(_) => {
                warnings.push(Warning::Config(
                    "[theme.palette] is not a table, no overrides".to_owned(),
                ));
                *flavor
            }
        };
        let gui = match section.get("gui") {
            None => Gui::App,
            Some(toml::Value::String(gui)) if gui == "app" => Gui::App,
            Some(toml::Value::String(gui)) if gui == "plugin" => Gui::Plugin,
            Some(_) => {
                warnings.push(Warning::Config(
                    "[theme] gui is not \"app\" or \"plugin\", using app".to_owned(),
                ));
                Gui::App
            }
        };
        (
            Self {
                gui,
                ..Self::of(&palette)
            },
            warnings,
        )
    }

    /// The palette as the herdr-app theme names the panel reads, each `#rrggbb` in lower case.
    pub fn panel_colors(&self) -> BTreeMap<String, String> {
        let p = &self.palette;
        let names: [(&str, u32); 17] = [
            ("background", p.base),
            ("foreground", p.text),
            ("dim", p.overlay1),
            ("muted", p.overlay1),
            ("border", p.surface2),
            ("hover", p.surface0),
            ("raised", p.surface0),
            ("selected", p.surface1),
            ("chrome", p.mantle),
            ("accent", p.blue),
            ("ansi1", p.red),
            ("ansi2", p.green),
            ("ansi3", p.yellow),
            ("ansi4", p.blue),
            ("ansi5", p.mauve),
            ("ansi6", p.peach),
            ("ansi7", p.sky),
        ];
        names
            .into_iter()
            .map(|(name, hex)| (name.to_owned(), format!("#{hex:06x}")))
            .collect()
    }

    /// The colour of a piece of code.
    pub const fn token(&self, token: Token) -> Color {
        match token {
            Token::Comment => self.subtle,
            Token::String => self.string,
            Token::Number => self.number,
            Token::Keyword => self.keyword,
            Token::Operator => self.operator,
            Token::Function => self.function,
            Token::Type => self.kind,
        }
    }

    /// Text that matters less.
    pub fn dim(&self) -> Style {
        Style::new().fg(self.subtle)
    }

    /// Give every cell that still has the terminal's own colours the theme's base and text. It
    /// runs after everything is drawn, so a cleared rectangle and unstyled text get them too.
    pub fn paint(&self, buffer: &mut Buffer) {
        for cell in &mut buffer.content {
            if cell.bg == Color::Reset {
                cell.bg = self.base;
            }
            if cell.fg == Color::Reset {
                cell.fg = self.text;
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    fn config(text: &str) -> (Theme, Vec<String>) {
        let (theme, warnings) = Theme::from_toml(text);
        (theme, warnings.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn each_flavor_loads_by_its_name_with_its_own_base() {
        let bases = [
            ("catppuccin-mocha", Color::Rgb(0x1e, 0x1e, 0x2e)),
            ("catppuccin-macchiato", Color::Rgb(0x24, 0x27, 0x3a)),
            ("catppuccin-frappe", Color::Rgb(0x30, 0x34, 0x46)),
            ("catppuccin-latte", Color::Rgb(0xef, 0xf1, 0xf5)),
        ];
        for (name, base) in bases {
            let (theme, warnings) = config(&format!("[theme]\nname = \"{name}\"\n"));
            assert_eq!(theme.base, base, "{name}");
            assert_eq!(warnings, Vec::<String>::new(), "{name}");
        }
    }

    #[test]
    fn no_theme_table_is_mocha_without_a_warning() {
        for text in ["", "[keys]\nsend = \"s\"\n", "[theme]\n"] {
            assert_eq!(config(text), (Theme::default(), Vec::new()), "{text}");
        }
        assert_eq!(Theme::default(), Theme::named(DEFAULT).unwrap());
    }

    #[test]
    fn an_unknown_name_warns_and_falls_back_to_mocha() {
        let (theme, warnings) = config("[theme]\nname = \"solarized\"\n");
        assert_eq!(theme, Theme::default());
        assert_eq!(
            warnings,
            ["unknown theme 'solarized', using catppuccin-mocha"]
        );
    }

    #[test]
    fn a_theme_table_of_the_wrong_shape_warns_and_falls_back_to_mocha() {
        let (theme, warnings) = config("theme = \"catppuccin-latte\"\n");
        assert_eq!(theme, Theme::default());
        assert_eq!(warnings, ["[theme] is not a table, using catppuccin-mocha"]);
        let (theme, warnings) = config("[theme]\nname = 3\n");
        assert_eq!(theme, Theme::default());
        assert_eq!(
            warnings,
            ["[theme] name is not a string, using catppuccin-mocha"]
        );
    }

    #[test]
    fn a_file_that_is_missing_or_not_toml_is_left_to_the_keymap_to_report() {
        assert_eq!(config("[theme"), (Theme::default(), Vec::new()));
        assert_eq!(Theme::load(None), (Theme::default(), Vec::new()));
        let missing = Path::new("/nonexistent/herdr-review/config.toml");
        assert_eq!(Theme::load(Some(missing)), (Theme::default(), Vec::new()));
    }

    #[test]
    fn the_tints_are_the_flavor_s_green_and_red_mixed_into_its_base() {
        // 15 parts of a6e3a1 and 85 of 1e1e2e, each channel rounded down.
        assert_eq!(Theme::default().added_bg, Color::Rgb(50, 59, 63));
        assert_eq!(Theme::default().removed_bg, Color::Rgb(61, 46, 64));
        let latte = Theme::named("catppuccin-latte").unwrap();
        assert_ne!(latte.added_bg, Theme::default().added_bg);
        assert_ne!(latte.removed_bg, latte.added_bg);
    }

    #[test]
    fn a_palette_key_changes_its_role_and_the_tints_mixed_from_it_over_each_flavor() {
        for (name, _) in FLAVORS {
            let flavor = Theme::named(name).unwrap();
            let text = format!(
                "[theme]\nname = \"{name}\"\n[theme.palette]\nbase = \"#102030\"\ngreen = \"#00FF80\"\n"
            );
            let (theme, warnings) = config(&text);
            assert_eq!(warnings, Vec::<String>::new(), "{name}");
            assert_eq!(theme.base, Color::Rgb(0x10, 0x20, 0x30), "{name}");
            assert_eq!(theme.added, Color::Rgb(0, 0xff, 0x80), "{name}");
            assert_eq!(theme.success, theme.added, "{name}");
            assert_eq!(theme.string, theme.added, "{name}");
            // 15 and 35 parts of 00ff80 and the rest of 102030, each channel rounded down.
            assert_eq!(theme.added_bg, Color::Rgb(13, 65, 60), "{name}");
            assert_eq!(theme.added_word, Color::Rgb(10, 110, 76), "{name}");
            // The red tint is the flavor's red mixed into the new base.
            assert_ne!(theme.removed_bg, flavor.removed_bg, "{name}");
            assert_eq!(theme.removed, flavor.removed, "{name}");
            assert_eq!(theme.text, flavor.text, "{name}");
        }
        let (theme, _) = config("[theme.palette]\nblue = \"#7aa2f7\"\n");
        assert_eq!(theme.base, Theme::default().base);
        assert_ne!(theme.selection, Theme::default().selection);
    }

    #[test]
    fn a_light_base_leaves_the_other_keys_the_flavor_s() {
        let latte = Theme::named("catppuccin-latte").unwrap();
        let (theme, warnings) =
            config("[theme]\nname = \"catppuccin-latte\"\n[theme.palette]\nbase = \"#fdf6e3\"\n");
        assert_eq!(warnings, Vec::<String>::new());
        assert_eq!(theme.base, Color::Rgb(0xfd, 0xf6, 0xe3));
        assert_eq!((theme.text, theme.added), (latte.text, latte.added));
    }

    #[test]
    fn each_bad_palette_key_warns_once_and_keeps_the_flavor_s_colour() {
        let mocha = Theme::default();
        let (theme, warnings) = config(
            "[theme.palette]\nblue = 3\nred = \"#12345\"\ngreen = \"112233\"\nmauve = \"#gggggg\"\n\
             peach = \"#1234567\"\npink = \"#ffffff\"\ntext = \"#abcdef\"\n",
        );
        assert_eq!(
            warnings,
            [
                "[theme.palette] blue is not a \"#rrggbb\" string, keeping the flavor's colour",
                "[theme.palette] green is not a \"#rrggbb\" string, keeping the flavor's colour",
                "[theme.palette] mauve is not a \"#rrggbb\" string, keeping the flavor's colour",
                "[theme.palette] peach is not a \"#rrggbb\" string, keeping the flavor's colour",
                "[theme.palette] pink is not a palette colour, ignored",
                "[theme.palette] red is not a \"#rrggbb\" string, keeping the flavor's colour",
            ]
        );
        assert_eq!(theme.accent, mocha.accent);
        assert_eq!(theme.removed, mocha.removed);
        assert_eq!(theme.agent, mocha.agent);
        assert_eq!(theme.text, Color::Rgb(0xab, 0xcd, 0xef));
    }

    #[test]
    fn a_palette_that_is_not_a_table_warns_and_overrides_nothing() {
        let (theme, warnings) = config("[theme]\npalette = \"#ffffff\"\n");
        assert_eq!(theme, Theme::default());
        assert_eq!(warnings, ["[theme.palette] is not a table, no overrides"]);
    }

    #[test]
    fn a_bad_name_keeps_a_good_palette_over_mocha() {
        let (theme, warnings) =
            config("[theme]\nname = \"solarized\"\n[theme.palette]\nbase = \"#102030\"\n");
        assert_eq!(theme.base, Color::Rgb(0x10, 0x20, 0x30));
        assert_eq!(theme.text, Theme::default().text);
        assert_eq!(
            warnings,
            ["unknown theme 'solarized', using catppuccin-mocha"]
        );
        let (theme, warnings) = config("[theme]\nname = 3\n[theme.palette]\nbase = \"#102030\"\n");
        assert_eq!(theme.base, Color::Rgb(0x10, 0x20, 0x30));
        assert_eq!(
            warnings,
            ["[theme] name is not a string, using catppuccin-mocha"]
        );
    }

    #[test]
    fn gui_is_app_when_missing_or_bad_and_plugin_only_when_asked() {
        for (text, gui, warned) in [
            ("[theme]\n", Gui::App, false),
            ("[theme]\ngui = \"app\"\n", Gui::App, false),
            ("[theme]\ngui = \"plugin\"\n", Gui::Plugin, false),
            ("[theme]\ngui = \"pane\"\n", Gui::App, true),
            ("[theme]\ngui = true\n", Gui::App, true),
        ] {
            let (theme, warnings) = config(text);
            assert_eq!(theme.gui, gui, "{text}");
            assert_eq!(
                warnings,
                if warned {
                    vec!["[theme] gui is not \"app\" or \"plugin\", using app"]
                } else {
                    vec![]
                },
                "{text}"
            );
            assert_eq!(theme.base, Theme::default().base, "{text}");
        }
    }

    #[test]
    fn a_changed_word_has_more_of_the_green_or_red_behind_it_than_its_row() {
        // 35 parts of a6e3a1 and 65 of 1e1e2e.
        assert_eq!(Theme::default().added_word, Color::Rgb(77, 98, 86));
        assert_eq!(Theme::default().removed_word, Color::Rgb(104, 68, 88));
    }

    #[test]
    fn a_scope_takes_the_token_of_the_narrowest_entry_it_is_under() {
        for (scope, token) in [
            ("comment.block.documentation.rust", Some(Token::Comment)),
            ("string.quoted.double.js", Some(Token::String)),
            ("constant.numeric.integer.decimal.rust", Some(Token::Number)),
            ("constant.language.boolean", Some(Token::Number)),
            ("keyword.control.rust", Some(Token::Keyword)),
            ("keyword.operator.arithmetic.rust", Some(Token::Operator)),
            ("keyword", Some(Token::Keyword)),
            ("storage.type.function.rust", Some(Token::Keyword)),
            ("variable.language.this.js", Some(Token::Keyword)),
            ("entity.name.function.rust", Some(Token::Function)),
            ("support.function.builtin", Some(Token::Function)),
            ("entity.name.struct.rust", Some(Token::Type)),
            ("support.type.primitive", Some(Token::Type)),
            // Not under any entry: a look-alike prefix, plain variables, punctuation, the file.
            ("keywordish.thing", None),
            ("variable.other.readwrite.js", None),
            ("punctuation.separator", None),
            ("source.rust", None),
            ("", None),
        ] {
            assert_eq!(token_of(scope), token, "{scope}");
        }
    }

    #[test]
    fn every_token_has_a_colour_that_follows_the_flavor() {
        let (mocha, latte) = (Theme::default(), Theme::named("catppuccin-latte").unwrap());
        assert_eq!(mocha.token(Token::Keyword), Color::Rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(mocha.token(Token::Number), Color::Rgb(0xfa, 0xb3, 0x87));
        assert_eq!(mocha.token(Token::Comment), mocha.subtle);
        for token in [
            Token::Comment,
            Token::String,
            Token::Number,
            Token::Keyword,
            Token::Operator,
            Token::Function,
            Token::Type,
        ] {
            assert_ne!(mocha.token(token), latte.token(token), "{token:?}");
            assert_ne!(mocha.token(token), mocha.text, "{token:?}");
        }
    }

    #[test]
    fn no_other_module_names_a_colour() {
        let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.file_name().unwrap() != "theme.rs" {
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(!text.contains("Color::"), "{}", path.display());
                }
            }
        }
    }

    #[test]
    fn paint_fills_only_the_cells_nothing_coloured() {
        let theme = Theme::default();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer[(1, 0)].set_style(Style::new().bg(theme.cursor).fg(theme.added));
        theme.paint(&mut buffer);
        assert_eq!(
            (buffer[(0, 0)].bg, buffer[(0, 0)].fg),
            (theme.base, theme.text)
        );
        assert_eq!(
            (buffer[(1, 0)].bg, buffer[(1, 0)].fg),
            (theme.cursor, theme.added)
        );
    }
}
