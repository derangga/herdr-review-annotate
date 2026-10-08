//! Which words of a removed line and of the added line that replaced it differ.

use std::ops::Range;

/// A line with more tokens than this is not compared, since the table of the comparison has one
/// cell for every pair of tokens.
const MAX_TOKENS: usize = 200;

/// The byte ranges of a line's text that the line it is paired with does not have.
pub type Marks = Vec<Range<usize>>;

#[derive(PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class(character: char) -> Class {
    if character.is_alphanumeric() || character == '_' {
        Class::Word
    } else if character.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

fn is_space(token: &str) -> bool {
    token.starts_with(char::is_whitespace)
}

/// `text` cut into tokens: a run of letters, digits and underscores, a run of spaces, and every
/// other character on its own.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let kind = class(first);
        let len = if kind == Class::Other {
            first.len_utf8()
        } else {
            rest.find(|other| class(other) != kind)
                .unwrap_or(rest.len())
        };
        let (token, tail) = rest.split_at(len);
        out.push(token);
        rest = tail;
    }
    out
}

/// For each token of `a` and of `b`, whether it is part of their longest common subsequence.
fn common(a: &[&str], b: &[&str]) -> (Vec<bool>, Vec<bool>) {
    let width = b.len() + 1;
    // `table[i * width + j]` is the length of the longest common subsequence of `a[i..]` and
    // `b[j..]`. A line has at most `MAX_TOKENS` tokens, so a length fits.
    let mut table = vec![0_u16; (a.len() + 1) * width];
    let at = |table: &[u16], i: usize, j: usize| table.get(i * width + j).copied().unwrap_or(0);
    for (i, left) in a.iter().enumerate().rev() {
        for (j, right) in b.iter().enumerate().rev() {
            let length = if left == right {
                at(&table, i + 1, j + 1) + 1
            } else {
                at(&table, i + 1, j).max(at(&table, i, j + 1))
            };
            if let Some(cell) = table.get_mut(i * width + j) {
                *cell = length;
            }
        }
    }
    let (mut in_a, mut in_b) = (vec![false; a.len()], vec![false; b.len()]);
    let (mut i, mut j) = (0, 0);
    while let (Some(left), Some(right)) = (a.get(i), b.get(j)) {
        if left == right {
            for (kept, index) in [(&mut in_a, i), (&mut in_b, j)] {
                if let Some(kept) = kept.get_mut(index) {
                    *kept = true;
                }
            }
            (i, j) = (i + 1, j + 1);
        } else if at(&table, i + 1, j) >= at(&table, i, j + 1) {
            i += 1;
        } else {
            j += 1;
        }
    }
    (in_a, in_b)
}

/// The byte ranges of the tokens that are not `kept`, with neighbours joined into one range.
fn marks(tokens: &[&str], kept: &[bool]) -> Marks {
    let mut out = Marks::new();
    let mut at = 0;
    for (token, kept) in tokens.iter().zip(kept) {
        let end = at + token.len();
        match out.last_mut() {
            _ if *kept => {}
            Some(last) if last.end == at => last.end = end,
            _ => out.push(at..end),
        }
        at = end;
    }
    out
}

/// What to mark in a removed line `old` and in the added line `new` paired with it: the tokens of
/// each that are not in the longest run of tokens the two share in order. `None` when the pair
/// gets no marks: a line has too many tokens to compare, or under half of the tokens that are not
/// spaces are common to both, so the line was rewritten and marking most of it says nothing.
pub fn changed(old: &str, new: &str) -> Option<(Marks, Marks)> {
    let (a, b) = (tokens(old), tokens(new));
    if a.len() > MAX_TOKENS || b.len() > MAX_TOKENS {
        return None;
    }
    // Most pairs differ in a few tokens, so the table is built for what lies between the tokens
    // they start with and end with in common.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a
        .iter()
        .skip(head)
        .rev()
        .zip(b.iter().skip(head).rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (in_a, in_b) = common(a.get(head..a.len() - tail)?, b.get(head..b.len() - tail)?);
    let whole = |middle: Vec<bool>| {
        let mut kept = vec![true; head];
        kept.extend(middle);
        kept.extend(std::iter::repeat_n(true, tail));
        kept
    };
    let (in_a, in_b) = (whole(in_a), whole(in_b));
    let words = |tokens: &[&str]| tokens.iter().filter(|token| !is_space(token)).count();
    let shared = a
        .iter()
        .zip(&in_a)
        .filter(|(token, kept)| **kept && !is_space(token))
        .count();
    if shared * 2 < words(&a).max(words(&b)) {
        return None;
    }
    Some((marks(&a, &in_a), marks(&b, &in_b)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn marked(old: &str, new: &str) -> Option<(Vec<String>, Vec<String>)> {
        let (a, b) = changed(old, new)?;
        let cut = |text: &str, marks: Marks| {
            marks
                .into_iter()
                .map(|range| text.get(range).unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        Some((cut(old, a), cut(new, b)))
    }

    fn pair(old: &[&str], new: &[&str]) -> (Vec<String>, Vec<String>) {
        let owned = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect();
        (owned(old), owned(new))
    }

    #[test]
    fn a_line_is_cut_into_words_runs_of_spaces_and_single_other_characters() {
        assert_eq!(
            tokens("  let foo_bar2 = é(x)->y;"),
            [
                "  ", "let", " ", "foo_bar2", " ", "=", " ", "é", "(", "x", ")", "-", ">", "y", ";"
            ]
        );
        assert!(tokens("").is_empty());
    }

    #[test]
    fn a_renamed_identifier_is_marked_whole_on_both_lines() {
        assert_eq!(
            marked("let total = count(items);", "let total = count_all(items);"),
            Some(pair(&["count"], &["count_all"]))
        );
    }

    #[test]
    fn tokens_added_to_one_line_mark_nothing_on_the_other() {
        assert_eq!(
            marked("call(a, b)", "call(a, b, c)"),
            Some(pair(&[], &[", c"]))
        );
    }

    #[test]
    fn changed_tokens_next_to_each_other_are_one_mark() {
        assert_eq!(
            marked("x = a.b(1, 2, 3)", "x = c-d(1, 2, 3)"),
            Some(pair(&["a.b"], &["c-d"]))
        );
    }

    #[test]
    fn a_change_of_indent_marks_the_spaces() {
        assert_eq!(
            marked("  done()", "      done()"),
            Some(pair(&["  "], &["      "]))
        );
    }

    #[test]
    fn a_pair_with_under_half_of_its_words_in_common_has_no_marks() {
        assert_eq!(marked("let x = 1;", "return compute(input, 2);"), None);
        // Three of the six tokens that are not spaces are shared, which is half.
        assert!(marked("a b c d e f", "a b c x y z").is_some());
        assert_eq!(marked("a b c d e f", "a b w x y z"), None);
        // The longer line decides.
        assert_eq!(marked("a", "a b c"), None);
    }

    #[test]
    fn a_line_with_too_many_tokens_has_no_marks() {
        let long = "a ".repeat(MAX_TOKENS / 2);
        assert!(changed(&long, &format!("{long}b")).is_none());
        let fits = "a ".repeat(MAX_TOKENS / 2 - 1);
        assert!(changed(&fits, &format!("{fits}b")).is_some());
    }

    #[test]
    fn two_equal_lines_have_no_marks_and_a_repeated_token_is_matched_once() {
        assert_eq!(marked("same", "same"), Some(pair(&[], &[])));
        assert_eq!(marked("a a a b", "a a b"), Some(pair(&["a "], &[])));
    }
}
