// The boundary: everything that comes from outside the mod is checked here, once, by hand. Inside the
// mod the types are trusted. A bad reply throws ContractMismatch with the path of the field.
import type { Anchor, ArchiveReply, Candidate, Change, Colors, Comment, Focus, FileSummary, FileView, MessageReply, MessageSentReply, PaneEntry, Panel, Placement, Props, ReviewError, Row, SendReply, ThemeReply, Spec, State, Thread, Mark, Token, TokenKind } from "./types.ts";

const SCHEMA = 1;

function bad(path: string, detail: string): never {
  const error: ReviewError = { _tag: "ContractMismatch", path, detail };
  throw error;
}

type Obj = Record<string, unknown>;

function obj(v: unknown, path: string): Obj {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    return bad(path, "expected an object");
  }
  return v as Obj;
}
function str(o: Obj, k: string, path: string): string {
  const v = o[k];
  return typeof v === "string" ? v : bad(`${path}.${k}`, "expected a string");
}
function strOrNull(o: Obj, k: string, path: string): string | null {
  const v = o[k];
  return v === null ? null : typeof v === "string" ? v : bad(`${path}.${k}`, "expected a string or null");
}
function int(o: Obj, k: string, path: string): number {
  const v = o[k];
  return typeof v === "number" && Number.isInteger(v) ? v : bad(`${path}.${k}`, "expected an integer");
}
function intOrNull(o: Obj, k: string, path: string): number | null {
  const v = o[k];
  return v === null ? null : typeof v === "number" && Number.isInteger(v) ? v : bad(`${path}.${k}`, "expected an integer or null");
}
function bool(o: Obj, k: string, path: string): boolean {
  const v = o[k];
  return typeof v === "boolean" ? v : bad(`${path}.${k}`, "expected a boolean");
}
function list<T>(o: Obj, k: string, path: string, item: (v: unknown, path: string) => T): T[] {
  const v = o[k];
  if (!Array.isArray(v)) {
    return bad(`${path}.${k}`, "expected an array");
  }
  return v.map((x, i) => item(x, `${path}.${k}[${i}]`));
}
function oneOf<T extends string>(o: Obj, k: string, path: string, allowed: readonly T[]): T {
  const v = o[k];
  return typeof v === "string" && (allowed as readonly string[]).includes(v) ? (v as T) : bad(`${path}.${k}`, `expected one of ${allowed.join(", ")}`);
}
function strings(v: unknown, path: string): string {
  return typeof v === "string" ? v : bad(path, "expected a string");
}

// "worktree" or "branch:<base>", the text meta.json and the CLI use.
function spec(o: Obj, k: string, path: string): Spec {
  const text = str(o, k, path);
  if (text === "worktree") {
    return { kind: "worktree" };
  }
  if (text.startsWith("branch:") && text.length > 7) {
    return { kind: "branch", base: text.slice(7) };
  }
  return bad(`${path}.${k}`, "expected worktree or branch:<base>");
}

function schema(o: Obj): void {
  const got = o["schema"];
  if (got !== SCHEMA) {
    bad("schema", `this mod reads schema ${SCHEMA}, the binary sent ${JSON.stringify(got)}`);
  }
}

const CHANGES: readonly Change[] = ["modified", "added", "deleted", "renamed", "untracked", "binary", "submodule", "too_large", "unparsed"];

function fileSummary(v: unknown, path: string): FileSummary {
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
    unsent: int(o, "unsent", path),
  };
}

const TOKEN_KINDS: readonly TokenKind[] = ["comment", "string", "number", "keyword", "operator", "function", "type"];

// The coloured parts of a row, `[start, end, kind]` each. The field is optional (a binary built without syntax
// colours or older than the mod sends none), and colour is only decoration, so it is read as loosely as props: a part
// that is malformed, out of order, overlapping, past the text or of a kind this mod does not know is dropped, and a
// field that is not a list is no parts.
function tokens(o: Obj, text: string): Token[] {
  const v = o["tokens"];
  if (!Array.isArray(v)) {
    return [];
  }
  const out: Token[] = [];
  let end = 0;
  for (const t of v) {
    if (!Array.isArray(t) || t.length !== 3) {
      continue;
    }
    const [from, to, kind] = t as unknown[];
    if (typeof from !== "number" || typeof to !== "number" || !Number.isInteger(from) || !Number.isInteger(to)) {
      continue;
    }
    if (typeof kind !== "string" || !(TOKEN_KINDS as readonly string[]).includes(kind)) {
      continue;
    }
    if (from < end || from >= to || to > text.length) {
      continue;
    }
    out.push({ from, to, kind: kind as TokenKind });
    end = to;
  }
  return out;
}

// The changed words of a row, `{from, to}` each, read as loosely as tokens and dropped on the same grounds.
function marks(o: Obj, text: string): Mark[] {
  const v = o["marks"];
  if (!Array.isArray(v)) {
    return [];
  }
  const out: Mark[] = [];
  let end = 0;
  for (const m of v) {
    if (typeof m !== "object" || m === null) {
      continue;
    }
    const { from, to } = m as Record<string, unknown>;
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

function row(v: unknown, path: string): Row {
  const o = obj(v, path);
  const text = str(o, "text", path);
  return {
    kind: oneOf(o, "kind", path, ["hunk", "ctx", "add", "del"] as const),
    old: intOrNull(o, "old", path),
    new: intOrNull(o, "new", path),
    text,
    tokens: tokens(o, text),
    marks: marks(o, text),
  };
}

function placement(v: unknown, path: string): Placement {
  const o = obj(v, path);
  return {
    kind: oneOf(o, "kind", path, ["matched", "outdated", "not_in_diff"] as const),
    row: intOrNull(o, "row", path),
    start_row: intOrNull(o, "start_row", path),
  };
}

function anchor(v: unknown, path: string): Anchor {
  const o = obj(v, path);
  const side = o["side"];
  return {
    path: str(o, "path", path),
    old_path: strOrNull(o, "old_path", path),
    kind: oneOf(o, "kind", path, ["line", "range", "file"] as const),
    side: side === null ? null : oneOf(o, "side", path, ["old", "new"] as const),
    line: intOrNull(o, "line", path),
    end_line: intOrNull(o, "end_line", path),
    text: strOrNull(o, "text", path),
    spec: spec(o, "spec", path),
  };
}

function comment(v: unknown, path: string): Comment {
  const o = obj(v, path);
  return {
    id: str(o, "id", path),
    parent: strOrNull(o, "parent", path),
    author: str(o, "author", path),
    at: str(o, "at", path),
    body: str(o, "body", path),
    sent: bool(o, "sent", path),
    edited_since_sent: bool(o, "edited_since_sent", path),
  };
}

function thread(v: unknown, path: string): Thread {
  const o = obj(v, path);
  return {
    id: str(o, "id", path),
    status: oneOf(o, "status", path, ["open", "resolved"] as const),
    resolved_by: strOrNull(o, "resolved_by", path),
    is_new: bool(o, "is_new", path),
    unsent: bool(o, "unsent", path),
    placement: placement(o["placement"], `${path}.placement`),
    anchor: anchor(o["anchor"], `${path}.anchor`),
    comments: list(o, "comments", path, comment),
  };
}

export function decodeState(raw: unknown): State {
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
    orphans: list(o, "orphans", "$", thread),
  };
}

export function decodeFile(raw: unknown): FileView {
  const o = obj(raw, "$");
  schema(o);
  const rows = list(o, "rows", "$", row);
  const threads = list(o, "threads", "$", thread);
  for (const [i, t] of threads.entries()) {
    for (const k of ["row", "start_row"] as const) {
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
    threads,
  };
}

// The reply of `files`: the files asked for, each as `file` prints it, in the order of the paths. Anything else is
// a contract mismatch, so a file is never drawn from another file's reply.
export function decodeFiles(raw: unknown, paths: string[]): FileView[] {
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

function candidate(v: unknown, path: string): Candidate {
  const o = obj(v, path);
  return { address: str(o, "address", path), agent: str(o, "agent", path), cwd: str(o, "cwd", path), status: str(o, "status", path) };
}

// A send reply, or the JSON error of a refused one: the error is thrown as the tagged object the reducer reads.
export function decodeSend(raw: unknown): SendReply {
  const o = obj(raw, "$");
  schema(o);
  if (o["error"] !== undefined) {
    const kind = oneOf(o, "error", "$", ["ambiguous", "no_agent", "blocked", "not_ready", "agent_gone", "busy", "backend"] as const);
    const message = str(o, "message", "$");
    const candidates = list(o, "candidates", "$", candidate);
    const error: ReviewError =
      kind === "ambiguous" && candidates.length > 0
        ? { _tag: "AmbiguousTarget", candidates }
        : kind === "no_agent"
          ? { _tag: "NoTarget" }
          : kind === "blocked"
            ? { _tag: "Refused", why: "Blocked" }
            : kind === "not_ready"
              ? { _tag: "Refused", why: "NotReady" }
              : kind === "agent_gone"
                ? { _tag: "Refused", why: "AgentGone" }
                : kind === "busy"
                  ? { _tag: "Busy" }
                  : { _tag: "BackendFailed", code: 1, stderr: message }; // backend, or an ambiguous reply with nothing to pick
    throw error;
  }
  return { schema: SCHEMA, sent: int(o, "sent", "$"), queued: bool(o, "queued", "$"), agent: strOrNull(o, "agent", "$"), warnings: list(o, "warnings", "$", strings) };
}

// `message --json`: the agent's newest message. A failure is not this reply: the CLI exits non-zero with the reason on stderr.
export function decodeMessage(raw: unknown): MessageReply {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, agent: str(o, "agent", "$"), status: str(o, "status", "$"), id: str(o, "id", "$"), lines: list(o, "lines", "$", strings) };
}

export function decodeMessageSent(raw: unknown): MessageSentReply {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, sent: int(o, "sent", "$") };
}

// `theme --json`. The colours are decoration, so a key that is not #rrggbb is dropped and the panel keeps herdr-app's colour
// for that name. The null of "app" is the only null accepted.
export function decodeTheme(raw: unknown): ThemeReply {
  const o = obj(raw, "$");
  schema(o);
  const gui = oneOf(o, "gui", "$", ["app", "plugin"] as const);
  const warnings = list(o, "warnings", "$", strings);
  if (o["colors"] === null) {
    return { schema: SCHEMA, gui, colors: null, warnings };
  }
  const colors: Colors = {};
  for (const [k, v] of Object.entries(obj(o["colors"], "$.colors"))) {
    if (typeof v === "string" && /^#[0-9a-fA-F]{6}$/.test(v)) {
      colors[k] = v as `#${string}`;
    }
  }
  return { schema: SCHEMA, gui, colors, warnings };
}

export function decodeArchive(raw: unknown): ArchiveReply {
  const o = obj(raw, "$");
  schema(o);
  return { schema: SCHEMA, archived: int(o, "archived", "$"), unsent: int(o, "unsent", "$") };
}

// The text of a reply is JSON, or it is not. Both failures are ContractMismatch.
export function parseJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return bad("$", "the reply is not JSON");
  }
}

function field(v: unknown, k: string): unknown {
  return typeof v === "object" && v !== null ? (v as Obj)[k] : undefined;
}
const nonEmpty = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);

// Never throws: a field that is not what it should be is dropped.
export function decodeProps(raw: unknown): Props {
  return { root: nonEmpty(field(raw, "root")), target: nonEmpty(field(raw, "target")), bin: nonEmpty(field(raw, "bin")) };
}

// The settings file is the user's to edit, so it is untrusted too. Never throws: text that is not JSON, or a field
// that is not one of the allowed values, gives that field's default (left, shown).
export function decodePanel(text: string | null): Panel {
  let raw: unknown = null;
  try {
    raw = text === null ? null : JSON.parse(text);
  } catch {
    raw = null;
  }
  return { side: field(raw, "side") === "right" ? "right" : "left", open: field(raw, "open") !== false };
}

// focused() and panes() are the host's, not ours, so they are read as loosely as props.
export function decodeFocus(raw: unknown): Focus | null {
  if (typeof raw !== "object" || raw === null) {
    return null;
  }
  return {
    tab: nonEmpty(field(raw, "tab")),
    cwd: nonEmpty(field(raw, "cwd")),
    terminal: nonEmpty(field(raw, "terminal")),
    agent: nonEmpty(field(raw, "agent")) !== null,
  };
}

export function decodePanes(raw: unknown): PaneEntry[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: PaneEntry[] = [];
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
