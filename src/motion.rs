//! vim's `w`, `b` and `e` over the lines of a message, for the cursor of the message pane.
//!
//! A position is a line and the byte its character starts at. An empty line has one position, byte 0.

/// A line, from 0, and a byte of it at the start of a character.
pub type Pos = (usize, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A space, or the break between two lines.
    Space,
    /// An empty line: `w` and `b` stop on it, `e` does not.
    Empty,
    /// Letters, digits and `_`.
    Word,
    /// Any other character that is not a space.
    Other,
}

fn class(character: char) -> Class {
    if character.is_whitespace() {
        Class::Space
    } else if character.is_alphanumeric() || character == '_' {
        Class::Word
    } else {
        Class::Other
    }
}

/// Every position of the message with its class, a line break between two lines included as a space
/// at the end of the line before it.
fn cells(lines: &[String]) -> Vec<(Pos, Class)> {
    let mut out = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        if line > 0 {
            out.push(((line - 1, usize::MAX), Class::Space));
        }
        if text.is_empty() {
            out.push(((line, 0), Class::Empty));
        }
        out.extend(
            text.char_indices()
                .map(|(byte, character)| ((line, byte), class(character))),
        );
    }
    out
}

/// The position of cell `at`, or the last character of the message when `at` is past it.
fn land(cells: &[(Pos, Class)], at: usize) -> Option<Pos> {
    cells
        .get(at)
        .or_else(|| cells.iter().rev().find(|(pos, _)| pos.1 != usize::MAX))
        .map(|(pos, _)| *pos)
}

fn index(cells: &[(Pos, Class)], pos: Pos) -> Option<usize> {
    cells.iter().position(|(at, _)| *at == pos)
}

/// `w`: the start of the next word, or the next empty line.
pub fn word_next(lines: &[String], pos: Pos) -> Pos {
    let cells = cells(lines);
    let class_at = |at: usize| cells.get(at).map(|(_, class)| *class);
    let Some(mut at) = index(&cells, pos) else {
        return pos;
    };
    match class_at(at) {
        Some(own @ (Class::Word | Class::Other)) => {
            while class_at(at) == Some(own) {
                at += 1;
            }
        }
        Some(Class::Empty) => at += 1,
        _ => {}
    }
    while class_at(at) == Some(Class::Space) {
        at += 1;
    }
    land(&cells, at).unwrap_or(pos)
}

/// `e`: the end of this word when the cursor is inside one, else of the next.
pub fn word_end(lines: &[String], pos: Pos) -> Pos {
    let cells = cells(lines);
    let class_at = |at: usize| cells.get(at).map(|(_, class)| *class);
    let Some(mut at) = index(&cells, pos) else {
        return pos;
    };
    at += 1;
    while matches!(class_at(at), Some(Class::Space | Class::Empty)) {
        at += 1;
    }
    let Some(own) = class_at(at) else {
        return land(&cells, at).unwrap_or(pos);
    };
    while class_at(at + 1) == Some(own) {
        at += 1;
    }
    land(&cells, at).unwrap_or(pos)
}

/// `b`: the start of this word when the cursor is inside one past its start, else of the one before,
/// or the empty line before.
pub fn word_prev(lines: &[String], pos: Pos) -> Pos {
    let cells = cells(lines);
    let class_at = |at: usize| cells.get(at).map(|(_, class)| *class);
    let Some(mut at) = index(&cells, pos) else {
        return pos;
    };
    if at == 0 {
        return pos;
    }
    at -= 1;
    while at > 0 && class_at(at) == Some(Class::Space) {
        at -= 1;
    }
    if let Some(own @ (Class::Word | Class::Other)) = class_at(at) {
        while at > 0 && class_at(at - 1) == Some(own) {
            at -= 1;
        }
    }
    land(&cells, at).unwrap_or(pos)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_owned).collect()
    }

    /// Every stop of `step` from the start until it stops moving.
    fn walk(text: &str, step: fn(&[String], Pos) -> Pos, from: Pos) -> Vec<Pos> {
        let lines = lines(text);
        let mut stops = vec![from];
        loop {
            let next = step(&lines, *stops.last().unwrap_or(&from));
            if stops.contains(&next) {
                return stops;
            }
            stops.push(next);
        }
    }

    #[test]
    fn w_stops_at_words_and_runs_of_punctuation() {
        // foo.bar() baz
        // 0  34  7  10
        assert_eq!(
            walk("foo.bar() baz", word_next, (0, 0)),
            [(0, 0), (0, 3), (0, 4), (0, 7), (0, 10), (0, 12)],
            "the last word goes on to the last character, as in vim"
        );
    }

    #[test]
    fn w_crosses_lines_and_stops_at_an_empty_line() {
        assert_eq!(
            walk("ab cd\n\n  ef", word_next, (0, 0)),
            [(0, 0), (0, 3), (1, 0), (2, 2), (2, 3)]
        );
    }

    #[test]
    fn e_goes_to_word_ends_and_skips_empty_lines() {
        assert_eq!(
            walk("ab cd\n\nef.", word_end, (0, 0)),
            [(0, 0), (0, 1), (0, 4), (2, 1), (2, 2)]
        );
    }

    #[test]
    fn b_goes_back_to_word_starts_and_stops_at_an_empty_line() {
        assert_eq!(
            walk("ab cd\n\nef.", word_prev, (2, 2)),
            [(2, 2), (2, 0), (1, 0), (0, 3), (0, 0)]
        );
        assert_eq!(word_prev(&lines("abc"), (0, 2)), (0, 0), "inside a word");
    }

    #[test]
    fn the_message_ends_stop_the_cursor() {
        let text = lines("one two");
        assert_eq!(word_next(&text, (0, 4)), (0, 6), "w on the last word");
        assert_eq!(word_end(&text, (0, 6)), (0, 6));
        assert_eq!(word_prev(&text, (0, 0)), (0, 0));
    }

    #[test]
    fn wide_characters_are_one_position() {
        assert_eq!(word_next(&lines("日本 語"), (0, 0)), (0, 7));
        assert_eq!(word_end(&lines("日本 語"), (0, 0)), (0, 3));
    }
}
