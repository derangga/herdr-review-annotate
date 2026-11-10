use std::path::{Path, PathBuf};

use super::*;

/// A temporary config directory, removed on drop.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str, config: Option<&str>) -> Self {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("herdr-review-tjson-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(config) = config {
            std::fs::write(dir.join("config.toml"), config).unwrap();
        }
        Self(dir)
    }

    fn env(&self) -> Env {
        Env::new(
            [(
                "HERDR_PLUGIN_CONFIG_DIR".to_owned(),
                self.0.display().to_string(),
            )],
            self.0.clone(),
        )
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn no_herdr(args: &[String]) -> Result<String, String> {
    panic!("herdr was asked {args:?}")
}

fn reply(output: &Output) -> ThemeJson {
    assert_eq!(output.code, 0);
    serde_json::from_str(&output.stdout).unwrap()
}

const PLUGIN: &str = "[theme]\nname = \"catppuccin-latte\"\ngui = \"plugin\"\n\n[theme.palette]\nbase = \"#1A1B26\"\nblue = \"#7aa2f7\"\n";

#[test]
fn plugin_prints_the_palette_under_the_panel_s_names() {
    let dir = Dir::new("plugin", Some(PLUGIN));
    let json = reply(&command(&dir.env(), no_herdr));
    assert_eq!((json.schema, json.gui), (1, Gui::Plugin));
    assert_eq!(json.warnings, Vec::<String>::new());
    // The overrides for base and blue, and latte's colour for everything else.
    let expected = [
        ("background", "#1a1b26"),
        ("foreground", "#4c4f69"),
        ("dim", "#8c8fa1"),
        ("muted", "#8c8fa1"),
        ("border", "#acb0be"),
        ("hover", "#ccd0da"),
        ("raised", "#ccd0da"),
        ("selected", "#bcc0cc"),
        ("chrome", "#e6e9ef"),
        ("accent", "#7aa2f7"),
        ("ansi1", "#d20f39"),
        ("ansi2", "#40a02b"),
        ("ansi3", "#df8e1d"),
        ("ansi4", "#7aa2f7"),
        ("ansi5", "#8839ef"),
        ("ansi6", "#fe640b"),
        ("ansi7", "#04a5e5"),
    ];
    let expected = expected
        .into_iter()
        .map(|(name, hex)| (name.to_owned(), hex.to_owned()))
        .collect();
    assert_eq!(json.colors, Some(expected));
}

#[test]
fn app_prints_no_colors() {
    for config in [
        "[theme]\ngui = \"app\"\n",
        "[theme]\n",
        "[theme]\ngui = 3\n",
    ] {
        let dir = Dir::new("app", Some(config));
        let json = reply(&command(&dir.env(), no_herdr));
        assert_eq!((json.gui, json.colors), (Gui::App, None), "{config}");
    }
}

#[test]
fn a_bad_palette_key_prints_its_warning_and_the_flavor_s_colour() {
    let config =
        "[theme]\ngui = \"plugin\"\n[theme.palette]\nblue = \"nope\"\ntext = \"#ffffff\"\n";
    let dir = Dir::new("badkey", Some(config));
    let json = reply(&command(&dir.env(), no_herdr));
    assert_eq!(
        json.warnings,
        ["[theme.palette] blue is not a \"#rrggbb\" string, keeping the flavor's colour"]
    );
    let colors = json.colors.unwrap();
    assert_eq!(colors["accent"], "#89b4fa");
    assert_eq!(colors["foreground"], "#ffffff");
}

#[test]
fn the_environment_wins_over_herdr() {
    let dir = Dir::new("env", Some(PLUGIN));
    let json = reply(&command(&dir.env(), no_herdr));
    assert_eq!(json.gui, Gui::Plugin);
}

#[test]
fn without_the_environment_the_dir_is_what_herdr_prints() {
    let dir = Dir::new("herdr", Some(PLUGIN));
    let mut asked = Vec::new();
    let printed = format!("{}\n", dir.0.display());
    let json = reply(&command(&Env::default(), |args| {
        asked.push(args.to_vec());
        Ok(printed.clone())
    }));
    assert_eq!(
        asked,
        [["plugin", "config-dir", "review"].map(str::to_owned)]
    );
    assert_eq!(json.gui, Gui::Plugin);
    assert_eq!(json.warnings, Vec::<String>::new());
}

#[test]
fn herdr_failing_prints_the_defaults_and_a_warning() {
    for failure in [
        Err("herdr is not installed".to_owned()),
        Ok("\n".to_owned()),
    ] {
        let json = reply(&command(&Env::default(), |_| failure.clone()));
        assert_eq!((json.gui, json.colors), (Gui::App, None));
        let [warning] = json.warnings.as_slice() else {
            panic!("{:?}", json.warnings)
        };
        assert!(
            warning.starts_with("could not find the config dir: "),
            "{warning}"
        );
    }
}

fn assert_defaults_without_warning(dir: &Dir) {
    let json = reply(&command(&dir.env(), no_herdr));
    assert_eq!(
        (json.gui, json.colors, json.warnings),
        (Gui::App, None, Vec::new())
    );
}

#[test]
fn a_missing_file_prints_the_defaults_without_a_warning() {
    assert_defaults_without_warning(&Dir::new("missing", None));
}

#[test]
fn an_unreadable_file_prints_the_defaults_without_a_warning() {
    let dir = Dir::new("unreadable", None);
    std::fs::create_dir(dir.0.join("config.toml")).unwrap();
    assert_defaults_without_warning(&dir);
}

#[test]
fn a_file_that_is_not_toml_prints_the_defaults_without_a_warning() {
    assert_defaults_without_warning(&Dir::new("notoml", Some("[theme")));
}

#[test]
fn the_golden_file_is_a_plugin_reply() {
    let dir = Dir::new("golden", Some(PLUGIN));
    let json = reply(&command(&dir.env(), no_herdr));
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("herdr-app-mod/test/fixtures/theme.json");
    // Rewritten by `REVIEW_UPDATE_FIXTURES=1 cargo test`, and the same commit carries it.
    if std::env::var_os("REVIEW_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();
    }
    let saved = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        saved,
        serde_json::to_value(&json).unwrap(),
        "theme.json differs, rerun with REVIEW_UPDATE_FIXTURES=1 and commit the new file"
    );
}
