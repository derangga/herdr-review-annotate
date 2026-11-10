import assert from "node:assert/strict";
import { test } from "node:test";
import { decodeFile, decodeFocus, decodePanel, decodePanes, decodeProps, decodeState, parseJson } from "../src/decode.ts";
import { fixture } from "./fake-host.ts";

const state = () => JSON.parse(fixture("state"));
const file = () => JSON.parse(fixture("file"));

function mismatch(run: () => unknown): { path: string; detail: string } {
  try {
    run();
  } catch (e) {
    assert.equal((e as { _tag: string })._tag, "ContractMismatch");
    return e as { path: string; detail: string };
  }
  throw new assert.AssertionError({ message: "did not throw" });
}

test("the state fixture decodes", () => {
  const s = decodeState(state());
  assert.equal(s.root, "/repo");
  assert.deepEqual(s.spec, { kind: "worktree" });
  assert.equal(s.base, "main");
  assert.equal(s.archivable, 1);
  assert.equal(s.unread_resolved, 1);
  assert.equal(s.files.length, 5);
  assert.equal(s.files[3]?.path, "src/a.rs");
  assert.equal(s.files[3]?.new, 1);
  assert.equal(s.files[4]?.old_path, "src/old_b.rs");
  assert.equal(s.orphans[0]?.placement.kind, "not_in_diff");
  assert.equal(s.orphans[0]?.anchor.kind, "file");
});

test("archivable is required and must be an integer", () => {
  for (const value of [undefined, 1.5, "1", null]) {
    const raw = { ...state(), archivable: value };
    assert.equal(mismatch(() => decodeState(raw)).path, "$.archivable");
  }
});

test("unread_resolved is required and must be an integer", () => {
  for (const value of [undefined, 1.5, "1", null]) {
    const raw = { ...state(), unread_resolved: value };
    assert.equal(mismatch(() => decodeState(raw)).path, "$.unread_resolved");
  }
});

test("the file fixture decodes, rows and threads with their row indexes", () => {
  const f = decodeFile(file());
  assert.equal(f.path, "src/a.rs");
  assert.equal(f.rows.length, 7);
  assert.deepEqual(f.rows[2], { kind: "del", old: 2, new: null, text: "    old();", tokens: [{ from: 4, to: 7, kind: "function" }], marks: [{ from: 4, to: 7 }] });
  assert.deepEqual(f.rows[4]?.marks, []);
  assert.equal(f.threads.length, 5);
  assert.deepEqual(f.threads[1]?.placement, { kind: "matched", row: 4, start_row: 3 });
  assert.equal(f.threads[1]?.comments[1]?.parent, "u2");
  assert.equal(f.threads[2]?.anchor.side, "old");
  assert.equal(f.threads[4]?.anchor.side, null);
});

// The row the Rust test `tokens_are_kinds_with_utf16_offsets_from_the_whole_file` prints: the grinning face is 4
// bytes and 2 UTF-16 units, so the string is bytes 8..16 and units 8..13. Offsets that counted bytes or code points
// would cut this row in the wrong places.
const NON_ASCII = 'let s = "\u00e9\u{1f600}"; // x';
const NON_ASCII_TOKENS = [
  [0, 3, "keyword"],
  [8, 13, "string"],
  [15, 19, "comment"],
];

test("E: tokens are read as UTF-16 offsets into the text, one row with a non-ASCII character", () => {
  const raw = file();
  raw.rows[1] = { kind: "add", old: null, new: 3, text: NON_ASCII, tokens: NON_ASCII_TOKENS };
  const row = decodeFile(raw).rows[1];
  assert.ok(row !== undefined);
  assert.deepEqual(
    row.tokens.map((t) => [row.text.slice(t.from, t.to), t.kind]),
    [
      ["let", "keyword"],
      ['"\u00e9\u{1f600}"', "string"],
      ["// x", "comment"],
    ],
  );
});

test("E: a row with no tokens field has none, and a field that is not a list is no tokens", () => {
  const raw = file();
  delete raw.rows[1].tokens;
  assert.deepEqual(decodeFile(raw).rows[1]?.tokens, []);
  for (const odd of [null, "keyword", 7, { 0: 1 }]) {
    raw.rows[1].tokens = odd;
    assert.deepEqual(decodeFile(raw).rows[1]?.tokens, [], String(odd));
  }
});

test("E: a token that is malformed, out of order, overlapping, past the text or of an unknown kind is dropped, the rest stay", () => {
  const raw = file();
  const text = "fn main() {";
  raw.rows[1].text = text;
  raw.rows[1].tokens = [
    [0, 2, "keyword"],
    "x",
    [3],
    [3, 7, "function", "extra"],
    [1.5, 4, "type"],
    ["3", 7, "type"],
    [3, 7, "shiny"],
    [3, 7, 4],
    [1, 5, "type"], // starts inside the one before
    [5, 5, "type"], // empty
    [7, 6, "type"], // backwards
    [-1, 2, "type"],
    [3, text.length + 1, "type"], // past the end
    [3, 7, "function"],
    [8, text.length, "operator"],
  ];
  assert.deepEqual(decodeFile(raw).rows[1]?.tokens, [
    { from: 0, to: 2, kind: "keyword" },
    { from: 3, to: 7, kind: "function" },
    { from: 8, to: 11, kind: "operator" },
  ]);
});

test("E: marks are read as UTF-16 offsets, a missing or odd field is none, and a bad mark is dropped, the rest stay", () => {
  const raw = file();
  const text = "fn main() {";
  raw.rows[1].text = text;
  raw.rows[1].marks = [{ from: 3, to: 7 }];
  assert.deepEqual(decodeFile(raw).rows[1]?.marks, [{ from: 3, to: 7 }]);
  delete raw.rows[1].marks;
  assert.deepEqual(decodeFile(raw).rows[1]?.marks, []);
  for (const odd of [null, "main", 7, { from: 3, to: 7 }]) {
    raw.rows[1].marks = odd;
    assert.deepEqual(decodeFile(raw).rows[1]?.marks, [], String(odd));
  }
  raw.rows[1].marks = [
    { from: 0, to: 2 },
    "x",
    null,
    [3, 7],
    { from: 3 },
    { from: 1.5, to: 4 },
    { from: "3", to: 7 },
    { from: 1, to: 5 }, // starts inside the one before
    { from: 5, to: 5 }, // empty
    { from: 7, to: 6 }, // backwards
    { from: -1, to: 2 },
    { from: 3, to: text.length + 1 }, // past the end
    { from: 3, to: 7 },
    { from: 8, to: text.length },
  ];
  assert.deepEqual(decodeFile(raw).rows[1]?.marks, [
    { from: 0, to: 2 },
    { from: 3, to: 7 },
    { from: 8, to: 11 },
  ]);
});

test("a branch spec text decodes to the base", () => {
  const raw = state();
  raw.spec = "branch:origin/main";
  assert.deepEqual(decodeState(raw).spec, { kind: "branch", base: "origin/main" });
  raw.spec = "somewhere";
  assert.equal(mismatch(() => decodeState(raw)).path, "$.spec");
});

test("another schema number is a mismatch, checked before any other field", () => {
  const raw = state();
  raw.schema = 2;
  delete raw.files;
  const e = mismatch(() => decodeState(raw));
  assert.equal(e.path, "schema");
  assert.match(e.detail, /schema 1.*2/);
  const old = mismatch(() => decodeFile({ path: "x" }));
  assert.equal(old.path, "schema");
});

test("a missing or retyped field names its path", () => {
  const raw = state();
  delete raw.files[0].add;
  assert.equal(mismatch(() => decodeState(raw)).path, "$.files[0].add");
  const h = state();
  delete h.files[0].hash;
  assert.equal(mismatch(() => decodeState(h)).path, "$.files[0].hash");
  const f = file();
  f.threads[1].comments[0].sent = "yes";
  assert.equal(mismatch(() => decodeFile(f)).path, "$.threads[1].comments[0].sent");
  const g = file();
  g.rows[0].kind = "weird";
  assert.equal(mismatch(() => decodeFile(g)).path, "$.rows[0].kind");
});

test("a thread row outside the rows is a mismatch", () => {
  const f = file();
  f.threads[0].placement.row = 7;
  assert.equal(mismatch(() => decodeFile(f)).path, "$.threads[0].placement.row");
});

test("a reply that is not JSON is a mismatch", () => {
  assert.equal(mismatch(() => parseJson("{ half")).path, "$");
  assert.equal(mismatch(() => decodeState(null)).path, "$");
});

test("props never throw and drop a bad field", () => {
  assert.deepEqual(decodeProps({ root: "/r", target: 7, bin: "" }), { root: "/r", target: null, bin: null });
  assert.deepEqual(decodeProps(undefined), { root: null, target: null, bin: null });
  assert.deepEqual(decodeProps("text"), { root: null, target: null, bin: null });
});

test("focused and panes are read loosely", () => {
  assert.equal(decodeFocus(null), null);
  assert.deepEqual(decodeFocus({ tab: "t1", cwd: "/r", terminal: "t7", agent: "claude" }), { tab: "t1", cwd: "/r", terminal: "t7", agent: true });
  assert.deepEqual(decodeFocus({ tab: 3, cwd: null }), { tab: null, cwd: null, terminal: null, agent: false });
  assert.deepEqual(decodePanes("nope"), []);
  const panes = decodePanes([{ tab: "t1", cwd: "/a", terminal: "t2", program: { agent: true } }, { tab: "t1", cwd: null }, 5]);
  assert.deepEqual(panes, [{ tab: "t1", cwd: "/a", terminal: "t2", agent: true }]);
});

test("the panel settings decode from a good file", () => {
  assert.deepEqual(decodePanel('{"side":"right","open":false}'), { side: "right", open: false });
  assert.deepEqual(decodePanel('{"side":"left","open":true}'), { side: "left", open: true });
});

test("E: a missing, unreadable or odd settings file gives left and shown, field by field", () => {
  const defaults = { side: "left", open: true };
  for (const text of [null, "", "not json", "{", "null", "[]", "7", '"right"', "{}", '{"side":3,"open":"no"}', '{"side":"top"}']) {
    assert.deepEqual(decodePanel(text), defaults, String(text));
  }
  assert.deepEqual(decodePanel('{"side":"right","open":"no"}'), { side: "right", open: true });
  assert.deepEqual(decodePanel('{"side":"up","open":false}'), { side: "left", open: false });
  assert.deepEqual(decodePanel('{"open":false}'), { side: "left", open: false });
});
