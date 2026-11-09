// The E: lines of design/graphs/gui-comment.md and gui-thread.md, one test each, against FakeHost.
import assert from "node:assert/strict";
import { test } from "node:test";
import { makeCli } from "../src/cli.ts";
import { decodeFile, decodeState } from "../src/decode.ts";
import { anchorOf, batchOf, confirmKey, confirmRow, dock, editorKey, itemClick, layout, pick, selectionLabel, span } from "../src/rows.ts";
import { run } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import { initModel, update } from "../src/update.ts";
import type { Cmd, FileView, Model, Msg, ReviewError } from "../src/types.ts";
import { exit, fakeHost, fixture, ok } from "./fake-host.ts";
import type { Reply, Setup } from "./fake-host.ts";

const state = decodeState(JSON.parse(fixture("state")));
const file = decodeFile(JSON.parse(fixture("file")));
const DIR = "/home/app/data/herdr-review";

function steps(m: Model, ...msgs: Msg[]): { model: Model; cmds: Cmd[] } {
  const cmds: Cmd[] = [];
  for (const msg of msgs) {
    const [next, out] = update(m, msg);
    m = next;
    cmds.push(...out);
  }
  return { model: m, cmds };
}
const loadsOf = (cmds: Cmd[]) => cmds.filter((c) => c._tag === "Load");
const click = (row: number): Msg => ({ _tag: "RowClicked", row });
const act = (id: string, action: "focus" | "toggle" | "reply" | "edit" | "resolve" | "reopen" | "delete"): Msg => ({ _tag: "ThreadAction", id, action });
const failedMsg = (error: ReviewError, op: "write" | "seen" = "write"): Msg => ({ _tag: "Failed", op, error });

// A model that has loaded src/a.rs and waits.
function loaded(withFile: FileView = file): Model {
  const { model, cmds } = steps(initModel(), { _tag: "Opened", root: "/repo", target: "t7" });
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  return steps(model, { _tag: "Loaded", key: load.key, state, file: withFile }).model;
}
const withEditor = (m: Model, ...msgs: Msg[]): Model => steps(m, ...msgs).model;

// Rows of the fixture: 0 hunk, 1 ctx 1/1, 2 del old 2, 3 add new 2, 4 add new 3, 5 ctx 3/4, 6 ctx 4/5.

test("E: a hunk header row is ignored", () => {
  assert.equal(pick(file, 0), null);
  assert.equal(steps(loaded(), click(0)).model.selection, null);
});

test("a click picks one row: side new for ctx and add, old for del", () => {
  assert.deepEqual(pick(file, 1), { path: "src/a.rs", side: "new", from: 1, to: 1 });
  assert.deepEqual(pick(file, 3), { path: "src/a.rs", side: "new", from: 3, to: 3 });
  assert.deepEqual(pick(file, 2), { path: "src/a.rs", side: "old", from: 2, to: 2 });
  assert.deepEqual(anchorOf(file, pick(file, 2)!), { path: "src/a.rs", old_path: null, side: "old", line: 2, end_line: null });
});

test("a second click extends the selection to a range, from the first row's side", () => {
  const first = steps(loaded(), click(1), click(4)).model;
  assert.deepEqual(first.selection, { path: "src/a.rs", side: "new", from: 1, to: 4 });
  // ctx to add anchors on the new side, and the del row in between has no new line so it is not part of it.
  assert.deepEqual(anchorOf(file, first.selection!), { path: "src/a.rs", old_path: null, side: "new", line: 1, end_line: 3 });
  // A range over a del and an add row keeps the first row's side, and ends at the last row on that side.
  assert.deepEqual(span(file, 2, 4), { path: "src/a.rs", side: "old", from: 2, to: 2 });
  assert.deepEqual(span(file, 3, 2), { path: "src/a.rs", side: "new", from: 3, to: 3 });
  assert.deepEqual(span(file, 1, 2), { path: "src/a.rs", side: "new", from: 1, to: 1 });
  assert.deepEqual(span(file, 4, 1), { path: "src/a.rs", side: "new", from: 1, to: 4 });
  assert.equal(span(file, 0, 3), null);
});

test("a click on the selected row drops it, and a click on a range starts a new selection", () => {
  assert.equal(steps(loaded(), click(3), click(3)).model.selection, null);
  assert.deepEqual(steps(loaded(), click(1), click(4), click(5)).model.selection, { path: "src/a.rs", side: "new", from: 5, to: 5 });
});

test("the selection label says where a comment would point and how many lines it covers", () => {
  assert.equal(selectionLabel(file, span(file, 1, 4)!), "src/a.rs R1-3 (3 lines)");
  assert.equal(selectionLabel(file, pick(file, 2)!), "src/a.rs L2 (1 line)");
});

test("an old-side comment on a renamed file carries old_path", () => {
  const renamed = { ...file, path: "src/b.rs", old_path: "src/old_b.rs" };
  assert.equal(anchorOf(renamed, pick(renamed, 2)!).old_path, "src/old_b.rs");
  assert.equal(anchorOf(renamed, pick(renamed, 3)!).old_path, null);
  assert.deepEqual(JSON.parse(batchOf(anchorOf(renamed, pick(renamed, 2)!), "x")), { comments: [{ path: "src/b.rs", old_path: "src/old_b.rs", side: "old", line: 2, body: "x" }] });
});

test("a file comment writes {path, body} and opens the editor with no selection", () => {
  const m = steps(loaded(), click(1), { _tag: "FileCommentClicked" }).model;
  assert.equal(m.selection, null);
  assert.deepEqual(m.editor?.anchor, { path: "src/a.rs", old_path: null, side: null, line: null, end_line: null });
  assert.deepEqual(JSON.parse(batchOf(m.editor!.anchor!, "Look")), { comments: [{ path: "src/a.rs", body: "Look" }] });
});

test("the Comment button opens the editor on the selection, and rows are not clickable while it is open", () => {
  const m = steps(loaded(), click(1), click(4), { _tag: "CommentClicked" }).model;
  assert.deepEqual(m.editor?.anchor, { path: "src/a.rs", old_path: null, side: "new", line: 1, end_line: 3 });
  assert.equal(steps(m, click(5)).model.selection, m.selection);
  assert.equal(steps(loaded(), { _tag: "CommentClicked" }).model.editor, null);
});

test("E: empty text after trim -> ignored, the editor stays", () => {
  const m = withEditor(loaded(), click(3), { _tag: "CommentClicked" });
  const r = steps(m, { _tag: "EditorSaved", text: " \n\t " });
  assert.deepEqual(r.cmds, []);
  assert.equal(r.model.editor, m.editor);
  assert.equal(r.model.saving, false);
});

test("a save writes the batch for the selection with the model's root and spec, once", () => {
  const m = withEditor(loaded(), click(1), click(4), { _tag: "CommentClicked" });
  const r = steps(m, { _tag: "EditorSaved", text: "Why?" }, { _tag: "EditorSaved", text: "Why?" });
  assert.deepEqual(r.cmds, [{ _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "comment", anchor: m.editor!.anchor!, text: "Why?" } }]);
  assert.equal(r.model.saving, true);
  assert.equal(r.model.editor, m.editor);
});

test("E: Wrote -> editor closed, selection cleared, and the reload shows the new thread", () => {
  const m = withEditor(loaded(), click(1), { _tag: "CommentClicked" }, { _tag: "EditorSaved", text: "Why?" });
  const r = steps(m, { _tag: "Wrote", kind: "comment" });
  assert.equal(r.model.editor, null);
  assert.equal(r.model.selection, null);
  assert.equal(r.model.saving, false);
  assert.equal(loadsOf(r.cmds).length, 1);
});

test("E: Busy, BackendFailed, BadRequest -> the editor keeps its draft, and BadRequest reloads once the editor closes", () => {
  const m = withEditor(loaded(), click(1), { _tag: "CommentClicked" }, { _tag: "EditorSaved", text: "Why?" });
  for (const error of [{ _tag: "Busy" }, { _tag: "BackendFailed", code: 1, stderr: "disk full" }, { _tag: "BadRequest", stderr: "invalid batch: line 99 is past the end" }] as ReviewError[]) {
    const r = steps(m, failedMsg(error));
    assert.equal(r.model.editor, m.editor, `${error._tag} keeps the same editor, so its textarea is not replaced`);
    assert.equal(r.model.saving, false);
    assert.deepEqual(r.cmds.filter((c) => c._tag === "Write"), []);
    assert.equal(loadsOf(r.cmds).length, 0);
  }
  const bad = steps(m, failedMsg({ _tag: "BadRequest", stderr: "invalid batch" }));
  assert.equal(bad.model.status, "invalid batch");
  assert.equal(bad.model.pending, true);
  assert.equal(loadsOf(steps(bad.model, { _tag: "EditorCancelled" }).cmds).length, 1);
});

test("a selection survives a reload that leaves its rows alone, and goes when they change", () => {
  const picked = steps(loaded(), click(3)).model;
  const [busy, cmds] = update(picked, { _tag: "Triggered", source: "timer" });
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  assert.deepEqual(update(busy, { _tag: "Loaded", key: load.key, state, file: { ...file } })[0].selection, picked.selection);
  const changed = { ...file, rows: file.rows.map((r, i) => (i === 3 ? { ...r, text: "other();" } : r)) };
  assert.equal(update(busy, { _tag: "Loaded", key: load.key, state, file: changed })[0].selection, null);
});

test("the cards sit under their rows, the file comment before the rows, and the orphans first", () => {
  const items = layout({ ...loaded(), expanded: new Set(["u2"]) }, Date.parse("2026-11-09T10:00:30Z"));
  assert.deepEqual(items.slice(0, 3).map((i) => i.key), ["l:orphans", "h:u6", "c:u6"]);
  const keys = items.map((i) => i.key);
  assert.ok(keys.indexOf("h:u5") < keys.indexOf("r0"));
  assert.ok(keys.indexOf("r2") < keys.indexOf("h:u3") && keys.indexOf("h:u3") < keys.indexOf("r3"));
  assert.ok(keys.indexOf("r3") < keys.indexOf("h:u1") && keys.indexOf("h:u1") < keys.indexOf("r4"));
  assert.deepEqual(keys.slice(keys.indexOf("r4"), keys.indexOf("r5")), ["r4", "h:u2", "c:u2", "c:a1"]);
  assert.ok(keys.indexOf("r6") < keys.indexOf("h:u4"));
});

test("a card says who, how old, where, and the markers", () => {
  const items = layout({ ...loaded(), expanded: new Set(["u2"]) }, Date.parse("2026-11-09T10:00:30Z"));
  const head = (id: string) => items.find((i) => i.kind === "head" && i.id === id);
  assert.deepEqual(head("u1"), { kind: "head", key: "h:u1", id: "u1", tone: "user", title: "● Your note · now · src/a.rs R2", badges: ["unsent"], quote: null, collapsible: false, open: true, focused: false, last: false });
  const u2 = head("u2");
  assert.ok(u2 && u2.kind === "head");
  assert.equal(u2.title, "✓ Your note · now · src/a.rs R2-3");
  assert.deepEqual(u2.badges, ["resolved by claude", "new"]);
  assert.equal(u2.tone, "resolved");
  const u4 = head("u4");
  assert.ok(u4 && u4.kind === "head");
  assert.deepEqual([u4.badges, u4.quote], [["outdated", "unsent"], "text that is gone"]);
  const reply = items.find((i) => i.kind === "body" && i.id === "a1");
  assert.ok(reply && reply.kind === "body");
  assert.equal(reply.meta, "claude · now");
  assert.equal(reply.last, true);
});

test("a resolved thread is collapsed until the user opens it", () => {
  const keys = layout(loaded(), 0).map((i) => i.key);
  assert.ok(keys.includes("h:u2") && !keys.includes("c:u2") && !keys.includes("c:a1"));
});

// --- threads ---

test("expanding a resolved thread that is new writes seen, once, and collapsing writes nothing", () => {
  const open = steps(loaded(), act("u2", "toggle"));
  assert.deepEqual(open.cmds, [{ _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "seen", id: "u2" } }]);
  assert.ok(open.model.expanded.has("u2"));
  assert.equal(open.model.focus, "u2");
  const shut = steps(open.model, act("u2", "toggle"));
  assert.deepEqual(shut.cmds, []);
  assert.equal(shut.model.expanded.has("u2"), false);
});

test("a seen for a thread that is not new is never sent", () => {
  const calm = { ...file, threads: file.threads.map((t) => (t.id === "u2" ? { ...t, is_new: false } : t)) };
  assert.deepEqual(steps(loaded(calm), act("u2", "toggle")).cmds, []);
  assert.deepEqual(steps(loaded(), act("u1", "toggle")).cmds, []);
});

test("E: seen failed of any kind -> nothing shown, the marker stays until the next load", () => {
  for (const error of [{ _tag: "Busy" }, { _tag: "BackendFailed", code: 1, stderr: "x" }, { _tag: "BadRequest", stderr: "unknown id" }] as ReviewError[]) {
    const m = loaded();
    const r = steps(m, failedMsg(error, "seen"));
    assert.equal(r.model.status, null);
    assert.equal(r.model.banner, null);
    assert.deepEqual(r.cmds, []);
  }
});

test("reply opens the editor for the thread, and a save writes a reply", () => {
  const m = steps(loaded(), act("u1", "reply")).model;
  assert.deepEqual(m.editor, { kind: "reply", anchor: null, thread: "u1", comment: null, initial: "" });
  assert.deepEqual(steps(m, { _tag: "EditorSaved", text: "ok" }).cmds[0], { _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "reply", id: "u1", text: "ok" } });
});

test("edit opens the editor with the comment's text, only on the user's own comment", () => {
  const m = steps(loaded(), act("u2", "edit")).model;
  assert.deepEqual(m.editor, { kind: "edit", anchor: null, thread: "u2", comment: "u2", initial: "Why two lines?" });
  assert.deepEqual(steps(m, { _tag: "EditorSaved", text: "Why?" }).cmds[0], { _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "edit", id: "u2", text: "Why?" } });
  assert.equal(steps(loaded(), act("a1", "edit")).model.editor, null);
});

test("resolve only on an open thread, reopen only on a resolved one", () => {
  assert.equal(steps(loaded(), act("u1", "resolve")).cmds.length, 1);
  assert.deepEqual(steps(loaded(), act("u1", "reopen")).cmds, []);
  assert.equal(steps(loaded(), act("u2", "reopen")).cmds.length, 1);
  assert.deepEqual(steps(loaded(), act("u2", "resolve")).cmds, []);
  // A click on a reply's id acts on its thread.
  assert.deepEqual(steps(loaded(), act("a1", "reopen")).cmds[0], { _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "reopen", id: "u2" } });
});

const delWrite: Cmd = { _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "delete", id: "u1" } };

test("a click on Delete only asks, and the row names what goes with a Delete and a Cancel button", () => {
  const before = steps(loaded(), act("u1", "focus")).model;
  const asked = steps(before, act("u1", "delete"));
  assert.deepEqual(asked.cmds, []);
  assert.equal(asked.model.confirm, "u1");
  const row = confirmRow(asked.model);
  assert.equal(row?.sentence, "Delete this comment?");
  assert.deepEqual(row?.buttons.map((b) => [b.label, b.danger, b.msg._tag]), [["Delete", true, "DeleteConfirmed"], ["Cancel", false, "ConfirmCancelled"]]);
  // A root with a reply says its replies go too, a reply and a root alone do not.
  const withReply = steps(loaded(), act("u2", "delete")).model;
  assert.equal(confirmRow(withReply)?.sentence, "Delete this comment? Its replies go too.");
  assert.equal(confirmRow(steps(loaded(), act("a1", "delete")).model), null); // an agent's comment is never asked about
});

test("a click on a Delete in the row writes the delete, and only that button does", () => {
  const asked = steps(loaded(), act("u1", "delete")).model;
  const yes = confirmRow(asked)?.buttons[0]?.msg;
  assert.ok(yes);
  const out = steps(asked, yes);
  assert.deepEqual(out.cmds, [delWrite]);
  assert.equal(out.model.confirm, null);
  // Without the question, or for another comment, or for an agent's comment, nothing is written.
  assert.deepEqual(steps(loaded(), { _tag: "DeleteConfirmed", id: "u1" }).cmds, []);
  assert.deepEqual(steps(asked, { _tag: "DeleteConfirmed", id: "u3" }).cmds, []);
  assert.deepEqual(steps(loaded(), act("a1", "delete"), { _tag: "DeleteConfirmed", id: "a1" }).cmds, []);
});

test("Cancel and Esc after a click on Delete leave the model as it was and write nothing", () => {
  const before = steps(loaded(), act("u1", "focus")).model;
  const asked = steps(before, act("u1", "delete")).model;
  const cancel = confirmRow(asked)?.buttons[1]?.msg;
  assert.ok(cancel);
  for (const msg of [cancel, confirmKey("escape")]) {
    assert.ok(msg);
    const out = steps(asked, msg);
    assert.deepEqual(out.cmds, []);
    assert.deepEqual(out.model, before);
  }
  assert.equal(confirmKey("enter"), null);
});

test("the question goes when another comment is focused, and a reload keeps it while the comment is there", () => {
  const asked = steps(loaded(), act("u1", "delete")).model;
  assert.equal(steps(asked, act("u3", "focus")).model.confirm, null);
  assert.equal(steps(asked, { _tag: "Loaded", key: asked.key, state, file }).model.confirm, "u1");
  const gone = { ...file, threads: file.threads.filter((t) => t.id !== "u1") };
  assert.equal(steps(asked, { _tag: "Loaded", key: asked.key, state, file: gone }).model.confirm, null);
});

test("the edit and delete buttons are absent on an agent's comment", () => {
  const labels = (id: string) => dock(steps(loaded(), act(id, "focus")).model).buttons.map((b) => b.label);
  assert.deepEqual(labels("u1"), ["Reply", "Resolve", "Edit", "Delete", "Comment on file"]);
  assert.deepEqual(labels("a1"), ["Reply", "Reopen", "Comment on file"]);
});

test("an action while the editor is open waits, and focusing a comment does not", () => {
  const m = steps(loaded(), act("u1", "reply")).model;
  const r = steps(m, act("u3", "resolve"));
  assert.deepEqual(r.cmds, []);
  assert.equal(r.model.status, "finish or cancel the comment first");
  assert.equal(steps(m, act("u3", "focus")).model.focus, "u3");
});

test("E: Busy -> status, BadRequest unknown id or not allowed -> status and one reload", () => {
  const busy = steps(loaded(), act("u1", "resolve"), failedMsg({ _tag: "Busy" }));
  assert.equal(busy.model.status, "review is busy, try again");
  for (const stderr of ["unknown id u9: open ids are u1", "not allowed: a1 is an agent's comment"]) {
    const r = steps(loaded(), act("u1", "resolve"), failedMsg({ _tag: "BadRequest", stderr }));
    assert.equal(r.model.status, stderr);
    assert.equal(loadsOf(r.cmds).length, 1);
  }
});

test("E: a failed reply keeps the editor, the text lives on in the textarea the same editor owns", () => {
  const m = steps(loaded(), act("u1", "reply"), { _tag: "EditorSaved", text: "ok" }).model;
  assert.equal(m.saving, true);
  const r = steps(m, failedMsg({ _tag: "BackendFailed", code: 1, stderr: "disk full" })).model;
  assert.equal(r.editor, m.editor);
  assert.equal(r.status, "disk full");
  assert.equal(r.saving, false);
});

test("the Wrote of a seen leaves an editor opened since alone, and the Wrote of a reply closes it", () => {
  const m = steps(loaded(), act("u1", "reply")).model;
  assert.equal(steps(m, { _tag: "Wrote", kind: "seen" }).model.editor, m.editor);
  assert.equal(steps(m, { _tag: "Wrote", kind: "reply" }).model.editor, null);
});

test("Esc closes the editor and keeps the selection, the next one drops it", () => {
  const m = steps(loaded(), click(1), { _tag: "CommentClicked" }).model;
  const a = steps(m, { _tag: "EditorCancelled" }).model;
  assert.equal(a.editor, null);
  assert.ok(a.selection);
  assert.equal(steps(a, { _tag: "EditorCancelled" }).model.selection, null);
});

test("keys: Ctrl+S and Cmd+S save the text, Esc cancels, the rest are the textarea's", () => {
  assert.deepEqual(editorKey("ctrl-s", "t"), { _tag: "EditorSaved", text: "t" });
  assert.deepEqual(editorKey("cmd-s", "t"), { _tag: "EditorSaved", text: "t" });
  assert.deepEqual(editorKey("escape", "t"), { _tag: "EditorCancelled" });
  assert.equal(editorKey("a", "t"), null);
});

test("a click on a list item names its message", () => {
  assert.deepEqual(itemClick("r12"), { _tag: "RowClicked", row: 12 });
  assert.deepEqual(itemClick("h:u3"), { _tag: "ThreadAction", id: "u3", action: "toggle" });
  assert.deepEqual(itemClick("c:a1"), { _tag: "ThreadAction", id: "a1", action: "focus" });
  assert.equal(itemClick("l:orphans"), null);
});

// --- the runner: argv, the body file, the error rows ---

function env(setup: Setup = {}) {
  const fake = fakeHost(setup);
  const e: Env = { host: fake.host, cli: makeCli(fake.host, null), dispatch: () => {}, closed: () => false };
  return { ...fake, env: e };
}
const writeCmd = (op: Extract<Cmd, { _tag: "Write" }>["op"], spec: Extract<Cmd, { _tag: "Write" }>["spec"] = { kind: "worktree" }): Cmd => ({ _tag: "Write", root: "/repo", spec, op });
const execs = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "exec").map((c) => ({ command: c.args[0] as string, argv: c.args[1] as string[], options: c.args[2] }));
const quiet = (command: string, args: string[]): Reply => (command === "mkdir" || command === "chmod" || args[0] === "comment" ? ok("") : exit(2, "unexpected call"));
const files = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "writeTextFile").map((c) => ({ path: c.args[0] as string, text: c.args[1] as string, expected: c.args[2] }));

test("a new comment writes the batch to a file in the private folder and passes only its path", async () => {
  const t = env({ exec: quiet });
  const anchor = anchorOf(file, span(file, 1, 4)!);
  const msg = await run(t.env, writeCmd({ kind: "comment", anchor, text: "Why `$(rm -rf /)`?" }, { kind: "branch", base: "origin/main" }));
  assert.deepEqual(msg, { _tag: "Wrote", kind: "comment" });
  const [written] = files(t.calls);
  assert.ok(written);
  assert.match(written.path, new RegExp(`^${DIR}/[a-z0-9]+\\.txt$`));
  assert.equal(written.expected, null);
  assert.deepEqual(JSON.parse(written.text), { comments: [{ path: "src/a.rs", side: "new", line: 1, end_line: 3, body: "Why `$(rm -rf /)`?" }] });
  const calls = execs(t.calls);
  assert.deepEqual(calls.slice(0, 2).map((c) => [c.command, c.argv]), [["mkdir", ["-p", DIR]], ["chmod", ["700", DIR]]]);
  const apply = calls[2];
  assert.ok(apply);
  assert.deepEqual(apply.argv, ["comment", "apply", "--as", "user", "--file", written.path, "--repo", "/repo", "--spec", "branch", "--base", "origin/main"]);
  assert.deepEqual(apply.options, { timeout: 30000 });
  for (const c of calls) assert.ok(!c.argv.join(" ").includes("rm -rf"), "the body is never on argv");
});

test("the six calls and their flags: --as user on reply, resolve and reopen only", async () => {
  const t = env({ exec: quiet });
  await run(t.env, writeCmd({ kind: "reply", id: "u1", text: "ok" }));
  await run(t.env, writeCmd({ kind: "edit", id: "u1", text: "ok" }));
  await run(t.env, writeCmd({ kind: "resolve", id: "u1" }));
  await run(t.env, writeCmd({ kind: "reopen", id: "u1" }));
  await run(t.env, writeCmd({ kind: "delete", id: "u1" }));
  await run(t.env, writeCmd({ kind: "seen", id: "u1" }));
  const comment = execs(t.calls).filter((c) => c.argv[0] === "comment").map((c) => c.argv);
  const [reply, edit] = files(t.calls);
  assert.deepEqual(comment, [
    ["comment", "reply", "--as", "user", "u1", "--body-file", reply!.path, "--repo", "/repo"],
    ["comment", "edit", "u1", "--body-file", edit!.path, "--repo", "/repo"],
    ["comment", "resolve", "--as", "user", "u1", "--repo", "/repo"],
    ["comment", "reopen", "--as", "user", "u1", "--repo", "/repo"],
    ["comment", "delete", "u1", "--repo", "/repo"],
    ["comment", "seen", "u1", "--repo", "/repo"],
  ]);
  assert.equal(reply!.text, "ok");
  assert.equal(files(t.calls).length, 2);
});

test("the body file is made once per write, in a folder closed with chmod", async () => {
  const t = env({ exec: quiet });
  await run(t.env, writeCmd({ kind: "reply", id: "u1", text: "a" }));
  await run(t.env, writeCmd({ kind: "reply", id: "u1", text: "b" }));
  const [a, b] = files(t.calls);
  assert.notEqual(a!.path, b!.path);
});

test("E: writeTextFile rejects -> Failed, and nothing reaches the store", async () => {
  const t = env({ exec: quiet });
  t.host.writeTextFile = () => Promise.reject(new Error("read-only"));
  const msg = await run(t.env, writeCmd({ kind: "reply", id: "u1", text: "a" }));
  assert.ok(msg && msg._tag === "Failed" && msg.op === "write" && msg.error._tag === "BackendFailed");
  assert.equal(execs(t.calls).filter((c) => c.argv[0] === "comment").length, 0);
});

test("E: chmod rejected -> the write stops before any file is written", async () => {
  const t = env({ exec: (command) => (command === "chmod" ? exit(1, "Operation not permitted") : ok("")) });
  const msg = await run(t.env, writeCmd({ kind: "reply", id: "u1", text: "a" }));
  assert.deepEqual(msg, { _tag: "Failed", op: "write", error: { _tag: "BackendFailed", code: -1, stderr: "Operation not permitted" } });
  assert.deepEqual(files(t.calls), []);
});

test("E: BinaryMissing and TooOld -> the body file is overwritten with nothing, and Failed", async () => {
  const gone = env({ exec: (command) => (command === "mkdir" || command === "chmod" ? ok("") : "reject") });
  const a = await run(gone.env, writeCmd({ kind: "reply", id: "u1", text: "secret" }));
  assert.deepEqual(a, { _tag: "Failed", op: "write", error: { _tag: "BinaryMissing" } });
  const [made, emptied] = files(gone.calls);
  assert.equal(made!.text, "secret");
  assert.deepEqual(emptied, { path: made!.path, text: "", expected: "secret" });
  const old = env({ exec: (command) => (command === "mkdir" || command === "chmod" ? ok("") : exit(2, "unknown comment command 'edit'")) });
  const b = await run(old.env, writeCmd({ kind: "edit", id: "u1", text: "secret" }));
  assert.deepEqual(b, { _tag: "Failed", op: "write", error: { _tag: "TooOld" } });
  assert.equal(files(old.calls)[1]?.text, "");
});

test("E: Busy, BackendFailed and BadRequest of a write -> Failed with the error, and the file is left to the CLI", async () => {
  for (const [reply, error] of [
    [exit(1, "review is busy, try again"), { _tag: "Busy" }],
    [exit(1, "disk full"), { _tag: "BackendFailed", code: 1, stderr: "disk full" }],
    [exit(2, "invalid batch: entry 0: line 99 is past the end"), { _tag: "BadRequest", stderr: "invalid batch: entry 0: line 99 is past the end" }],
  ] as [Reply, ReviewError][]) {
    const t = env({ exec: (command, args) => (args[0] === "comment" ? reply : ok("")) });
    const msg = await run(t.env, writeCmd({ kind: "comment", anchor: anchorOf(file, pick(file, 3)!), text: "x" }));
    assert.deepEqual(msg, { _tag: "Failed", op: "write", error });
    assert.equal(files(t.calls).length, 1);
  }
});

test("a failed seen is an op of its own, so the reducer can keep quiet", async () => {
  const t = env({ exec: () => exit(1, "boom") });
  const msg = await run(t.env, writeCmd({ kind: "seen", id: "u2" }));
  assert.ok(msg && msg._tag === "Failed" && msg.op === "seen");
});

test("a resolve, reopen, delete and seen touch no file and no folder", async () => {
  const t = env({ exec: quiet });
  await run(t.env, writeCmd({ kind: "resolve", id: "u1" }));
  await run(t.env, writeCmd({ kind: "seen", id: "u1" }));
  assert.deepEqual(files(t.calls), []);
  assert.deepEqual(execs(t.calls).map((c) => c.command), ["/home/app/mods/herdr-review/bin/herdr-review", "/home/app/mods/herdr-review/bin/herdr-review"]);
});
