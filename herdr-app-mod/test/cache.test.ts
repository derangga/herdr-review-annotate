// The per-render caches in rows.ts: each gives what a plain recomputation gives, the same object while its inputs are the
// same objects, and a new answer when one of them changes.
import assert from "node:assert/strict";
import { test } from "node:test";
import { fileMatches, filterCount, filterInfo, filterPick, noStops, sidebarRows, stops, totals, unsentCount } from "../src/rows.ts";
import type { FileSummary, Model, State } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel } from "./model.ts";

const state = fixtureState();
const base: Model = loadedModel(state, fixtureFile());

const f = (path: string, over: Partial<FileSummary> = {}): FileSummary => ({ path, old_path: null, change: "modified", add: 1, del: 2, hash: "0123456789abcdef", viewed: false, threads: 0, open: 0, new: 0, unsent: 0, ...over });
const files = [f("src/a.rs"), f("src/b.rs"), f("README.md"), f("src/deep/c.rs"), f("Cargo.toml")];

test("sidebar rows are what the uncached walk gives, and the same array while files and filter are the same", () => {
  for (const q of ["", "rs", "src", "Cargo", "zzz", "dc"]) {
    const hits = files.filter((x) => fileMatches(q, x.path));
    const rows = sidebarRows(files, q);
    assert.deepEqual(
      rows.flatMap((r) => (r.kind === "file" ? [r.path] : [])),
      hits.map((x) => x.path),
    );
    assert.equal(rows.filter((r) => r.kind === "file").length, hits.length);
    assert.equal(sidebarRows(files, q), rows);
  }
});

test("a new filter or a new files array gives new rows", () => {
  const a = sidebarRows(files, "rs");
  const b = sidebarRows(files, "src");
  assert.notEqual(a, b);
  assert.notEqual(sidebarRows(files, "rs"), a); // only the latest query of an array is kept
  const copy = [...files];
  assert.notEqual(sidebarRows(copy, "src"), sidebarRows(files, "src"));
  assert.deepEqual(sidebarRows(copy, "src"), sidebarRows(files, "src"));
  assert.deepEqual(sidebarRows([]), []);
});

test("the count and the box info agree with the uncached count and are kept", () => {
  for (const q of ["", "rs", "zzz"]) {
    const shown = files.filter((x) => fileMatches(q, x.path)).length;
    assert.deepEqual(filterCount(files, q), { shown, total: files.length });
    assert.equal(filterCount(files, q), filterCount(files, q));
    const m: Model = { ...base, state: { ...state, files }, filter: q };
    assert.deepEqual(filterInfo(m), { count: `${shown}/${files.length}`, none: shown === 0 });
    assert.equal(filterInfo(m), filterInfo({ ...m, offset: 10 }));
  }
  const m: Model = { ...base, state: { ...state, files }, filter: "rs" };
  assert.notEqual(filterInfo(m), filterInfo({ ...m, filter: "src" }));
  assert.notEqual(filterInfo(m), filterInfo({ ...m, state: { ...state, files: [...files] } }));
  assert.deepEqual(filterInfo({ ...m, state: null, filter: "x" }), { count: "0/0", none: false });
});

test("Enter in the box picks from the same matches", () => {
  const m: Model = { ...base, state: { ...state, files }, filter: "deep", selected: "src/a.rs" };
  assert.equal(filterPick(m), "src/deep/c.rs");
  assert.equal(filterPick({ ...m, selected: "src/deep/c.rs" }), null);
  assert.equal(filterPick({ ...m, filter: "zzz" }), null);
});

test("totals add the counts, and are the same object for the same state", () => {
  const s: State = { ...state, files };
  assert.deepEqual(totals(s), { files: 5, add: 5, del: 10 });
  assert.equal(totals(s), totals(s));
  const other = { ...s, files: files.slice(0, 2) };
  assert.notEqual(totals(other), totals(s));
  assert.deepEqual(totals(other), { files: 2, add: 2, del: 4 });
  assert.deepEqual(totals(null), { files: 0, add: 0, del: 0 });
});

test("unsent comments are counted as before and kept for the state", () => {
  const recount = (s: State) => s.files.reduce((n, x) => n + x.unsent, 0) + s.orphans.filter((t) => t.unsent).length;
  assert.equal(unsentCount(state), recount(state));
  assert.equal(unsentCount(state), unsentCount(state));
  const more: State = { ...state, files: state.files.map((x) => ({ ...x, unsent: x.unsent + 1 })) };
  assert.equal(unsentCount(more), recount(more));
  assert.notEqual(unsentCount(more), unsentCount(state));
  assert.equal(unsentCount(null), 0);
});

test("noStops is stops(m).all.length === 0 for a few models, and follows state, folds and files", () => {
  const same = (m: Model) => assert.equal(noStops(m), stops(m).all.length === 0);
  const bare: State = { ...state, orphans: [], files: state.files.map((x) => ({ ...x, threads: 0 })) };
  const quiet: Model = { ...base, state: bare, files: new Map() };
  same(base);
  same(quiet);
  same({ ...quiet, state: null });
  same({ ...base, folds: new Map([["src/a.rs", true]]) });
  assert.equal(noStops(base), false);
  assert.equal(noStops(quiet), true);
  assert.equal(noStops({ ...quiet, state: null }), true);
  // orphans alone are a stop
  assert.equal(noStops({ ...quiet, state: { ...bare, orphans: state.orphans } }), false);
  // a file the summary counts a thread in, not drawn, is one stop
  const counted: State = { ...bare, files: bare.files.map((x) => (x.path === "src/b.rs" ? { ...x, threads: 1 } : x)) };
  same({ ...quiet, state: counted });
  assert.equal(noStops({ ...quiet, state: counted }), false);
  // a drawn file with threads is a stop, and folding it leaves the summary's count
  const drawn: Model = { ...base, state: bare };
  assert.equal(noStops(drawn), false);
  same(drawn);
  const folded = { ...drawn, folds: new Map([["src/a.rs", true]]) };
  assert.equal(noStops(folded), true);
  same(folded);
  assert.equal(noStops(drawn), false); // the same files map, other folds
  assert.equal(noStops({ ...drawn, state: { ...bare, orphans: state.orphans } }), false); // other state
});
