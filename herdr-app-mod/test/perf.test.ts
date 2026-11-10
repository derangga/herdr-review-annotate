import assert from "node:assert/strict";
import { test } from "node:test";
import { appendCapped, fileSink, makeRecorder, noop, percentile, recorderFor, summary } from "../src/perf.ts";
import { layoutCache } from "../src/rows.ts";
import { fakeHost } from "./fake-host.ts";
import { fixtureFile, fixtureState, loadedModel } from "./model.ts";

const clockAt = () => {
  const c = { t: 0, now: () => c.t };
  return c;
};

test("percentile is the nearest rank, and 0 for nothing", () => {
  assert.equal(percentile([], 0.95), 0);
  assert.equal(percentile([5], 0.95), 5);
  assert.equal(percentile([4, 1, 3, 2], 0.5), 2);
  const hundred = Array.from({ length: 100 }, (_, i) => i + 1);
  assert.equal(percentile(hundred, 0.95), 95);
  assert.equal(percentile(hundred, 1), 100);
});

test("the summary line has count, mean, p95 and max for render and dispatch, the rebuilds and the drawn items", () => {
  assert.equal(
    summary(5, [1, 2, 3, 10], [0.5], 2, [4, 6]),
    "perf window=5.0s render n=4 mean=4.00ms p95=10.00ms max=10.00ms | dispatch n=1 mean=0.50ms p95=0.50ms max=0.50ms | layout_rebuilds=2 | drawn mean=5.0 max=6",
  );
});

test("a window rolls over on the first event after it ended, emits one line and starts again", () => {
  const c = clockAt();
  const lines: string[] = [];
  const r = makeRecorder(c.now, (l) => lines.push(l), 5000);
  c.t = 100;
  const t0 = r.start();
  c.t = 104;
  r.render(t0, 12);
  r.rebuild();
  c.t = 4000;
  r.dispatch(3990);
  assert.deepEqual(lines, []); // still inside the window
  c.t = 5001;
  r.dispatch(5000);
  assert.equal(lines.length, 1);
  assert.match(lines[0] as string, /render n=1 mean=4\.00ms .* \| dispatch n=2 mean=5\.50ms .* \| layout_rebuilds=1 \| drawn mean=12\.0 max=12$/);
  c.t = 11000;
  r.render(10990, 3);
  assert.equal(lines.length, 2);
  assert.match(lines[1] as string, /render n=1 mean=10\.00ms .* \| dispatch n=0 .* layout_rebuilds=0 \| drawn mean=3\.0 max=3$/);
});

test("flush emits what the window holds, and an empty window emits nothing", () => {
  const c = clockAt();
  const lines: string[] = [];
  const r = makeRecorder(c.now, (l) => lines.push(l));
  r.flush();
  assert.deepEqual(lines, []);
  r.rebuild();
  r.flush();
  assert.equal(lines.length, 1);
  r.flush();
  assert.equal(lines.length, 1);
});

test("the recorder is the no-op one unless the props say perf: true", () => {
  assert.equal(recorderFor(fakeHost().host), noop());
  assert.equal(recorderFor(fakeHost({ props: { perf: false } }).host), noop());
  assert.equal(recorderFor(fakeHost({ props: { perf: "yes" } }).host), noop());
  assert.equal(noop().on, false);
  assert.equal(recorderFor(fakeHost({ props: { perf: true } }).host).on, true);
});

test("the layout cache calls back once per rebuild and not on a hit", () => {
  const m = loadedModel(fixtureState(), fixtureFile());
  let n = 0;
  const layout = layoutCache(() => n++);
  const a = layout(m, 0);
  assert.equal(layout({ ...m, offset: 40 }, 0), a);
  assert.equal(n, 1);
  layout({ ...m, width: m.width + 1 }, 0);
  assert.equal(n, 2);
});

test("the log keeps its last 200 lines", () => {
  assert.equal(appendCapped(null, "a"), "a\n");
  assert.equal(appendCapped("a\nb\n", "c"), "a\nb\nc\n");
  const old = Array.from({ length: 200 }, (_, i) => `l${i}`).join("\n") + "\n";
  const out = appendCapped(old, "new").split("\n");
  assert.equal(out.length, 201); // 200 lines and the empty piece after the last newline
  assert.equal(out[0], "l1");
  assert.equal(out[199], "new");
});

test("the file sink appends to perf.log under the app home with the text it read as the expected text", async () => {
  const { host, calls } = fakeHost({ file: "old line\n" });
  fileSink(host, () => "T")("perf window");
  await new Promise((r) => setTimeout(r, 20));
  const write = calls.find((c) => c.method === "writeTextFile");
  assert.ok(write !== undefined);
  assert.match(write.args[0] as string, /\/data\/herdr-review\/perf\.log$/);
  assert.equal(write.args[1], "old line\nT perf window\n");
  assert.equal(write.args[2], "old line\n");
});

test("a sink whose reads fail drops the line and does not throw", async () => {
  const { host, calls } = fakeHost({ file: "reject" });
  fileSink(host)("x");
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(calls.filter((c) => c.method === "writeTextFile").length, 0);
});
