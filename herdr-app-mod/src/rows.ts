// Pure helpers for drawing: the diff rows in order, the text cleaned for the screen, the file list lines.
import { fileIcon } from "./icons.ts";
import type { Candidate, Change, Comment, Editor, FileSummary, FileView, Model, Msg, NewAnchor, Row, Selection, State, Thread, Token, TokenKind } from "./types.ts";

// Control characters become nothing and a tab becomes spaces, as in the TUI. The text is the agent's or
// the file's, so it is never trusted to be printable.
export function cleanText(text: string): string {
  return text.replace(/\t/g, "    ").replace(/[\u0000-\u001f\u007f-\u009f]/g, "");
}

// Comment bodies keep their newlines, which cleanText would drop with the other control characters.
export function bodyLines(text: string): string[] {
  return text.replace(/\r\n?/g, "\n").split("\n").map(cleanText);
}

// A stretch of a code row drawn in one colour, named as `theme()` names it.
export type Run = { text: string; color: string };

// The theme colour each kind is drawn in, by the name `theme()` gives it. The TUI's choices, as near as the
// terminal palette goes: green strings, blue functions, pink keywords, yellow numbers, teal types, a comment as dim
// text. An operator is the palette's pale ansi7, which sits close to the plain text. The reds are left to the
// removed lines, and `blocked` was tried for numbers and is red.
const TOKEN_COLORS: Record<TokenKind, string> = {
  comment: "dim",
  string: "ansi2",
  number: "ansi3",
  keyword: "ansi5",
  operator: "ansi7",
  function: "ansi4",
  type: "ansi6",
};
export const tokenColor = (kind: TokenKind): string => TOKEN_COLORS[kind];
const PLAIN = "foreground";

// The row's text cut where its tokens begin and end, each piece cleaned for the screen. Cleaning cuts piece by
// piece and gives what cleaning the whole would, since it works one character at a time. Offsets are UTF-16 code
// units, as `slice` counts them. Text between tokens is the plain foreground, and a row with no tokens is one run.
export function runs(text: string, tokens: Token[]): Run[] {
  const out: Run[] = [];
  const push = (from: number, to: number, color: string): void => {
    const piece = cleanText(text.slice(from, to));
    if (piece !== "") out.push({ text: piece, color });
  };
  let at = 0;
  for (const t of tokens) {
    push(at, t.from, PLAIN);
    push(t.from, t.to, tokenColor(t.kind));
    at = t.to;
  }
  push(at, text.length, PLAIN);
  return out;
}

export type Tone = "user" | "agent" | "resolved";

// The theme colour each thing is drawn in, by the name `theme()` gives it, as the syntax colours are. The user's
// cards take the accent, the agent's their own pink, a warning is yellow, and red (ansi1) is left to removed lines
// and Delete. `blocked` is the agent-status colour, red in some themes, so nothing here returns it.
export const WARN = "ansi3";
export const toneColor = (tone: Tone): string => (tone === "user" ? "accent" : tone === "agent" ? "ansi5" : "ansi2");
export const tagColor = (tag: string): string => (tag === "unsent" || tag === "new" ? WARN : "dim");
export const letterColor = (letter: string): string => (letter === "A" || letter === "?" ? "ansi2" : letter === "D" ? "ansi1" : WARN);
export type RowItem = { kind: "hunk" | "ctx" | "add" | "del"; key: string; index: number; old: string; new: string; text: string; runs: Run[] };
// A thread card is a head, then one body per comment while it is open. The last item of a card closes the box.
export type HeadItem = { kind: "head"; key: string; id: string; tone: Tone; title: string; badges: string[]; quote: string | null; collapsible: boolean; open: boolean; focused: boolean; last: boolean };
export type BodyItem = { kind: "body"; key: string; id: string; tone: Tone; meta: string | null; lines: string[]; focused: boolean; last: boolean };
export type LabelItem = { kind: "label"; key: string; text: string };
// The row that starts a file in the stream. A folded file is this row alone. `hides` says what a fold covers.
export type FileItem = { kind: "file"; key: string; path: string; title: string; letter: string; add: number; del: number; viewed: boolean; folded: boolean; hides: string };
// What stands for a file whose diff has not arrived: a stretch as tall as the diff is guessed to be.
export type WaitItem = { kind: "wait"; key: string; text: string; height: number };
export type NoteItem = { kind: "note"; key: string; text: string };
export type StreamItem = RowItem | HeadItem | BodyItem | LabelItem | FileItem | WaitItem | NoteItem;

const sideOf = (row: Row): "old" | "new" => (row.kind === "del" ? "old" : "new");
const lineOn = (row: Row, side: "old" | "new"): number | null => (side === "old" ? row.old : row.new);

// One row picked: a one-row selection on the side the row lives on, and nothing for a hunk header.
export function pick(file: FileView, row: number): Selection | null {
  const r = file.rows[row];
  if (r === undefined) throw new Error(`row ${row} is out of range`);
  return r.kind === "hunk" ? null : { path: file.path, side: sideOf(r), from: row, to: row, anchor: row };
}

// What a [+] on a row comments on: the selection when the row is one of its lines, else the row alone. null for a
// hunk header.
export function plusTarget(file: FileView, sel: Selection | null, row: number): Selection | null {
  const r = file.rows[row];
  if (r === undefined) throw new Error(`row ${row} is out of range`);
  return sel !== null && sel.path === file.path && row >= sel.from && row <= sel.to && lineOn(r, sel.side) !== null ? sel : pick(file, row);
}

// The range a drag makes between the row it began on (`first`) and the row the pointer is on. It takes the first row's
// side, and its ends are the first and last row in between that have a line on that side.
export function span(file: FileView, first: number, second: number): Selection | null {
  const a = file.rows[first];
  if (a === undefined || file.rows[second] === undefined) throw new Error(`row ${first} or ${second} is out of range`);
  if (a.kind === "hunk") return null;
  const side = sideOf(a);
  const on: number[] = [];
  for (let i = Math.min(first, second); i <= Math.max(first, second); i++) {
    const r = file.rows[i];
    if (r !== undefined && lineOn(r, side) !== null) on.push(i);
  }
  const from = on[0];
  const to = on[on.length - 1];
  return from === undefined || to === undefined ? null : { path: file.path, side, from, to, anchor: first };
}

export function anchorOf(file: FileView, sel: Selection): NewAnchor {
  const from = file.rows[sel.from];
  const to = file.rows[sel.to];
  if (from === undefined || to === undefined) throw new Error("selection is out of range");
  const line = lineOn(from, sel.side);
  const end = lineOn(to, sel.side);
  return { path: file.path, old_path: sel.side === "old" ? file.old_path : null, side: sel.side, line, end_line: sel.from === sel.to ? null : end };
}

// The one-entry batch `comment apply --file` reads.
export function batchOf(anchor: NewAnchor, text: string): string {
  const entry: Record<string, unknown> = { path: anchor.path };
  if (anchor.old_path !== null) entry.old_path = anchor.old_path;
  if (anchor.side !== null) entry.side = anchor.side;
  if (anchor.line !== null) entry.line = anchor.line;
  if (anchor.end_line !== null) entry.end_line = anchor.end_line;
  entry.body = text;
  return JSON.stringify({ comments: [entry] });
}

const lines = (from: number | null, to: number | null): string => (from === null ? "" : to === null || to === from ? String(from) : `${from}-${to}`);

// Where a comment points, as the TUI writes it: `src/a.rs R12-18`, `L` for the old side, the bare path for a file.
export function anchorLabel(a: { path: string; side: "old" | "new" | null; line: number | null; end_line: number | null }): string {
  return a.side === null || a.line === null ? cleanText(a.path) : `${cleanText(a.path)} ${a.side === "old" ? "L" : "R"}${lines(a.line, a.end_line)}`;
}

export function selectionLabel(file: FileView, sel: Selection): string {
  const a = anchorOf(file, sel);
  const count = a.line === null ? 1 : (a.end_line ?? a.line) - a.line + 1;
  return `${anchorLabel(a)} (${count} ${count === 1 ? "line" : "lines"})`;
}

// A selection survives a reload when the rows it covers are the same.
export function sameRows(before: FileView | null, after: FileView, sel: Selection): boolean {
  if (before === null || before.path !== sel.path || after.path !== sel.path) return false;
  for (let i = sel.from; i <= sel.to; i++) {
    const a = before.rows[i];
    const b = after.rows[i];
    if (a === undefined || b === undefined || a.kind !== b.kind || a.old !== b.old || a.new !== b.new || a.text !== b.text) return false;
  }
  return true;
}

export function findComment(files: ReadonlyMap<string, FileView>, state: State | null, id: string | null): { thread: Thread; comment: Comment } | null {
  if (id === null) return null;
  for (const thread of [...[...files.values()].flatMap((f) => f.threads), ...(state === null ? [] : state.orphans)]) {
    const comment = thread.comments.find((c) => c.id === id);
    if (comment !== undefined) return { thread, comment };
  }
  return null;
}

// The root ids of a file's threads in the order its cards are drawn: the ones with no row first, then by row.
// The reply lists them in the order the review wrote them, which is not the order on screen.
export function cardOrder(file: FileView): string[] {
  const row = (t: Thread): number => t.placement.row ?? -1;
  return [...file.threads].sort((a, b) => row(a) - row(b)).map((t) => t.id);
}

// A place a jump can land: a thread, or a file whose threads are not on screen (not loaded, or folded).
export type Stop = { id: string } | { path: string };

const summaryOf = (m: Model, path: string): FileSummary | undefined => m.state?.files.find((f) => f.path === path);

// A file the user folded, or one marked viewed that they have not opened. A fold is not saved.
export const isFolded = (m: Model, f: FileSummary): boolean => m.folds.get(f.path) ?? f.viewed;

// Every thread in the order the TUI walks them: the ones not in the diff, then the files in the sidebar's order
// with each file's threads in card order. A file whose threads are not drawn (not loaded yet, or folded) that
// holds some is one stop. `top` is where the selected file's stops begin.
export function stops(m: Model): { all: Stop[]; top: number } {
  const all: Stop[] = (m.state === null ? [] : m.state.orphans).map((t) => ({ id: t.id }));
  let top = 0;
  for (const f of m.state === null ? [] : m.state.files) {
    if (m.selected === f.path) top = all.length;
    const view = m.files.get(f.path);
    if (view !== undefined && !isFolded(m, f)) all.push(...cardOrder(view).map((id) => ({ id })));
    else if (f.threads > 0) all.push({ path: f.path });
  }
  return { all, top };
}

// Where next or previous comment lands, wrapping round at the ends. From the focused thread it is the one
// after or before it. With nothing focused it counts from the top of the selected file: next is that file's first
// thread, previous is the last one before the file. null when the review has no thread.
export function jumpStop(m: Model, forward: boolean): Stop | null {
  const { all, top } = stops(m);
  if (all.length === 0) return null;
  const hit = findComment(m.files, m.state, m.focus);
  const at = hit === null ? -1 : all.findIndex((s) => "id" in s && s.id === hit.thread.id);
  const from = at >= 0 ? (forward ? at + 1 : at - 1) : forward ? top : top - 1;
  return all[(from + all.length) % all.length] ?? null;
}

// What a key pressed on the panel means: n is the next comment and shift+n the previous, / goes to the filter
// box and Esc clears the selection, else the filter. Not while the editor holds the keyboard, so a note can say "n", and not while
// a question or the picker is up.
export function panelKey(m: Model, keystroke: string): Msg | null {
  if (m.editor !== null || m.picker !== null || m.confirm !== null || m.confirmArchive) return null;
  // In the box a key types: only Esc is a command (Enter reaches the model as the box's own submit).
  if (m.filtering) return keystroke === "escape" ? { _tag: "FilterCleared" } : null;
  if (keystroke === "/") return { _tag: "FilterOpened" };
  if (keystroke === "escape") return m.selection !== null ? { _tag: "SelectionCleared" } : m.filter === "" ? null : { _tag: "FilterCleared" };
  if (keystroke === "n") return { _tag: "ThreadJumped", forward: true };
  if (keystroke === "shift-n" || keystroke === "N") return { _tag: "ThreadJumped", forward: false };
  return null;
}

// `now` under a minute, then 2m, 3h, 2d. A time that does not parse has no age.
export function ageText(at: string, now: number): string | null {
  const t = Date.parse(at);
  if (Number.isNaN(t)) return null;
  const s = Math.max(0, Math.floor((now - t) / 1000));
  return s < 60 ? "now" : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`;
}

const who = (author: string, you: string): string => (author === "user" ? you : author.startsWith("agent:") ? cleanText(author.slice(6)) : "agent");

function cardItems(out: StreamItem[], path: string, thread: Thread, m: Model, now: number): void {
  const root = thread.comments[0];
  if (root === undefined) throw new Error(`thread ${thread.id} has no comments`);
  const resolved = thread.status === "resolved";
  const open = !resolved || m.expanded.has(thread.id);
  const tone: Tone = resolved ? "resolved" : root.author === "user" ? "user" : "agent";
  const age = ageText(root.at, now);
  const badges: string[] = [];
  if (!resolved && thread.placement.kind === "outdated") badges.push("outdated");
  if (resolved) badges.push(`resolved by ${thread.resolved_by === null ? "?" : who(thread.resolved_by, "you")}`);
  if (thread.is_new) badges.push("new");
  if (thread.comments.some((c) => c.edited_since_sent)) badges.push("edited since sent");
  if (thread.unsent) badges.push("unsent");
  const outdated = !resolved && thread.placement.kind !== "matched" && thread.anchor.text !== null;
  out.push({
    kind: "head",
    key: itemKey(path, `h:${thread.id}`),
    id: thread.id,
    tone,
    title: `${resolved ? "\u2713" : "\u25cf"} ${who(root.author, "Your note")}${age === null ? "" : ` \u00b7 ${age}`} \u00b7 ${anchorLabel(thread.anchor)}`,
    badges,
    quote: outdated ? cleanText(thread.anchor.text ?? "") : null,
    collapsible: resolved,
    open,
    focused: m.focus === thread.id,
    last: !open,
  });
  if (!open) return;
  thread.comments.forEach((c, i) => {
    const when = ageText(c.at, now);
    out.push({
      kind: "body",
      key: itemKey(path, `c:${c.id}`),
      id: c.id,
      tone,
      meta: c.parent === null ? null : `${who(c.author, "You")}${when === null ? "" : ` \u00b7 ${when}`}`,
      lines: bodyLines(c.body),
      focused: m.focus === c.id,
      last: i === thread.comments.length - 1,
    });
  });
}

// An item's key is its file's path and what it is in the file, so a click names the file it belongs to. The path of
// the threads not in the diff is empty. Nothing after the last `|` holds one, so the path may.
export const itemKey = (path: string, local: string): string => `${path}|${local}`;
export const keyPath = (key: string): string => key.slice(0, key.lastIndexOf("|"));
const keyLocal = (key: string): string => key.slice(key.lastIndexOf("|") + 1);

// Sizes, in pixels, that the stream places its items by: the prefix sums, the scrollbar and a scroll to an item all use
// them, and nothing is measured. A code row is a line of fixed height. A card is as tall as its text wraps, which only the
// renderer knows, so it is estimated from the text's length and the width, a little over so that text never reaches the
// next item. The view draws every item in a box of exactly its size, so what is drawn and what is counted agree.
export const ROW = 20;
export const CARD_W = 560;
const FILE_H = 30;
const LABEL_H = 30;
const NOTE_H = 24;
const CHAR_W = 7.5;
const TEXT_H = 18;

// The card's width in a stream this wide: the view gives it 48 px on the left and 12 on the right, up to CARD_W.
export const cardWidth = (width: number): number => Math.max(160, Math.min(CARD_W, width - 60));

export function heightOf(i: StreamItem, width: number): number {
  switch (i.kind) {
    case "hunk":
    case "ctx":
    case "add":
    case "del":
      return ROW;
    case "label":
      return LABEL_H;
    case "file":
      return FILE_H;
    case "note":
      return NOTE_H;
    case "wait":
      return i.height;
    case "head":
      return 34 + (i.quote === null ? 0 : 18);
    case "body": {
      const cols = Math.max(8, Math.floor((cardWidth(width) - (i.meta === null ? 22 : 34)) / CHAR_W));
      const wrapped = i.lines.reduce((n, line) => n + Math.max(1, Math.ceil(line.length / cols)), 0);
      return 6 + (i.meta === null ? 0 : 16) + wrapped * TEXT_H + (i.last ? 6 : 0);
    }
    default: {
      const unhandled: never = i;
      throw new Error(`unhandled item ${JSON.stringify(unhandled)}`);
    }
  }
}

// How many lines a file's diff is guessed to have before it arrives: its changed lines and a hunk's header and
// context. A file with no text to show (binary, a submodule) is one line.
export function estimateRows(f: FileSummary): number {
  if (f.change === "binary" || f.change === "submodule" || f.change === "too_large" || f.change === "unparsed") return 1;
  return f.add + f.del + (f.change === "added" || f.change === "untracked" || f.change === "deleted" ? 2 : 8);
}

// `sizes` holds one height for each item and `starts` the pixel each one begins at, a prefix sum one longer than `items`
// (`starts[items.length]` is `total`, the stream's height). `wide` is the width the longest code line needs.
export type Stream = { items: StreamItem[]; sizes: number[]; starts: number[]; total: number; wide: number };
type FileItems = { items: StreamItem[]; sizes: number[] };

// The rows and cards of a loaded file, with the height of each. It is the costly part of a frame (a row is cut into
// runs), and a scroll, a hover or a drag leaves it as it was, so it is kept per file reply and rebuilt only when
// what it shows changes: an opened thread, the focus, the minute (the ages) or the width. The map is
// keyed by the reply's own object, so no copy of the mod sees another's and a reply that is gone takes its entry.
const memo = new WeakMap<FileView, { dep: string; stream: FileItems }>();

// A file's code rows: the text cut into runs, the costly part. Kept for as long as the reply object lives, so a new
// minute or a width does not cut every row of every file again inside one frame.
const rowMemo = new WeakMap<FileView, RowItem[]>();

function rowsOf(file: FileView): RowItem[] {
  const kept = rowMemo.get(file);
  if (kept !== undefined) return kept;
  const rows = file.rows.map((row, index): RowItem => ({ kind: row.kind, key: itemKey(file.path, `r${index}`), index, old: row.old === null ? "" : String(row.old), new: row.new === null ? "" : String(row.new), text: cleanText(row.text), runs: runs(row.text, row.tokens) }));
  rowMemo.set(file, rows);
  return rows;
}

function fileStream(m: Model, file: FileView, now: number): FileItems {
  const ids = new Set(file.threads.flatMap((t) => [t.id, ...t.comments.map((c) => c.id)]));
  const dep = [file.threads.filter((t) => m.expanded.has(t.id)).map((t) => t.id).join(","), m.focus !== null && ids.has(m.focus) ? m.focus : "", Math.floor(now / 60000), m.width].join("|");
  const kept = memo.get(file);
  if (kept !== undefined && kept.dep === dep) return kept.stream;
  const items: StreamItem[] = [];
  const under = new Map<number, Thread[]>();
  const top: Thread[] = [];
  for (const t of file.threads) {
    const row = t.placement.row;
    if (row === null) top.push(t);
    else if (file.rows[row] === undefined) throw new Error(`thread ${t.id} is placed at row ${row}, past the end`);
    else under.set(row, [...(under.get(row) ?? []), t]);
  }
  if (file.note !== null) items.push({ kind: "note", key: itemKey(file.path, "note"), text: cleanText(file.note) });
  for (const t of top) cardItems(items, file.path, t, m, now);
  const base = rowsOf(file);
  file.rows.forEach((_row, index) => {
    items.push(base[index] as RowItem);
    for (const t of under.get(index) ?? []) cardItems(items, file.path, t, m, now);
  });
  const stream = { items, sizes: items.map((i) => heightOf(i, m.width)) };
  memo.set(file, { dep, stream });
  return stream;
}

function fileHead(m: Model, f: FileSummary, folded: boolean): FileItem {
  const hides = [f.viewed ? "viewed" : "", folded && f.threads > 0 ? plural(f.threads, "thread", "threads") : ""].filter((x) => x !== "").join(", ");
  return { kind: "file", key: itemKey(f.path, "file"), path: f.path, title: cleanText(f.path), letter: LETTERS[f.change], add: f.add, del: f.del, viewed: f.viewed, folded, hides };
}

// Every file's diff in one stream, in the diff's order: first the threads not in the diff, then for each file a header
// row and under it its rows and cards. A folded file is its header. A file that has not arrived is its header and a
// stretch as tall as its diff is guessed to be.
export function streamLayout(m: Model, now: number): Stream {
  const items: StreamItem[] = [];
  const sizes: number[] = [];
  const add = (more: StreamItem[], heights = more.map((i) => heightOf(i, m.width))): void => {
    items.push(...more);
    sizes.push(...heights);
  };
  const orphans = m.state === null ? [] : m.state.orphans;
  if (orphans.length > 0) {
    const out: StreamItem[] = [{ kind: "label", key: itemKey("", "l:orphans"), text: "Comments not in this diff" }];
    for (const t of orphans) cardItems(out, "", t, m, now);
    add(out);
  }
  for (const f of m.state === null ? [] : m.state.files) {
    const folded = isFolded(m, f);
    add([fileHead(m, f, folded)]);
    if (folded) continue;
    const view = m.files.get(f.path);
    if (view !== undefined) {
      const s = fileStream(m, view, now);
      add(s.items, s.sizes);
    } else {
      const rows = estimateRows(f);
      add([{ kind: "wait", key: itemKey(f.path, "wait"), text: m.skipped.has(f.path) ? "Could not load this file. Refresh to try again" : "Loading...", height: rows * ROW }]);
    }
  }
  const starts = [0];
  for (const size of sizes) starts.push((starts[starts.length - 1] as number) + size);
  return { items, sizes, starts, total: starts[starts.length - 1] as number, wide: diffWidth(items) };
}

// streamLayout, kept while nothing it reads changes. A wheel tick, a hover, a drag or a selection changes none of these,
// so none of them cuts the items and the prefix sums again: only the files, the folds, the open threads, the focus, the
// width, the files that failed, and the minute (the ages on the cards) do.
export function layoutCache(): (m: Model, now: number) => Stream {
  let deps: unknown[] = [];
  let kept: Stream | null = null;
  return (m, now) => {
    const next = [m.state, m.files, m.folds, m.expanded, m.focus, m.width, m.skipped, Math.floor(now / 60000)];
    if (kept !== null && next.every((d, i) => d === deps[i])) return kept;
    deps = next;
    kept = streamLayout(m, now);
    return kept;
  };
}

// --- scrolling: the mod keeps the offset itself, in pixels, and draws only the items that overlap the screen ---

// The most the stream can be scrolled: its end at the bottom of the screen. A stream shorter than the screen does not scroll.
export const maxOffset = (total: number, height: number): number => Math.max(0, total - height);
export const clampOffset = (offset: number, total: number, height: number): number => Math.min(maxOffset(total, height), Math.max(0, offset));

// The item that holds pixel `y`: the last one that begins at or above it. 0 for an empty stream. This is the hit test of
// a pixel against the items.
export function itemAt(starts: number[], y: number): number {
  let lo = 0;
  let hi = Math.max(0, starts.length - 2);
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if ((starts[mid] as number) <= y) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

// The items that overlap the screen: `first` to `end` (exclusive), and how far the first one is above the top, which the
// view shifts it up by. The offset is clamped, so a view drawn before the model caught up with a resize still draws.
export function visibleRange(stream: Stream, offset: number, height: number): { first: number; end: number; shift: number } {
  const n = stream.items.length;
  if (n === 0) return { first: 0, end: 0, shift: 0 };
  const at = clampOffset(offset, stream.total, height);
  const first = itemAt(stream.starts, at);
  let end = first + 1;
  while (end < n && (stream.starts[end] as number) < at + height) end++;
  return { first, end, shift: at - (stream.starts[first] as number) };
}

// The pixel an item begins at, as an offset the stream can be scrolled to (the end of the stream stops it short).
export const offsetOfKey = (stream: Stream, key: string, height: number): number | null => {
  const at = indexOfKey(stream.items, key);
  return at < 0 ? null : clampOffset(stream.starts[at] as number, stream.total, height);
};

// What the model must learn from the stream after any change: the offset (moved to a pending reveal, then clamped), the
// stream's height, the key of the first item on screen and the files on screen. null when the model knows it all. This is
// the one place `Scrolled` comes from, so the sidebar and the fetches follow every kind of scroll the same way.
export function scrolled(m: Model, stream: Stream): Msg | null {
  const target = m.reveal === null ? null : offsetOfKey(stream, m.reveal.key, m.height);
  const offset = clampOffset(target ?? m.offset, stream.total, m.height);
  const { first, end } = visibleRange(stream, offset, m.height);
  const top = stream.items[first]?.key ?? null;
  const visible = pathsOf(stream.items, first, end);
  const same = visible.length === m.visible.length && visible.every((p, i) => p === m.visible[i]);
  if (target === null && offset === m.offset && stream.total === m.total && top === m.top && same) return null;
  return { _tag: "Scrolled", offset, total: stream.total, top, visible, revealed: target !== null };
}

// A wheel or trackpad event as the pixels to scroll down by, or null when it is mostly sideways, which the code box's own
// sideways scroll takes. A device that reports lines is taken at a row a line.
export function wheelDelta(e: { delta: { x: number; y: number }; delta_lines?: { x: number; y: number } }): number | null {
  if (Math.abs(e.delta.x) > Math.abs(e.delta.y)) return null;
  const dy = e.delta_lines !== undefined ? e.delta_lines.y * ROW : e.delta.y;
  return dy === 0 ? 0 : -dy;
}

// The scrollbar's thumb: its top and height in the track, or null when the stream fits and there is nothing to scroll.
export const THUMB_MIN = 24;
export function thumbOf(total: number, height: number, offset: number): { top: number; size: number } | null {
  if (total <= height || height <= 0) return null;
  const size = Math.max(THUMB_MIN, (height * height) / total);
  const room = Math.max(1, height - size);
  return { top: (clampOffset(offset, total, height) / maxOffset(total, height)) * room, size };
}

// The offset a thumb's top at `top` stands for.
export function offsetOfThumb(total: number, height: number, top: number): number {
  const thumb = thumbOf(total, height, 0);
  if (thumb === null) return 0;
  return clampOffset((top / Math.max(1, height - thumb.size)) * maxOffset(total, height), total, height);
}

// The distinct files of the items from `start` to `end`, topmost first. The threads not in the diff have none.
export function pathsOf(items: StreamItem[], start: number, end: number): string[] {
  const out: string[] = [];
  for (const i of items.slice(Math.max(0, start), Math.max(0, end))) {
    const path = keyPath(i.key);
    if (path !== "" && !out.includes(path)) out.push(path);
  }
  return out;
}

// How many files past the last one on screen are fetched at once with it, and how many a fetch holds. A fetch
// is run one file at a time, so a file that scrolls into view waits for the rest of the fetch, not for all the files.
export const AHEAD = 3;
export const BATCH = 3;

// The files to fetch, in the order to fetch them: the ones on screen (or the one a scroll is going to) and the next
// few, then every other file below them and then the ones above. A file is wanted when it has no diff yet or its diff
// is stale, and not when it is folded (nothing draws it) or its last fetch failed.
export function wanted(m: Model): string[] {
  const files = m.state === null ? [] : m.state.files;
  const on = m.reveal !== null ? [keyPath(m.reveal.key)] : m.visible.length > 0 ? m.visible : m.selected === null ? [] : [m.selected];
  const idx = on.map((p) => files.findIndex((f) => f.path === p)).filter((i) => i >= 0);
  const lo = idx.length === 0 ? 0 : Math.min(...idx);
  const hi = idx.length === 0 ? 0 : Math.max(...idx);
  const order: number[] = [];
  const range = (from: number, to: number): void => {
    for (let i = from; i <= to; i++) order.push(i);
  };
  range(lo, Math.min(files.length - 1, hi + AHEAD));
  range(hi + AHEAD + 1, files.length - 1);
  range(0, lo - 1);
  return order.flatMap((i) => {
    const f = files[i];
    return f !== undefined && !isFolded(m, f) && !m.skipped.has(f.path) && (!m.files.has(f.path) || m.dirty.has(f.path)) ? [f.path] : [];
  });
}

// The key a scroll to a thread's card aims at, or null when the thread is in no file that is loaded.
export function cardKey(m: Model, id: string): string | null {
  if ((m.state === null ? [] : m.state.orphans).some((t) => t.id === id)) return itemKey("", `h:${id}`);
  for (const [path, file] of m.files) if (file.threads.some((t) => t.id === id)) return itemKey(path, `h:${id}`);
  return null;
}

// The index in the stream of the item with this key, which a scroll to it names.
export const indexOfKey = (items: StreamItem[], key: string): number => items.findIndex((i) => i.key === key);

// A code row has a gutter GUTTER_W pixels wide at its left edge, where the [+] is drawn on the hovered line. The [+] is
// icons/plus.svg, a filled rounded square with the + cut out, drawn in this theme colour.
export const GUTTER_W = 24;
export const PLUS = "accent";

// How a code row is drawn: tinted when it is in the selection (the rows of its file between `from` and `to` that have a line
// on the selection's side), and with a [+] when it is the row under the pointer. A hunk header has no line, so no [+].
export function rowState(m: Model, item: RowItem): { selected: boolean; hovered: boolean } {
  const path = keyPath(item.key);
  const sel = m.selection;
  const onSide = sel === null ? false : (sel.side === "old" ? item.old : item.new) !== "";
  return {
    selected: sel !== null && sel.path === path && item.index >= sel.from && item.index <= sel.to && onSide,
    hovered: item.kind !== "hunk" && m.hover !== null && m.hover.path === path && m.hover.row === item.index,
  };
}

// The message a click on an item of the stream stands for. A code row is not here: its press, hover and [+] have messages of
// their own. A header folds, a card toggles or focuses its thread, and the rest take no click.
export function itemClick(key: string): Msg | null {
  const path = keyPath(key);
  const local = keyLocal(key);
  if (local === "file") return { _tag: "FoldToggled", path };
  if (local.startsWith("h:")) return { _tag: "ThreadAction", id: local.slice(2), action: "toggle" };
  if (local.startsWith("c:")) return { _tag: "ThreadAction", id: local.slice(2), action: "focus" };
  return null;
}

// What a key pressed in the editor means: Ctrl+S or Cmd+S saves the text, Esc cancels.
export function editorKey(keystroke: string, text: string): Msg | null {
  if (keystroke === "ctrl-s" || keystroke === "cmd-s") return { _tag: "EditorSaved", text };
  if (keystroke === "escape") return { _tag: "EditorCancelled" };
  return null;
}

export function editorTitle(e: Editor): string {
  if (e.kind === "reply") return `Reply to ${e.thread ?? ""}`;
  if (e.kind === "edit") return `Edit ${e.comment ?? ""}`;
  return `Draft note - ${e.anchor === null ? "" : anchorLabel(e.anchor)}`;
}

// danger is a red button that destroys something; accent is the main, harmless one of a question.
export type DockButton = { id: string; label: string; msg: Msg; danger: boolean; off: boolean; accent?: boolean };

// Threads with a user comment that no send carried: the ones in the files, and the ones not in the diff.
export const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

// What archive would move now, as the last state said. Nothing known is nothing to archive.
export const archivableCount = (state: State | null): number => (state === null ? 0 : state.archivable);

export function unsentCount(state: State | null): number {
  if (state === null) return 0;
  return state.files.reduce((n, f) => n + f.unsent, 0) + state.orphans.filter((t) => t.unsent).length;
}

// The agents a send found for this folder, one button each, and a way out.
export function pickerOptions(candidates: Candidate[]): DockButton[] {
  const option = (c: Candidate): DockButton => ({
    id: `pick-${c.address}`,
    label: cleanText(`${c.address}  ${c.agent}  ${c.status}  ${c.cwd}`),
    msg: { _tag: "PickerChosen", target: c.address },
    danger: false,
    off: false,
  });
  return [...candidates.map(option), { id: "pick-cancel", label: "Cancel", msg: { _tag: "PickerCancelled" }, danger: false, off: false }];
}

// color is the theme name of the sentence: red for a delete, plain text for an archive, which moves threads to the
// archive file and loses none.
export type ConfirmRow = { sentence: string; color: string; buttons: DockButton[] };

// The question a click on Delete or Archive asked, with the button that does it and the way out. It replaces
// the bar under the diff until answered. A root's replies go with it, so only then does the sentence say so.
export function confirmRow(m: Model): ConfirmRow | null {
  const cancel: DockButton = { id: "confirm-cancel", label: "Cancel", msg: { _tag: "ConfirmCancelled" }, danger: false, off: false };
  if (m.confirm !== null) {
    const hit = findComment(m.files, m.state, m.confirm);
    if (hit === null) return null;
    const replies = hit.comment.parent === null && hit.thread.comments.length > 1;
    const ok: DockButton = { id: "confirm-delete", label: "Delete", msg: { _tag: "DeleteConfirmed", id: m.confirm }, danger: true, off: false };
    return { sentence: replies ? "Delete this comment? Its replies go too." : "Delete this comment?", color: "ansi1", buttons: [ok, cancel] };
  }
  if (!m.confirmArchive) return null;
  const ok: DockButton = { id: "confirm-archive", label: "Archive", msg: { _tag: "ArchiveConfirmed" }, danger: false, off: m.inFlight, accent: true };
  return { sentence: `Archive ${plural(archivableCount(m.state), "resolved thread", "resolved threads")}?`, color: "foreground", buttons: [ok, cancel] };
}

// What a key pressed while the question is up means: Esc cancels.
export function confirmKey(keystroke: string): Msg | null {
  return keystroke === "escape" ? { _tag: "ConfirmCancelled" } : null;
}

// The bar under the diff when no editor is open: what is selected or focused, and what can be done with it.
export function dock(m: Model): { label: string; buttons: DockButton[]; actions: DockButton[] } {
  const buttons: DockButton[] = [];
  const add = (id: string, label: string, msg: Msg, danger = false): void => void buttons.push({ id, label, msg, danger, off: false });
  const hit = findComment(m.files, m.state, m.focus);
  const picked = m.selection === null ? null : (m.files.get(m.selection.path) ?? null);
  let label = "Drag over lines to select them, [+] to comment";
  if (picked !== null && m.selection !== null) {
    label = `${selectionLabel(picked, m.selection)}. [+] to comment, Esc to clear`;
  } else if (hit !== null) {
    const { thread, comment } = hit;
    label = `${thread.id}${comment.id === thread.id ? "" : ` / ${comment.id}`}`;
    add("reply", "Reply", { _tag: "ThreadAction", id: comment.id, action: "reply" });
    if (thread.status === "open") add("resolve", "Resolve", { _tag: "ThreadAction", id: comment.id, action: "resolve" });
    else add("reopen", "Reopen", { _tag: "ThreadAction", id: comment.id, action: "reopen" });
    if (thread.status === "open" && !thread.unsent) add("resend", "Resend", { _tag: "SendClicked", threads: [thread.id] });
    if (comment.author === "user") {
      add("edit", "Edit", { _tag: "ThreadAction", id: comment.id, action: "edit" });
      add("delete", "Delete", { _tag: "ThreadAction", id: comment.id, action: "delete" }, true);
    }
  }
  const shown = m.selected === null ? undefined : summaryOf(m, m.selected);
  if (shown !== undefined) add("file", "Comment on file", { _tag: "FileCommentClicked" });
  // What acts on the whole review, or on the file shown.
  const unsent = unsentCount(m.state);
  const none = stops(m).all.length === 0;
  const archivable = archivableCount(m.state);
  const actions: DockButton[] = [
    { id: "prev-thread", label: "Previous comment", msg: { _tag: "ThreadJumped", forward: false }, danger: false, off: none },
    { id: "next-thread", label: "Next comment", msg: { _tag: "ThreadJumped", forward: true }, danger: false, off: none },
    { id: "send", label: unsent > 0 ? `Send ${unsent}` : "Send", msg: { _tag: "SendClicked" }, danger: false, off: m.sending },
    { id: "archive", label: archivable > 0 ? `Archive ${archivable}` : "Archive", msg: { _tag: "ArchiveClicked" }, danger: false, off: m.inFlight || archivable === 0 },
  ];
  if (shown !== undefined) actions.push({ id: "viewed", label: shown.viewed ? "Mark unviewed" : "Mark viewed", msg: { _tag: "ViewedClicked", path: shown.path, viewed: !shown.viewed }, danger: false, off: false });
  return { label, buttons, actions };
}

// A width that fits the longest line, for the horizontal scroll of the diff.
export function diffWidth(items: StreamItem[]): number {
  let wide = 300;
  for (const i of items) if (i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del") wide = Math.max(wide, i.text.length * 7 + 150);
  return wide;
}

const LETTERS: Record<Change, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "?",
  binary: "B",
  submodule: "S",
  too_large: "L",
  unparsed: "U",
};

export type Badge = { text: string; tone: "unsent" | "open" | "new" | "viewed" };
export const badgeColor = (b: Badge): string => (b.tone === "unsent" ? WARN : b.tone === "open" ? "accent" : b.tone === "new" ? "done" : "dim");

// One row of the sidebar: a folder heading, or a file under it. A heading takes no click.
export type SideRow =
  | { kind: "dir"; key: string; text: string }
  | { kind: "file"; key: string; path: string; name: string; icon: string; letter: string; add: number; del: number; viewed: boolean; badges: Badge[] };

function fileRow(f: FileSummary): SideRow {
  const badges: Badge[] = [];
  if (f.unsent > 0) badges.push({ text: `${f.unsent} unsent`, tone: "unsent" });
  if (f.open > 0) badges.push({ text: `${f.open} open`, tone: "open" });
  if (f.new > 0) badges.push({ text: `${f.new} new`, tone: "new" });
  if (f.viewed) badges.push({ text: "viewed", tone: "viewed" });
  const name = f.path.slice(f.path.lastIndexOf("/") + 1);
  return { kind: "file", key: `f:${f.path}`, path: f.path, name: cleanText(name), icon: fileIcon(name), letter: LETTERS[f.change], add: f.add, del: f.del, viewed: f.viewed, badges };
}

// The query's characters appear in the path in order, as the TUI's `matches` in view.rs says it: a query with no
// upper case letter ignores case, one upper case letter makes it exact, a space is a character, and an empty
// query matches everything. There is no score. Walks code points, as Rust walks chars.
export function fileMatches(query: string, path: string): boolean {
  const fold = !/\p{Uppercase}/u.test(query);
  const chars = [...path].map((c) => (fold ? ([...c.toLowerCase()][0] ?? c) : c));
  let at = 0;
  for (const wanted of query) {
    const found = chars.indexOf(wanted, at);
    if (found < 0) return false;
    at = found + 1;
  }
  return true;
}

// How many of the diff's files the query matches, and how many there are: the `2/4` in the box.
export function filterCount(files: FileSummary[], query: string): { shown: number; total: number } {
  return { shown: files.filter((f) => fileMatches(query, f.path)).length, total: files.length };
}

// Where Enter in the box sends the file shown: the first match, unless the file shown matches already or
// nothing does.
export function filterPick(m: Model): string | null {
  const hits = (m.state === null ? [] : m.state.files).filter((f) => fileMatches(m.filter, f.path));
  return hits.length === 0 || hits.some((f) => f.path === m.selected) ? null : (hits[0]?.path ?? null);
}

// The files grouped by folder, in the diff's order as the TUI does: a heading (`src/`, or `./` for the top
// folder) where the folder changes, then the files. A folder that comes again later, as untracked files do,
// gets a second heading. A file the query does not match is left out, and so is the heading of a folder with no
// match. Cutting a name that is too long is left to the drawing.
export function sidebarRows(files: FileSummary[], query = ""): SideRow[] {
  const out: SideRow[] = [];
  let last: string | null = null;
  for (const f of files) {
    if (!fileMatches(query, f.path)) continue;
    const at = f.path.lastIndexOf("/");
    const dir = at < 0 ? "./" : f.path.slice(0, at + 1);
    if (dir !== last) {
      out.push({ kind: "dir", key: `d:${out.length}`, text: cleanText(dir) });
      last = dir;
    }
    out.push(fileRow(f));
  }
  return out;
}

export type HeaderButton = { id: string; label: string; msg: Msg };

// The two buttons for the sidebar. Moving it is offered only while it shows, since nothing would change on screen.
export function sidebarButtons(m: Model): HeaderButton[] {
  const buttons: HeaderButton[] = [{ id: "sidebar-toggle", label: m.panel.open ? "Hide files" : "Show files", msg: { _tag: "SidebarToggled" } }];
  if (m.panel.open) buttons.push({ id: "sidebar-flip", label: m.panel.side === "left" ? "Move right" : "Move left", msg: { _tag: "SidebarFlipped" } });
  return buttons;
}

// What sits under the header, left to right: the sidebar on its side, or left out when hidden, and the diff.
export function paneOrder(m: Model): ("sidebar" | "diff")[] {
  if (!m.panel.open) return ["diff"];
  return m.panel.side === "left" ? ["sidebar", "diff"] : ["diff", "sidebar"];
}

// What the sidebar's box shows besides the text: the `n/m` count, and whether the query leaves no file.
export function filterInfo(m: Model): { count: string; none: boolean } {
  const { shown, total } = filterCount(m.state === null ? [] : m.state.files, m.filter);
  return { count: `${shown}/${total}`, none: total > 0 && shown === 0 };
}

export function totals(state: State | null): { files: number; add: number; del: number } {
  const files = state === null ? [] : state.files;
  return { files: files.length, add: files.reduce((n, f) => n + f.add, 0), del: files.reduce((n, f) => n + f.del, 0) };
}
