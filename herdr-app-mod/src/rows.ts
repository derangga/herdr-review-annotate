// Pure helpers for drawing: the diff rows in order, the text cleaned for the screen, the file list lines.
import { fileIcon } from "./icons.ts";
import type { Candidate, Change, Comment, DiffWindow, Editor, FileSummary, FileView, Model, Msg, NewAnchor, Row, Selection, State, Thread, Token, TokenKind } from "./types.ts";

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
export type RowItem = { kind: "hunk" | "ctx" | "add" | "del"; key: string; index: number; old: string; new: string; text: string; runs: Run[]; selected: boolean };
// A thread card is a head, then one body per comment while it is open. The last item of a card closes the box.
export type HeadItem = { kind: "head"; key: string; id: string; tone: Tone; title: string; badges: string[]; quote: string | null; collapsible: boolean; open: boolean; focused: boolean; last: boolean };
export type BodyItem = { kind: "body"; key: string; id: string; tone: Tone; meta: string | null; lines: string[]; focused: boolean; last: boolean };
export type LabelItem = { kind: "label"; key: string; text: string };
// The first item of a list that starts below the top of the file: a click shows the whole file again.
export type MoreItem = { kind: "more"; key: string };
export type LayoutItem = RowItem | HeadItem | BodyItem | LabelItem | MoreItem;

const sideOf = (row: Row): "old" | "new" => (row.kind === "del" ? "old" : "new");
const lineOn = (row: Row, side: "old" | "new"): number | null => (side === "old" ? row.old : row.new);

// A click on a row: a one-row selection on the side the row lives on, and nothing for a hunk header.
export function pick(file: FileView, row: number): Selection | null {
  const r = file.rows[row];
  if (r === undefined) throw new Error(`row ${row} is out of range`);
  return r.kind === "hunk" ? null : { path: file.path, side: sideOf(r), from: row, to: row };
}

// The range between the row first clicked and a second one. It takes the first row's side, and its ends are
// the first and last row in between that have a line on that side.
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
  return from === undefined || to === undefined ? null : { path: file.path, side, from, to };
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

export function findComment(file: FileView | null, state: State | null, id: string | null): { thread: Thread; comment: Comment } | null {
  if (id === null) return null;
  for (const thread of [...(file === null ? [] : file.threads), ...(state === null ? [] : state.orphans)]) {
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

// A place a jump can land: a thread, or a file whose threads are not loaded yet.
export type Stop = { id: string } | { path: string };

// Every thread in the order the TUI walks them: the ones not in the diff, then the files in the sidebar's order
// with each file's threads in card order. Only the shown file has its threads loaded, so any other file that
// holds a thread is one stop. `top` is where the shown file's stops begin.
export function stops(m: Model): { all: Stop[]; top: number } {
  const all: Stop[] = (m.state === null ? [] : m.state.orphans).map((t) => ({ id: t.id }));
  let top = 0;
  for (const f of m.state === null ? [] : m.state.files) {
    if (m.file !== null && m.file.path === f.path) {
      top = all.length;
      all.push(...cardOrder(m.file).map((id) => ({ id })));
    } else if (f.threads > 0) all.push({ path: f.path });
  }
  return { all, top };
}

// Where next or previous comment lands, wrapping round at the ends. From the focused thread it is the one
// after or before it. With nothing focused it counts from the top of the shown file: next is that file's first
// thread, previous is the last one before the file. null when the review has no thread.
export function jumpStop(m: Model, forward: boolean): Stop | null {
  const { all, top } = stops(m);
  if (all.length === 0) return null;
  const hit = findComment(m.file, m.state, m.focus);
  const at = hit === null ? -1 : all.findIndex((s) => "id" in s && s.id === hit.thread.id);
  const from = at >= 0 ? (forward ? at + 1 : at - 1) : forward ? top : top - 1;
  return all[(from + all.length) % all.length] ?? null;
}

// What a key pressed on the panel means: n is the next comment and shift+n the previous, / goes to the filter
// box and Esc clears the filter. Not while the editor holds the keyboard, so a note can say "n", and not while
// a question or the picker is up.
export function panelKey(m: Model, keystroke: string): Msg | null {
  if (m.editor !== null || m.picker !== null || m.confirm !== null || m.confirmArchive) return null;
  // In the box a key types: only Esc is a command (Enter reaches the model as the box's own submit).
  if (m.filtering) return keystroke === "escape" ? { _tag: "FilterCleared" } : null;
  if (keystroke === "/") return { _tag: "FilterOpened" };
  if (keystroke === "escape") return m.filter === "" ? null : { _tag: "FilterCleared" };
  if (keystroke === "n") return { _tag: "ThreadJumped", forward: true };
  if (keystroke === "shift-n" || keystroke === "N") return { _tag: "ThreadJumped", forward: false };
  return null;
}

// The key of the item the diff list starts at to show a thread: two lines above the line its card hangs from,
// so the line has its context, or the card itself when it hangs from no line.
export function windowStart(file: FileView | null, id: string): string {
  const row = file === null ? undefined : file.threads.find((t) => t.id === id)?.placement.row;
  return row === undefined || row === null ? `h:${id}` : `r${Math.max(0, row - 2)}`;
}

// `now` under a minute, then 2m, 3h, 2d. A time that does not parse has no age.
export function ageText(at: string, now: number): string | null {
  const t = Date.parse(at);
  if (Number.isNaN(t)) return null;
  const s = Math.max(0, Math.floor((now - t) / 1000));
  return s < 60 ? "now" : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`;
}

const who = (author: string, you: string): string => (author === "user" ? you : author.startsWith("agent:") ? cleanText(author.slice(6)) : "agent");

function cardItems(out: LayoutItem[], thread: Thread, m: Model, now: number): void {
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
    key: `h:${thread.id}`,
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
      key: `c:${c.id}`,
      id: c.id,
      tone,
      meta: c.parent === null ? null : `${who(c.author, "You")}${when === null ? "" : ` \u00b7 ${when}`}`,
      lines: bodyLines(c.body),
      focused: m.focus === c.id,
      last: i === thread.comments.length - 1,
    });
  });
}

// The diff rows in order, with the thread cards under the rows they are placed at. Threads that are not in
// the diff come first, under a label, and file comments and threads with no row come before the rows.
export function layout(m: Model, now: number): LayoutItem[] {
  const out: LayoutItem[] = [];
  const orphans = m.state === null ? [] : m.state.orphans;
  if (orphans.length > 0) {
    out.push({ kind: "label", key: "l:orphans", text: "Comments not in this diff" });
    for (const t of orphans) cardItems(out, t, m, now);
  }
  const file = m.file;
  if (file === null) return out;
  const under = new Map<number, Thread[]>();
  const top: Thread[] = [];
  for (const t of file.threads) {
    const row = t.placement.row;
    if (row === null) top.push(t);
    else if (file.rows[row] === undefined) throw new Error(`thread ${t.id} is placed at row ${row}, past the end`);
    else under.set(row, [...(under.get(row) ?? []), t]);
  }
  for (const t of top) cardItems(out, t, m, now);
  const sel = m.selection !== null && m.selection.path === file.path ? m.selection : null;
  file.rows.forEach((row, index) => {
    const selected = sel !== null && index >= sel.from && index <= sel.to && lineOn(row, sel.side) !== null;
    out.push({ kind: row.kind, key: `r${index}`, index, old: row.old === null ? "" : String(row.old), new: row.new === null ? "" : String(row.new), text: cleanText(row.text), runs: runs(row.text, row.tokens), selected });
    for (const t of under.get(index) ?? []) cardItems(out, t, m, now);
  });
  return windowed(out, m.window);
}

// A window that starts below the top drops the items above it and says so in a first item. A start that is no
// longer there (the line went away in a reload) shows the whole file.
function windowed(items: LayoutItem[], w: DiffWindow): LayoutItem[] {
  const at = w.from === null ? -1 : items.findIndex((i) => i.key === w.from);
  return at <= 0 ? items : [{ kind: "more", key: "more" }, ...items.slice(at)];
}

// The id of the diff list, which the scrollbar beside it is named after.
export const listId = (w: DiffWindow): string => `review-diff-${w.id}`;

// The message a click on an item of the diff list stands for. The list reports only the key.
export function itemClick(key: string): Msg | null {
  const row = /^r(\d+)$/.exec(key);
  if (row !== null) return { _tag: "RowClicked", row: Number(row[1]) };
  if (key === "more") return { _tag: "EarlierClicked" };
  if (key.startsWith("h:")) return { _tag: "ThreadAction", id: key.slice(2), action: "toggle" };
  if (key.startsWith("c:")) return { _tag: "ThreadAction", id: key.slice(2), action: "focus" };
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

export type DockButton = { id: string; label: string; msg: Msg; danger: boolean; off: boolean };

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

export type ConfirmRow = { sentence: string; buttons: DockButton[] };

// The question a click on Delete or Archive asked, with the button that does it and the way out. It replaces
// the bar under the diff until answered. A root's replies go with it, so only then does the sentence say so.
export function confirmRow(m: Model): ConfirmRow | null {
  const cancel: DockButton = { id: "confirm-cancel", label: "Cancel", msg: { _tag: "ConfirmCancelled" }, danger: false, off: false };
  if (m.confirm !== null) {
    const hit = findComment(m.file, m.state, m.confirm);
    if (hit === null) return null;
    const replies = hit.comment.parent === null && hit.thread.comments.length > 1;
    const ok: DockButton = { id: "confirm-delete", label: "Delete", msg: { _tag: "DeleteConfirmed", id: m.confirm }, danger: true, off: false };
    return { sentence: replies ? "Delete this comment? Its replies go too." : "Delete this comment?", buttons: [ok, cancel] };
  }
  if (!m.confirmArchive) return null;
  const ok: DockButton = { id: "confirm-archive", label: "Archive", msg: { _tag: "ArchiveConfirmed" }, danger: true, off: m.inFlight };
  return { sentence: `Archive ${plural(archivableCount(m.state), "resolved thread", "resolved threads")}?`, buttons: [ok, cancel] };
}

// What a key pressed while the question is up means: Esc cancels.
export function confirmKey(keystroke: string): Msg | null {
  return keystroke === "escape" ? { _tag: "ConfirmCancelled" } : null;
}

// The bar under the diff when no editor is open: what is selected or focused, and what can be done with it.
export function dock(m: Model): { label: string; buttons: DockButton[]; actions: DockButton[] } {
  const buttons: DockButton[] = [];
  const add = (id: string, label: string, msg: Msg, danger = false): void => void buttons.push({ id, label, msg, danger, off: false });
  const hit = findComment(m.file, m.state, m.focus);
  let label = "Click a line to comment, then another line for a range";
  if (m.file !== null && m.selection !== null) {
    label = selectionLabel(m.file, m.selection);
    add("comment", "Comment", { _tag: "CommentClicked" });
    add("cancel", "Cancel", { _tag: "EditorCancelled" });
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
  if (m.file !== null) add("file", "Comment on file", { _tag: "FileCommentClicked" });
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
  if (m.file !== null) actions.push({ id: "viewed", label: m.file.viewed ? "Mark unviewed" : "Mark viewed", msg: { _tag: "ViewedClicked", path: m.file.path, viewed: !m.file.viewed }, danger: false, off: false });
  return { label, buttons, actions };
}

// A width that fits the longest line, for the horizontal scroll of the diff.
export function diffWidth(items: LayoutItem[]): number {
  return Math.max(300, ...items.map((i) => (i.kind === "hunk" || i.kind === "ctx" || i.kind === "add" || i.kind === "del" ? i.text.length * 7 + 120 : 0)));
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
