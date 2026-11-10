// The stream of every file's diff: its items and heights, the order files are fetched in, and which file the
// sidebar shows. The E: lines of design/graphs/gui-stream.md are the tests here.
import assert from "node:assert/strict";
import { test } from "node:test";
import { AHEAD, BATCH, cardWidth, estimateRows, heightOf, indexOfKey, isFolded, layoutCache, pathsOf, ROW, streamLayout, wanted } from "../src/rows.ts";
import type { BodyItem } from "../src/rows.ts";
import { initModel, update } from "../src/update.ts";
import type { FileSummary, FileView, Model, Msg, State } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
// A Scrolled as the panel reports it, with no scroll under way.
const scroll = (top: string | null, visible: string[], revealed = false): Msg => ({ _tag: "Scrolled", offset: 0, total: 0, top, visible, fresh: [], revealed });
const NOW = Date.parse("2026-11-09T10:00:30Z");

// The fixtures' review with src/a.rs and notes.txt drawn: notes.txt has three rows, the others are blank.
function drawn(): Model {
  const m = loadedModel(state, file);
  const notes: FileView = { ...file, path: "notes.txt", threads: [], rows: file.rows.slice(0, 3) };
  return { ...m, files: new Map(m.files).set("notes.txt", notes) };
}
const keys = (m: Model) => streamLayout(m, NOW).items.map((i) => i.key);

test("E: the stream is the threads not in the diff, then each file as a header and its rows, in the diff's order", () => {
  const { items, sizes } = streamLayout(drawn(), NOW);
  assert.equal(sizes.length, items.length);
  const all = items.map((i) => i.key);
  assert.deepEqual(all.slice(0, 4), ["|l:orphans", "|h:u6", "|c:u6", "img.png|file"]);
  // The file boundary: the last row of notes.txt, then the header of src/a.rs, then its file comment card.
  const at = all.indexOf("notes.txt|r2");
  assert.deepEqual(all.slice(at, at + 3), ["notes.txt|r2", "src/a.rs|file", "src/a.rs|h:u5"]);
  assert.deepEqual(sizes.slice(at, at + 2), [ROW, 30]);
  assert.equal(all.at(-1), "src/b.rs|file");
  assert.equal(new Set(all).size, all.length, "every key is one item");
});

test("the stream counts the pixel each item begins at, and its height, from the item sizes", () => {
  const { sizes, starts, total, wide } = streamLayout(drawn(), NOW);
  assert.equal(starts.length, sizes.length + 1);
  assert.equal(starts[0], 0);
  assert.ok(starts.every((x, i) => i === 0 || x === (starts[i - 1] as number) + (sizes[i - 1] as number)));
  assert.equal(total, sizes.reduce((a, b) => a + b, 0));
  assert.equal(starts[sizes.length], total);
  assert.ok(wide >= 300);
});

test("a file marked viewed is its header alone, a folded one says what it hides, and a header click opens it", () => {
  const m = drawn();
  assert.equal(isFolded(m, state.files[0] as FileSummary), true); // img.png is viewed
  const head = streamLayout(m, NOW).items.find((i) => i.key === "img.png|file");
  assert.ok(head && head.kind === "file");
  assert.deepEqual([head.folded, head.viewed, head.hides], [true, true, ""]);
  const folded = steps(m, { _tag: "FoldToggled", path: "src/a.rs" }).model;
  assert.deepEqual(keys(folded).filter((k) => k.startsWith("src/a.rs|")), ["src/a.rs|file"]);
  const a = streamLayout(folded, NOW).items.find((i) => i.key === "src/a.rs|file");
  assert.ok(a && a.kind === "file");
  assert.equal(a.hides, "5 threads");
  // The fold is the model's alone and is not saved: no command goes out.
  assert.deepEqual(steps(m, { _tag: "FoldToggled", path: "src/a.rs" }).cmds, []);
  const open = steps(folded, { _tag: "FoldToggled", path: "src/a.rs" }).model;
  assert.ok(keys(open).includes("src/a.rs|r0"));
  // A viewed file opens by a click too, and a click on it again folds it.
  const viewed = steps(m, { _tag: "FoldToggled", path: "img.png" }).model;
  assert.equal(isFolded(viewed, state.files[0] as FileSummary), false);
  assert.equal(steps(viewed, { _tag: "FoldToggled", path: "img.png" }).model.folds.get("img.png"), true);
});

test("marking a file viewed folds it and scrolls to its header, unmarking opens it, and a reply that agrees drops the override", () => {
  const m = drawn();
  const marked = steps(m, { _tag: "ViewedClicked", path: "src/a.rs", viewed: true });
  assert.equal(marked.model.folds.get("src/a.rs"), true);
  assert.deepEqual(marked.model.reveal, { key: "src/a.rs|file" });
  assert.deepEqual(keys(marked.model).filter((k) => k.startsWith("src/a.rs|")), ["src/a.rs|file"]);
  const unmarked = steps(m, { _tag: "ViewedClicked", path: "img.png", viewed: false }).model;
  assert.equal(isFolded(unmarked, state.files[0] as FileSummary), false);
  // The reload says a.rs is viewed: the fold is the file's own now.
  const after = steps(marked.model, { _tag: "Loaded", key: marked.model.key, state: { ...state, files: state.files.map((f) => (f.path === "src/a.rs" ? { ...f, viewed: true } : f)) } }).model;
  assert.equal(after.folds.has("src/a.rs"), false);
  assert.equal(isFolded(after, after.state?.files[3] as FileSummary), true);
});

test("a file that has not arrived is a header and a stretch as tall as its diff is guessed to be", () => {
  const m = { ...drawn(), files: new Map<string, FileView>() };
  const { items, sizes } = streamLayout(m, NOW);
  const wait = (path: string) => {
    const i = items.findIndex((x) => x.key === `${path}|wait`);
    return { item: items[i], height: sizes[i] };
  };
  assert.deepEqual(wait("notes.txt").height, 4 * ROW); // untracked, 2 lines added: those and a hunk header
  assert.equal(wait("src/a.rs").height, (2 + 1 + 8) * ROW); // modified: the changed lines and a hunk's header and context
  assert.equal(wait("mod").height, ROW); // a submodule is one line
  assert.equal(estimateRows({ ...(state.files[3] as FileSummary), add: 100, del: 50 }), 158);
  // A file whose fetch failed says so, and is not asked for again.
  const failed = { ...m, skipped: new Set(["notes.txt"]) };
  const text = streamLayout(failed, NOW).items.find((i) => i.key === "notes.txt|wait");
  assert.ok(text && text.kind === "wait" && text.text.startsWith("Could not load"));
});

test("a card is estimated from its text and the width, taller for a narrower stream and for more lines", () => {
  const body = (lines: string[], meta: string | null = null, last = true): BodyItem => ({ kind: "body", key: "x|c:1", id: "1", tone: "user", meta, lines, buttons: [], focused: false, last, half: null });
  const one = heightOf(body(["short"]), 600);
  assert.equal(one, 6 + 18 + 6);
  // 540 px of card minus 22 of padding is 69 characters of 7.5 px: 200 characters wrap to 3 lines.
  assert.equal(cardWidth(600), 540);
  assert.equal(heightOf(body(["x".repeat(200)]), 600), 6 + 3 * 18 + 6);
  assert.equal(heightOf(body(["a", "", "b"]), 600), 6 + 3 * 18 + 6); // an empty line is a line
  assert.ok(heightOf(body(["x".repeat(200)]), 300) > heightOf(body(["x".repeat(200)]), 600));
  assert.equal(heightOf(body(["a"], "claude · now", false), 600), 6 + 16 + 18);
  // The card stops at 560 however wide the stream is.
  assert.equal(heightOf(body(["x".repeat(200)]), 3000), heightOf(body(["x".repeat(200)]), 620));
  // A comment's button row is 4 px of space and a 24 px row of buttons, and wraps to another row in a narrow card.
  const withButtons = (labels: string[]): BodyItem => ({ ...body(["a"]), buttons: labels.map((label) => ({ id: label, label, msg: { _tag: "SelectionCleared" } as Msg, danger: false, off: false })) });
  assert.equal(heightOf(withButtons(["Reply", "Edit"]), 600), 6 + 18 + 4 + 24 + 6);
  assert.equal(heightOf(withButtons(["Reply", "Resolve", "Resend", "Edit", "Delete"]), 600), 6 + 18 + 4 + 24 + 6);
  // A 200 px stream gives a 160 px card and 138 px of text: the five buttons are about 330 px wide, so three rows with a 6 px gap.
  assert.equal(heightOf(withButtons(["Reply", "Resolve", "Resend", "Edit", "Delete"]), 200), 6 + 18 + 4 + 3 * 24 + 2 * 6 + 6);
  // The head is one line, and a quote is another.
  assert.equal(heightOf({ kind: "head", key: "x|h:1", id: "1", tone: "user", title: "t", badges: [], quote: null, collapsible: false, open: true, focused: false, last: false, half: null }, 600), 34);
  assert.equal(heightOf({ kind: "head", key: "x|h:1", id: "1", tone: "user", title: "t", badges: [], quote: "q", collapsible: false, open: true, focused: false, last: false, half: null }, 600), 52);
});

test("the sizes follow the width and height the stream reports", () => {
  const m = drawn();
  const wide = streamLayout({ ...m, expanded: new Set(["u2"]) }, NOW);
  const narrow = streamLayout({ ...m, expanded: new Set(["u2"]), width: 250 }, NOW);
  assert.equal(wide.sizes.length, narrow.sizes.length);
  assert.ok(narrow.sizes.reduce((a, b) => a + b, 0) >= wide.sizes.reduce((a, b) => a + b, 0));
  assert.deepEqual(update(m, { _tag: "Resized", width: m.width, height: m.height }), [m, []]);
  assert.equal(update(m, { _tag: "Resized", width: 321, height: m.height })[0].width, 321);
  assert.equal(update(m, { _tag: "Resized", width: m.width, height: 250 })[0].height, 250);
});

test("E: the stream is built once and kept while a wheel tick, a hover, a drag or a selection changes nothing it is built from", () => {
  const layout = layoutCache();
  const m = drawn();
  const first = layout(m, NOW);
  const row = (x: Model) => layout(x, NOW);
  const press: Msg[] = [{ _tag: "RowPressed", path: "src/a.rs", row: 1, side: null }, { _tag: "Hovered", path: "src/a.rs", row: 4, side: null, on: true }, { _tag: "Wheeled", dy: 40 }, { _tag: "Released" }];
  let x: Model = { ...m, total: first.total };
  for (const msg of press) {
    x = steps(x, msg).model;
    assert.equal(row(x), first, msg._tag);
  }
  assert.ok(x.selection !== null && x.offset > 0); // those did change the model
  assert.equal(layout(steps(m, { _tag: "Scrolled", offset: 0, total: first.total, top: "notes.txt|file", visible: ["notes.txt"], fresh: [], revealed: false }).model, NOW), first);
  // What the items are built from does: the minute, the width, the open threads, the focus, the folds, the files.
  assert.notEqual(layout(m, NOW + 120000), first);
  const again = layout(m, NOW + 120000);
  assert.notEqual(layout({ ...m, width: 300 }, NOW + 120000), again);
  const wide = layout({ ...m, width: 300 }, NOW + 120000);
  assert.notEqual(layout({ ...m, width: 300, focus: "u5" }, NOW + 120000), wide);
  assert.notEqual(layout(steps(m, { _tag: "FoldToggled", path: "src/a.rs" }).model, NOW + 120000), wide);
});

test("a file's items are kept while nothing they show changes, and the same rows serve a new minute or a width", () => {
  const m = drawn();
  const first = streamLayout(m, NOW).items.find((i) => i.key === "src/a.rs|r1");
  assert.equal(streamLayout(steps(m, { _tag: "Wheeled", dy: 10 }).model, NOW).items.find((i) => i.key === "src/a.rs|r1"), first);
  const runsOf = (x: Model, now: number) => {
    const row = streamLayout(x, now).items.find((i) => i.key === "src/a.rs|r1");
    assert.ok(row && row.kind === "ctx");
    return row.runs;
  };
  const cut = runsOf(m, NOW);
  assert.equal(runsOf(m, NOW + 120000), cut);
  assert.equal(runsOf({ ...m, width: m.width + 200 }, NOW), cut);
  assert.equal(runsOf(steps(m, { _tag: "RowPressed", path: "src/a.rs", row: 1, side: null }, { _tag: "Hovered", path: "src/a.rs", row: 3, side: null, on: true }).model, NOW), cut);
});

// --- the order files are fetched in ---

const many = (n: number): FileSummary[] => Array.from({ length: n }, (_, i) => ({ path: `f${i}.rs`, old_path: null, change: "modified" as const, add: 1, del: 0, hash: "0123456789abcdef", viewed: false, threads: 0, open: 0, new: 0, unsent: 0 }));
const big = (n: number): State => ({ ...state, files: many(n), orphans: [] });
const opened = (n: number): { model: Model; first: string[] } => {
  const open = steps(initModel(), { _tag: "Opened", root: "/repo", target: null });
  const load = steps(open.model, { _tag: "Loaded", key: open.model.key, state: big(n) });
  const fetch = load.cmds.find((c) => c._tag === "LoadFiles");
  assert.ok(fetch && fetch._tag === "LoadFiles");
  return { model: load.model, first: fetch.paths };
};
const blankOf = (path: string): FileView => ({ ...file, path, rows: [], threads: [], note: null });

test("E: with nothing on screen yet the first files are fetched, three at a time", () => {
  const { model, first } = opened(10);
  assert.equal(BATCH, 3);
  assert.deepEqual(first, ["f0.rs", "f1.rs", "f2.rs"]);
  assert.equal(model.inFlight, true);
});

test("E: the files on screen and the next few come first, then the rest below them, then the ones above", () => {
  const { model } = opened(10);
  // The screen shows f4 and f5 while the first fetch runs: nothing new starts, one fetch at a time.
  const busy = steps(model, scroll("f4.rs|file", ["f4.rs", "f5.rs"]));
  assert.deepEqual(busy.cmds, []);
  assert.equal(AHEAD, 3);
  assert.deepEqual(wanted({ ...busy.model, files: new Map() }), ["f4.rs", "f5.rs", "f6.rs", "f7.rs", "f8.rs", "f9.rs", "f0.rs", "f1.rs", "f2.rs", "f3.rs"]);
  // When the first fetch ends, the next one is the screen's files first.
  const next = steps(busy.model, { _tag: "FilesLoaded", key: model.key, files: ["f0.rs", "f1.rs", "f2.rs"].map(blankOf) });
  const fetch = next.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles");
  assert.deepEqual(fetch.paths, ["f4.rs", "f5.rs", "f6.rs"]);
  // And so on, down to the last, then the ones above the screen, then nothing.
  const rest = steps(next.model, { _tag: "FilesLoaded", key: model.key, files: fetch.paths.map(blankOf) });
  assert.deepEqual((rest.cmds[0] as { paths: string[] }).paths, ["f7.rs", "f8.rs", "f9.rs"]);
  const last = steps(rest.model, { _tag: "FilesLoaded", key: model.key, files: ["f7.rs", "f8.rs", "f9.rs"].map(blankOf) });
  assert.deepEqual((last.cmds[0] as { paths: string[] }).paths, ["f3.rs"]);
  const done = steps(last.model, { _tag: "FilesLoaded", key: model.key, files: [blankOf("f3.rs")] });
  assert.deepEqual(done.cmds, []);
  assert.equal(done.model.inFlight, false);
  assert.equal(done.model.files.size, 10);
});

test("a scroll while nothing runs starts the fetch for the files now on screen", () => {
  const idle = { ...opened(10).model, inFlight: false, batch: [] };
  const { cmds } = steps(idle, scroll("f7.rs|file", ["f7.rs"]));
  assert.deepEqual((cmds[0] as { paths: string[] }).paths, ["f7.rs", "f8.rs", "f9.rs"]);
});

test("a folded file is not fetched, and a file whose fetch failed is not asked for again until the next load", () => {
  const { model } = opened(5);
  const folded = { ...model, folds: new Map([["f1.rs", true]]) };
  assert.deepEqual(wanted(folded), ["f0.rs", "f2.rs", "f3.rs", "f4.rs"]);
  const failed = steps(model, { _tag: "Failed", op: "files", error: { _tag: "BackendFailed", code: 1, stderr: "boom" } });
  assert.deepEqual([...failed.model.skipped], ["f0.rs", "f1.rs", "f2.rs"]);
  assert.equal(failed.model.status, "boom");
  // The rest is asked for, and the three that failed are not.
  assert.deepEqual((failed.cmds[0] as { paths: string[] }).paths, ["f3.rs", "f4.rs"]);
  const again = steps(failed.model, { _tag: "FilesLoaded", key: model.key, files: ["f3.rs", "f4.rs"].map(blankOf) });
  assert.deepEqual(again.cmds, []);
  // The next load tries them again.
  const timer = steps(again.model, { _tag: "Triggered", source: "timer" });
  assert.equal(timer.model.skipped.size, 0);
  assert.equal(steps(timer.model, { _tag: "Loaded", key: timer.model.key, state: big(5) }).cmds[0]?._tag, "LoadFiles");
});

test("a reload fetches again the files whose summary changed and the files on screen, and no others", () => {
  const m = steps(drawn(), scroll("notes.txt|file", ["notes.txt"])).model;
  const next = { ...state, files: state.files.map((f) => (f.path === "src/a.rs" ? { ...f, add: 9 } : f)) };
  const out = steps({ ...m, inFlight: true }, { _tag: "Loaded", key: m.key, state: next });
  assert.deepEqual([...out.model.dirty].sort(), ["notes.txt", "src/a.rs"]);
  const fetch = out.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles");
  assert.deepEqual(fetch.paths, ["notes.txt", "src/a.rs"]);
  // Until it arrives the old diff stays drawn.
  assert.ok(out.model.files.has("src/a.rs"));
  // A file that left the diff goes.
  const gone = steps({ ...m, inFlight: true }, { _tag: "Loaded", key: m.key, state: { ...state, files: state.files.filter((f) => f.path !== "notes.txt") } });
  assert.equal(gone.model.files.has("notes.txt"), false);
});

test("a reload fetches again an off-screen file whose summary differs only in its hash, and not one whose summary is the same", () => {
  const m = steps(drawn(), scroll("notes.txt|file", ["notes.txt"])).model;
  assert.ok(m.files.has("src/a.rs") && !m.visible.includes("src/a.rs"));
  const same = steps({ ...m, inFlight: true, dirty: new Set() }, { _tag: "Loaded", key: m.key, state });
  assert.equal(same.model.dirty.has("src/a.rs"), false);
  assert.equal(wanted(same.model).includes("src/a.rs"), false);
  const next = { ...state, files: state.files.map((f) => (f.path === "src/a.rs" ? { ...f, hash: "ffffffffffffffff" } : f)) };
  const out = steps({ ...m, inFlight: true, dirty: new Set() }, { _tag: "Loaded", key: m.key, state: next });
  assert.equal(out.model.dirty.has("src/a.rs"), true);
  assert.ok(wanted(out.model).includes("src/a.rs"));
});

test("E: a reply for another root or spec than the model's is dropped", () => {
  const { model } = opened(4);
  const stale = steps(model, { _tag: "FilesLoaded", key: "old", files: [blankOf("f0.rs")] });
  assert.equal(stale.model.files.size, 0);
  // The fetch that was asked for is asked again.
  assert.deepEqual((stale.cmds[0] as { paths: string[] }).paths, ["f0.rs", "f1.rs", "f2.rs"]);
});

// --- which file the sidebar shows ---

test("E: the topmost file on screen is the sidebar's selected file", () => {
  const m = drawn();
  const s = steps(m, scroll("notes.txt|r1", ["notes.txt", "src/a.rs"])).model;
  assert.equal(s.selected, "notes.txt");
  assert.equal(s.top, "notes.txt|r1");
  // Only the threads not in the diff on screen: the selection stays.
  assert.equal(steps(s, scroll("|h:u6", [])).model.selected, "notes.txt");
  // A scroll that moves nothing changes nothing the sidebar draws.
  assert.equal(steps(s, scroll("notes.txt|r2", ["notes.txt", "src/a.rs"])).model.selected, "notes.txt");
});

test("E: a click in the sidebar scrolls the stream to that file's header, and the file stays selected where the stream ends too soon", () => {
  const m = drawn();
  const click = steps(m, { _tag: "FileChosen", path: "src/b.rs" }).model;
  assert.equal(click.selected, "src/b.rs");
  const target = click.reveal;
  assert.deepEqual(target, { key: "src/b.rs|file" });
  const { items, sizes } = streamLayout(click, NOW);
  const at = indexOfKey(items, target?.key ?? "");
  assert.equal(at, items.length - 1);
  // The scroll offset is the heights before it.
  assert.equal(sizes.slice(0, at).reduce((a, b) => a + b, 0), sizes.reduce((a, b) => a + b, 0) - 30);
  // The stream could not put the header at the top: the last screen shows a.rs above it. The clicked file wins.
  const landed = steps(click, scroll("src/a.rs|r3", ["src/a.rs", "src/b.rs"], true)).model;
  assert.equal(landed.selected, "src/b.rs");
  assert.equal(landed.reveal, null);
  // Once the request is done, the topmost file wins again.
  assert.equal(steps(landed, scroll("src/a.rs|r2", ["src/a.rs", "src/b.rs"])).model.selected, "src/a.rs");
});

test("next and previous comment scroll to the card: the item the card is", () => {
  const m = drawn();
  const { items } = streamLayout(m, NOW);
  const next = steps({ ...m, focus: "u5" }, { _tag: "ThreadJumped", forward: true }).model;
  assert.equal(next.focus, "u3");
  assert.equal(items[indexOfKey(items, next.reveal?.key ?? "")]?.key, "src/a.rs|h:u3");
  const prev = steps({ ...m, focus: "u5" }, { _tag: "ThreadJumped", forward: false }).model;
  assert.equal(items[indexOfKey(items, prev.reveal?.key ?? "")]?.key, "|h:u6");
  assert.ok(indexOfKey(items, "|h:u6") < indexOfKey(items, "src/a.rs|h:u5"));
});

test("a file that arrives above the top of the screen puts the item that was at the top back at the top", () => {
  const { model } = opened(6);
  const at = steps(model, scroll("f4.rs|r2", ["f4.rs", "f5.rs"])).model;
  const above = steps(at, { _tag: "FilesLoaded", key: at.key, files: [blankOf("f0.rs")] });
  assert.deepEqual(above.model.reveal, { key: "f4.rs|r2" });
  // One below the top or on screen moves nothing, and a scroll the panel is making is left alone.
  assert.equal(steps(at, { _tag: "FilesLoaded", key: at.key, files: [blankOf("f5.rs")] }).model.reveal, null);
  const going = { ...at, reveal: { key: "f2.rs|file" } };
  assert.deepEqual(steps(going, { _tag: "FilesLoaded", key: at.key, files: [blankOf("f0.rs")] }).model.reveal, { key: "f2.rs|file" });
});

test("the files on screen are the distinct paths of the items from the first to the last, and the threads not in the diff have none", () => {
  const { items } = streamLayout(drawn(), NOW);
  const from = indexOfKey(items, "notes.txt|r1");
  const to = indexOfKey(items, "src/a.rs|h:u5") + 1;
  assert.deepEqual(pathsOf(items, from, to), ["notes.txt", "src/a.rs"]);
  assert.deepEqual(pathsOf(items, 0, 3), []);
  assert.deepEqual(pathsOf(items, -5, 1), []);
});
