// The icon buttons: every record the panel draws as a button carries a drawing that exists, and an icon-only button is one square wide.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { test } from "node:test";
import { buttonsHeight, cardWidth, dock, layoutButton, sidebarButtons, SPLIT_W, streamLayout } from "../src/rows.ts";
import type { DockButton } from "../src/rows.ts";
import { UI_ICONS } from "../src/icons.ts";
import type { FileView, Model } from "../src/types.ts";
import { fixtureFile, fixtureState, loadedModel, steps } from "./model.ts";

const state = fixtureState();
const file = fixtureFile();
const NOW = Date.parse("2026-11-09T10:00:30Z");

test("every drawing named in UI_ICONS is a file in icons/", () => {
  for (const name of UI_ICONS) assert.ok(existsSync(new URL(`../icons/${name}.svg`, import.meta.url)), name);
});

// The same review with its first thread sent and answered by the agent, so the card of the agent's reply carries Resolve and Resend.
const answered = { ...file, threads: file.threads.map((t) => (t.id === "u1" ? { ...t, unsent: false, comments: [...t.comments, { ...t.comments[0], id: "r1", parent: "u1", author: "agent:claude" }] } : t)) } as FileView;
const empty = { ...state, orphans: [], files: state.files.map((f) => ({ ...f, threads: 0 })) };

test("each button of the header, the dock and the cards has a drawing from UI_ICONS", () => {
  const models: Model[] = [
    loadedModel(state, file),
    loadedModel(state, answered),
    loadedModel(empty, { ...file, threads: [] }),
  ];
  const seen: DockButton[] = [];
  const header: { id: string; icon: string }[] = [];
  for (const base of models) {
    for (const side of ["left", "right"] as const) {
      for (const open of [true, false]) {
        for (const width of [SPLIT_W, 300]) {
          const m: Model = { ...base, panel: { side, open }, width };
          header.push(...sidebarButtons(m), layoutButton(m));
        }
      }
    }
    const d = dock(base);
    seen.push(...d.buttons, ...d.actions);
    // A file shown, viewed and not: the Mark viewed button is a toggle.
    const viewed = steps(base, { _tag: "ViewedClicked", path: "src/a.rs", viewed: true }).model;
    seen.push(...dock(viewed).actions, ...dock({ ...base, selected: null }).actions);
    for (const i of streamLayout({ ...base, expanded: new Set(["u1", "u2", "u3", "u4", "u5"]) }, NOW).items) if (i.kind === "body") seen.push(...i.buttons);
  }
  assert.ok(header.length > 0 && seen.length > 0);
  for (const b of header) assert.ok(UI_ICONS.includes(b.icon), `${b.id}: ${b.icon}`);
  for (const b of seen) assert.ok(b.icon !== undefined && UI_ICONS.includes(b.icon), `${b.id}: ${b.icon}`);
  // The header's four are all drawn: the files toggle follows the side, and the arrows are in the dock.
  const ids = new Set(seen.map((b) => b.id));
  for (const id of ["reply", "resolve", "reopen", "resend", "edit", "delete", "prev-thread", "next-thread", "send", "archive", "viewed", "file"]) assert.ok(ids.has(id), id);
});

test("the files toggle draws the panel for the side the sidebar is on, and is lit while it shows", () => {
  const m = loadedModel(state, file);
  const at = (side: "left" | "right", open: boolean) => sidebarButtons({ ...m, panel: { side, open } }).map((b) => [b.icon, b.on]);
  assert.deepEqual(at("left", true), [["panel-left", true], ["swap", false]]);
  assert.deepEqual(at("right", true), [["panel-right", true], ["swap", false]]);
  assert.deepEqual(at("right", false), [["panel-right", false]]);
  assert.deepEqual([layoutButton({ ...m, width: SPLIT_W }).on, layoutButton({ ...m, width: 300 }).on], [true, false]);
});

test("an icon-only card button is one square wide, so four of them are one row of today's height", () => {
  const four = ["reply", "resolve", "edit", "delete"].map((id): DockButton => ({ id, label: id === "resolve" ? "Resolve" : id, msg: { _tag: "ConfirmCancelled" }, danger: false, off: false, icon: "check", iconOnly: true }));
  const one = 4 + 24;
  assert.equal(buttonsHeight(four, 160), one);
  assert.equal(buttonsHeight(four, cardWidth(0)), one);
  // The same four with their words do not fit 160 px in one row.
  assert.ok(buttonsHeight(four.map((b) => ({ ...b, iconOnly: false })), 160) > one);
});

test("the card buttons, the dock's arrows and Mark viewed are icons alone; Send and Archive keep their words", () => {
  const body = streamLayout({ ...loadedModel(state, file), expanded: new Set(["u1", "u2"]) }, NOW).items.filter((i) => i.kind === "body");
  assert.ok(body.length > 0);
  for (const i of body) if (i.kind === "body") for (const b of i.buttons) assert.equal(b.iconOnly, true, b.id);
  const actions = dock(loadedModel(state, file)).actions;
  assert.deepEqual(actions.map((b) => [b.id, b.iconOnly === true]), [["prev-thread", true], ["next-thread", true], ["send", false], ["archive", false], ["viewed", true]]);
  assert.deepEqual(actions.filter((b) => b.id.endsWith("-thread")).map((b) => b.key), ["shift+n", "n"]);
});
