// The mod scrolls the stream itself: the pixel offset, the items that overlap the screen, the scrollbar's thumb, and what
// the sidebar and the fetches learn from the offset. The E: lines of design/graphs/gui-stream.md are the tests here.
import assert from "node:assert/strict";
import { test } from "node:test";
import { clampOffset, itemAt, maxOffset, offsetOfKey, offsetOfThumb, ROW, scrolled, streamLayout, thumbOf, THUMB_MIN, visibleRange, wheelDelta } from "../src/rows.ts";
import type { Stream } from "../src/rows.ts";
import type { Model, Msg } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const NOW = Date.parse("2026-11-09T10:00:30Z");

// A stream of plain heights, to check the arithmetic on numbers that can be added up by eye.
function plain(sizes: number[]): Stream {
  const starts = [0];
  for (const s of sizes) starts.push((starts[starts.length - 1] as number) + s);
  const items = sizes.map((_, i) => ({ kind: "note" as const, key: `f|n${i}`, text: "" }));
  return { items, sizes, starts, total: starts[starts.length - 1] as number, wide: 300 };
}

// The panel as it runs: after every message the model learns the offset, the height and the files on screen from the stream.
function settle(m: Model): Model {
  for (let i = 0; i < 4; i++) {
    const msg = scrolled(m, streamLayout(m, NOW));
    if (msg === null) return m;
    m = steps(m, msg).model;
  }
  throw new Error("the scroll state did not settle");
}
const send = (m: Model, ...msgs: Msg[]): Model => settle(steps(m, ...msgs).model);

// The fixtures' review with a screen 100 px tall.
const shown = (height = 100): Model => settle({ ...loadedModel(fixtureState(), fixtureFile(), "t7", null), height });

test("E: item positions are the prefix sum of the heights, and the item at a pixel is the last one that begins at or above it", () => {
  const s = plain([20, 20, 50, 20]);
  assert.deepEqual(s.starts, [0, 20, 40, 90, 110]);
  assert.deepEqual([0, 19, 20, 39, 40, 89, 90, 109, 110, 500].map((y) => itemAt(s.starts, y)), [0, 0, 1, 1, 2, 2, 3, 3, 3, 3]);
  assert.equal(itemAt([0], 10), 0); // an empty stream
});

test("E: the items drawn are the ones that overlap the screen, the first shifted up by the part above the top", () => {
  const s = plain([20, 20, 50, 20, 20, 20]);
  assert.deepEqual(visibleRange(s, 0, 40), { first: 0, end: 2, shift: 0 });
  assert.deepEqual(visibleRange(s, 30, 40), { first: 1, end: 3, shift: 10 }); // item 1 from 20, item 2 from 40 to 90
  assert.deepEqual(visibleRange(s, 55, 30), { first: 2, end: 3, shift: 15 }); // one tall item holds the whole screen
  assert.deepEqual(visibleRange(s, 90, 40), { first: 3, end: 5, shift: 0 }); // an item that begins at the bottom edge is not drawn
  assert.deepEqual(visibleRange(s, 91, 40), { first: 3, end: 6, shift: 1 });
  assert.deepEqual(visibleRange(plain([]), 0, 40), { first: 0, end: 0, shift: 0 });
  // The offset is clamped, so a frame drawn before the model caught up with a smaller stream still draws the end.
  assert.deepEqual(visibleRange(s, 5000, 40), { first: 4, end: 6, shift: 0 });
});

test("E: the offset stays between the top and the end of the stream, and a stream shorter than the screen does not scroll", () => {
  assert.equal(maxOffset(1000, 300), 700);
  assert.equal(maxOffset(200, 300), 0);
  assert.deepEqual([-5, 0, 250, 700, 9000].map((o) => clampOffset(o, 1000, 300)), [0, 0, 250, 700, 700]);
  assert.equal(clampOffset(50, 200, 300), 0);
});

test("E: a wheel tick scrolls by its pixels, a device that reports lines by a row a line, and a sideways one is left to the code box", () => {
  assert.equal(wheelDelta({ delta: { x: 0, y: -30 } }), 30); // the wheel moving the content up scrolls down
  assert.equal(wheelDelta({ delta: { x: 0, y: 12 } }), -12);
  assert.equal(wheelDelta({ delta: { x: 0, y: -999 }, delta_lines: { x: 0, y: -3 } }), 3 * ROW);
  assert.equal(wheelDelta({ delta: { x: 40, y: -3 } }), null);
  assert.equal(wheelDelta({ delta: { x: 5, y: -5 } }) === null, false); // a tie is a vertical scroll
  const m = shown();
  assert.ok(m.total > 100);
  const down = send(m, { _tag: "Wheeled", dy: 30 });
  assert.equal(down.offset, 30);
  assert.equal(send(down, { _tag: "Wheeled", dy: -500 }).offset, 0, "the top");
  assert.equal(send(m, { _tag: "Wheeled", dy: 1e9 }).offset, m.total - 100, "the end");
  // At either end nothing changes, so there is no frame.
  assert.equal(steps(m, { _tag: "Wheeled", dy: -10 }).model, m);
  const end = send(m, { _tag: "Wheeled", dy: 1e9 });
  assert.equal(steps(end, { _tag: "Wheeled", dy: 10 }).model, end);
});

test("E: the thumb is sized and placed from the offset, the screen and the stream, and a stream that fits has none", () => {
  assert.equal(thumbOf(300, 300, 0), null);
  assert.equal(thumbOf(100, 300, 0), null);
  assert.deepEqual(thumbOf(1000, 300, 0), { top: 0, size: 90 });
  assert.deepEqual(thumbOf(1000, 300, 700), { top: 210, size: 90 }); // the end of the stream is the end of the track
  assert.deepEqual(thumbOf(1000, 300, 350), { top: 105, size: 90 });
  assert.equal(thumbOf(1000, 300, 9999)?.top, 210, "an offset past the end is clamped");
  assert.equal(thumbOf(300000, 300, 0)?.size, THUMB_MIN, "a long stream keeps a thumb that can be grabbed");
  // The thumb's place and the offset are inverses.
  for (const o of [0, 100, 350, 700]) assert.equal(Math.round(offsetOfThumb(1000, 300, thumbOf(1000, 300, o)?.top ?? 0)), o);
  assert.equal(offsetOfThumb(300, 300, 50), 0);
});

test("E: dragging the thumb moves the stream by the pointer's distance in the track, and the release lets go", () => {
  const m = shown();
  const room = 100 - (thumbOf(m.total, 100, 0)?.size ?? 0);
  const held = send(m, { _tag: "ThumbGrabbed", y: 40 });
  assert.deepEqual(held.grip, { y: 40, offset: 0 });
  assert.equal(held.offset, 0, "the press alone moves nothing");
  const half = send(held, { _tag: "ThumbMoved", y: 40 + room / 2 });
  assert.equal(Math.round(half.offset), Math.round((m.total - 100) / 2));
  assert.equal(send(held, { _tag: "ThumbMoved", y: 40 + room * 3 }).offset, m.total - 100, "past the end of the track");
  assert.equal(send(held, { _tag: "ThumbMoved", y: -500 }).offset, 0);
  // A move is measured from the press, not from the last move, so the thumb does not drift.
  assert.equal(send(half, { _tag: "ThumbMoved", y: 40 + room / 2 }).offset, half.offset);
  // With no grip a move is nothing, and the release lets go, also of a drag at the same time.
  assert.equal(steps(m, { _tag: "ThumbMoved", y: 90 }).model, m);
  assert.equal(steps(half, { _tag: "Released" }).model.grip, null);
  assert.equal(steps(steps(half, { _tag: "Released" }).model, { _tag: "ThumbMoved", y: 10 }).model.offset, half.offset);
});

test("E: a click on the track centres the thumb there", () => {
  const m = shown();
  const thumb = thumbOf(m.total, 100, 0);
  assert.ok(thumb !== null);
  const mid = send(m, { _tag: "TrackClicked", y: 50 });
  assert.equal(Math.round(mid.offset), Math.round((m.total - 100) * ((50 - thumb.size / 2) / (100 - thumb.size))));
  assert.equal(send(m, { _tag: "TrackClicked", y: 100 }).offset, m.total - 100);
  assert.equal(send(mid, { _tag: "TrackClicked", y: 0 }).offset, 0);
  assert.equal(steps({ ...m, total: 50 }, { _tag: "TrackClicked", y: 20 }).model.offset, 0, "a stream that fits has no track");
});

test("E: after any change the model learns the offset, the first item on screen and the files on screen from the stream", () => {
  const m = shown();
  const s = streamLayout(m, NOW);
  assert.equal(m.top, s.items[0]?.key);
  assert.equal(m.total, s.total);
  assert.deepEqual(m.visible, ["img.png"]); // the threads not in the diff come first, and the first file's header is below them
  // A model that already knows it all is not told again.
  assert.equal(scrolled(m, s), null);
  // Scrolled to a file's header, that file is on screen and is the sidebar's.
  const at = send(m, { _tag: "Wheeled", dy: s.starts[s.items.findIndex((i) => i.key === "notes.txt|file")] as number });
  assert.equal(at.top, "notes.txt|file");
  assert.equal(at.selected, "notes.txt");
  assert.deepEqual(at.visible.slice(0, 1), ["notes.txt"]);
  // The topmost file on screen moves with the offset, which is what a timer used to report.
  const further = send(at, { _tag: "Wheeled", dy: (s.starts[s.items.findIndex((i) => i.key === "src/a.rs|file")] as number) - at.offset });
  assert.equal(further.selected, "src/a.rs");
});

test("E: a request to scroll to an item moves the offset to it, and the end of the stream stops it short", () => {
  const m = shown();
  const s = streamLayout(m, NOW);
  const at = (key: string) => s.starts[s.items.findIndex((i) => i.key === key)] as number;
  const click = send(m, { _tag: "FileChosen", path: "src/a.rs" });
  assert.equal(click.offset, at("src/a.rs|file"));
  assert.equal(click.reveal, null);
  assert.equal(click.selected, "src/a.rs");
  assert.equal(offsetOfKey(s, "src/a.rs|file", 100), at("src/a.rs|file"));
  assert.equal(offsetOfKey(s, "nowhere|file", 100), null);
  // The last header cannot reach the top; the file stays the sidebar's while it is on screen.
  const last = send(m, { _tag: "FileChosen", path: "src/b.rs" });
  assert.equal(last.offset, m.total - 100);
  assert.ok(last.offset < at("src/b.rs|file"));
  assert.equal(last.selected, "src/b.rs");
  // A card is scrolled to as well (next and previous comment), and the key of an item not in the stream waits.
  const card = send(m, { _tag: "ThreadAction", id: "u3", action: "focus" }, { _tag: "ThreadJumped", forward: true });
  assert.ok(card.reveal === null && card.offset > 0);
  const waiting = settle({ ...m, reveal: { key: "later.rs|h:u9" } });
  assert.deepEqual(waiting.reveal, { key: "later.rs|h:u9" });
  assert.equal(waiting.offset, 0);
});

test("E: a file that arrives above the top of the screen puts the item that was at the top back at the top", () => {
  const m = send(shown(), { _tag: "FileChosen", path: "src/a.rs" });
  assert.equal(m.top, "src/a.rs|file");
  // Its placeholder grows when the diff replaces it: the stream above the top is another height.
  const taller = { ...m, files: new Map([...m.files].filter(([p]) => p !== "notes.txt")) };
  const grown = settle({ ...taller, reveal: { key: m.top ?? "" } });
  assert.equal(grown.top, "src/a.rs|file");
  assert.equal(grown.offset, (streamLayout(grown, NOW).starts[streamLayout(grown, NOW).items.findIndex((i) => i.key === "src/a.rs|file")] as number));
});

test("the files on screen start the fetch of the files below them", () => {
  const m = shown();
  const idle = { ...m, inFlight: false, dirty: new Set(["src/b.rs"]) };
  const out = steps(idle, scrolled(idle, streamLayout(idle, NOW)) ?? { _tag: "Scrolled", offset: 0, total: m.total, top: "x|file", visible: ["src/b.rs"], fresh: [], revealed: false });
  assert.ok(out.cmds.some((c) => c._tag === "LoadFiles"));
});

test("a stream shorter than the screen sits at the top whatever the offset was", () => {
  const m = send(shown(), { _tag: "Wheeled", dy: 150 });
  assert.ok(m.offset > 0);
  const tall = settle({ ...m, height: 100000 });
  assert.equal(tall.offset, 0);
  assert.equal(thumbOf(tall.total, tall.height, tall.offset), null);
});
