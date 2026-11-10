// A resolved card an agent closed counts as seen once it has been on screen for a whole tick. The E: lines of the "seen by
// looking" section of design/graphs/gui-stream.md are the tests here.
import assert from "node:assert/strict";
import { test } from "node:test";
import { freshOn, scrolled, streamLayout } from "../src/rows.ts";
import type { FileView, Model, Msg, Thread } from "../src/types.ts";
import { fixtureFile, loadedModel, reload, steps } from "./model.ts";

const NOW = Date.parse("2026-11-09T10:00:30Z");
const seen = (id: string) => ({ _tag: "Write", root: "/repo", spec: { kind: "worktree" }, op: { kind: "seen", id } });
const tick: Msg = { _tag: "Glanced" };

// The panel as it runs: after every message the model learns what is on screen from the stream.
function settle(m: Model): Model {
  for (let i = 0; i < 4; i++) {
    const msg = scrolled(m, streamLayout(m, NOW));
    if (msg === null) {
      return m;
    }
    m = steps(m, msg).model;
  }
  throw new Error("the scroll state did not settle");
}
// A tall screen: each open comment on a card carries a row of buttons, which takes room.
const view = (file: FileView = fixtureFile()): Model => settle({ ...loadedModel(undefined, file), height: 900 });
const resolved = (t: Thread, id: string, is_new: boolean): Thread => ({ ...t, id, status: "resolved", resolved_by: "agent:claude", is_new, comments: t.comments.map((c, i) => (i === 0 ? { ...c, id } : c)) });
const withThreads = (...extra: Thread[]): FileView => {
  const f = fixtureFile();
  return { ...f, threads: [...f.threads, ...extra] };
};

test("the fresh cards are the resolved threads an agent closed that are new, among the cards on screen", () => {
  assert.deepEqual(view().fresh, ["u2"]);
  // Not on screen: the screen is the top 40 pixels, a few file headers.
  const short = settle({ ...loadedModel(), height: 40 });
  assert.deepEqual(short.fresh, []);
});

test("an open thread and a thread the user resolved or already saw are never fresh", () => {
  const base = fixtureFile().threads.find((t) => t.id === "u2") as Thread;
  const file = { ...fixtureFile(), threads: fixtureFile().threads.map((t) => (t.id === "u2" ? { ...t, is_new: false } : t)) };
  assert.deepEqual(view(file).fresh, []);
  const byUser = { ...resolved(base, "u9", false), resolved_by: "user" };
  assert.deepEqual(view({ ...file, threads: [...file.threads, byUser] }).fresh, []);
  // The open thread u1 is on screen and is not in the list.
  assert.ok(streamLayout(view(file), NOW).items.some((i) => i.kind === "head" && i.id === "u1"));
  assert.deepEqual(freshOn(streamLayout(view(file), NOW).items, 0, 1000), []);
});

test("a card on screen for a whole tick sends its seen, once", () => {
  const m = view();
  const first = steps(m, tick);
  assert.deepEqual(first.cmds, []); // it has only just come into view
  const second = steps(first.model, tick);
  assert.deepEqual(second.cmds, [seen("u2")]);
  assert.deepEqual(steps(second.model, tick, tick).cmds, []);
});

test("a card scrolled past between two ticks sends nothing", () => {
  const away = settle({ ...steps(view(), tick).model, height: 40 });
  assert.deepEqual(away.fresh, []);
  const ticked = steps(away, tick).model;
  assert.deepEqual(ticked.glance, []);
  const back = settle({ ...ticked, height: 600 });
  // Back in view, but it has to stay for a tick of its own.
  assert.deepEqual(steps(back, tick).cmds, []);
});

test("one seen a tick, so a screen of new cards is a short queue and not a burst", () => {
  const base = fixtureFile().threads.find((t) => t.id === "u2") as Thread;
  const m = view(withThreads({ ...resolved(base, "u8", true), placement: { ...base.placement, row: 0, start_row: 0 } }, { ...resolved(base, "u9", true), placement: { ...base.placement, row: 1, start_row: 1 } }));
  assert.deepEqual([...m.fresh].sort(), ["u2", "u8", "u9"]);
  const out = steps(m, tick, tick, tick, tick);
  assert.equal(out.cmds.length, 3);
  assert.deepEqual(out.cmds.map((c) => (c._tag === "Write" && c.op.kind === "seen" ? c.op.id : null)).sort(), ["u2", "u8", "u9"]);
  const one = steps(m, tick, tick);
  assert.equal(one.cmds.length, 1);
});

test("a card is not sent again after the reload shows it seen, nor while a reload still shows it new", () => {
  const sent = steps(view(), tick, tick);
  assert.equal(sent.cmds.length, 1);
  // The reload that follows the write still says new (a stale file): nothing is sent twice.
  assert.deepEqual(steps(sent.model, tick, tick).cmds, []);
  // The reload that says seen: the card leaves the fresh list.
  const calm = { ...fixtureFile(), threads: fixtureFile().threads.map((t) => (t.id === "u2" ? { ...t, is_new: false } : t)) };
  const after = settle(reload(sent.model, sent.model.state!, calm).model);
  assert.deepEqual(after.fresh, []);
  assert.deepEqual(steps(after, tick, tick).cmds, []);
});

test("opening a new resolved card sends its seen, and looking at it afterwards sends none", () => {
  const opened = steps(view(), { _tag: "ThreadAction", id: "u2", action: "toggle" });
  assert.deepEqual(opened.cmds, [seen("u2")]);
  assert.deepEqual(steps(opened.model, tick, tick).cmds, []);
});

test("a stopped panel or one without a root sends nothing", () => {
  assert.deepEqual(steps({ ...view(), stopped: true }, tick, tick).cmds, []);
  assert.deepEqual(steps({ ...view(), root: null }, tick, tick).cmds, []);
});

test("a tick with nothing new on screen leaves the model as it was", () => {
  const calm = { ...fixtureFile(), threads: fixtureFile().threads.map((t) => (t.id === "u2" ? { ...t, is_new: false } : t)) };
  const m = view(calm);
  assert.equal(steps(m, tick).model, m);
});

test("a failed seen shows nothing and is not sent again by looking, opening the card still sends it", () => {
  const sent = steps(view(), tick, tick);
  const failed = steps(sent.model, { _tag: "Failed", op: "seen", error: { _tag: "Busy" } });
  assert.deepEqual(failed.cmds, []);
  assert.deepEqual(steps(failed.model, tick, tick).cmds, []);
  assert.deepEqual(steps(failed.model, { _tag: "ThreadAction", id: "u2", action: "toggle" }).cmds, [seen("u2")]);
});
