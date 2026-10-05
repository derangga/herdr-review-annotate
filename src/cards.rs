//! A thread as lines of text: the card under a diff line, or in the not-in-diff block.
//!
//! The height of a card is its number of lines, so the stream can number its rows before
//! anything is drawn. Every string from the store passes through `sanitize_terminal_text`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::diff::Placement;
use crate::store::{Anchor, AnchorTarget, Author, Side, Status, Thread};
use crate::theme::Theme;
use crate::tui::sanitize_terminal_text;
use crate::width::{char_width, string_width, truncate_to_width};

/// The width of the line-number gutter of a diff row: two numbers and a space.
pub const GUTTER: usize = 11;

/// A stream narrower than this draws cards with a small indent instead of lining up under the code.
const WIDE: usize = 50;

/// How far a card, and the editor, are indented in a stream `width` cells wide.
pub fn indent(width: usize) -> usize {
    if width >= WIDE { GUTTER } else { 2 }
}

/// The lines of a card, and which comment of the thread each line belongs to: 0 is the root and
/// 1 is the first reply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    pub lines: Vec<Line<'static>>,
    pub owners: Vec<usize>,
}

impl Card {
    fn push(&mut self, line: Line<'static>, owner: usize) {
        self.lines.push(line);
        self.owners.push(owner);
    }
}

/// `text` cut into lines of at most `width` cells. It breaks at newlines, then after spaces, and
/// inside a word longer than a line. Leading spaces stay, so indented code keeps its shape.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for token in paragraph.split_inclusive(' ') {
            let word = string_width(token.trim_end_matches(' '));
            if used > 0 && used + word > width {
                lines.push(std::mem::take(&mut line).trim_end().to_owned());
                used = 0;
            }
            if word <= width {
                line.push_str(token);
                used += string_width(token);
                continue;
            }
            for character in token.chars() {
                let cells = char_width(character);
                if used + cells > width && used > 0 {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(character);
                used += cells;
            }
        }
        lines.push(line);
    }
    lines
}

fn name(author: &Author) -> String {
    sanitize_terminal_text(&author.to_string())
}

fn first_line(text: &str) -> String {
    sanitize_terminal_text(text.lines().next().unwrap_or_default())
}

fn side_letter(side: Side) -> char {
    match side {
        Side::Old => 'L',
        Side::New => 'R',
    }
}

/// Where a comment points, as the send prompt writes it: `src/lib.rs:42 (R)`.
pub fn location(anchor: &Anchor) -> String {
    let path = sanitize_terminal_text(anchor.path.as_str());
    match &anchor.target {
        AnchorTarget::File => format!("{path} (file)"),
        AnchorTarget::Line { side, line, .. } => format!("{path}:{line} ({})", side_letter(*side)),
        AnchorTarget::Range {
            side, start, end, ..
        } => format!("{path}:{start}-{end} ({})", side_letter(*side)),
    }
}

/// The text of the commented line as it was when the comment was written.
fn was(anchor: &Anchor) -> Option<&str> {
    match &anchor.target {
        AnchorTarget::File => None,
        AnchorTarget::Line { text, .. } | AnchorTarget::Range { text, .. } => Some(text),
    }
}

/// The lines of one thread at `width` cells. A resolved thread is one line, which says `[new]` while
/// an agent's resolve has not been looked at. An open thread is its
/// header, the body, and each reply indented under it. The `outdated` tag is on open threads only,
/// and a thread that is not in the diff says where it pointed.
pub fn card(thread: &Thread, placement: Placement, width: usize, theme: &Theme) -> Card {
    let dim = || theme.dim();
    let indent = indent(width);
    let pad = " ".repeat(indent);
    let id = thread.root.id.to_string();
    let in_block = placement == Placement::NotInDiff;
    let outdated = matches!(placement, Placement::Outdated { .. });
    if let Status::Resolved { by } = &thread.status {
        let last = thread.comments().last().unwrap_or(&thread.root);
        let place = if in_block {
            format!(" {}", location(&thread.anchor))
        } else {
            String::new()
        };
        let head = format!("{pad}✓ {id}");
        let tag = if thread.is_new { " [new]" } else { "" };
        let rest = format!(
            "{place} resolved by {}: {}",
            name(by),
            first_line(&last.body)
        );
        let left = width.saturating_sub(string_width(&head) + string_width(tag));
        let mut card = Card::default();
        card.push(
            Line::from(vec![
                Span::styled(head, dim()),
                Span::styled(
                    tag,
                    Style::new().fg(theme.success).add_modifier(Modifier::BOLD),
                ),
                Span::styled(truncate_to_width(&rest, left), dim()),
            ]),
            thread.replies.len(),
        );
        return card;
    }
    let room = width.saturating_sub(indent + 2).max(8);
    let color = if thread.root.author.is_user() {
        theme.accent
    } else {
        theme.agent
    };
    let bar = Style::new().fg(color);
    let row = |spans: Vec<Span<'static>>| {
        let mut line = vec![Span::raw(pad.clone()), Span::styled("│ ", bar)];
        line.extend(spans);
        Line::from(line)
    };
    let mut header = vec![Span::styled(
        format!("{id} {}", name(&thread.root.author)),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    if outdated {
        header.push(Span::styled(" [outdated]", Style::new().fg(theme.warning)));
    }
    if thread.root.edited_since_sent {
        header.push(Span::styled(" (edited since sent)", dim()));
    }
    let mut card = Card::default();
    card.push(row(header), 0);
    if in_block {
        let place = truncate_to_width(&location(&thread.anchor), room);
        card.push(row(vec![Span::styled(place, dim())]), 0);
    }
    if let Some(text) = was(&thread.anchor)
        && (in_block || outdated)
    {
        let text = format!("was: {}", sanitize_terminal_text(text.trim()));
        card.push(
            row(vec![Span::styled(truncate_to_width(&text, room), dim())]),
            0,
        );
    }
    for text in wrap(&sanitize_terminal_text(&thread.root.body), room) {
        card.push(row(vec![Span::raw(text)]), 0);
    }
    for (owner, reply) in (1..).zip(&thread.replies) {
        let mark = if reply.edited_since_sent {
            " (edited since sent)"
        } else {
            ""
        };
        let text = format!(
            "{}{mark}: {}",
            name(&reply.author),
            sanitize_terminal_text(&reply.body)
        );
        for (index, text) in wrap(&text, room.saturating_sub(4).max(1))
            .into_iter()
            .enumerate()
        {
            let lead = if index == 0 { "  ↳ " } else { "    " };
            card.push(row(vec![Span::raw(format!("{lead}{text}"))]), owner);
        }
    }
    card
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::store::{Comment, CommentId, RelPath, Spec};

    fn comment(id: &str, author: Author, body: &str) -> Comment {
        Comment {
            id: CommentId::parse(id).unwrap(),
            parent: None,
            author,
            body: body.into(),
            sent_batch: None,
            edited_since_sent: false,
        }
    }

    fn thread(body: &str) -> Thread {
        Thread {
            root: comment("u1", Author::User, body),
            anchor: Anchor {
                path: RelPath::parse("src/a.rs").unwrap(),
                old_path: None,
                target: AnchorTarget::Line {
                    side: Side::New,
                    line: 7,
                    text: "let x = 1;".into(),
                },
                spec: Spec::WorkTree,
            },
            replies: Vec::new(),
            status: Status::Open,
            is_new: false,
            reopened: false,
            unsent: true,
        }
    }

    fn text(card: &Card) -> Vec<String> {
        card.lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn wrap_breaks_at_newlines_spaces_and_inside_long_words() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("a\n\nb", 9), ["a", "", "b"]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("ab abcdefg", 4), ["ab", "abcd", "efg"]);
        assert_eq!(wrap("", 4), [""]);
    }

    #[test]
    fn wrap_keeps_indentation_and_counts_wide_characters_in_cells() {
        assert_eq!(wrap("    let a = 1;", 20), ["    let a = 1;"]);
        assert_eq!(wrap("한글한", 5), ["한글", "한"]);
        assert!(wrap("x y z", 0).iter().all(|line| string_width(line) <= 1));
    }

    #[test]
    fn an_open_card_is_a_header_the_body_and_indented_replies() {
        let mut thread = thread("first\nsecond");
        thread.replies.push(comment(
            "a1",
            Author::Agent(Some("claude".into())),
            "done, one two three four five six",
        ));
        let lines = card(
            &thread,
            Placement::Matched { line: Some(7) },
            40,
            &Theme::default(),
        );
        assert_eq!(
            text(&lines),
            [
                "  │ u1 user",
                "  │ first",
                "  │ second",
                "  │   ↳ agent:claude: done, one two",
                "  │     three four five six",
            ]
        );
    }

    #[test]
    fn a_wide_stream_lines_the_card_up_under_the_code() {
        let lines = card(
            &thread("x"),
            Placement::Matched { line: Some(7) },
            60,
            &Theme::default(),
        );
        assert_eq!(text(&lines)[1], format!("{}│ x", " ".repeat(GUTTER)));
    }

    #[test]
    fn the_outdated_tag_and_the_old_line_text_show_on_an_open_thread_only() {
        let mut open = thread("fix");
        let placement = Placement::Outdated { near: Some(3) };
        let lines = text(&card(&open, placement, 40, &Theme::default()));
        assert_eq!(lines[0], "  │ u1 user [outdated]");
        assert_eq!(lines[1], "  │ was: let x = 1;");
        open.status = Status::Resolved {
            by: Author::Agent(Some("claude".into())),
        };
        open.replies
            .push(comment("a1", Author::Agent(None), "Added with_capacity"));
        let lines = text(&card(&open, placement, 80, &Theme::default()));
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("outdated"), "{}", lines[0]);
        assert!(
            lines[0].ends_with("✓ u1 resolved by agent:claude: Added with_capacity"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn a_resolved_thread_says_new_until_it_has_been_seen() {
        let mut thread = thread("fix");
        thread.status = Status::Resolved {
            by: Author::Agent(Some("claude".into())),
        };
        thread
            .replies
            .push(comment("a1", Author::Agent(None), "Added with_capacity"));
        thread.is_new = true;
        let placement = Placement::Matched { line: Some(7) };
        let new = text(&card(&thread, placement, 80, &Theme::default()));
        assert_eq!(
            new,
            ["           ✓ u1 [new] resolved by agent:claude: Added with_capacity"]
        );
        thread.is_new = false;
        let seen = text(&card(&thread, placement, 80, &Theme::default()));
        assert_eq!(
            seen,
            ["           ✓ u1 resolved by agent:claude: Added with_capacity"]
        );
        // The tag survives a cut: it is before what gets truncated.
        thread.is_new = true;
        assert!(text(&card(&thread, placement, 30, &Theme::default()))[0].contains("[new]"));
        assert!(string_width(&text(&card(&thread, placement, 30, &Theme::default()))[0]) <= 30);
    }

    #[test]
    fn a_thread_not_in_the_diff_says_where_it_pointed() {
        let lines = text(&card(
            &thread("fix"),
            Placement::NotInDiff,
            40,
            &Theme::default(),
        ));
        assert_eq!(
            lines,
            [
                "  │ u1 user",
                "  │ src/a.rs:7 (R)",
                "  │ was: let x = 1;",
                "  │ fix"
            ]
        );
        let mut range = thread("fix");
        range.anchor.target = AnchorTarget::Range {
            side: Side::Old,
            start: 3,
            end: 5,
            text: "x".into(),
        };
        assert_eq!(
            text(&card(&range, Placement::NotInDiff, 40, &Theme::default()))[1],
            "  │ src/a.rs:3-5 (L)"
        );
        let mut file = thread("fix");
        file.anchor.target = AnchorTarget::File;
        let lines = text(&card(&file, Placement::NotInDiff, 40, &Theme::default()));
        assert_eq!(lines[1], "  │ src/a.rs (file)");
        assert_eq!(lines.len(), 3, "a file comment has no line text to show");
        file.status = Status::Resolved { by: Author::User };
        assert_eq!(
            text(&card(&file, Placement::NotInDiff, 60, &Theme::default()))[0],
            format!(
                "{}✓ u1 src/a.rs (file) resolved by user: fix",
                " ".repeat(GUTTER)
            )
        );
    }

    #[test]
    fn an_edited_comment_says_so_and_every_string_loses_its_control_characters() {
        let mut thread = thread("a\u{1b}[2Jb");
        thread.root.edited_since_sent = true;
        let mut reply = comment("a1", Author::Agent(Some("e\u{1b}vil".into())), "x\u{9b}y");
        reply.edited_since_sent = true;
        thread.replies.push(reply);
        let lines = text(&card(
            &thread,
            Placement::Matched { line: Some(7) },
            80,
            &Theme::default(),
        ));
        assert!(
            lines[0].ends_with("u1 user (edited since sent)"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("a[2Jb"));
        assert!(
            lines[2].contains("agent:evil (edited since sent): xy"),
            "{}",
            lines[2]
        );
        assert!(!lines.iter().any(|line| line.chars().any(char::is_control)));
    }

    #[test]
    fn a_narrow_stream_still_gets_a_card() {
        let card = card(
            &thread("a long comment body here"),
            Placement::NotInDiff,
            4,
            &Theme::default(),
        );
        assert!(card.lines.len() >= 4);
    }

    #[test]
    fn every_line_knows_which_comment_it_belongs_to() {
        let mut thread = thread("first\nsecond");
        thread
            .replies
            .push(comment("a1", Author::Agent(None), "one"));
        thread.replies.push(comment("u2", Author::User, "two"));
        let placement = Placement::Outdated { near: Some(1) };
        // Header, old text, two body lines, then a line per reply.
        assert_eq!(
            card(&thread, placement, 40, &Theme::default()).owners,
            [0, 0, 0, 0, 1, 2]
        );
        // A resolved thread is one line, which shows its last comment.
        thread.status = Status::Resolved { by: Author::User };
        assert_eq!(card(&thread, placement, 40, &Theme::default()).owners, [2]);
    }
}
