import assert from "node:assert/strict";
import { test } from "node:test";
import { initModel, update } from "../src/update.ts";
import type { Cmd, Model, Msg, ReviewError } from "../src/types.ts";
import { answer, fixtureFile, fixtureState, loadedModel, reload, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();

const loads = (cmds: Cmd[]) => cmds.filter((c) => c._tag === "Load");
const opened: Msg = { _tag: "Opened", root: "/repo", target: "t7" };
const trig = (source: "watch" | "timer" | "refresh" | "write" | "change" = "watch"): Msg => ({ _tag: "Triggered", source });
const failedMsg = (error: ReviewError, op: "load" | "watch-store" | "watch-root" = "load"): Msg => ({ _tag: "Failed", op, error });

// A model that has loaded every file under the working tree, with src/a.rs on top, and waits.
const loaded = (): Model => loadedModel(state, file);

test("opening with no folder shows the banner and loads nothing", () => {
  const { model, cmds } = steps(initModel(), { _tag: "Opened", root: null, target: null });
  assert.equal(model.banner, "open the panel from a pane that has a folder");
  assert.deepEqual(cmds, []);
  assert.deepEqual(steps(model, trig("timer"), trig("refresh")).cmds, []);
});

test("opening starts one load with no spec, so the CLI reads meta.json", () => {
  const { model, cmds } = steps(initModel(), opened);
  assert.equal(model.inFlight, true);
  assert.equal(model.target, "t7");
  assert.deepEqual(cmds, [{ _tag: "Load", key: model.key, root: "/repo", spec: null }]);
});

test("a reply replaces the state and the file, selects the file, takes the spec and clears the banner", () => {
  const m = loaded();
  assert.equal(m.state, state);
  assert.equal(m.files.get("src/a.rs"), file);
  assert.equal(m.selected, "src/a.rs");
  assert.deepEqual(m.spec, { kind: "worktree" });
  assert.equal(m.inFlight, false);
  assert.equal(m.banner, null);
});

test("the store watch starts once, from the first reply", () => {
  const first = steps(initModel(), opened);
  const load = first.cmds[0];
  assert.ok(load && load._tag === "Load");
  const reply: Msg = { _tag: "Loaded", key: load.key, state };
  const { model, cmds } = steps(first.model, reply);
  assert.deepEqual(cmds.filter((c) => c._tag === "WatchStore"), [{ _tag: "WatchStore", dir: "/state" }]);
  const again = steps(answer(model, cmds).model, trig("timer"));
  const second = again.cmds[0];
  assert.ok(second && second._tag === "Load");
  assert.deepEqual(reload(again.model, state).cmds.filter((c) => c._tag === "WatchStore"), []);
});

test("E: WatchUnavailable -> the timer only, and the watch is tried again after the next load", () => {
  const m = loaded();
  assert.equal(m.storeWatch, "/state");
  const [lost, cmds] = update(m, failedMsg({ _tag: "WatchUnavailable" }, "watch-store"));
  assert.deepEqual(cmds, []);
  assert.equal(lost.storeWatch, null);
  assert.equal(lost.banner, null);
  const retry = steps(lost, trig("timer"));
  const load = retry.cmds[0];
  assert.ok(load && load._tag === "Load");
  assert.deepEqual(steps(retry.model, { _tag: "Loaded", key: load.key, state }).cmds.filter((c) => c._tag === "WatchStore"), [{ _tag: "WatchStore", dir: "/state" }]);
});

test("a trigger during a load starts nothing, and the end of the load starts one more", () => {
  const { model, cmds } = steps(initModel(), opened, trig(), trig("timer"));
  assert.equal(loads(cmds).length, 1);
  assert.equal(model.pending, true);
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  const after = steps(model, { _tag: "Loaded", key: load.key, state });
  assert.equal(loads(after.cmds).length, 1);
  assert.equal(after.model.pending, false);
  assert.equal(after.model.inFlight, true);
});

test("a burst of fifty triggers runs two loads at most", () => {
  const burst = Array.from({ length: 50 }, () => trig());
  const first = steps(initModel(), opened, ...burst);
  const a = first.cmds[0];
  assert.ok(a && a._tag === "Load");
  const second = steps(first.model, { _tag: "Loaded", key: a.key, state }, ...burst);
  const b = second.cmds.find((c) => c._tag === "Load");
  assert.ok(b && b._tag === "Load");
  const third = steps(second.model, { _tag: "Loaded", key: b.key, state });
  assert.equal(loads(first.cmds).length + loads(second.cmds).length + loads(third.cmds).length, 3);
  // Each burst met exactly one load, so one more load per burst and not fifty.
  assert.equal(loads(first.cmds).length, 1);
  assert.equal(loads(second.cmds).length, 1);
  assert.equal(loads(third.cmds).length, 1);
});

test("a trigger while the editor is open loads nothing until the editor closes", () => {
  const m: Model = { ...loaded(), editor: { kind: "comment", anchor: null, thread: null, comment: null, initial: "" } };
  const held = steps(m, trig(), trig("timer"), trig("refresh"));
  assert.deepEqual(held.cmds, []);
  assert.equal(held.model.pending, true);
  const closed = steps(held.model, { _tag: "EditorCancelled" });
  assert.equal(loads(closed.cmds).length, 1);
  assert.equal(closed.model.editor, null);
  assert.equal(closed.model.pending, false);
});

test("a reply for the previous spec is dropped, and the load that was asked for follows", () => {
  const m = loaded();
  const [busy, cmds] = update(m, trig("timer"));
  const old = cmds[0];
  assert.ok(old && old._tag === "Load");
  const chosen = steps(busy, { _tag: "SpecChosen", spec: { kind: "branch", base: "main" } });
  assert.deepEqual(chosen.cmds, []);
  const stale = steps(chosen.model, { _tag: "Loaded", key: old.key, state });
  assert.equal(stale.model.state, m.state);
  assert.deepEqual(stale.model.spec, { kind: "branch", base: "main" });
  assert.deepEqual(stale.cmds, [{ _tag: "Load", key: stale.model.key, root: "/repo", spec: { kind: "branch", base: "main" } }]);
});

test("switching the spec keeps the selected file, drops every diff fetched and loads with the new spec", () => {
  const { model, cmds } = steps(loaded(), { _tag: "SpecChosen", spec: { kind: "branch", base: "main" } });
  assert.equal(model.selected, "src/a.rs");
  assert.equal(model.files.size, 0);
  assert.deepEqual(cmds, [{ _tag: "Load", key: model.key, root: "/repo", spec: { kind: "branch", base: "main" } }]);
  assert.deepEqual(steps(loaded(), { _tag: "SpecChosen", spec: { kind: "worktree" } }).cmds, []);
});

test("the spec a reply names replaces the one asked for", () => {
  const { model, cmds } = steps(loaded(), { _tag: "SpecChosen", spec: { kind: "branch", base: "gone" } });
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  const fallback = { ...state, spec: { kind: "worktree" as const }, base: null, notices: ["no base branch found"] };
  const after = steps(model, { _tag: "Loaded", key: load.key, state: fallback }).model;
  assert.deepEqual(after.spec, { kind: "worktree" });
  assert.deepEqual(after.state?.notices, ["no base branch found"]);
});

test("choosing a file selects it and asks the stream to scroll to its header, with no fetch when it is there", () => {
  const { model, cmds } = steps(loaded(), { _tag: "FileChosen", path: "notes.txt" });
  assert.equal(model.selected, "notes.txt");
  assert.equal(model.selection, null);
  assert.deepEqual(model.reveal, { key: "notes.txt|file" });
  assert.deepEqual(cmds, []);
});

test("choosing a file that has not arrived fetches it first, with the files after it", () => {
  const { model } = steps(initModel(), opened);
  const arrived = steps(model, { _tag: "Loaded", key: model.key, state });
  const idle = { ...arrived.model, inFlight: false, batch: [] }; // the fetch that began with the state has not been answered yet
  const { cmds } = steps(idle, { _tag: "FileChosen", path: "src/a.rs" });
  const fetch = cmds[0];
  assert.ok(fetch && fetch._tag === "LoadFiles");
  assert.deepEqual(fetch.paths, ["src/a.rs", "src/b.rs", "mod"]); // the folded img.png is left out, and the files above come last
});

test("a file switch with the editor open is ignored with a status", () => {
  const m: Model = { ...loaded(), editor: { kind: "comment", anchor: null, thread: null, comment: null, initial: "" } };
  const { model, cmds } = steps(m, { _tag: "FileChosen", path: "notes.txt" });
  assert.deepEqual(cmds, []);
  assert.equal(model.selected, "src/a.rs");
  assert.equal(model.status, "finish or cancel the comment first");
});

test("a failed load keeps the screen", () => {
  const m = loaded();
  const { model } = steps(m, trig("timer"), failedMsg({ _tag: "BackendFailed", code: 1, stderr: "git failed" }));
  assert.equal(model.state, m.state);
  assert.equal(model.files, m.files);
  assert.equal(model.status, "git failed");
  assert.equal(model.banner, null);
  assert.equal(model.inFlight, false);
});

test("E: BackendFailed with no good model yet -> banner", () => {
  const { model } = steps(initModel(), opened, failedMsg({ _tag: "BackendFailed", code: 1, stderr: "not a git repository" }));
  assert.equal(model.banner, "not a git repository");
  assert.equal(model.status, null);
});

test("E: Busy -> status line, no retry", () => {
  const { model, cmds } = steps(loaded(), trig("timer"), failedMsg({ _tag: "Busy" }));
  assert.equal(model.status, "review is busy, try again");
  assert.equal(loads(cmds).length, 1);
  assert.equal(model.inFlight, false);
  assert.equal(model.stopped, false);
});

test("E: BadRequest -> status line and one reload, not a second in a row", () => {
  const m = loaded();
  const first = steps(m, trig("timer"), failedMsg({ _tag: "BadRequest", stderr: "not in diff: src/a.rs" }));
  assert.equal(first.model.status, "not in diff: src/a.rs");
  assert.equal(loads(first.cmds).length, 2);
  assert.equal(first.model.inFlight, true);
  const second = steps(first.model, failedMsg({ _tag: "BadRequest", stderr: "not in diff: src/a.rs" }));
  assert.deepEqual(second.cmds, []);
  assert.equal(second.model.inFlight, false);
});

test("E: BinaryMissing, TooOld and ContractMismatch -> banner, and only Refresh loads again", () => {
  const errors: [ReviewError, RegExp][] = [
    [{ _tag: "BinaryMissing" }, /herdr-review not found/],
    [{ _tag: "TooOld" }, /update herdr-review/],
    [{ _tag: "ContractMismatch", path: "schema", detail: "this mod reads schema 1, the binary sent 2" }, /schema.*schema 1.*2/],
  ];
  for (const [error, banner] of errors) {
    const { model } = steps(initModel(), opened, trig(), failedMsg(error));
    assert.match(model.banner ?? "", banner);
    assert.equal(model.stopped, true);
    assert.equal(model.pending, false);
    assert.deepEqual(steps(model, trig("timer"), trig("watch")).cmds, []);
    const again = steps(model, trig("refresh"));
    assert.equal(loads(again.cmds).length, 1);
    assert.equal(again.model.stopped, false);
  }
});

test("a good reply after a banner clears it", () => {
  const stopped = steps(initModel(), opened, failedMsg({ _tag: "TooOld" })).model;
  const again = steps(stopped, trig("refresh"));
  const load = again.cmds[0];
  assert.ok(load && load._tag === "Load");
  const { model } = steps(again.model, { _tag: "Loaded", key: load.key, state });
  assert.equal(model.banner, null);
  assert.equal(model.state, state);
});

test("a path that is not in the diff is not chosen", () => {
  const m = loaded();
  assert.deepEqual(steps(m, { _tag: "FileChosen", path: "gone.rs" }), { model: m, cmds: [] });
});

test("a reply that leaves the sidebar's file out of the diff selects the first file, and drops its diff", () => {
  const { model, cmds } = steps(loaded(), trig("timer"));
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  const left = { ...state, files: state.files.filter((f) => f.path !== "src/a.rs") };
  const after = steps(model, { _tag: "Loaded", key: load.key, state: left }).model;
  assert.equal(after.selected, "img.png");
  assert.equal(after.files.has("src/a.rs"), false);
});

test("a reply with no files clears the selection", () => {
  const { model, cmds } = steps(loaded(), trig("timer"));
  const load = cmds[0];
  assert.ok(load && load._tag === "Load");
  const after = steps(model, { _tag: "Loaded", key: load.key, state: { ...state, files: [] } }).model;
  assert.equal(after.selected, null);
  assert.equal(after.files.size, 0);
});

test("the error rows of the later work are in the table", () => {
  const refused = steps(loaded(), failedMsg({ _tag: "Refused", why: "Blocked" }));
  assert.equal(refused.model.status, "nothing was sent: Blocked");
  assert.equal(refused.cmds[0]?._tag, "Notify");
  assert.equal(steps(loaded(), failedMsg({ _tag: "NoTarget" })).model.status, "no agent found for this folder");
  const candidates = [{ address: "t7", agent: "claude-code", cwd: "/repo", status: "idle" }];
  assert.deepEqual(steps(loaded(), failedMsg({ _tag: "AmbiguousTarget", candidates })).model.picker, candidates);
});

test("the sidebar starts on the left and shown", () => {
  assert.deepEqual(initModel().panel, { side: "left", open: true });
});

test("the toggle hides and shows the sidebar, and each click saves the new setting", () => {
  const hid = steps(initModel(), { _tag: "SidebarToggled" });
  assert.deepEqual(hid.model.panel, { side: "left", open: false });
  assert.deepEqual(hid.cmds, [{ _tag: "SavePanel", panel: { side: "left", open: false } }]);
  const shown = steps(hid.model, { _tag: "SidebarToggled" });
  assert.deepEqual(shown.model.panel, { side: "left", open: true });
  assert.deepEqual(shown.cmds, [{ _tag: "SavePanel", panel: { side: "left", open: true } }]);
});

test("the flip moves the sidebar to the other side and back, keeping whether it shows", () => {
  const right = steps(initModel(), { _tag: "SidebarFlipped" });
  assert.deepEqual(right.model.panel, { side: "right", open: true });
  assert.deepEqual(right.cmds, [{ _tag: "SavePanel", panel: { side: "right", open: true } }]);
  const hidden = steps(right.model, { _tag: "SidebarToggled" }).model;
  assert.deepEqual(steps(hidden, { _tag: "SidebarFlipped" }).model.panel, { side: "left", open: false });
});

test("the loaded settings replace the panel and save nothing", () => {
  const { model, cmds } = steps(initModel(), { _tag: "PanelLoaded", panel: { side: "right", open: false } });
  assert.deepEqual(model.panel, { side: "right", open: false });
  assert.deepEqual(cmds, []);
});

test("a settings change touches nothing else: the loaded review is kept and no load starts", () => {
  const m = loaded();
  const next = steps(m, { _tag: "SidebarToggled" }, { _tag: "SidebarFlipped" }, { _tag: "PanelLoaded", panel: { side: "right", open: true } }).model;
  assert.deepEqual({ ...next, panel: m.panel }, m);
});

test("E: a settings write that failed leaves the view as it is and says so", () => {
  const m = steps(initModel(), { _tag: "SidebarToggled" }).model;
  const { model, cmds } = steps(m, { _tag: "Failed", op: "panel", error: { _tag: "BackendFailed", code: -1, stderr: "x" } });
  assert.deepEqual(model.panel, { side: "left", open: false });
  assert.equal(model.status, "could not remember the sidebar setting");
  assert.deepEqual(cmds, []);
});
