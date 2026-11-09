import assert from "node:assert/strict";
import { test } from "node:test";
import { fileMatches, filterCount, filterInfo, filterPick, panelKey, sidebarRows, streamLayout } from "../src/rows.ts";
import { update } from "../src/update.ts";
import type { FileSummary, Model, Msg } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
const base: Model = loadedModel(state, file);

const f = (path: string): FileSummary => ({ path, old_path: null, change: "modified", add: 1, del: 0, viewed: false, threads: 0, open: 0, new: 0, unsent: 0 });
const paths = (q: string, files: FileSummary[]) => sidebarRows(files, q).flatMap((r) => (r.kind === "file" ? [r.path] : []));
const run = (m: Model, ...msgs: Msg[]) => msgs.reduce<{ m: Model; cmds: unknown[] }>((acc, msg) => {
  const [next, out] = update(acc.m, msg);
  return { m: next, cmds: [...acc.cmds, ...out] };
}, { m, cmds: [] });

test("a path matches when the query's characters appear in it in order", () => {
  assert.ok(fileMatches("sra", "src/a.rs"));
  assert.ok(fileMatches("", "anything"));
  assert.ok(!fileMatches("asr", "src/a.rs")); // out of order
  assert.ok(!fileMatches("aa", "src/a.rs")); // each character is used once
  assert.ok(fileMatches("é中", "docs/é/中.md"));
});

test("a query with no upper case ignores case, one upper case letter makes it exact", () => {
  assert.ok(fileMatches("readme", "README.md"));
  assert.ok(fileMatches("RE", "README.md"));
  assert.ok(!fileMatches("Re", "README.md"));
  assert.ok(!fileMatches("rEadme", "README.md".toLowerCase()));
  assert.ok(fileMatches("Cargo", "Cargo.toml"));
  assert.ok(!fileMatches("Cargo", "cargo.toml"));
});

test("a space is a character of the query", () => {
  assert.ok(!fileMatches("a b", "src/a.rs"));
  assert.ok(fileMatches("a b", "my dir/a b.rs"));
  assert.ok(!fileMatches(" ", "src/a.rs"));
});

test("the sidebar keeps the diff's order and its headings, and a folder with no match has no heading", () => {
  const files = [f("src/a.rs"), f("src/b.rs"), f("README.md"), f("src/deep/c.rs"), f("src/d.rs")];
  assert.deepEqual(sidebarRows(files, "").map((r) => r.kind), ["dir", "file", "file", "dir", "file", "dir", "file", "dir", "file"]);
  const rows = sidebarRows(files, "srcd");
  assert.deepEqual(rows.map((r) => (r.kind === "dir" ? r.text : r.kind === "file" ? r.path : "")), ["src/deep/", "src/deep/c.rs", "src/", "src/d.rs"]);
  assert.deepEqual(paths("b", files), ["src/b.rs"]);
  assert.deepEqual(sidebarRows(files, "readme").map((r) => (r.kind === "dir" ? r.text : r.name)), ["./", "README.md"]);
  assert.deepEqual(sidebarRows(files, "zzz"), []);
});

test("the count is the files that match over all of them", () => {
  const files = state.files;
  assert.deepEqual(filterCount(files, ""), { shown: 5, total: 5 });
  assert.deepEqual(filterCount(files, "src"), { shown: 2, total: 5 });
  assert.deepEqual(filterInfo({ ...base, filter: "src" }), { count: "2/5", none: false });
  assert.deepEqual(filterInfo({ ...base, filter: "zzz" }), { count: "0/5", none: true });
  assert.deepEqual(filterInfo({ ...base, state: null, filter: "zzz" }), { count: "0/0", none: false });
});

test("only the sidebar is filtered: the diff, the jumps and send see every file", () => {
  const m = { ...base, filter: "zzz" };
  assert.deepEqual(streamLayout(m, 0), streamLayout(base, 0));
  assert.deepEqual(update(m, { _tag: "ThreadJumped", forward: true })[0].jump, update(base, { _tag: "ThreadJumped", forward: true })[0].jump);
});

test("typing sets the query and focus says whether a key types", () => {
  const { m } = run(base, { _tag: "FilterFocused", on: true }, { _tag: "FilterChanged", text: "src" });
  assert.equal(m.filter, "src");
  assert.equal(m.filtering, true);
  assert.equal(run(m, { _tag: "FilterFocused", on: false }).m.filtering, false);
});

test("while the box has the keyboard n, N and / type and only Esc is a command", () => {
  const m = { ...base, filtering: true, filter: "x" };
  for (const key of ["n", "shift-n", "N", "/", "enter", "a"]) assert.equal(panelKey(m, key), null, key);
  assert.deepEqual(panelKey(m, "escape"), { _tag: "FilterCleared" });
  assert.deepEqual(panelKey({ ...m, filter: "" }, "escape"), { _tag: "FilterCleared" });
});

test("on the panel / opens the box, n still jumps, and Esc clears a filter that is on", () => {
  assert.deepEqual(panelKey(base, "/"), { _tag: "FilterOpened" });
  assert.deepEqual(panelKey(base, "n"), { _tag: "ThreadJumped", forward: true });
  assert.equal(panelKey(base, "escape"), null);
  assert.deepEqual(panelKey({ ...base, filter: "a" }, "escape"), { _tag: "FilterCleared" });
});

test("/ does nothing while the editor, a question or the picker is up", () => {
  const editor = { kind: "reply" as const, anchor: null, thread: "t1", comment: null, initial: "" };
  assert.equal(panelKey({ ...base, editor }, "/"), null);
  assert.equal(panelKey({ ...base, confirm: "u1" }, "/"), null);
  assert.equal(panelKey({ ...base, confirmArchive: true }, "/"), null);
  assert.equal(panelKey({ ...base, picker: [] }, "/"), null);
  assert.deepEqual(update({ ...base, picker: [] }, { _tag: "FilterOpened" }), [{ ...base, picker: [] }, []]);
});

test("/ with the sidebar hidden shows it and remembers that", () => {
  const hidden = { ...base, panel: { side: "left" as const, open: false } };
  const { m, cmds } = run(hidden, { _tag: "FilterOpened" });
  assert.equal(m.panel.open, true);
  assert.deepEqual(cmds, [{ _tag: "SavePanel", panel: { side: "left", open: true } }]);
  assert.deepEqual(update(base, { _tag: "FilterOpened" }), [base, []]);
});

test("hiding the sidebar ends typing in the box, since it is no longer drawn", () => {
  assert.equal(run({ ...base, filtering: true }, { _tag: "SidebarToggled" }).m.filtering, false);
});

test("Esc empties the filter and gives the keyboard back", () => {
  const { m } = run({ ...base, filtering: true, filter: "src" }, { _tag: "FilterCleared" });
  assert.deepEqual([m.filter, m.filtering], ["", false]);
});

test("Enter keeps the filter and shows the first match when the file shown does not match", () => {
  const { m, cmds } = run({ ...base, filtering: true, filter: "notes" }, { _tag: "FilterEntered" });
  assert.equal(m.filter, "notes");
  assert.equal(m.filtering, false);
  assert.equal(m.selected, "notes.txt");
  assert.deepEqual(m.reveal, { key: "notes.txt|file" });
  assert.deepEqual(cmds, []);
});

test("Enter leaves the file shown alone when it matches", () => {
  const { m, cmds } = run({ ...base, filtering: true, filter: "a.rs" }, { _tag: "FilterEntered" });
  assert.equal(m.selected, "src/a.rs");
  assert.equal(m.filtering, false);
  assert.deepEqual(cmds, []);
  assert.equal(filterPick({ ...base, filter: "rs" }), null);
});

test("Enter with no match does nothing, and on an empty query it only gives the keyboard back", () => {
  const none = { ...base, filtering: true, filter: "zzz" };
  assert.deepEqual(update(none, { _tag: "FilterEntered" }), [none, []]);
  assert.equal(filterPick(none), null);
  const empty = { ...base, filtering: true };
  assert.deepEqual(update(empty, { _tag: "FilterEntered" }), [{ ...empty, filtering: false }, []]);
});

test("the query survives a reload", () => {
  const m = { ...base, filter: "src" };
  const [next] = update({ ...m, inFlight: true }, { _tag: "Loaded", key: m.key, state });
  assert.equal(next.filter, "src");
});
