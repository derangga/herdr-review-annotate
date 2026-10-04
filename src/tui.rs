//! The review pane. For now it prints its environment and waits for a key.

use std::io::Write;
use std::process::ExitCode;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};

fn report(
    out: &mut impl Write,
    vars: impl Iterator<Item = (String, String)>,
) -> std::io::Result<()> {
    let mut vars = vars
        .filter(|(name, _)| name.starts_with("HERDR_") || name.starts_with("REVIEW_"))
        .collect::<Vec<_>>();
    vars.sort();
    for (name, value) in vars {
        writeln!(out, "{name}={value}")?;
    }
    writeln!(out, "\npress any key to close")
}

pub(crate) fn run() -> ExitCode {
    let mut out = std::io::stdout().lock();
    if report(&mut out, std::env::vars())
        .and_then(|()| out.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    if enable_raw_mode().is_err() {
        return ExitCode::FAILURE;
    }
    let pressed = loop {
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => break true,
            Ok(_) => {}
            Err(_) => break false,
        }
    };
    let _ = disable_raw_mode();
    if pressed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_herdr_and_review_variables_are_printed() {
        let vars = [
            ("PATH", "/bin"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("REVIEW_DELIVER_TO", "w1:p2"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()));
        let mut out = Vec::new();
        report(&mut out, vars).unwrap_or_default();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("HERDR_PANE_ID=w1:p1\nREVIEW_DELIVER_TO=w1:p2\n"));
        assert!(!text.contains("PATH"));
    }
}
