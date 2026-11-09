// The reducer. (Model, Msg) -> (Model, Cmd[]). Pure. Single flight lives here: at most one load runs, and
// a trigger that comes during one, or while an editor is open, sets `pending` and starts one more load
// when the load ends or the editor closes.
import { anchorOf, archivableCount, cardOrder, filterCount, filterPick, findComment, jumpStop, pick, plural, sameRows, span, unsentCount, windowStart } from "./rows.ts";
import type { Cmd, Jump, Model, Msg, Panel, ReviewError, Spec, ThreadActionKind, WriteOp } from "./types.ts";

export const specText = (spec: Spec | null): string => (spec === null ? "meta" : spec.kind === "worktree" ? "worktree" : `branch:${spec.base}`);
const keyOf = (root: string | null, spec: Spec | null, selected: string | null): string => JSON.stringify([root, specText(spec), selected]);

export const DEFAULT_PANEL: Panel = { side: "left", open: true };

export function initModel(): Model {
  return {
    root: null,
    target: null,
    spec: null,
    key: keyOf(null, null, null),
    state: null,
    file: null,
    selected: null,
    expanded: new Set(),
    selection: null,
    focus: null,
    confirm: null,
    jump: null,
    window: { from: null, id: 0 },
    editor: null,
    saving: false,
    picker: null,
    sending: false,
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
  };
}

type Step = [Model, Cmd[]];

const BUSY_EDITOR = "finish or cancel the comment first";

// Replaces root, spec or selected and recomputes the key with them.
function rekey(m: Model, change: Partial<Pick<Model, "root" | "spec" | "selected">>): Model {
  const next = { ...m, ...change };
  return { ...next, key: keyOf(next.root, next.spec, next.selected) };
}

function startLoad(m: Model): Step {
  if (m.root === null) return [m, []];
  return [{ ...m, inFlight: true, pending: false }, [{ _tag: "Load", key: m.key, root: m.root, spec: m.spec, selected: m.selected }]];
}

// Starts the one load that `pending` stands for, when nothing holds it back.
function flush(m: Model, cmds: Cmd[] = []): Step {
  if (!m.pending || m.inFlight || m.editor !== null || m.stopped) return [m, cmds];
  const [next, load] = startLoad(m);
  return [next, [...cmds, ...load]];
}

function trigger(m: Model): Step {
  if (m.root === null) return [m, []];
  if (m.inFlight || m.editor !== null) return [{ ...m, pending: true }, []];
  return startLoad(m);
}

// A click on a row: one row selects it, a second row extends it to a range, a click on the selected row drops it,
// and a click on a range starts a new selection. The editor open means the rows are not clickable.
function rowClicked(m: Model, row: number): Step {
  if (m.editor !== null || m.file === null) return [m, []];
  const sel = m.selection !== null && m.selection.path === m.file.path ? m.selection : null;
  if (sel !== null && sel.from === sel.to) {
    if (sel.from === row) return [{ ...m, selection: null }, []];
    return [{ ...m, selection: span(m.file, sel.from, row) ?? sel }, []];
  }
  return [{ ...m, selection: pick(m.file, row), focus: null, confirm: null }, []];
}

function write(m: Model, op: WriteOp, change: Partial<Model> = {}): Step {
  if (m.root === null) return [m, []];
  return [{ ...m, ...change }, [{ _tag: "Write", root: m.root, spec: m.spec, op }]];
}

function saved(m: Model, text: string): Step {
  const e = m.editor;
  if (e === null || m.saving || text.trim() === "") return [m, []];
  const saving = { saving: true, status: null };
  if (e.kind === "comment") return e.anchor === null ? [m, []] : write(m, { kind: "comment", anchor: e.anchor, text }, saving);
  if (e.kind === "reply") return e.thread === null ? [m, []] : write(m, { kind: "reply", id: e.thread, text }, saving);
  return e.comment === null ? [m, []] : write(m, { kind: "edit", id: e.comment, text }, saving);
}

function threadAction(m: Model, id: string, action: ThreadActionKind): Step {
  const hit = findComment(m.file, m.state, id);
  if (hit === null) return [m, []]; // the thread went away since the click
  const { thread, comment } = hit;
  const mine = comment.author === "user";
  if (action === "focus") return [{ ...m, focus: id, confirm: null }, []];
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
  const base = { ...m, focus: id, confirm: null, status: null };
  switch (action) {
    case "toggle": {
      if (thread.status !== "resolved") return [{ ...base, focus: thread.id }, []];
      const open = m.expanded.has(thread.id);
      const expanded = new Set(m.expanded);
      if (open) expanded.delete(thread.id);
      else expanded.add(thread.id);
      const next = { ...base, focus: thread.id, expanded };
      // Opening a resolved thread an agent closed is looking at it: it is not new any more.
      return !open && thread.is_new ? write(next, { kind: "seen", id: thread.id }) : [next, []];
    }
    case "reply":
      return [{ ...base, editor: { kind: "reply", anchor: null, thread: thread.id, comment: null, initial: "" } }, []];
    case "edit":
      return mine ? [{ ...base, editor: { kind: "edit", anchor: null, thread: thread.id, comment: id, initial: comment.body } }, []] : [base, []];
    case "resolve":
      return thread.status === "open" ? write(base, { kind: "resolve", id: thread.id }) : [base, []];
    case "reopen":
      return thread.status === "resolved" ? write(base, { kind: "reopen", id: thread.id }) : [base, []];
    case "delete":
      if (!mine) return [base, []];
      return [{ ...m, confirm: id }, []]; // only the question: DeleteConfirmed writes, ConfirmCancelled undoes this
    default: {
      const unhandled: never = action;
      throw new Error(`unhandled thread action ${String(unhandled)}`);
    }
  }
}

// Next or previous comment. A thread already loaded is focused and shown at once. A thread in another file
// opens that file and waits in `jump` for the load that brings it (Loaded finishes the jump).
function jumped(m: Model, forward: boolean): Step {
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
  if (m.root === null || m.picker !== null) return [m, []];
  const stop = jumpStop(m, forward);
  if (stop === null) return [{ ...m, status: "no comments in this review" }, []];
  const base = { ...m, confirm: null, confirmArchive: false, status: null };
  if ("id" in stop) return [{ ...base, jump: null, focus: stop.id, window: shownAt(m, stop.id) }, []];
  const edge = forward ? "first" : "last";
  return trigger(rekey({ ...base, selection: null, window: whole(m), jump: { path: stop.path, edge } }, { selected: stop.path }));
}

// The file a jump waited for has arrived: land on its first or last card.
function landed(m: Model, jump: Jump): Model {
  const ids = m.file !== null && m.file.path === jump.path ? cardOrder(m.file) : [];
  const id = jump.edge === "first" ? ids[0] : ids[ids.length - 1];
  if (id === undefined) return { ...m, jump: null, status: `no comments in ${jump.path}` };
  return { ...m, jump: null, focus: id, window: shownAt(m, id) };
}

// The list restarts a few lines above the thread: a new list, so it begins at the top.
const shownAt = (m: Model, id: string): Model["window"] => ({ from: windowStart(m.file, id), id: m.window.id + 1 });
const whole = (m: Model): Model["window"] => ({ from: null, id: m.window.id + 1 });

function sendClicked(m: Model, threads: string[]): Step {
  if (m.root === null || m.sending) return [m, []];
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR, sticky: false }, []];
  if (threads.length === 0 && unsentCount(m.state) === 0) return [{ ...m, status: "nothing to send", sticky: false }, []];
  return [{ ...m, sending: true, sendThreads: threads, picker: null, confirmArchive: false, status: "sending...", sticky: true }, [{ _tag: "Send", root: m.root, target: m.target, threads }]];
}

function sent(m: Model, msg: Extract<Msg, { _tag: "Sent" }>): Step {
  const to = msg.agent === null ? "the agent" : msg.agent;
  const text = msg.sent === 0 ? "nothing to send" : msg.queued ? `${to} is working, ${plural(msg.sent, "comment", "comments")} queued` : `sent ${msg.sent} to ${to}`;
  return trigger({ ...m, sending: false, status: msg.warnings.length > 0 ? `${text} (${msg.warnings.join("; ")})` : text, sticky: true });
}

// The click only asks: the question is the one field, so ConfirmCancelled puts the model back as it was.
function archiveClicked(m: Model): Step {
  if (m.root === null || m.inFlight || m.editor !== null || archivableCount(m.state) === 0) return [m, []];
  return [{ ...m, confirmArchive: true }, []];
}

function archiveConfirmed(m: Model): Step {
  if (m.root === null || m.inFlight || !m.confirmArchive) return [m, []];
  return [{ ...m, confirmArchive: false, status: "archiving...", sticky: true }, [{ _tag: "Archive", root: m.root }]];
}

function deleteConfirmed(m: Model, id: string): Step {
  const hit = findComment(m.file, m.state, id);
  if (m.confirm !== id || hit === null || hit.comment.author !== "user" || m.editor !== null) return [m, []];
  return write({ ...m, focus: id, confirm: null, status: null }, { kind: "delete", id });
}

function archived(m: Model, msg: Extract<Msg, { _tag: "Archived" }>): Step {
  const never = msg.unsent === 0 ? "" : `, ${msg.unsent} of them ${msg.unsent === 1 ? "was" : "were"} never sent`;
  const text = msg.archived === 0 ? "nothing to archive" : `archived ${plural(msg.archived, "thread", "threads")}${never}`;
  return trigger({ ...m, confirmArchive: false, status: text, sticky: true });
}

// The view changes at once, and the file is written so the next opening starts the same way.
function savePanel(m: Model, panel: Panel): Step {
  return [{ ...m, panel }, [{ _tag: "SavePanel", panel }]];
}

// Enter in the box keeps the filter and gives the keyboard back. The file shown goes to the first match unless
// it matches already. With no match nothing happens, so the user keeps typing or presses Esc.
function entered(m: Model): Step {
  if (m.filter === "") return [{ ...m, filtering: false }, []];
  if (filterCount(m.state === null ? [] : m.state.files, m.filter).shown === 0) return [m, []];
  const left = { ...m, filtering: false };
  const first = filterPick(m);
  return first === null ? [left, []] : update(left, { _tag: "FileChosen", path: first });
}

export function update(m: Model, msg: Msg): Step {
  switch (msg._tag) {
    case "Opened": {
      if (msg.root === null) return [{ ...m, banner: "open the panel from a pane that has a folder" }, []];
      return trigger(rekey({ ...m, target: msg.target }, { root: msg.root }));
    }
    case "Triggered": {
      if (msg.source === "refresh") return trigger({ ...m, stopped: false, retried: false, status: m.sticky ? null : m.status, sticky: false });
      if (m.stopped && (msg.source === "watch" || msg.source === "timer")) return [m, []];
      return trigger(m);
    }
    case "Loaded": {
      if (msg.key !== m.key) return flush({ ...m, inFlight: false }); // stale: the reply is dropped
      const selected = msg.file === null ? null : msg.file.path;
      const shown = rekey(
        {
          ...m,
          state: msg.state,
          file: msg.file,
          banner: null,
          status: m.sticky ? m.status : null,
          inFlight: false,
          retried: false,
          selection: m.selection !== null && msg.file !== null && sameRows(m.file, msg.file, m.selection) ? m.selection : null,
          focus: findComment(msg.file, msg.state, m.focus) === null ? null : m.focus,
          confirm: findComment(msg.file, msg.state, m.confirm) === null ? null : m.confirm,
          // The question names a number, so it goes when the new state has nothing to archive.
          confirmArchive: m.confirmArchive && msg.state.archivable > 0,
          // Another file than the one shown has other lines, so a window on the old one means nothing.
          window: m.file !== null && msg.file !== null && m.file.path === msg.file.path ? m.window : whole(m),
        },
        { spec: msg.state.spec, selected },
      );
      const next = m.jump === null ? shown : landed(shown, m.jump);
      if (next.storeWatch === null && msg.state.store_dir !== "") {
        return flush({ ...next, storeWatch: msg.state.store_dir }, [{ _tag: "WatchStore", dir: msg.state.store_dir }]);
      }
      return flush(next);
    }
    case "Failed":
      return failed(m, msg);
    case "SpecChosen": {
      if (specText(m.spec) === specText(msg.spec)) return [m, []];
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      return trigger(rekey({ ...m, jump: null, status: m.sticky ? null : m.status, sticky: false }, { spec: msg.spec }));
    }
    case "FileChosen": {
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      return trigger(rekey({ ...m, selection: null, jump: null, window: whole(m), status: m.sticky ? null : m.status, sticky: false }, { selected: msg.path }));
    }
    case "EditorCancelled":
      // The first one closes the editor and keeps the selection, the next one drops the selection.
      return m.editor !== null ? flush({ ...m, editor: null, saving: false, status: null }) : [{ ...m, selection: null }, []];
    case "RowClicked":
      return rowClicked(m, msg.row);
    case "FileCommentClicked":
      if (m.editor !== null || m.file === null) return [m, []];
      return [{ ...m, selection: null, status: null, editor: { kind: "comment", anchor: { path: m.file.path, old_path: null, side: null, line: null, end_line: null }, thread: null, comment: null, initial: "" } }, []];
    case "CommentClicked": {
      if (m.editor !== null || m.file === null || m.selection === null) return [m, []];
      return [{ ...m, status: null, editor: { kind: "comment", anchor: anchorOf(m.file, m.selection), thread: null, comment: null, initial: "" } }, []];
    }
    case "EditorSaved":
      return saved(m, msg.text);
    case "ThreadAction":
      return threadAction(m, msg.id, msg.action);
    case "ThreadJumped":
      return jumped(m, msg.forward);
    case "EarlierClicked":
      return [{ ...m, window: whole(m) }, []];
    case "Wrote": {
      // A draft is done: the editor and the selection go, and the reload shows the thread. The other writes
      // leave an editor opened since then alone.
      const draft = msg.kind === "comment" || msg.kind === "reply" || msg.kind === "edit";
      return trigger(draft ? { ...m, editor: null, selection: null, saving: false, status: null } : m);
    }
    case "SendClicked":
      return sendClicked(m, msg.threads ?? []);
    case "PickerChosen": {
      if (m.root === null || m.sending || m.picker === null || !m.picker.some((c) => c.address === msg.target)) return [m, []];
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
      if (m.root === null) return [m, []];
      return [m, [{ _tag: "SetViewed", root: m.root, spec: m.spec, path: msg.path, viewed: msg.viewed }]];
    case "SidebarToggled":
      // A box that is no longer drawn cannot hold the keyboard.
      return savePanel({ ...m, filtering: m.panel.open ? false : m.filtering }, { ...m.panel, open: !m.panel.open });
    case "SidebarFlipped":
      return savePanel(m, { ...m.panel, side: m.panel.side === "left" ? "right" : "left" });
    case "FilterChanged":
      return [{ ...m, filter: msg.text }, []];
    case "FilterFocused":
      return [{ ...m, filtering: msg.on }, []];
    case "FilterOpened":
      // `/` with the sidebar hidden shows it first, as the TUI does. The view gives the box the keyboard.
      if (m.editor !== null || m.picker !== null || m.confirm !== null || m.confirmArchive) return [m, []];
      return m.panel.open ? [m, []] : savePanel(m, { ...m.panel, open: true });
    case "FilterCleared":
      return [{ ...m, filter: "", filtering: false }, []];
    case "FilterEntered":
      return entered(m);
    case "PanelLoaded":
      return [{ ...m, panel: msg.panel }, []];
    default: {
      const unhandled: never = msg;
      throw new Error(`unhandled Msg ${JSON.stringify(unhandled)}`);
    }
  }
}

// What each error does. The one place that says it: each row of the table in design/gui.md.
export function failed(m: Model, msg: Extract<Msg, { _tag: "Failed" }>): Step {
  const { op, error } = msg;
  if (op === "watch-store") return [{ ...m, storeWatch: null }, []]; // tried again after the next load
  if (op === "watch-root") return [m, []]; // the timer carries the panel
  if (op === "seen") return [m, []]; // nothing shown, the marker stays until the next load
  if (op === "panel") return [{ ...m, status: "could not remember the sidebar setting", sticky: false }, []]; // the view is right, only the file is not
  const base = {
    ...m,
    inFlight: op === "load" ? false : m.inFlight,
    jump: op === "load" ? null : m.jump,
    saving: op === "write" ? false : m.saving,
    sending: op === "send" ? false : m.sending,
    sticky: false,
  };
  return flush(...reduceError(base, error));
}

function reduceError(m: Model, error: ReviewError): Step {
  switch (error._tag) {
    case "BinaryMissing":
      return [{ ...m, banner: "herdr-review not found, set props.bin or run scripts/stage-app-mod.sh", stopped: true, pending: false }, []];
    case "TooOld":
      return [{ ...m, banner: "update herdr-review", stopped: true, pending: false }, []];
    case "ContractMismatch":
      return [{ ...m, banner: `unexpected reply at ${error.path}: ${error.detail}`, stopped: true, pending: false }, []];
    case "Busy":
      return [{ ...m, status: "review is busy, try again" }, []];
    case "BackendFailed":
      return [m.state === null ? { ...m, banner: error.stderr || `herdr-review failed (${error.code})` } : { ...m, status: error.stderr || `herdr-review failed (${error.code})` }, []];
    case "BadRequest":
      // The screen is stale ("not in diff", "unknown id"): one reload fixes it. Not a second one in a row.
      return [{ ...m, status: error.stderr, pending: m.pending || !m.retried, retried: true }, []];
    case "Refused": {
      // A gone terminal never comes back, so drop it: the next Send, which the user clicks knowing
      // this, finds the agent by folder or offers the picker.
      const gone = error.why === "AgentGone";
      const status = gone
        ? "nothing was sent: the agent is gone. Send again to find an agent for this folder"
        : `nothing was sent: ${error.why}`;
      return [{ ...m, target: gone ? null : m.target, status }, [{ _tag: "Notify", title: "herdr-review", body: status }]];
    }
    case "NoTarget":
      return [{ ...m, status: "no agent found for this folder" }, []];
    case "AmbiguousTarget":
      return [{ ...m, picker: error.candidates, status: null }, []];
    case "WatchUnavailable":
      return [m, []];
    default: {
      const unhandled: never = error;
      throw new Error(`unhandled error ${JSON.stringify(unhandled)}`);
    }
  }
}
