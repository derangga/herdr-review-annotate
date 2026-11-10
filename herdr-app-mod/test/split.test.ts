// The side-by-side layout: the pairing, where cards sit, the width rule and the toggle, and the half each pointer message names.
import assert from "node:assert/strict";
import { test } from "node:test";
import { codeWidth, heightOf, indexOfKey, isSplit, layoutButton, layoutCache, maxShift, partner, pick, plusTarget, ROW, rowState, scrolled, span, SPLIT_W, streamLayout, wheelShift } from "../src/rows.ts";
import type { RowItem, SplitItem, StreamItem } from "../src/rows.ts";
import type { FileView, Model, Msg } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
const NOW = Date.parse("2026-11-09T10:00:30Z");
// Rows of the fixture: 0 hunk, 1 ctx 1/1, 2 del old 2, 3 add new 2, 4 add new 3, 5 ctx 3/4, 6 ctx 4/5.
// Threads: u3 old row 2, u1 new row 3, u2 new row 4, u4 new row 6, u5 on no row.
const wide = (m: Model): Model => ({ ...m, width: SPLIT_W });
const split = (): Model => wide(loadedModel(state, file));
const own = (m: Model): StreamItem[] => streamLayout(m, NOW).items.filter((i) => i.key.startsWith("src/a.rs|"));
const halves = (i: StreamItem): string => (i.kind === "split" ? `${i.left?.index ?? "-"}|${i.right?.index ?? "-"}` : i.kind);
const msg = (path: string, row: number, side: "old" | "new" | null) => ({ path, row, side });

test("the layout follows the width, and a forced layout beats it", () => {
  const m = loadedModel(state, file);
  assert.equal(isSplit({ ...m, width: SPLIT_W - 1 }), false);
  assert.equal(isSplit({ ...m, width: SPLIT_W }), true);
  assert.equal(isSplit({ ...m, width: SPLIT_W, split: false }), false);
  assert.equal(isSplit({ ...m, width: 300, split: true }), true);
});

test("the header button names the other layout, and pressing it forces that layout and clears the hover", () => {
  const m = steps(split(), { _tag: "Hovered", ...msg("src/a.rs", 3, "new"), on: true }).model;
  assert.deepEqual(layoutButton(m), { id: "layout-toggle", label: "Unified", icon: "columns", on: true, msg: { _tag: "LayoutToggled" } });
  const once = steps(m, { _tag: "LayoutToggled" }).model;
  assert.equal(once.split, false);
  assert.equal(once.hover, null);
  assert.equal(layoutButton(once).label, "Side by side");
  assert.equal(steps(once, { _tag: "LayoutToggled" }).model.split, true);
  // Narrow, the first press goes the other way.
  assert.equal(steps(loadedModel(state, file), { _tag: "LayoutToggled" }).model.split, true);
});

test("the selection survives a toggle", () => {
  const picked = steps(split(), { _tag: "RowPressed", ...msg("src/a.rs", 3, "new") }, { _tag: "Hovered", ...msg("src/a.rs", 4, "new"), on: true }, { _tag: "Released" }).model;
  assert.deepEqual(steps(picked, { _tag: "LayoutToggled" }).model.selection, picked.selection);
});

test("unified is the diff's rows, and a split stream pairs removed with added, line by line", () => {
  const unified = own({ ...split(), split: false }).filter((i) => i.key.includes("|r"));
  assert.deepEqual(unified.map((i) => i.kind), ["hunk", "ctx", "del", "add", "add", "ctx", "ctx"]);
  // Hunk full width, ctx on both halves, del 2 against add 3, add 4 against an empty half.
  assert.deepEqual(own(split()).filter((i) => i.kind === "hunk" || i.kind === "split").map(halves), ["hunk", "1|1", "2|3", "-|4", "5|5", "6|6"]);
  const items = own(split()).filter((i): i is SplitItem => i.kind === "split");
  assert.deepEqual(items.map((i) => i.key), ["src/a.rs|p1", "src/a.rs|p2", "src/a.rs|p4", "src/a.rs|p5", "src/a.rs|p6"]);
  assert.equal(items[0]?.left, items[0]?.right, "a context row is one item on both halves");
  assert.equal(items[2]?.left, null);
  assert.ok(items.every((i) => heightOf(i, 900) === ROW));
});

test("pairing: the longer run's extra rows face an empty half, and a run of added rows alone has no left", () => {
  const row = (kind: "ctx" | "add" | "del", n: number) => ({ kind, old: kind === "add" ? null : n, new: kind === "del" ? null : n, text: `t${n}`, tokens: [], marks: [] });
  const rows = [row("del", 1), row("del", 2), row("del", 3), row("add", 1), row("ctx", 9), row("add", 5), row("add", 6), row("del", 7)];
  const m = wide(loadedModel(state, { ...file, rows, threads: [] }));
  // dels 0-2 against add 3; ctx 4; adds 5-6 (no dels before them); del 7 alone.
  assert.deepEqual(own(m).filter((i) => i.kind === "split").map(halves), ["0|3", "1|-", "2|-", "4|4", "-|5", "-|6", "7|-"]);
});

test("cards sit under the paired item that holds their row, the left row's threads before the right row's", () => {
  const keys = own(split()).map((i) => i.key);
  const at = (k: string) => keys.indexOf(k);
  // u3 is on the old row 2 and u1 on the new row 3, the pair p2; u2 on row 4 sits under the next pair.
  assert.deepEqual(keys.slice(at("src/a.rs|p2"), at("src/a.rs|p5")).filter((k) => !k.includes("|c:")), ["src/a.rs|p2", "src/a.rs|h:u3", "src/a.rs|h:u1", "src/a.rs|p4", "src/a.rs|h:u2"]);
  assert.ok(at("src/a.rs|h:u4") > at("src/a.rs|p6"));
  // Every thread's card is once in the stream, as in unified.
  const heads = (m: Model) => own(m).filter((i) => i.kind === "head").map((i) => i.key).sort();
  assert.deepEqual(heads(split()), heads({ ...split(), split: false }));
});

test("the cache is rebuilt when the layout flips at the same width", () => {
  const cached = layoutCache();
  const m = split();
  const a = cached(m, NOW);
  assert.equal(cached(m, NOW), a);
  const b = cached({ ...m, split: false }, NOW);
  assert.notEqual(b, a);
  assert.ok(b.items.every((i) => i.kind !== "split"));
});

test("side by side fits the pane, and its halves' code shifts sideways together, held to the longest line", () => {
  const item = (text: string): RowItem => ({ kind: "add", key: "f|r1", index: 1, old: "", new: "1", text, runs: [] });
  const long = "x".repeat(300);
  assert.equal(codeWidth([{ kind: "split", key: "f|p1", left: item("a"), right: item(long) }]), 300 * 7);
  // A long line does not widen a side-by-side stream; unified still scrolls as a whole.
  const m = split();
  assert.equal(streamLayout(m, NOW).wide, 0);
  assert.ok(streamLayout({ ...m, split: false }, NOW).wide >= 300);
  assert.equal(maxShift(2100, 1001), 2100 - (500 - 88));
  assert.equal(maxShift(100, 1001), 0);
  // Sideways moves the code, vertical does not; a reported line is four characters.
  assert.equal(wheelShift({ delta: { x: -30, y: 2 } }), 30);
  assert.equal(wheelShift({ delta: { x: 1, y: 10 } }), null);
  assert.equal(wheelShift({ delta: { x: -3, y: 0 }, delta_lines: { x: -1, y: 0 } }), 28);
  const right = steps(m, { _tag: "Shifted", dx: 500, max: 200 }).model;
  assert.equal(right.shift, 200);
  assert.equal(steps(right, { _tag: "Shifted", dx: -900, max: 200 }).model.shift, 0);
  // Past the limit of the lines now on screen, a scroll right holds still and a scroll left moves smoothly from where it is.
  assert.equal(steps(right, { _tag: "Shifted", dx: 30, max: 50 }).model.shift, 200);
  assert.equal(steps(right, { _tag: "Shifted", dx: -30, max: 50 }).model.shift, 170);
  // A flip starts the code at its left edge again.
  assert.equal(steps(right, { _tag: "LayoutToggled" }).model.shift, 0);
});

test("a half decides the side: pick, span and plusTarget take it, a row with no line there gives nothing", () => {
  // Row 1 is a context row: both sides.
  assert.deepEqual(pick(file, 1, "old"), { path: "src/a.rs", side: "old", from: 1, to: 1, anchor: 1 });
  assert.deepEqual(pick(file, 1, "new"), pick(file, 1));
  assert.equal(pick(file, 2, "new"), null); // a removed row has no new line
  assert.equal(pick(file, 3, "old"), null);
  assert.equal(pick(file, 0, "new"), null);
  assert.deepEqual(span(file, 1, 6, "old"), { path: "src/a.rs", side: "old", from: 1, to: 6, anchor: 1 }); // the rows with an old line: 1, 2, 5, 6
  assert.equal(span(file, 3, 6, "old"), null);
  assert.deepEqual(span(file, 1, 4, null), span(file, 1, 4));
  const range = pick(file, 1, "old")!;
  assert.equal(plusTarget(file, range, 1, "old"), range);
  assert.deepEqual(plusTarget(file, range, 1, "new"), pick(file, 1, "new")); // the other half is the row alone
  assert.equal(plusTarget(file, range, 1), range); // unified: no half, the selection as before
});

test("the pointer messages carry the half into the hover, the drag and the selection", () => {
  const m = steps(split(), { _tag: "RowPressed", ...msg("src/a.rs", 1, "old") }).model;
  assert.deepEqual(m.hover, { path: "src/a.rs", row: 1, side: "old" });
  assert.deepEqual(m.drag, { path: "src/a.rs", row: 1, head: 1, moved: false, side: "old" });
  // The drag takes the side it began on, whichever half the pointer is over.
  const dragged = steps(m, { _tag: "Hovered", ...msg("src/a.rs", 3, "new"), on: true }).model;
  assert.deepEqual(dragged.selection, { path: "src/a.rs", side: "old", from: 1, to: 2, anchor: 1 });
  // The same row's other half is a new hover, not the same one.
  const other = steps(m, { _tag: "Hovered", ...msg("src/a.rs", 1, "new"), on: true }).model;
  assert.deepEqual(other.hover, { path: "src/a.rs", row: 1, side: "new" });
  assert.equal(steps(other, { _tag: "Hovered", ...msg("src/a.rs", 1, "new"), on: true }).model, other);
  assert.equal(steps(other, { _tag: "Hovered", ...msg("src/a.rs", 1, "old"), on: false }).model, other, "a leave from the half the model does not have is stale");
  assert.equal(steps(other, { _tag: "Hovered", ...msg("src/a.rs", 1, "new"), on: false }).model.hover, null);
});

test("a [+] on a half comments on that side of a context row", () => {
  const left = steps(split(), { _tag: "PlusClicked", ...msg("src/a.rs", 1, "old") }).model;
  assert.deepEqual(left.editor?.anchor, { path: "src/a.rs", old_path: null, side: "old", line: 1, end_line: null });
  const right = steps(split(), { _tag: "PlusClicked", ...msg("src/a.rs", 1, "new") }).model;
  assert.deepEqual(right.editor?.anchor, { path: "src/a.rs", old_path: null, side: "new", line: 1, end_line: null });
  assert.equal(steps(split(), { _tag: "PlusClicked", ...msg("src/a.rs", 3, "old") }).model.editor, null);
});

test("a selection is tinted on the half of its side, and the [+] is on the half under the pointer", () => {
  const ctx = (own(split()).find((i) => i.kind === "split") as SplitItem).left as RowItem;
  const old = { ...split(), selection: pick(file, 1, "old"), hover: { path: "src/a.rs", row: 1, side: "new" as const } };
  assert.deepEqual([rowState(old, ctx, "old"), rowState(old, ctx, "new")], [{ selected: true, hovered: false }, { selected: false, hovered: true }]);
  const range = { ...old, selection: span(file, 1, 4, "new"), hover: null };
  assert.equal(rowState(range, ctx, "old").selected, false);
  assert.equal(rowState(range, ctx, "new").selected, true);
  assert.equal(rowState(range, ctx).selected, true, "without a half, as in unified");
});

// Rows: 0 hunk, 1 ctx, 2-4 del, 5-7 add, 8 ctx, 9-10 del, 11 add, 12 add, 13 add.
const rowOf = (kind: "hunk" | "ctx" | "add" | "del", n: number) => ({ kind, old: kind === "add" || kind === "hunk" ? null : n, new: kind === "del" || kind === "hunk" ? null : n, text: `t${n}`, tokens: [], marks: [] });
const runs: FileView = { ...file, rows: [rowOf("hunk", 0), rowOf("ctx", 1), ...[2, 3, 4].map((n) => rowOf("del", n)), ...[5, 6, 7].map((n) => rowOf("add", n)), rowOf("ctx", 8), rowOf("del", 9), rowOf("del", 10), ...[11, 12, 13].map((n) => rowOf("add", n))], threads: [] };
const runsModel = (): Model => wide(loadedModel(state, runs));

test("partner is the row that faces a row in the side-by-side pairing", () => {
  assert.deepEqual(runs.rows.map((_, i) => partner(runs, i)), [null, 1, 5, 6, 7, 2, 3, 4, 8, 11, 12, 9, 10, null]);
  assert.equal(partner(file, 3), 2);
  assert.equal(partner(file, 4), null);
  // It is the pairing the stream draws.
  for (const i of own(runsModel()).filter((x): x is SplitItem => x.kind === "split")) {
    if (i.left !== null && i.right !== null && i.left !== i.right) assert.deepEqual([partner(runs, i.left.index), partner(runs, i.right.index)], [i.right.index, i.left.index]);
    else if (i.left !== i.right) assert.equal(partner(runs, ((i.left ?? i.right) as RowItem).index), null);
  }
});

test("a drag takes the pair the pointer is on, whichever half it is over", () => {
  const press = (row: number, side: "old" | "new"): Msg => ({ _tag: "RowPressed", path: "src/a.rs", row, side });
  const over = (row: number, side: "old" | "new"): Msg => ({ _tag: "Hovered", path: "src/a.rs", row, side, on: true });
  const sel = (...msgs: Msg[]) => steps(runsModel(), ...msgs).model.selection;
  // Pressed on the old half of pair 0, the pointer moves to the new half of pair 0: it has not left the pair, so no range.
  assert.equal(sel(press(2, "old"), over(5, "new")), null);
  assert.deepEqual(sel(press(2, "old"), over(6, "new")), { path: "src/a.rs", side: "old", from: 2, to: 3, anchor: 2 });
  // On to the new half of pair 2, the range reaches the last removed row.
  assert.deepEqual(sel(press(2, "old"), over(7, "new")), { path: "src/a.rs", side: "old", from: 2, to: 4, anchor: 2 });
  // The mirror: pressed on the new half of pair 0, moved to the old half of pair 2, reaches the last added row.
  assert.deepEqual(sel(press(5, "new"), over(4, "old")), { path: "src/a.rs", side: "new", from: 5, to: 7, anchor: 5 });
  assert.equal(sel(press(5, "new"), over(2, "old")), null);
  // The same half is the row itself.
  assert.deepEqual(sel(press(2, "old"), over(3, "old")), { path: "src/a.rs", side: "old", from: 2, to: 3, anchor: 2 });
});

test("a flip of the layout scrolls back to the item on top and drops the hover", () => {
  const unified = { ...loadedModel(state, file), top: "src/a.rs|r3", hover: { path: "src/a.rs", row: 3, side: null } };
  const toSplit = steps(unified, { _tag: "LayoutToggled" }).model;
  assert.equal(toSplit.hover, null);
  assert.deepEqual(toSplit.reveal, { key: "src/a.rs|r3" });
  // Row 3 is the new half of the pair that begins at row 2.
  const items = streamLayout(toSplit, NOW).items;
  assert.equal(items[indexOfKey(items, "src/a.rs|r3")]?.key, "src/a.rs|p2");
  const msg = scrolled(toSplit, streamLayout(toSplit, NOW));
  assert.equal(msg?._tag === "Scrolled" && msg.revealed, true);
  // And back: the key of the pair is the item of its first row.
  // A scroll already on its way (a jump, a file chosen) is kept: the top it would leave is not the place to go back to.
  assert.deepEqual(steps({ ...toSplit, top: "src/a.rs|p4" }, { _tag: "LayoutToggled" }).model.reveal, { key: "src/a.rs|r3" });
  const back = steps({ ...toSplit, top: "src/a.rs|p4", reveal: null }, { _tag: "LayoutToggled" }).model;
  const rows = streamLayout(back, NOW).items;
  assert.equal(rows[indexOfKey(rows, back.reveal?.key ?? "")]?.key, "src/a.rs|r4");
  // A key that is in the stream is itself, and one that is not a row has no stand-in.
  assert.equal(indexOfKey(rows, "src/a.rs|r1"), rows.findIndex((i) => i.key === "src/a.rs|r1"));
  assert.equal(indexOfKey(rows, "src/a.rs|r99"), -1);
  assert.equal(indexOfKey(rows, "src/a.rs|h:nope"), -1);
});

test("a resize that crosses the width threshold flips the layout the same way, and one that does not leaves things alone", () => {
  const m = { ...loadedModel(state, file), top: "src/a.rs|r3", hover: { path: "src/a.rs", row: 3, side: null } };
  const crossed = steps(m, { _tag: "Resized", width: SPLIT_W, height: 600 }).model;
  assert.deepEqual([crossed.reveal, crossed.hover], [{ key: "src/a.rs|r3" }, null]);
  const same = steps(m, { _tag: "Resized", width: SPLIT_W - 100, height: 500 }).model;
  assert.deepEqual([same.reveal, same.hover], [null, m.hover]);
  // A forced layout does not flip with the width.
  assert.equal(steps({ ...m, split: false }, { _tag: "Resized", width: SPLIT_W, height: 600 }).model.reveal, null);
});
