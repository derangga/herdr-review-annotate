//! A thread as lines of text: the card under a diff line, or in the not-in-diff block.
//!
//! The height of a card is its number of lines, so the stream can number its rows before
//! anything is drawn. Every string from the store passes through `sanitize_terminal_text`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::diff::Placement;
use crate::keymap::{Action, Keymap};
use crate::store::{Anchor, AnchorTarget, Author, Side, Status, Thread};
use crate::theme::Theme;
use crate::tui::sanitize_terminal_text;
use crate::width::{char_width, string_width, tail_to_width, truncate_to_width};

/// What a card is drawn with besides its thread: the colours, the keys its bottom border names,
/// and the time its age is counted from, as RFC 3339.
#[derive(Debug, Clone, Copy)]
pub struct Look<'a> {
    pub theme: &'a Theme,
    pub keymap: &'a Keymap,
    pub now: &'a str,
}

/// The lines of a card, and which comment of the thread each line belongs to: 0 is the root and
/// 1 is the first reply. The two borders of a box belong to the root.
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

/// How long before `now` the time `at` was: `now` under a minute, then `2m`, `3h`, `2d`. Empty
/// when either is not RFC 3339. A time after `now` is `now`.
pub fn ago(at: &str, now: &str) -> String {
    let parse = |time: &str| chrono::DateTime::parse_from_rfc3339(time).ok();
    let Some((now, at)) = parse(now).zip(parse(at)) else {
        return String::new();
    };
    match (now - at).num_seconds() {
        ..60 => "now".to_owned(),
        seconds @ 60..3600 => format!("{}m", seconds / 60),
        seconds @ 3600..86_400 => format!("{}h", seconds / 3600),
        seconds => format!("{}d", seconds / 86_400),
    }
}

/// The keys on the bottom border of a box: reply for any thread, and edit and delete when the
/// thread is the user's. Each is the action's first key, and an action with no key is left out.
fn footer(keymap: &Keymap, own: bool) -> String {
    let actions: &[(Action, &str)] = if own {
        &[
            (Action::Reply, "reply"),
            (Action::Edit, "edit"),
            (Action::Delete, "delete"),
        ]
    } else {
        &[(Action::Reply, "reply")]
    };
    let keys = actions
        .iter()
        .filter_map(|(action, what)| Some(format!("{} {what}", keymap.keys(*action).first()?)))
        .collect::<Vec<_>>();
    sanitize_terminal_text(&keys.join("  "))
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

/// Where a comment points, as the title of a box writes it: `src/lib.rs R42`, `src/lib.rs L3-5`,
/// or the bare path for a file comment.
pub fn place(anchor: &Anchor) -> String {
    let path = sanitize_terminal_text(anchor.path.as_str());
    match &anchor.target {
        AnchorTarget::File => path,
        AnchorTarget::Line { side, line, .. } => format!("{path} {}{line}", side_letter(*side)),
        AnchorTarget::Range {
            side, start, end, ..
        } => format!("{path} {}{start}-{end}", side_letter(*side)),
    }
}

/// The text of the commented line as it was when the comment was written.
fn was(anchor: &Anchor) -> Option<&str> {
    match &anchor.target {
        AnchorTarget::File => None,
        AnchorTarget::Line { text, .. } | AnchorTarget::Range { text, .. } => Some(text),
    }
}

/// The one line of a resolved thread: a green check, its id, `[new]` while an agent's resolve has
/// not been looked at, where it pointed when it is not in the diff, who resolved it, and the first
/// line of its last comment.
fn resolved(thread: &Thread, by: &Author, in_block: bool, width: usize, theme: &Theme) -> Card {
    let last = thread.comments().last().unwrap_or(&thread.root);
    let place = if in_block {
        format!(" {}", place(&thread.anchor))
    } else {
        String::new()
    };
    let id = format!(" {}", thread.root.id);
    let tag = if thread.is_new { " [new]" } else { "" };
    let rest = format!(
        "{place} resolved by {}: {}",
        name(by),
        first_line(&last.body)
    );
    let left = width.saturating_sub(1 + string_width(&id) + string_width(tag));
    let mut card = Card::default();
    card.push(
        Line::from(vec![
            Span::styled("✓", Style::new().fg(theme.success)),
            Span::styled(id, theme.dim()),
            Span::styled(
                tag,
                Style::new().fg(theme.success).add_modifier(Modifier::BOLD),
            ),
            Span::styled(truncate_to_width(&rest, left), theme.dim()),
        ]),
        thread.replies.len(),
    );
    card
}

/// The top border of a box: a bullet, who wrote the note, how long ago, where it points, and the
/// badges. The place is cut from the left when the line is too long, and left out when there is
/// no room for it at all.
fn top(thread: &Thread, outdated: bool, width: usize, look: &Look, border: Style) -> Line<'static> {
    let theme = look.theme;
    let author = match &thread.root.author {
        Author::User => "Your note".to_owned(),
        Author::Agent(None) => "agent".to_owned(),
        Author::Agent(Some(name)) => sanitize_terminal_text(name),
    };
    let age = ago(&thread.root.at, look.now);
    let mut head = vec![
        Span::styled("● ", border),
        Span::styled(author, border.add_modifier(Modifier::BOLD)),
    ];
    if !age.is_empty() {
        head.push(Span::styled(format!(" · {age}"), theme.dim()));
    }
    let mut badges = Vec::new();
    if outdated {
        badges.push(Span::styled(" [outdated]", Style::new().fg(theme.warning)));
    }
    if thread.is_new {
        badges.push(Span::styled(
            " [new]",
            Style::new().fg(theme.success).add_modifier(Modifier::BOLD),
        ));
    }
    if thread.root.edited_since_sent {
        badges.push(Span::styled(" (edited since sent)", theme.dim()));
    }
    if thread.unsent {
        badges.push(Span::styled(" [unsent]", Style::new().fg(theme.accent)));
    }
    let cells = |spans: &[Span]| -> usize { spans.iter().map(|s| string_width(&s.content)).sum() };
    // Two corners, a space after the first and before the last, and ` · ` before the place.
    let room = width.saturating_sub(4 + 3 + cells(&head) + cells(&badges));
    if room > 1 {
        let place = tail_to_width(&place(&thread.anchor), room);
        head.push(Span::styled(format!(" · {place}"), theme.dim()));
    }
    head.extend(badges);
    let fill = width.saturating_sub(4 + cells(&head));
    let mut line = vec![Span::styled("╭ ", border)];
    line.extend(head);
    line.push(Span::styled(format!(" {}╮", "─".repeat(fill)), border));
    Line::from(line)
}

/// The lines of one thread at `width` cells. A resolved thread is one line. An open thread is a
/// rounded box as wide as the stream: the top border says who wrote it, when and where, the body
/// and each reply are wrapped inside, and the bottom border names the keys that act on it. The
/// `outdated` tag is on open threads only, and an outdated thread, or one that is not in the diff,
/// says what its line was.
pub fn card(thread: &Thread, placement: Placement, width: usize, look: &Look) -> Card {
    let theme = look.theme;
    let in_block = placement == Placement::NotInDiff;
    let outdated = matches!(placement, Placement::Outdated { .. });
    if let Status::Resolved { by } = &thread.status {
        return resolved(thread, by, in_block, width, theme);
    }
    let own = thread.root.author.is_user();
    let border = Style::new().fg(if own { theme.warning } else { theme.agent });
    // A side and a space at each end of a row.
    let room = width.saturating_sub(4).max(8);
    let row = |text: String, style: Style| {
        let pad = " ".repeat(room.saturating_sub(string_width(&text)));
        Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(text, style),
            Span::styled(format!("{pad} │"), border),
        ])
    };
    let mut card = Card::default();
    card.push(top(thread, outdated, width, look, border), 0);
    if let Some(text) = was(&thread.anchor)
        && (in_block || outdated)
    {
        let text = format!("was: {}", sanitize_terminal_text(text.trim()));
        card.push(row(truncate_to_width(&text, room), theme.dim()), 0);
    }
    for text in wrap(&sanitize_terminal_text(&thread.root.body), room) {
        card.push(row(text, Style::new()), 0);
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
            card.push(row(format!("{lead}{text}"), Style::new()), owner);
        }
    }
    let keys = footer(look.keymap, own);
    let keys = if keys.is_empty() {
        String::new()
    } else {
        format!(" {keys} ")
    };
    let fill = width.saturating_sub(2 + string_width(&keys));
    card.push(
        Line::from(vec![
            Span::styled(format!("╰{}", "─".repeat(fill)), border),
            Span::styled(keys, theme.dim()),
            Span::styled("╯", border),
        ]),
        0,
    );
    card
}

#[cfg(test)]
impl Look<'static> {
    /// Mocha, the default keys, and ten minutes past midnight on 2026-10-05.
    pub fn test() -> Self {
        use std::sync::LazyLock;
        static THEME: LazyLock<Theme> = LazyLock::new(Theme::default);
        static KEYMAP: LazyLock<Keymap> = LazyLock::new(Keymap::default);
        Self {
            theme: &THEME,
            keymap: &KEYMAP,
            now: "2026-10-05T00:10:00Z",
        }
    }
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
            at: "2026-10-05T00:00:00Z".into(),
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

    /// A user thread on `src/a.rs` line 7, written ten minutes before `Look::test` says it is.
    fn boxed(thread: &Thread, placement: Placement, width: usize) -> Vec<String> {
        let lines = text(&card(thread, placement, width, &Look::test()));
        for line in &lines {
            assert_eq!(string_width(line), width, "{line}");
        }
        lines
    }

    const MATCHED: Placement = Placement::Matched { line: Some(7) };

    #[test]
    fn an_open_card_is_a_box_with_the_body_and_indented_replies() {
        let mut thread = thread("first\nsecond");
        thread.replies.push(comment(
            "a1",
            Author::Agent(Some("claude".into())),
            "done, one two three four five six",
        ));
        assert_eq!(
            boxed(&thread, MATCHED, 50),
            [
                "╭ ● Your note · 10m · src/a.rs R7 [unsent] ──────╮",
                "│ first                                          │",
                "│ second                                         │",
                "│   ↳ agent:claude: done, one two three four     │",
                "│     five six                                   │",
                "╰───────────────────── r reply  e edit  d delete ╯",
            ]
        );
    }

    #[test]
    fn a_card_is_as_wide_as_the_stream_in_both_layouts_with_no_indent() {
        // 60 cells is the stream of an 80 column pane, 98 that of a 130 column one.
        for width in [60, 98] {
            let lines = boxed(&thread("x"), MATCHED, width);
            assert!(lines[0].starts_with("╭ ● Your note"), "{}", lines[0]);
            assert!(lines[1].starts_with("│ x  "), "{}", lines[1]);
            assert!(lines[1].ends_with(" │"), "{}", lines[1]);
            assert!(lines[2].starts_with("╰──"), "{}", lines[2]);
        }
    }

    #[test]
    fn the_box_of_the_user_is_the_warning_colour_and_an_agent_s_is_the_agent_colour() {
        let theme = Theme::default();
        let user = card(&thread("x"), MATCHED, 60, &Look::test());
        let mut theirs = thread("x");
        theirs.root = comment("a1", Author::Agent(Some("claude".into())), "x");
        let agent = card(&theirs, MATCHED, 60, &Look::test());
        for (card, color) in [(user, theme.warning), (agent, theme.agent)] {
            for line in &card.lines {
                assert_eq!(line.spans[0].style.fg, Some(color), "{line}");
                assert_eq!(line.spans.last().unwrap().style.fg, Some(color), "{line}");
            }
        }
    }

    #[test]
    fn an_agent_s_box_names_the_agent_and_offers_only_reply() {
        let mut thread = thread("why is this here");
        thread.root = comment(
            "a1",
            Author::Agent(Some("claude".into())),
            "why is this here",
        );
        thread.unsent = false;
        let lines = boxed(&thread, MATCHED, 50);
        assert!(
            lines[0].starts_with("╭ ● claude · 10m · src/a.rs R7 ─"),
            "{}",
            lines[0]
        );
        assert!(lines[2].ends_with("── r reply ╯"), "{}", lines[2]);
        thread.root.author = Author::Agent(None);
        let lines = boxed(&thread, MATCHED, 50);
        assert!(lines[0].starts_with("╭ ● agent · 10m · "), "{}", lines[0]);
    }

    #[test]
    fn the_footer_keys_follow_the_keymap() {
        let theme = Theme::default();
        let keymap = Keymap::from_toml(
            "[keys]\nreply = \"ctrl+r\"\nedit = \"\"\ndelete = [\"x\", \"shift+d\"]\n",
        );
        let look = Look {
            theme: &theme,
            keymap: &keymap,
            now: "",
        };
        let lines = text(&card(&thread("x"), MATCHED, 60, &look));
        // `edit` has no key, so it is left out. `delete` shows the first of its two.
        assert!(
            lines[2].ends_with("── ctrl+r reply  D delete ╯"),
            "{}",
            lines[2]
        );
        let none = Keymap::from_toml("[keys]\nreply = \"\"\nedit = \"\"\ndelete = \"\"\n");
        let look = Look {
            keymap: &none,
            ..look
        };
        let lines = text(&card(&thread("x"), MATCHED, 20, &look));
        assert_eq!(lines[2], format!("╰{}╯", "─".repeat(18)));
    }

    #[test]
    fn the_age_of_a_note_is_counted_from_now_in_the_largest_whole_unit() {
        let now = "2026-10-05T12:00:00Z";
        for (at, label) in [
            ("2026-10-05T12:00:00Z", "now"),
            ("2026-10-05T11:59:01Z", "now"),
            ("2026-10-05T11:59:00Z", "1m"),
            ("2026-10-05T11:58:00Z", "2m"),
            ("2026-10-05T11:00:01Z", "59m"),
            ("2026-10-05T09:00:00Z", "3h"),
            ("2026-10-03T12:00:00Z", "2d"),
            ("2026-08-26T12:00:00Z", "40d"),
            // Another offset is the same instant, and a clock that ran ahead is not negative.
            ("2026-10-05T14:00:00+02:00", "now"),
            ("2026-10-05T12:30:00Z", "now"),
            ("yesterday", ""),
            ("", ""),
        ] {
            assert_eq!(ago(at, now), label, "{at}");
        }
        assert_eq!(ago(now, "soon"), "");
        // A comment with no time, as an old log may give, has no age on its border.
        let mut thread = thread("x");
        thread.root.at = String::new();
        let lines = boxed(&thread, MATCHED, 50);
        assert!(
            lines[0].starts_with("╭ ● Your note · src/a.rs R7 [unsent] ─"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn a_long_path_is_cut_from_the_left_and_gives_way_before_the_badges() {
        let mut thread = thread("x");
        thread.anchor.path = RelPath::parse("src/routes/newsletter_subscription.js").unwrap();
        assert_eq!(
            boxed(&thread, MATCHED, 50)[0],
            "╭ ● Your note · 10m · …bscription.js R7 [unsent] ╮"
        );
        let whole = boxed(&thread, MATCHED, 80);
        assert!(
            whole[0].starts_with(
                "╭ ● Your note · 10m · src/routes/newsletter_subscription.js R7 [unsent] ─"
            ),
            "{}",
            whole[0]
        );
        // With no room for a path at all, it is left out and the badges stay.
        let lines = text(&card(&thread, MATCHED, 34, &Look::test()));
        assert_eq!(lines[0], "╭ ● Your note · 10m [unsent] ────╮");
    }

    #[test]
    fn the_badges_follow_the_place_in_a_fixed_order() {
        let mut thread = thread("x");
        thread.is_new = true;
        thread.root.edited_since_sent = true;
        let lines = boxed(&thread, Placement::Outdated { near: Some(3) }, 90);
        assert!(
            lines[0].starts_with(
                "╭ ● Your note · 10m · src/a.rs R7 [outdated] [new] (edited since sent) [unsent] ─"
            ),
            "{}",
            lines[0]
        );
        thread.is_new = false;
        thread.root.edited_since_sent = false;
        thread.unsent = false;
        let lines = boxed(&thread, MATCHED, 90);
        assert!(
            lines[0].starts_with("╭ ● Your note · 10m · src/a.rs R7 ─"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn the_outdated_tag_and_the_old_line_text_show_on_an_open_thread_only() {
        let mut open = thread("fix");
        let placement = Placement::Outdated { near: Some(3) };
        let lines = boxed(&open, placement, 60);
        assert!(
            lines[0].contains(" R7 [outdated] [unsent] ─"),
            "{}",
            lines[0]
        );
        assert!(lines[1].starts_with("│ was: let x = 1;  "), "{}", lines[1]);
        open.status = Status::Resolved {
            by: Author::Agent(Some("claude".into())),
        };
        open.replies
            .push(comment("a1", Author::Agent(None), "Added with_capacity"));
        let lines = text(&card(&open, placement, 80, &Look::test()));
        assert_eq!(
            lines,
            ["✓ u1 resolved by agent:claude: Added with_capacity"]
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
        let new = card(&thread, MATCHED, 80, &Look::test());
        assert_eq!(
            text(&new),
            ["✓ u1 [new] resolved by agent:claude: Added with_capacity"]
        );
        // The check is green, in place of the bullet of an open note.
        assert_eq!(new.lines[0].spans[0].content, "✓");
        assert_eq!(
            new.lines[0].spans[0].style.fg,
            Some(Theme::default().success)
        );
        thread.is_new = false;
        let seen = text(&card(&thread, MATCHED, 80, &Look::test()));
        assert_eq!(seen, ["✓ u1 resolved by agent:claude: Added with_capacity"]);
        // The tag survives a cut: it is before what gets truncated.
        thread.is_new = true;
        let cut = text(&card(&thread, MATCHED, 30, &Look::test()));
        assert!(cut[0].contains("[new]"));
        assert!(string_width(&cut[0]) <= 30);
    }

    #[test]
    fn a_thread_not_in_the_diff_says_where_it_pointed() {
        let lines = boxed(&thread("fix"), Placement::NotInDiff, 50);
        assert!(
            lines[0].starts_with("╭ ● Your note · 10m · src/a.rs R7 "),
            "{}",
            lines[0]
        );
        assert!(lines[1].starts_with("│ was: let x = 1;  "), "{}", lines[1]);
        assert!(lines[2].starts_with("│ fix  "), "{}", lines[2]);
        assert_eq!(lines.len(), 4);
        let mut range = thread("fix");
        range.anchor.target = AnchorTarget::Range {
            side: Side::Old,
            start: 3,
            end: 5,
            text: "x".into(),
        };
        let lines = boxed(&range, Placement::NotInDiff, 50);
        assert!(lines[0].contains(" · src/a.rs L3-5 "), "{}", lines[0]);
        let mut file = thread("fix");
        file.anchor.target = AnchorTarget::File;
        let lines = boxed(&file, Placement::NotInDiff, 50);
        assert!(lines[0].contains(" · src/a.rs [unsent] "), "{}", lines[0]);
        assert_eq!(lines.len(), 3, "a file comment has no line text to show");
        file.status = Status::Resolved { by: Author::User };
        assert_eq!(
            text(&card(&file, Placement::NotInDiff, 60, &Look::test())),
            ["✓ u1 src/a.rs resolved by user: fix"]
        );
    }

    #[test]
    fn a_box_title_names_the_path_the_side_letter_and_the_line_or_range() {
        let mut anchor = thread("x").anchor;
        assert_eq!(place(&anchor), "src/a.rs R7");
        anchor.target = AnchorTarget::Line {
            side: Side::Old,
            line: 3,
            text: String::new(),
        };
        assert_eq!(place(&anchor), "src/a.rs L3");
        anchor.target = AnchorTarget::Range {
            side: Side::New,
            start: 101,
            end: 110,
            text: String::new(),
        };
        assert_eq!(place(&anchor), "src/a.rs R101-110");
        anchor.target = AnchorTarget::File;
        assert_eq!(place(&anchor), "src/a.rs");
    }

    #[test]
    fn an_edited_comment_says_so_and_every_string_loses_its_control_characters() {
        let mut thread = thread("a\u{1b}[2Jb");
        thread.root.edited_since_sent = true;
        let mut reply = comment("a1", Author::Agent(Some("e\u{1b}vil".into())), "x\u{9b}y");
        reply.edited_since_sent = true;
        thread.replies.push(reply);
        let lines = boxed(&thread, MATCHED, 80);
        assert!(
            lines[0].contains(" R7 (edited since sent) [unsent] ─"),
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
        // The name on the border of an agent's own box is cleaned too.
        thread.root.author = Author::Agent(Some("e\u{1b}vil".into()));
        let lines = boxed(&thread, MATCHED, 80);
        assert!(lines[0].starts_with("╭ ● evil · "), "{}", lines[0]);
    }

    #[test]
    fn a_narrow_stream_still_gets_a_card() {
        let card = card(
            &thread("a long comment body here"),
            Placement::NotInDiff,
            4,
            &Look::test(),
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
        // The top border, the old text, two body lines, a line per reply, the bottom border. The
        // borders belong to the root, which is what the keys on the bottom border act on.
        let look = Look::test();
        assert_eq!(
            card(&thread, placement, 40, &look).owners,
            [0, 0, 0, 0, 1, 2, 0]
        );
        // A resolved thread is one line, which shows its last comment.
        thread.status = Status::Resolved { by: Author::User };
        assert_eq!(card(&thread, placement, 40, &look).owners, [2]);
    }
}
