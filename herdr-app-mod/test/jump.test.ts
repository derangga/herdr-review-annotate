// The E: lines of design/graphs/gui-thread.md for next and previous comment, one test each.
import assert from "node:assert/strict";
import { test } from "node:test";
import { cardOrder, dock, jumpStop, panelKey, stops } from "../src/rows.ts";
import { listen } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import type { FileView, Model, Msg, State } from "../src/types.ts";
import { fakeHost } from "./fake-host.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const base = fixtureState();
const file = fixtureFile();

// The review as the fixtures have it: one thread not in the diff (u6) and src/a.rs with u5, u3, u1, u2, u4 on
// screen. The other files are given threads here as a test needs them.
const withThreads = (counts: Record<string, number>): State => ({ ...base, files: base.files.map((f) => ({ ...f, threads: counts[f.path] ?? f.threads })) });
const A = ["u5", "u3", "u1", "u2", "u4"];

// A panel that has every file but the ones named, so those are files whose threads are not drawn yet.
function loaded(state: State = base, shown: FileView = file, ...unloaded: string[]): Model {
  const m = loadedModel(state, shown);
  return { ...m, files: new Map([...m.files].filter(([path]) => !unloaded.includes(path))) };
}
const at = (m: Model, id: string | null): Model => ({ ...m, focus: id });
const ids = (list: { id?: string; path?: string }[]) => list.map((s) => s.id ?? `file:${s.path}`);
const jump = (m: Model, forward: boolean) => steps(m, { _tag: "ThreadJumped", forward });
const other = (path: string, threads: FileView["threads"]): FileView => ({ ...file, path, threads });

test("the cards of a file are in screen order: no row first, then by row, not the reply's order", () => {
  assert.deepEqual(
    file.threads.map((t) => t.id),
    ["u1", "u2", "u3", "u4", "u5"],
  );
  assert.deepEqual(cardOrder(file), A);
});

test("the stops are the threads not in the diff, then each file in the sidebar's order: a file that is drawn by its cards, the others by one stop", () => {
  const state = withThreads({ "notes.txt": 2, "src/b.rs": 1 });
  const { all, top } = stops(loaded(state, file, "notes.txt", "src/b.rs"));
  assert.deepEqual(ids(all), ["u6", "file:notes.txt", ...A, "file:src/b.rs"]);
  assert.equal(top, 2);
  // A file with no thread has no stop.
  assert.deepEqual(ids(stops(loaded()).all), ["u6", ...A]);
});

test("a folded file with threads is one stop, even when it is loaded", () => {
  const m = loaded(withThreads({ "img.png": 1 }), file, "img.png");
  assert.deepEqual(ids(stops(m).all), ["u6", "file:img.png", ...A]);
  const folded = { ...loaded(), folds: new Map([["src/a.rs", true]]) };
  assert.deepEqual(ids(stops(folded).all), ["u6", "file:src/a.rs"]);
});

test("next and previous walk the stops across files and wrap round at both ends", () => {
  const m = loaded(withThreads({ "notes.txt": 2, "src/b.rs": 1 }), file, "notes.txt", "src/b.rs");
  const next = (id: string) => ids([jumpStop(at(m, id), true) ?? {}]);
  const prev = (id: string) => ids([jumpStop(at(m, id), false) ?? {}]);
  assert.deepEqual(next("u6"), ["file:notes.txt"]);
  assert.deepEqual(next("u5"), ["u3"]);
  assert.deepEqual(next("u4"), ["file:src/b.rs"]);
  assert.deepEqual(prev("u5"), ["file:notes.txt"]);
  assert.deepEqual(prev("u3"), ["u5"]);
  // First stop back wraps to the last, last forward wraps to the first.
  assert.deepEqual(prev("u6"), ["file:src/b.rs"]);
  const plain = loaded();
  assert.deepEqual(ids([jumpStop(at(plain, "u4"), true) ?? {}]), ["u6"]);
  assert.deepEqual(ids([jumpStop(at(plain, "u6"), false) ?? {}]), ["u4"]);
});

test("a focus on a reply counts as a focus on its thread", () => {
  const m = loaded();
  assert.deepEqual(jumpStop(at(m, "a1"), true), { id: "u4" }); // a1 is the agent's reply in u2, and u4 follows u2
  assert.deepEqual(jumpStop(at(m, "a1"), false), { id: "u1" });
});

test("with nothing focused, next is the selected file's first thread and previous the last one before the file", () => {
  const m = loaded(withThreads({ "notes.txt": 2, "src/b.rs": 1 }), file, "notes.txt", "src/b.rs");
  assert.deepEqual(jumpStop(m, true), { id: "u5" });
  assert.deepEqual(jumpStop(m, false), { path: "notes.txt" });
  // Nothing before it but the threads not in the diff.
  assert.deepEqual(jumpStop(loaded(), false), { id: "u6" });
  // A focus the review no longer has counts as none.
  assert.deepEqual(jumpStop(at(m, "gone"), true), { id: "u5" });
});

test("no threads in the review: nothing to jump to, a status, the buttons off", () => {
  const empty = { ...base, orphans: [] };
  const none: FileView = { ...file, threads: [] };
  const m = loadedModel({ ...empty, files: empty.files.map((f) => ({ ...f, threads: 0 })) }, none);
  assert.equal(jumpStop(m, true), null);
  const { model, cmds } = jump(m, true);
  assert.equal(model.status, "no comments in this review");
  assert.deepEqual(cmds, []);
  assert.equal(model.focus, null);
  const nav = dock(m).actions.filter((b) => b.id === "prev-thread" || b.id === "next-thread");
  assert.deepEqual(
    nav.map((b) => [b.label, b.off]),
    [
      ["Previous comment", true],
      ["Next comment", true],
    ],
  );
  assert.deepEqual(
    dock(loaded())
      .actions.filter((b) => b.id.endsWith("-thread"))
      .map((b) => b.off),
    [false, false],
  );
});

test("a thread that is drawn is focused and scrolled to at once, with no fetch", () => {
  const m = loaded();
  const first = jump(at(m, "u5"), true);
  assert.equal(first.model.focus, "u3");
  assert.deepEqual(first.model.reveal, { key: "src/a.rs|h:u3" });
  assert.deepEqual(first.cmds, []);
  // The thread that is not in the diff hangs from no file.
  const back = jump(at(m, "u5"), false);
  assert.equal(back.model.focus, "u6");
  assert.deepEqual(back.model.reveal, { key: "|h:u6" });
  // Asking for the same thread again is a new request, so the stream scrolls again.
  const once = jump(at(m, "u3"), true).model;
  const twice = jump(at(m, "u3"), true).model;
  assert.equal(twice.focus, "u1");
  assert.notEqual(once.reveal, twice.reveal);
  assert.deepEqual(twice.reveal, once.reveal);
  // It also drops a question that was up and the status.
  const asked = jump({ ...at(m, "u5"), confirm: "u5", status: "old" }, true).model;
  assert.deepEqual([asked.confirm, asked.status], [null, null]);
});

test("a jump into a file that has not arrived scrolls to its header, fetches it first, and lands when it arrives", () => {
  const state = withThreads({ "src/b.rs": 2 });
  const m = loaded(state, file, "src/b.rs");
  const go = jump(at(m, "u4"), true);
  assert.deepEqual(go.model.jump, { path: "src/b.rs", edge: "first" });
  assert.deepEqual(go.model.reveal, { key: "src/b.rs|file" });
  const fetch = go.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles" && fetch.paths[0] === "src/b.rs");
  // The focus stays where it was until the file is there.
  assert.equal(go.model.focus, "u4");
  const [t1, t2] = [file.threads[0], file.threads[1]] as [FileView["threads"][number], FileView["threads"][number]];
  const b = other("src/b.rs", [
    { ...t2, id: "b2", placement: { kind: "matched", row: 5, start_row: null }, comments: [{ ...t2.comments[0]!, id: "b2" }] },
    { ...t1, id: "b1", placement: { kind: "matched", row: 1, start_row: null }, comments: [{ ...t1.comments[0]!, id: "b1" }] },
  ]);
  const done = steps(go.model, { _tag: "FilesLoaded", key: fetch.key, files: [b] }).model;
  assert.equal(done.jump, null);
  // Going forward lands on the first card on screen, which is not the first the reply lists.
  assert.equal(done.focus, "b1");
  assert.deepEqual(done.reveal, { key: "src/b.rs|h:b1" });
});

test("going back into a file lands on its last card", () => {
  const state = withThreads({ "notes.txt": 2 });
  const go = jump(at(loaded(state, file, "notes.txt"), "u5"), false);
  assert.deepEqual(go.model.jump, { path: "notes.txt", edge: "last" });
  const fetch = go.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles");
  const [t1, t2] = [file.threads[0], file.threads[1]] as [FileView["threads"][number], FileView["threads"][number]];
  const n = other("notes.txt", [
    { ...t1, id: "n1", placement: { kind: "matched", row: 1, start_row: null } },
    { ...t2, id: "n2", placement: { kind: "matched", row: 4, start_row: null } },
  ]);
  assert.equal(steps(go.model, { _tag: "FilesLoaded", key: fetch.key, files: [n] }).model.focus, "n2");
});

test("a folded file that is loaded is opened and landed on at once, and one that is not is opened and waited for", () => {
  const folded = { ...loaded(), folds: new Map([["src/a.rs", true]]) };
  const go = jump(at(folded, "u6"), true);
  assert.equal(go.model.folds.get("src/a.rs"), false);
  assert.equal(go.model.focus, "u5");
  assert.deepEqual(go.model.reveal, { key: "src/a.rs|h:u5" });
  assert.equal(go.model.jump, null);
  // The review's last thread going back, from the first one, is the same file's last card.
  const back = jump(at(folded, "u6"), false);
  assert.equal(back.model.focus, "u4");
  // A viewed file with a thread that was never fetched: unfolded, and the fetch starts with it.
  const viewed = loaded(withThreads({ "img.png": 1 }), file, "img.png");
  const wait = jump(at({ ...viewed, selected: "img.png" }, null), true);
  assert.equal(wait.model.folds.get("img.png"), false);
  assert.deepEqual(wait.model.jump, { path: "img.png", edge: "first" });
  const fetch = wait.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles" && fetch.paths[0] === "img.png");
});

test("a reply that is not the one the jump asked for leaves it waiting, and a failed fetch drops it", () => {
  const state = withThreads({ "src/b.rs": 1 });
  const go = jump(at(loaded(state, file, "src/b.rs"), "u4"), true);
  const stale = steps(go.model, { _tag: "FilesLoaded", key: "an old key", files: [file] }).model;
  assert.deepEqual(stale.jump, { path: "src/b.rs", edge: "first" });
  assert.equal(stale.focus, "u4");
  const failed = steps(go.model, { _tag: "Failed", op: "files", error: { _tag: "Busy" } }).model;
  assert.equal(failed.jump, null);
});

test("choosing a file or a spec while a jump waits drops the jump", () => {
  const go = jump(at(loaded(withThreads({ "src/b.rs": 1 }), file, "src/b.rs"), "u4"), true).model;
  assert.equal(steps(go, { _tag: "FileChosen", path: "notes.txt" }).model.jump, null);
  assert.equal(steps(go, { _tag: "SpecChosen", spec: { kind: "branch", base: "main" } }).model.jump, null);
});

test("a file that has lost its threads by the time it arrives says so and keeps the focus", () => {
  const state = withThreads({ "src/b.rs": 1 });
  const go = jump(at(loaded(state, file, "src/b.rs"), "u4"), true);
  const fetch = go.cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles");
  const done = steps(go.model, { _tag: "FilesLoaded", key: fetch.key, files: [other("src/b.rs", [])] }).model;
  assert.equal(done.jump, null);
  assert.equal(done.status, "no comments in src/b.rs");
  assert.equal(done.focus, "u4");
});

test("a jump with the editor open changes nothing but the status, and the picker swallows it", () => {
  const m = loaded();
  const editing: Model = { ...at(m, "u5"), editor: { kind: "comment", anchor: null, thread: null, comment: null, initial: "" } };
  const r = jump(editing, true);
  assert.equal(r.model.focus, "u5");
  assert.equal(r.model.status, "finish or cancel the comment first");
  assert.deepEqual(r.cmds, []);
  assert.deepEqual(jump({ ...at(m, "u5"), picker: [] }, true).model.focus, "u5");
});

test("n is next, shift+n is previous, other chords are not ours, and nothing is taken while a note is typed or asked", () => {
  const m = loaded();
  assert.deepEqual(panelKey(m, "n"), { _tag: "ThreadJumped", forward: true });
  assert.deepEqual(panelKey(m, "shift-n"), { _tag: "ThreadJumped", forward: false });
  assert.deepEqual(panelKey(m, "N"), { _tag: "ThreadJumped", forward: false });
  for (const key of ["cmd-n", "ctrl-n", "x", "escape", "enter"]) {
    assert.equal(panelKey(m, key), null, key);
  }
  const editing: Model = { ...m, editor: { kind: "reply", anchor: null, thread: "u1", comment: null, initial: "" } };
  assert.equal(panelKey(editing, "n"), null);
  assert.equal(panelKey({ ...m, confirm: "u1" }, "n"), null);
  assert.equal(panelKey({ ...m, confirmArchive: true }, "n"), null);
  assert.equal(panelKey({ ...m, picker: [] }, "n"), null);
});

test("herdr-app's commands: focus gives the panel the keyboard, the two comment commands jump, others do nothing, and it stops on close", async () => {
  const fake = fakeHost();
  const commands = ["focus", "alt-s", "next-comment", "previous-comment", "focus"];
  let closed = false;
  fake.host.nextCommand = async () => {
    const c = commands.shift();
    if (c === undefined) {
      closed = true;
      return "focus";
    }
    return c;
  };
  const sent: Msg[] = [];
  const env: Env = { host: fake.host, cli: null as never, dispatch: (m) => sent.push(m), closed: () => closed };
  let focused = 0;
  await listen(env, () => focused++);
  assert.equal(focused, 2); // the last "focus" came after the close
  assert.deepEqual(sent, [
    { _tag: "ThreadJumped", forward: true },
    { _tag: "ThreadJumped", forward: false },
  ]);
  fake.host.nextCommand = () => Promise.reject(new Error("gone"));
  assert.equal(await listen({ ...env, closed: () => false }, () => focused++), null);
});
