//! `theme --json`: the pane's colour scheme for the herdr-app panel (design/graphs/json.md).
//!
//! The scheme is read with the code the pane uses, so both show the same colours. The panel has no
//! environment of its own, so without `HERDR_PLUGIN_CONFIG_DIR` the directory is asked of Herdr.

use std::path::PathBuf;

use serde::Serialize;

use crate::cli::Output;
use crate::env::Env;
use crate::json::{SCHEMA, ThemeJson};
use crate::theme::{Gui, Theme};

/// The directory of `config.toml`: the environment's, else the one `herdr plugin config-dir review`
/// prints.
fn config_dir(
    env: &Env,
    herdr: &mut impl FnMut(&[String]) -> Result<String, String>,
) -> Result<PathBuf, String> {
    if let Some(dir) = env.get("HERDR_PLUGIN_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let args = ["plugin", "config-dir", "review"].map(str::to_owned);
    let printed = herdr(&args)?;
    match printed.trim() {
        "" => Err("herdr printed nothing".to_owned()),
        dir => Ok(PathBuf::from(dir)),
    }
}

fn print(value: &impl Serialize) -> Output {
    Output {
        stdout: format!("{}\n", serde_json::to_string(value).unwrap_or_default()),
        ..Output::default()
    }
}

/// `theme --json`. Nothing here fails: a config dir that cannot be found is a warning and the
/// defaults, and a file that cannot be read is the defaults.
pub fn command(env: &Env, mut herdr: impl FnMut(&[String]) -> Result<String, String>) -> Output {
    let (dir, mut warnings) = match config_dir(env, &mut herdr) {
        Ok(dir) => (Some(dir), Vec::new()),
        Err(why) => (None, vec![format!("could not find the config dir: {why}")]),
    };
    let path = dir.map(|dir| dir.join("config.toml"));
    let (theme, found) = Theme::load(path.as_deref());
    warnings.extend(found.iter().map(ToString::to_string));
    print(&ThemeJson {
        schema: SCHEMA,
        gui: theme.gui,
        colors: (theme.gui == Gui::Plugin).then(|| theme.panel_colors()),
        warnings,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unnecessary_wraps
)]
mod tests;
