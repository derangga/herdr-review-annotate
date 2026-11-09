// The E: lines of design/graphs/gui-thread.md for next and previous comment, one test each.
import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeFile, decodeState } from "../src/decode.ts";
import { cardOrder, dock, itemClick, jumpStop, layout, listId, panelKey, stops, windowStart } from "../src/rows.ts";
import { listen } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import { initModel, update } from "../src/update.ts";
import type { Cmd, FileView, Model, Msg, State } from "../src/types.ts";
import { fakeHost, fixture } from "./fake-host.ts";

const base = decodeState(JSON.parse(fixture("state")));
const file = decodeFile(JSON.parse(fixture("file")));

// The review as the fixtures have it: one thread not in the diff (u6) and src/a.rs with u5, u3, u1, u2, u4 on
// screen. The other files are given threads here as a test needs them.
const withThreads = (counts: Record<string, number>): State => ({ ...base, files: base.files.map((f) => ({ ...f, threads: counts[f.path] ?? f.threads })) });
const A = ["u5", "u3", "u1", "u2", "u4"];

function steps(m: Model, ...msgs: Msg[]): { model: Model; cmds: Cmd[] } {
  const cmds: Cmd[] = [];
  for (const msg of msgs) {
    const [next, out] = update(m, msg);
    m = next;
    cmds.push(...out);
  }
  return { model: m, cmds };
}

// A panel that has loaded src/a.rs.
function loaded(state: State = base, shown: FileView | null = file): Model {
  const { model, cmds } = steps(initModel(), { _tag: "Opened", root: "/repo", target: null });
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  return steps(model, { _tag: "Loaded", key: load.key, state, file: shown }).model;
}
const at = (m: Model, id: string | null): Model => ({ ...m, focus: id });
const ids = (list: { id?: string; path?: string }[]) => list.map((s) => s.id ?? `file:${s.path}`);
const jump = (m: Model, forward: boolean) => steps(m, { _tag: "ThreadJumped", forward });

test("the cards of a file are in screen order: no row first, then by row, not the reply's order", () => {
  assert.deepEqual(file.threads.map((t) => t.id), ["u1", "u2", "u3", "u4", "u5"]);
  assert.deepEqual(cardOrder(file), A);
});

test("the stops are the threads not in the diff, then each file in the sidebar's order, the shown file by its cards", () => {
  const state = withThreads({ "notes.txt": 2, "src/b.rs": 1 });
  const { all, top } = stops(loaded(state));
  assert.deepEqual(ids(all), ["u6", "file:notes.txt", ...A, "file:src/b.rs"]);
  assert.equal(top, 2);
  // A file with no thread has no stop.
  assert.deepEqual(ids(stops(loaded()).all), ["u6", ...A]);
});

test("next and previous walk the stops across files and wrap round at both ends", () => {
  const m = loaded(withThreads({ "notes.txt": 2, "src/b.rs": 1 }));
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

test("with nothing focused, next is the shown file's first thread and previous the last one before the file", () => {
  const m = loaded(withThreads({ "notes.txt": 2, "src/b.rs": 1 }));
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
  const m = loaded({ ...empty, files: empty.files.map((f) => ({ ...f, threads: 0 })) }, none);
  assert.equal(jumpStop(m, true), null);
  const { model, cmds } = jump(m, true);
  assert.equal(model.status, "no comments in this review");
  assert.deepEqual(cmds, []);
  assert.equal(model.focus, null);
  const nav = dock(m).actions.filter((b) => b.id === "prev-thread" || b.id === "next-thread");
  assert.deepEqual(nav.map((b) => [b.label, b.off]), [["Previous comment", true], ["Next comment", true]]);
  assert.deepEqual(dock(loaded()).actions.filter((b) => b.id.endsWith("-thread")).map((b) => b.off), [false, false]);
});

test("a thread in the shown file or not in the diff is focused and shown at once, with no load", () => {
  const m = loaded();
  const first = jump(at(m, "u5"), true);
  assert.equal(first.model.focus, "u3");
  assert.deepEqual(first.model.window, { from: "r0", id: m.window.id + 1 });
  assert.deepEqual(first.cmds, []);
  // The orphan: previous from the first card of a.rs, with only one file.
  const back = jump(at(m, "u5"), false);
  assert.equal(back.model.focus, "u6");
  assert.deepEqual(back.model.window, { from: "h:u6", id: m.window.id + 1 });
  // Asking for the same thread again is a new list, so it starts at the top again.
  const alone = loaded({ ...base, orphans: [], files: base.files.map((f) => ({ ...f, threads: 0 })) }, { ...file, threads: [file.threads[0] as FileView["threads"][number]] });
  const once = jump(at(alone, "u1"), true).model;
  const twice = jump(once, true).model;
  assert.equal(twice.focus, "u1");
  assert.deepEqual([once.window, twice.window], [{ from: "r1", id: alone.window.id + 1 }, { from: "r1", id: alone.window.id + 2 }]);
  // It also drops a question that was up and the status.
  const asked = jump({ ...at(m, "u5"), confirm: "u5", status: "old" }, true).model;
  assert.deepEqual([asked.confirm, asked.status], [null, null]);
});

const other = (path: string, threads: FileView["threads"]): FileView => ({ ...file, path, threads });

test("a jump into another file loads it, keeps the target as a pending jump, and lands when the file arrives", () => {
  const state = withThreads({ "src/b.rs": 2 });
  const m = loaded(state);
  const go = jump(at(m, "u4"), true);
  assert.deepEqual(go.model.jump, { path: "src/b.rs", edge: "first" });
  assert.equal(go.model.selected, "src/b.rs");
  const load = go.cmds[0];
  assert.ok(load && load._tag === "Load" && load.selected === "src/b.rs");
  // The focus stays where it was until the file is there.
  assert.equal(go.model.focus, "u4");
  const [t1, t2] = [file.threads[0], file.threads[1]] as [FileView["threads"][number], FileView["threads"][number]];
  const b = other("src/b.rs", [{ ...t2, id: "b2", placement: { kind: "matched", row: 5, start_row: null }, comments: [{ ...t2.comments[0]!, id: "b2" }] }, { ...t1, id: "b1", placement: { kind: "matched", row: 1, start_row: null }, comments: [{ ...t1.comments[0]!, id: "b1" }] }]);
  const done = steps(go.model, { _tag: "Loaded", key: load.key, state, file: b }).model;
  assert.equal(done.jump, null);
  // Going forward lands on the first card on screen, which is not the first the reply lists.
  assert.equal(done.focus, "b1");
  assert.equal(done.window.from, "r0");
  assert.ok(done.window.id > m.window.id); // a new list
});

test("going back into a file lands on its last card", () => {
  const state = withThreads({ "notes.txt": 2 });
  const go = jump(at(loaded(state), "u5"), false);
  assert.deepEqual(go.model.jump, { path: "notes.txt", edge: "last" });
  const load = go.cmds[0];
  assert.ok(load && load._tag === "Load");
  const [t1, t2] = [file.threads[0], file.threads[1]] as [FileView["threads"][number], FileView["threads"][number]];
  const n = other("notes.txt", [{ ...t1, id: "n1", placement: { kind: "matched", row: 1, start_row: null } }, { ...t2, id: "n2", placement: { kind: "matched", row: 4, start_row: null } }]);
  assert.equal(steps(go.model, { _tag: "Loaded", key: load.key, state, file: n }).model.focus, "n2");
});

test("a reply that is not the one the jump asked for leaves it waiting, and a failed load drops it", () => {
  const state = withThreads({ "src/b.rs": 1 });
  const go = jump(at(loaded(state), "u4"), true);
  const stale = steps(go.model, { _tag: "Loaded", key: "an old key", state, file }).model;
  assert.deepEqual(stale.jump, { path: "src/b.rs", edge: "first" });
  assert.equal(stale.focus, "u4");
  const failed = steps(go.model, { _tag: "Failed", op: "load", error: { _tag: "Busy" } }).model;
  assert.equal(failed.jump, null);
});

test("choosing a file or a spec while a jump waits drops the jump", () => {
  const go = jump(at(loaded(withThreads({ "src/b.rs": 1 })), "u4"), true).model;
  assert.equal(steps(go, { _tag: "FileChosen", path: "notes.txt" }).model.jump, null);
  assert.equal(steps(go, { _tag: "SpecChosen", spec: { kind: "branch", base: "main" } }).model.jump, null);
});

test("a file that has lost its threads by the time it arrives says so and keeps the focus", () => {
  const state = withThreads({ "src/b.rs": 1 });
  const go = jump(at(loaded(state), "u4"), true);
  const load = go.cmds[0];
  assert.ok(load && load._tag === "Load");
  const done = steps(go.model, { _tag: "Loaded", key: load.key, state, file: other("src/b.rs", []) }).model;
  assert.equal(done.jump, null);
  assert.equal(done.status, "no comments in src/b.rs");
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
  for (const key of ["cmd-n", "ctrl-n", "x", "escape", "enter"]) assert.equal(panelKey(m, key), null, key);
  const editing: Model = { ...m, editor: { kind: "reply", anchor: null, thread: "u1", comment: null, initial: "" } };
  assert.equal(panelKey(editing, "n"), null);
  assert.equal(panelKey({ ...m, confirm: "u1" }, "n"), null);
  assert.equal(panelKey({ ...m, confirmArchive: true }, "n"), null);
  assert.equal(panelKey({ ...m, picker: [] }, "n"), null);
});

test("the list starts two lines above the line a card hangs from, or at the card when it hangs from no line", () => {
  assert.equal(windowStart(file, "u3"), "r0"); // row 2, never above row 0
  assert.equal(windowStart(file, "u4"), "r4"); // row 6
  assert.equal(windowStart(file, "u5"), "h:u5"); // no row
  assert.equal(windowStart(file, "u6"), "h:u6"); // not in the diff
  assert.equal(windowStart(null, "u1"), "h:u1");
});

test("a window drops what is above its start and puts a first item that shows the whole file again", () => {
  const now = Date.parse("2026-11-09T12:00:00Z");
  const m = loaded();
  const whole = layout(m, now);
  assert.equal(whole.some((i) => i.kind === "more"), false);
  const from = layout({ ...m, window: { from: "r4", id: 3 } }, now);
  const at = whole.findIndex((i) => i.key === "r4");
  assert.deepEqual(from.map((i) => i.key), ["more", ...whole.slice(at).map((i) => i.key)]);
  // A start that is gone, and a start that is the first item, leave the list whole.
  assert.equal(layout({ ...m, window: { from: "r99", id: 3 } }, now).length, whole.length);
  assert.equal(layout({ ...m, window: { from: whole[0]?.key ?? null, id: 3 } }, now).length, whole.length);
  assert.equal(listId({ from: null, id: 4 }), "review-diff-4");
});

test("a click on the first item of a window shows the whole file in a new list", () => {
  assert.deepEqual(itemClick("more"), { _tag: "EarlierClicked" });
  const m = { ...loaded(), window: { from: "r4", id: 3 } };
  const { model, cmds } = steps(m, { _tag: "EarlierClicked" });
  assert.deepEqual(model.window, { from: null, id: 4 });
  assert.deepEqual(cmds, []);
});

test("choosing another file, or a reply for another file, drops the window", () => {
  const m = { ...loaded(), window: { from: "r4", id: 3 } };
  assert.deepEqual(steps(m, { _tag: "FileChosen", path: "src/b.rs" }).model.window, { from: null, id: 4 });
  const { model, cmds } = steps(m, { _tag: "FileChosen", path: "src/b.rs" });
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  const b = steps(model, { _tag: "Loaded", key: load.key, state: base, file: other("src/b.rs", []) }).model;
  assert.equal(b.window.from, null);
  // A reload of the same file keeps it.
  const again = steps(m, { _tag: "Triggered", source: "timer" });
  const reload = again.cmds[0];
  assert.ok(reload && reload._tag === "Load");
  assert.deepEqual(steps(again.model, { _tag: "Loaded", key: reload.key, state: base, file }).model.window, { from: "r4", id: 3 });
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
  assert.deepEqual(sent, [{ _tag: "ThreadJumped", forward: true }, { _tag: "ThreadJumped", forward: false }]);
  fake.host.nextCommand = () => Promise.reject(new Error("gone"));
  assert.equal(await listen({ ...env, closed: () => false }, () => focused++), null);
});
