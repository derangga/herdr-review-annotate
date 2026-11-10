// src/main.ts
import { View } from "gpui-kit";
import { InputState, TextareaState } from "gpui-base";

// src/decode.ts
var SCHEMA = 1;
function bad(path, detail) {
  const error = { _tag: "ContractMismatch", path, detail };
  throw error;
}
function obj(v, path) {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    return bad(path, "expected an object");
  }
  return v;
}
function str(o, k, path) {
  const v = o[k];
  return typeof v === "string" ? v : bad(`${path}.${k}`, "expected a string");
}
function strOrNull(o, k, path) {
  const v = o[k];
  return v === null ? null : typeof v === "string" ? v : bad(`${path}.${k}`, "expected a string or null");
}
function int(o, k, path) {
  const v = o[k];
  return typeof v === "number" && Number.isInteger(v) ? v : bad(`${path}.${k}`, "expected an integer");
}
function intOrNull(o, k, path) {
  const v = o[k];
  return v === null ? null : typeof v === "number" && Number.isInteger(v) ? v : bad(`${path}.${k}`, "expected an integer or null");
}
function bool(o, k, path) {
  const v = o[k];
  return typeof v === "boolean" ? v : bad(`${path}.${k}`, "expected a boolean");
}
function list(o, k, path, item2) {
  const v = o[k];
  if (!Array.isArray(v)) {
    return bad(`${path}.${k}`, "expected an array");
  }
  return v.map((x, i) => item2(x, `${path}.${k}[${i}]`));
}
function oneOf(o, k, path, allowed) {
  const v = o[k];
  return typeof v === "string" && allowed.includes(v) ? v : bad(`${path}.${k}`, `expected one of ${allowed.join(", ")}`);
}
function strings(v, path) {
  return typeof v === "string" ? v : bad(path, "expected a string");
}
function spec(o, k, path) {
  const text = str(o, k, path);
  if (text === "worktree") {
    return { kind: "worktree" };
  }
  if (text.startsWith("branch:") && text.length > 7) {
    return { kind: "branch", base: text.slice(7) };
  }
  return bad(`${path}.${k}`, "expected worktree or branch:<base>");
}
function schema(o) {
  const got = o["schema"];
  if (got !== SCHEMA) {
    bad("schema", `this mod reads schema ${SCHEMA}, the binary sent ${JSON.stringify(got)}`);
  }
}
var CHANGES = ["modified", "added", "deleted", "renamed", "untracked", "binary", "submodule", "too_large", "unparsed"];
function fileSummary(v, path) {
  const o = obj(v, path);
  return {
    path: str(o, "path", path),
    old_path: strOrNull(o, "old_path", path),
    change: oneOf(o, "change", path, CHANGES),
    add: int(o, "add", path),
    del: int(o, "del", path),
    hash: str(o, "hash", path),
    viewed: bool(o, "viewed", path),
    threads: int(o, "threads", path),
    open: int(o, "open", path),
    new: int(o, "new", path),
    unsent: int(o, "unsent", path)
  };
}
var TOKEN_KINDS = ["comment", "string", "number", "keyword", "operator", "function", "type"];
function tokens(o, text) {
  const v = o["tokens"];
  if (!Array.isArray(v)) {
    return [];
  }
  const out = [];
  let end = 0;
  for (const t of v) {
    if (!Array.isArray(t) || t.length !== 3) {
      continue;
    }
    const [from, to, kind] = t;
    if (typeof from !== "number" || typeof to !== "number" || !Number.isInteger(from) || !Number.isInteger(to)) {
      continue;
    }
    if (typeof kind !== "string" || !TOKEN_KINDS.includes(kind)) {
      continue;
    }
    if (from < end || from >= to || to > text.length) {
      continue;
    }
    out.push({ from, to, kind });
    end = to;
  }
  return out;
}
function marks(o, text) {
  const v = o["marks"];
  if (!Array.isArray(v)) {
    return [];
  }
  const out = [];
  let end = 0;
  for (const m of v) {
    if (typeof m !== "object" || m === null) {
      continue;
    }
    const { from, to } = m;
    if (typeof from !== "number" || typeof to !== "number" || !Number.isInteger(from) || !Number.isInteger(to)) {
      continue;
    }
    if (from < end || from >= to || to > text.length) {
      continue;
    }
    out.push({ from, to });
    end = to;
  }
  return out;
}
function row(v, path) {
  const o = obj(v, path);
  const text = str(o, "text", path);
  return {
    kind: oneOf(o, "kind", path, ["hunk", "ctx", "add", "del"]),
    old: intOrNull(o, "old", path),
    new: intOrNull(o, "new", path),
    text,
    tokens: tokens(o, text),
    marks: marks(o, text)
  };
}
function placement(v, path) {
  const o = obj(v, path);
  return {
    kind: oneOf(o, "kind", path, ["matched", "outdated", "not_in_diff"]),
    row: intOrNull(o, "row", path),
    start_row: intOrNull(o, "start_row", path)
  };
}
function anchor(v, path) {
  const o = obj(v, path);
  const side = o["side"];
  return {
    path: str(o, "path", path),
    old_path: strOrNull(o, "old_path", path),
    kind: oneOf(o, "kind", path, ["line", "range", "file"]),
    side: side === null ? null : oneOf(o, "side", path, ["old", "new"]),
    line: intOrNull(o, "line", path),
    end_line: intOrNull(o, "end_line", path),
    text: strOrNull(o, "text", path),
    spec: spec(o, "spec", path)
  };
}
function comment(v, path) {
  const o = obj(v, path);
  return {
    id: str(o, "id", path),
    parent: strOrNull(o, "parent", path),
    author: str(o, "author", path),
    at: str(o, "at", path),
    body: str(o, "body", path),
    sent: bool(o, "sent", path),
    edited_since_sent: bool(o, "edited_since_sent", path)
  };
}
function thread(v, path) {
  const o = obj(v, path);
  return {
    id: str(o, "id", path),
    status: oneOf(o, "status", path, ["open", "resolved"]),
    resolved_by: strOrNull(o, "resolved_by", path),
    is_new: bool(o, "is_new", path),
    unsent: bool(o, "unsent", path),
    placement: placement(o["placement"], `${path}.placement`),
    anchor: anchor(o["anchor"], `${path}.anchor`),
    comments: list(o, "comments", path, comment)
  };
}
function decodeState(raw) {
  const o = obj(raw, "$");
  schema(o);
  return {
    schema: SCHEMA,
    root: str(o, "root", "$"),
    store_dir: str(o, "store_dir", "$"),
    spec: spec(o, "spec", "$"),
    base: strOrNull(o, "base", "$"),
    rev: str(o, "rev", "$"),
    notices: list(o, "notices", "$", strings),
    warnings: list(o, "warnings", "$", strings),
    archivable: int(o, "archivable", "$"),
    unread_resolved: int(o, "unread_resolved", "$"),
    files: list(o, "files", "$", fileSummary),
    orphans: list(o, "orphans", "$", thread)
  };
}
function decodeFile(raw) {
  const o = obj(raw, "$");
  schema(o);
  const rows = list(o, "rows", "$", row);
  const threads = list(o, "threads", "$", thread);
  for (const [i, t] of threads.entries()) {
    for (const k of ["row", "start_row"]) {
      const at = t.placement[k];
      if (at !== null && (at < 0 || at >= rows.length)) {
        bad(`$.threads[${i}].placement.${k}`, "row index out of range");
      }
    }
  }
  return {
    schema: SCHEMA,
    path: str(o, "path", "$"),
    old_path: strOrNull(o, "old_path", "$"),
    change: oneOf(o, "change", "$", CHANGES),
    add: int(o, "add", "$"),
    del: int(o, "del", "$"),
    viewed: bool(o, "viewed", "$"),
    note: strOrNull(o, "note", "$"),
    rows,
    threads
  };
}
function decodeFiles(raw, paths) {
  const o = obj(raw, "$");
  schema(o);
  const files = list(o, "files", "$", (v) => decodeFile(v));
  if (files.length !== paths.length) {
    bad("$.files", `expected ${paths.length} files, got ${files.length}`);
  }
  for (const [i, f] of files.entries()) {
    if (f.path !== paths[i]) {
      bad(`$.files[${i}].path`, `expected ${JSON.stringify(paths[i])}, got ${JSON.stringify(f.path)}`);
    }
  }
  return files;
}
function candidate(v, path) {
  const o = obj(v, path);
  return { address: str(o, "address", path), agent: str(o, "agent", path), cwd: str(o, "cwd", path), status: str(o, "status", path) };
}
function decodeSend(raw) {
  const o = obj(raw, "$");
  schema(o);
  if (o["error"] !== void 0) {
    const kind = oneOf(o, "error", "$", ["ambiguous", "no_agent", "blocked", "not_ready", "agent_gone", "busy", "backend"]);
    const message = str(o, "message", "$");
    const candidates = list(o, "candidates", "$", candidate);
    const error = kind === "ambiguous" && candidates.length > 0 ? { _tag: "AmbiguousTarget", candidates } : kind === "no_agent" ? { _tag: "NoTarget" } : kind === "blocked" ? { _tag: "Refused", why: "Blocked" } : kind === "not_ready" ? { _tag: "Refused", why: "NotReady" } : kind === "agent_gone" ? { _tag: "Refused", why: "AgentGone" } : kind === "busy" ? { _tag: "Busy" } : { _tag: "BackendFailed", code: 1, stderr: message };
    throw error;
  }
  return { schema: SCHEMA, sent: int(o, "sent", "$"), queued: bool(o, "queued", "$"), agent: strOrNull(o, "agent", "$"), warnings: list(o, "warnings", "$", strings) };
}
function decodeMessage(raw) {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, agent: str(o, "agent", "$"), status: str(o, "status", "$"), id: str(o, "id", "$"), lines: list(o, "lines", "$", strings) };
}
function decodeMessageSent(raw) {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, sent: int(o, "sent", "$") };
}
function decodeTheme(raw) {
  const o = obj(raw, "$");
  schema(o);
  const gui = oneOf(o, "gui", "$", ["app", "plugin"]);
  const warnings = list(o, "warnings", "$", strings);
  if (o["colors"] === null) {
    return { schema: SCHEMA, gui, colors: null, warnings };
  }
  const colors = {};
  for (const [k, v] of Object.entries(obj(o["colors"], "$.colors"))) {
    if (typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v)) {
      colors[k] = v;
    }
  }
  return { schema: SCHEMA, gui, colors, warnings };
}
function decodeArchive(raw) {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, archived: int(o, "archived", "$"), unsent: int(o, "unsent", "$") };
}
function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return bad("$", "the reply is not JSON");
  }
}
function field(v, k) {
  return typeof v === "object" && v !== null ? v[k] : void 0;
}
var nonEmpty = (v) => typeof v === "string" && v !== "" ? v : null;
function decodeProps(raw) {
  return { root: nonEmpty(field(raw, "root")), target: nonEmpty(field(raw, "target")), bin: nonEmpty(field(raw, "bin")), toggle: field(raw, "role") === "toggle" };
}
function reviewPane(panes2, focus) {
  const tab = nonEmpty(field(focus, "tab"));
  if (tab === null || !Array.isArray(panes2)) {
    return null;
  }
  const p = panes2.find((x) => field(x, "mod") === "herdr-review" && field(x, "tab") === tab);
  return nonEmpty(field(p, "pane"));
}
function decodePanel(text) {
  let raw = null;
  try {
    raw = text === null ? null : JSON.parse(text);
  } catch {
    raw = null;
  }
  return { side: field(raw, "side") === "right" ? "right" : "left", open: field(raw, "open") !== false };
}
function decodeFocus(raw) {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }
  return {
    tab: nonEmpty(field(raw, "tab")),
    cwd: nonEmpty(field(raw, "cwd")),
    terminal: nonEmpty(field(raw, "terminal")),
    agent: nonEmpty(field(raw, "agent")) !== null
  };
}
function decodePanes(raw) {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out = [];
  for (const p of raw) {
    const cwd = nonEmpty(field(p, "cwd"));
    if (cwd === null) {
      continue;
    }
    const agent = field(field(p, "program"), "agent") === true || nonEmpty(field(p, "status")) !== null;
    out.push({ tab: nonEmpty(field(p, "tab")), cwd, terminal: nonEmpty(field(p, "terminal")), agent });
  }
  return out;
}

// src/toggle.ts
import { svg } from "gpui-kit";
import { h_flex } from "gpui-base";
function drawToggle(on, t, click) {
  return h_flex().size_full().pr(10).items_center().bg(t.chrome).border_b(1).border_color(t.border).child(
    h_flex().id("toggle-review").h(26).px(8).gap(5).flex_none().items_center().rounded(6).text_size(12).bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : t.muted).hover((s) => s.bg(t.hover)).on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(click).child(svg("mods/herdr-review/icons/annotate.svg").size(14)).child("Annotate")
  );
}

// src/cli.ts
var READ_TIMEOUT = 6e4;
var WRITE_TIMEOUT = 3e4;
function fail(error) {
  throw error;
}
function specArgs(spec2) {
  if (spec2 === null) {
    return [];
  }
  return spec2.kind === "worktree" ? ["--spec", "worktree"] : ["--spec", "branch", "--base", spec2.base];
}
function makeCli(host, bin) {
  let chosen = null;
  async function spawn(args, timeout) {
    const candidates = chosen !== null ? [chosen] : [bin, `${host.appHome()}/mods/herdr-review/bin/herdr-review`, "herdr-review"].filter((c) => c !== null);
    for (const candidate2 of candidates) {
      let result;
      try {
        result = await host.exec(candidate2, args, { timeout });
      } catch {
        continue;
      }
      chosen = candidate2;
      return result;
    }
    return fail({ _tag: "BinaryMissing" });
  }
  function check(result) {
    if (result.code === 0) {
      return result.stdout;
    }
    const stderr = result.stderr.trim();
    if (result.code === 2 && /^unknown (command|option|comment command)/.test(stderr)) {
      return fail({ _tag: "TooOld" });
    }
    if (result.code === 1 && stderr.includes("review is busy")) {
      return fail({ _tag: "Busy" });
    }
    if (result.code === 2) {
      return fail({ _tag: "BadRequest", stderr });
    }
    return fail({ _tag: "BackendFailed", code: result.code, stderr });
  }
  const exec2 = async (args, timeout) => check(await spawn(args, timeout));
  const write3 = async (args) => {
    await exec2(args, WRITE_TIMEOUT);
  };
  return {
    async state(root, spec2) {
      return decodeState(parseJson(await exec2(["state", "--json", "--repo", root, ...specArgs(spec2)], READ_TIMEOUT)));
    },
    async files(root, spec2, paths) {
      const argv = ["files", "--json", "--repo", root, ...specArgs(spec2), ...paths.flatMap((p) => ["--path", p])];
      return decodeFiles(parseJson(await exec2(argv, READ_TIMEOUT)), paths);
    },
    apply: (root, spec2, file) => write3(["comment", "apply", "--as", "user", "--file", file, "--repo", root, ...specArgs(spec2)]),
    reply: (root, id, file) => write3(["comment", "reply", "--as", "user", id, "--body-file", file, "--repo", root]),
    edit: (root, id, file) => write3(["comment", "edit", id, "--body-file", file, "--repo", root]),
    resolve: (root, id) => write3(["comment", "resolve", "--as", "user", id, "--repo", root]),
    reopen: (root, id) => write3(["comment", "reopen", "--as", "user", id, "--repo", root]),
    remove: (root, id) => write3(["comment", "delete", id, "--repo", root]),
    seen: (root, id) => write3(["comment", "seen", id, "--repo", root]),
    async send(root, target, threads) {
      const argv = ["send", "--repo", root, "--json", ...target === null ? ["--backend", "hc"] : ["--target", target], ...threads.flatMap((id) => ["--thread", id])];
      const result = await spawn(argv, WRITE_TIMEOUT);
      return decodeSend(parseJson(result.code === 1 && result.stdout.trim().startsWith("{") ? result.stdout : check(result)));
    },
    async archive(root) {
      return decodeArchive(parseJson(await exec2(["archive", "--repo", root, "--json"], WRITE_TIMEOUT)));
    },
    async message(target) {
      return decodeMessage(parseJson(await exec2(["message", "--json", "--target", target], READ_TIMEOUT)));
    },
    async messageSend(target, file) {
      return decodeMessageSent(parseJson(await exec2(["message", "send", "--target", target, "--file", file], WRITE_TIMEOUT)));
    },
    async theme() {
      return decodeTheme(parseJson(await exec2(["theme", "--json"], READ_TIMEOUT)));
    },
    view: (root, spec2, path, set2) => write3(["view", "--repo", root, "--path", path, set2 ? "--set" : "--unset", ...specArgs(spec2)])
  };
}

// src/host.ts
import { appHome, exec, focused, nextCommand, notify, panes, props, readTextFile, run, terminals, theme, watchFolder, writeTextFile } from "herdr";
var realHost = {
  exec: (command, args, options) => exec(command, args, options),
  readTextFile: (path) => readTextFile(path),
  writeTextFile: (path, text, expected) => writeTextFile(path, text, expected),
  watchFolder: (path) => watchFolder(path),
  notify: (notice) => notify(notice),
  run: (name, args) => run(name, args),
  focused: () => focused(),
  panes: () => panes(),
  terminals: () => terminals(),
  props: () => props(),
  appHome: () => appHome(),
  nextCommand: () => nextCommand(),
  // The setters want `#${string}` and theme() says string, so this is the one cast.
  theme: () => theme()
};

// src/icons.ts
var GENERIC = "file";
var FOLDER = "folder";
var NAMES = /* @__PURE__ */ new Map([
  ["Cargo.toml", "rust"],
  ["Cargo.lock", "rust"],
  ["Dockerfile", "docker"],
  ["Makefile", "makefile"],
  ["LICENSE", "license"],
  [".gitignore", "git"],
  [".gitattributes", "git"]
]);
var group = (icon, extensions) => extensions.split(" ").map((e) => [e, icon]);
var EXTENSIONS = new Map([
  ...group("rust", "rs"),
  ...group("toml", "toml"),
  ...group("lock", "lock"),
  ...group("markdown", "md"),
  ...group("json", "json jsonc"),
  ...group("yaml", "yaml yml"),
  ...group("console", "sh bash zsh fish"),
  ...group("javascript", "js mjs cjs"),
  ...group("react", "jsx"),
  ...group("typescript", "ts mts cts"),
  ...group("react_ts", "tsx"),
  ...group("python", "py"),
  ...group("go", "go"),
  ...group("c", "c"),
  ...group("h", "h"),
  ...group("cpp", "cpp"),
  ...group("hpp", "hpp"),
  ...group("java", "java"),
  ...group("kotlin", "kt"),
  ...group("swift", "swift"),
  ...group("ruby", "rb"),
  ...group("php", "php"),
  ...group("lua", "lua"),
  ...group("nix", "nix"),
  ...group("database", "sql"),
  ...group("vue", "vue"),
  ...group("svelte", "svelte"),
  ...group("astro", "astro"),
  ...group("html", "html"),
  ...group("css", "css"),
  ...group("sass", "scss"),
  ...group("less", "less"),
  ...group("image", "png jpg jpeg gif webp ico"),
  ...group("svg", "svg"),
  ...group("document", "txt"),
  ...group("diff", "patch diff")
]);
function fileIcon(name) {
  const whole = NAMES.get(name);
  if (whole !== void 0) {
    return whole;
  }
  const dot = name.lastIndexOf(".");
  return dot < 0 ? GENERIC : EXTENSIONS.get(name.slice(dot + 1).toLowerCase()) ?? GENERIC;
}
var ICON_NAMES = [.../* @__PURE__ */ new Set([GENERIC, FOLDER, ...NAMES.values(), ...EXTENSIONS.values()])];

// src/rows.ts
function cleanText(text) {
  return text.replace(/\t/g, "    ").replace(/[\u0000-\u001f\u007f-\u009f]/g, "");
}
function bodyLines(text) {
  return text.replace(/\r\n?/g, "\n").split("\n").map(cleanText);
}
var TOKEN_COLORS = {
  comment: "dim",
  string: "ansi2",
  number: "ansi3",
  keyword: "ansi5",
  operator: "ansi7",
  function: "ansi4",
  type: "ansi6"
};
var tokenColor = (kind) => TOKEN_COLORS[kind];
var PLAIN = "foreground";
function runs(text, tokens2, marks2 = []) {
  const out = [];
  const emit = (from, to, color, marked) => {
    const piece = cleanText(text.slice(from, to));
    if (piece !== "") {
      out.push(marked ? { text: piece, color, marked: true } : { text: piece, color });
    }
  };
  const push = (from, to, color) => {
    let at2 = from;
    for (const k of marks2) {
      if (k.to <= at2) {
        continue;
      }
      if (k.from >= to) {
        break;
      }
      const start = Math.max(at2, k.from);
      const stop = Math.min(to, k.to);
      emit(at2, start, color, false);
      emit(start, stop, color, true);
      at2 = stop;
    }
    emit(at2, to, color, false);
  };
  let at = 0;
  for (const t of tokens2) {
    push(at, t.from, PLAIN);
    push(t.from, t.to, tokenColor(t.kind));
    at = t.to;
  }
  push(at, text.length, PLAIN);
  return out;
}
var WARN = "ansi3";
var toneColor = (tone) => tone === "user" ? "accent" : tone === "agent" ? "ansi5" : "ansi2";
var tagColor = (tag) => tag === "unsent" || tag === "new" ? WARN : "dim";
var letterColor = (letter) => letter === "A" || letter === "?" ? "ansi2" : letter === "D" ? "ansi1" : WARN;
var SPLIT_W = 900;
var isSplit = (m) => m.split ?? m.width >= SPLIT_W;
var sideOf = (row2) => row2.kind === "del" ? "old" : "new";
var lineOn = (row2, side) => side === "old" ? row2.old : row2.new;
function pick(file, row2, side = null) {
  const r = file.rows[row2];
  if (r === void 0) {
    throw new Error(`row ${row2} is out of range`);
  }
  const on = side ?? sideOf(r);
  return r.kind === "hunk" || lineOn(r, on) === null ? null : { path: file.path, side: on, from: row2, to: row2, anchor: row2 };
}
function plusTarget(file, sel, row2, side = null) {
  const r = file.rows[row2];
  if (r === void 0) {
    throw new Error(`row ${row2} is out of range`);
  }
  return sel !== null && (side === null || side === sel.side) && sel.path === file.path && row2 >= sel.from && row2 <= sel.to && lineOn(r, sel.side) !== null ? sel : pick(file, row2, side);
}
function span(file, first, second, side = null) {
  const a = file.rows[first];
  if (a === void 0 || file.rows[second] === void 0) {
    throw new Error(`row ${first} or ${second} is out of range`);
  }
  if (a.kind === "hunk") {
    return null;
  }
  const on = [];
  side ??= sideOf(a);
  if (lineOn(a, side) === null) {
    return null;
  }
  for (let i = Math.min(first, second); i <= Math.max(first, second); i++) {
    const r = file.rows[i];
    if (r !== void 0 && lineOn(r, side) !== null) {
      on.push(i);
    }
  }
  const from = on[0];
  const to = on[on.length - 1];
  return from === void 0 || to === void 0 ? null : { path: file.path, side, from, to, anchor: first };
}
function partner(file, row2) {
  const kind = (i) => file.rows[i]?.kind;
  const k = kind(row2);
  if (k === "ctx") {
    return row2;
  }
  if (k !== "del" && k !== "add") {
    return null;
  }
  const edge = (from, step, of) => {
    let i = from;
    while (kind(i + step) === of) {
      i += step;
    }
    return i;
  };
  if (k === "del") {
    const start2 = edge(row2, 1, "del") + 1;
    const at2 = start2 + row2 - edge(row2, -1, "del");
    return kind(start2) === "add" && at2 <= edge(start2, 1, "add") ? at2 : null;
  }
  const start = edge(row2, -1, "add");
  const end = start - 1;
  const at = edge(end, -1, "del") + row2 - start;
  return kind(end) === "del" && at <= end ? at : null;
}
function anchorOf(file, sel) {
  const from = file.rows[sel.from];
  const to = file.rows[sel.to];
  if (from === void 0 || to === void 0) {
    throw new Error("selection is out of range");
  }
  const line2 = lineOn(from, sel.side);
  const end = lineOn(to, sel.side);
  return { path: file.path, old_path: sel.side === "old" ? file.old_path : null, side: sel.side, line: line2, end_line: sel.from === sel.to ? null : end };
}
function batchOf(anchor2, text) {
  const entry = { path: anchor2.path };
  if (anchor2.old_path !== null) {
    entry.old_path = anchor2.old_path;
  }
  if (anchor2.side !== null) {
    entry.side = anchor2.side;
  }
  if (anchor2.line !== null) {
    entry.line = anchor2.line;
  }
  if (anchor2.end_line !== null) {
    entry.end_line = anchor2.end_line;
  }
  entry.body = text;
  return JSON.stringify({ comments: [entry] });
}
var lines = (from, to) => from === null ? "" : to === null || to === from ? String(from) : `${from}-${to}`;
function anchorLabel(a) {
  return a.side === null || a.line === null ? cleanText(a.path) : `${cleanText(a.path)} ${a.side === "old" ? "L" : "R"}${lines(a.line, a.end_line)}`;
}
function selectionLabel(file, sel) {
  const a = anchorOf(file, sel);
  const count = a.line === null ? 1 : (a.end_line ?? a.line) - a.line + 1;
  return `${anchorLabel(a)} (${count} ${count === 1 ? "line" : "lines"})`;
}
function sameRows(before, after, sel) {
  if (before === null || before.path !== sel.path || after.path !== sel.path) {
    return false;
  }
  for (let i = sel.from; i <= sel.to; i++) {
    const a = before.rows[i];
    const b = after.rows[i];
    if (a === void 0 || b === void 0 || a.kind !== b.kind || a.old !== b.old || a.new !== b.new || a.text !== b.text) {
      return false;
    }
  }
  return true;
}
function findComment(files, state, id) {
  if (id === null) {
    return null;
  }
  for (const thread2 of [...[...files.values()].flatMap((f) => f.threads), ...state === null ? [] : state.orphans]) {
    const comment2 = thread2.comments.find((c) => c.id === id);
    if (comment2 !== void 0) {
      return { thread: thread2, comment: comment2 };
    }
  }
  return null;
}
function cardOrder(file) {
  const row2 = (t) => t.placement.row ?? -1;
  return [...file.threads].sort((a, b) => row2(a) - row2(b)).map((t) => t.id);
}
var summaryOf = (m, path) => m.state?.files.find((f) => f.path === path);
var isFolded = (m, f) => m.folds.get(f.path) ?? f.viewed;
function stops(m) {
  const all = (m.state === null ? [] : m.state.orphans).map((t) => ({ id: t.id }));
  let top = 0;
  for (const f of m.state === null ? [] : m.state.files) {
    if (m.selected === f.path) {
      top = all.length;
    }
    const view = m.files.get(f.path);
    if (view !== void 0 && !isFolded(m, f)) {
      all.push(...cardOrder(view).map((id) => ({ id })));
    } else if (f.threads > 0) {
      all.push({ path: f.path });
    }
  }
  return { all, top };
}
var noStopsMemo = /* @__PURE__ */ new WeakMap();
function noStops(m) {
  const kept = noStopsMemo.get(m.files);
  if (kept !== void 0 && kept.state === m.state && kept.folds === m.folds) {
    return kept.none;
  }
  const files = m.state === null ? [] : m.state.files;
  const none = (m.state === null || m.state.orphans.length === 0) && files.every((f) => {
    const view = m.files.get(f.path);
    return view !== void 0 && !isFolded(m, f) ? view.threads.length === 0 : f.threads === 0;
  });
  noStopsMemo.set(m.files, { state: m.state, folds: m.folds, none });
  return none;
}
function jumpStop(m, forward) {
  const { all, top } = stops(m);
  if (all.length === 0) {
    return null;
  }
  const hit = findComment(m.files, m.state, m.focus);
  const at = hit === null ? -1 : all.findIndex((s) => "id" in s && s.id === hit.thread.id);
  const from = at >= 0 ? forward ? at + 1 : at - 1 : forward ? top : top - 1;
  return all[(from + all.length) % all.length] ?? null;
}
function panelKey(m, keystroke) {
  if (m.editor !== null || m.picker !== null || m.confirm !== null || m.confirmArchive) {
    return null;
  }
  if (m.view === "message") {
    return null;
  }
  if (m.filtering) {
    return keystroke === "escape" ? { _tag: "FilterCleared" } : null;
  }
  if (keystroke === "/") {
    return { _tag: "FilterOpened" };
  }
  if (keystroke === "escape") {
    return m.selection !== null ? { _tag: "SelectionCleared" } : m.filter === "" ? null : { _tag: "FilterCleared" };
  }
  if (keystroke === "n") {
    return { _tag: "ThreadJumped", forward: true };
  }
  if (keystroke === "shift-n" || keystroke === "N") {
    return { _tag: "ThreadJumped", forward: false };
  }
  return cursorKey(m, keystroke);
}
function spots(m) {
  const out = [];
  const split = isSplit(m);
  for (const f of m.state === null ? [] : m.state.files) {
    const file = m.files.get(f.path);
    if (file === void 0 || isFolded(m, f)) {
      continue;
    }
    let seen = /* @__PURE__ */ new Set();
    const put = (row2, side) => {
      out.push({ path: f.path, row: row2, side, first: !seen.has(side) });
      seen.add(side);
    };
    const kind = (i) => file.rows[i]?.kind;
    for (let i = 0; i < file.rows.length; ) {
      if (kind(i) === "hunk") {
        seen = /* @__PURE__ */ new Set();
        i++;
      } else if (!split) {
        put(i++, null);
      } else if (kind(i) === "ctx") {
        put(i, "old");
        put(i++, "new");
      } else {
        let mid = i;
        while (kind(mid) === "del") {
          mid++;
        }
        let end = mid;
        while (kind(end) === "add") {
          end++;
        }
        for (let p = 0; p < Math.max(mid - i, end - mid); p++) {
          if (i + p < mid) {
            put(i + p, "old");
          }
          if (mid + p < end) {
            put(mid + p, "new");
          }
        }
        i = end;
      }
    }
  }
  return out;
}
var sameSpot = (s, c) => s.path === c.path && s.row === c.row && s.side === c.side;
var cursorOn = (m, item2, side = null) => m.cursor !== null && item2.kind !== "hunk" && m.cursor.path === keyPath(item2.key) && m.cursor.row === item2.index && m.cursor.side === side;
function cursorSpot(m) {
  const c = m.cursor;
  return c === null ? null : spots(m).find((s) => sameSpot(s, c)) ?? null;
}
var visualOn = (m) => m.visual && m.selection !== null && m.cursor !== null && m.selection.path === m.cursor.path;
function lane(m, all) {
  const c = m.cursor;
  const own = visualOn(m);
  return c === null ? all : all.filter((s) => s.side === c.side && (!own || s.path === c.path));
}
function topSpot(m, all) {
  const home = all.filter((s) => s.side !== "old" || m.files.get(s.path)?.rows[s.row]?.kind === "del");
  const path = m.top === null ? "" : keyPath(m.top);
  const at = m.top === null ? null : /^[rp](\d+)$/.exec(keyLocal(m.top));
  const row2 = at === null ? 0 : Number(at[1]);
  return home.find((s) => s.path === path && s.row >= row2) ?? home.find((s) => m.visible.includes(s.path)) ?? home[0] ?? null;
}
var THREAD_KEYS = { r: ["reply"], e: ["edit"], d: ["delete"], x: ["resolve", "reopen"] };
function cursorKey(m, key) {
  const half = Math.max(1, Math.floor(m.height / ROW / 2));
  switch (key) {
    case "j":
    case "down":
      return { _tag: "CursorMoved", by: 1 };
    case "k":
    case "up":
      return { _tag: "CursorMoved", by: -1 };
    case "ctrl-d":
    case "pagedown":
      return { _tag: "CursorMoved", by: half };
    case "ctrl-u":
    case "pageup":
      return { _tag: "CursorMoved", by: -half };
    case "]":
      return { _tag: "CursorHunk", forward: true };
    case "[":
      return { _tag: "CursorHunk", forward: false };
    case "h":
    case "left":
      return isSplit(m) ? { _tag: "CursorSide", side: "old" } : null;
    case "l":
    case "right":
      return isSplit(m) ? { _tag: "CursorSide", side: "new" } : null;
    case "v":
      return { _tag: "VisualToggled" };
    case "c": {
      const at = cursorSpot(m);
      return at === null ? { _tag: "CursorMoved", by: 0 } : { _tag: "PlusClicked", path: at.path, row: at.row, side: at.side };
    }
    case "r":
    case "e":
    case "d":
    case "x": {
      const hit = m.selection === null ? findComment(m.files, m.state, m.focus) : null;
      return hit === null ? null : cardButtons(hit.thread, hit.comment, true).find((b) => (THREAD_KEYS[key] ?? []).includes(b.id))?.msg ?? null;
    }
    default:
      return null;
  }
}
function ageText(at, now) {
  const t = Date.parse(at);
  if (Number.isNaN(t)) {
    return null;
  }
  const s = Math.max(0, Math.floor((now - t) / 1e3));
  return s < 60 ? "now" : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`;
}
var who = (author, you) => author === "user" ? you : author.startsWith("agent:") ? cleanText(author.slice(6)) : "agent";
function cardButtons(thread2, comment2, last) {
  const out = [];
  const add = (id, label, icon, msg, key, danger = false) => void out.push({ id, label, msg, danger, off: false, icon, iconOnly: true, key });
  add("reply", "Reply", "reply", { _tag: "ThreadAction", id: comment2.id, action: "reply" }, "r");
  if (last) {
    if (thread2.status === "open") {
      add("resolve", "Resolve", "check", { _tag: "ThreadAction", id: comment2.id, action: "resolve" }, "x");
    } else {
      add("reopen", "Reopen", "undo", { _tag: "ThreadAction", id: comment2.id, action: "reopen" }, "x");
    }
    if (thread2.status === "open" && !thread2.unsent) {
      add("resend", "Resend", "send", { _tag: "SendClicked", threads: [thread2.id] });
    }
  }
  if (comment2.author === "user") {
    add("edit", "Edit", "pencil", { _tag: "ThreadAction", id: comment2.id, action: "edit" }, "e");
    add("delete", "Delete", "trash", { _tag: "ThreadAction", id: comment2.id, action: "delete" }, "d", true);
  }
  return out;
}
function threadEditor(out, path, thread2, m, half) {
  const e = m.editor;
  if (m.view !== "message" && m.draft === null && e !== null && e.kind !== "comment" && e.thread === thread2.id) {
    out.push(editorItem(path, e, half));
  }
}
var DRAFT = "draft";
var draftComment = (m, now, parent) => ({ id: DRAFT, parent, author: "user", at: new Date(now).toISOString(), body: m.draft ?? "", sent: false, edited_since_sent: false });
function draftThread(m, a, row2, now) {
  const kind = a.line === null ? "file" : a.end_line !== null ? "range" : "line";
  return {
    id: DRAFT,
    status: "open",
    resolved_by: null,
    is_new: false,
    unsent: true,
    placement: { kind: "matched", row: row2, start_row: null },
    anchor: { path: a.path, old_path: a.old_path, kind, side: a.side, line: a.line, end_line: a.end_line, text: null, spec: m.spec ?? { kind: "worktree" } },
    comments: [draftComment(m, now, null)]
  };
}
function withDraft(thread2, m, now) {
  const e = m.editor;
  if (m.draft === null || m.view === "message" || e === null || e.thread !== thread2.id) {
    return thread2;
  }
  if (e.kind === "reply") {
    return { ...thread2, unsent: true, comments: [...thread2.comments, draftComment(m, now, thread2.id)] };
  }
  if (e.kind === "edit") {
    return { ...thread2, comments: thread2.comments.map((c) => c.id === e.comment ? { ...c, body: m.draft ?? c.body, edited_since_sent: c.sent || c.edited_since_sent } : c) };
  }
  return thread2;
}
var editorItem = (path, e, half) => ({ kind: "editor", key: itemKey(path, "editor"), title: editorTitle(e), half });
function cardItems(out, path, given, m, now, half = null) {
  const thread2 = withDraft(given, m, now);
  const root = thread2.comments[0];
  if (root === void 0) {
    throw new Error(`thread ${thread2.id} has no comments`);
  }
  const resolved = thread2.status === "resolved";
  const open2 = !resolved || m.expanded.has(thread2.id);
  const tone = resolved ? "resolved" : root.author === "user" ? "user" : "agent";
  const age = ageText(root.at, now);
  const badges = [];
  if (!resolved && thread2.placement.kind === "outdated") {
    badges.push("outdated");
  }
  if (resolved) {
    badges.push(`resolved by ${thread2.resolved_by === null ? "?" : who(thread2.resolved_by, "you")}`);
  }
  if (thread2.is_new) {
    badges.push("new");
  }
  if (thread2.comments.some((c) => c.edited_since_sent)) {
    badges.push("edited since sent");
  }
  if (thread2.unsent) {
    badges.push("unsent");
  }
  const outdated = !resolved && thread2.placement.kind !== "matched" && thread2.anchor.text !== null;
  out.push({
    kind: "head",
    key: itemKey(path, `h:${thread2.id}`),
    id: thread2.id,
    tone,
    title: `${resolved ? "\u2713" : "\u25CF"} ${who(root.author, "Your note")}${age === null ? "" : ` \xB7 ${age}`} \xB7 ${anchorLabel(thread2.anchor)}`,
    badges,
    quote: outdated ? cleanText(thread2.anchor.text ?? "") : null,
    collapsible: resolved,
    open: open2,
    focused: m.focus === thread2.id,
    last: !open2,
    half
  });
  if (open2) {
    thread2.comments.forEach((c, i) => {
      const when = ageText(c.at, now);
      out.push({
        kind: "body",
        key: itemKey(path, `c:${c.id}`),
        id: c.id,
        tone,
        meta: c.parent === null ? null : `${who(c.author, "You")}${when === null ? "" : ` \xB7 ${when}`}`,
        lines: bodyLines(c.body),
        buttons: cardButtons(thread2, c, i === thread2.comments.length - 1),
        focused: m.focus === c.id,
        last: i === thread2.comments.length - 1,
        half
      });
    });
  }
  threadEditor(out, path, thread2, m, half);
}
var itemKey = (path, local) => `${path}|${local}`;
var keyPath = (key) => key.slice(0, key.lastIndexOf("|"));
var keyLocal = (key) => key.slice(key.lastIndexOf("|") + 1);
var ROW = 20;
var CARD_W = 560;
var FILE_H = 30;
var LABEL_H = 30;
var NOTE_H = 24;
var CHAR_W = 7.5;
var TEXT_H = 18;
var BTN_H = 24;
var BTN_GAP = 6;
var EDITOR_TEXT = 108;
var EDITOR_H = 12 + 12 + 16 + 4 + EDITOR_TEXT + 4 + 16 + 4 + 24;
function buttonsHeight(buttons, avail) {
  if (buttons.length === 0) {
    return 0;
  }
  let rows = 1;
  let used = 0;
  for (const b of buttons) {
    const w = b.iconOnly === true ? BTN_H : b.label.length * CHAR_W + 18;
    if (used > 0 && used + BTN_GAP + w > avail) {
      rows++;
      used = w;
    } else {
      used += (used > 0 ? BTN_GAP : 0) + w;
    }
  }
  return 4 + rows * BTN_H + (rows - 1) * BTN_GAP;
}
var cardWidth = (width) => Math.max(160, Math.min(CARD_W, width - 60));
var halfCardWidth = (width) => Math.max(160, Math.min(CARD_W, (width - 1) / 2 - 40));
function heightOf(i, width) {
  switch (i.kind) {
    case "hunk":
    case "ctx":
    case "add":
    case "del":
    case "split":
      return ROW;
    case "label":
      return LABEL_H;
    case "file":
      return FILE_H;
    case "note":
      return NOTE_H;
    case "wait":
      return i.height;
    case "editor":
      return EDITOR_H;
    case "head":
      return 34 + (i.quote === null ? 0 : 18);
    case "body": {
      const avail = (i.half === null ? cardWidth(width) : halfCardWidth(width)) - (i.meta === null ? 22 : 34);
      const cols = Math.max(8, Math.floor(avail / CHAR_W));
      const wrapped = i.lines.reduce((n, line2) => n + Math.max(1, Math.ceil(line2.length / cols)), 0);
      return 6 + (i.meta === null ? 0 : 16) + wrapped * TEXT_H + buttonsHeight(i.buttons, avail) + (i.last ? 6 : 0);
    }
    default: {
      const unhandled = i;
      throw new Error(`unhandled item ${JSON.stringify(unhandled)}`);
    }
  }
}
function estimateRows(f) {
  if (f.change === "binary" || f.change === "submodule" || f.change === "too_large" || f.change === "unparsed") {
    return 1;
  }
  return f.add + f.del + (f.change === "added" || f.change === "untracked" || f.change === "deleted" ? 2 : 8);
}
var memo = /* @__PURE__ */ new WeakMap();
var rowMemo = /* @__PURE__ */ new WeakMap();
function rowsOf(file) {
  const kept = rowMemo.get(file);
  if (kept !== void 0) {
    return kept;
  }
  const rows = file.rows.map((row2, index) => ({ kind: row2.kind, key: itemKey(file.path, `r${index}`), index, old: row2.old === null ? "" : String(row2.old), new: row2.new === null ? "" : String(row2.new), text: cleanText(row2.text), runs: runs(row2.text, row2.tokens, row2.marks) }));
  rowMemo.set(file, rows);
  return rows;
}
function editorRow(file, a) {
  const line2 = a.end_line ?? a.line;
  if (a.side === null || line2 === null) {
    return null;
  }
  for (let i = 0; i < file.rows.length; i++) {
    if (lineOn(file.rows[i], a.side) === line2) {
      return i;
    }
  }
  return null;
}
function fileStream(m, file, now) {
  const ids = new Set(file.threads.flatMap((t) => [t.id, ...t.comments.map((c) => c.id)]));
  const ed = m.view !== "message" && m.editor !== null && m.editor.kind === "comment" && m.editor.anchor?.path === file.path ? m.editor : null;
  const at = ed?.anchor == null ? null : editorRow(file, ed.anchor);
  const split = isSplit(m);
  const dep = [
    file.threads.filter((t) => m.expanded.has(t.id)).map((t) => t.id).join(","),
    m.focus !== null && ids.has(m.focus) ? m.focus : "",
    Math.floor(now / 6e4),
    m.width,
    split,
    ed === null ? "" : `${editorTitle(ed)}|${at}`,
    m.view !== "message" && m.editor?.thread != null && ids.has(m.editor.thread) ? `${m.editor.kind}|${m.editor.thread}` : "",
    (ed !== null || m.editor?.thread != null && ids.has(m.editor.thread)) && m.draft !== null ? m.draft : ""
  ].join("|");
  const kept = memo.get(file);
  if (kept !== void 0 && kept.dep === dep) {
    return kept.stream;
  }
  const items = [];
  const under = /* @__PURE__ */ new Map();
  const top = [];
  for (const t of file.threads) {
    const row2 = t.placement.row;
    if (row2 === null) {
      top.push(t);
    } else if (file.rows[row2] === void 0) {
      throw new Error(`thread ${t.id} is placed at row ${row2}, past the end`);
    } else {
      under.set(row2, [...under.get(row2) ?? [], t]);
    }
  }
  if (file.note !== null) {
    items.push({ kind: "note", key: itemKey(file.path, "note"), text: cleanText(file.note) });
  }
  if (ed !== null && at === null && m.draft === null) {
    items.push(editorItem(file.path, ed, null));
  }
  for (const t of top) {
    cardItems(items, file.path, t, m, now);
  }
  if (ed !== null && at === null && m.draft !== null && ed.anchor !== null) {
    cardItems(items, file.path, draftThread(m, ed.anchor, null, now), m, now);
  }
  const base = rowsOf(file);
  const cards = (...rows) => {
    for (const index of new Set(rows.flatMap((r) => r === null ? [] : [r.index]))) {
      for (const t of under.get(index) ?? []) {
        cardItems(items, file.path, t, m, now, split ? t.anchor.side : null);
      }
    }
    if (ed !== null && at !== null && rows.some((r) => r?.index === at)) {
      const half = split ? ed.anchor?.side ?? null : null;
      if (m.draft === null || ed.anchor === null) {
        items.push(editorItem(file.path, ed, half));
      } else {
        cardItems(items, file.path, draftThread(m, ed.anchor, at, now), m, now, half);
      }
    }
  };
  if (!split) {
    base.forEach((row2) => {
      items.push(row2);
      cards(row2);
    });
  } else {
    const kindAt = (i) => base[i]?.kind;
    for (let i = 0; i < base.length; ) {
      const row2 = base[i];
      if (row2.kind === "hunk" || row2.kind === "ctx") {
        items.push(row2.kind === "hunk" ? row2 : { kind: "split", key: itemKey(file.path, `p${i}`), left: row2, right: row2 });
        cards(row2);
        i++;
        continue;
      }
      let mid = i;
      while (kindAt(mid) === "del") {
        mid++;
      }
      let end = mid;
      while (kindAt(end) === "add") {
        end++;
      }
      for (let p = 0; p < Math.max(mid - i, end - mid); p++) {
        const left = i + p < mid ? base[i + p] : null;
        const right = mid + p < end ? base[mid + p] : null;
        items.push({ kind: "split", key: itemKey(file.path, `p${(left ?? right).index}`), left, right });
        cards(left, right);
      }
      i = end;
    }
  }
  const stream = { items, sizes: items.map((i) => heightOf(i, m.width)) };
  memo.set(file, { dep, stream });
  return stream;
}
var viewedClick = (f) => ({ _tag: "ViewedClicked", path: f.path, viewed: !f.viewed });
function fileHead(m, f, folded) {
  const hides = folded && f.threads > 0 ? plural(f.threads, "thread", "threads") : "";
  return { kind: "file", key: itemKey(f.path, "file"), path: f.path, title: cleanText(f.path), letter: LETTERS[f.change], add: f.add, del: f.del, viewed: f.viewed, folded, hides };
}
function streamLayout(m, now) {
  const items = [];
  const sizes = [];
  const add = (more, heights = more.map((i) => heightOf(i, m.width))) => {
    items.push(...more);
    sizes.push(...heights);
  };
  const orphans = m.state === null ? [] : m.state.orphans;
  if (orphans.length > 0) {
    const out = [{ kind: "label", key: itemKey("", "l:orphans"), text: "Comments not in this diff" }];
    for (const t of orphans) {
      cardItems(out, "", t, m, now);
    }
    add(out);
  }
  for (const f of m.state === null ? [] : m.state.files) {
    const folded = isFolded(m, f);
    add([fileHead(m, f, folded)]);
    if (folded) {
      continue;
    }
    const view = m.files.get(f.path);
    if (view !== void 0) {
      const s = fileStream(m, view, now);
      add(s.items, s.sizes);
    } else {
      const rows = estimateRows(f);
      add([{ kind: "wait", key: itemKey(f.path, "wait"), text: m.skipped.has(f.path) ? "Could not load this file. Refresh to try again" : "Loading...", height: rows * ROW }]);
    }
  }
  const starts = [0];
  for (const size of sizes) {
    starts.push(starts[starts.length - 1] + size);
  }
  return { items, sizes, starts, total: starts[starts.length - 1], wide: isSplit(m) ? 0 : diffWidth(items) };
}
function layoutCache(onRebuild = () => {
}) {
  let deps = [];
  let kept = null;
  return (m, now) => {
    const next = [m.state, m.files, m.folds, m.expanded, m.focus, m.width, isSplit(m), m.skipped, m.editor, m.draft, m.view, Math.floor(now / 6e4)];
    if (kept !== null && next.every((d, i) => d === deps[i])) {
      return kept;
    }
    deps = next;
    onRebuild();
    kept = streamLayout(m, now);
    return kept;
  };
}
var maxOffset = (total, height) => Math.max(0, total - height);
var clampOffset = (offset, total, height) => Math.min(maxOffset(total, height), Math.max(0, offset));
function itemAt(starts, y) {
  let lo = 0;
  let hi = Math.max(0, starts.length - 2);
  while (lo < hi) {
    const mid = lo + hi + 1 >> 1;
    if (starts[mid] <= y) {
      lo = mid;
    } else {
      hi = mid - 1;
    }
  }
  return lo;
}
function visibleRange(stream, offset, height) {
  const n = stream.items.length;
  if (n === 0) {
    return { first: 0, end: 0, shift: 0 };
  }
  const at = clampOffset(offset, stream.total, height);
  const first = itemAt(stream.starts, at);
  let end = first + 1;
  while (end < n && stream.starts[end] < at + height) {
    end++;
  }
  return { first, end, shift: at - stream.starts[first] };
}
function revealOffset(stream, reveal, offset, height) {
  const at = indexOfKey(stream.items, reveal.key);
  if (at < 0) {
    return null;
  }
  const top = stream.starts[at];
  const bottom = stream.starts[at + 1];
  const want = reveal.near !== true ? top : top < offset ? top : bottom > offset + height ? bottom - height : offset;
  return clampOffset(want, stream.total, height);
}
function scrolled(m, stream) {
  const target = m.reveal === null ? null : revealOffset(stream, m.reveal, m.offset, m.height);
  const offset = clampOffset(target ?? m.offset, stream.total, m.height);
  const { first, end } = visibleRange(stream, offset, m.height);
  const top = stream.items[first]?.key ?? null;
  const visible = pathsOf(stream.items, first, end);
  const fresh = freshOn(stream.items, first, end);
  const same = (a, b) => a.length === b.length && a.every((p, i) => p === b[i]);
  if (target === null && offset === m.offset && stream.total === m.total && top === m.top && same(visible, m.visible) && same(fresh, m.fresh)) {
    return null;
  }
  return { _tag: "Scrolled", offset, total: stream.total, top, visible, fresh, revealed: target !== null };
}
function wheelShift(e) {
  if (Math.abs(e.delta.x) <= Math.abs(e.delta.y)) {
    return null;
  }
  return -(e.delta_lines !== void 0 ? e.delta_lines.x * 4 * CODE_CHAR : e.delta.x);
}
var maxShift = (code, width) => Math.max(0, code - ((width - 1) / 2 - 88));
function wheelDelta(e) {
  if (Math.abs(e.delta.x) > Math.abs(e.delta.y)) {
    return null;
  }
  const dy = e.delta_lines !== void 0 ? e.delta_lines.y * ROW : e.delta.y;
  return dy === 0 ? 0 : -dy;
}
var THUMB_MIN = 24;
function thumbOf(total, height, offset) {
  if (total <= height || height <= 0) {
    return null;
  }
  const size = Math.max(THUMB_MIN, height * height / total);
  const room = Math.max(1, height - size);
  return { top: clampOffset(offset, total, height) / maxOffset(total, height) * room, size };
}
function offsetOfThumb(total, height, top) {
  const thumb = thumbOf(total, height, 0);
  if (thumb === null) {
    return 0;
  }
  return clampOffset(top / Math.max(1, height - thumb.size) * maxOffset(total, height), total, height);
}
function freshOn(items, start, end) {
  return items.slice(Math.max(0, start), Math.max(0, end)).flatMap((i) => i.kind === "head" && i.tone === "resolved" && i.badges.includes("new") ? [i.id] : []);
}
function pathsOf(items, start, end) {
  const out = [];
  for (const i of items.slice(Math.max(0, start), Math.max(0, end))) {
    const path = keyPath(i.key);
    if (path !== "" && !out.includes(path)) {
      out.push(path);
    }
  }
  return out;
}
var AHEAD = 3;
var BATCH = 3;
function wanted(m) {
  const files = m.state === null ? [] : m.state.files;
  const on = m.reveal !== null ? [keyPath(m.reveal.key)] : m.visible.length > 0 ? m.visible : m.selected === null ? [] : [m.selected];
  const idx = on.map((p) => files.findIndex((f) => f.path === p)).filter((i) => i >= 0);
  const lo = idx.length === 0 ? 0 : Math.min(...idx);
  const hi = idx.length === 0 ? 0 : Math.max(...idx);
  const order = [];
  const range2 = (from, to) => {
    for (let i = from; i <= to; i++) {
      order.push(i);
    }
  };
  range2(lo, Math.min(files.length - 1, hi + AHEAD));
  range2(hi + AHEAD + 1, files.length - 1);
  range2(0, lo - 1);
  return order.flatMap((i) => {
    const f = files[i];
    return f !== void 0 && !isFolded(m, f) && !m.skipped.has(f.path) && (!m.files.has(f.path) || m.dirty.has(f.path)) ? [f.path] : [];
  });
}
function cardKey(m, id) {
  if ((m.state === null ? [] : m.state.orphans).some((t) => t.id === id)) {
    return itemKey("", `h:${id}`);
  }
  for (const [path, file] of m.files) {
    if (file.threads.some((t) => t.id === id)) {
      return itemKey(path, `h:${id}`);
    }
  }
  return null;
}
var holds = (i, n) => i.kind === "split" ? i.left?.index === n || i.right?.index === n : (i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del") && i.index === n;
function indexOfKey(items, key) {
  const at = items.findIndex((i) => i.key === key);
  const row2 = /^[rp](\d+)$/.exec(keyLocal(key));
  if (at >= 0 || row2 === null) {
    return at;
  }
  const path = keyPath(key);
  return items.findIndex((i) => keyPath(i.key) === path && holds(i, Number(row2[1])));
}
var GUTTER_W = 24;
var PLUS = "accent";
function rowState(m, item2, side = null) {
  const path = keyPath(item2.key);
  const sel = m.selection;
  const onSide = sel === null ? false : (sel.side === "old" ? item2.old : item2.new) !== "";
  return {
    selected: sel !== null && sel.path === path && item2.index >= sel.from && item2.index <= sel.to && onSide && (side === null || sel.side === side),
    hovered: item2.kind !== "hunk" && m.hover !== null && m.hover.path === path && m.hover.row === item2.index && m.hover.side === side
  };
}
function itemClick(key) {
  const path = keyPath(key);
  const local = keyLocal(key);
  if (local === "file") {
    return { _tag: "FoldToggled", path };
  }
  if (local.startsWith("h:")) {
    return { _tag: "ThreadAction", id: local.slice(2), action: "toggle" };
  }
  if (local.startsWith("c:")) {
    return { _tag: "ThreadAction", id: local.slice(2), action: "focus" };
  }
  return null;
}
function editorKey(keystroke, text) {
  if (keystroke === "ctrl-s" || keystroke === "cmd-s") {
    return { _tag: "EditorSaved", text };
  }
  if (keystroke === "escape") {
    return { _tag: "EditorCancelled" };
  }
  return null;
}
function editorTitle(e) {
  if (e.title !== void 0) {
    return e.title;
  }
  if (e.kind === "reply") {
    return `Reply to ${e.thread ?? ""}`;
  }
  if (e.kind === "edit") {
    return `Edit ${e.comment ?? ""}`;
  }
  return `Draft note - ${e.anchor === null ? "" : anchorLabel(e.anchor)}`;
}
var plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
var archivableCount = (state) => state === null ? 0 : state.archivable;
var unreadCount = (state) => state === null ? 0 : state.unread_resolved;
var unsentMemo = /* @__PURE__ */ new WeakMap();
function unsentCount(state) {
  if (state === null) {
    return 0;
  }
  let n = unsentMemo.get(state);
  if (n === void 0) {
    n = state.files.reduce((sum, f) => sum + f.unsent, 0) + state.orphans.filter((t) => t.unsent).length;
    unsentMemo.set(state, n);
  }
  return n;
}
function pickerOptions(candidates) {
  const option = (c) => ({
    id: `pick-${c.address}`,
    label: cleanText(`${c.address}  ${c.agent}  ${c.status}  ${c.cwd}`),
    msg: { _tag: "PickerChosen", target: c.address },
    danger: false,
    off: false
  });
  return [...candidates.map(option), { id: "pick-cancel", label: "Cancel", msg: { _tag: "PickerCancelled" }, danger: false, off: false }];
}
function confirmRow(m) {
  const cancel = { id: "confirm-cancel", label: "Cancel", msg: { _tag: "ConfirmCancelled" }, danger: false, off: false };
  if (m.confirm !== null) {
    const hit = findComment(m.files, m.state, m.confirm);
    if (hit === null) {
      return null;
    }
    const replies = hit.comment.parent === null && hit.thread.comments.length > 1;
    const ok2 = { id: "confirm-delete", label: "Delete", msg: { _tag: "DeleteConfirmed", id: m.confirm }, danger: true, off: false };
    return { sentence: replies ? "Delete this comment? Its replies go too." : "Delete this comment?", color: "ansi1", buttons: [ok2, cancel] };
  }
  if (!m.confirmArchive) {
    return null;
  }
  const ok = { id: "confirm-archive", label: "Archive", msg: { _tag: "ArchiveConfirmed" }, danger: false, off: m.inFlight, accent: true };
  const unread = unreadCount(m.state);
  const more = unread === 0 ? "" : ` ${unread} more resolved by the agent ${unread === 1 ? "is" : "are"} unread.`;
  return { sentence: `Archive ${plural(archivableCount(m.state), "resolved thread", "resolved threads")}?${more}`, color: "foreground", buttons: [ok, cancel] };
}
function confirmKey(keystroke) {
  return keystroke === "escape" ? { _tag: "ConfirmCancelled" } : null;
}
function dock(m) {
  const buttons = [];
  const add = (id, label2, icon, msg) => void buttons.push({ id, label: label2, msg, danger: false, off: false, icon });
  const hit = findComment(m.files, m.state, m.focus);
  const picked = m.selection === null ? null : m.files.get(m.selection.path) ?? null;
  let label = "Drag over lines to select them, [+] to comment";
  if (picked !== null && m.selection !== null) {
    label = `${selectionLabel(picked, m.selection)}. [+] to comment, Esc to clear`;
  } else if (hit !== null) {
    const { thread: thread2, comment: comment2 } = hit;
    label = `${thread2.id}${comment2.id === thread2.id ? "" : ` / ${comment2.id}`}`;
  }
  const shown = m.selected === null ? void 0 : summaryOf(m, m.selected);
  if (shown !== void 0) {
    add("file", "Comment on file", "comment-plus", { _tag: "FileCommentClicked" });
  }
  const unsent = unsentCount(m.state);
  const none = noStops(m);
  const archivable = archivableCount(m.state);
  const unread = unreadCount(m.state);
  const note = archivable === 0 && unread > 0 ? { text: `Nothing to archive yet: ${unread} resolved by the agent ${unread === 1 ? "is" : "are"} unread (marked new)`, color: "dim" } : null;
  const actions = [
    { id: "prev-thread", label: "Previous comment", icon: "arrow-up", iconOnly: true, key: "shift+n", tip: none ? "No comments yet" : void 0, msg: { _tag: "ThreadJumped", forward: false }, danger: false, off: none },
    { id: "next-thread", label: "Next comment", icon: "arrow-down", iconOnly: true, key: "n", tip: none ? "No comments yet" : void 0, msg: { _tag: "ThreadJumped", forward: true }, danger: false, off: none },
    { id: "send", label: unsent > 0 ? `Send ${unsent}` : "Send", icon: "send", accent: unsent > 0, tip: m.sending ? "Sending..." : unsent > 0 ? `Send ${plural(unsent, "unsent comment", "unsent comments")}` : "No unsent comments", msg: { _tag: "SendClicked" }, danger: false, off: m.sending || unsent === 0 },
    {
      id: "archive",
      label: archivable > 0 ? `Archive ${archivable}` : "Archive",
      icon: "archive",
      tip: archivable > 0 ? `Archive ${plural(archivable, "resolved thread", "resolved threads")}` : note?.text ?? "No resolved threads to archive",
      msg: { _tag: "ArchiveClicked" },
      danger: false,
      off: m.inFlight || archivable === 0
    }
  ];
  return { label, buttons, actions, note };
}
var CODE_CHAR = 7;
function codeWidth(items) {
  let wide = 0;
  for (const i of items) {
    if (i.kind === "split") {
      wide = Math.max(wide, (i.left?.text.length ?? 0) * CODE_CHAR, (i.right?.text.length ?? 0) * CODE_CHAR);
    }
  }
  return wide;
}
function diffWidth(items) {
  let wide = 300;
  for (const i of items) {
    if (i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del") {
      wide = Math.max(wide, i.text.length * CODE_CHAR + 150);
    }
  }
  return wide;
}
var LETTERS = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "?",
  binary: "B",
  submodule: "S",
  too_large: "L",
  unparsed: "U"
};
var badgeColor = (b) => b.tone === "unsent" ? WARN : b.tone === "open" ? "accent" : b.tone === "new" ? "done" : "dim";
function fileRow(f) {
  const badges = [];
  if (f.unsent > 0) {
    badges.push({ text: `${f.unsent} unsent`, tone: "unsent" });
  }
  if (f.open > 0) {
    badges.push({ text: `${f.open} open`, tone: "open" });
  }
  if (f.new > 0) {
    badges.push({ text: `${f.new} new`, tone: "new" });
  }
  if (f.viewed) {
    badges.push({ text: "viewed", tone: "viewed" });
  }
  const name = f.path.slice(f.path.lastIndexOf("/") + 1);
  return { kind: "file", key: `f:${f.path}`, path: f.path, name: cleanText(name), icon: fileIcon(name), letter: LETTERS[f.change], add: f.add, del: f.del, viewed: f.viewed, badges };
}
function fileMatches(query, path) {
  const fold = !/\p{Uppercase}/u.test(query);
  const chars = [...path].map((c) => fold ? [...c.toLowerCase()][0] ?? c : c);
  let at = 0;
  for (const wanted2 of query) {
    const found = chars.indexOf(wanted2, at);
    if (found < 0) {
      return false;
    }
    at = found + 1;
  }
  return true;
}
function filterCount(files, query) {
  return sidebarOf(files, query).count;
}
function filterPick(m) {
  const hits = sidebarOf(m.state === null ? NO_FILES : m.state.files, m.filter).hits;
  return hits.length === 0 || hits.some((f) => f.path === m.selected) ? null : hits[0]?.path ?? null;
}
function sidebarRows(files, query = "") {
  return sidebarOf(files, query).rows;
}
var NO_FILES = [];
var sidebarMemo = /* @__PURE__ */ new WeakMap();
function sidebarOf(files, query) {
  const kept = sidebarMemo.get(files);
  if (kept !== void 0 && kept.query === query) {
    return kept;
  }
  const hits = files.filter((f) => fileMatches(query, f.path));
  const rows = [];
  let last = null;
  for (const f of hits) {
    const at = f.path.lastIndexOf("/");
    const dir = at < 0 ? "./" : f.path.slice(0, at + 1);
    if (dir !== last) {
      rows.push({ kind: "dir", key: `d:${rows.length}`, text: cleanText(dir) });
      last = dir;
    }
    rows.push(fileRow(f));
  }
  const count = { shown: hits.length, total: files.length };
  const made = { query, hits, rows, count, info: { count: `${count.shown}/${count.total}`, none: count.total > 0 && count.shown === 0 } };
  sidebarMemo.set(files, made);
  return made;
}
function sidebarButtons(m) {
  const buttons = [{ id: "sidebar-toggle", label: m.panel.open ? "Hide files" : "Show files", icon: m.panel.side === "left" ? "panel-left" : "panel-right", on: m.panel.open, msg: { _tag: "SidebarToggled" } }];
  if (m.panel.open) {
    buttons.push({ id: "sidebar-flip", label: m.panel.side === "left" ? "Move file list to the right side" : "Move file list to the left side", icon: "swap", on: false, msg: { _tag: "SidebarFlipped" } });
  }
  return buttons;
}
function layoutButton(m) {
  return { id: "layout-toggle", label: isSplit(m) ? "Unified" : "Side by side", icon: "columns", on: isSplit(m), msg: { _tag: "LayoutToggled" } };
}
function paneOrder(m) {
  if (!m.panel.open) {
    return ["diff"];
  }
  return m.panel.side === "left" ? ["sidebar", "diff"] : ["diff", "sidebar"];
}
function filterInfo(m) {
  return sidebarOf(m.state === null ? NO_FILES : m.state.files, m.filter).info;
}
var NO_TOTALS = { files: 0, add: 0, del: 0 };
var totalsMemo = /* @__PURE__ */ new WeakMap();
function totals(state) {
  if (state === null) {
    return NO_TOTALS;
  }
  let sum = totalsMemo.get(state);
  if (sum === void 0) {
    sum = { files: state.files.length, add: state.files.reduce((n, f) => n + f.add, 0), del: state.files.reduce((n, f) => n + f.del, 0) };
    totalsMemo.set(state, sum);
  }
  return sum;
}

// src/message.ts
var BUSY_EDITOR = "finish or cancel the comment first";
var FETCH = 'bash "${HERDR_APP_HOME:-$HOME/.config/herdr-app}/mods/herdr-review/fetch-binary.sh"';
var NO_AGENT = "no agent to read: send once to choose one, or open the panel from an agent's terminal";
var initMessage = () => ({ data: null, loading: false, sending: false, comments: [], next: 1, selection: null, drag: null, hover: null, status: null });
var inMessage = (m, line2) => m.message.data !== null && Number.isInteger(line2) && line2 >= 1 && line2 <= m.message.data.lines.length;
var set = (m, change) => ({ ...m, message: { ...m.message, ...change } });
var told = (m, text) => m.view === "message" ? set(m, { status: text }) : { ...m, status: text };
var rangeLabel = (from, to) => from === to ? `line ${from}` : `lines ${from}-${to}`;
var range = (a, b) => ({ from: Math.min(a, b), to: Math.max(a, b) });
var viewButton = (m) => m.view === "diff" ? { id: "view-message", label: "Agent message", msg: { _tag: "ViewChosen", view: "message" } } : { id: "view-diff", label: "Diff", msg: { _tag: "ViewChosen", view: "diff" } };
function sendButton(m) {
  const n = m.message.comments.length;
  return { label: n === 0 ? "Send" : `Send ${n}`, off: n === 0 || m.message.sending || m.message.loading };
}
function read(m) {
  if (m.target === null) {
    return [set(m, { loading: false, status: NO_AGENT }), []];
  }
  return [set(m, { loading: true, status: null }), [{ _tag: "ReadMessage", target: m.target }]];
}
function viewChosen(m, view) {
  if (view === m.view) {
    return [m, []];
  }
  if (m.editor !== null) {
    return [told(m, BUSY_EDITOR), []];
  }
  if (view === "diff") {
    return [{ ...set(m, { drag: null, hover: null }), view: "diff" }, []];
  }
  return read({ ...m, view: "message" });
}
function refreshed(m) {
  if (m.view !== "message" || m.message.loading) {
    return [m, []];
  }
  return m.editor !== null ? [told(m, BUSY_EDITOR), []] : read(m);
}
function messageLoaded(m, reply) {
  const old = m.message;
  const same = old.data !== null && old.data.id === reply.id;
  const dropped = !same && old.comments.length > 0;
  const status = dropped ? `the agent wrote a new message, ${plural(old.comments.length, "comment", "comments")} dropped` : null;
  if (same) {
    return [set(m, { data: reply, loading: false, status }), []];
  }
  const next = set(m, { data: reply, loading: false, status, comments: [], selection: null, drag: null, hover: null });
  return [{ ...next, editor: m.view === "message" ? null : m.editor }, []];
}
function hovered(m, line2, on) {
  const mv = m.message;
  if (!on) {
    return mv.hover === line2 ? [set(m, { hover: null }), []] : [m, []];
  }
  if (!inMessage(m, line2)) {
    return [m, []];
  }
  const d = mv.drag;
  if (d === null) {
    return mv.hover === line2 ? [m, []] : [set(m, { hover: line2 }), []];
  }
  if (d.head === line2 && mv.hover === line2) {
    return [m, []];
  }
  return [set(m, { hover: line2, drag: { ...d, head: line2, moved: d.moved || line2 !== d.line }, selection: line2 === d.line && !d.moved ? mv.selection : range(d.line, line2) }), []];
}
function pressed(m, line2) {
  if (m.editor !== null || !inMessage(m, line2)) {
    return [m, []];
  }
  return [set(m, { hover: line2, drag: { line: line2, head: line2, moved: false } }), []];
}
function released(m) {
  const d = m.message.drag;
  if (d === null) {
    return [m, []];
  }
  return [set(m, { drag: null, selection: d.moved ? m.message.selection : null }), []];
}
var SENDING = "wait for the send to finish";
function plusClicked(m, line2) {
  if (m.editor !== null) {
    return [told(m, BUSY_EDITOR), []];
  }
  if (m.message.sending) {
    return [told(m, SENDING), []];
  }
  if (!inMessage(m, line2)) {
    return [m, []];
  }
  const s = m.message.selection;
  const sel = s !== null && line2 >= s.from && line2 <= s.to ? s : { from: line2, to: line2 };
  const editor = { kind: "comment", anchor: null, thread: null, comment: null, initial: "", title: `Comment on ${rangeLabel(sel.from, sel.to)}` };
  return [{ ...set(m, { selection: sel, drag: null, status: null }), editor }, []];
}
function messageSaved(m, text) {
  const e = m.editor;
  const body2 = text.trim();
  if (e === null || body2 === "") {
    return [m, []];
  }
  const mv = m.message;
  if (e.kind === "edit") {
    const id = Number(e.comment);
    return [{ ...set(m, { comments: mv.comments.map((c) => c.id === id ? { ...c, body: body2 } : c), status: null }), editor: null }, []];
  }
  if (mv.selection === null) {
    return [m, []];
  }
  const comment2 = { id: mv.next, start: mv.selection.from, end: mv.selection.to, body: body2 };
  return [{ ...set(m, { comments: [...mv.comments, comment2], next: mv.next + 1, selection: null, status: null }), editor: null }, []];
}
function commentAction(m, id, action) {
  if (m.editor !== null) {
    return [told(m, BUSY_EDITOR), []];
  }
  if (m.message.sending) {
    return [told(m, SENDING), []];
  }
  const c = m.message.comments.find((x) => x.id === id);
  if (c === void 0) {
    return [m, []];
  }
  if (action === "delete") {
    return [set(m, { comments: m.message.comments.filter((x) => x.id !== id) }), []];
  }
  return [{ ...m, editor: { kind: "edit", anchor: null, thread: null, comment: String(id), initial: c.body, title: `Edit comment on ${rangeLabel(c.start, c.end)}` } }, []];
}
function sendClicked(m) {
  const mv = m.message;
  if (mv.sending || mv.loading || m.editor !== null || mv.data === null) {
    return [m, []];
  }
  if (mv.comments.length === 0) {
    return [set(m, { status: "nothing to send" }), []];
  }
  if (m.target === null) {
    return [set(m, { status: NO_AGENT }), []];
  }
  const comments = mv.comments.map((c) => ({ start: c.start, end: c.end, body: c.body }));
  return [set(m, { sending: true, status: "sending..." }), [{ _tag: "SendMessage", target: m.target, id: mv.data.id, comments }]];
}
function messageSent(m, sent2) {
  const to = m.message.data === null ? "the agent" : m.message.data.agent;
  return [set(m, { sending: false, comments: [], selection: null, drag: null, status: `sent ${plural(sent2, "comment", "comments")} to ${to}` }), []];
}
function messageFailed(m, error) {
  return [set(m, { sending: false, loading: false, status: errorText(error) }), []];
}
var firstLine = (text) => text.trim().split("\n")[0] ?? "";
function errorText(error) {
  switch (error._tag) {
    case "BinaryMissing":
      return `herdr-review not found. Run: ${FETCH}`;
    case "TooOld":
      return `update herdr-review. Run: ${FETCH}`;
    case "ContractMismatch":
      return `unexpected reply at ${error.path}: ${error.detail}`;
    case "Busy":
      return "review is busy, try again";
    case "BackendFailed":
      return firstLine(error.stderr) || `herdr-review failed (${error.code})`;
    case "BadRequest":
      return firstLine(error.stderr) || "herdr-review refused the request";
    case "Refused":
      return `nothing was sent: ${error.why}`;
    case "NoTarget":
      return NO_AGENT;
    case "AmbiguousTarget":
      return "several agents match, send once to choose one";
    case "WatchUnavailable":
      return "could not watch the folder";
    default: {
      const unhandled = error;
      throw new Error(`unhandled error ${JSON.stringify(unhandled)}`);
    }
  }
}
function messageItems(mv) {
  if (mv.data === null) {
    return [];
  }
  const out = [];
  const sel = mv.selection;
  mv.data.lines.forEach((text, i) => {
    const n = i + 1;
    out.push({ kind: "line", n, text, selected: sel !== null && n >= sel.from && n <= sel.to, hovered: mv.hover === n });
    for (const c of mv.comments.filter((x) => x.end === n)) {
      out.push({ kind: "card", id: c.id, label: rangeLabel(c.start, c.end), body: c.body.split("\n") });
    }
  });
  return out;
}

// src/runner.ts
function isTagged(e) {
  return typeof e === "object" && e !== null && typeof e._tag === "string";
}
async function withRecovery(op, handler) {
  try {
    return await handler();
  } catch (e) {
    if (isTagged(e)) {
      return { _tag: "Failed", op, error: e };
    }
    throw e;
  }
}
function run2(env, cmd) {
  switch (cmd._tag) {
    case "Load":
      return withRecovery("load", async () => ({ _tag: "Loaded", key: cmd.key, state: await env.cli.state(cmd.root, cmd.spec) }));
    case "LoadFiles":
      return withRecovery("files", async () => ({ _tag: "FilesLoaded", key: cmd.key, files: await env.cli.files(cmd.root, cmd.spec, cmd.paths) }));
    case "WatchStore":
      return watch(env, cmd.dir, "watch-store");
    case "Notify":
      env.host.notify({ title: cmd.title, body: cmd.body, level: "warning" });
      return Promise.resolve(null);
    case "Write":
      return withRecovery(cmd.op.kind === "seen" ? "seen" : "write", async () => {
        await write(env, cmd.root, cmd.spec, cmd.op);
        return { _tag: "Wrote", kind: cmd.op.kind };
      });
    case "Send":
      return withRecovery("send", async () => {
        const r = await env.cli.send(cmd.root, cmd.target, cmd.threads);
        return { _tag: "Sent", sent: r.sent, agent: r.agent, queued: r.queued, warnings: r.warnings };
      });
    case "Archive":
      return withRecovery("archive", async () => {
        const r = await env.cli.archive(cmd.root);
        return { _tag: "Archived", archived: r.archived, unsent: r.unsent };
      });
    case "SetViewed":
      return withRecovery("viewed", async () => {
        await env.cli.view(cmd.root, cmd.spec, cmd.path, cmd.viewed);
        return { _tag: "Wrote", kind: "viewed" };
      });
    case "ReadMessage":
      return withRecovery("message", async () => ({ _tag: "MessageLoaded", reply: await env.cli.message(cmd.target) }));
    case "SendMessage":
      return withRecovery("message-send", async () => {
        let sent2 = 0;
        await withBodyFile(env, JSON.stringify({ id: cmd.id, comments: cmd.comments }), async (file) => {
          sent2 = (await env.cli.messageSend(cmd.target, file)).sent;
        });
        return { _tag: "MessageSent", sent: sent2 };
      });
    case "ReadPanel":
      return withRecovery("panel", async () => {
        const text = await env.host.readTextFile(panelFile(env.host)).catch(() => null);
        return { _tag: "PanelLoaded", panel: decodePanel(text) };
      });
    case "ReadTheme":
      return env.cli.theme().then(
        (r) => ({ _tag: "ThemeLoaded", colors: r.colors, warnings: r.warnings }),
        (e) => {
          if (!isTagged(e)) {
            throw e;
          }
          return { _tag: "ThemeLoaded", colors: null, warnings: [`could not read the plugin theme: ${errorText(e)}`] };
        }
      );
    case "SavePanel":
      return withRecovery("panel", async () => {
        await savePanel(env.host, cmd.panel);
        return null;
      });
    default: {
      const unhandled = cmd;
      throw new Error(`unhandled Cmd ${JSON.stringify(unhandled)}`);
    }
  }
}
var panelFile = (host) => `${host.appHome()}/data/herdr-review/panel.json`;
async function savePanel(host, panel) {
  const file = panelFile(host);
  for (let i = 0; i < 3; i++) {
    try {
      await host.writeTextFile(file, `${JSON.stringify(panel)}
`, await host.readTextFile(file));
      return;
    } catch {
    }
  }
  throw { _tag: "BackendFailed", code: -1, stderr: "could not write the panel settings" };
}
async function shell(host, command, args) {
  const fail2 = (stderr) => {
    throw { _tag: "BackendFailed", code: -1, stderr };
  };
  let result;
  try {
    result = await host.exec(command, args, { timeout: 3e4 });
  } catch {
    return fail2(`could not run ${command}`);
  }
  if (result.code !== 0) {
    fail2(result.stderr.trim() || `${command} failed (${result.code})`);
  }
}
async function bodyFile(env, text) {
  const dir = `${env.host.appHome()}/data/herdr-review`;
  await shell(env.host, "mkdir", ["-p", dir]);
  await shell(env.host, "chmod", ["700", dir]);
  const file = `${dir}/${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}.txt`;
  try {
    await env.host.writeTextFile(file, text, null);
  } catch {
    throw { _tag: "BackendFailed", code: -1, stderr: "could not write the draft file" };
  }
  return file;
}
async function withBodyFile(env, text, send) {
  const file = await bodyFile(env, text);
  try {
    await send(file);
  } catch (e) {
    if (typeof e === "object" && e !== null && ["BinaryMissing", "TooOld"].includes(e._tag ?? "")) {
      await env.host.writeTextFile(file, "", text).catch(() => null);
    }
    throw e;
  }
}
async function write(env, root, spec2, op) {
  const cli = env.cli;
  switch (op.kind) {
    case "comment":
      return withBodyFile(env, batchOf(op.anchor, op.text), (file) => cli.apply(root, spec2, file));
    case "reply":
      return withBodyFile(env, op.text, (file) => cli.reply(root, op.id, file));
    case "edit":
      return withBodyFile(env, op.text, (file) => cli.edit(root, op.id, file));
    case "resolve":
      return cli.resolve(root, op.id);
    case "reopen":
      return cli.reopen(root, op.id);
    case "delete":
      return cli.remove(root, op.id);
    case "seen":
      return cli.seen(root, op.id);
    default: {
      const unhandled = op;
      throw new Error(`unhandled write ${JSON.stringify(unhandled)}`);
    }
  }
}
async function watch(env, dir, op) {
  while (!env.closed()) {
    try {
      await env.host.watchFolder(dir);
    } catch {
      if (!env.closed()) {
        env.dispatch({ _tag: "Failed", op, error: { _tag: "WatchUnavailable" } });
      }
      return null;
    }
    if (env.closed()) {
      break;
    }
    env.dispatch({ _tag: "Triggered", source: "watch" });
  }
  return null;
}
var commandMsg = (command) => command === "next-comment" ? { _tag: "ThreadJumped", forward: true } : command === "previous-comment" ? { _tag: "ThreadJumped", forward: false } : null;
async function listen(env, onFocus) {
  while (!env.closed()) {
    try {
      const command = await env.host.nextCommand();
      if (env.closed()) {
        break;
      }
      if (command === "focus") {
        onFocus();
      }
      const msg = commandMsg(command);
      if (msg !== null) {
        env.dispatch(msg);
      }
    } catch {
      return null;
    }
  }
  return null;
}
function open(host) {
  const props2 = decodeProps(host.props());
  if (props2.root !== null) {
    return { _tag: "Opened", root: props2.root, target: props2.target };
  }
  const focus = decodeFocus(host.focused());
  if (focus !== null && focus.cwd !== null) {
    return { _tag: "Opened", root: focus.cwd, target: props2.target ?? (focus.agent ? focus.terminal : null) };
  }
  const tab = focus === null ? null : focus.tab;
  const panes2 = decodePanes(host.panes()).filter((p) => p.tab === tab);
  const pane = panes2.find((p) => p.agent) ?? panes2[0];
  if (pane === void 0) {
    return { _tag: "Opened", root: null, target: null };
  }
  return { _tag: "Opened", root: pane.cwd, target: props2.target ?? (pane.agent ? pane.terminal : null) };
}

// src/perf.ts
var WINDOW_MS = 5e3;
var LOG_LINES = 200;
var NOOP = { on: false, start: () => 0, render: () => {
}, dispatch: () => {
}, rebuild: () => {
}, flush: () => {
} };
var noop = () => NOOP;
function percentile(values, p) {
  if (values.length === 0) {
    return 0;
  }
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))];
}
var ms = (n) => n.toFixed(2);
var stat = (name, v) => `${name} n=${v.length} mean=${ms(v.length === 0 ? 0 : v.reduce((a, b) => a + b, 0) / v.length)}ms p95=${ms(percentile(v, 0.95))}ms max=${ms(Math.max(0, ...v))}ms`;
function summary(seconds, render, dispatch, rebuilds, drawn) {
  const mean = drawn.length === 0 ? 0 : drawn.reduce((a, b) => a + b, 0) / drawn.length;
  return `perf window=${seconds.toFixed(1)}s ${stat("render", render)} | ${stat("dispatch", dispatch)} | layout_rebuilds=${rebuilds} | drawn mean=${mean.toFixed(1)} max=${Math.max(0, ...drawn)}`;
}
function makeRecorder(now, emit, windowMs = WINDOW_MS) {
  let began = now();
  let renders = [];
  let dispatches = [];
  let drawn = [];
  let rebuilds = 0;
  const flush2 = () => {
    if (renders.length + dispatches.length + rebuilds > 0) {
      emit(summary((now() - began) / 1e3, renders, dispatches, rebuilds, drawn));
    }
    began = now();
    renders = [];
    dispatches = [];
    drawn = [];
    rebuilds = 0;
  };
  const tick = () => {
    if (now() - began >= windowMs) {
      flush2();
    }
  };
  return {
    on: true,
    start: now,
    render(from, count) {
      renders.push(now() - from);
      drawn.push(count);
      tick();
    },
    dispatch(from) {
      dispatches.push(now() - from);
      tick();
    },
    rebuild() {
      rebuilds++;
    },
    flush: flush2
  };
}
var appendCapped = (old, line2, max = LOG_LINES) => `${[...(old ?? "").split("\n").filter((l) => l !== ""), line2].slice(-max).join("\n")}
`;
function fileSink(host, stamp = () => (/* @__PURE__ */ new Date()).toISOString()) {
  const file = `${host.appHome()}/data/herdr-review/perf.log`;
  let tail = Promise.resolve();
  return (line2) => {
    tail = tail.then(async () => {
      for (let i = 0; i < 3; i++) {
        try {
          const old = await host.readTextFile(file);
          await host.writeTextFile(file, appendCapped(old, `${stamp()} ${line2}`), old);
          return;
        } catch {
        }
      }
    });
  };
}
var clock = () => typeof performance !== "undefined" ? performance.now() : Date.now();
function recorderFor(host, now = clock) {
  const props2 = host.props();
  if (typeof props2 !== "object" || props2 === null || props2.perf !== true) {
    return NOOP;
  }
  return makeRecorder(now, fileSink(host));
}

// src/update.ts
var specText = (spec2) => spec2 === null ? "meta" : spec2.kind === "worktree" ? "worktree" : `branch:${spec2.base}`;
var keyOf = (root, spec2) => JSON.stringify([root, specText(spec2)]);
var themed = (base, m) => ({ ...base, ...m.colors });
var DEFAULT_PANEL = { side: "left", open: true };
function initModel() {
  return {
    root: null,
    target: null,
    spec: null,
    key: keyOf(null, null),
    state: null,
    files: /* @__PURE__ */ new Map(),
    selected: null,
    top: null,
    visible: [],
    reveal: null,
    width: 600,
    height: 600,
    offset: 0,
    total: 0,
    split: null,
    shift: 0,
    hover: null,
    drag: null,
    grip: null,
    fresh: [],
    glance: [],
    seenSent: /* @__PURE__ */ new Set(),
    folds: /* @__PURE__ */ new Map(),
    dirty: /* @__PURE__ */ new Set(),
    skipped: /* @__PURE__ */ new Set(),
    batch: [],
    expanded: /* @__PURE__ */ new Set(),
    selection: null,
    cursor: null,
    visual: false,
    focus: null,
    confirm: null,
    jump: null,
    editor: null,
    saving: false,
    draft: null,
    landing: null,
    picker: null,
    sending: false,
    view: "diff",
    message: initMessage(),
    sendThreads: [],
    confirmArchive: false,
    sticky: false,
    banner: null,
    status: null,
    inFlight: false,
    pending: false,
    stopped: false,
    storeWatch: null,
    retried: false,
    panel: DEFAULT_PANEL,
    filter: "",
    filtering: false,
    themeAsked: false,
    colors: null
  };
}
var BUSY_EDITOR2 = "finish or cancel the comment first";
function rekey(m, change) {
  const next = { ...m, ...change };
  return { ...next, key: keyOf(next.root, next.spec) };
}
function startLoad(m) {
  if (m.root === null) {
    return [m, []];
  }
  return [{ ...m, inFlight: true, pending: false, skipped: /* @__PURE__ */ new Set() }, [{ _tag: "Load", key: m.key, root: m.root, spec: m.spec }]];
}
var holding = (m) => m.editor !== null && m.landing === null;
function nextFiles(m) {
  if (m.root === null || m.inFlight || holding(m) || m.stopped) {
    return [m, []];
  }
  const paths = wanted(m).slice(0, BATCH);
  if (paths.length === 0) {
    return [m, []];
  }
  return [{ ...m, inFlight: true, batch: paths }, [{ _tag: "LoadFiles", key: m.key, root: m.root, spec: m.spec, paths }]];
}
function flush(m, cmds = []) {
  if (m.inFlight || holding(m) || m.stopped) {
    return [m, cmds];
  }
  const [next, more] = m.pending ? startLoad(m) : nextFiles(m);
  return [next, [...cmds, ...more]];
}
function trigger(m) {
  if (m.root === null) {
    return [m, []];
  }
  if (m.inFlight || holding(m)) {
    return [{ ...m, pending: true }, []];
  }
  return startLoad(m);
}
var rowOf = (m, path, row2) => {
  const file = m.files.get(path);
  return file !== void 0 && file.rows[row2] !== void 0 ? file : null;
};
function flipped(m, next) {
  if (isSplit(m) === isSplit(next)) {
    return next;
  }
  const c = next.cursor;
  const row2 = c === null ? void 0 : next.files.get(c.path)?.rows[c.row];
  const side = !isSplit(next) || row2 === void 0 ? null : next.visual && next.selection !== null ? next.selection.side : sideOf(row2);
  const cursor = c === null || row2 === void 0 ? null : { ...c, side };
  return { ...next, cursor, hover: null, shift: 0, reveal: next.reveal ?? (m.top === null ? null : { key: m.top }) };
}
function hovered2(m, path, row2, side, on) {
  const same = m.hover !== null && m.hover.path === path && m.hover.row === row2 && m.hover.side === side;
  if (!on) {
    return same ? [{ ...m, hover: null }, []] : [m, []];
  }
  const file = rowOf(m, path, row2);
  if (file === null) {
    return [m, []];
  }
  const next = same ? m : { ...m, hover: { path, row: row2, side } };
  const d = next.drag;
  const head2 = d !== null && d.side !== null && side !== null && side !== d.side ? partner(file, row2) ?? row2 : row2;
  if (d === null || d.path !== path || d.head === head2 || rowOf(m, path, d.row) === null) {
    return [next, []];
  }
  return [{ ...next, drag: { ...d, head: head2, moved: true }, selection: span(file, d.row, head2, d.side), focus: null, confirm: null }, []];
}
function pressed2(m, path, row2, side) {
  const file = m.editor !== null ? null : rowOf(m, path, row2);
  if (file === null) {
    return [m, []];
  }
  const cursor = file.rows[row2]?.kind === "hunk" ? m.cursor : { path, row: row2, side };
  return [{ ...m, cursor, visual: false, hover: { path, row: row2, side }, drag: { path, row: row2, head: row2, moved: false, side } }, []];
}
function released2(m) {
  if (m.drag === null && m.grip === null) {
    return [m, []];
  }
  return [{ ...m, drag: null, grip: null, selection: m.drag !== null && !m.drag.moved ? null : m.selection }, []];
}
function scrollTo(m, offset) {
  const next = clampOffset(offset, m.total, m.height);
  return next === m.offset ? [m, []] : [{ ...m, offset: next }, []];
}
function thumbMoved(m, y) {
  const thumb = thumbOf(m.total, m.height, 0);
  if (m.grip === null || thumb === null) {
    return [m, []];
  }
  return scrollTo(m, m.grip.offset + (y - m.grip.y) * maxOffset(m.total, m.height) / Math.max(1, m.height - thumb.size));
}
function trackClicked(m, y) {
  const thumb = thumbOf(m.total, m.height, m.offset);
  return thumb === null ? [m, []] : scrollTo(m, offsetOfThumb(m.total, m.height, y - thumb.size / 2));
}
function plusClicked2(m, path, row2, side) {
  const file = m.files.get(path);
  if (file === void 0) {
    return [m, []];
  }
  if (m.editor !== null) {
    return [{ ...m, status: BUSY_EDITOR2 }, []];
  }
  const sel = plusTarget(file, m.selection, row2, side);
  if (sel === null) {
    return [m, []];
  }
  return [{ ...m, selection: sel, visual: false, focus: null, confirm: null, status: null, reveal: showEditor(path), editor: { kind: "comment", anchor: anchorOf(file, sel), thread: null, comment: null, initial: "" } }, []];
}
var showEditor = (path) => ({ key: itemKey(path, "editor"), near: true });
var draftLanded = (m) => ({ ...m, editor: null, selection: null, saving: false, status: null, draft: null, landing: null });
function draftFile(m) {
  const e = m.editor;
  if (e === null) {
    return "";
  }
  if (e.anchor !== null) {
    return e.anchor.path;
  }
  const key = cardKey(m, e.thread ?? "");
  return key === null ? "" : keyPath(key);
}
function cursorTo(m, s) {
  const sel = m.selection;
  const file = m.files.get(s.path);
  return {
    ...m,
    cursor: { path: s.path, row: s.row, side: s.side },
    selection: visualOn(m) && sel !== null && file !== void 0 ? span(file, sel.anchor, s.row, sel.side) : sel,
    reveal: { key: itemKey(s.path, `r${s.row}`), near: true },
    focus: null
  };
}
function cursorMoved(m, by) {
  const all = spots(m);
  const walk = lane(m, all);
  const c = m.cursor;
  const i = c === null ? -1 : walk.findIndex((s) => sameSpot(s, c));
  if (c === null || i < 0) {
    const top = topSpot(m, all);
    return top === null ? [m, []] : [cursorTo(m, top), []];
  }
  const to = walk[Math.max(0, Math.min(walk.length - 1, i + by))];
  return to === void 0 || sameSpot(to, c) ? [m, []] : [cursorTo(m, to), []];
}
function cursorHunk(m, forward) {
  const all = spots(m);
  const walk = lane(m, all);
  const c = m.cursor;
  const i = c === null ? -1 : walk.findIndex((s) => sameSpot(s, c));
  if (i < 0) {
    return cursorMoved(m, 0);
  }
  const to = forward ? walk.slice(i + 1).find((s) => s.first) : walk.slice(0, i).reverse().find((s) => s.first);
  return to === void 0 ? [m, []] : [cursorTo(m, to), []];
}
function cursorSide(m, side) {
  const at = cursorSpot(m);
  const file = at === null ? void 0 : m.files.get(at.path);
  if (at === null || file === void 0 || !isSplit(m) || at.side === side || visualOn(m)) {
    return [m, []];
  }
  const row2 = partner(file, at.row);
  return row2 === null ? [m, []] : [cursorTo(m, { path: at.path, row: row2, side, first: false }), []];
}
function visualToggled(m) {
  if (m.selection !== null) {
    return [{ ...m, selection: null, visual: false }, []];
  }
  const at = cursorSpot(m);
  const file = at === null ? void 0 : m.files.get(at.path);
  if (at === null || file === void 0) {
    return cursorMoved(m, 0);
  }
  const sel = pick(file, at.row, at.side);
  return sel === null ? [m, []] : [{ ...m, selection: sel, visual: true, focus: null, confirm: null }, []];
}
function write2(m, op, change = {}) {
  if (m.root === null) {
    return [m, []];
  }
  return [{ ...m, ...change }, [{ _tag: "Write", root: m.root, spec: m.spec, op }]];
}
function saved(m, text) {
  if (m.view === "message") {
    return messageSaved(m, text);
  }
  const e = m.editor;
  if (e === null || m.saving || text.trim() === "") {
    return [m, []];
  }
  const saving = { saving: true, status: null, draft: text };
  if (e.kind === "comment") {
    return e.anchor === null ? [m, []] : write2(m, { kind: "comment", anchor: e.anchor, text }, saving);
  }
  if (e.kind === "reply") {
    return e.thread === null ? [m, []] : write2(m, { kind: "reply", id: e.thread, text }, saving);
  }
  return e.comment === null ? [m, []] : write2(m, { kind: "edit", id: e.comment, text }, saving);
}
function threadAction(m, id, action) {
  const hit = findComment(m.files, m.state, id);
  if (hit === null) {
    return [m, []];
  }
  const { thread: thread2, comment: comment2 } = hit;
  const mine = comment2.author === "user";
  if (action === "focus") {
    return [{ ...m, focus: id, confirm: null, selection: null }, []];
  }
  if (m.editor !== null) {
    return [{ ...m, status: BUSY_EDITOR2 }, []];
  }
  const base = { ...m, focus: id, confirm: null, status: null };
  const there = showEditor(keyPath(cardKey(m, thread2.id) ?? ""));
  switch (action) {
    case "toggle": {
      if (thread2.status !== "resolved") {
        return [{ ...base, focus: thread2.id }, []];
      }
      const open2 = m.expanded.has(thread2.id);
      const expanded = new Set(m.expanded);
      if (open2) {
        expanded.delete(thread2.id);
      } else {
        expanded.add(thread2.id);
      }
      const next = { ...base, focus: thread2.id, expanded };
      return !open2 && thread2.is_new ? write2({ ...next, seenSent: new Set(m.seenSent).add(thread2.id) }, { kind: "seen", id: thread2.id }) : [next, []];
    }
    case "reply":
      return [{ ...base, reveal: there, editor: { kind: "reply", anchor: null, thread: thread2.id, comment: null, initial: "" } }, []];
    case "edit":
      return mine ? [{ ...base, reveal: there, editor: { kind: "edit", anchor: null, thread: thread2.id, comment: id, initial: comment2.body } }, []] : [base, []];
    case "resolve":
      return thread2.status === "open" ? write2(base, { kind: "resolve", id: thread2.id }) : [base, []];
    case "reopen":
      return thread2.status === "resolved" ? write2(base, { kind: "reopen", id: thread2.id }) : [base, []];
    case "delete":
      if (!mine) {
        return [base, []];
      }
      return [{ ...m, confirm: id, focus: id }, []];
    default: {
      const unhandled = action;
      throw new Error(`unhandled thread action ${String(unhandled)}`);
    }
  }
}
function jumped(m, forward) {
  if (m.editor !== null) {
    return [{ ...m, status: BUSY_EDITOR2 }, []];
  }
  if (m.root === null || m.picker !== null) {
    return [m, []];
  }
  const stop = jumpStop(m, forward);
  if (stop === null) {
    return [{ ...m, status: "no comments in this review" }, []];
  }
  const base = { ...m, confirm: null, confirmArchive: false, status: null };
  if ("id" in stop) {
    const key = cardKey(m, stop.id);
    return [{ ...base, jump: null, focus: stop.id, reveal: key === null ? m.reveal : { key } }, []];
  }
  const jump = { path: stop.path, edge: forward ? "first" : "last" };
  const open2 = { ...base, selection: null, folds: new Map(m.folds).set(stop.path, false) };
  const view = m.files.get(stop.path);
  if (view !== void 0) {
    return flush(landed(open2, jump, view));
  }
  return flush({ ...open2, jump, reveal: { key: itemKey(stop.path, "file") } });
}
function landed(m, jump, view) {
  const ids = cardOrder(view);
  const id = jump.edge === "first" ? ids[0] : ids[ids.length - 1];
  if (id === void 0) {
    return { ...m, jump: null, status: `no comments in ${jump.path}` };
  }
  const key = cardKey({ ...m, files: new Map(m.files).set(jump.path, view) }, id);
  return { ...m, jump: null, focus: id, reveal: key === null ? m.reveal : { key } };
}
function sendClicked2(m, threads) {
  if (m.root === null || m.sending) {
    return [m, []];
  }
  if (m.editor !== null) {
    return [{ ...m, status: BUSY_EDITOR2, sticky: false }, []];
  }
  if (threads.length === 0 && unsentCount(m.state) === 0) {
    return [{ ...m, status: "nothing to send", sticky: false }, []];
  }
  return [{ ...m, sending: true, sendThreads: threads, picker: null, confirmArchive: false, status: "sending...", sticky: true }, [{ _tag: "Send", root: m.root, target: m.target, threads }]];
}
function sent(m, msg) {
  const to = msg.agent === null ? "the agent" : msg.agent;
  const text = msg.sent === 0 ? "nothing to send" : msg.queued ? `${to} is working, ${plural(msg.sent, "comment", "comments")} queued` : `sent ${msg.sent} to ${to}`;
  return trigger({ ...m, sending: false, status: msg.warnings.length > 0 ? `${text} (${msg.warnings.join("; ")})` : text, sticky: true });
}
function archiveClicked(m) {
  if (m.root === null || m.inFlight || m.editor !== null || archivableCount(m.state) === 0) {
    return [m, []];
  }
  return [{ ...m, confirmArchive: true }, []];
}
function archiveConfirmed(m) {
  if (m.root === null || m.inFlight || !m.confirmArchive) {
    return [m, []];
  }
  return [{ ...m, confirmArchive: false, status: "archiving...", sticky: true }, [{ _tag: "Archive", root: m.root }]];
}
function glanced(m) {
  const id = m.fresh.find((x) => m.glance.includes(x) && !m.seenSent.has(x));
  const same = m.glance.length === m.fresh.length && m.glance.every((x, i) => x === m.fresh[i]);
  if (m.root === null || m.stopped || id === void 0) {
    return same ? [m, []] : [{ ...m, glance: m.fresh }, []];
  }
  return write2({ ...m, glance: m.fresh, seenSent: new Set(m.seenSent).add(id) }, { kind: "seen", id });
}
function deleteConfirmed(m, id) {
  const hit = findComment(m.files, m.state, id);
  if (m.confirm !== id || hit === null || hit.comment.author !== "user" || m.editor !== null) {
    return [m, []];
  }
  return write2({ ...m, focus: id, confirm: null, status: null }, { kind: "delete", id });
}
function archived(m, msg) {
  const never = msg.unsent === 0 ? "" : `, ${msg.unsent} of them ${msg.unsent === 1 ? "was" : "were"} never sent`;
  const text = msg.archived === 0 ? "nothing to archive" : `archived ${plural(msg.archived, "thread", "threads")}${never}`;
  return trigger({ ...m, confirmArchive: false, status: text, sticky: true });
}
function savePanel2(m, panel) {
  return [{ ...m, panel }, [{ _tag: "SavePanel", panel }]];
}
function entered(m) {
  if (m.filter === "") {
    return [{ ...m, filtering: false }, []];
  }
  if (filterCount(m.state === null ? [] : m.state.files, m.filter).shown === 0) {
    return [m, []];
  }
  const left = { ...m, filtering: false };
  const first = filterPick(m);
  return first === null ? [left, []] : update(left, { _tag: "FileChosen", path: first });
}
var summaryJson = (state, path) => JSON.stringify(state?.files.find((f) => f.path === path) ?? null);
function settle(m) {
  const sel = m.selection;
  const file = sel === null ? void 0 : m.files.get(sel.path);
  return {
    ...m,
    selection: sel !== null && file === void 0 ? null : sel,
    drag: m.drag !== null && !m.files.has(m.drag.path) ? null : m.drag,
    focus: findComment(m.files, m.state, m.focus) === null ? null : m.focus,
    confirm: findComment(m.files, m.state, m.confirm) === null ? null : m.confirm
  };
}
function loadedState(m, key, state) {
  if (key !== m.key) {
    return flush({ ...m, inFlight: false });
  }
  const paths = new Set(state.files.map((f) => f.path));
  const files = new Map([...m.files].filter(([path]) => paths.has(path)));
  const dirty = new Set([...m.dirty, ...[...files.keys()].filter((p) => summaryJson(m.state, p) !== summaryJson(state, p))].filter((p) => files.has(p)));
  const folds = new Map([...m.folds].filter(([path, fold]) => paths.has(path) && state.files.find((f) => f.path === path)?.viewed !== fold));
  const shown = settle(
    rekey(
      {
        ...m,
        state,
        files,
        dirty,
        folds,
        selected: m.selected !== null && paths.has(m.selected) ? m.selected : state.files[0]?.path ?? null,
        banner: null,
        status: m.sticky ? m.status : null,
        inFlight: false,
        retried: false,
        // The question names a number, so it goes when the new state has nothing to archive.
        confirmArchive: m.confirmArchive && state.archivable > 0
      },
      { spec: state.spec }
    )
  );
  const done = shown.landing !== null && (shown.landing === "" || !shown.dirty.has(shown.landing)) ? draftLanded(shown) : shown;
  if (done.storeWatch === null && state.store_dir !== "") {
    return flush({ ...done, storeWatch: state.store_dir }, [{ _tag: "WatchStore", dir: state.store_dir }]);
  }
  return flush(done);
}
function loadedFiles(m, key, replies) {
  if (key !== m.key) {
    return flush({ ...m, inFlight: false, batch: [] });
  }
  const files = new Map(m.files);
  const dirty = new Set(m.dirty);
  let selection = m.selection;
  let cursor = m.cursor;
  for (const f of replies) {
    if (selection !== null && selection.path === f.path && !sameRows(m.files.get(f.path) ?? null, f, selection)) {
      selection = null;
    }
    if (cursor !== null && cursor.path === f.path && !sameRows(m.files.get(f.path) ?? null, f, { path: f.path, side: "new", from: cursor.row, to: cursor.row, anchor: cursor.row })) {
      cursor = null;
    }
    files.set(f.path, f);
    dirty.delete(f.path);
  }
  const order = (m.state?.files ?? []).map((f) => f.path);
  const first = m.visible[0] === void 0 ? -1 : order.indexOf(m.visible[0]);
  const above = first >= 0 && replies.some((f) => order.indexOf(f.path) >= 0 && order.indexOf(f.path) < first);
  let next = settle({ ...m, files, dirty, selection, cursor, inFlight: false, batch: [], reveal: above && m.top !== null && m.reveal === null ? { key: m.top } : m.reveal });
  if (next.landing !== null && replies.some((f) => f.path === next.landing)) {
    next = draftLanded(next);
  }
  const waiting = m.jump === null ? void 0 : replies.find((f) => f.path === m.jump.path);
  if (m.jump !== null && waiting !== void 0) {
    next = landed(next, m.jump, waiting);
  }
  return flush(next);
}
function update(m, msg) {
  switch (msg._tag) {
    case "Opened": {
      if (msg.root === null) {
        return [{ ...m, banner: "open the panel from a pane that has a folder" }, []];
      }
      return trigger(rekey({ ...m, target: msg.target }, { root: msg.root }));
    }
    case "Triggered": {
      if (msg.source === "refresh") {
        return trigger({ ...m, dirty: /* @__PURE__ */ new Set([...m.dirty, ...m.visible.filter((p) => m.files.has(p))]), stopped: false, retried: false, status: m.sticky ? null : m.status, sticky: false });
      }
      if (m.stopped && (msg.source === "watch" || msg.source === "timer")) {
        return [m, []];
      }
      return trigger(m);
    }
    case "Loaded": {
      const [next, cmds] = loadedState(m, msg.key, msg.state);
      return msg.key !== m.key || m.themeAsked ? [next, cmds] : [{ ...next, themeAsked: true }, [...cmds, { _tag: "ReadTheme" }]];
    }
    case "FilesLoaded":
      return loadedFiles(m, msg.key, msg.files);
    case "Scrolled": {
      const goal = msg.revealed && m.reveal !== null ? keyPath(m.reveal.key) : null;
      const selected = goal !== null && msg.visible.includes(goal) ? goal : msg.visible[0] ?? m.selected;
      return flush({ ...m, offset: msg.offset, total: msg.total, top: msg.top, visible: msg.visible, fresh: msg.fresh, reveal: msg.revealed ? null : m.reveal, selected });
    }
    case "Glanced":
      return glanced(m);
    case "Wheeled":
      return scrollTo(m, m.offset + msg.dy);
    case "ThumbGrabbed":
      return [{ ...m, grip: { y: msg.y, offset: m.offset } }, []];
    case "ThumbMoved":
      return thumbMoved(m, msg.y);
    case "TrackClicked":
      return trackClicked(m, msg.y);
    case "Resized":
      return msg.width === m.width && msg.height === m.height ? [m, []] : [flipped(m, { ...m, width: msg.width, height: msg.height }), []];
    case "Hovered":
      return hovered2(m, msg.path, msg.row, msg.side, msg.on);
    case "RowPressed":
      return pressed2(m, msg.path, msg.row, msg.side);
    case "LayoutToggled":
      return [flipped(m, { ...m, split: !isSplit(m) }), []];
    case "Shifted": {
      const next = m.shift + msg.dx;
      const shift = msg.dx < 0 ? Math.max(0, next) : Math.max(m.shift, Math.min(msg.max, next));
      return shift === m.shift ? [m, []] : [{ ...m, shift }, []];
    }
    case "Released":
      return released2(m);
    case "FoldToggled": {
      if (m.editor !== null) {
        return [{ ...m, status: BUSY_EDITOR2 }, []];
      }
      const f = m.state?.files.find((x) => x.path === msg.path);
      if (f === void 0) {
        return [m, []];
      }
      const folded = !isFolded(m, f);
      return flush({ ...m, folds: new Map(m.folds).set(f.path, folded), selection: folded && m.selection?.path === f.path ? null : m.selection });
    }
    case "Failed":
      return failed(m, msg);
    case "SpecChosen": {
      if (specText(m.spec) === specText(msg.spec)) {
        return [m, []];
      }
      if (m.editor !== null) {
        return [{ ...m, status: BUSY_EDITOR2 }, []];
      }
      const fresh = { ...m, files: /* @__PURE__ */ new Map(), dirty: /* @__PURE__ */ new Set(), skipped: /* @__PURE__ */ new Set(), selection: null, cursor: null, visual: false, drag: null, hover: null, grip: null, offset: 0, jump: null, reveal: null, visible: [], top: null, status: m.sticky ? null : m.status, sticky: false };
      return trigger(rekey(fresh, { spec: msg.spec }));
    }
    case "FileChosen": {
      if (m.editor !== null) {
        return [{ ...m, status: BUSY_EDITOR2 }, []];
      }
      if (m.state?.files.every((f) => f.path !== msg.path)) {
        return [m, []];
      }
      return flush({ ...m, selection: null, jump: null, selected: msg.path, reveal: { key: itemKey(msg.path, "file") }, status: m.sticky ? null : m.status, sticky: false });
    }
    case "EditorCancelled":
      return m.editor !== null ? flush({ ...m, editor: null, saving: false, status: null, draft: null, landing: null }) : [m, []];
    case "SelectionCleared":
      return m.selection === null ? [m, []] : [{ ...m, selection: null, visual: false }, []];
    case "CursorMoved":
      return cursorMoved(m, msg.by);
    case "CursorHunk":
      return cursorHunk(m, msg.forward);
    case "CursorSide":
      return cursorSide(m, msg.side);
    case "VisualToggled":
      return visualToggled(m);
    case "PlusClicked":
      return plusClicked2(m, msg.path, msg.row, msg.side);
    case "FileCommentClicked":
      if (m.editor !== null || m.selected === null) {
        return [m, []];
      }
      return [{ ...m, selection: null, status: null, reveal: showEditor(m.selected), editor: { kind: "comment", anchor: { path: m.selected, old_path: null, side: null, line: null, end_line: null }, thread: null, comment: null, initial: "" } }, []];
    case "EditorSaved":
      return saved(m, msg.text);
    case "ThreadAction":
      return threadAction(m, msg.id, msg.action);
    case "ThreadJumped":
      return jumped(m, msg.forward);
    case "Wrote": {
      const draft = msg.kind === "comment" || msg.kind === "reply" || msg.kind === "edit";
      const at = draft ? draftFile(m) : "";
      return trigger(draft ? { ...m, saving: true, landing: at, dirty: at !== "" && m.files.has(at) ? /* @__PURE__ */ new Set([...m.dirty, at]) : m.dirty } : m);
    }
    case "SendClicked":
      return sendClicked2(m, msg.threads ?? []);
    case "PickerChosen": {
      if (m.root === null || m.sending || m.picker === null || !m.picker.some((c) => c.address === msg.target)) {
        return [m, []];
      }
      return [{ ...m, picker: null, target: msg.target, sending: true, status: "sending...", sticky: true }, [{ _tag: "Send", root: m.root, target: msg.target, threads: m.sendThreads }]];
    }
    case "PickerCancelled":
      return [{ ...m, picker: null, status: null }, []];
    case "Sent":
      return sent(m, msg);
    case "ArchiveClicked":
      return archiveClicked(m);
    case "ArchiveConfirmed":
      return archiveConfirmed(m);
    case "DeleteConfirmed":
      return deleteConfirmed(m, msg.id);
    case "ConfirmCancelled":
      return [{ ...m, confirm: null, confirmArchive: false }, []];
    case "Archived":
      return archived(m, msg);
    case "ViewedClicked":
      if (m.root === null) {
        return [m, []];
      }
      if (m.editor !== null) {
        return [{ ...m, status: BUSY_EDITOR2 }, []];
      }
      return [{ ...m, folds: new Map(m.folds).set(msg.path, msg.viewed), selection: m.selection?.path === msg.path && msg.viewed ? null : m.selection, reveal: msg.viewed ? { key: itemKey(msg.path, "file") } : m.reveal }, [{ _tag: "SetViewed", root: m.root, spec: m.spec, path: msg.path, viewed: msg.viewed }]];
    case "SidebarToggled":
      return savePanel2({ ...m, filtering: m.panel.open ? false : m.filtering }, { ...m.panel, open: !m.panel.open });
    case "SidebarFlipped":
      return savePanel2(m, { ...m.panel, side: m.panel.side === "left" ? "right" : "left" });
    case "FilterChanged":
      return [{ ...m, filter: msg.text }, []];
    case "FilterFocused":
      return [{ ...m, filtering: msg.on }, []];
    case "FilterOpened":
      if (m.editor !== null || m.picker !== null || m.confirm !== null || m.confirmArchive) {
        return [m, []];
      }
      return m.panel.open ? [m, []] : savePanel2(m, { ...m.panel, open: true });
    case "FilterCleared":
      return [{ ...m, filter: "", filtering: false }, []];
    case "FilterEntered":
      return entered(m);
    case "PanelLoaded":
      return [{ ...m, panel: msg.panel }, []];
    case "ThemeLoaded": {
      const colored = { ...m, colors: msg.colors };
      return [msg.warnings.length === 0 || m.sticky ? colored : { ...colored, status: msg.warnings.join("; "), sticky: true }, []];
    }
    case "ViewChosen":
      return viewChosen(m, msg.view);
    case "MessageLoaded":
      return messageLoaded(m, msg.reply);
    case "MessageRefreshed":
      return refreshed(m);
    case "MessageHovered":
      return hovered(m, msg.line, msg.on);
    case "MessagePressed":
      return pressed(m, msg.line);
    case "MessageReleased":
      return released(m);
    case "MessagePlusClicked":
      return plusClicked(m, msg.line);
    case "MessageCommentAction":
      return commentAction(m, msg.id, msg.action);
    case "MessageSendClicked":
      return sendClicked(m);
    case "MessageSent":
      return messageSent(m, msg.sent);
    default: {
      const unhandled = msg;
      throw new Error(`unhandled Msg ${JSON.stringify(unhandled)}`);
    }
  }
}
function failed(m, msg) {
  const { op, error } = msg;
  if (op === "watch-store") {
    return [{ ...m, storeWatch: null }, []];
  }
  if (op === "watch-root") {
    return [m, []];
  }
  if (op === "seen") {
    return [m, []];
  }
  if (op === "panel") {
    return [{ ...m, status: "could not remember the sidebar setting", sticky: false }, []];
  }
  if (op === "message" || op === "message-send") {
    return messageFailed(m, error);
  }
  const fetch = op === "load" || op === "files";
  const base = {
    ...m,
    inFlight: fetch ? false : m.inFlight,
    // The files of a fetch that failed are not asked for again until the next load, or the stream would ask for ever.
    skipped: op === "files" ? /* @__PURE__ */ new Set([...m.skipped, ...m.batch]) : m.skipped,
    batch: op === "files" ? [] : m.batch,
    jump: fetch ? null : m.jump,
    saving: op === "write" ? false : m.saving,
    // The write failed: the editor, still open behind the draft's card, is drawn again with its text.
    draft: op === "write" ? null : m.draft,
    sending: op === "send" ? false : m.sending,
    sticky: false
  };
  if (fetch && m.landing !== null) {
    return flush(...reduceError(draftLanded(base), error));
  }
  return flush(...reduceError(base, error));
}
function reduceError(m, error) {
  switch (error._tag) {
    case "BinaryMissing":
      return [{ ...m, banner: `herdr-review not found. Run: ${FETCH}`, stopped: true, pending: false }, []];
    case "TooOld":
      return [{ ...m, banner: `update herdr-review. Run: ${FETCH}`, stopped: true, pending: false }, []];
    case "ContractMismatch":
      return [{ ...m, banner: `unexpected reply at ${error.path}: ${error.detail}`, stopped: true, pending: false }, []];
    case "Busy":
      return [{ ...m, status: "review is busy, try again" }, []];
    case "BackendFailed":
      return [m.state === null ? { ...m, banner: error.stderr || `herdr-review failed (${error.code})` } : { ...m, status: error.stderr || `herdr-review failed (${error.code})` }, []];
    case "BadRequest":
      return [{ ...m, status: error.stderr, pending: m.pending || !m.retried, retried: true }, []];
    case "Refused": {
      const gone = error.why === "AgentGone";
      const status = gone ? "nothing was sent: the agent is gone. Send again to find an agent for this folder" : `nothing was sent: ${error.why}`;
      return [{ ...m, target: gone ? null : m.target, status }, [{ _tag: "Notify", title: "herdr-review", body: status }]];
    }
    case "NoTarget":
      return [{ ...m, status: "no agent found for this folder" }, []];
    case "AmbiguousTarget":
      return [{ ...m, picker: error.candidates, status: null }, []];
    case "WatchUnavailable":
      return [m, []];
    default: {
      const unhandled = error;
      throw new Error(`unhandled error ${JSON.stringify(unhandled)}`);
    }
  }
}

// src/view.ts
import { div as div2, image, list as list2, svg as svg3 } from "gpui-kit";
import { h_flex as h_flex3, Input, Scrollbar as Scrollbar2, Textarea, v_flex as v_flex2 } from "gpui-base";

// src/message-view.ts
import { div, svg as svg2 } from "gpui-kit";
import { h_flex as h_flex2, Scrollbar, v_flex } from "gpui-base";
var tint = (color, alpha) => `${color}${alpha}`;
var NUM_W = 36;
function line(i, t, dispatch) {
  const over = (on) => dispatch({ _tag: "MessageHovered", line: i.n, on });
  return h_flex2().id(`msg-${i.n}`).w_full().min_h(ROW).flex_none().items_start().gap(6).when(i.selected, (e) => e.bg(tint(t.accent, "30"))).on_hover((on) => over(on)).on_mouse_move(() => over(true)).on_mouse_down("left", () => dispatch({ _tag: "MessagePressed", line: i.n })).child(
    div().id(`msg-plus-${i.n}`).w(GUTTER_W).h(ROW).flex_none().flex().items_center().justify_center().when(
      i.hovered,
      (e) => e.cursor_pointer().on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(() => dispatch({ _tag: "MessagePlusClicked", line: i.n })).child(
        svg2("mods/herdr-review/icons/plus.svg").size(16).flex_none().text_color(t[PLUS] ?? t.accent)
      )
    )
  ).child(div().w(NUM_W).h(ROW).flex_none().text_right().text_size(11).text_color(t.dim).child(String(i.n))).child(
    div().flex_1().min_w(0).text_size(12).text_color(t.foreground).child(i.text === "" ? " " : i.text)
  );
}
function card(i, t, dispatch) {
  return v_flex().id(`msg-card-${i.id}`).w_full().flex_none().pl(GUTTER_W + NUM_W + 12).pr(12).py(3).child(
    v_flex().flex_1().max_w(CARD_W).px(10).py(4).gap(2).border_l(2).border_color(t.accent).bg(tint(t.accent, "14")).child(
      h_flex2().gap(6).items_center().child(div().flex_1().min_w(0).truncate().font_semibold().text_size(11).text_color(t.accent).child(i.label)).child(iconButton(`msg-edit-${i.id}`, "pencil", "Edit", "rest", t, () => dispatch({ _tag: "MessageCommentAction", id: i.id, action: "edit" }), 24)).child(iconButton(`msg-delete-${i.id}`, "trash", "Delete", "danger", t, () => dispatch({ _tag: "MessageCommentAction", id: i.id, action: "delete" }), 24))
    ).children(
      i.body.map(
        (text) => div().min_h(16).text_size(12).text_color(t.foreground).child(text === "" ? " " : text)
      )
    )
  );
}
function drawMessage(m, t, dispatch, ui) {
  const mv = m.message;
  const swap = viewButton(m);
  const send = sendButton(m);
  const items = messageItems(mv);
  const waiting = mv.data === null && mv.status === null;
  return v_flex().id("review").size_full().min_h(0).overflow_hidden().bg(t.background).text_color(t.foreground).text_size(13).on_mouse_up("left", () => dispatch({ _tag: "MessageReleased" })).child(
    h_flex2().min_h(42).flex_none().flex_wrap().px(14).py(6).gap(8).items_center().border_b(1).border_color(t.border).child(div().font_semibold().child("Agent message")).when(
      mv.data !== null,
      (e) => e.child(
        div().text_size(11).text_color(t.muted).child(`${mv.data?.agent ?? ""}, ${mv.data?.status ?? ""}`)
      )
    ).child(div().flex_1()).child(button(swap.id, swap.label, false, t, () => dispatch(swap.msg))).child(iconButton("message-refresh", "refresh", mv.loading ? "Reading the message..." : "Refresh", "rest", t, mv.loading ? null : () => dispatch({ _tag: "MessageRefreshed" }))).child(button("message-send", send.label, !send.off, t, send.off ? null : () => dispatch({ _tag: "MessageSendClicked" }), false, "send"))
  ).when(
    mv.status !== null,
    (e) => e.child(
      div().px(12).py(4).text_size(11).text_color(t[WARN] ?? t.foreground).child(mv.status ?? "")
    )
  ).when(mv.loading || waiting, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t.dim).child("Reading the agent's message..."))).child(
    div().flex_1().min_h(0).relative().child(
      v_flex().id("review-message").absolute().inset_0().overflow_y_scroll().py(6).children(items.map((i) => i.kind === "line" ? line(i, t, dispatch) : card(i, t, dispatch)))
    ).child(Scrollbar.vertical("review-message").absolute().inset_0())
  ).child(m.editor !== null ? dockBar(m, t, dispatch, ui) : div().flex_none().border_t(1).border_color(t.border).px(12).py(8).text_size(11).text_color(t.muted).child("Drag over lines to select them, [+] to comment"));
}

// src/view.ts
var SIDEBAR_W = 280;
var INDENT = 20;
var tint2 = (color, alpha) => `${color}${alpha}`;
var drawing = (name) => `mods/herdr-review/icons/${name}.svg`;
function button(id, label, on, t, click, danger = false, icon, tip) {
  return div2().id(id).h(24).px(8).flex().items_center().gap(6).rounded(6).border_1().border_color(danger ? t.ansi1 : on ? t.accent : t.border).text_size(11).text_color(on || danger ? t.foreground : t.muted).when(on && !danger, (e) => e.bg(t.selected)).when(danger, (e) => e.bg(tint2(t.ansi1, "30"))).when(tip !== void 0, (e) => e.tooltip(tip ?? "").accessibility_label(tip ?? "")).when(
    click !== null,
    (e) => e.cursor_pointer().hover((s) => s.bg(t.hover)).on_click((_e, cx) => {
      cx.stop_propagation();
      click?.();
    })
  ).when(click === null, (e) => e.opacity(0.4)).when(
    icon !== void 0,
    (e) => e.child(
      svg3(drawing(icon ?? "")).size(14).flex_none().text_color(danger ? t.ansi1 : on ? t.accent : t.muted)
    )
  ).child(label);
}
function iconButton(id, icon, tip, state, t, click, size = 28) {
  const color = state === "danger" ? t.ansi1 : state === "on" ? t.accent : t.muted;
  return div2().id(id).w(size).h(size).flex_none().flex().items_center().justify_center().rounded(6).text_color(color).when(state === "on", (e) => e.bg(t.selected)).when(state === "danger", (e) => e.bg(tint2(t.ansi1, "30"))).tooltip(tip).accessibility_label(tip).when(
    click !== null,
    (e) => e.cursor_pointer().hover((s) => s.bg(t.hover).text_color(state === "rest" ? t.foreground : color)).active((s) => s.bg(t.selected)).on_click((_e, cx) => {
      cx.stop_propagation();
      click?.();
    })
  ).when(click === null, (e) => e.opacity(0.4)).child(svg3(drawing(icon)).size(16).flex_none().text_color(color));
}
function recordButton(id, b, t, dispatch, size = 24) {
  const click = b.off ? null : () => dispatch(b.msg);
  const tip = b.tip !== void 0 || b.key === void 0 ? b.tip ?? b.label : `${b.label} (${b.key})`;
  if (b.icon !== void 0 && b.iconOnly === true) {
    return iconButton(id, b.icon, tip, b.danger ? "danger" : b.accent === true ? "on" : "rest", t, click, size);
  }
  return button(id, b.label, b.accent === true, t, click, b.danger, b.icon, b.icon === void 0 ? void 0 : tip);
}
function segmented(id, options, t) {
  return h_flex3().id(id).h(28).flex_none().p(2).gap(2).items_center().rounded(7).border_1().border_color(t.border).children(
    options.map(
      (o) => div2().id(o.id).h(22).px(10).flex().items_center().rounded(5).text_size(11).text_color(o.on ? t.foreground : t.muted).when(o.on, (e) => e.bg(t.selected)).tooltip(o.tip).accessibility_label(o.tip).when(
        o.click !== null,
        (e) => e.cursor_pointer().when(!o.on, (x) => x.hover((s) => s.bg(t.hover))).on_click((_e, cx) => {
          cx.stop_propagation();
          o.click?.();
        })
      ).when(o.click === null, (e) => e.opacity(0.4)).child(o.label)
    )
  );
}
var pick2 = (name, t) => t[name] ?? t.foreground;
function slot(half, t, box) {
  const one = box(
    v_flex2().h_full().pr(12).pl(half === null ? 48 : 28).w_full().flex_none()
  );
  if (half === null) {
    return one;
  }
  const cell = (child) => div2().flex_1().min_w(0).h_full().overflow_hidden().when(child !== null, (e) => e.child(child));
  return h_flex3().w_full().h_full().child(cell(half === "old" ? one : null)).child(div2().w(1).h_full().flex_none().bg(t.border)).child(cell(half === "new" ? one : null));
}
function head(item2, t) {
  const color = pick2(toneColor(item2.tone), t);
  return slot(
    item2.half,
    t,
    (e) => e.pt(6).child(
      v_flex2().flex_1().max_w(CARD_W).px(10).py(4).gap(2).border_l(2).border_color(color).bg(item2.focused ? t.selected : tint2(color, "14")).child(
        h_flex3().gap(8).items_center().when(
          item2.collapsible,
          (e2) => e2.child(
            div2().w(10).text_color(t.dim).child(item2.open ? "\u25BE" : "\u25B8")
          )
        ).child(div2().min_w(0).truncate().font_semibold().text_size(11).text_color(color).child(item2.title)).children(
          item2.badges.map(
            (b) => div2().flex_none().text_size(10).text_color(pick2(tagColor(b), t)).child(`[${b}]`)
          )
        )
      ).when(
        item2.quote !== null,
        (e2) => e2.child(
          div2().truncate().text_size(11).text_color(t.dim).child(item2.quote ?? "")
        )
      )
    )
  );
}
function body(item2, t, dispatch) {
  const color = pick2(toneColor(item2.tone), t);
  return slot(
    item2.half,
    t,
    (e) => e.when(item2.last, (x) => x.pb(6)).child(
      v_flex2().flex_1().max_w(CARD_W).px(10).py(2).when(item2.meta !== null, (e2) => e2.pl(22)).border_l(2).border_color(color).bg(item2.focused ? t.selected : tint2(color, "14")).when(
        item2.meta !== null,
        (e2) => e2.child(
          div2().text_size(10).text_color(t.dim).child(item2.meta ?? "")
        )
      ).children(
        item2.lines.map(
          (line2) => div2().min_h(16).text_size(12).text_color(t.foreground).child(line2 === "" ? " " : line2)
        )
      ).when(
        item2.buttons.length > 0,
        (e2) => e2.child(
          h_flex3().flex_wrap().gap(6).pt(4).children(item2.buttons.map((b) => recordButton(`card-${item2.id}-${b.id}`, b, t, dispatch)))
        )
      )
    )
  );
}
function editorBox(item2, m, t, dispatch, ui) {
  const ta = ui.textarea;
  if (ta === null) {
    return div2();
  }
  return slot(
    item2.half,
    t,
    (e) => e.pt(6).pb(6).child(
      v_flex2().id("review-editor").h(EDITOR_H - 12).flex_none().overflow_hidden().px(10).py(6).gap(4).border_l(2).border_color(t.accent).bg(tint2(t.accent, "14")).track_focus(ui.handle).on_key_down((ev, cx) => {
        const msg = editorKey(ev.keystroke, ta.value());
        if (msg !== null) {
          dispatch(msg);
          cx.stop_propagation();
        }
      }).child(
        div2().h(16).flex_none().truncate().text_size(11).font_semibold().text_color(pick2(toneColor("user"), t)).child(item2.title)
      ).child(div2().h(EDITOR_TEXT).flex_none().child(Textarea.new(ta).w_full().h(EDITOR_TEXT).px(8).py(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12).text_color(t.foreground))).child(
        div2().h(16).flex_none().truncate().text_size(11).text_color(t.ansi1).child(m.status ?? "")
      ).child(
        h_flex3().h(24).flex_none().gap(8).items_center().child(button("editor-save", m.saving ? "Saving..." : "Save", true, t, () => dispatch({ _tag: "EditorSaved", text: ta.value() }))).child(button("editor-cancel", "Cancel", false, t, () => dispatch({ _tag: "EditorCancelled" }))).child(div2().min_w(0).truncate().text_size(11).text_color(t.dim).child("Ctrl+S save, Esc cancel"))
      )
    )
  );
}
function viewedBox(item2, t, dispatch) {
  const tip = item2.viewed ? "Mark unviewed" : "Mark viewed";
  return h_flex3().id(`viewed-${item2.key}`).h(22).flex_none().px(6).gap(6).items_center().rounded(6).cursor_pointer().hover((s) => s.bg(t.hover)).active((s) => s.bg(t.selected)).tooltip(tip).accessibility_label(tip).on_click((_e, cx) => {
    cx.stop_propagation();
    dispatch(viewedClick(item2));
  }).child(
    div2().w(14).h(14).flex_none().flex().items_center().justify_center().rounded(3).border_1().border_color(item2.viewed ? t.accent : t.dim).when(item2.viewed, (e) => e.bg(t.selected).child(svg3(drawing("check")).size(10).flex_none().text_color(t.accent)))
  ).child(
    div2().text_size(11).text_color(item2.viewed ? t.foreground : t.muted).child("Viewed")
  );
}
function fileHead2(item2, t, dispatch) {
  return h_flex3().w_full().h_full().pl(12).pr(24).gap(8).items_center().bg(t.raised).border_t(1).border_color(t.border).child(
    div2().w(10).flex_none().text_color(t.dim).child(item2.folded ? "\u25B8" : "\u25BE")
  ).child(
    div2().w(12).flex_none().text_size(11).text_color(pick2(letterColor(item2.letter), t)).child(item2.letter)
  ).child(
    div2().min_w(0).truncate().font_semibold().text_size(12).text_color(item2.viewed ? t.dim : t.foreground).child(item2.title)
  ).when(item2.add > 0, (e) => e.child(div2().flex_none().text_size(11).text_color(t.ansi2).child(`+${item2.add}`))).when(item2.del > 0, (e) => e.child(div2().flex_none().text_size(11).text_color(t.ansi1).child(`-${item2.del}`))).when(item2.hides !== "", (e) => e.child(div2().flex_none().text_size(11).text_color(t.dim).child(item2.hides))).child(div2().flex_1().min_w(0)).child(viewedBox(item2, t, dispatch));
}
function item(i, size, shift, m, t, dispatch, ui) {
  const click = itemClick(i.key);
  return div2().id(`s-${i.key}`).w_full().h(size).flex_none().overflow_hidden().when(
    click !== null,
    (e) => e.cursor_pointer().on_click(() => {
      if (click !== null) {
        dispatch(click);
      }
    })
  ).child(inner(i, shift, m, t, dispatch, ui));
}
function inner(i, shift, m, t, dispatch, ui) {
  if (i.kind === "head") {
    return head(i, t);
  }
  if (i.kind === "body") {
    return body(i, t, dispatch);
  }
  if (i.kind === "editor") {
    return editorBox(i, m, t, dispatch, ui);
  }
  if (i.kind === "file") {
    return fileHead2(i, t, dispatch);
  }
  if (i.kind === "wait") {
    return div2().w_full().pl(48).pt(8).text_size(11).text_color(t.dim).child(i.text);
  }
  if (i.kind === "note") {
    return div2().w_full().pl(48).pt(4).text_size(11).text_color(t.dim).child(i.text);
  }
  if (i.kind === "label") {
    return div2().w_full().px(12).pt(8).pb(2).text_size(11).font_semibold().text_color(t.dim).child(i.text);
  }
  if (i.kind === "split") {
    return splitRow(i, shift, m, t, dispatch);
  }
  return diffRow(i, null, 0, m, t, dispatch);
}
function diffRow(item2, side, shift, m, t, dispatch) {
  const path = keyPath(item2.key);
  const over = (on) => dispatch({ _tag: "Hovered", path, row: item2.index, on, side });
  const key = side === null ? item2.key : `${item2.key}-${side}`;
  if (item2.kind === "hunk") {
    return div2().id(`row-${key}`).w_full().px(12).h(ROW).flex_none().truncate().bg(t.raised).text_color(t.dim).on_hover((on) => over(on)).on_mouse_move(() => over(true)).child(item2.text);
  }
  const { selected, hovered: hovered3 } = rowState(m, item2, side);
  const cursor = cursorOn(m, item2, side);
  const sign = item2.kind === "add" ? "+" : item2.kind === "del" ? "-" : " ";
  const num = (n) => div2().w(36).flex_none().text_right().text_color(t.dim).child(n);
  return h_flex3().id(`row-${key}`).h(ROW).flex_none().gap(6).when(side === null, (e) => e.w_full()).when(side !== null, (e) => e.flex_1().min_w(0).overflow_hidden()).when(item2.kind === "add", (e) => e.bg(tint2(t.ansi2, "18"))).when(item2.kind === "del", (e) => e.bg(tint2(t.ansi1, "18"))).when(selected, (e) => e.bg(tint2(t.accent, "30"))).when(cursor, (e) => e.bg(tint2(t.foreground, "30"))).on_hover((on) => over(on)).on_mouse_move(() => over(true)).on_mouse_down("left", () => dispatch({ _tag: "RowPressed", path, row: item2.index, side })).child(
    div2().id(`plus-${key}`).w(GUTTER_W).h_full().flex_none().flex().items_center().justify_center().when(
      hovered3,
      (e) => e.cursor_pointer().on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click(() => dispatch({ _tag: "PlusClicked", path, row: item2.index, side })).child(svg3("mods/herdr-review/icons/plus.svg").size(16).flex_none().text_color(pick2(PLUS, t)))
    )
  ).when(side !== "new", (e) => e.child(num(item2.old))).when(side !== "old", (e) => e.child(num(item2.new))).child(
    div2().w(10).flex_none().text_color(item2.kind === "add" ? t.ansi2 : item2.kind === "del" ? t.ansi1 : t.dim).child(sign)
  ).child(
    // A half clips its code and shifts it by the model's shift; the unified stream scrolls sideways as a whole instead.
    div2().flex_1().min_w(0).overflow_hidden().child(
      h_flex3().when(side !== null, (e) => e.ml(-shift)).children(
        // A changed word has its own, stronger background, drawn over the row's tint and the selection's alike.
        item2.runs.map(
          (r) => div2().flex_none().whitespace_nowrap().text_color(t[r.color] ?? t.foreground).when(r.marked === true, (e) => e.bg(tint2(item2.kind === "add" ? t.ansi2 : t.ansi1, "48"))).child(r.text)
        )
      )
    )
  );
}
function splitRow(i, shift, m, t, dispatch) {
  const half = (row2, side) => row2 === null ? div2().flex_1().min_w(0).h_full().bg(tint2(t.dim, "10")) : diffRow(row2, side, shift, m, t, dispatch);
  return h_flex3().w_full().h(ROW).flex_none().child(half(i.left, "old")).child(div2().w(1).h_full().flex_none().bg(t.border)).child(half(i.right, "new"));
}
function dockBar(m, t, dispatch, ui) {
  const wrap = v_flex2().id("review-dock").flex_none().border_t(1).border_color(t.border).px(12).py(8).gap(6).text_size(11);
  if (m.picker !== null) {
    return wrap.child(div2().font_semibold().text_color(pick2(WARN, t)).child("Several agents match this folder. Send to which?")).children(pickerOptions(m.picker).map((b) => h_flex3().child(button(`dock-${b.id}`, b.label, false, t, () => dispatch(b.msg)))));
  }
  const btn = (b) => recordButton(`dock-${b.id}`, b, t, dispatch);
  const ask = confirmRow(m);
  if (ask !== null) {
    return wrap.track_focus(ui.handle).on_key_down((ev, cx) => {
      const msg = confirmKey(ev.keystroke);
      if (msg !== null) {
        dispatch(msg);
        cx.stop_propagation();
      }
    }).child(div2().font_semibold().text_color(pick2(ask.color, t)).child(ask.sentence)).child(h_flex3().gap(8).items_center().children(ask.buttons.map(btn)));
  }
  const bar = dock(m);
  return wrap.child(h_flex3().gap(8).items_center().child(div2().flex_1().min_w(0).truncate().text_color(t.muted).child(bar.label)).children(bar.buttons.map(btn))).child(
    h_flex3().gap(8).items_center().flex_wrap().children(bar.actions.map(btn)).when(
      bar.note !== null,
      (x) => x.child(
        div2().text_color(pick2(bar.note?.color ?? "dim", t)).child(bar.note?.text ?? "")
      )
    )
  );
}
function sideRow(r, m, t) {
  if (r.kind === "dir") {
    return h_flex3().w_full().flex_none().px(8).pt(8).pb(2).gap(6).items_center().child(image(drawing(FOLDER)).size(14).flex_none()).child(div2().flex_1().min_w(0).truncate().text_ellipsis_start().text_size(11).text_color(t.dim).child(r.text));
  }
  const letter = pick2(letterColor(r.letter), t);
  return v_flex2().id(r.key).w_full().flex_none().px(8).py(3).rounded(7).cursor_pointer().when(m.selected === r.path, (e) => e.bg(t.selected)).hover((s) => s.bg(t.hover)).child(
    h_flex3().w_full().pl(INDENT).gap(6).items_center().child(div2().w(12).flex_none().text_size(11).text_color(letter).child(r.letter)).child(
      image(drawing(r.icon)).size(14).flex_none().when(r.viewed, (e) => e.opacity(0.5))
    ).child(
      div2().flex_1().min_w(0).truncate().text_color(r.viewed ? t.dim : t.foreground).child(r.name)
    ).when(r.add > 0, (e) => e.child(div2().flex_none().text_size(11).text_color(t.ansi2).child(`+${r.add}`))).when(r.del > 0, (e) => e.child(div2().flex_none().text_size(11).text_color(t.ansi1).child(`-${r.del}`)))
  ).when(
    r.badges.length > 0,
    (e) => e.child(
      h_flex3().w_full().pl(38 + INDENT).gap(8).children(
        r.badges.map(
          (b) => div2().flex_none().text_size(10).text_color(pick2(badgeColor(b), t)).child(b.text)
        )
      )
    )
  );
}
function sidebar(m, t, dispatch, ui, empty) {
  const left = m.panel.side === "left";
  const rows = sidebarRows(m.state === null ? NO_FILES : m.state.files, m.filter);
  const info = filterInfo(m);
  return v_flex2().w(SIDEBAR_W).flex_none().h_full().min_h(0).when(left, (e) => e.border_r(1)).when(!left, (e) => e.border_l(1)).border_color(t.border).child(
    div2().flex_none().px(6).pt(6).child(
      h_flex3().id("review-filter").h(28).px(8).gap(6).items_center().rounded(7).border_1().border_color(m.filtering ? t.accent : t.border).cursor_text().on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(div2().flex_1().min_w(0).child(Input.new(ui.filter).h_full().text_size(12).text_color(t.foreground))).child(div2().flex_none().text_size(10).text_color(t.dim).child(info.count))
    ).when(info.none, (e) => e.child(div2().px(8).pt(4).text_size(11).text_color(t.dim).child("no match")))
  ).child(
    div2().flex_1().min_h(0).relative().child(
      rows.length === 0 ? div2().p(14).text_color(t.dim).child(info.none ? "" : empty) : list2(
        "review-files",
        rows.length,
        (i) => rows[i]?.key ?? `row:${i}`,
        (i) => {
          const r = rows[i];
          return div2().w_full().px(6).when(i === 0, (e) => e.pt(6)).when(i === rows.length - 1, (e) => e.pb(6)).child(r === void 0 ? div2() : sideRow(r, m, t));
        }
      ).absolute().inset_0().on_item_click((key) => {
        const r = rows.find((x) => x.key === key);
        if (r?.kind === "file") {
          dispatch({ _tag: "FileChosen", path: r.path });
        }
      })
    ).child(Scrollbar2.vertical("review-files").absolute().inset_0())
  );
}
function diffPane(m, stream, t, dispatch, ui) {
  const { first, end, shift } = visibleRange(stream, m.offset, m.height);
  const kids = [];
  const max = maxShift(codeWidth(stream.items.slice(first, end)), m.width);
  for (let i = first; i < end; i++) {
    const it = stream.items[i];
    if (it !== void 0) {
      kids.push(item(it, stream.sizes[i] ?? ROW, m.shift, m, t, dispatch, ui));
    }
  }
  const wheel = (e, cx) => {
    const dx = isSplit(m) ? wheelShift(e) : null;
    if (dx !== null) {
      cx.stop_propagation();
      dispatch({ _tag: "Shifted", dx, max });
      return;
    }
    const dy = wheelDelta(e);
    if (dy === null) {
      return;
    }
    cx.stop_propagation();
    dispatch({ _tag: "Wheeled", dy });
  };
  const thumb = thumbOf(stream.total, m.height, m.offset);
  return v_flex2().flex_1().min_w(0).h_full().min_h(0).child(
    div2().id("review-stream-box").flex_1().min_h(0).relative().overflow_hidden().text_size(11).on_size_change((size) => dispatch({ _tag: "Resized", width: Math.round(size.width), height: Math.round(size.height) })).on_mouse_move((e) => {
      if (m.grip !== null) {
        dispatch({ _tag: "ThumbMoved", y: e.position.y });
      }
    }).child(
      div2().id("review-stream-x").size_full().overflow_x_scrollbar().child(div2().id("review-stream-content").w_full().min_w(stream.wide).h_full().relative().overflow_hidden().on_scroll_wheel(wheel).child(v_flex2().id("review-stream-items").absolute().top(-shift).left(0).w_full().children(kids)))
    ).when(
      thumb !== null,
      (e) => e.child(
        div2().id("review-stream-track").absolute().top(0).right(0).w(12).h(m.height).bg(t.chrome).on_scroll_wheel(wheel).on_mouse_down("left", (ev) => {
          if (ev.local_position !== void 0) {
            dispatch({ _tag: "TrackClicked", y: ev.local_position.y });
          }
        }).child(
          div2().id("review-stream-thumb").absolute().top(thumb?.top ?? 0).left(2).w(8).h(thumb?.size ?? 0).rounded(4).bg(m.grip !== null ? t.accent : t.dim).on_mouse_down("left", (ev, cx) => {
            cx.stop_propagation();
            dispatch({ _tag: "ThumbGrabbed", y: ev.position.y });
          })
        )
      )
    )
  ).child(dockBar(m, t, dispatch, ui));
}
function draw(m, stream, t, dispatch, ui) {
  if (m.view === "message") {
    return drawMessage(m, t, dispatch, ui);
  }
  const sum = totals(m.state);
  const base = m.state === null ? null : m.state.base;
  const notices = m.state === null ? [] : [...m.state.notices, ...m.state.warnings];
  const empty = m.banner !== null ? "" : m.state === null ? "Loading changes..." : "No changes";
  return v_flex2().id("review").size_full().min_h(0).overflow_hidden().bg(t.background).text_color(t.foreground).text_size(13).on_mouse_up("left", () => dispatch({ _tag: "Released" })).when(
    m.editor === null,
    (e) => e.track_focus(ui.nav).on_key_down((ev, cx) => {
      const msg = panelKey(m, ev.keystroke);
      if (msg !== null) {
        dispatch(msg);
        cx.stop_propagation();
      }
    })
  ).child(
    h_flex3().min_h(42).flex_none().flex_wrap().px(14).py(6).gap(8).items_center().border_b(1).border_color(t.border).child(div2().font_semibold().child("Review")).child(div2().text_size(11).text_color(t.muted).child(`${sum.files} files`)).child(div2().text_size(11).text_color(t.ansi2).child(`+${sum.add}`)).child(div2().text_size(11).text_color(t.ansi1).child(`-${sum.del}`)).child(div2().flex_1()).child(
      h_flex3().flex_none().gap(8).items_center().child(
        segmented(
          "spec",
          [
            { id: "spec-worktree", label: "Uncommitted", tip: "Working tree: changes not yet committed (staged, unstaged and untracked)", on: m.spec !== null && m.spec.kind === "worktree", click: () => dispatch({ _tag: "SpecChosen", spec: { kind: "worktree" } }) },
            {
              id: "spec-branch",
              label: base === null ? "Branch" : `vs ${base}`,
              tip: base === null ? "No base branch found" : `Branch: everything this branch changed since it left ${base}, committed or not`,
              on: m.spec !== null && m.spec.kind === "branch",
              click: base === null ? null : () => dispatch({ _tag: "SpecChosen", spec: { kind: "branch", base } })
            }
          ],
          t
        )
      ).child(button("view-message", "Agent message", false, t, () => dispatch({ _tag: "ViewChosen", view: "message" }))).child(div2().w(1).h(16).flex_none().bg(t.border)).children([...sidebarButtons(m), layoutButton(m)].map((b) => iconButton(b.id, b.icon, b.label, b.on ? "on" : "rest", t, () => dispatch(b.msg)))).child(iconButton("refresh", "refresh", "Refresh", "rest", t, () => dispatch({ _tag: "Triggered", source: "refresh" })))
    )
  ).when(
    m.banner !== null,
    (e) => e.child(
      div2().px(12).py(6).text_size(12).text_color(t.ansi1).child(m.banner ?? "")
    )
  ).when(
    m.status !== null,
    (e) => e.child(
      div2().px(12).py(4).text_size(11).text_color(pick2(WARN, t)).child(m.status ?? "")
    )
  ).when(notices.length > 0, (e) => e.child(div2().px(12).py(4).text_size(11).text_color(t.dim).child(notices.join(" | ")))).child(
    h_flex3().flex_1().min_h(0).items_start().children(paneOrder(m).map((p) => p === "sidebar" ? sidebar(m, t, dispatch, ui, empty) : diffPane(m, stream, t, dispatch, ui)))
  );
}

// src/main.ts
var TIMER_MS = 1e4;
var GLANCE_MS = 500;
var TOGGLE_MS = 500;
var ReviewView = class extends View {
  model = initModel();
  closed = false;
  // Set only on the tab-bar button's copy: the review pane it last saw in the focused tab (null for none).
  toggle = void 0;
  cx;
  env;
  handle;
  nav;
  textarea = null;
  filter;
  // Timing: a recorder that does nothing until init reads `perf: true` from the props.
  perf = noop();
  // The stream's items, heights and prefix sums, kept while nothing they are built from changes (a wheel tick, a hover and a
  // drag change none of it).
  layout = layoutCache(() => this.perf.rebuild());
  init(_props, cx) {
    this.cx = cx;
    if (decodeProps(realHost.props()).toggle) {
      this.toggle = reviewPane(realHost.panes(), realHost.focused());
      cx.timer.every(TOGGLE_MS, () => {
        const now = reviewPane(realHost.panes(), realHost.focused());
        if (now !== this.toggle) {
          this.toggle = now;
          cx.notify();
        }
      });
      return;
    }
    this.perf = recorderFor(realHost);
    this.handle = cx.focus_handle();
    this.nav = cx.focus_handle();
    this.filter = InputState.new({ placeholder: "filter (/)" });
    this.filter.on("change", () => this.dispatch({ _tag: "FilterChanged", text: this.filter.value() }));
    this.filter.on("submit", () => this.dispatch({ _tag: "FilterEntered" }));
    this.filter.on("focus", () => this.dispatch({ _tag: "FilterFocused", on: true }));
    this.filter.on("blur", () => this.dispatch({ _tag: "FilterFocused", on: false }));
    this.env = {
      host: realHost,
      cli: makeCli(realHost, decodeProps(realHost.props()).bin),
      dispatch: (msg) => this.dispatch(msg),
      closed: () => this.closed
    };
    const opened = open(realHost);
    this.dispatch(opened);
    this.start({ _tag: "ReadPanel" });
    cx.spawn(() => listen(this.env, () => this.focusNav()));
    if (opened._tag === "Opened" && opened.root !== null) {
      const root = opened.root;
      cx.spawn(() => watch(this.env, root, "watch-root"));
      cx.timer.every(TIMER_MS, () => this.dispatch({ _tag: "Triggered", source: "timer" }));
      cx.timer.every(GLANCE_MS, () => this.dispatch({ _tag: "Glanced" }));
    }
  }
  dispatch(msg) {
    if (this.closed) {
      return;
    }
    const t0 = this.perf.start();
    const before = this.model;
    let [model, cmds] = update(before, msg);
    if (model !== before) {
      const sync = scrolled(model, this.layout(model, Date.now()));
      if (sync !== null) {
        const [next, more] = update(model, sync);
        model = next;
        cmds = [...cmds, ...more];
      }
    }
    this.model = model;
    this.perf.dispatch(t0);
    if (model.editor !== before.editor) {
      this.syncEditor();
    }
    if (model.confirm !== null && model.confirm !== before.confirm || model.confirmArchive && !before.confirmArchive) {
      this.focusAsk();
    }
    if (msg._tag === "ThreadJumped" || before.editor !== null && model.editor === null || (before.confirm !== null || before.confirmArchive) && model.confirm === null && !model.confirmArchive) {
      this.focusNav();
    }
    if (msg._tag === "RowPressed") {
      this.focusNav();
    }
    if (msg._tag === "FilterCleared") {
      this.filter.set_value("");
    }
    if (msg._tag === "FilterOpened") {
      this.focusFilter();
    }
    if (before.filtering && !model.filtering && msg._tag !== "FilterFocused") {
      this.focusNav();
    }
    if (model !== before) {
      this.cx.notify();
    }
    for (const cmd of cmds) {
      this.start(cmd);
    }
  }
  // The editor's text lives in a textarea state the view owns: made when an editor opens, released when it
  // closes. Its box in the stream has a fixed height (EDITOR_TEXT), so it does not grow with the text. Dispatch runs in click handlers when an editor opens, where creating state is allowed.
  syncEditor() {
    this.textarea?.release();
    this.textarea = null;
    const e = this.model.editor;
    if (e === null) {
      return;
    }
    const ta = TextareaState.new({ placeholder: "Write a note...", value: e.initial, rows: 5 });
    ta.focus();
    this.cx.timer.after(600, () => {
      if (this.textarea === ta) {
        ta.focus();
      }
    });
    this.textarea = ta;
  }
  // A question is up: the keyboard goes to the bar that holds it, so Esc reaches it. Tried again once the
  // bar is drawn, as for the editor.
  focusAsk() {
    this.handle.focus();
    this.cx.timer.after(300, () => {
      if (this.model.confirm !== null || this.model.confirmArchive) {
        this.handle.focus();
      }
    });
  }
  // The keyboard goes to the panel so n and N reach it. Tried again once the root is drawn with its handle,
  // as for the editor.
  focusNav() {
    this.nav.focus();
    this.cx.timer.after(300, () => {
      if (!this.closed && this.model.editor === null) {
        this.nav.focus();
      }
    });
  }
  // `/`: the keyboard goes to the box. Tried again once it is drawn, for a sidebar that was hidden.
  focusFilter() {
    this.filter.focus();
    this.cx.timer.after(300, () => {
      if (!this.closed && this.model.panel.open && !this.model.filtering) {
        this.filter.focus();
      }
    });
  }
  start(cmd) {
    this.cx.spawn(async () => {
      const reply = await run2(this.env, cmd);
      if (reply !== null) {
        this.dispatch(reply);
      }
    });
  }
  render(_cx) {
    if (this.toggle !== void 0) {
      const pane = reviewPane(realHost.panes(), realHost.focused());
      const click = () => pane === null ? realHost.run("pane.split", { direction: "right", mod: "herdr-review" }) : realHost.run("pane.close", { pane });
      return drawToggle(pane !== null, realHost.theme(), click);
    }
    const t0 = this.perf.start();
    const stream = this.layout(this.model, Date.now());
    const el = draw(this.model, stream, themed(realHost.theme(), this.model), (msg) => this.dispatch(msg), { handle: this.handle, nav: this.nav, textarea: this.textarea, filter: this.filter });
    if (this.perf.on) {
      const { first, end } = this.model.view === "message" ? { first: 0, end: 0 } : visibleRange(stream, this.model.offset, this.model.height);
      this.perf.render(t0, end - first);
    }
    return el;
  }
  // Bounded: set the flag and return. The watch loops end after their next wake.
  close(_cx) {
    this.closed = true;
    if (this.toggle !== void 0) {
      return;
    }
    this.perf.flush();
    this.textarea?.release();
    this.textarea = null;
    this.filter.release();
  }
};
export {
  ReviewView as default
};
