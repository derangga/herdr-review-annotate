//! Terminal cell-width helpers. They measure as ratatui draws: a string with ratatui's own
//! `CellWidth`, a character with the `unicode-width` crate that `CellWidth` uses.

use ratatui::buffer::CellWidth;
use unicode_width::UnicodeWidthChar;

/// Cells occupied by a single character. Control characters count as zero. The halfwidth
/// voiced-sound marks (U+FF9E, U+FF9F) are zero for `unicode-width` and one cell for ratatui, which
/// adds a cell for each in a string.
pub fn char_width(character: char) -> usize {
    if matches!(character, '\u{ff9e}' | '\u{ff9f}') {
        return 1;
    }
    character.width().unwrap_or(0)
}

/// Cells occupied by a string. Control characters are left out, as ratatui leaves them out before
/// it measures (`CellWidth` asserts on a lone control character), and an emoji sequence is one wide
/// glyph, so this is not always the sum of `char_width`.
pub fn string_width(text: &str) -> usize {
    let cells = if text.chars().any(char::is_control) {
        text.chars()
            .filter(|character| !character.is_control())
            .collect::<String>()
            .cell_width()
    } else {
        text.cell_width()
    };
    usize::from(cells)
}

/// Longest prefix of `text` that fits without splitting a character.
pub fn truncate_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    text.chars()
        .take_while(|character| {
            let next = char_width(*character);
            let fits = used + next <= width;
            if fits {
                used += next;
            }
            fits
        })
        .collect()
}

/// The last `width` cells of `text`, with a leading `…` when something was cut.
pub fn tail_to_width(text: &str, width: usize) -> String {
    if string_width(text) <= width {
        return text.to_owned();
    }
    let mut used = 1;
    let mut kept = Vec::new();
    for character in text.chars().rev() {
        used += char_width(character);
        if used > width {
            break;
        }
        kept.push(character);
    }
    kept.push('…');
    kept.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_and_ascii_widths() {
        for character in ['한', 'ㄱ', '漢', 'あ', 'Ａ'] {
            assert_eq!(char_width(character), 2);
        }
        for character in ['a', '·'] {
            assert_eq!(char_width(character), 1);
        }
        assert_eq!(char_width('\u{0007}'), 0);
    }

    #[test]
    fn string_width_adds_character_cells() {
        assert_eq!(string_width("한글abc"), 7);
        assert_eq!(string_width(""), 0);
    }

    #[test]
    fn emoji_that_ratatui_draws_in_two_cells_are_two_cells() {
        for character in ['✅', '❌', '✨', '⭐', '⚡', '🚀'] {
            assert_eq!(char_width(character), 2, "{character}");
        }
    }

    #[test]
    fn a_variation_selector_and_combining_marks_take_no_cells() {
        for character in ['\u{fe0f}', '\u{0301}', '\u{0e31}', '\u{094d}', '\u{200d}'] {
            assert_eq!(char_width(character), 0, "{character:?}");
        }
    }

    #[test]
    fn a_string_is_as_wide_as_ratatui_draws_it() {
        for (text, cells) in [
            ("✅", 2),
            ("🚀 go", 5),
            ("❤\u{fe0f}", 2),
            ("👨\u{200d}👩\u{200d}👧", 2),
            ("한글abc", 7),
            ("e\u{301}", 1),
            ("ﾊﾟ", 2),
            ("a·b", 3),
            ("\u{e68b} main.rs", 9),
        ] {
            assert_eq!(string_width(text), cells, "{text:?}");
        }
    }

    #[test]
    fn control_characters_take_no_cells_in_a_string() {
        assert_eq!(string_width("a\tb\nc\u{7}"), 3);
    }

    #[test]
    fn truncation_never_splits_wide_characters() {
        assert_eq!(truncate_to_width("한글", 3), "한");
        assert_eq!(truncate_to_width("한글", 4), "한글");
        assert_eq!(truncate_to_width("한", 0), "");
    }

    #[test]
    fn a_long_text_is_cut_from_the_left_with_an_ellipsis() {
        assert_eq!(tail_to_width("src/a.rs", 8), "src/a.rs");
        assert_eq!(tail_to_width("src/a.rs", 5), "…a.rs");
        assert_eq!(tail_to_width("한글한", 4), "…한");
    }
}
