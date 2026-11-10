// The comment editor drawn in the stream, and the cards in the half their line is on: the E: lines of design/graphs/gui-comment.md,
// gui-thread.md and gui-stream.md that name them. The view draws what these items say, so item building and heights are the tests.
import assert from "node:assert/strict";
import { test } from "node:test";
import { EDITOR_H, halfCardWidth, heightOf, layoutCache, revealOffset, scrolled, SPLIT_W, streamLayout } from "../src/rows.ts";
import type { BodyItem, EditorItem, StreamItem } from "../src/rows.ts";
import type { Model, Msg } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
const NOW = Date.parse("2026-11-09T10:00:30Z");
// Rows of the fixture: 0 hunk, 1 ctx 1/1, 2 del old 2, 3 add new 2, 4 add new 3, 5 ctx 3/4, 6 ctx 4/5.
// Threads: u3 old row 2, u1 new row 3, u2 new row 4 (resolved, so collapsed), u4 new row 6, u5 no row, u6 not in the diff.
const unified = (): Model => loadedModel(state, file);
const split = (): Model => ({ ...loadedModel(state, file), width: SPLIT_W });
const plus = (row: number, side: "old" | "new" | null): Msg => ({ _tag: "PlusClicked", path: "src/a.rs", row, side });
const act = (id: string, action: "reply" | "edit"): Msg => ({ _tag: "ThreadAction", id, action });
const items = (m: Model): StreamItem[] => streamLayout(m, NOW).items;
const own = (m: Model): string[] =>
  items(m)
    .filter((i) => i.key.startsWith("src/a.rs|"))
    .map((i) => i.key.slice("src/a.rs|".length));
const editors = (m: Model): EditorItem[] => items(m).filter((i): i is EditorItem => i.kind === "editor");
// What stands between two keys, which is where the editor is found.
const between = (m: Model, from: string, to: string): string[] => {
  const keys = own(m);
  return keys.slice(keys.indexOf(from), keys.indexOf(to));
};

test("E: side by side, a card sits in the half its line is on, and a file comment and the threads not in the diff take the width", () => {
  const halves = (m: Model): Record<string, string | null> => Object.fromEntries(items(m).flatMap((i) => (i.kind === "head" || i.kind === "body" ? [[i.key, i.half]] : [])));
  const m = { ...split(), expanded: new Set(["u2"]) };
  const h = halves(m);
  assert.equal(h["src/a.rs|h:u3"], "old");
  assert.equal(h["src/a.rs|h:u1"], "new");
  assert.equal(h["src/a.rs|c:u1"], "new", "the comments of a card follow its head");
  assert.equal(h["src/a.rs|c:a1"], "new");
  assert.equal(h["src/a.rs|h:u4"], "new");
  assert.equal(h["src/a.rs|h:u5"], null, "a file comment");
  assert.equal(h["|h:u6"], null, "a thread not in the diff");
  assert.ok(
    Object.values(halves({ ...m, split: false })).every((x) => x === null),
    "unified is as it was",
  );
});

test("a card in a half is counted at the half's width", () => {
  const body = (half: "old" | null): BodyItem => ({ kind: "body", key: "x|c:1", id: "1", tone: "user", meta: null, lines: ["x".repeat(200)], buttons: [], focused: false, last: true, half });
  // 1000 px: the half is 499.5, less 28 on the left and 12 on the right.
  assert.equal(halfCardWidth(1000), 459.5);
  assert.ok(heightOf(body("old"), 1000) > heightOf(body(null), 1000), "the same text wraps more in a half than across the stream");
  // 459.5 - 22 of padding is 58 characters of 7.5 px: 200 characters take 4 lines.
  assert.equal(heightOf(body("old"), 1000), 6 + 4 * 18 + 6);
});

test("E: the [+] opens the editor under its row's cards, in the half it was clicked on in a split stream", () => {
  const right = steps(split(), plus(3, "new")).model;
  assert.deepEqual(between(right, "p2", "p4"), ["p2", "h:u3", "h:u1", "c:u1", "editor"]);
  assert.deepEqual(
    editors(right).map((e) => [e.key, e.half, e.title]),
    [["src/a.rs|editor", "new", "Draft note - src/a.rs R2"]],
  );
  // On the old half of the pair it hangs on the left, after the same cards.
  const left = steps(split(), plus(2, "old")).model;
  assert.deepEqual(between(left, "p2", "p4"), ["p2", "h:u3", "h:u1", "c:u1", "editor"]);
  assert.equal(editors(left)[0]?.half, "old");
  // A row without cards: the editor follows the row.
  assert.deepEqual(between(steps(split(), plus(1, "new")).model, "p1", "p2"), ["p1", "editor"]);
});

test("E: the [+] in the unified layout opens the editor under its row's cards, across the width", () => {
  const m = steps(unified(), plus(3, null)).model;
  assert.deepEqual(between(m, "r3", "r4"), ["r3", "h:u1", "c:u1", "editor"]);
  assert.equal(editors(m)[0]?.half, null);
  // A range goes under its last row, after that row's cards.
  const range = steps(unified(), { _tag: "RowPressed", path: "src/a.rs", row: 3, side: null }, { _tag: "Hovered", path: "src/a.rs", row: 4, side: null, on: true }, { _tag: "Released" }, plus(3, null)).model;
  assert.deepEqual(range.editor?.anchor?.end_line, 3);
  assert.deepEqual(between(range, "r4", "r5"), ["r4", "h:u2", "editor"]);
  // A new comment whose row is no longer in the file hangs at the top of the file rather than nowhere.
  const gone = { ...steps(unified(), plus(3, null)).model };
  const lost = { ...gone, editor: gone.editor === null || gone.editor.anchor === null ? null : { ...gone.editor, anchor: { ...gone.editor.anchor, line: 99, end_line: null } } };
  assert.equal(own(lost)[1], "editor");
});

test("E: a reply opens the editor under the thread's card, an edit too, in the card's half", () => {
  const reply = steps(split(), act("u1", "reply")).model;
  assert.deepEqual(between(reply, "h:u1", "p4"), ["h:u1", "c:u1", "editor"]);
  assert.equal(editors(reply)[0]?.half, "new");
  assert.equal(editors(reply)[0]?.title, "Reply to u1");
  const onOld = steps(split(), act("u3", "edit")).model;
  assert.deepEqual(between(onOld, "h:u3", "h:u1"), ["h:u3", "editor"]);
  assert.equal(editors(onOld)[0]?.half, "old");
  // A resolved thread is a head until opened, and its editor is right after it.
  assert.deepEqual(between(steps(unified(), act("u2", "edit")).model, "h:u2", "r5"), ["h:u2", "editor"]);
  assert.equal(editors(steps(unified(), act("u1", "reply")).model)[0]?.half, null);
  // The threads not in the diff have one too, under a key with no file.
  const orphan = steps(unified(), act("u6", "reply")).model;
  const keys = items(orphan).map((i) => i.key);
  assert.deepEqual(keys.slice(keys.indexOf("|h:u6"), keys.indexOf("|h:u6") + 3), ["|h:u6", "|c:u6", "|editor"]);
});

test("E: a comment on the file opens the editor at the top of the file, full width in both layouts", () => {
  for (const m of [unified(), split()]) {
    const open = steps(m, { _tag: "FileCommentClicked" }).model;
    assert.deepEqual(own(open).slice(0, 3), ["file", "editor", "h:u5"]);
    assert.deepEqual(
      editors(open).map((e) => [e.half, e.title]),
      [[null, "Draft note - src/a.rs"]],
    );
  }
});

test("the editor's height is fixed, and it is the one the stream counts", () => {
  const m = steps(split(), plus(3, "new")).model;
  const s = streamLayout(m, NOW);
  const at = s.items.findIndex((i) => i.kind === "editor");
  assert.equal(s.sizes[at], EDITOR_H);
  assert.equal(EDITOR_H, 200);
  // Whatever the width, and whatever the model's status says: the status line is kept free.
  assert.equal(heightOf(s.items[at] as StreamItem, 300), EDITOR_H);
  assert.equal(streamLayout({ ...m, status: "busy" }, NOW).sizes[at], EDITOR_H);
  // The stream is as tall as it was plus the editor.
  assert.equal(s.total, streamLayout({ ...m, editor: null }, NOW).total + EDITOR_H);
});

test("the stream is cut again when the editor opens, moves or closes, and the file's items follow", () => {
  const cached = layoutCache();
  const m = split();
  const none = cached(m, NOW);
  assert.equal(cached(m, NOW), none);
  const open = steps(m, plus(3, "new")).model;
  const withEditor = cached(open, NOW);
  assert.notEqual(withEditor, none);
  assert.ok(withEditor.items.some((i) => i.kind === "editor"));
  // Another place for it is another stream, though the file's own object is the same.
  const moved = steps(m, plus(1, "new")).model;
  assert.notEqual(cached(moved, NOW), withEditor);
  assert.deepEqual(between(moved, "p1", "p2"), ["p1", "editor"]);
  // Switching layout moves it to the other half and the item changes with the layout.
  assert.equal(editors({ ...open, split: false })[0]?.half, null);
  const closed = cached({ ...open, editor: null }, NOW);
  assert.ok(closed.items.every((i) => i.kind !== "editor"));
});

test("the agent message's editor is not the stream's", () => {
  const m = { ...steps(unified(), plus(3, null)).model, view: "message" as const };
  assert.deepEqual(editors(m), []);
  const reply = { ...steps(unified(), act("u1", "reply")).model, view: "message" as const };
  assert.deepEqual(editors(reply), []);
});

test("E: opening an editor scrolls the stream just far enough to show it", () => {
  const key = "src/a.rs|editor";
  for (const [open, name] of [
    [steps(split(), plus(3, "new")).model, "plus"],
    [steps(unified(), act("u1", "reply")).model, "reply"],
    [steps(unified(), act("u2", "edit")).model, "edit"],
    [steps(unified(), { _tag: "FileCommentClicked" }).model, "file"],
  ] as const) {
    assert.deepEqual(open.reveal, { key, near: true }, name);
    const small = { ...open, height: 300, offset: 0 };
    const stream = streamLayout(small, NOW);
    const at = stream.items.findIndex((i) => i.key === key);
    const offset = revealOffset(stream, open.reveal as NonNullable<Model["reveal"]>, 0, 300) ?? -1;
    // The whole editor is on screen, and the stream did not move more than that takes.
    assert.ok(offset <= (stream.starts[at] as number) && (stream.starts[at + 1] as number) <= offset + 300, name);
    assert.ok(offset === 0 || (stream.starts[at + 1] as number) - 300 === offset, name);
    const sync = scrolled(small, stream);
    assert.ok(sync !== null && sync._tag === "Scrolled" && sync.revealed === true, name);
  }
  // A thread's own reply: the card's key names the file the editor is in, the orphans' has none.
  assert.deepEqual(steps(unified(), act("u6", "reply")).model.reveal, { key: "|editor", near: true });
  // A key press for the editor is the same message as the click.
  assert.deepEqual(steps(unified(), { _tag: "PlusClicked", path: "src/a.rs", row: 3, side: null }).model.reveal, { key, near: true });
});

test("with the editor open a fold or a viewed mark is refused, since it would hide the editor", () => {
  const m = steps(loadedModel(fixtureState(), fixtureFile()), { _tag: "PlusClicked", path: "src/a.rs", row: 3, side: null }).model;
  assert.notEqual(m.editor, null);
  for (const msg of [
    { _tag: "FoldToggled", path: "src/a.rs" },
    { _tag: "ViewedClicked", path: "src/a.rs", viewed: true },
  ] as Msg[]) {
    const out = steps(m, msg);
    assert.deepEqual(out.cmds, []);
    assert.equal(out.model.folds, m.folds);
    assert.notEqual(out.model.editor, null);
  }
});

test("on Save the draft is drawn at once as its card, in the editor's place, and the real card that replaces it is the same height", () => {
  const NOW_T = Date.parse("2026-11-09T10:00:30Z");
  const base = loadedModel(fixtureState(), fixtureFile());
  const open = steps(base, { _tag: "PlusClicked", path: "src/a.rs", row: 3, side: null }).model;
  const before = streamLayout(open, NOW_T);
  const at = before.items.findIndex((i) => i.kind === "editor");
  assert.ok(at > 0);
  const saving = steps(open, { _tag: "EditorSaved", text: "Why?" }).model;
  const drawn = streamLayout(saving, NOW_T);
  assert.equal(
    drawn.items.some((i) => i.kind === "editor"),
    false,
  );
  const head = drawn.items[at];
  assert.equal(head?.kind === "head" && head.id, "draft");
  // The store's thread: the same anchor, row and text, with real ids.
  const anchor = open.editor?.anchor;
  assert.ok(anchor !== null && anchor !== undefined);
  const view = saving.files.get("src/a.rs");
  assert.ok(view !== undefined);
  const real = {
    id: "u9",
    status: "open" as const,
    resolved_by: null,
    is_new: false,
    unsent: true,
    placement: { kind: "matched" as const, row: 3, start_row: null },
    anchor: { path: anchor.path, old_path: anchor.old_path, kind: "line" as const, side: anchor.side, line: anchor.line, end_line: null, text: null, spec: { kind: "worktree" as const } },
    comments: [{ id: "u9", parent: null, author: "user", at: new Date(NOW_T).toISOString(), body: "Why?", sent: false, edited_since_sent: false }],
  };
  const landedModel: Model = { ...saving, editor: null, draft: null, landing: null, saving: false, selection: null, files: new Map(saving.files).set("src/a.rs", { ...view, threads: [...view.threads, real] }) };
  const after = streamLayout(landedModel, NOW_T);
  assert.equal(after.items[at]?.kind === "head" && after.items[at].id, "u9");
  assert.equal(after.total, drawn.total);
  // A failed write: the editor is drawn again, its text still in it.
  const failed = steps(saving, { _tag: "Failed", op: "write", error: { _tag: "Busy" } }).model;
  assert.equal(failed.draft, null);
  assert.equal(streamLayout(failed, NOW_T).items[at]?.kind, "editor");
});

test("a reply being saved is drawn in its thread's card at once", () => {
  const NOW_T = Date.parse("2026-11-09T10:00:30Z");
  const m = loadedModel(fixtureState(), fixtureFile());
  const thread = [...m.files.values()].flatMap((f) => f.threads).find((t) => t.status === "open");
  assert.ok(thread !== undefined);
  const saving = steps(m, { _tag: "ThreadAction", id: thread.id, action: "reply" }, { _tag: "EditorSaved", text: "on it" }).model;
  const items = streamLayout(saving, NOW_T).items;
  assert.equal(
    items.some((i) => i.kind === "editor"),
    false,
  );
  assert.ok(items.some((i) => i.kind === "body" && i.id === "draft" && i.lines.join("\n") === "on it"));
});
