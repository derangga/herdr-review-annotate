import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { test } from "node:test";
import { FOLDER, GENERIC, ICON_NAMES, fileIcon } from "../src/icons.ts";

test("a whole name wins over its extension", () => {
  assert.equal(fileIcon("Cargo.toml"), "rust");
  assert.equal(fileIcon("other.toml"), "toml");
  assert.equal(fileIcon("Cargo.lock"), "rust");
  assert.equal(fileIcon("Dockerfile"), "docker");
  assert.equal(fileIcon("Makefile"), "makefile");
  assert.equal(fileIcon(".gitignore"), "git");
});

test("the extension ignores case, and only the last one counts", () => {
  assert.equal(fileIcon("main.RS"), "rust");
  assert.equal(fileIcon("App.Test.TSX"), "react_ts");
  assert.equal(fileIcon("a.tar.json"), "json");
});

test("the common types each have an icon", () => {
  const want: Record<string, string> = {
    "a.rs": "rust",
    "a.toml": "toml",
    "a.lock": "lock",
    "a.md": "markdown",
    "a.json": "json",
    "a.yml": "yaml",
    "a.yaml": "yaml",
    "a.sh": "console",
    "a.ts": "typescript",
    "a.tsx": "react_ts",
    "a.js": "javascript",
    "a.jsx": "react",
    "a.png": "image",
    "a.svg": "svg",
    "a.txt": "document",
  };
  for (const [name, icon] of Object.entries(want)) {
    assert.equal(fileIcon(name), icon, name);
  }
});

test("a language has an icon of its own, not a shared one", () => {
  const languages = ["py", "go", "c", "h", "cpp", "hpp", "java", "kt", "swift", "rb", "php", "lua", "nix", "sql", "vue", "svelte", "astro", "html", "css", "scss", "less"];
  const icons = languages.map((e) => fileIcon(`a.${e}`));
  assert.equal(new Set(icons).size, languages.length);
  assert.ok(!icons.includes(GENERIC));
});

test("a name with no dot, an unknown extension or a name like an object key gets the generic icon", () => {
  for (const name of ["README", "data.unknownext", "", "constructor", "x.constructor", "x.__proto__", "trailing."]) {
    assert.equal(fileIcon(name), GENERIC, name);
  }
});

test("every icon the map names, and the folder, is a file in icons/", () => {
  assert.ok(ICON_NAMES.includes(FOLDER));
  for (const name of ICON_NAMES) {
    assert.ok(existsSync(new URL(`../icons/${name}.svg`, import.meta.url)), name);
  }
});

test("the upstream license ships beside the drawings", () => {
  assert.ok(existsSync(new URL("../icons/LICENSE-material-icon-theme", import.meta.url)));
});
