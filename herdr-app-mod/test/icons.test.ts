import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { test } from "node:test";
import { GENERIC, ICON_NAMES, fileIcon } from "../src/icons.ts";

test("a whole name wins over its extension", () => {
  assert.equal(fileIcon("Cargo.toml"), "rust");
  assert.equal(fileIcon("other.toml"), "toml");
  assert.equal(fileIcon("Cargo.lock"), "rust");
  assert.equal(fileIcon("Dockerfile"), "docker");
  assert.equal(fileIcon(".gitignore"), "git");
});

test("the extension ignores case, and only the last one counts", () => {
  assert.equal(fileIcon("main.RS"), "rust");
  assert.equal(fileIcon("App.Test.TSX"), "ts");
  assert.equal(fileIcon("a.tar.json"), "json");
});

test("the common types each have an icon", () => {
  const want: Record<string, string> = { "a.rs": "rust", "a.toml": "toml", "a.lock": "lock", "a.md": "markdown", "a.json": "json", "a.yml": "yaml", "a.yaml": "yaml", "a.sh": "shell", "a.ts": "ts", "a.js": "js", "a.png": "image" };
  for (const [name, icon] of Object.entries(want)) {
    assert.equal(fileIcon(name), icon, name);
  }
});

test("a name with no dot, an unknown extension or a name like an object key gets the generic icon", () => {
  for (const name of ["README", "data.unknownext", "", "constructor", "x.constructor", "x.__proto__", "trailing."]) {
    assert.equal(fileIcon(name), GENERIC, name);
  }
});

test("every icon the map names is a file in icons/", () => {
  for (const name of ICON_NAMES) {
    assert.ok(existsSync(new URL(`../icons/${name}.svg`, import.meta.url)), name);
  }
});
