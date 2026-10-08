use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyModifiers;

use super::*;
use crate::diff::{MAX_PATCH, parse};
use crate::icons::icon;
use crate::store::{Anchor, Author, Comment, CommentId, Status, Thread};

const PATCH: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 a1
-a2
+A2
 a3
@@ -10,2 +10,3 @@
 a10
+a11
 a12
diff --git a/b.rs b/b.rs
new file mode 100644
--- /dev/null
+++ b/b.rs
@@ -0,0 +1,2 @@
+b1
+b2
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
";
// Rows: a.rs 0..=9 (header, hunk 1, a1, a2, A2, a3, hunk 6, a10, a11, a12), b.rs 10..=13,
// img.png 14..=15.

fn diff_of(patch: &str) -> Diff {
    Diff {
        rev: "HEAD".into(),
        files: parse(patch.as_bytes(), MAX_PATCH),
        spec: Spec::WorkTree,
        notices: Vec::new(),
    }
}

fn thread(id: &str, path: &str, target: AnchorTarget) -> Thread {
    Thread {
        root: Comment {
            id: CommentId::parse(id).unwrap(),
            parent: None,
            author: Author::User,
            at: String::new(),
            body: "fix".into(),
            sent_batch: None,
            edited_since_sent: false,
        },
        anchor: Anchor {
            path: RelPath::parse(path).unwrap(),
            old_path: None,
            target,
            spec: Spec::WorkTree,
        },
        replies: Vec::new(),
        status: Status::Open,
        is_new: false,
        reopened: false,
        unsent: true,
    }
}

fn line(line: u32, text: &str) -> AnchorTarget {
    AnchorTarget::Line {
        side: Side::New,
        line,
        text: text.into(),
    }
}

fn review() -> Review {
    Review {
        threads: vec![
            thread("u1", "a.rs", line(4, "not there anymore")),
            thread("u2", "b.rs", AnchorTarget::File),
            thread("u3", "a.rs", line(3, "A2")),
            thread("u4", "gone.rs", line(1, "x")),
        ],
        ..Review::default()
    }
}

fn view(diff: &Diff, review: &Review, width: u16, height: u16) -> View {
    let mut view = View::default();
    view.resize(Rect::new(0, 0, width, height));
    view.rebuild(diff, review, None, &Look::test());
    view
}

fn plain() -> (Diff, View) {
    let diff = diff_of(PATCH);
    let view = view(&diff, &Review::default(), 80, 12);
    (diff, view)
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(
    terminal: &mut Terminal<TestBackend>,
    view: &View,
    diff: &Diff,
    keymap: &Keymap,
) -> String {
    terminal
        .draw(|frame| {
            draw(
                frame,
                view,
                diff,
                keymap,
                &Theme::default(),
                None,
                &Cache::default(),
            );
        })
        .unwrap();
    screen(terminal)
}

fn fresh(view: &View, diff: &Diff) -> String {
    let mut terminal = Terminal::new(TestBackend::new(view.area.width, view.area.height)).unwrap();
    render(&mut terminal, view, diff, &Keymap::default())
}

#[test]
fn the_stream_numbers_every_row_of_every_file() {
    let (diff, view) = plain();
    assert_eq!(view.stream.len(), 16);
    assert_eq!(view.stream.hunk_rows, [1, 6, 11]);
    assert_eq!(view.stream.file_at(9), 0);
    assert_eq!(view.stream.file_at(10), 1);
    assert_eq!(view.stream.file_at(15), 2);
    let kind = |row| match view.stream.locate(&diff, row).unwrap() {
        RowRef::File(_) => "file",
        RowRef::Note(_) => "note",
        RowRef::Hunk(_) => "hunk",
        RowRef::Line(_) | RowRef::Pair { .. } => "line",
        RowRef::Gap(_) => "gap",
        RowRef::BlockHeader(_) => "block",
        RowRef::Card { .. } => "card",
        RowRef::Empty(_) => "empty",
    };
    let kinds = (0..16).map(kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "file", "hunk", "line", "line", "line", "line", "hunk", "line", "line", "line", "file",
            "hunk", "line", "line", "file", "note"
        ]
    );
    assert!(view.stream.locate(&diff, 16).is_none());
}

#[test]
fn up_and_down_move_one_row_and_stop_at_the_ends() {
    let (_, mut view) = plain();
    view.apply(Action::Up);
    assert_eq!(view.cursor, 0);
    view.apply(Action::Down);
    view.apply(Action::Down);
    assert_eq!(view.cursor, 2);
    view.apply(Action::Up);
    assert_eq!(view.cursor, 1);
    for _ in 0..40 {
        view.apply(Action::Down);
    }
    assert_eq!(view.cursor, 15);
    assert_eq!(view.scroll, 6);
}

#[test]
fn page_down_and_page_up_scroll_a_page() {
    let (_, mut view) = plain();
    assert_eq!(view.height(), 10);
    view.apply(Action::PageDown);
    assert_eq!((view.scroll, view.cursor), (6, 10));
    view.apply(Action::PageDown);
    assert_eq!((view.scroll, view.cursor), (6, 15));
    view.apply(Action::PageUp);
    assert_eq!((view.scroll, view.cursor), (0, 5));
    view.apply(Action::PageUp);
    assert_eq!((view.scroll, view.cursor), (0, 0));
}

#[test]
fn next_and_previous_hunk_jump_between_hunk_headers_across_files() {
    let (_, mut view) = plain();
    let mut seen = Vec::new();
    for _ in 0..4 {
        view.apply(Action::NextHunk);
        seen.push(view.cursor);
    }
    assert_eq!(seen, [1, 6, 11, 11]);
    view.apply(Action::PrevHunk);
    assert_eq!(view.cursor, 6);
    // From inside a hunk, previous goes to that hunk's own header.
    view.move_to(8);
    view.apply(Action::PrevHunk);
    assert_eq!(view.cursor, 6);
    view.move_to(0);
    view.apply(Action::PrevHunk);
    assert_eq!(view.cursor, 0);
}

// With `review()` at 80 columns the stream is 60 wide. A card is a box: two borders around
// an empty row and its body, and the old line text when it is outdated or not in the diff.
// The block of threads not in the diff is rows 0..=5: the heading, and u4's five rows.
// a.rs starts at 6: header 6, hunk 7, a1 8, a2 9, A2 10, then u3 (rows 11-14), a3 15, then
// u1 (16-20), hunk 21, a10 22, a11 23, a12 24. b.rs starts at 25, and u2 is rows 26-29.
// Its hunk is 30, b1 31, b2 32, img.png 33, and its note 34.

#[test]
fn hunk_headers_move_down_by_the_cards_above_them_and_no_more() {
    let patch = "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1,2 +1,2 @@\n-c1\n+C1\n c2\n@@ -10,2 +10,2 @@\n-c10\n+C10\n c11\n@@ -20,2 +20,2 @@\n-c20\n+C20\n c21\n";
    // Rows: header 0, hunk 1, c1 2, C1 3, c2 4, hunk 5, c10 6, C10 7, c11 8, hunk 9, c20 10,
    // C20 11, c21 12. Each card is four rows: a file card under the header, one under c2,
    // one under C10, and one under c21, below the last hunk.
    let diff = diff_of(patch);
    let review = Review {
        threads: vec![
            thread("u1", "c.rs", AnchorTarget::File),
            thread("u2", "c.rs", line(2, "c2")),
            thread("u3", "c.rs", line(10, "C10")),
            thread("u4", "c.rs", line(21, "c21")),
        ],
        ..Review::default()
    };
    let view = view(&diff, &review, 80, 12);
    assert_eq!(view.stream.thread_rows, [1, 9, 16, 25]);
    assert_eq!(view.stream.hunk_rows, [5, 13, 21]);
}

#[test]
fn cards_take_rows_under_the_lines_they_are_placed_at() {
    let diff = diff_of(PATCH);
    let review = review();
    let view = view(&diff, &review, 80, 12);
    assert_eq!(view.stream.len(), 35);
    assert_eq!(view.stream.hunk_rows, [7, 21, 30]);
    assert_eq!(view.stream.thread_rows, [1, 11, 16, 26]);
    assert_eq!(view.stream.file_start(1), Some(25));
    assert_eq!(
        view.stream.placements,
        [
            Placement::Outdated { near: Some(3) },
            Placement::Matched { line: None },
            Placement::Matched { line: Some(2) },
            Placement::NotInDiff,
        ]
    );
    let kinds = (0..35)
        .map(|row| match view.stream.locate(&diff, row).unwrap() {
            RowRef::BlockHeader(_) => "block".to_owned(),
            RowRef::Card { thread, line } => format!("t{thread}.{line}"),
            RowRef::File(_) => "file".to_owned(),
            RowRef::Hunk(_) => "hunk".to_owned(),
            RowRef::Line(row) => row.text.clone(),
            RowRef::Pair { .. } | RowRef::Gap(_) => "split".to_owned(),
            RowRef::Note(_) => "note".to_owned(),
            RowRef::Empty(_) => "empty".to_owned(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        kinds.join(" "),
        "block t3.0 t3.1 t3.2 t3.3 t3.4 file hunk a1 a2 A2 t2.0 t2.1 t2.2 t2.3 a3 t0.0 t0.1 \
             t0.2 t0.3 t0.4 hunk a10 a11 a12 file t1.0 t1.1 t1.2 t1.3 hunk b1 b2 file note"
    );
    assert!(view.stream.locate(&diff, 35).is_none());
}

#[test]
fn next_and_previous_thread_jump_between_cards() {
    let diff = diff_of(PATCH);
    let review = review();
    let mut view = view(&diff, &review, 80, 12);
    let mut seen = Vec::new();
    for _ in 0..5 {
        view.apply(Action::NextThread);
        seen.push(view.cursor);
    }
    assert_eq!(seen, [1, 11, 16, 26, 26]);
    let mut back = Vec::new();
    for _ in 0..5 {
        view.apply(Action::PrevThread);
        back.push(view.cursor);
    }
    assert_eq!(back, [16, 11, 1, 1, 1]);
}

#[test]
fn the_focused_thread_is_the_card_the_cursor_is_in_or_the_first_under_its_line() {
    let diff = diff_of(PATCH);
    let review = review();
    let mut view = view(&diff, &review, 80, 12);
    let focused = |view: &mut View, row| {
        view.move_to(row);
        view.focused()
    };
    // u3's card, and the A2 line it hangs under.
    assert_eq!(focused(&mut view, 12), Some(2));
    assert_eq!(focused(&mut view, 10), Some(2));
    // The file header with a file comment, and the line u1 hangs under.
    assert_eq!(focused(&mut view, 25), Some(1));
    assert_eq!(focused(&mut view, 15), Some(0));
    // The block, its heading, a line with no card, and a hunk header.
    assert_eq!(focused(&mut view, 3), Some(3));
    assert_eq!(focused(&mut view, 0), None);
    assert_eq!(focused(&mut view, 8), None);
    assert_eq!(focused(&mut view, 21), None);
}

#[test]
fn a_card_is_drawn_for_each_placement_case() {
    let diff = diff_of(PATCH);
    let review = review();
    let mut view = view(&diff, &review, 80, 30);
    let screen = fresh(&view, &diff);
    let rows = screen
        .lines()
        .map(|row| row.split_once('│').map_or(row, |(_, rest)| rest).to_owned())
        .collect::<Vec<_>>();
    let at = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap();
    // Not in the diff: the block comes first, with where it pointed.
    assert_eq!(at("Comments not in this diff (1)"), 0);
    assert!(at("● Your note · gone.rs R1 ") < at("M a.rs"));
    assert!(screen.contains("was: x"), "{screen}");
    // Matched: under the line, with no tag.
    assert_eq!(at("· a.rs R3 "), at("+A2") + 1);
    assert!(!rows[at("· a.rs R3 ")].contains("outdated"));
    // Outdated: under the nearest line, tagged, with the old text.
    assert_eq!(at("· a.rs R4 [outdated]"), at(" a3") + 1);
    assert!(screen.contains("was: not there anymore"), "{screen}");
    // A file comment: under the file header, with the bare path.
    assert_eq!(at("· b.rs [unsent]"), at("A b.rs") + 1);
    // Every box closes with the keys that act on it, against the stream's right edge.
    assert!(rows[at("· a.rs R3 ") + 3].ends_with(" r reply  e edit  d delete ╯"));
    // In the unified layout a box is indented four cells and runs to the right edge.
    assert!(rows[at("· a.rs R3 ")].starts_with("    ╭ ● Your note"));
    assert!(rows[at("· a.rs R3 ") + 2].starts_with("    │ "));
    // The cursor row is highlighted whether it is a card or a diff row.
    view.move_to(11);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal
        .draw(|frame| {
            draw(
                frame,
                &view,
                &diff,
                &Keymap::default(),
                &Theme::default(),
                None,
                &Cache::default(),
            );
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(30, 11)].bg, Theme::default().cursor);
    assert_ne!(buffer[(30, 10)].bg, Theme::default().cursor);
}

#[test]
fn a_note_docks_to_its_half_side_by_side_and_is_indented_otherwise() {
    let (split, unified) = (DiffLayout::Split, DiffLayout::Unified);
    // The halves of a 98 cell stream are 48 and 49 with a cell between them.
    assert_eq!(note_box(98, split, Some(Side::New)), (49, 49));
    assert_eq!(note_box(98, split, Some(Side::Old)), (0, 48));
    // A file comment has no half, and the unified layout has none either.
    assert_eq!(note_box(98, split, None), (4, 94));
    assert_eq!(note_box(60, unified, Some(Side::New)), (4, 56));
    // A split stream too narrow to dock in indents, and a narrow stream gives the box all of it.
    assert_eq!(note_box(80, split, Some(Side::New)), (4, 76));
    assert_eq!(note_box(30, unified, None), (2, 28));
    assert_eq!(note_box(20, unified, None), (0, 20));
}

#[test]
fn a_card_is_docked_to_the_half_its_line_is_on_in_the_side_by_side_layout() {
    let diff = diff_of(PATCH);
    let review = review();
    let view = view(&diff, &review, 130, 40);
    assert_eq!(view.stream.layout, DiffLayout::Split);
    let screen = fresh(&view, &diff);
    // The sidebar is 32 columns and the stream is the other 98.
    let tops = screen
        .lines()
        .map(|row| row.chars().skip(32).collect::<String>())
        .filter(|row| row.trim_start().starts_with("╭ ● Your note · "))
        .collect::<Vec<_>>();
    assert_eq!(tops.len(), 4, "{screen}");
    // The cells to the left of a box, and its width.
    let shape = |needle: &str| {
        let top = tops.iter().find(|top| top.contains(needle)).unwrap();
        assert!(top.ends_with("─╮"), "{top}");
        let left = top.chars().take_while(|c| *c == ' ').count();
        (left, string_width(top.trim_start()))
    };
    // u3 and u1 are on lines of the new side, so their boxes are the right half.
    assert_eq!(shape("· a.rs R3 "), (49, 49));
    assert_eq!(shape("· a.rs R4 "), (49, 49));
    // A file comment and a thread that is not in the diff have no half.
    assert_eq!(shape("· b.rs "), (4, 94));
    assert_eq!(shape("· gone.rs R1 "), (4, 94));
    assert_eq!(
        screen
            .lines()
            .filter(|row| row.ends_with(" r reply  e edit  d delete ╯"))
            .count(),
        4,
        "{screen}"
    );
}

#[test]
fn a_resolved_outdated_thread_is_a_box_with_no_tag() {
    let diff = diff_of(PATCH);
    let mut review = review();
    review.threads[0].status = Status::Resolved {
        by: Author::Agent(Some("claude".into())),
    };
    review.threads[0].replies.push(Comment {
        id: CommentId::parse("a1").unwrap(),
        parent: None,
        author: Author::Agent(Some("claude".into())),
        at: String::new(),
        body: "Added with_capacity".into(),
        sent_batch: None,
        edited_since_sent: false,
    });
    let view = view(&diff, &review, 120, 30);
    // The thread is still placed as outdated, and its card is a box of five rows.
    assert_eq!(
        view.stream.placements[0],
        Placement::Outdated { near: Some(3) }
    );
    assert_eq!(view.stream.len(), 35);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("[resolved by agent:claude]"), "{screen}");
    assert!(
        screen.contains("↳ agent:claude: Added with_capacity"),
        "{screen}"
    );
    assert!(!screen.contains("outdated"), "{screen}");
    assert!(!screen.contains("was: not there anymore"), "{screen}");
}

#[test]
fn with_an_empty_diff_the_block_still_lists_the_threads() {
    let diff = diff_of("");
    let review = Review {
        threads: vec![
            thread("u1", "a.rs", line(1, "fn main() {}")),
            thread("u2", "b.rs", AnchorTarget::File),
        ],
        ..Review::default()
    };
    let view = view(&diff, &review, 80, 20);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("Comments not in this diff (2)"), "{screen}");
    assert!(screen.contains("· a.rs R1 "), "{screen}");
    assert!(screen.contains("· b.rs [unsent]"), "{screen}");
    assert!(
        screen.contains("No changes in the working tree."),
        "{screen}"
    );
    assert_eq!(view.stream.thread_rows, [1, 6]);
    // With no threads either, the message is the only row.
    let bare = fresh(&view_of(&diff), &diff);
    assert!(!bare.contains("not in this diff"));
}

#[test]
fn a_comment_on_a_renamed_files_old_name_hangs_under_the_new_file() {
    let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\n";
    let diff = diff_of(patch);
    let mut review = Review {
        threads: vec![thread("a1", "old.rs", AnchorTarget::File)],
        ..Review::default()
    };
    review.threads[0].root.author = Author::Agent(None);
    let view = view(&diff, &review, 80, 12);
    assert_eq!(view.stream.len(), 8);
    assert_eq!(view.stream.thread_rows, [1]);
}

#[test]
fn a_narrow_stream_wraps_a_long_body_into_more_rows() {
    let diff = diff_of(PATCH);
    let mut review = review();
    review.threads[2].root.body = "word ".repeat(30);
    let wide = view(&diff, &review, 120, 12);
    let narrow = view(&diff, &review, 60, 12);
    assert!(narrow.stream.len() > wide.stream.len());
    assert!(!wide.needs_rebuild() && !narrow.needs_rebuild());
    let mut changed = narrow.clone();
    changed.resize(Rect::new(0, 0, 120, 12));
    assert!(changed.needs_rebuild());
}

#[test]
fn a_file_chosen_in_the_sidebar_has_its_header_on_the_top_row_from_either_direction() {
    let diff = diff_of(PATCH);
    let mut view = view(&diff, &Review::default(), 80, 6);
    assert_eq!(view.height(), 4);
    view.apply(Action::SwitchPanel);
    view.apply(Action::Down);
    assert_eq!((view.cursor, view.scroll), (10, 10));
    // The last file is shorter than the window, so the stream stops at its end.
    view.apply(Action::Down);
    assert_eq!((view.cursor, view.scroll), (14, 12));
    view.apply(Action::Up);
    assert_eq!((view.cursor, view.scroll), (10, 10));
    view.apply(Action::Up);
    assert_eq!((view.cursor, view.scroll), (0, 0));
}

#[test]
fn the_cursor_row_of_the_stream_keeps_its_bar_and_its_text_colour_while_the_sidebar_is_focused() {
    let (diff, mut view) = plain();
    let theme = Theme::default();
    let cell = |view: &View| {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        render(&mut terminal, view, &diff, &Keymap::default());
        let stream = view.areas().stream;
        let cell = &terminal.backend().buffer()[(stream.x + 2, stream.y)];
        (cell.bg, cell.fg)
    };
    let focused = cell(&view);
    assert_eq!(focused.0, theme.cursor);
    view.apply(Action::SwitchPanel);
    assert_eq!(cell(&view), focused);
}

#[test]
fn the_switch_panel_key_moves_up_and_down_between_files_in_the_sidebar() {
    let (_, mut view) = plain();
    assert_eq!(view.panel, Panel::Stream);
    view.apply(Action::SwitchPanel);
    assert_eq!(view.panel, Panel::Sidebar);
    view.apply(Action::Down);
    assert_eq!(view.cursor, 10);
    view.apply(Action::Down);
    assert_eq!(view.cursor, 14);
    view.apply(Action::Down);
    assert_eq!(view.cursor, 14);
    view.apply(Action::Up);
    assert_eq!(view.cursor, 10);
    view.apply(Action::PageUp);
    assert_eq!(view.cursor, 0);
    view.apply(Action::PageDown);
    assert_eq!(view.cursor, 14);
    view.apply(Action::SwitchPanel);
    assert_eq!(view.panel, Panel::Stream);
}

#[test]
fn help_toggles_and_the_other_actions_are_not_the_bodys() {
    let (_, mut view) = plain();
    assert!(view.apply(Action::Help));
    assert!(view.help);
    assert!(view.apply(Action::Help));
    assert!(!view.help);
    for action in [
        Action::Reload,
        Action::SwitchSpec,
        Action::Quit,
        Action::Comment,
        Action::Send,
    ] {
        assert!(!view.apply(action), "{action:?}");
    }
}

#[test]
fn the_wheel_scrolls_and_a_click_moves_the_cursor() {
    let (_, mut view) = plain();
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
    assert_eq!((view.scroll, view.cursor), (3, 3));
    view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
    view.mouse(mouse(MouseEventKind::ScrollDown, 40, 3));
    assert_eq!(view.scroll, 6);
    view.mouse(mouse(MouseEventKind::ScrollUp, 40, 3));
    assert_eq!(view.scroll, 3);
    // A click in the stream.
    let stream = view.areas().stream;
    view.mouse(mouse(
        MouseEventKind::Down(MouseButton::Left),
        stream.x + 4,
        stream.y + 5,
    ));
    assert_eq!((view.cursor, view.panel), (8, Panel::Stream));
    // A click in the sidebar on the second file. Rows 0 to 2 are the filter's box and row 3 is the
    // directory heading.
    view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 5));
    assert_eq!((view.cursor, view.panel), (10, Panel::Sidebar));
    // A click on the heading, or below the last file, does nothing.
    view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 3));
    view.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 9));
    assert_eq!(view.cursor, 10);
}

#[test]
fn the_stream_and_the_sidebar_are_drawn() {
    let (diff, view) = plain();
    let screen = fresh(&view, &diff);
    assert!(screen.contains("M a.rs"));
    assert!(screen.contains("A b.rs"));
    assert!(screen.contains("B img.png"));
    assert!(screen.contains("@@ -1,3 +1,3 @@"));
    assert!(screen.contains("    1    1  a1"), "{screen}");
    assert!(screen.contains("    2      -a2"), "{screen}");
    assert!(screen.contains("         2 +A2"), "{screen}");
}

#[test]
fn a_file_with_no_rows_says_why() {
    let (diff, mut view) = plain();
    view.move_to(15);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("binary file, not shown"));
}

#[test]
fn scrolling_to_the_end_and_back_leaves_no_stale_rows() {
    let (diff, mut view) = plain();
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    let keymap = Keymap::default();
    let top = render(&mut terminal, &view, &diff, &keymap);
    for _ in 0..40 {
        view.apply(Action::Down);
    }
    let bottom = render(&mut terminal, &view, &diff, &keymap);
    assert_ne!(top, bottom);
    assert_eq!(bottom, fresh(&view, &diff));
    for _ in 0..40 {
        view.apply(Action::Up);
    }
    assert_eq!(render(&mut terminal, &view, &diff, &keymap), top);
    // A shorter diff over the same terminal leaves nothing of the longer one behind.
    let short = diff_of(&PATCH[..PATCH.find("diff --git a/b.rs").unwrap()]);
    let short_view = view_of(&short);
    assert_eq!(
        render(&mut terminal, &short_view, &short, &keymap),
        fresh(&short_view, &short)
    );
}

fn view_of(diff: &Diff) -> View {
    view(diff, &Review::default(), 80, 12)
}

#[test]
fn a_file_name_with_an_escape_byte_draws_without_control_characters() {
    let patch = "diff --git \"a/evil\\033[2Jname.rs\" \"b/evil\\033[2Jname.rs\"\n--- \"a/evil\\033[2Jname.rs\"\n+++ \"b/evil\\033[2Jname.rs\"\n@@ -1 +1 @@\n-old\\u{1b}x\n+new\n";
    let diff = diff_of(patch);
    assert_eq!(diff.files.len(), 1);
    assert!(diff.files[0].path.as_str().contains('\u{1b}'));
    let view = view_of(&diff);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("evil[2Jname.rs"));
    assert!(!screen.chars().any(|c| c != '\n' && c.is_control()));
}

#[test]
fn a_long_name_is_cut_at_its_end_in_the_sidebar_and_long_lines_are_cut() {
    let long = "x".repeat(60);
    let patch = format!(
        "diff --git a/dir/{long}.rs b/dir/{long}.rs\n--- a/dir/{long}.rs\n+++ b/dir/{long}.rs\n@@ -1 +1 @@\n-{long}{long}\n+new\n"
    );
    let diff = diff_of(&patch);
    let view = view_of(&diff);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("dir/"), "{screen}");
    assert!(
        screen.contains(" M xx") && screen.contains("… +1 -1│"),
        "{screen}"
    );
    assert!(screen.lines().all(|row| string_width(row) <= 80));
}

#[test]
fn an_empty_diff_says_so_for_each_spec() {
    let mut diff = diff_of("");
    let view = view_of(&diff);
    assert!(fresh(&view, &diff).contains("No changes in the working tree."));
    diff.spec = Spec::Branch {
        base: "main".into(),
    };
    assert!(fresh(&view, &diff).contains("No changes against main."));
}

#[test]
fn the_help_overlay_lists_every_action_with_its_current_keys() {
    let (diff, mut view) = plain();
    view.help = true;
    let mut terminal = Terminal::new(TestBackend::new(80, 34)).unwrap();
    view.resize(Rect::new(0, 0, 80, 34));
    let defaults = render(&mut terminal, &view, &diff, &Keymap::default());
    assert!(
        defaults.contains("R                reload the diff"),
        "{defaults}"
    );
    assert!(defaults.contains("q                quit"));
    let rebound = Keymap::from_toml("[keys]\nreload = \"r\"\nquit = [\"q\", \"ctrl+c\"]\n");
    let after = render(&mut terminal, &view, &diff, &rebound);
    assert!(after.contains("r                reload the diff"));
    assert!(after.contains("ctrl+c, q"));
    assert!(after.contains("-                reply to the thread"));
}

#[test]
fn a_reload_puts_the_cursor_back_on_its_row_in_the_same_file() {
    let (diff, mut view) = plain();
    view.move_to(12);
    let anchor = view.spot(&diff);
    // The first file lost its second hunk, so b.rs starts earlier.
    let smaller = diff_of(&PATCH.replace("@@ -10,2 +10,3 @@\n a10\n+a11\n a12\n", ""));
    view.rebuild(&smaller, &Review::default(), anchor, &Look::test());
    assert_eq!(view.cursor, 8);
    assert!(matches!(view.stream.locate(&smaller, 8), Some(RowRef::Line(row)) if row.text == "b1"));
    // The file is gone: the cursor stays on the nearest row that exists.
    let anchor = view.spot(&smaller);
    view.rebuild(&diff_of(""), &Review::default(), anchor, &Look::test());
    assert_eq!(view.cursor, 0);
}

#[test]
fn a_rebuild_keeps_the_cursor_on_its_line_of_the_same_card() {
    let diff = diff_of(PATCH);
    let mut review = review();
    let mut view = view(&diff, &review, 80, 12);
    // Line 1 of u3's card, which is row 12.
    view.move_to(12);
    // A new thread above it pushes everything down by its rows.
    review.threads.push(thread("u5", "a.rs", line(1, "a1")));
    let spot = view.spot(&diff);
    view.rebuild(&diff, &review, spot, &Look::test());
    assert!(matches!(
        view.stream.locate(&diff, view.cursor),
        Some(RowRef::Card { thread: 2, line: 1 })
    ));
    assert_eq!(view.cursor, 12 + 4);
    // The thread is deleted: the cursor goes to the line it hung under.
    review.threads.remove(2);
    let spot = view.spot(&diff);
    view.rebuild(&diff, &review, spot, &Look::test());
    assert!(
        matches!(view.stream.locate(&diff, view.cursor), Some(RowRef::Line(row)) if row.text == "A2")
    );
}

fn anchor_at(view: &mut View, diff: &Diff, row: usize) -> Result<Anchor, &'static str> {
    view.move_to(row);
    view.capture(diff)
}

fn target_at(view: &mut View, diff: &Diff, row: usize) -> Result<AnchorTarget, &'static str> {
    anchor_at(view, diff, row).map(|anchor| anchor.target)
}

#[test]
fn a_comment_on_a_line_points_at_its_side_number_and_text() {
    let (diff, mut view) = plain();
    assert_eq!(target_at(&mut view, &diff, 4), Ok(line(2, "A2")));
    let removed = AnchorTarget::Line {
        side: Side::Old,
        line: 2,
        text: "a2".into(),
    };
    assert_eq!(target_at(&mut view, &diff, 3), Ok(removed));
    // A context row is on the new side.
    assert_eq!(target_at(&mut view, &diff, 2), Ok(line(1, "a1")));
    let anchor = anchor_at(&mut view, &diff, 4).unwrap();
    assert_eq!(anchor.path.as_str(), "a.rs");
    assert_eq!((anchor.old_path, anchor.spec), (None, Spec::WorkTree));
}

#[test]
fn a_comment_on_a_file_header_or_its_note_points_at_the_file() {
    let (diff, mut view) = plain();
    assert_eq!(target_at(&mut view, &diff, 0), Ok(AnchorTarget::File));
    let binary = anchor_at(&mut view, &diff, 15).unwrap();
    assert_eq!(binary.target, AnchorTarget::File);
    assert_eq!(binary.path.as_str(), "img.png");
    assert_eq!(
        target_at(&mut view, &diff, 1),
        Err("comment on a line or a file header")
    );
}

#[test]
fn a_range_runs_from_where_it_started_to_the_cursor_on_the_side_of_its_first_line() {
    let (diff, mut view) = plain();
    view.move_to(2);
    view.toggle_select();
    let range = |side, start, end, text: &str| AnchorTarget::Range {
        side,
        start,
        end,
        text: text.into(),
    };
    // From a1 to a3 on the new side, over the removed row, which has no new line.
    assert_eq!(
        target_at(&mut view, &diff, 5),
        Ok(range(Side::New, 1, 3, "a1"))
    );
    // Upwards is the same range.
    let mut upwards = View::default();
    upwards.resize(Rect::new(0, 0, 80, 12));
    upwards.rebuild(&diff, &Review::default(), None, &Look::test());
    upwards.move_to(5);
    upwards.toggle_select();
    assert_eq!(
        target_at(&mut upwards, &diff, 2),
        Ok(range(Side::New, 1, 3, "a1"))
    );
    // Starting on a removed row, the range is on the old side.
    let mut old = View::default();
    old.resize(Rect::new(0, 0, 80, 12));
    old.rebuild(&diff, &Review::default(), None, &Look::test());
    old.move_to(3);
    old.toggle_select();
    assert_eq!(
        target_at(&mut old, &diff, 5),
        Ok(range(Side::Old, 2, 3, "a2"))
    );
    // Across hunks, the end is the last line on that side.
    assert_eq!(
        target_at(&mut old, &diff, 9),
        Ok(range(Side::Old, 2, 11, "a2"))
    );
    // Where it started is where it ends: one line.
    assert_eq!(
        target_at(&mut old, &diff, 3).map(|t| matches!(t, AnchorTarget::Line { .. })),
        Ok(true)
    );
}

#[test]
fn a_range_cannot_cross_files_or_hold_no_lines() {
    let (diff, mut view) = plain();
    view.move_to(2);
    view.toggle_select();
    assert_eq!(
        target_at(&mut view, &diff, 12),
        Err("a range stays inside one file")
    );
    // The file header and the hunk header under it hold no line.
    view.select = Some(Select { row: 0, half: None });
    assert_eq!(
        target_at(&mut view, &diff, 1),
        Err("select lines to comment on")
    );
    // Selecting again drops the range, and a rebuild does too.
    view.toggle_select();
    assert_eq!(view.select, None);
    view.toggle_select();
    assert_eq!(view.select.map(|select| select.row), Some(1));
    view.rebuild(&diff, &Review::default(), None, &Look::test());
    assert_eq!(view.select, None);
}

#[test]
fn a_comment_written_from_a_card_points_where_the_card_hangs() {
    let diff = diff_of(PATCH);
    let review = review();
    let mut view = view(&diff, &review, 80, 12);
    // u3's card hangs under A2, and u2's under the b.rs header.
    assert_eq!(target_at(&mut view, &diff, 12), Ok(line(2, "A2")));
    assert_eq!(target_at(&mut view, &diff, 26), Ok(AnchorTarget::File));
    // A thread in the block has no file in the diff to point into.
    let block = anchor_at(&mut view, &diff, 3).unwrap_err();
    assert!(block.contains("reply"), "{block}");
    assert!(anchor_at(&mut view, &diff, 0).is_err());
}

#[test]
fn a_comment_on_a_renamed_file_keeps_the_old_name_and_the_spec_on_screen() {
    let patch = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-a\n+b\n";
    let mut diff = diff_of(patch);
    diff.spec = Spec::Branch {
        base: "main".into(),
    };
    let mut view = view_of(&diff);
    let anchor = anchor_at(&mut view, &diff, 2).unwrap();
    assert_eq!(anchor.path.as_str(), "new.rs");
    assert_eq!(anchor.old_path.unwrap().as_str(), "old.rs");
    assert_eq!(anchor.spec, diff.spec);
}

#[test]
fn the_focused_comment_is_the_one_whose_line_of_the_card_the_cursor_is_on() {
    let diff = diff_of(PATCH);
    let mut review = review();
    review.threads[2].replies.push(Comment {
        id: CommentId::parse("a1").unwrap(),
        parent: None,
        author: Author::Agent(None),
        at: String::new(),
        body: "done".into(),
        sent_batch: None,
        edited_since_sent: false,
    });
    let mut view = view(&diff, &review, 80, 14);
    let at = |view: &mut View, row| {
        view.move_to(row);
        view.focused_comment()
    };
    // u3's card is rows 11 to 15 now: the top border, the empty row, the body, the reply, the
    // bottom border.
    assert_eq!(at(&mut view, 11), Some((2, 0)));
    assert_eq!(at(&mut view, 12), Some((2, 0)));
    assert_eq!(at(&mut view, 13), Some((2, 0)));
    assert_eq!(at(&mut view, 14), Some((2, 1)));
    // The bottom border names the keys that act on the root, so it is the root.
    assert_eq!(at(&mut view, 15), Some((2, 0)));
    // Past the box is a3, the line u1 hangs under.
    assert_eq!(at(&mut view, 16), Some((0, 0)));
    // The line the card hangs under counts as the root.
    assert_eq!(at(&mut view, 10), Some((2, 0)));
    assert_eq!(at(&mut view, 8), None);
    assert_eq!(at(&mut view, 0), None);
    assert_eq!(view.thread_id(2).unwrap().as_str(), "u3");
}

#[test]
fn focusing_a_thread_puts_the_cursor_on_its_card() {
    let diff = diff_of(PATCH);
    let review = review();
    let mut view = view(&diff, &review, 80, 12);
    view.focus_thread(&CommentId::parse("u2").unwrap());
    assert_eq!(view.cursor, 26);
    view.focus_thread(&CommentId::parse("u9").unwrap());
    assert_eq!(view.cursor, 26);
}

#[test]
fn the_editor_goes_under_the_cursor_then_above_it_then_to_the_bottom() {
    let stream = Rect::new(20, 0, 60, 20);
    // It is as wide as the rectangle it is given.
    assert_eq!(editor_rect(stream, 5, 4), Rect::new(20, 6, 60, 4));
    assert_eq!(editor_rect(stream, 17, 4), Rect::new(20, 13, 60, 4));
    assert_eq!(editor_rect(stream, 2, 19), Rect::new(20, 1, 60, 19));
    assert_eq!(
        editor_rect(Rect::new(0, 3, 30, 8), 1, 20),
        Rect::new(0, 3, 30, 8)
    );
}

#[test]
fn a_comment_s_rows_are_its_line_its_range_or_its_file_header_in_both_layouts() {
    let diff = diff_of(PATCH);
    let anchor = |path: &str, target| Anchor {
        path: RelPath::parse(path).unwrap(),
        old_path: None,
        target,
        spec: Spec::WorkTree,
    };
    let range = |side, start, end| AnchorTarget::Range {
        side,
        start,
        end,
        text: String::new(),
    };
    let old = AnchorTarget::Line {
        side: Side::Old,
        line: 2,
        text: "a2".into(),
    };
    // Unified: a.rs is rows 0..=9, with `-a2` on row 3 and `+A2` on row 4.
    let unified = view(&diff, &Review::default(), 80, 12);
    assert_eq!(
        unified.rows_of(&diff, &anchor("a.rs", line(2, "A2"))),
        Some((4, 4))
    );
    assert_eq!(
        unified.rows_of(&diff, &anchor("a.rs", old.clone())),
        Some((3, 3))
    );
    assert_eq!(
        unified.rows_of(&diff, &anchor("a.rs", range(Side::New, 2, 11))),
        Some((4, 8))
    );
    assert_eq!(
        unified.rows_of(&diff, &anchor("b.rs", AnchorTarget::File)),
        Some((10, 10))
    );
    assert_eq!(
        unified.rows_of(&diff, &anchor("gone.rs", line(1, "x"))),
        None
    );
    assert_eq!(unified.rows_of(&diff, &anchor("a.rs", line(99, "x"))), None);
    // Side by side pairs `-a2` with `+A2`, so both sides of line 2 are one row.
    let split = view(&diff, &Review::default(), 130, 12);
    let new = split.rows_of(&diff, &anchor("a.rs", line(2, "A2")));
    assert!(new.is_some());
    assert_eq!(new, split.rows_of(&diff, &anchor("a.rs", old)));
}

#[test]
fn the_mark_tints_its_rows_and_puts_a_bar_where_no_digit_is() {
    let (diff, view) = plain();
    let theme = Theme::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal
        .draw(|frame| {
            draw(
                frame,
                &view,
                &diff,
                &Keymap::default(),
                &theme,
                Some((3, 4)),
                &Cache::default(),
            );
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    // The stream starts at column 20.
    for y in [3, 4] {
        assert_eq!(buffer[(20, y)].symbol(), "▌", "row {y}");
        assert_eq!(buffer[(20, y)].fg, theme.warning, "row {y}");
        assert_eq!(buffer[(60, y)].bg, theme.selection, "row {y}");
    }
    assert_ne!(buffer[(20, 2)].symbol(), "▌");
    assert_ne!(buffer[(60, 5)].bg, theme.selection);
    assert!(!screen(&terminal).contains("[+]"));
}

#[test]
fn a_selected_range_is_drawn_with_the_cursor_on_its_end() {
    let (diff, mut view) = plain();
    view.move_to(2);
    view.toggle_select();
    view.move_to(4);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal
        .draw(|frame| {
            draw(
                frame,
                &view,
                &diff,
                &Keymap::default(),
                &Theme::default(),
                None,
                &Cache::default(),
            );
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    // The stream starts at column 20. Rows 2 and 3 are selected, and row 4 is the cursor.
    assert_ne!(buffer[(40, 1)].bg, Theme::default().selection);
    assert_eq!(buffer[(40, 2)].bg, Theme::default().selection);
    assert_eq!(buffer[(40, 3)].bg, Theme::default().selection);
    assert_eq!(buffer[(40, 4)].bg, Theme::default().cursor);
    assert_ne!(buffer[(40, 5)].bg, Theme::default().cursor);
}

const TREE: &str = "diff --git a/app/a.js b/app/a.js
--- a/app/a.js
+++ b/app/a.js
@@ -1,2 +1,3 @@
 keep
-gone
+one
+two
diff --git a/app/b.js b/app/b.js
--- a/app/b.js
+++ b/app/b.js
@@ -1 +1 @@
-x
+y
diff --git a/top.md b/top.md
--- a/top.md
+++ b/top.md
@@ -1 +1,2 @@
 t
+u
";

#[test]
fn the_sidebar_groups_files_under_their_directory_with_letters_and_counts() {
    let diff = diff_of(TREE);
    let view = view(&diff, &Review::default(), 80, 12);
    assert_eq!(
        view.stream.side,
        [
            SideRow::Heading("app/".into()),
            SideRow::File(0),
            SideRow::File(1),
            SideRow::Heading("./".into()),
            SideRow::File(2),
        ]
    );
    let screen = fresh(&view, &diff);
    let rows = screen
        .lines()
        .map(|row| row.split('│').next().unwrap_or("").to_owned());
    // The first three rows are the filter's box.
    let rows = rows.skip(3).take(5).collect::<Vec<_>>();
    assert_eq!(rows[0].trim_end(), "app/");
    assert_eq!(rows[1], format!("{:<14}+2 -1", " M a.js"));
    assert_eq!(rows[2], format!("{:<14}+1 -1", " M b.js"));
    assert_eq!(rows[3].trim_end(), "./");
    assert_eq!(rows[4], format!("{:<17}+1", " M top.md"));
}

#[test]
fn a_directory_that_comes_back_gets_a_second_heading() {
    let patch = format!(
        "{TREE}diff --git a/app/z.js b/app/z.js\n--- a/app/z.js\n+++ b/app/z.js\n@@ -1 +1 @@\n-1\n+2\n"
    );
    let diff = diff_of(&patch);
    let headings = sidebar_rows(&diff, None)
        .into_iter()
        .filter(|row| matches!(row, SideRow::Heading(_)))
        .count();
    assert_eq!(headings, 3);
}

#[test]
fn a_file_with_an_unsent_thread_carries_a_mark_and_the_others_do_not() {
    let diff = diff_of(TREE);
    let mut review = Review {
        threads: vec![thread("u1", "app/b.js", line(1, "y"))],
        ..Review::default()
    };
    review.threads[0].unsent = true;
    let view = view(&diff, &review, 80, 12);
    assert_eq!(view.stream.unsent, [false, true, false]);
    let screen = fresh(&view, &diff);
    assert!(
        screen.contains("•M b.js") && screen.contains(" M a.js"),
        "{screen}"
    );
    review.threads[0].unsent = false;
    let sent = self::view(&diff, &review, 80, 12);
    assert_eq!(sent.stream.unsent, [false, false, false]);
}

#[test]
fn the_sidebar_follows_the_cursor_through_headings() {
    let diff = diff_of(TREE);
    let mut view = view(&diff, &Review::default(), 80, 4);
    view.apply(Action::SwitchPanel);
    view.apply(Action::Down);
    view.apply(Action::Down);
    assert_eq!(view.stream.file_at(view.cursor), 2);
    assert!(fresh(&view, &diff).contains("M top.md"));
}

#[test]
fn the_cursors_file_has_a_bar_in_the_sidebar_that_is_dimmer_while_the_stream_is_focused() {
    let diff = diff_of(TREE);
    let mut view = view(&diff, &Review::default(), 80, 20);
    let theme = Theme::default();
    let bars = |view: &View| {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let (keymap, cache) = (Keymap::default(), Cache::default());
        terminal
            .draw(|frame| draw(frame, view, &diff, &keymap, &theme, None, &cache))
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The background of the sidebar's rows, at a column inside it.
        (0..20).map(|y| buffer[(5, y)].bg).collect::<Vec<_>>()
    };
    let count = |rows: &[Color], colour| rows.iter().filter(|bg| **bg == colour).count();
    let stream = bars(&view);
    assert_eq!(count(&stream, theme.header), 1);
    assert_eq!(count(&stream, theme.cursor), 0);
    view.apply(Action::SwitchPanel);
    let sidebar = bars(&view);
    assert_eq!(count(&sidebar, theme.cursor), 1);
    assert_eq!(count(&sidebar, theme.header), 0);
    // The bar is on the same row in both.
    let row = |rows: &[Color], colour| rows.iter().position(|bg| *bg == colour);
    assert_eq!(row(&stream, theme.header), row(&sidebar, theme.cursor));
}

const SPLIT_PATCH: &str = "diff --git a/s.rs b/s.rs
--- a/s.rs
+++ b/s.rs
@@ -3,5 +3,4 @@
 c3
-old4
-old5
+new4
 c6
 c7
@@ -20,2 +19,3 @@
 c20
+add21
 c22
";
// Split rows of s.rs: header 0, gap 1, hunk 2, c3 3, old4/new4 4, old5/- 5, c6 6, c7 7, gap 8,
// hunk 9, c20 10, -/add21 11, c22 12.

fn kinds_of(view: &View, diff: &Diff) -> Vec<String> {
    (0..view.stream.len())
        .map(|row| match view.stream.locate(diff, row).unwrap() {
            RowRef::File(_) => "file".to_owned(),
            RowRef::Hunk(_) => "hunk".to_owned(),
            RowRef::Gap(n) => format!("gap{n}"),
            RowRef::Pair { old, new } => format!(
                "{}|{}",
                old.map_or("-", |row| row.text.as_str()),
                new.map_or("-", |row| row.text.as_str())
            ),
            RowRef::Line(row) => row.text.clone(),
            _ => "other".to_owned(),
        })
        .collect()
}

#[test]
fn a_wide_pane_pairs_removed_with_added_and_marks_the_unchanged_lines() {
    let diff = diff_of(SPLIT_PATCH);
    let view = view(&diff, &Review::default(), 130, 14);
    assert_eq!(view.layout(), DiffLayout::Split);
    assert_eq!(
        kinds_of(&view, &diff),
        [
            "file",
            "gap2",
            "hunk",
            "c3|c3",
            "old4|new4",
            "old5|-",
            "c6|c6",
            "c7|c7",
            "gap12",
            "hunk",
            "c20|c20",
            "-|add21",
            "c22|c22"
        ]
    );
    assert_eq!(view.stream.hunk_rows, [2, 9]);
}

#[test]
fn a_narrow_pane_stays_unified_without_gap_rows() {
    let diff = diff_of(SPLIT_PATCH);
    let view = view(&diff, &Review::default(), 119, 14);
    assert_eq!(view.layout(), DiffLayout::Unified);
    assert_eq!(
        kinds_of(&view, &diff),
        [
            "file", "hunk", "c3", "old4", "old5", "new4", "c6", "c7", "hunk", "c20", "add21", "c22"
        ]
    );
}

#[test]
fn the_toggle_forces_the_other_layout_and_the_width_no_longer_decides() {
    let diff = diff_of(SPLIT_PATCH);
    let mut view = view(&diff, &Review::default(), 130, 14);
    view.toggle_layout();
    assert_eq!(view.layout(), DiffLayout::Unified);
    assert!(view.needs_rebuild());
    view.rebuild(&diff, &Review::default(), None, &Look::test());
    assert!(!view.needs_rebuild());
    view.resize(Rect::new(0, 0, 200, 14));
    assert_eq!(view.layout(), DiffLayout::Unified);
    view.toggle_layout();
    assert_eq!(view.layout(), DiffLayout::Split);
    let mut narrow = self::view(&diff, &Review::default(), 90, 14);
    narrow.toggle_layout();
    assert_eq!(narrow.layout(), DiffLayout::Split);
}

#[test]
fn a_split_row_draws_old_and_new_on_one_line_with_an_empty_half_opposite() {
    let diff = diff_of(SPLIT_PATCH);
    let view = view(&diff, &Review::default(), 130, 14);
    let screen = fresh(&view, &diff);
    assert!(screen.contains("▾ 2 unchanged lines"), "{screen}");
    assert!(screen.contains("▾ 12 unchanged lines"));
    let row = screen.lines().find(|row| row.contains("old4")).unwrap();
    let (old, new) = row.split_once("old4").unwrap();
    assert!(old.ends_with("4 - ") && new.contains("new4"), "{row}");
    assert!(row.matches('│').count() >= 2, "{row}");
    let alone = screen.lines().find(|row| row.contains("old5")).unwrap();
    assert!(!alone.contains("new"), "{alone}");
    assert!(screen.lines().all(|row| string_width(row) <= 130));
    let unified = fresh(&self::view(&diff, &Review::default(), 100, 14), &diff);
    assert!(!unified.contains("unchanged"));
}

#[test]
fn the_file_header_shows_the_counts_on_the_right() {
    let diff = diff_of(SPLIT_PATCH);
    let view = view(&diff, &Review::default(), 130, 14);
    let screen = fresh(&view, &diff);
    let header = screen.lines().find(|row| row.contains("M s.rs")).unwrap();
    let stream = header.split('│').nth(1).unwrap_or(header);
    assert!(stream.trim_end().ends_with("+2 -2"), "{header}");
}

fn click(view: &mut View, column: u16, row: u16) {
    view.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

#[test]
fn a_comment_on_a_split_row_takes_the_new_side_unless_a_click_chose_the_old_one() {
    let diff = diff_of(SPLIT_PATCH);
    let mut view = view(&diff, &Review::default(), 130, 14);
    let line = |side, line, text: &str| AnchorTarget::Line {
        side,
        line,
        text: text.into(),
    };
    assert_eq!(
        target_at(&mut view, &diff, 4),
        Ok(line(Side::New, 4, "new4"))
    );
    // A removed line with nothing opposite is the old side, and so is a change in a hunk's end.
    assert_eq!(
        target_at(&mut view, &diff, 5),
        Ok(line(Side::Old, 5, "old5"))
    );
    assert_eq!(
        target_at(&mut view, &diff, 11),
        Ok(line(Side::New, 20, "add21"))
    );
    // Clicks: the left half is the old side, the right half the new side.
    let stream = view.areas().stream;
    let (left, _) = split_widths(usize::from(stream.width));
    let y = stream.y + 4;
    click(&mut view, stream.x + 3, y);
    assert_eq!(
        view.capture(&diff).unwrap().target,
        line(Side::Old, 4, "old4")
    );
    click(&mut view, stream.x + u16::try_from(left).unwrap() + 3, y);
    assert_eq!(
        view.capture(&diff).unwrap().target,
        line(Side::New, 4, "new4")
    );
    // A click on the empty half of a row falls back to the side that has the line.
    click(
        &mut view,
        stream.x + u16::try_from(left).unwrap() + 3,
        stream.y + 5,
    );
    assert_eq!(
        view.capture(&diff).unwrap().target,
        line(Side::Old, 5, "old5")
    );
    // Moving the cursor forgets the click.
    click(&mut view, stream.x + 3, y);
    view.apply(Action::Down);
    view.apply(Action::Up);
    assert_eq!(
        view.capture(&diff).unwrap().target,
        line(Side::New, 4, "new4")
    );
}

#[test]
fn a_range_over_split_rows_follows_the_side_of_its_first_row() {
    let diff = diff_of(SPLIT_PATCH);
    let mut view = view(&diff, &Review::default(), 130, 14);
    view.move_to(5);
    view.toggle_select();
    let got = target_at(&mut view, &diff, 7).unwrap();
    // Starting on a removed-only row the range is old: old5, c6 (old 6), c7 (old 7).
    assert_eq!(
        got,
        AnchorTarget::Range {
            side: Side::Old,
            start: 5,
            end: 7,
            text: "old5".into()
        }
    );
    let mut view = self::view(&diff, &Review::default(), 130, 14);
    view.move_to(3);
    view.toggle_select();
    let got = target_at(&mut view, &diff, 7).unwrap();
    assert_eq!(
        got,
        AnchorTarget::Range {
            side: Side::New,
            start: 3,
            end: 6,
            text: "c3".into()
        }
    );
}

#[test]
fn a_card_hangs_under_its_split_row_and_the_cursor_survives_a_layout_change() {
    let diff = diff_of(SPLIT_PATCH);
    let review = Review {
        threads: vec![thread("u1", "s.rs", line(4, "new4"))],
        ..Review::default()
    };
    let mut view = view(&diff, &review, 130, 14);
    // The card sits under old4|new4, which is row 4, so its first row is 5.
    assert_eq!(view.stream.card_rows, [5]);
    let add21 = 11 + view.stream.cards[0].lines.len();
    view.move_to(add21);
    assert!(matches!(
        view.stream.locate(&diff, view.cursor),
        Some(RowRef::Pair { new: Some(row), .. }) if row.text == "add21"
    ));
    let spot = view.spot(&diff);
    view.toggle_layout();
    view.rebuild(&diff, &review, spot, &Look::test());
    assert!(matches!(
        view.stream.locate(&diff, view.cursor),
        Some(RowRef::Line(row)) if row.text == "add21"
    ));
    let spot = view.spot(&diff);
    view.toggle_layout();
    view.rebuild(&diff, &review, spot, &Look::test());
    assert!(matches!(
        view.stream.locate(&diff, view.cursor),
        Some(RowRef::Pair { new: Some(row), .. }) if row.text == "add21"
    ));
}

fn motion(view: &mut View, column: u16, row: u16) {
    view.mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
}

fn plus_rows(screen: &str) -> Vec<usize> {
    let rows = screen.lines().enumerate();
    rows.filter(|(_, row)| row.contains("[+]"))
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn hovering_a_code_row_shows_a_plus_on_it_and_nowhere_else() {
    let (diff, mut view) = plain();
    let stream = view.areas().stream;
    // Row 2 is a1, a context line. Rows 0 and 1 are the file header and the hunk header.
    motion(&mut view, stream.x + 12, stream.y + 2);
    assert_eq!(view.plus(), Some(Plus { row: 2, col: 0 }));
    assert_eq!(plus_rows(&fresh(&view, &diff)), [2]);
    motion(&mut view, stream.x + 12, stream.y + 3);
    assert_eq!(plus_rows(&fresh(&view, &diff)), [3]);
    for header in [0, 1] {
        motion(&mut view, stream.x + 12, stream.y + header);
        assert_eq!(view.plus(), None, "row {header}");
    }
    // Off the stream, in the sidebar, there is none.
    motion(&mut view, 2, 3);
    assert_eq!(view.plus(), None);
}

#[test]
fn without_any_motion_the_plus_is_on_the_cursor_row() {
    let (diff, mut view) = plain();
    view.move_to(3);
    assert_eq!(view.plus(), Some(Plus { row: 3, col: 0 }));
    assert_eq!(plus_rows(&fresh(&view, &diff)), [3]);
    view.move_to(1);
    assert_eq!(view.plus(), None);
    // Once the mouse has moved, the cursor no longer shows one.
    let stream = view.areas().stream;
    view.move_to(3);
    motion(&mut view, 2, 0);
    assert_eq!(view.plus(), None);
    motion(&mut view, stream.x + 12, stream.y + 4);
    assert_eq!(view.plus().map(|p| p.row), Some(4));
}

#[test]
fn the_plus_follows_the_wheel_while_the_mouse_stands_still() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    motion(&mut view, stream.x + 12, stream.y + 2);
    assert_eq!(view.plus().map(|p| p.row), Some(2));
    view.mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: stream.x + 12,
        row: stream.y + 2,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(view.plus().map(|p| p.row), Some(2 + view.scroll));
}

#[test]
fn a_click_on_the_plus_asks_for_a_comment_and_a_click_elsewhere_only_moves_the_cursor() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    let down = |view: &mut View, column: u16, row: u16| {
        view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };
    motion(&mut view, stream.x + 1, stream.y + 3);
    assert!(down(&mut view, stream.x + 1, stream.y + 3));
    assert_eq!(view.cursor, 3);
    assert!(!down(&mut view, stream.x + 12, stream.y + 3));
    assert!(
        !down(&mut view, stream.x + 3, stream.y + 3),
        "the cell after the marker"
    );
    // The header row has no marker to click.
    motion(&mut view, stream.x + 1, stream.y);
    assert!(!down(&mut view, stream.x + 1, stream.y));
}

#[test]
fn in_a_split_row_the_plus_sits_on_the_hovered_half_and_a_click_on_it_chooses_that_half() {
    let diff = diff_of(SPLIT_PATCH);
    let mut view = view(&diff, &Review::default(), 130, 14);
    let stream = view.areas().stream;
    let (left, _) = split_widths(usize::from(stream.width));
    let y = stream.y + 4;
    motion(&mut view, stream.x + 10, y);
    assert_eq!(view.plus(), Some(Plus { row: 4, col: 4 }));
    let screen = fresh(&view, &diff);
    let row = screen.lines().nth(4).unwrap();
    assert!(row.contains("   4[+]old4"), "the number stays: {row}");
    let right = u16::try_from(left).unwrap() + 10;
    motion(&mut view, stream.x + right, y);
    assert_eq!(
        view.plus(),
        Some(Plus {
            row: 4,
            col: left + 5
        })
    );
    let cell = stream.x + u16::try_from(left + 5).unwrap();
    let hit = view.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: cell,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(hit);
    assert_eq!(
        view.capture(&diff).unwrap().target,
        AnchorTarget::Line {
            side: Side::New,
            line: 4,
            text: "new4".into()
        }
    );
    // On the old half of the same row it is the old line.
    motion(&mut view, stream.x + 10, y);
    let hit = view.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: stream.x + 5,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(hit);
    assert!(matches!(
        view.capture(&diff).unwrap().target,
        AnchorTarget::Line {
            side: Side::Old,
            line: 4,
            ..
        }
    ));
    // Over the empty right half of old5, the marker falls to the half that has the line.
    motion(&mut view, stream.x + right, stream.y + 5);
    assert_eq!(view.plus(), Some(Plus { row: 5, col: 4 }));
}

#[test]
fn the_status_letter_has_the_colour_of_its_change() {
    let diff = diff_of(PATCH);
    let theme = Theme::default();
    let letter_color = |file: usize, letter: &str| {
        side_file_line(&diff.files[file], false, false, 24, &theme)
            .spans
            .iter()
            .find(|span| span.content == letter)
            .and_then(|span| span.style.fg)
    };
    assert_eq!(letter_color(0, "M"), Some(theme.warning));
    assert_eq!(letter_color(1, "A"), Some(theme.added));
    assert_eq!(letter_color(2, "B"), Some(theme.subtle));
    let header = file_header(&diff.files[0], 30, &theme);
    assert_eq!(header.spans[0].content, "M");
    assert_eq!(header.spans[0].style.fg, Some(theme.warning));
}

fn side_row(file: usize, icons: bool, width: usize) -> Line<'static> {
    side_file_line(
        &diff_of(PATCH).files[file],
        false,
        icons,
        width,
        &Theme::default(),
    )
}

fn text_of(line: &Line) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn an_icon_sits_between_the_letter_and_the_name_in_the_subtle_colour() {
    let theme = Theme::default();
    let line = side_row(0, true, 24);
    assert_eq!(
        text_of(&line),
        format!(" M {} a.rs          +2 -1", icon("a.rs"))
    );
    assert_eq!(string_width(&text_of(&line)), 24);
    let glyph = line
        .spans
        .iter()
        .find(|span| span.content == icon("a.rs").to_string());
    assert_eq!(glyph.and_then(|span| span.style.fg), Some(theme.subtle));
}

#[test]
fn a_row_without_icons_is_the_row_as_it_was() {
    let line = side_row(0, false, 24);
    assert_eq!(text_of(&line), " M a.rs            +2 -1");
    assert_eq!(string_width(&text_of(&line)), 24);
}

#[test]
fn a_narrow_sidebar_cuts_the_name_and_keeps_the_icon_before_the_counts() {
    let line = side_row(0, true, 11);
    assert_eq!(text_of(&line), format!(" M {} …+2 -1", icon("a.rs")));
    assert_eq!(string_width(&text_of(&line)), 11);
}

#[test]
fn a_row_always_fits_its_width() {
    // Below 5 cells the counts alone do not fit, which the name cannot help.
    for icons in [false, true] {
        for width in 5..30 {
            let line = text_of(&side_row(0, icons, width));
            assert_eq!(string_width(&line), width, "{icons} {width} {line:?}");
        }
    }
}

#[test]
fn scrolling_sideways_moves_the_code_and_leaves_the_gutter() {
    let long = "0123456789".repeat(10);
    let patch = format!(
        "diff --git a/l.rs b/l.rs\nnew file mode 100644\n--- /dev/null\n+++ b/l.rs\n@@ -0,0 +1,1 @@\n+{long}\ndiff --git a/s.rs b/s.rs\nnew file mode 100644\n--- /dev/null\n+++ b/s.rs\n@@ -0,0 +1,1 @@\n+short\n"
    );
    let diff = diff_of(&patch);
    let mut view = view(&diff, &Review::default(), 49, 12);
    let start = fresh(&view, &diff);
    assert!(start.contains("    1 +0123456789"), "{start}");
    assert!(start.contains('›') && !start.contains('‹'), "{start}");
    view.apply(Action::ScrollRight);
    assert_eq!(view.hscroll, HSCROLL_COLS);
    let moved = fresh(&view, &diff);
    assert!(moved.contains("    1 +‹9012345678"), "{moved}");
    view.apply(Action::ScrollLeft);
    view.apply(Action::ScrollLeft);
    assert_eq!(view.hscroll, 0);
    // It stops where the widest line ends.
    for _ in 0..20 {
        view.apply(Action::ScrollRight);
    }
    assert_eq!(view.hscroll, 100 - view.code_room());
    assert!(!fresh(&view, &diff).contains('›'));
    // Another file starts over, and so does the reset key.
    view.move_to_file(1);
    assert_eq!(view.hscroll, 0);
    view.hscroll = 8;
    view.apply(Action::ScrollReset);
    assert_eq!(view.hscroll, 0);
}

fn mouse_event(view: &mut View, kind: MouseEventKind, column: u16, row: u16) -> bool {
    view.mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn press_at(view: &mut View, column: u16, row: u16) -> bool {
    mouse_event(view, MouseEventKind::Down(MouseButton::Left), column, row)
}

fn drag_to(view: &mut View, column: u16, row: u16) {
    mouse_event(view, MouseEventKind::Drag(MouseButton::Left), column, row);
}

fn release(view: &mut View) {
    mouse_event(view, MouseEventKind::Up(MouseButton::Left), 0, 0);
}

/// The pane starts at screen row 4, so there are rows above the stream to drag over.
fn lowered() -> (Diff, View) {
    let (diff, mut view) = plain();
    view.resize(Rect::new(0, 4, 80, 14));
    (diff, view)
}

#[test]
fn dragging_from_a_press_selects_from_the_press_row_and_keeps_it_after_release() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    press_at(&mut view, stream.x + 4, stream.y + 2);
    assert_eq!(view.select, None);
    drag_to(&mut view, stream.x + 4, stream.y + 3);
    drag_to(&mut view, stream.x + 9, stream.y + 4);
    assert_eq!(view.select, Some(Select { row: 2, half: None }));
    assert_eq!(view.cursor, 4);
    release(&mut view);
    assert_eq!(
        (view.select.map(|select| select.row), view.cursor),
        (Some(2), 4)
    );
    // The gesture is over, so a drag without a new press does nothing.
    drag_to(&mut view, stream.x + 4, stream.y + 7);
    assert_eq!(view.cursor, 4);
}

#[test]
fn a_drag_that_stays_on_the_press_row_still_selects() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    press_at(&mut view, stream.x + 4, stream.y + 3);
    drag_to(&mut view, stream.x + 8, stream.y + 3);
    assert_eq!(view.select, Some(Select { row: 3, half: None }));
}

#[test]
fn a_drag_whose_press_was_not_in_the_stream_changes_nothing() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    // A click in the sidebar on the second file moves the cursor there.
    press_at(&mut view, 2, 2);
    let before = view.clone();
    drag_to(&mut view, stream.x + 4, stream.y + 5);
    drag_to(&mut view, stream.x + 4, stream.y + 8);
    assert_eq!(view, before);
    // A press past the end of the stream holds nothing either.
    view.resize(Rect::new(0, 0, 80, 40));
    press_at(&mut view, stream.x + 4, stream.y + 30);
    drag_to(&mut view, stream.x + 4, stream.y + 5);
    assert_eq!(view.select, None);
}

#[test]
fn a_drag_above_the_stream_scrolls_up_one_row_and_stops_at_the_top() {
    let (_, mut view) = lowered();
    let stream = view.areas().stream;
    mouse_event(&mut view, MouseEventKind::ScrollDown, stream.x, stream.y);
    assert_eq!(view.scroll, 3);
    press_at(&mut view, stream.x + 4, stream.y + 5);
    assert_eq!(view.cursor, 8);
    for (scroll, cursor) in [(2, 2), (1, 1), (0, 0), (0, 0)] {
        // The column does not matter, and the row is above the pane.
        drag_to(&mut view, 70, stream.y - 3);
        assert_eq!((view.scroll, view.cursor), (scroll, cursor));
    }
    assert_eq!(view.select, Some(Select { row: 8, half: None }));
}

#[test]
fn a_drag_below_the_stream_scrolls_down_one_row_and_stops_at_the_end() {
    let (_, mut view) = lowered();
    let stream = view.areas().stream;
    press_at(&mut view, stream.x + 4, stream.y + 2);
    // The stream has 16 rows and shows 12, so it scrolls 4 rows at most.
    let below = stream.y + stream.height + 5;
    for (scroll, cursor) in [(1, 12), (2, 13), (3, 14), (4, 15), (4, 15)] {
        drag_to(&mut view, 1, below);
        assert_eq!((view.scroll, view.cursor), (scroll, cursor));
    }
    // Dragging back into the stream moves the cursor to that row.
    drag_to(&mut view, 40, stream.y + 1);
    assert_eq!(view.cursor, 5);
}

#[test]
fn a_plain_press_leaves_visual_mode_and_one_on_the_plus_keeps_it() {
    let (_, mut view) = plain();
    let stream = view.areas().stream;
    press_at(&mut view, stream.x + 4, stream.y + 2);
    drag_to(&mut view, stream.x + 4, stream.y + 4);
    release(&mut view);
    assert!(view.select.is_some());
    // The `[+]` of row 5 is in its first three cells once the mouse is over the row.
    mouse_event(&mut view, MouseEventKind::Moved, stream.x + 1, stream.y + 5);
    assert!(press_at(&mut view, stream.x + 1, stream.y + 5));
    assert_eq!(
        (view.select.map(|select| select.row), view.cursor),
        (Some(2), 5)
    );
    // Any other press drops the selection and moves the cursor.
    assert!(!press_at(&mut view, stream.x + 8, stream.y + 7));
    assert_eq!((view.select, view.cursor), (None, 7));
}

#[test]
fn a_drag_that_starts_on_the_old_half_gives_an_old_side_range_in_both_directions() {
    let diff = diff_of(SPLIT_PATCH);
    let mut view = view(&diff, &Review::default(), 130, 14);
    let stream = view.areas().stream;
    let range = |start, end| AnchorTarget::Range {
        side: Side::Old,
        start,
        end,
        text: "old4".into(),
    };
    // Down from the old half of row 4 to row 6, and then up from row 6 to row 4.
    press_at(&mut view, stream.x + 3, stream.y + 4);
    drag_to(&mut view, stream.x + 3, stream.y + 6);
    assert_eq!(view.capture(&diff).unwrap().target, range(4, 6));
    release(&mut view);
    press_at(&mut view, stream.x + 3, stream.y + 6);
    drag_to(&mut view, stream.x + 3, stream.y + 4);
    assert_eq!(view.capture(&diff).unwrap().target, range(4, 6));
    // The same on the new half starts from the new side.
    let left = u16::try_from(split_widths(usize::from(stream.width)).0).unwrap();
    release(&mut view);
    press_at(&mut view, stream.x + left + 3, stream.y + 6);
    drag_to(&mut view, stream.x + left + 3, stream.y + 4);
    assert_eq!(
        view.capture(&diff).unwrap().target,
        AnchorTarget::Range {
            side: Side::New,
            start: 4,
            end: 5,
            text: "new4".into()
        }
    );
    // `v` after a click remembers the half through the motions that clear the click.
    press_at(&mut view, stream.x + 3, stream.y + 4);
    view.toggle_select();
    view.apply(Action::Down);
    view.apply(Action::Down);
    assert_eq!(view.capture(&diff).unwrap().target, range(4, 6));
}

#[test]
fn a_rebuild_in_the_middle_of_a_drag_drops_the_selection_and_the_gesture() {
    let (diff, mut view) = plain();
    let stream = view.areas().stream;
    press_at(&mut view, stream.x + 4, stream.y + 2);
    drag_to(&mut view, stream.x + 4, stream.y + 4);
    assert!(view.select.is_some() && view.drag);
    view.rebuild(&diff, &Review::default(), None, &Look::test());
    assert!(view.select.is_none() && !view.drag);
    // With no press behind it, the next drag selects nothing until a new press.
    drag_to(&mut view, stream.x + 4, stream.y + 6);
    assert_eq!(view.select, None);
    press_at(&mut view, stream.x + 4, stream.y + 6);
    drag_to(&mut view, stream.x + 4, stream.y + 8);
    assert_eq!(view.select.map(|select| select.row), Some(6));
}

#[test]
fn a_query_matches_the_characters_of_a_path_in_order() {
    assert!(matches("vw", "src/view.rs"));
    assert!(matches("s/v", "src/view.rs"));
    assert!(!matches("wv", "src/view.rs"));
    assert!(!matches("vww", "src/view.rs"));
    assert!(matches("", "src/view.rs"));
}

#[test]
fn a_query_with_no_upper_case_letter_ignores_case_and_one_that_has_makes_it_exact() {
    assert!(matches("readme", "README.md"));
    assert!(matches("README", "README.md"));
    assert!(!matches("Readme", "README.md"));
    assert!(!matches("Src", "src/view.rs"));
}

#[test]
fn a_directory_with_no_match_has_no_heading_and_matches_around_a_skipped_file_share_one() {
    let file = |path: &str| {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-1\n+2\n")
    };
    let patch = ["src/a.rs", "lib/x.rs", "src/b.rs", "doc/c.md"]
        .map(file)
        .concat();
    let diff = diff_of(&patch);
    // `doc/` has no match, so it has no heading.
    assert_eq!(
        sidebar_rows(&diff, Some("rs")),
        [
            SideRow::Heading("src/".into()),
            SideRow::File(0),
            SideRow::Heading("lib/".into()),
            SideRow::File(1),
            SideRow::Heading("src/".into()),
            SideRow::File(2),
        ]
    );
    // `lib/x.rs` is skipped, so the two `src/` files share one heading.
    assert_eq!(
        sidebar_rows(&diff, Some("src")),
        [
            SideRow::Heading("src/".into()),
            SideRow::File(0),
            SideRow::File(2)
        ]
    );
}
