// The send graph and the archive graph of the GUI: each E: line is a test here.
import assert from "node:assert/strict";
import { test } from "node:test";
import { makeCli } from "../src/cli.ts";
import { confirmKey, confirmRow, dock, pickerOptions } from "../src/rows.ts";
import { run } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import { initModel } from "../src/update.ts";
import type { Cmd, Model, Msg, ReviewError } from "../src/types.ts";
import { exit, fakeHost, ok } from "./fake-host.ts";
import type { Call, Reply } from "./fake-host.ts";
import { fixtureFile, fixtureState, loadedModel, reload, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();

// A model that has loaded src/a.rs and waits, with a target as the panel has when it opened beside an agent.
const loaded = (target: string | null = "t7"): Model => loadedModel(state, file, target);

function env(exec: (command: string, args: string[]) => Reply): { env: Env; calls: Call[] } {
  const fake = fakeHost({ exec });
  return { env: { host: fake.host, cli: makeCli(fake.host, "/bin/rv"), dispatch: () => {}, closed: () => false }, calls: fake.calls };
}
const argvs = (calls: Call[]) => calls.filter((c) => c.method === "exec").map((c) => c.args[1] as string[]);
const sendCmd = (target: string | null, threads: string[] = []): Cmd => ({ _tag: "Send", root: "/repo", target, threads });
const reply = (extra: Record<string, unknown>) => ({ schema: 1, sent: 3, queued: false, agent: "claude-code", target: null, warnings: [], ...extra });
const refusal = (error: string, extra: Record<string, unknown> = {}) => JSON.stringify({ schema: 1, error, message: `m ${error}`, candidates: [], ...extra });
const failedMsg = (error: ReviewError, op: "send" | "archive" | "viewed" = "send"): Msg => ({ _tag: "Failed", op, error });

async function sendWith(result: Reply, target: string | null = "t7") {
  const t = env(() => result);
  return { msg: await run(t.env, sendCmd(target)), calls: t.calls };
}
const failure = (msg: Msg | null): ReviewError => {
  assert.ok(msg && msg._tag === "Failed", `expected Failed, got ${JSON.stringify(msg)}`);
  return msg.error;
};

// send: update

test("E: no thread given and no unsent thread in the state -> status, no call", () => {
  const none = { ...state, files: state.files.map((f) => ({ ...f, unsent: 0 })), orphans: [] };
  const m = reload(loaded(), none).model;
  const out = steps(m, { _tag: "SendClicked" });
  assert.equal(out.model.status, "nothing to send");
  assert.deepEqual(out.cmds, []);
});

test("E: editor open -> status finish or cancel, no call", () => {
  const editing = steps(loaded(), { _tag: "FileCommentClicked" }).model;
  const out = steps(editing, { _tag: "SendClicked" });
  assert.equal(out.model.status, "finish or cancel the comment first");
  assert.deepEqual(out.cmds, []);
});

test("a send click sets status sending and asks for one Send with the target and the threads", () => {
  const out = steps(loaded(), { _tag: "SendClicked" });
  assert.equal(out.model.status, "sending...");
  assert.deepEqual(out.cmds, [sendCmd("t7")]);
  assert.deepEqual(steps(loaded(null), { _tag: "SendClicked", threads: ["u1", "u3"] }).cmds, [sendCmd(null, ["u1", "u3"])]);
});

test("a second click while a send runs asks for nothing, so one click is one exec", () => {
  assert.deepEqual(steps(loaded(), { _tag: "SendClicked" }, { _tag: "SendClicked" }, { _tag: "SendClicked", threads: ["u1"] }).cmds, [sendCmd("t7")]);
});

test("a resend is offered on an open thread whose comments are all sent, not on an unsent or a resolved one", () => {
  const withFocus = (id: string) => steps(loaded(), { _tag: "ThreadAction", id, action: "focus" }).model;
  const resend = (id: string) => dock(withFocus(id)).buttons.find((b) => b.id === "resend");
  assert.equal(resend("u1"), undefined); // unsent
  assert.equal(resend("u2"), undefined); // resolved
  const sentThread = { ...file, threads: file.threads.map((t) => (t.id === "u1" ? { ...t, unsent: false } : t)) };
  const m = steps(reload(loaded(), state, sentThread).model, { _tag: "ThreadAction", id: "u1", action: "focus" }).model;
  assert.deepEqual(dock(m).buttons.find((b) => b.id === "resend")?.msg, { _tag: "SendClicked", threads: ["u1"] });
});

// send: ReviewCli

test("the argv: with a target, without one under hc, and with two --thread flags", async () => {
  const t = env(() => ok(JSON.stringify(reply({}))));
  await run(t.env, sendCmd("t7"));
  await run(t.env, sendCmd(null));
  await run(t.env, sendCmd("t7", ["u1", "u3"]));
  assert.deepEqual(argvs(t.calls), [
    ["send", "--repo", "/repo", "--json", "--target", "t7"],
    ["send", "--repo", "/repo", "--json", "--backend", "hc"],
    ["send", "--repo", "/repo", "--json", "--target", "t7", "--thread", "u1", "--thread", "u3"],
  ]);
  assert.deepEqual(t.calls.find((c) => c.method === "exec")?.args[2], { timeout: 30000 });
});

test("exit 0 answers Sent with the count, the agent and queued", async () => {
  const { msg } = await sendWith(ok(JSON.stringify(reply({ sent: 2, queued: true }))));
  assert.deepEqual(msg, { _tag: "Sent", sent: 2, agent: "claude-code", queued: true, warnings: [] });
});

test("E: json error blocked, not_ready, agent_gone -> Refused", async () => {
  for (const [kind, why] of [["blocked", "Blocked"], ["not_ready", "NotReady"], ["agent_gone", "AgentGone"]] as const) {
    assert.deepEqual(failure((await sendWith({ code: 1, stdout: refusal(kind), stderr: "" })).msg), { _tag: "Refused", why });
  }
});

test("E: json error no_agent -> NoTarget", async () => {
  assert.deepEqual(failure((await sendWith({ code: 1, stdout: refusal("no_agent"), stderr: "" }, null)).msg), { _tag: "NoTarget" });
});

test("E: json error ambiguous -> AmbiguousTarget with the candidates", async () => {
  const candidates = [{ address: "t7", agent: "claude-code", cwd: "/repo", status: "idle" }, { address: "t9", agent: "claude-code", cwd: "/repo/sub", status: "working" }];
  assert.deepEqual(failure((await sendWith({ code: 1, stdout: refusal("ambiguous", { candidates }), stderr: "" }, null)).msg), { _tag: "AmbiguousTarget", candidates });
});

test("E: json error busy -> Busy, backend -> BackendFailed with the message", async () => {
  assert.deepEqual(failure((await sendWith({ code: 1, stdout: refusal("busy"), stderr: "" })).msg), { _tag: "Busy" });
  assert.deepEqual(failure((await sendWith({ code: 1, stdout: refusal("backend"), stderr: "" })).msg), { _tag: "BackendFailed", code: 1, stderr: "m backend" });
  assert.deepEqual(failure((await sendWith({ code: 1, stdout: "", stderr: "disk full" })).msg), { _tag: "BackendFailed", code: 1, stderr: "disk full" });
});

test("E: BinaryMissing, TooOld and ContractMismatch -> Failed", async () => {
  assert.deepEqual(failure((await sendWith("reject")).msg), { _tag: "BinaryMissing" });
  assert.deepEqual(failure((await sendWith(exit(2, "unknown option --json"))).msg), { _tag: "TooOld" });
  const mismatch = failure((await sendWith(ok(JSON.stringify(reply({ schema: 2 }))))).msg);
  assert.equal(mismatch._tag, "ContractMismatch");
  assert.equal(failure((await sendWith(ok(JSON.stringify(reply({ sent: "3" }))))).msg)._tag, "ContractMismatch");
});

test("a refusal costs one exec and no retry", async () => {
  const t = env(() => ({ code: 1, stdout: refusal("blocked"), stderr: "" }));
  await run(t.env, sendCmd("t7"));
  assert.equal(argvs(t.calls).length, 1);
});

// send: the reducer after the reply

test("Sent shows sent N to the agent and reloads, and the line survives that reload", () => {
  const out = steps(loaded(), { _tag: "SendClicked" }, { _tag: "Sent", sent: 3, agent: "claude-code", queued: false, warnings: [] });
  assert.equal(out.model.status, "sent 3 to claude-code");
  assert.equal(out.model.sending, false);
  const load = out.cmds.find((c) => c._tag === "Load");
  assert.ok(load && load._tag === "Load");
  assert.equal(reload(out.model, state).model.status, "sent 3 to claude-code");
});

test("Sent with queued shows the queued line, with warnings adds them", () => {
  const queued = steps(loaded(), { _tag: "Sent", sent: 2, agent: "claude-code", queued: true, warnings: [] }).model;
  assert.equal(queued.status, "claude-code is working, 2 comments queued");
  assert.equal(steps(loaded(), { _tag: "Sent", sent: 1, agent: null, queued: true, warnings: [] }).model.status, "the agent is working, 1 comment queued");
  assert.equal(steps(loaded(), { _tag: "Sent", sent: 1, agent: "claude-code", queued: false, warnings: ["target not saved"] }).model.status, "sent 1 to claude-code (target not saved)");
  assert.equal(steps(loaded(), { _tag: "Sent", sent: 0, agent: null, queued: false, warnings: [] }).model.status, "nothing to send");
});

test("E: Refused -> status and one notify, nothing marked sent, the next click sends again", () => {
  for (const why of ["Blocked", "NotReady", "AgentGone"] as const) {
    const out = steps(loaded(), { _tag: "SendClicked" }, failedMsg({ _tag: "Refused", why }));
    if (why !== "AgentGone") assert.equal(out.model.status, `nothing was sent: ${why}`);
    assert.equal(out.cmds.filter((c) => c._tag === "Notify").length, 1);
    assert.equal(out.model.state, state); // the screen is as it was, no reload
    assert.equal(out.model.sending, false);
    assert.equal(steps(out.model, { _tag: "SendClicked" }).cmds.length, 1);
  }
});

test("E: Refused AgentGone drops the gone target, so the next click resolves by folder", () => {
  const out = steps(loaded("t7"), { _tag: "SendClicked" }, failedMsg({ _tag: "Refused", why: "AgentGone" }));
  assert.equal(out.model.target, null);
  assert.match(out.model.status ?? "", /agent is gone/);
  assert.deepEqual(steps(out.model, { _tag: "SendClicked" }).cmds, [sendCmd(null)]);
  const blocked = steps(loaded("t7"), { _tag: "SendClicked" }, failedMsg({ _tag: "Refused", why: "Blocked" }));
  assert.equal(blocked.model.target, "t7");
});

test("E: NoTarget -> status, nothing else", () => {
  const out = steps(loaded(null), { _tag: "SendClicked" }, failedMsg({ _tag: "NoTarget" }));
  assert.equal(out.model.status, "no agent found for this folder");
  assert.equal(out.model.sending, false);
});

const candidates = [{ address: "t7", agent: "claude-code", cwd: "/repo", status: "idle" }, { address: "t9", agent: "claude-code", cwd: "/repo/sub", status: "working" }];

test("E: AmbiguousTarget -> picker, then a picked address sends again with --target and the same threads", () => {
  const asked = steps(loaded(null), { _tag: "SendClicked", threads: ["u1"] }, failedMsg({ _tag: "AmbiguousTarget", candidates }));
  assert.deepEqual(asked.model.picker, candidates);
  assert.equal(asked.model.status, null);
  assert.deepEqual(pickerOptions(candidates).map((b) => b.msg), [{ _tag: "PickerChosen", target: "t7" }, { _tag: "PickerChosen", target: "t9" }, { _tag: "PickerCancelled" }]);
  const picked = steps(asked.model, { _tag: "PickerChosen", target: "t9" });
  assert.deepEqual(picked.cmds, [sendCmd("t9", ["u1"])]);
  assert.equal(picked.model.target, "t9");
  assert.equal(picked.model.picker, null);
  assert.equal(picked.model.state, asked.model.state);
});

test("a picked address that was not offered sends nothing", () => {
  const asked = steps(loaded(null), { _tag: "SendClicked" }, failedMsg({ _tag: "AmbiguousTarget", candidates }));
  assert.deepEqual(steps(asked.model, { _tag: "PickerChosen", target: "t99" }).cmds, []);
});

test("a picker cancel calls nothing", () => {
  const asked = steps(loaded(null), { _tag: "SendClicked" }, failedMsg({ _tag: "AmbiguousTarget", candidates }));
  const out = steps(asked.model, { _tag: "PickerCancelled" });
  assert.equal(out.model.picker, null);
  assert.deepEqual(out.cmds, []);
});

test("the picker text is cleaned of control characters", () => {
  assert.equal(pickerOptions([{ address: "t1", agent: "a\u001b[31m", cwd: "/x\t", status: "idle" }])[0]?.label, "t1  a[31m  idle  /x    ");
});

// archive

test("a click on Archive only asks, with a sentence, an Archive button and Cancel", () => {
  const first = steps(loaded(), { _tag: "ArchiveClicked" });
  assert.deepEqual(first.cmds, []);
  assert.equal(first.model.confirmArchive, true);
  const row = confirmRow(first.model);
  assert.equal(row?.sentence, "Archive 1 resolved thread?");
  assert.deepEqual(row?.buttons.map((b) => [b.label, b.danger, b.msg._tag]), [["Archive", false, "ArchiveConfirmed"], ["Cancel", false, "ConfirmCancelled"]]);
  assert.equal(dock(first.model).actions.find((b) => b.id === "archive")?.label, "Archive 1");
});

test("the Archive button in the row is Cmd.Archive, and without the question nothing is archived", () => {
  const asked = steps(loaded(), { _tag: "ArchiveClicked" }).model;
  const second = steps(asked, { _tag: "ArchiveConfirmed" });
  assert.deepEqual(second.cmds, [{ _tag: "Archive", root: "/repo" }]);
  assert.equal(second.model.confirmArchive, false);
  assert.equal(second.model.status, "archiving...");
  assert.deepEqual(steps(loaded(), { _tag: "ArchiveConfirmed" }).cmds, []);
});

test("Cancel and Esc after a click on Archive leave the model as it was and archive nothing", () => {
  const before = loaded();
  const asked = steps(before, { _tag: "ArchiveClicked" }).model;
  for (const msg of [confirmRow(asked)?.buttons[1]?.msg, confirmKey("escape")]) {
    assert.ok(msg);
    const out = steps(asked, msg);
    assert.deepEqual(out.cmds, []);
    assert.deepEqual(out.model, before);
  }
});

test("the archive button is off while a load is in flight, and a click then asks nothing", () => {
  const { model } = steps(loaded(), { _tag: "Triggered", source: "refresh" });
  assert.equal(model.inFlight, true);
  assert.equal(dock(model).actions.find((b) => b.id === "archive")?.off, true);
  const out = steps(model, { _tag: "ArchiveClicked" });
  assert.deepEqual(out.cmds, []);
  assert.equal(out.model.confirmArchive, false);
});

// A loaded model whose state counts `n` archivable threads.
const counting = (n: number): Model => ({ ...loaded(), state: { ...loaded().state!, archivable: n } });
const archiveButton = (m: Model) => dock(m).actions.find((b) => b.id === "archive");

test("the archive button and the question name the number, singular for one", () => {
  for (const [n, label, sentence] of [[1, "Archive 1", "Archive 1 resolved thread?"], [3, "Archive 3", "Archive 3 resolved threads?"]] as const) {
    const m = counting(n);
    assert.equal(archiveButton(m)?.label, label);
    assert.equal(archiveButton(m)?.off, false);
    assert.equal(confirmRow(steps(m, { _tag: "ArchiveClicked" }).model)?.sentence, sentence);
  }
});

test("the archive question is plain text with an accent button and no red, the delete question stays red", () => {
  const row = confirmRow(steps(counting(2), { _tag: "ArchiveClicked" }).model);
  assert.equal(row?.color, "foreground");
  assert.deepEqual(row?.buttons.map((b) => [b.label, b.danger, b.accent === true]), [["Archive", false, true], ["Cancel", false, false]]);
  const del = confirmRow(steps(loaded(), { _tag: "ThreadAction", id: "u1", action: "delete" }).model);
  assert.deepEqual([del?.color, del?.buttons[0]?.danger], ["ansi1", true]);
});

test("with nothing to archive the button is off, says only Archive, and a click asks nothing", () => {
  const m = counting(0);
  assert.deepEqual([archiveButton(m)?.label, archiveButton(m)?.off], ["Archive", true]);
  const out = steps(m, { _tag: "ArchiveClicked" });
  assert.deepEqual(out.cmds, []);
  assert.equal(out.model.confirmArchive, false);
  assert.equal(archiveButton(initModel())?.off, true);
});

test("a reload that leaves nothing to archive drops the question, and one that leaves some keeps it with the new number", () => {
  const asked = steps(counting(3), { _tag: "ArchiveClicked" }).model;
  const after = (n: number) => reload(asked, { ...state, archivable: n }).model;
  assert.equal(after(0).confirmArchive, false);
  assert.equal(confirmRow(after(0)), null);
  assert.equal(confirmRow(after(2))?.sentence, "Archive 2 resolved threads?");
});

test("a reload keeps the question, and a send drops it", () => {
  const asked = steps(loaded(), { _tag: "ArchiveClicked" }).model;
  assert.equal(reload(asked, state).model.confirmArchive, true);
  assert.equal(steps(asked, { _tag: "SendClicked" }).model.confirmArchive, false);
});

test("the archive argv and the Archived msg", async () => {
  const t = env(() => ok(JSON.stringify({ schema: 1, archived: 2, unsent: 1 })));
  assert.deepEqual(await run(t.env, { _tag: "Archive", root: "/repo" }), { _tag: "Archived", archived: 2, unsent: 1 });
  assert.deepEqual(argvs(t.calls), [["archive", "--repo", "/repo", "--json"]]);
  assert.deepEqual(t.calls.find((c) => c.method === "exec")?.args[2], { timeout: 30000 });
});

test("the status text for 0, 1 and several unsent, and the reply's count wins", () => {
  const text = (archived: number, unsent: number) => steps(loaded(), { _tag: "Archived", archived, unsent }).model.status;
  assert.equal(text(0, 0), "nothing to archive");
  assert.equal(text(1, 0), "archived 1 thread");
  assert.equal(text(3, 0), "archived 3 threads");
  assert.equal(text(3, 1), "archived 3 threads, 1 of them was never sent");
  assert.equal(text(3, 2), "archived 3 threads, 2 of them were never sent");
});

test("Archived reloads", () => {
  assert.equal(steps(loaded(), { _tag: "Archived", archived: 1, unsent: 0 }).cmds.filter((c) => c._tag === "Load").length, 1);
});

test("E: archive Busy -> status review is busy, BackendFailed -> the stderr, ContractMismatch -> Failed", async () => {
  const code = async (result: Reply) => failure(await run(env(() => result).env, { _tag: "Archive", root: "/repo" }));
  assert.deepEqual(await code(exit(1, "review is busy")), { _tag: "Busy" });
  assert.deepEqual(await code(exit(1, "disk full")), { _tag: "BackendFailed", code: 1, stderr: "disk full" });
  assert.equal((await code(ok(JSON.stringify({ schema: 1, archived: "2", unsent: 0 }))))._tag, "ContractMismatch");
  assert.equal(steps(loaded(), failedMsg({ _tag: "Busy" }, "archive")).model.status, "review is busy, try again");
  assert.equal(steps(loaded(), failedMsg({ _tag: "BackendFailed", code: 1, stderr: "disk full" }, "archive")).model.status, "disk full");
});

// viewed

test("a viewed click asks for SetViewed with the model's spec, and the dock offers the opposite", () => {
  const m = loaded();
  assert.deepEqual(dock(m).actions.find((b) => b.id === "viewed")?.msg, { _tag: "ViewedClicked", path: "src/a.rs", viewed: true });
  assert.deepEqual(steps(m, { _tag: "ViewedClicked", path: "src/a.rs", viewed: true }).cmds, [{ _tag: "SetViewed", root: "/repo", spec: { kind: "worktree" }, path: "src/a.rs", viewed: true }]);
  const branch = steps(m, { _tag: "SpecChosen", spec: { kind: "branch", base: "origin/main" } }).model;
  assert.deepEqual(steps(branch, { _tag: "ViewedClicked", path: "src/a.rs", viewed: false }).cmds, [{ _tag: "SetViewed", root: "/repo", spec: { kind: "branch", base: "origin/main" }, path: "src/a.rs", viewed: false }]);
  const done = loadedModel({ ...state, files: state.files.map((f) => (f.path === "src/a.rs" ? { ...f, viewed: true } : f)) }, { ...file, viewed: true });
  assert.equal(dock(done).actions.find((b) => b.id === "viewed")?.label, "Mark unviewed");
});

test("the view argv: --set or --unset, and the spec flags", async () => {
  const t = env(() => ok(""));
  const cmd = (viewed: boolean, spec: Extract<Cmd, { _tag: "SetViewed" }>["spec"]): Cmd => ({ _tag: "SetViewed", root: "/repo", spec, path: "src/a.rs", viewed });
  assert.deepEqual(await run(t.env, cmd(true, { kind: "worktree" })), { _tag: "Wrote", kind: "viewed" });
  await run(t.env, cmd(false, { kind: "branch", base: "origin/main" }));
  await run(t.env, cmd(true, null));
  assert.deepEqual(argvs(t.calls), [
    ["view", "--repo", "/repo", "--path", "src/a.rs", "--set", "--spec", "worktree"],
    ["view", "--repo", "/repo", "--path", "src/a.rs", "--unset", "--spec", "branch", "--base", "origin/main"],
    ["view", "--repo", "/repo", "--path", "src/a.rs", "--set"],
  ]);
});

test("Wrote viewed reloads and leaves an open editor alone", () => {
  const editing = steps(loaded(), { _tag: "FileCommentClicked" }).model;
  const out = steps(editing, { _tag: "Wrote", kind: "viewed" });
  assert.notEqual(out.model.editor, null);
  assert.deepEqual(out.cmds, []); // the editor defers the reload
  assert.equal(steps(loaded(), { _tag: "Wrote", kind: "viewed" }).cmds.filter((c) => c._tag === "Load").length, 1);
});

test("E: viewed BadRequest (not in diff) -> status and one reload, a second one only the status", () => {
  const bad: Msg = failedMsg({ _tag: "BadRequest", stderr: "src/a.rs is not in the diff" }, "viewed");
  const one = steps(loaded(), bad);
  assert.equal(one.model.status, "src/a.rs is not in the diff");
  assert.equal(one.cmds.filter((c) => c._tag === "Load").length, 1);
  assert.equal(steps(one.model, bad).cmds.filter((c) => c._tag === "Load").length, 0);
});

test("E: viewed Busy and BackendFailed -> status, the mark is as it was", () => {
  assert.equal(steps(loaded(), failedMsg({ _tag: "Busy" }, "viewed")).model.status, "review is busy, try again");
  const out = steps(loaded(), failedMsg({ _tag: "BackendFailed", code: 1, stderr: "git failed" }, "viewed"));
  assert.equal(out.model.status, "git failed");
  assert.equal(out.model.state, state);
});
