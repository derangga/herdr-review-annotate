import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeFile, decodeState } from "../src/decode.ts";
import { cleanText, diffWidth, layout, paneOrder, runs, sidebarButtons, sidebarRows, tokenColor, totals } from "../src/rows.ts";
import { initModel } from "../src/update.ts";
import type { RowItem } from "../src/rows.ts";
import type { FileSummary } from "../src/types.ts";
import { fixture } from "./fake-host.ts";

const file = decodeFile(JSON.parse(fixture("file")));
const state = decodeState(JSON.parse(fixture("state")));

const model = { ...initModel(), root: "/repo", state, file };
const NOW = Date.parse("2026-11-09T10:00:30Z");

test("layout keeps the rows in order with their line numbers and the index into the file", () => {
  const items = layout(model, NOW).filter((i): i is RowItem => i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del");
  assert.equal(items.length, 7);
  assert.deepEqual(items.map((i) => i.kind), ["hunk", "ctx", "del", "add", "add", "ctx", "ctx"]);
  assert.deepEqual(items.map((i) => i.index), [0, 1, 2, 3, 4, 5, 6]);
  assert.deepEqual([items[2]?.old, items[2]?.new], ["2", ""]);
  assert.deepEqual([items[3]?.old, items[3]?.new], ["", "2"]);
  assert.equal(items[0]?.text, "@@ -1,4 +1,5 @@");
  assert.deepEqual(layout({ ...model, state: null, file: null }, NOW), []);
});

test("text is drawn without control characters and a tab is spaces", () => {
  assert.equal(cleanText("a\tb\u001b[31mc\u0007\r"), "a    b[31mc");
  assert.equal(cleanText("plain, with unicode: é中"), "plain, with unicode: é中");
});

test("a row is cut into runs at its tokens, the text between them plain, each piece cleaned", () => {
  const plain = "foreground";
  assert.deepEqual(runs("fn main() {", []), [{ text: "fn main() {", color: plain }]);
  assert.deepEqual(runs("fn main() {", [{ from: 0, to: 2, kind: "keyword" }, { from: 3, to: 7, kind: "function" }]), [
    { text: "fn", color: tokenColor("keyword") },
    { text: " ", color: plain },
    { text: "main", color: tokenColor("function") },
    { text: "() {", color: plain },
  ]);
  // The runs put back together are the cleaned text, a tab included, and a piece that cleans to nothing is left out.
  const text = "\tlet a = 1;\u0007 // x";
  const tokens = [{ from: 1, to: 4, kind: "keyword" as const }, { from: 12, to: 13, kind: "comment" as const }, { from: 14, to: 18, kind: "comment" as const }];
  const cut = runs(text, tokens);
  assert.equal(cut.map((r) => r.text).join(""), cleanText(text));
  assert.deepEqual(cut[0], { text: "    ", color: plain });
  assert.ok(cut.every((r) => r.text !== ""));
  assert.deepEqual(runs("", []), []);
});

test("E: a row with a non-ASCII character is cut where Rust cut it", () => {
  const text = 'let s = "\u00e9\u{1f600}"; // x';
  const cut = runs(text, [
    { from: 0, to: 3, kind: "keyword" },
    { from: 8, to: 13, kind: "string" },
    { from: 15, to: 19, kind: "comment" },
  ]);
  assert.deepEqual(
    cut.map((r) => r.text),
    ["let", " s = ", '"\u00e9\u{1f600}"', "; ", "// x"],
  );
  assert.equal(cut[2]?.color, tokenColor("string"));
});

test("every token kind has a colour from the theme, and no two of the TUI's kinds share one", () => {
  // The names herdr.d.ts lists for theme().
  const names = new Set(["background", "surface", "canvas", "chrome", "popover", "raised", "hover", "selected", "border", "input", "foreground", "muted", "dim", "accent", "accent_foreground", "solid", "solid_foreground", "selection", "working", "blocked", "done", "idle", "unknown", ...Array.from({ length: 16 }, (_, i) => `ansi${i}`)]);
  const kinds = ["comment", "string", "number", "keyword", "operator", "function", "type"] as const;
  const colors = kinds.map(tokenColor);
  for (const c of colors) assert.ok(names.has(c), c);
  assert.equal(new Set(colors).size, kinds.length);
});

test("layout gives each code row its runs from the file's tokens", () => {
  const items = layout(model, NOW).filter((i): i is RowItem => i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del");
  assert.deepEqual(items[1]?.runs, [
    { text: "fn", color: tokenColor("keyword") },
    { text: " ", color: "foreground" },
    { text: "main", color: tokenColor("function") },
    { text: "() {", color: "foreground" },
  ]);
  assert.deepEqual(items[0]?.runs, [{ text: "@@ -1,4 +1,5 @@", color: "foreground" }]);
});

test("the diff is as wide as its longest line, and at least 300", () => {
  assert.equal(diffWidth([]), 300);
  assert.equal(diffWidth([{ kind: "ctx", key: "r0", index: 0, old: "1", new: "1", text: "x".repeat(100), runs: [], selected: false }]), 820);
});

test("the sidebar has a heading where the folder changes, and the files under it", () => {
  const rows = sidebarRows(state.files);
  const kinds = rows.map((r) => (r.kind === "dir" ? `dir ${r.text}` : `file ${r.kind === "file" ? r.path : ""}`));
  assert.equal(rows.filter((r) => r.kind === "file").length, state.files.length);
  assert.equal(rows[0]?.kind, "dir");
  for (let i = 1; i < rows.length; i++) assert.ok(!(rows[i]?.kind === "dir" && rows[i - 1]?.kind === "dir"), kinds.join("\n"));
  const a = rows.find((r) => r.kind === "file" && r.path === "src/a.rs");
  assert.deepEqual(a, {
    kind: "file",
    key: "f:src/a.rs",
    path: "src/a.rs",
    name: "a.rs",
    icon: "rust",
    letter: "M",
    add: 2,
    del: 1,
    viewed: false,
    badges: [
      { text: "4 unsent", tone: "unsent" },
      { text: "3 open", tone: "open" },
      { text: "1 new", tone: "new" },
    ],
  });
  const bin = rows.find((r) => r.kind === "file" && r.path === "img.png");
  assert.ok(bin && bin.kind === "file");
  assert.deepEqual([bin.letter, bin.viewed, bin.badges.at(-1)], ["B", true, { text: "viewed", tone: "viewed" }]);
});

const f = (path: string, change: FileSummary["change"] = "modified"): FileSummary => ({ path, old_path: null, change, add: 1, del: 0, viewed: false, threads: 0, open: 0, new: 0, unsent: 0 });

test("files group by consecutive folder, the top folder is ./, and a folder that returns gets a second heading", () => {
  const rows = sidebarRows([f("src/a.rs"), f("src/b.rs"), f("README.md"), f("src/deep/c.rs"), f("src/d.rs", "untracked")]);
  assert.deepEqual(
    rows.map((r) => (r.kind === "dir" ? r.text : r.kind === "file" ? r.name : "")),
    ["src/", "a.rs", "b.rs", "./", "README.md", "src/deep/", "c.rs", "src/", "d.rs"],
  );
  assert.equal(new Set(rows.map((r) => r.key)).size, rows.length);
  assert.deepEqual(sidebarRows([]), []);
});

test("a folder or file name is drawn without control characters", () => {
  const rows = sidebarRows([f("sr\u001bc/a\tb.rs")]);
  assert.deepEqual(rows.map((r) => (r.kind === "dir" ? r.text : r.kind === "file" ? r.name : "")), ["src/", "a    b.rs"]);
});

test("totals add the counts of every file", () => {
  assert.deepEqual(totals(state), { files: 5, add: 5, del: 1 });
  assert.deepEqual(totals(null), { files: 0, add: 0, del: 0 });
});

test("the sidebar buttons name what a click does, and moving is offered only while the sidebar shows", () => {
  const at = (side: "left" | "right", open: boolean) => sidebarButtons({ ...model, panel: { side, open } }).map((b) => [b.label, b.msg._tag]);
  assert.deepEqual(at("left", true), [["Hide files", "SidebarToggled"], ["Move right", "SidebarFlipped"]]);
  assert.deepEqual(at("right", true), [["Hide files", "SidebarToggled"], ["Move left", "SidebarFlipped"]]);
  assert.deepEqual(at("right", false), [["Show files", "SidebarToggled"]]);
});

test("the panes go left to right with the sidebar on its side, and without it when hidden", () => {
  const at = (side: "left" | "right", open: boolean) => paneOrder({ ...model, panel: { side, open } });
  assert.deepEqual(at("left", true), ["sidebar", "diff"]);
  assert.deepEqual(at("right", true), ["diff", "sidebar"]);
  assert.deepEqual(at("left", false), ["diff"]);
  assert.deepEqual(at("right", false), ["diff"]);
});
