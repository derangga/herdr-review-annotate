import assert from "node:assert/strict";
import { test } from "node:test";
import { makeCli } from "../src/cli.ts";
import { FETCH } from "../src/message.ts";
import { open, run, watch, withRecovery } from "../src/runner.ts";
import type { Env } from "../src/runner.ts";
import { initModel, update } from "../src/update.ts";
import type { Cmd, Msg, Spec } from "../src/types.ts";
import { defaultExec, exit, fakeHost, fixture, ok } from "./fake-host.ts";
import type { Setup } from "./fake-host.ts";

const STAGED = "/home/app/mods/herdr-review/bin/herdr-review";

function env(setup: Setup = {}, bin: string | null = null) {
  const fake = fakeHost(setup);
  const sent: Msg[] = [];
  let closed = false;
  const e: Env = { host: fake.host, cli: makeCli(fake.host, bin), dispatch: (m) => sent.push(m), closed: () => closed };
  return { ...fake, env: e, sent, close: () => (closed = true) };
}
const load = (spec: Spec | null = null): Cmd => ({ _tag: "Load", key: "k", root: "/repo", spec });
const files = (paths: string[], spec: Spec | null = null): Cmd => ({ _tag: "LoadFiles", key: "k", root: "/repo", spec, paths });
const execs = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "exec").map((c) => ({ command: c.args[0], argv: c.args[1] as string[] }));

test("a load runs state alone and answers Loaded with the key it was asked for", async () => {
  const t = env();
  const msg = await run(t.env, load());
  assert.ok(msg && msg._tag === "Loaded");
  assert.equal(msg.key, "k");
  assert.equal(msg.state.files.length, 5);
  assert.deepEqual(
    execs(t.calls).map((c) => c.argv),
    [["state", "--json", "--repo", "/repo"]],
  );
});

test("a fetch of files is one files call with a --path for each, and answers FilesLoaded with the key and the files in the order asked", async () => {
  const t = env();
  const msg = await run(t.env, files(["src/a.rs", "notes.txt"]));
  assert.ok(msg && msg._tag === "FilesLoaded");
  assert.equal(msg.key, "k");
  assert.deepEqual(
    msg.files.map((f) => f.path),
    ["src/a.rs", "notes.txt"],
  );
  assert.deepEqual(
    execs(t.calls).map((c) => c.argv),
    [["files", "--json", "--repo", "/repo", "--path", "src/a.rs", "--path", "notes.txt"]],
  );
});

test("E: a path not in the diff fails the whole fetch, as op files with BadRequest", async () => {
  const t = env({ exec: () => exit(2, "not in diff: gone.rs") });
  const msg = await run(t.env, files(["src/a.rs", "gone.rs"]));
  assert.deepEqual(msg, { _tag: "Failed", op: "files", error: { _tag: "BadRequest", stderr: "not in diff: gone.rs" } });
  assert.equal(execs(t.calls).length, 1);
});

test("E: an old binary without files -> TooOld, and files in another order or number than asked -> ContractMismatch", async () => {
  const old = env({ exec: () => exit(2, "unknown command 'files'\nusage: ...") });
  assert.deepEqual(await run(old.env, files(["src/a.rs"])), { _tag: "Failed", op: "files", error: { _tag: "TooOld" } });
  const swapped = env({ exec: () => ok(JSON.stringify({ schema: 1, files: [JSON.parse(fixture("file"))] })) });
  const msg = await run(swapped.env, files(["notes.txt"]));
  assert.ok(msg && msg._tag === "Failed" && msg.error._tag === "ContractMismatch");
});

test("the spec goes on both calls as --spec and --base", async () => {
  const t = env();
  const spec: Spec = { kind: "branch", base: "origin/main" };
  await run(t.env, load(spec));
  await run(t.env, files(["src/a.rs"], spec));
  assert.deepEqual(execs(t.calls)[0]?.argv, ["state", "--json", "--repo", "/repo", "--spec", "branch", "--base", "origin/main"]);
  assert.deepEqual(execs(t.calls)[1]?.argv.slice(0, 8), ["files", "--json", "--repo", "/repo", "--spec", "branch", "--base", "origin/main"]);
});

test("a read gets 60 seconds", async () => {
  const t = env();
  await run(t.env, load());
  assert.deepEqual(t.calls.find((c) => c.method === "exec")?.args[2], { timeout: 60000 });
});

test("no Host method other than exec and watchFolder is called by a load or a watch", async () => {
  const t = env();
  await run(t.env, load());
  const w = watch(t.env, "/repo", "watch-root");
  t.close();
  t.wake();
  await w;
  assert.deepEqual([...new Set(t.calls.map((c) => c.method))].sort(), ["appHome", "exec", "watchFolder"]);
});

test("E: BinaryMissing -> the next candidate, in the order props.bin, the staged copy, PATH", async () => {
  const t = env({ exec: (command, args) => (command === "herdr-review" ? defaultExec(command, args) : "reject") }, "/custom/herdr-review");
  const msg = await run(t.env, load());
  assert.ok(msg && msg._tag === "Loaded");
  assert.deepEqual(
    execs(t.calls)
      .slice(0, 3)
      .map((c) => c.command),
    ["/custom/herdr-review", STAGED, "herdr-review"],
  );
});

test("the binary that started is kept for every later call", async () => {
  const t = env({ exec: (command, args) => (command === STAGED ? defaultExec(command, args) : "reject") });
  await run(t.env, load());
  await run(t.env, load());
  assert.deepEqual(
    execs(t.calls).map((c) => c.command),
    [STAGED, STAGED],
  );
});

test("E: no candidate left -> Failed BinaryMissing", async () => {
  const t = env({ exec: () => "reject" });
  const msg = await run(t.env, load());
  assert.deepEqual(msg, { _tag: "Failed", op: "load", error: { _tag: "BinaryMissing" } });
  assert.deepEqual(
    execs(t.calls).map((c) => c.command),
    [STAGED, "herdr-review"],
  );
});

test("E: TooOld -> exit 2 with an unknown command, option or comment command, and the search stops", async () => {
  for (const stderr of ["unknown command 'state'\nusage: ...", "unknown option --json\nusage: ...", "unknown comment command 'seen'\nusage"]) {
    const t = env({ exec: () => exit(2, stderr) });
    const msg = await run(t.env, load());
    assert.deepEqual(msg, { _tag: "Failed", op: "load", error: { _tag: "TooOld" } });
    assert.equal(execs(t.calls).length, 1);
  }
});

test("E: BadRequest -> any other exit 2", async () => {
  const t = env({ exec: () => exit(2, "not in diff: img.png") });
  const msg = await run(t.env, load());
  assert.deepEqual(msg, { _tag: "Failed", op: "load", error: { _tag: "BadRequest", stderr: "not in diff: img.png" } });
});

test("E: Busy -> exit 1 with the busy text", async () => {
  const t = env({ exec: () => exit(1, "review is busy, try again\n") });
  assert.deepEqual(await run(t.env, load()), { _tag: "Failed", op: "load", error: { _tag: "Busy" } });
});

test("E: BackendFailed -> any other exit, and a signal", async () => {
  const t = env({ exec: () => exit(1, "git: not a repository\n") });
  assert.deepEqual(await run(t.env, load()), { _tag: "Failed", op: "load", error: { _tag: "BackendFailed", code: 1, stderr: "git: not a repository" } });
  const s = env({ exec: () => exit(-1, "") });
  assert.deepEqual(await run(s.env, load()), { _tag: "Failed", op: "load", error: { _tag: "BackendFailed", code: -1, stderr: "" } });
});

test("E: ContractMismatch -> a schema number that differs, a field that is missing, and text that is not JSON", async () => {
  const other = env({ exec: () => ok(JSON.stringify({ ...JSON.parse(fixture("state")), schema: 2 })) });
  const a = await run(other.env, load());
  assert.ok(a && a._tag === "Failed" && a.error._tag === "ContractMismatch");
  assert.equal(a.error.path, "schema");
  const missing = JSON.parse(fixture("state"));
  delete missing.root;
  const b = await run(env({ exec: () => ok(JSON.stringify(missing)) }).env, load());
  assert.ok(b && b._tag === "Failed" && b.error._tag === "ContractMismatch");
  assert.equal(b.error.path, "$.root");
  const c = await run(env({ exec: () => ok("half a reply") }).env, load());
  assert.ok(c && c._tag === "Failed" && c.error._tag === "ContractMismatch");
  const fileBad = await run(env({ exec: () => ok("{}") }).env, files(["src/a.rs"]));
  assert.ok(fileBad && fileBad._tag === "Failed" && fileBad.op === "files" && fileBad.error._tag === "ContractMismatch");
});

test("E: WatchUnavailable -> watchFolder rejects", async () => {
  const t = env({ watchRejects: true });
  await watch(t.env, "/state", "watch-store");
  assert.deepEqual(t.sent, [{ _tag: "Failed", op: "watch-store", error: { _tag: "WatchUnavailable" } }]);
});

test("a watch wake is a trigger, and a wake after close dispatches nothing", async () => {
  const t = env();
  const loop = watch(t.env, "/repo", "watch-root");
  t.wake();
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(t.sent, [{ _tag: "Triggered", source: "watch" }]);
  t.close();
  t.wake();
  await loop;
  assert.equal(t.sent.length, 1);
});

test("a thrown value that is not a tagged object is a bug and goes up", async () => {
  await assert.rejects(
    withRecovery("load", async () => Promise.reject(new TypeError("boom"))),
    TypeError,
  );
  await assert.rejects(
    withRecovery("load", async () => Promise.reject(null)),
    (e) => e === null,
  );
});

test("Notify calls Host.notify and answers nothing", async () => {
  const t = env();
  assert.equal(await run(t.env, { _tag: "Notify", title: "herdr-review", body: "nothing was sent" }), null);
  assert.equal(t.calls[0]?.method, "notify");
});

test("open: props.root and props.target win", () => {
  const t = env({ props: { root: "/r", target: "t3" }, focused: { tab: "a", cwd: "/other", terminal: "t9", agent: "claude" } });
  assert.deepEqual(open(t.host), { _tag: "Opened", root: "/r", target: "t3" });
});

test("open: the focused terminal's folder is the root, and the terminal the target when an agent runs there", () => {
  const agent = env({ focused: { tab: "a", cwd: "/work", terminal: "t9", agent: "claude" } });
  assert.deepEqual(open(agent.host), { _tag: "Opened", root: "/work", target: "t9" });
  const shell = env({ focused: { tab: "a", cwd: "/work", terminal: "t9", agent: null } });
  assert.deepEqual(open(shell.host), { _tag: "Opened", root: "/work", target: null });
});

test("open: a focus that names the mod pane falls to a pane of the same tab, preferring the agent", () => {
  const t = env({
    focused: { tab: "a", cwd: null, terminal: null, agent: null },
    panes: [
      { tab: "b", cwd: "/elsewhere", terminal: "t1", program: { agent: true } },
      { tab: "a", cwd: "/shell", terminal: "t2", program: { agent: false } },
      { tab: "a", cwd: "/agent", terminal: "t3", program: { agent: true } },
      { tab: "a", cwd: null, terminal: null },
    ],
  });
  assert.deepEqual(open(t.host), { _tag: "Opened", root: "/agent", target: "t3" });
  const plain = env({ focused: { tab: "a", cwd: null }, panes: [{ tab: "a", cwd: "/shell", terminal: "t2", program: { agent: false } }] });
  assert.deepEqual(open(plain.host), { _tag: "Opened", root: "/shell", target: null });
});

test("open: no folder anywhere gives Opened with no root, and the model shows the banner and throws nothing", () => {
  for (const setup of [{ focused: null }, { focused: { tab: "a", cwd: null }, panes: [] }]) {
    const t = env(setup);
    const msg = open(t.host);
    assert.deepEqual(msg, { _tag: "Opened", root: null, target: null });
    const [model, cmds] = update(initModel(), msg);
    assert.equal(model.banner, "open the panel from a pane that has a folder");
    assert.deepEqual(cmds, []);
  }
});

const PANEL = "/home/app/data/herdr-review/panel.json";
const writes = (calls: { method: string; args: unknown[] }[]) => calls.filter((c) => c.method === "writeTextFile").map((c) => c.args);

test("ReadPanel reads panel.json under appHome and answers the decoded settings", async () => {
  const t = env({ file: '{"side":"right","open":false}' });
  assert.deepEqual(await run(t.env, { _tag: "ReadPanel" }), { _tag: "PanelLoaded", panel: { side: "right", open: false } });
  assert.deepEqual(
    t.calls.filter((c) => c.method === "readTextFile").map((c) => c.args),
    [[PANEL]],
  );
});

test("E: ReadPanel with no file, a bad file or a read that rejects answers the defaults", async () => {
  for (const file of [null, "{ nope", "reject"] as const) {
    const msg = await run(env({ file }).env, { _tag: "ReadPanel" });
    assert.deepEqual(msg, { _tag: "PanelLoaded", panel: { side: "left", open: true } });
  }
});

test("SavePanel replaces the file it read, naming that text as expected, and answers nothing", async () => {
  const t = env({ file: '{"side":"left","open":true}\n' });
  assert.equal(await run(t.env, { _tag: "SavePanel", panel: { side: "right", open: false } }), null);
  assert.deepEqual(writes(t.calls), [[PANEL, '{"side":"right","open":false}\n', '{"side":"left","open":true}\n']]);
});

test("SavePanel creates the file when there is none, and writes nothing else", async () => {
  const t = env({ file: null });
  await run(t.env, { _tag: "SavePanel", panel: { side: "left", open: true } });
  assert.deepEqual(writes(t.calls), [[PANEL, '{"side":"left","open":true}\n', null]]);
  assert.deepEqual(execs(t.calls), []);
});

test("E: SavePanel that cannot read or write fails after three tries as op panel", async () => {
  const t = env({ file: "reject" });
  const msg = await run(t.env, { _tag: "SavePanel", panel: { side: "left", open: true } });
  assert.deepEqual(msg, { _tag: "Failed", op: "panel", error: { _tag: "BackendFailed", code: -1, stderr: "could not write the panel settings" } });
  assert.equal(t.calls.filter((c) => c.method === "readTextFile").length, 3);
});

test("a refused write (the file changed since the read) is read again and then succeeds", async () => {
  const t = env({ file: "{}" });
  let n = 0;
  const write = t.host.writeTextFile;
  t.host.writeTextFile = async (...a) => {
    if (n++ === 0) {
      throw new Error("stale");
    }
    return write(...a);
  };
  assert.equal(await run(t.env, { _tag: "SavePanel", panel: { side: "right", open: true } }), null);
  assert.equal(t.calls.filter((c) => c.method === "readTextFile").length, 2);
  assert.equal(writes(t.calls).length, 1);
});

const themeReply = (over: object = {}) => ok(JSON.stringify({ schema: 1, gui: "plugin", colors: { background: "#1e1e2e" }, warnings: [], ...over }));
const unreadable = "could not read the plugin theme: ";

test("ReadTheme runs theme --json on the binary that answered, and answers the colours and the warnings", async () => {
  const t = env({ exec: (_c, args) => (args[0] === "theme" ? themeReply({ warnings: ["bad key"] }) : defaultExec(_c, args)) }, "/opt/herdr-review");
  await run(t.env, load());
  const msg = await run(t.env, { _tag: "ReadTheme" });
  assert.deepEqual(msg, { _tag: "ThemeLoaded", colors: { background: "#1e1e2e" }, warnings: ["bad key"] });
  assert.deepEqual(
    execs(t.calls).map((c) => [c.command, c.argv]),
    [
      ["/opt/herdr-review", ["state", "--json", "--repo", "/repo"]],
      ["/opt/herdr-review", ["theme", "--json"]],
    ],
  );
});

test("E: ReadTheme under app answers colors null", async () => {
  const t = env({ exec: () => themeReply({ gui: "app", colors: null }) });
  assert.deepEqual(await run(t.env, { _tag: "ReadTheme" }), { _tag: "ThemeLoaded", colors: null, warnings: [] });
});

test("E: ReadTheme that fails (TooOld, ContractMismatch, BackendFailed, a timeout, no binary) answers colors null and one status, not Failed", async () => {
  const replies = [exit(2, "unknown command 'theme'"), themeReply({ schema: 2 }), ok("not json"), exit(1, "boom\nmore"), exit(124, "")] as const;
  const texts = [`update herdr-review. Run: ${FETCH}`, "unexpected reply at schema: this mod reads schema 1, the binary sent 2", "unexpected reply at $: the reply is not JSON", "boom", "herdr-review failed (124)"];
  for (const [i, reply] of replies.entries()) {
    const msg = await run(env({ exec: () => reply }).env, { _tag: "ReadTheme" });
    assert.deepEqual(msg, { _tag: "ThemeLoaded", colors: null, warnings: [unreadable + texts[i]] });
  }
  const missing = await run(env({ exec: () => "reject" }).env, { _tag: "ReadTheme" });
  assert.deepEqual(missing, { _tag: "ThemeLoaded", colors: null, warnings: [unreadable + `herdr-review not found. Run: ${FETCH}`] });
});
