// The E: lines of design/graphs/gui-keys.md: the keys of the cursor, the range and the focused thread. Each test drives
// panelKey and update, so what is checked is the key's effect on the model.
import assert from "node:assert/strict";
import { test } from "node:test";
import { cursorOn, indexOfKey, panelKey, revealOffset, ROW, scrolled, spots, SPLIT_W, streamLayout } from "../src/rows.ts";
import type { RowItem } from "../src/rows.ts";
import { update } from "../src/update.ts";
import type { Cursor, Model, Msg, Row } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
const NOW = Date.parse("2026-11-09T10:00:30Z");

const row = (kind: "ctx" | "add" | "del", n: number): Row => ({ kind, old: kind === "add" ? null : n, new: kind === "del" ? null : n, text: `t${n}`, tokens: [], marks: [] });
const hunk: Row = { kind: "hunk", old: null, new: null, text: "@@", tokens: [], marks: [] };
// 0 hunk, 1 ctx, 2 del, 3 add, 4 add, 5 ctx, 6 hunk, 7 ctx, 8 add. In side by side the pairs are 1|1, 2|3, -|4, 5|5, 7|7, -|8.
const ROWS: Row[] = [hunk, row("ctx", 1), row("del", 2), row("add", 2), row("add", 3), row("ctx", 3), hunk, row("ctx", 8), row("add", 9)];

// Files in the sidebar's order: img.png (viewed, so folded), mod, notes.txt, src/a.rs, src/b.rs. Three of them have the rows
// above and no cards, `mod` has none. The panel shows src/a.rs on top and is unified, 100 px tall (half a screen is 2 rows).
function review(): Model {
  const m = loadedModel(state, file);
  const files = new Map(m.files);
  for (const p of ["img.png", "notes.txt", "src/a.rs", "src/b.rs"]) {
    files.set(p, { ...file, path: p, threads: [], rows: ROWS });
  }
  return { ...m, files, height: 100 };
}
const wide = (m: Model): Model => ({ ...m, width: SPLIT_W });
const at = (m: Model, path: string, r: number, side: Cursor["side"] = null): Model => ({ ...m, cursor: { path, row: r, side } });

// The keys one after the other, each through panelKey and then update. A key that means nothing is dropped.
function press(m: Model, ...keys: string[]): Model {
  for (const k of keys) {
    const msg = panelKey(m, k);
    if (msg !== null) {
      m = update(m, msg)[0];
    }
  }
  return m;
}
const where = (m: Model): string => (m.cursor === null ? "none" : `${m.cursor.path}:${m.cursor.row}${m.cursor.side === null ? "" : `:${m.cursor.side}`}`);

test("there is no cursor until the first motion key, which puts it on the top code row of the screen and goes no further", () => {
  const m = review();
  assert.equal(m.cursor, null);
  const first = press(m, "j");
  assert.equal(where(first), "src/a.rs:1");
  assert.equal(where(press(m, "k")), "src/a.rs:1");
  assert.equal(where(press(m, "down")), "src/a.rs:1");
  assert.equal(where(press(m, "up")), "src/a.rs:1");
  assert.equal(where(press(first, "j")), "src/a.rs:2");
  // The item on top is a row: the cursor starts at it, not at the top of the file.
  assert.equal(where(press({ ...m, top: "src/a.rs|r4" }, "j")), "src/a.rs:4");
  // Nothing loaded, nothing to stand on.
  assert.equal(press({ ...m, files: new Map() }, "j").cursor, null);
});

test("j and k move a row, down and up too, and skip hunk headers", () => {
  const m = at(review(), "src/a.rs", 4);
  assert.equal(where(press(m, "j")), "src/a.rs:5");
  assert.equal(where(press(m, "j", "j")), "src/a.rs:7", "row 6 is a hunk header");
  assert.equal(where(press(m, "k")), "src/a.rs:3");
  assert.equal(where(press(m, "down", "down", "up")), "src/a.rs:5");
  assert.equal(where(press(at(review(), "src/a.rs", 7), "k")), "src/a.rs:5");
});

test("a motion across files goes in stream order and stops at the ends of the stream", () => {
  const last = at(review(), "src/a.rs", 8);
  assert.equal(where(press(last, "j")), "src/b.rs:1");
  assert.equal(where(press(at(review(), "src/b.rs", 1), "k")), "src/a.rs:8");
  assert.equal(where(press(at(review(), "src/a.rs", 1), "k")), "notes.txt:8");
  assert.equal(where(press(at(review(), "src/b.rs", 8), "j")), "src/b.rs:8");
});

test("files that are not loaded or are folded are skipped", () => {
  const m = review();
  // Not loaded: src/b.rs has no diff yet, so j from the end of src/a.rs stays.
  const gone = { ...at(m, "src/a.rs", 8), files: new Map([...m.files].filter(([p]) => p !== "src/b.rs")) };
  assert.equal(where(press(gone, "j")), "src/a.rs:8");
  // Folded by the user: the same.
  assert.equal(where(press({ ...at(m, "src/a.rs", 8), folds: new Map([["src/b.rs", true]]) }, "j")), "src/a.rs:8");
  // img.png is viewed, so folded, and has rows: k from notes.txt passes over it and over `mod`, which has none. Opened, it is the next one.
  assert.equal(where(press(at(m, "notes.txt", 1), "k")), "notes.txt:1");
  assert.equal(where(press({ ...at(m, "notes.txt", 1), folds: new Map([["img.png", false]]) }, "k")), "img.png:8");
  // A cursor on a file that has since folded is put on the top of the screen again, not moved from where it stood.
  assert.equal(where(press({ ...at(m, "src/b.rs", 3), folds: new Map([["src/b.rs", true]]) }, "j")), "src/a.rs:1");
});

test("a motion asks the stream to bring the row on screen, as little as it takes, and drops the focused thread", () => {
  const m = { ...at(review(), "src/a.rs", 8), focus: "u1" };
  const next = press(m, "j");
  assert.deepEqual(next.reveal, { key: "src/b.rs|r1", near: true });
  assert.equal(next.focus, null);
  const stream = streamLayout(next, NOW);
  const i = indexOfKey(stream.items, "src/b.rs|r1");
  const start = stream.starts[i] as number;
  // Below the screen: the row ends at its bottom edge. Above: it is at the top. On screen: the offset stays.
  assert.equal(revealOffset(stream, { key: "src/b.rs|r1", near: true }, 0, 100), start + ROW - 100);
  assert.equal(revealOffset(stream, { key: "src/b.rs|r1", near: true }, start + 500, 100), start);
  assert.equal(revealOffset(stream, { key: "src/b.rs|r1", near: true }, start - 40, 100), start - 40);
  assert.equal(revealOffset(stream, { key: "src/b.rs|r1" }, start - 40, 100), start, "a plain reveal still puts the item on top");
  assert.equal(revealOffset(stream, { key: "nowhere|r1", near: true }, 0, 100), null);
  // Through scrolled(), the one place a Scrolled comes from.
  const sync = scrolled({ ...next, offset: 0, total: stream.total }, stream);
  assert.equal(sync?._tag === "Scrolled" ? sync.offset : null, start + ROW - 100);
  assert.equal(sync?._tag === "Scrolled" ? sync.revealed : null, true);
});

test("ctrl+d and ctrl+u move half a screen of rows and stop at the ends", () => {
  const m = at(review(), "src/a.rs", 1);
  // 100 px is 5 rows, so half is 2 rows.
  assert.equal(where(press(m, "ctrl-d")), "src/a.rs:3");
  assert.equal(where(press(m, "pagedown")), "src/a.rs:3");
  assert.equal(where(press(at(review(), "src/a.rs", 3), "ctrl-u")), "src/a.rs:1");
  assert.equal(where(press(at(review(), "src/a.rs", 3), "pageup")), "src/a.rs:1");
  const tall = { ...m, height: 600 };
  assert.equal(where(press(tall, "ctrl-d")), "src/b.rs:8", "half a screen of 15 rows is more than what is left");
  assert.equal(where(press({ ...at(review(), "src/a.rs", 5), height: 600 }, "ctrl-u")), "notes.txt:1", "the first loaded, unfolded row of the stream");
});

test("[ and ] go to the first code row of the next and previous hunk, across files", () => {
  const m = at(review(), "src/a.rs", 1);
  assert.equal(where(press(m, "]")), "src/a.rs:7");
  assert.equal(where(press(m, "]", "]")), "src/b.rs:1");
  assert.equal(where(press(at(review(), "src/a.rs", 8), "[")), "src/a.rs:7");
  assert.equal(where(press(at(review(), "src/a.rs", 7), "[")), "src/a.rs:1");
  assert.equal(where(press(m, "[")), "notes.txt:7");
  assert.equal(where(press(at(review(), "src/b.rs", 8), "]")), "src/b.rs:8", "no hunk after the last one");
  assert.equal(where(press(review(), "]")), "src/a.rs:1", "with no cursor it is put on the top row");
});

test("side by side: the cursor has a half, j and k walk that half, h and l go to the other half of the pair", () => {
  const m = wide(review());
  const first = press(m, "j");
  assert.equal(where(first), "src/a.rs:1:new", "a context row starts on its new half");
  assert.equal(where(press(first, "j")), "src/a.rs:3:new", "the new half skips the removed row");
  assert.equal(where(press(first, "j", "j")), "src/a.rs:4:new");
  const old = at(m, "src/a.rs", 1, "old");
  assert.equal(where(press(old, "j")), "src/a.rs:2:old");
  assert.equal(where(press(old, "j", "j")), "src/a.rs:5:old", "the old half skips the added rows");
  // h and l: a changed pair, a context row, a half with no row facing it.
  assert.equal(where(press(at(m, "src/a.rs", 3, "new"), "h")), "src/a.rs:2:old");
  assert.equal(where(press(at(m, "src/a.rs", 2, "old"), "l")), "src/a.rs:3:new");
  assert.equal(where(press(at(m, "src/a.rs", 5, "new"), "h")), "src/a.rs:5:old");
  assert.equal(where(press(at(m, "src/a.rs", 5, "old"), "l")), "src/a.rs:5:new");
  assert.equal(where(press(at(m, "src/a.rs", 4, "new"), "h")), "src/a.rs:4:new", "row 4 has no removed row facing it");
  assert.equal(where(press(at(m, "src/a.rs", 3, "new"), "l")), "src/a.rs:3:new", "already on the new half");
  assert.equal(where(press(at(m, "src/a.rs", 3, "new"), "left")), "src/a.rs:2:old");
  assert.equal(where(press(at(m, "src/a.rs", 2, "old"), "right")), "src/a.rs:3:new");
  // [ and ] land on the first row of the hunk on the cursor's half.
  assert.equal(where(press(at(m, "src/a.rs", 1, "new"), "]")), "src/a.rs:7:new");
});

test("in unified h and l mean nothing", () => {
  const m = at(review(), "src/a.rs", 3);
  assert.equal(panelKey(m, "h"), null);
  assert.equal(panelKey(m, "l"), null);
});

test("v starts a range at the cursor and j and k grow it; the range stays in the file", () => {
  const m = press(at(review(), "src/a.rs", 3), "v");
  assert.deepEqual(m.selection, { path: "src/a.rs", side: "new", from: 3, to: 3, anchor: 3 });
  assert.equal(m.visual, true);
  const down = press(m, "j", "j");
  assert.deepEqual(down.selection, { path: "src/a.rs", side: "new", from: 3, to: 5, anchor: 3 });
  assert.equal(where(down), "src/a.rs:5");
  assert.deepEqual(press(down, "k", "k").selection, { path: "src/a.rs", side: "new", from: 3, to: 3, anchor: 3 });
  // Going above the anchor: the range is the rows that have a line on its side, here the one row.
  const above = press(m, "k");
  assert.equal(where(above), "src/a.rs:2");
  assert.deepEqual(above.selection, { path: "src/a.rs", side: "new", from: 3, to: 3, anchor: 3 });
  assert.deepEqual(press(above, "k", "k").selection, { path: "src/a.rs", side: "new", from: 1, to: 3, anchor: 3 });
  // Cursor and range stop at the last row of the file and do not go into the next one.
  const end = press(press(at(review(), "src/a.rs", 7), "v"), "j", "j", "j", "ctrl-d");
  assert.equal(where(end), "src/a.rs:8");
  assert.deepEqual(end.selection, { path: "src/a.rs", side: "new", from: 7, to: 8, anchor: 7 });
  const top = press(press(at(review(), "src/b.rs", 1), "v"), "k", "ctrl-u", "[");
  assert.equal(where(top), "src/b.rs:1");
  assert.equal(top.selection?.from, 1);
  // ] grows it to the next hunk of the same file, and no further.
  assert.equal(where(press(press(at(review(), "src/a.rs", 1), "v"), "]", "]")), "src/a.rs:7");
});

test("v again drops the range, and Esc does", () => {
  const m = press(at(review(), "src/a.rs", 3), "v", "j");
  const again = press(m, "v");
  assert.equal(again.selection, null);
  assert.equal(again.visual, false);
  assert.equal(where(again), "src/a.rs:4", "the cursor stays");
  assert.deepEqual(panelKey(m, "escape"), { _tag: "SelectionCleared" });
  const esc = press(m, "escape");
  assert.equal(esc.selection, null);
  assert.equal(esc.visual, false);
  // After that j only moves.
  assert.equal(press(esc, "j").selection, null);
  // v with no cursor shows it first and starts nothing.
  const none = press(review(), "v");
  assert.equal(where(none), "src/a.rs:1");
  assert.equal(none.selection, null);
});

test("side by side: a range keeps the half it began on, and h and l leave it alone", () => {
  const m = press(at(wide(review()), "src/a.rs", 2, "old"), "v", "j");
  assert.deepEqual(m.selection, { path: "src/a.rs", side: "old", from: 2, to: 5, anchor: 2 });
  assert.equal(where(m), "src/a.rs:5:old");
  assert.equal(where(press(m, "l")), "src/a.rs:5:old");
});

test("c comments on the selection when the cursor is in it, else on the cursor's row", () => {
  // No selection: the row, as a [+] on it.
  const one = press(at(review(), "src/a.rs", 3), "c");
  assert.deepEqual(one.editor?.anchor, { path: "src/a.rs", old_path: null, side: "new", line: 2, end_line: null });
  assert.deepEqual(one.selection, { path: "src/a.rs", side: "new", from: 3, to: 3, anchor: 3 });
  // A range with the cursor at its end: the range, and the same model as a click on that row's [+].
  const range = press(at(review(), "src/a.rs", 3), "v", "j", "c");
  assert.deepEqual(range.editor?.anchor, { path: "src/a.rs", old_path: null, side: "new", line: 2, end_line: 3 });
  assert.deepEqual(range, update(press(at(review(), "src/a.rs", 3), "v", "j"), { _tag: "PlusClicked", path: "src/a.rs", row: 4, side: null })[0]);
  assert.equal(range.visual, false);
  // The cursor outside the range: the row alone.
  const out = press({ ...press(at(review(), "src/a.rs", 3), "v", "j"), cursor: { path: "src/a.rs", row: 8, side: null } }, "c");
  assert.deepEqual(out.editor?.anchor, { path: "src/a.rs", old_path: null, side: "new", line: 9, end_line: null });
  // A removed row is on the old side, and in side by side the half is the cursor's.
  assert.equal(press(at(review(), "src/a.rs", 2), "c").editor?.anchor?.side, "old");
  const half = press(at(wide(review()), "src/a.rs", 5, "old"), "c");
  assert.deepEqual(half.editor?.anchor, { path: "src/a.rs", old_path: null, side: "old", line: 3, end_line: null });
  // With no cursor the key shows it and opens nothing.
  const none = press(review(), "c");
  assert.equal(none.editor, null);
  assert.equal(where(none), "src/a.rs:1");
});

test("c on a cursor whose file is folded does not open an editor", () => {
  const m = { ...at(review(), "src/b.rs", 3), folds: new Map([["src/b.rs", true]]) };
  assert.equal(press(m, "c").editor, null);
});

test("with a thread focused r, e, d and x are the card's buttons: reply, edit, delete asks, resolve and reopen", () => {
  const m = (focus: string | null): Model => ({ ...loadedModel(state, file), focus });
  assert.deepEqual(panelKey(m("u1"), "r"), { _tag: "ThreadAction", id: "u1", action: "reply" });
  assert.deepEqual(panelKey(m("u1"), "e"), { _tag: "ThreadAction", id: "u1", action: "edit" });
  assert.deepEqual(panelKey(m("u1"), "d"), { _tag: "ThreadAction", id: "u1", action: "delete" });
  assert.deepEqual(panelKey(m("u1"), "x"), { _tag: "ThreadAction", id: "u1", action: "resolve" });
  assert.deepEqual(panelKey(m("u2"), "x"), { _tag: "ThreadAction", id: "u2", action: "reopen" });
  // They do what the buttons do: a delete only asks, a reply opens the editor.
  assert.equal(press(m("u1"), "d").confirm, "u1");
  assert.equal(press(m("u1"), "r").editor?.kind, "reply");
  assert.equal(press(m("u1"), "e").editor?.initial, file.threads.find((t) => t.id === "u1")?.comments[0]?.body);
  assert.deepEqual(steps(m("u1"), { _tag: "ThreadAction", id: "u1", action: "resolve" }).cmds, update(m("u1"), panelKey(m("u1"), "x") as Msg)[1]);
  // An agent's comment has no edit or delete button, so no key; reply is there. With nothing focused no key does anything.
  assert.equal(panelKey(m("a1"), "e"), null);
  assert.equal(panelKey(m("a1"), "d"), null);
  assert.deepEqual(panelKey(m("a1"), "r"), { _tag: "ThreadAction", id: "a1", action: "reply" });
  for (const k of ["r", "e", "d", "x"]) {
    assert.equal(panelKey(m(null), k), null);
  }
});

test("the keys do nothing while the editor, a question, the picker or the filter box has the keyboard", () => {
  const keys = ["j", "k", "down", "up", "ctrl-d", "ctrl-u", "[", "]", "h", "l", "v", "c", "r", "e", "d", "x"];
  const base = { ...at(wide(review()), "src/a.rs", 3, "new"), focus: "u1" };
  const editor = { kind: "comment" as const, anchor: null, thread: null, comment: null, initial: "" };
  const holds: Model[] = [
    { ...base, editor },
    { ...base, confirm: "u1" },
    { ...base, confirmArchive: true },
    { ...base, picker: [{ address: "t7", agent: "claude", cwd: "/repo", status: "idle" }] },
    { ...base, filtering: true },
  ];
  for (const m of holds) {
    for (const k of keys) {
      assert.equal(panelKey(m, k), null, k);
    }
  }
  // Other chords are not these keys.
  assert.equal(panelKey(base, "alt-j"), null);
  assert.equal(panelKey(base, "ctrl-j"), null);
});

test("a flip of the layout keeps the cursor on its row: no half in unified, the row's own in side by side", () => {
  const unified = at(review(), "src/a.rs", 2);
  const split = update(unified, { _tag: "LayoutToggled" })[0];
  assert.equal(where(split), "src/a.rs:2:old");
  assert.equal(where(update(at(review(), "src/a.rs", 3), { _tag: "LayoutToggled" })[0]), "src/a.rs:3:new");
  assert.equal(where(update(at(review(), "src/a.rs", 5), { _tag: "LayoutToggled" })[0]), "src/a.rs:5:new");
  assert.equal(where(update(split, { _tag: "LayoutToggled" })[0]), "src/a.rs:2");
  // A resize across the width does the same, and the cursor is a spot of the new layout.
  const resized = update(unified, { _tag: "Resized", width: SPLIT_W, height: 100 })[0];
  assert.ok(spots(resized).some((s) => s.row === 2 && s.side === "old"));
  assert.equal(where(resized), "src/a.rs:2:old");
  // No cursor stays none.
  assert.equal(update(review(), { _tag: "LayoutToggled" })[0].cursor, null);
});

test("a press on a row moves the cursor there, in either layout, and a press on a hunk header does not", () => {
  const m = review();
  assert.equal(where(update(m, { _tag: "RowPressed", path: "src/a.rs", row: 4, side: null })[0]), "src/a.rs:4");
  assert.equal(where(update(wide(m), { _tag: "RowPressed", path: "src/a.rs", row: 2, side: "old" })[0]), "src/a.rs:2:old");
  assert.equal(where(update(at(m, "src/a.rs", 1), { _tag: "RowPressed", path: "src/a.rs", row: 0, side: null })[0]), "src/a.rs:1");
  // A press ends a range v began.
  assert.equal(update(press(at(m, "src/a.rs", 3), "v"), { _tag: "RowPressed", path: "src/a.rs", row: 5, side: null })[0].visual, false);
});

test("the cursor is drawn on its row and its half, and on no hunk header", () => {
  const item = (index: number, kind: RowItem["kind"] = "ctx"): RowItem => ({ kind, key: `src/a.rs|r${index}`, index, old: "", new: "", text: "", runs: [] });
  const m = at(review(), "src/a.rs", 3);
  assert.equal(cursorOn(m, item(3)), true);
  assert.equal(cursorOn(m, item(4)), false);
  assert.equal(cursorOn({ ...m, cursor: { path: "src/b.rs", row: 3, side: null } }, item(3)), false);
  assert.equal(cursorOn(at(m, "src/a.rs", 3, "old"), item(3), "old"), true);
  assert.equal(cursorOn(at(m, "src/a.rs", 3, "old"), item(3), "new"), false);
  assert.equal(cursorOn(at(m, "src/a.rs", 0), item(0, "hunk")), false);
  assert.equal(cursorOn(review(), item(3)), false);
});

test("a spec change drops the cursor and the range", () => {
  const m = press(at(review(), "src/a.rs", 3), "v");
  const next = update({ ...m, spec: { kind: "worktree" } }, { _tag: "SpecChosen", spec: { kind: "branch", base: "main" } })[0];
  assert.equal(next.cursor, null);
  assert.equal(next.visual, false);
});

test("the diff's keys do nothing in the agent-message view", () => {
  const m = { ...loadedModel(state, file), view: "message" as const };
  for (const key of ["j", "k", "v", "c", "n", "/", "r", "x", "escape"]) {
    assert.equal(panelKey(m, key), null, key);
  }
});

test("a fetch that changes the cursor's row drops the cursor, and one that keeps it keeps the cursor", () => {
  const m = at(review(), "src/a.rs", 3);
  const view = m.files.get("src/a.rs");
  assert.ok(view !== undefined);
  const same = update(m, { _tag: "FilesLoaded", key: m.key, files: [{ ...view, rows: [...ROWS] }] })[0];
  assert.equal(where(same), "src/a.rs:3");
  // A line above was added: row 3 is now another line.
  const moved = update(m, { _tag: "FilesLoaded", key: m.key, files: [{ ...view, rows: [ROWS[0] as Row, row("add", 1), ...ROWS.slice(1)] }] })[0];
  assert.equal(where(moved), "none");
});
