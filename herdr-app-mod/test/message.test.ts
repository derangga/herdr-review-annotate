// The E: lines of design/graphs/gui-message.md, one test each, against FakeHost.
import assert from "node:assert/strict";
import { test } from "node:test";
import { makeCli } from "../src/cli.ts";
import { decodeMessage, decodeMessageSent } from "../src/decode.ts";
import { messageItems, NO_AGENT, sendButton, viewButton } from "../src/message.ts";
import { editorTitle } from "../src/rows.ts";
import { run } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import type { Cmd, MessageReply, Model, Msg, ReviewError } from "../src/types.ts";
import { exit, fakeHost, ok } from "./fake-host.ts";
import type { Reply, Setup } from "./fake-host.ts";
import { loadedModel, steps } from "./model.ts";

const reply = (id = "m1", lines = ["# Plan", "Step one", "Step two", "Step three", "Done"]): MessageReply => ({ schema: 1, agent: "claude-code", status: "idle", id, lines });
const loadedMsg = (r: MessageReply = reply()): Msg => ({ _tag: "MessageLoaded", reply: r });
const hover = (line: number, on = true): Msg => ({ _tag: "MessageHovered", line, on });
const press = (line: number): Msg => ({ _tag: "MessagePressed", line });
const release: Msg = { _tag: "MessageReleased" };
const plus = (line: number): Msg => ({ _tag: "MessagePlusClicked", line });
const save = (text: string): Msg => ({ _tag: "EditorSaved", text });
const failedMsg = (error: ReviewError, op: "message" | "message-send"): Msg => ({ _tag: "Failed", op, error });

// The panel with the message view open and the message read.
const shown = (r: MessageReply = reply()): Model => steps(loadedModel(), { _tag: "ViewChosen", view: "message" }, loadedMsg(r)).model;
// A drag over the lines `a` to `b`, then the release.
const drag = (a: number, ...through: number[]): Msg[] => [press(a), ...through.map((l) => hover(l)), release];
// A comment on lines a to b with `text`.
const comment = (m: Model, a: number, b: number, text: string): Model => steps(m, ...drag(a, b), plus(b), save(text)).model;

test("the header button switches the main area and back, and opening it reads the message of the target", () => {
  const m = loadedModel();
  assert.equal(m.view, "diff");
  assert.deepEqual(viewButton(m).msg, { _tag: "ViewChosen", view: "message" });
  const opened = steps(m, { _tag: "ViewChosen", view: "message" });
  assert.equal(opened.model.view, "message");
  assert.equal(opened.model.message.loading, true);
  assert.deepEqual(opened.cmds, [{ _tag: "ReadMessage", target: "t7" }]);
  assert.deepEqual(viewButton(opened.model).msg, { _tag: "ViewChosen", view: "diff" });
  const back = steps(opened.model, { _tag: "ViewChosen", view: "diff" });
  assert.equal(back.model.view, "diff");
  assert.deepEqual(back.cmds, []);
  assert.deepEqual(steps(m, { _tag: "ViewChosen", view: "diff" }).cmds, []);
});

test("E: no target -> the message view says there is no agent and reads nothing", () => {
  const out = steps(loadedModel(undefined, undefined, null), { _tag: "ViewChosen", view: "message" });
  assert.deepEqual(out.cmds, []);
  assert.equal(out.model.view, "message");
  assert.equal(out.model.message.status, NO_AGENT);
  assert.equal(out.model.message.loading, false);
});

test("the message arrives and every line is an item with its number", () => {
  const m = shown();
  assert.equal(m.message.loading, false);
  assert.equal(m.message.data?.agent, "claude-code");
  const lines = messageItems(m.message).filter((i) => i.kind === "line");
  assert.deepEqual(
    lines.map((i) => (i.kind === "line" ? [i.n, i.text] : null)),
    [
      [1, "# Plan"],
      [2, "Step one"],
      [3, "Step two"],
      [4, "Step three"],
      [5, "Done"],
    ],
  );
});

test("the hover marks one line, and a leave only counts for the line the model has", () => {
  const m = shown();
  const on = steps(m, hover(2)).model;
  assert.equal(on.message.hover, 2);
  assert.equal(steps(on, hover(3, false)).model.message.hover, 2);
  assert.equal(steps(on, hover(2, false)).model.message.hover, null);
  assert.equal(steps(m, hover(99)).model.message.hover, null); // a line the message does not have
});

test("a drag over lines selects them, from either end, and a click clears the selection", () => {
  const m = shown();
  assert.deepEqual(steps(m, ...drag(2, 3, 4)).model.message.selection, { from: 2, to: 4 });
  assert.deepEqual(steps(m, ...drag(4, 3, 2)).model.message.selection, { from: 2, to: 4 });
  const picked = steps(m, ...drag(2, 3)).model;
  assert.equal(steps(picked, ...drag(5)).model.message.selection, null);
  assert.equal(steps(picked, ...drag(5)).model.message.drag, null);
});

test("the [+] opens the editor on the selection when the line is in it, else on the line alone", () => {
  const picked = steps(shown(), ...drag(2, 3)).model;
  const inside = steps(picked, plus(3)).model;
  assert.deepEqual(inside.message.selection, { from: 2, to: 3 });
  assert.equal(inside.editor?.kind, "comment");
  assert.equal(editorTitle(inside.editor!), "Comment on lines 2-3");
  const outside = steps(picked, plus(5)).model;
  assert.deepEqual(outside.message.selection, { from: 5, to: 5 });
  assert.equal(editorTitle(outside.editor!), "Comment on line 5");
});

test("save keeps the comment in the model, writes nothing, and puts a card under its last line", () => {
  const out = steps(shown(), ...drag(2, 3), plus(3), save("  Why two steps?  "));
  assert.deepEqual(out.cmds, []); // nothing is written or sent
  assert.equal(out.model.editor, null);
  assert.equal(out.model.message.selection, null);
  assert.deepEqual(out.model.message.comments, [{ id: 1, start: 2, end: 3, body: "Why two steps?" }]);
  const items = messageItems(out.model.message);
  assert.deepEqual(
    items.map((i) => (i.kind === "line" ? i.n : `card${i.id}`)),
    [1, 2, 3, "card1", 4, 5],
  );
  assert.deepEqual(sendButton(out.model), { label: "Send 1", off: false });
});

test("E: an empty save asks for nothing and keeps the editor", () => {
  const m = steps(shown(), plus(2)).model;
  const out = steps(m, save("   "));
  assert.equal(out.model.editor !== null, true);
  assert.deepEqual(out.model.message.comments, []);
});

test("cancel closes the editor and leaves no comment", () => {
  const m = steps(shown(), plus(2), { _tag: "EditorCancelled" }).model;
  assert.equal(m.editor, null);
  assert.deepEqual(m.message.comments, []);
});

test("edit opens the editor on the comment's text, save changes it, and the card stays where it was", () => {
  const m = comment(shown(), 2, 3, "first");
  const editing = steps(m, { _tag: "MessageCommentAction", id: 1, action: "edit" }).model;
  assert.equal(editing.editor?.initial, "first");
  assert.equal(editorTitle(editing.editor!), "Edit comment on lines 2-3");
  const done = steps(editing, save("second")).model;
  assert.deepEqual(done.message.comments, [{ id: 1, start: 2, end: 3, body: "second" }]);
  assert.equal(done.editor, null);
});

test("delete removes the comment at once, and no other", () => {
  const m = comment(comment(shown(), 2, 2, "a"), 4, 4, "b");
  const out = steps(m, { _tag: "MessageCommentAction", id: 1, action: "delete" }).model;
  assert.deepEqual(
    out.message.comments.map((c) => [c.id, c.body]),
    [[2, "b"]],
  );
  assert.deepEqual(steps(out, { _tag: "MessageCommentAction", id: 9, action: "delete" }).model, out);
});

test("with the editor open, another comment, a switch of view and a send wait", () => {
  const m = steps(comment(shown(), 2, 2, "a"), plus(3)).model;
  assert.equal(steps(m, plus(4)).model.message.status, "finish or cancel the comment first");
  assert.equal(steps(m, { _tag: "ViewChosen", view: "diff" }).model.view, "message");
  assert.deepEqual(steps(m, { _tag: "MessageSendClicked" }).cmds, []);
  assert.deepEqual(steps(m, { _tag: "MessageCommentAction", id: 1, action: "delete" }).model.message.comments.length, 1);
});

test("send asks for one command with all the comments and the id of the message they were written on", () => {
  const m = comment(comment(shown(), 2, 3, "a"), 5, 5, "b");
  const out = steps(m, { _tag: "MessageSendClicked" });
  assert.deepEqual(out.cmds, [
    {
      _tag: "SendMessage",
      target: "t7",
      id: "m1",
      comments: [
        { start: 2, end: 3, body: "a" },
        { start: 5, end: 5, body: "b" },
      ],
    },
  ] satisfies Cmd[]);
  assert.equal(out.model.message.sending, true);
  // A second click while it runs asks for nothing.
  assert.deepEqual(steps(out.model, { _tag: "MessageSendClicked" }).cmds, []);
  assert.deepEqual(sendButton(out.model), { label: "Send 2", off: true });
});

test("E: send with no comments says so and asks for nothing", () => {
  const out = steps(shown(), { _tag: "MessageSendClicked" });
  assert.deepEqual(out.cmds, []);
  assert.equal(out.model.message.status, "nothing to send");
  assert.equal(sendButton(shown()).off, true);
});

test("a send that succeeds clears the comments and says it was sent", () => {
  const m = steps(comment(shown(), 2, 3, "a"), { _tag: "MessageSendClicked" }).model;
  const out = steps(m, { _tag: "MessageSent", sent: 1 }).model;
  assert.deepEqual(out.message.comments, []);
  assert.equal(out.message.sending, false);
  assert.equal(out.message.status, "sent 1 comment to claude-code");
  assert.equal(out.message.data?.id, "m1"); // the message stays on screen
});

test("E: a send that is refused keeps the comments and shows the reason, and the diff's status is left alone", () => {
  const m = steps(comment(shown(), 2, 3, "a"), { _tag: "MessageSendClicked" }).model;
  const stderr = "The agent is waiting on a prompt. Nothing was sent; your comments are still unsent.\n";
  const out = steps(m, failedMsg({ _tag: "BackendFailed", code: 1, stderr }, "message-send")).model;
  assert.equal(out.message.comments.length, 1);
  assert.equal(out.message.sending, false);
  assert.equal(out.message.status, "The agent is waiting on a prompt. Nothing was sent; your comments are still unsent.");
  assert.equal(out.status, m.status);
  assert.equal(out.banner, null);
  // The user can send again.
  assert.equal(steps(out, { _tag: "MessageSendClicked" }).cmds.length, 1);
});

test("E: a refused request shows its first line, and a binary too old says to update", () => {
  const m = shown();
  const usage = steps(m, failedMsg({ _tag: "BadRequest", stderr: "the agent's message changed, refresh\nusage: herdr-review ..." }, "message-send")).model;
  assert.equal(usage.message.status, "the agent's message changed, refresh");
  const old = steps(m, failedMsg({ _tag: "TooOld" }, "message")).model;
  assert.equal(old.message.status, "update herdr-review");
  assert.equal(old.stopped, false); // the diff keeps working with an older binary
});

test("E: a failed read keeps the message and the comments", () => {
  const m = comment(shown(), 2, 2, "a");
  const out = steps(m, { _tag: "MessageRefreshed" }, failedMsg({ _tag: "BackendFailed", code: 1, stderr: "Herdr has no session for this agent yet.\n" }, "message")).model;
  assert.equal(out.message.loading, false);
  assert.equal(out.message.data?.id, "m1");
  assert.equal(out.message.comments.length, 1);
  assert.equal(out.message.status, "Herdr has no session for this agent yet.");
});

test("refresh reads again and keeps the comments when the message has the same id", () => {
  const m = comment(shown(), 2, 3, "a");
  const asked = steps(m, { _tag: "MessageRefreshed" });
  assert.deepEqual(asked.cmds, [{ _tag: "ReadMessage", target: "t7" }]);
  assert.deepEqual(steps(asked.model, { _tag: "MessageRefreshed" }).cmds, []); // one read at a time
  const out = steps(asked.model, loadedMsg(reply("m1"))).model;
  assert.deepEqual(out.message.comments, m.message.comments);
  assert.equal(out.message.status, null);
});

test("refresh drops the comments when the message has another id, and says so", () => {
  const m = comment(comment(shown(), 2, 3, "a"), 5, 5, "b");
  const out = steps(m, { _tag: "MessageRefreshed" }, loadedMsg(reply("m2", ["only", "two"]))).model;
  assert.deepEqual(out.message.comments, []);
  assert.equal(out.message.selection, null);
  assert.equal(out.message.status, "the agent wrote a new message, 2 comments dropped");
  assert.deepEqual(out.message.data?.lines, ["only", "two"]);
  // With no comments there is nothing to say.
  assert.equal(steps(shown(), { _tag: "MessageRefreshed" }, loadedMsg(reply("m2"))).model.message.status, null);
});

test("a new message closes an editor that was open on the old lines", () => {
  const m = steps(shown(), plus(2)).model;
  const out = steps(m, loadedMsg(reply("m2"))).model;
  assert.equal(out.editor, null);
});

test("the diff view is as it was after a visit to the message", () => {
  const m = loadedModel();
  const back = steps(m, { _tag: "ViewChosen", view: "message" }, loadedMsg(), ...drag(2, 3), { _tag: "ViewChosen", view: "diff" }).model;
  assert.deepEqual({ ...back, view: m.view, message: m.message }, m);
  assert.equal(back.message.comments.length, 0);
});

// decode

test("the message reply decodes, and a reply of another shape is a contract mismatch", () => {
  assert.deepEqual(decodeMessage({ schema: 1, agent: "claude", status: "working", id: "m1", lines: ["a", ""] }), { schema: 1, agent: "claude", status: "working", id: "m1", lines: ["a", ""] });
  assert.deepEqual(decodeMessageSent({ schema: 1, sent: 2 }), { schema: 1, sent: 2 });
  assert.throws(
    () => decodeMessage({ schema: 1, agent: "claude", status: "idle", id: "m1", lines: [1] }),
    (e) => (e as ReviewError)._tag === "ContractMismatch",
  );
  assert.throws(
    () => decodeMessage({ schema: 2, agent: "claude", status: "idle", id: "m1", lines: [] }),
    (e) => (e as ReviewError)._tag === "ContractMismatch",
  );
  assert.throws(
    () => decodeMessageSent({ schema: 1 }),
    (e) => (e as ReviewError)._tag === "ContractMismatch",
  );
});

// the runner

function env(setup: Setup = {}) {
  const fake = fakeHost(setup);
  const e: Env = { host: fake.host, cli: makeCli(fake.host, null), dispatch: () => {}, closed: () => false };
  return { ...fake, env: e };
}
const execs = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "exec").map((c) => ({ command: c.args[0] as string, argv: c.args[1] as string[], options: c.args[2] }));
const files = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "writeTextFile").map((c) => ({ path: c.args[0] as string, text: c.args[1] as string }));
const answers =
  (read: Reply, send: Reply) =>
  (command: string, args: string[]): Reply =>
    command === "mkdir" || command === "chmod" ? ok("") : args[0] === "message" && args[1] === "--json" ? read : args[0] === "message" && args[1] === "send" ? send : exit(2, "unexpected call");

test("a read runs message --json for the target and answers MessageLoaded", async () => {
  const t = env({ exec: answers(ok(JSON.stringify(reply())), ok("")) });
  const msg = await run(t.env, { _tag: "ReadMessage", target: "t7" });
  assert.deepEqual(msg, { _tag: "MessageLoaded", reply: reply() });
  assert.deepEqual(
    execs(t.calls).map((c) => c.argv),
    [["message", "--json", "--target", "t7"]],
  );
});

test("a send writes the comments and the message id to a file and passes only its path", async () => {
  const t = env({ exec: answers(ok(""), ok(JSON.stringify({ schema: 1, sent: 2 }))) });
  const comments = [
    { start: 2, end: 3, body: "Why `$(rm -rf /)`?" },
    { start: 5, end: 5, body: "b" },
  ];
  const msg = await run(t.env, { _tag: "SendMessage", target: "t7", id: "m1", comments });
  assert.deepEqual(msg, { _tag: "MessageSent", sent: 2 });
  const [written] = files(t.calls);
  assert.ok(written);
  assert.deepEqual(JSON.parse(written.text), { id: "m1", comments });
  const calls = execs(t.calls);
  assert.deepEqual(
    calls.slice(0, 2).map((c) => c.command),
    ["mkdir", "chmod"],
  );
  assert.deepEqual(calls[2]?.argv, ["message", "send", "--target", "t7", "--file", written.path]);
  assert.deepEqual(calls[2]?.options, { timeout: 30000 });
  for (const c of calls) {
    assert.ok(!c.argv.join(" ").includes("rm -rf"), "the comments are never on argv");
  }
});

test("E: a refusal is Failed with op message-send and the reason of stderr", async () => {
  const stderr = "The agent is waiting on a prompt. Nothing was sent; your comments are still unsent.\n";
  const t = env({ exec: answers(ok(""), exit(1, stderr)) });
  const msg = await run(t.env, { _tag: "SendMessage", target: "t7", id: "m1", comments: [{ start: 1, end: 1, body: "x" }] });
  assert.deepEqual(msg, { _tag: "Failed", op: "message-send", error: { _tag: "BackendFailed", code: 1, stderr: stderr.trim() } });
});

test("E: a binary that does not know message is TooOld, for the read and for the send, and the draft is overwritten", async () => {
  const old = env({ exec: answers(exit(2, "unknown option --json"), exit(2, "unknown option --target")) });
  assert.deepEqual(await run(old.env, { _tag: "ReadMessage", target: "t7" }), { _tag: "Failed", op: "message", error: { _tag: "TooOld" } });
  const msg = await run(old.env, { _tag: "SendMessage", target: "t7", id: "m1", comments: [{ start: 1, end: 1, body: "x" }] });
  assert.deepEqual(msg, { _tag: "Failed", op: "message-send", error: { _tag: "TooOld" } });
  const [draft, blank] = files(old.calls);
  assert.ok(draft && blank);
  assert.equal(blank.path, draft.path);
  assert.equal(blank.text, "");
});

test("E: a reply that is not the message shape is a contract mismatch", async () => {
  const t = env({ exec: answers(ok(JSON.stringify({ schema: 1, agent: "claude" })), ok("")) });
  const msg = await run(t.env, { _tag: "ReadMessage", target: "t7" });
  assert.ok(msg && msg._tag === "Failed" && msg.op === "message" && msg.error._tag === "ContractMismatch");
});

test("while a send runs no comment is added or changed, and Refresh waits for the editor to close", () => {
  const m = comment(shown(), 2, 2, "why");
  const sending = steps(m, { _tag: "MessageSendClicked" }).model;
  assert.equal(sending.message.sending, true);
  const added = steps(sending, plus(3));
  assert.equal(added.model.editor, null);
  assert.equal(added.model.message.status, "wait for the send to finish");
  const id = sending.message.comments[0]?.id ?? -1;
  assert.equal(steps(sending, { _tag: "MessageCommentAction", id, action: "delete" }).model.message.comments.length, 1);
  // The editor open: a Refresh that brought a new message would close it and lose the text, so it does not read.
  const editing = steps(shown(), plus(2)).model;
  const refresh = steps(editing, { _tag: "MessageRefreshed" });
  assert.deepEqual(refresh.cmds, []);
  assert.notEqual(refresh.model.editor, null);
});
