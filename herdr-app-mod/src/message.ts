// The agent-message view: comments on lines of the agent's last message (design/gui.md, "The agent-message view").
// Pure model helpers and the items the view draws. Nothing here knows the diff stream. The comments live in the model only, as
// the TUI keeps them, and `message send` turns them into the prompt: the mod holds no message logic.
import { plural } from "./rows.ts";
import type { Cmd, MessageComment, MessageView, Model, ReviewError } from "./types.ts";

type Step = [Model, Cmd[]];

const BUSY_EDITOR = "finish or cancel the comment first";
export const NO_AGENT = "no agent to read: send once to choose one, or open the panel from an agent's terminal";

export const initMessage = (): MessageView => ({ data: null, loading: false, sending: false, comments: [], next: 1, selection: null, drag: null, hover: null, status: null });

const inMessage = (m: Model, line: number): boolean => m.message.data !== null && Number.isInteger(line) && line >= 1 && line <= m.message.data.lines.length;
const set = (m: Model, change: Partial<MessageView>): Model => ({ ...m, message: { ...m.message, ...change } });
const told = (m: Model, text: string): Model => (m.view === "message" ? set(m, { status: text }) : { ...m, status: text });
export const rangeLabel = (from: number, to: number): string => (from === to ? `line ${from}` : `lines ${from}-${to}`);
const range = (a: number, b: number): { from: number; to: number } => ({ from: Math.min(a, b), to: Math.max(a, b) });

// What the header offers to switch to.
export const viewButton = (m: Model): { id: string; label: string; msg: { _tag: "ViewChosen"; view: "diff" | "message" } } =>
  m.view === "diff" ? { id: "view-message", label: "Agent message", msg: { _tag: "ViewChosen", view: "message" } } : { id: "view-diff", label: "Diff", msg: { _tag: "ViewChosen", view: "diff" } };

export function sendButton(m: Model): { label: string; off: boolean } {
  const n = m.message.comments.length;
  return { label: n === 0 ? "Send" : `Send ${n}`, off: n === 0 || m.message.sending || m.message.loading };
}

// Reading the message: the one place that asks the CLI. Without a target there is nothing to read.
function read(m: Model): Step {
  if (m.target === null) {
    return [set(m, { loading: false, status: NO_AGENT }), []];
  }
  return [set(m, { loading: true, status: null }), [{ _tag: "ReadMessage", target: m.target }]];
}

// The header button. Opening the message view reads the message again, and the diff view comes back as it was.
export function viewChosen(m: Model, view: "diff" | "message"): Step {
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

// Refresh. The comments stay when the message that comes back is the same one.
export function refreshed(m: Model): Step {
  if (m.view !== "message" || m.message.loading) {
    return [m, []];
  }
  // A new message would close the editor and lose what is typed in it.
  return m.editor !== null ? [told(m, BUSY_EDITOR), []] : read(m);
}

// The message arrived. A message with another id is another message, so the comments, whose lines they are, are dropped
// and the status says so.
export function messageLoaded(m: Model, reply: NonNullable<MessageView["data"]>): Step {
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

// The pointer is over a line, or left it. While a press is held, a line other than the one it began on makes the range.
export function hovered(m: Model, line: number, on: boolean): Step {
  const mv = m.message;
  if (!on) {
    return mv.hover === line ? [set(m, { hover: null }), []] : [m, []];
  }
  if (!inMessage(m, line)) {
    return [m, []];
  }
  const d = mv.drag;
  if (d === null) {
    return mv.hover === line ? [m, []] : [set(m, { hover: line }), []];
  }
  if (d.head === line && mv.hover === line) {
    return [m, []];
  }
  return [set(m, { hover: line, drag: { ...d, head: line, moved: d.moved || line !== d.line }, selection: line === d.line && !d.moved ? mv.selection : range(d.line, line) }), []];
}

// A press starts a drag and changes nothing else yet: a release without a move is a click, which clears the selection.
export function pressed(m: Model, line: number): Step {
  if (m.editor !== null || !inMessage(m, line)) {
    return [m, []];
  }
  return [set(m, { hover: line, drag: { line, head: line, moved: false } }), []];
}

export function released(m: Model): Step {
  const d = m.message.drag;
  if (d === null) {
    return [m, []];
  }
  return [set(m, { drag: null, selection: d.moved ? m.message.selection : null }), []];
}

// The [+] of a line opens the editor on the selection when the line is in it, else on the line alone.
// Not while a send runs: its reply clears the comments, which would take a new one with them unsent.
const SENDING = "wait for the send to finish";

export function plusClicked(m: Model, line: number): Step {
  if (m.editor !== null) {
    return [told(m, BUSY_EDITOR), []];
  }
  if (m.message.sending) {
    return [told(m, SENDING), []];
  }
  if (!inMessage(m, line)) {
    return [m, []];
  }
  const s = m.message.selection;
  const sel = s !== null && line >= s.from && line <= s.to ? s : { from: line, to: line };
  const editor = { kind: "comment" as const, anchor: null, thread: null, comment: null, initial: "", title: `Comment on ${rangeLabel(sel.from, sel.to)}` };
  return [{ ...set(m, { selection: sel, drag: null, status: null }), editor }, []];
}

// Save in the editor: a new comment on the selection, or the new text of the one being edited. It is kept in the model at once.
export function messageSaved(m: Model, text: string): Step {
  const e = m.editor;
  const body = text.trim();
  if (e === null || body === "") {
    return [m, []];
  }
  const mv = m.message;
  if (e.kind === "edit") {
    const id = Number(e.comment);
    return [{ ...set(m, { comments: mv.comments.map((c) => (c.id === id ? { ...c, body } : c)), status: null }), editor: null }, []];
  }
  if (mv.selection === null) {
    return [m, []];
  }
  const comment: MessageComment = { id: mv.next, start: mv.selection.from, end: mv.selection.to, body };
  return [{ ...set(m, { comments: [...mv.comments, comment], next: mv.next + 1, selection: null, status: null }), editor: null }, []];
}

// Edit opens the editor on the comment, and delete removes it at once, as the TUI does.
export function commentAction(m: Model, id: number, action: "edit" | "delete"): Step {
  if (m.editor !== null) {
    return [told(m, BUSY_EDITOR), []];
  }
  if (m.message.sending) {
    return [told(m, SENDING), []];
  }
  const c = m.message.comments.find((x) => x.id === id);
  if (c === undefined) {
    return [m, []];
  }
  if (action === "delete") {
    return [set(m, { comments: m.message.comments.filter((x) => x.id !== id) }), []];
  }
  return [{ ...m, editor: { kind: "edit", anchor: null, thread: null, comment: String(id), initial: c.body, title: `Edit comment on ${rangeLabel(c.start, c.end)}` } }, []];
}

// Send all the comments, on the message they were written on. The id lets the CLI refuse when the agent has written since.
export function sendClicked(m: Model): Step {
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

// The agent took the prompt: the comments are spent.
export function messageSent(m: Model, sent: number): Step {
  const to = m.message.data === null ? "the agent" : m.message.data.agent;
  return [set(m, { sending: false, comments: [], selection: null, drag: null, status: `sent ${plural(sent, "comment", "comments")} to ${to}` }), []];
}

// A read or a send failed. The message and the comments stay, and the reason is the status.
export function messageFailed(m: Model, error: ReviewError): Step {
  return [set(m, { sending: false, loading: false, status: errorText(error) }), []];
}

const firstLine = (text: string): string => text.trim().split("\n")[0] ?? "";

export function errorText(error: ReviewError): string {
  switch (error._tag) {
    case "BinaryMissing":
      return "herdr-review not found, set props.bin or run scripts/stage-app-mod.sh";
    case "TooOld":
      return "update herdr-review";
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
      const unhandled: never = error;
      throw new Error(`unhandled error ${JSON.stringify(unhandled)}`);
    }
  }
}

// What the view draws, top to bottom: every line of the message, and a card under the last line of each comment.
export type MessageItem = { kind: "line"; n: number; text: string; selected: boolean; hovered: boolean } | { kind: "card"; id: number; label: string; body: string[] };

export function messageItems(mv: MessageView): MessageItem[] {
  if (mv.data === null) {
    return [];
  }
  const out: MessageItem[] = [];
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
