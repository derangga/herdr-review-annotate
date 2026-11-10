// The reducer. (Model, Msg) -> (Model, Cmd[]). Pure. Single flight lives here: at most one load runs, and
// a trigger that comes during one, or while an editor is open, sets `pending` and starts one more load
// when the load ends or the editor closes.
import { anchorOf, archivableCount, BATCH, cardKey, cardOrder, clampOffset, cursorSpot, filterCount, filterPick, findComment, isFolded, isSplit, itemKey, jumpStop, keyPath, lane, maxOffset, offsetOfThumb, partner, pick, plural, plusTarget, sameRows, sameSpot, sideOf, span, spots, thumbOf, topSpot, unsentCount, visualOn, wanted } from "./rows.ts";
import type { Spot } from "./rows.ts";
import { commentAction, hovered as messageHovered, initMessage, messageFailed, messageLoaded, messageSaved, messageSent, plusClicked as messagePlusClicked, pressed as messagePressed, refreshed as messageRefreshed, released as messageReleased, sendClicked as messageSendClicked, viewChosen } from "./message.ts";
import type { Cmd, FileView, Jump, Model, Msg, Panel, ReviewError, Reveal, Spec, State, ThreadActionKind, WriteOp } from "./types.ts";

type Side = "old" | "new" | null;

export const specText = (spec: Spec | null): string => (spec === null ? "meta" : spec.kind === "worktree" ? "worktree" : `branch:${spec.base}`);
const keyOf = (root: string | null, spec: Spec | null): string => JSON.stringify([root, specText(spec)]);

export const DEFAULT_PANEL: Panel = { side: "left", open: true };

export function initModel(): Model {
  return {
    root: null,
    target: null,
    spec: null,
    key: keyOf(null, null),
    state: null,
    files: new Map(),
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
    seenSent: new Set(),
    folds: new Map(),
    dirty: new Set(),
    skipped: new Set(),
    batch: [],
    expanded: new Set(),
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
  };
}

type Step = [Model, Cmd[]];

const BUSY_EDITOR = "finish or cancel the comment first";

// Replaces root or spec and recomputes the key with them.
function rekey(m: Model, change: Partial<Pick<Model, "root" | "spec">>): Model {
  const next = { ...m, ...change };
  return { ...next, key: keyOf(next.root, next.spec) };
}

// A load of the state. The files whose last fetch failed are tried again by the files this load wants.
function startLoad(m: Model): Step {
  if (m.root === null) return [m, []];
  return [{ ...m, inFlight: true, pending: false, skipped: new Set() }, [{ _tag: "Load", key: m.key, root: m.root, spec: m.spec }]];
}

// An open editor holds the loads back, so the screen does not change under a draft. A saved draft waiting for its card
// (`landing`) does not: that load is the one it waits for.
const holding = (m: Model): boolean => m.editor !== null && m.landing === null;

// The next few files the stream wants, in the order `wanted` gives, fetched in one command.
function nextFiles(m: Model): Step {
  if (m.root === null || m.inFlight || holding(m) || m.stopped) return [m, []];
  const paths = wanted(m).slice(0, BATCH);
  if (paths.length === 0) return [m, []];
  return [{ ...m, inFlight: true, batch: paths }, [{ _tag: "LoadFiles", key: m.key, root: m.root, spec: m.spec, paths }]];
}

// Starts what waits when nothing holds it back: the one load that `pending` stands for, else the files the stream
// still wants. One command runs at a time.
function flush(m: Model, cmds: Cmd[] = []): Step {
  if (m.inFlight || holding(m) || m.stopped) return [m, cmds];
  const [next, more] = m.pending ? startLoad(m) : nextFiles(m);
  return [next, [...cmds, ...more]];
}

function trigger(m: Model): Step {
  if (m.root === null) return [m, []];
  if (m.inFlight || holding(m)) return [{ ...m, pending: true }, []];
  return startLoad(m);
}

// A code row, if the row index is one of the file's: a message from an old frame may name a row of a diff that has
// since been replaced.
const rowOf = (m: Model, path: string, row: number): FileView | null => {
  const file = m.files.get(path);
  return file !== undefined && file.rows[row] !== undefined ? file : null;
};

// `next` is `m` changed in a way that may flip the layout. The rows are other items in the other layout, so the pixel offset
// would land elsewhere: the stream scrolls back to the item on top (or the one that holds its row), unless a scroll is
// already on its way, and no hover is kept.
function flipped(m: Model, next: Model): Model {
  if (isSplit(m) === isSplit(next)) return next;
  // The cursor stays on its row: it has no half in unified, and in side by side the row's own, or the range's while one grows.
  const c = next.cursor;
  const row = c === null ? undefined : next.files.get(c.path)?.rows[c.row];
  const side = !isSplit(next) || row === undefined ? null : next.visual && next.selection !== null ? next.selection.side : sideOf(row);
  const cursor = c === null || row === undefined ? null : { ...c, side };
  return { ...next, cursor, hover: null, shift: 0, reveal: next.reveal ?? (m.top === null ? null : { key: m.top }) };
}

// The pointer is over a row, or left it. The [+] is drawn on the hovered row. A leave only counts when it is the row
// the model has, since the next row's enter can come before this row's leave. While a press is held, the pointer coming
// onto another row of the same file makes the range between the row the press began on and this one.
function hovered(m: Model, path: string, row: number, side: Side, on: boolean): Step {
  const same = m.hover !== null && m.hover.path === path && m.hover.row === row && m.hover.side === side;
  if (!on) return same ? [{ ...m, hover: null }, []] : [m, []];
  const file = rowOf(m, path, row);
  if (file === null) return [m, []];
  const next = same ? m : { ...m, hover: { path, row, side } };
  const d = next.drag;
  // The pointer's place in a side-by-side drag is a pair: on the other half than the press began on, it is the row facing this one.
  const head = d !== null && d.side !== null && side !== null && side !== d.side ? (partner(file, row) ?? row) : row;
  if (d === null || d.path !== path || d.head === head || rowOf(m, path, d.row) === null) return [next, []];
  return [{ ...next, drag: { ...d, head, moved: true }, selection: span(file, d.row, head, d.side), focus: null, confirm: null }, []];
}

// A left press on a code row. It starts a drag and changes nothing else yet: a release without a move is a click, which
// clears the selection, and a move makes the new one. A press that is still held from before (its release was lost
// outside the window) is replaced. The editor open means the rows take no press.
function pressed(m: Model, path: string, row: number, side: Side): Step {
  const file = m.editor !== null ? null : rowOf(m, path, row);
  if (file === null) return [m, []];
  // The keyboard's cursor goes where the press is. A press ends a range v began: what a drag makes is the mouse's.
  const cursor = file.rows[row]?.kind === "hunk" ? m.cursor : { path, row, side };
  return [{ ...m, cursor, visual: false, hover: { path, row, side }, drag: { path, row, head: row, moved: false, side } }, []];
}

// The button went up, anywhere in the panel. A press that never moved was a click, and a click on a line clears the
// selection and highlights nothing. A press that moved keeps the range it made.
function released(m: Model): Step {
  if (m.drag === null && m.grip === null) return [m, []];
  return [{ ...m, drag: null, grip: null, selection: m.drag !== null && !m.drag.moved ? null : m.selection }, []];
}

// The scrollbar: the offset is the pixel the stream is scrolled to, clamped to the stream and the screen. A model that
// already has it is returned as it is, so a wheel at the end of the stream draws nothing.
function scrollTo(m: Model, offset: number): Step {
  const next = clampOffset(offset, m.total, m.height);
  return next === m.offset ? [m, []] : [{ ...m, offset: next }, []];
}

// The thumb was pressed at window height `grip.y` with the stream at `grip.offset`. The pointer moving by some pixels moves
// the thumb by as many, which is that many pixels of the track and so more of the stream.
function thumbMoved(m: Model, y: number): Step {
  const thumb = thumbOf(m.total, m.height, 0);
  if (m.grip === null || thumb === null) return [m, []];
  return scrollTo(m, m.grip.offset + ((y - m.grip.y) * maxOffset(m.total, m.height)) / Math.max(1, m.height - thumb.size));
}

// A click on the track (`y` from its top) centres the thumb there.
function trackClicked(m: Model, y: number): Step {
  const thumb = thumbOf(m.total, m.height, m.offset);
  return thumb === null ? [m, []] : scrollTo(m, offsetOfThumb(m.total, m.height, y - thumb.size / 2));
}

// A click on the [+] of a row opens the editor on the range when the row is in it, else on the row alone. What is commented
// becomes the selection, so it stays lit while the editor is open.
function plusClicked(m: Model, path: string, row: number, side: Side): Step {
  const file = m.files.get(path);
  if (file === undefined) return [m, []];
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
  const sel = plusTarget(file, m.selection, row, side);
  if (sel === null) return [m, []];
  return [{ ...m, selection: sel, visual: false, focus: null, confirm: null, status: null, reveal: showEditor(path), editor: { kind: "comment", anchor: anchorOf(file, sel), thread: null, comment: null, initial: "" } }, []];
}

// The editor sits in the stream, so it opens with a scroll, only as far as it takes to bring it on screen.
const showEditor = (path: string): Reveal => ({ key: itemKey(path, "editor"), near: true });

// A saved draft is done: the editor and the selection go. Done in the update that draws the new card, so the stream does not
// close the editor's space first and open the card's after.
const draftLanded = (m: Model): Model => ({ ...m, editor: null, selection: null, saving: false, status: null, draft: null, landing: null });

// The file a draft's card will be in: its anchor's, or its thread's. "" for a thread not in the diff, whose card the state carries.
function draftFile(m: Model): string {
  const e = m.editor;
  if (e === null) return "";
  if (e.anchor !== null) return e.anchor.path;
  const key = cardKey(m, e.thread ?? "");
  return key === null ? "" : keyPath(key);
}

// The cursor stands on `s` and the stream scrolls only as far as it takes to show it (a `near` reveal of its row, which the
// other layout finds too). A range v began grows to it. The focused thread goes, as a drag's does: a line and a card are
// never both picked.
function cursorTo(m: Model, s: Spot): Model {
  const sel = m.selection;
  const file = m.files.get(s.path);
  return {
    ...m,
    cursor: { path: s.path, row: s.row, side: s.side },
    selection: visualOn(m) && sel !== null && file !== undefined ? span(file, sel.anchor, s.row, sel.side) : sel,
    reveal: { key: itemKey(s.path, `r${s.row}`), near: true },
    focus: null,
  };
}

// A motion: `by` spots along the lane, stopping at its ends, which in a range are its file's. With no cursor, or one that
// stands on nothing drawn, the cursor is put on the top row of the screen and goes no further.
function cursorMoved(m: Model, by: number): Step {
  const all = spots(m);
  const walk = lane(m, all);
  const c = m.cursor;
  const i = c === null ? -1 : walk.findIndex((s) => sameSpot(s, c));
  if (c === null || i < 0) {
    const top = topSpot(m, all);
    return top === null ? [m, []] : [cursorTo(m, top), []];
  }
  const to = walk[Math.max(0, Math.min(walk.length - 1, i + by))];
  return to === undefined || sameSpot(to, c) ? [m, []] : [cursorTo(m, to), []];
}

// [ and ]: the first code row of the next hunk, or of this one or the one before when going back, across files.
function cursorHunk(m: Model, forward: boolean): Step {
  const all = spots(m);
  const walk = lane(m, all);
  const c = m.cursor;
  const i = c === null ? -1 : walk.findIndex((s) => sameSpot(s, c));
  if (i < 0) return cursorMoved(m, 0);
  const to = forward ? walk.slice(i + 1).find((s) => s.first) : walk.slice(0, i).reverse().find((s) => s.first);
  return to === undefined ? [m, []] : [cursorTo(m, to), []];
}

// h and l in side by side: the other half of the cursor's pair, when it has a row. Not while a range grows, which keeps its side.
function cursorSide(m: Model, side: "old" | "new"): Step {
  const at = cursorSpot(m);
  const file = at === null ? undefined : m.files.get(at.path);
  if (at === null || file === undefined || !isSplit(m) || at.side === side || visualOn(m)) return [m, []];
  const row = partner(file, at.row);
  return row === null ? [m, []] : [cursorTo(m, { path: at.path, row, side, first: false }), []];
}

// v: with a selection open, ends it; else a one-row range at the cursor that the motions grow.
function visualToggled(m: Model): Step {
  if (m.selection !== null) return [{ ...m, selection: null, visual: false }, []];
  const at = cursorSpot(m);
  const file = at === null ? undefined : m.files.get(at.path);
  if (at === null || file === undefined) return cursorMoved(m, 0);
  const sel = pick(file, at.row, at.side);
  return sel === null ? [m, []] : [{ ...m, selection: sel, visual: true, focus: null, confirm: null }, []];
}

function write(m: Model, op: WriteOp, change: Partial<Model> = {}): Step {
  if (m.root === null) return [m, []];
  return [{ ...m, ...change }, [{ _tag: "Write", root: m.root, spec: m.spec, op }]];
}

function saved(m: Model, text: string): Step {
  if (m.view === "message") return messageSaved(m, text);
  const e = m.editor;
  if (e === null || m.saving || text.trim() === "") return [m, []];
  // The draft's card is drawn at once (rows.draftThread); a failed write brings the editor back with the text.
  const saving = { saving: true, status: null, draft: text };
  if (e.kind === "comment") return e.anchor === null ? [m, []] : write(m, { kind: "comment", anchor: e.anchor, text }, saving);
  if (e.kind === "reply") return e.thread === null ? [m, []] : write(m, { kind: "reply", id: e.thread, text }, saving);
  return e.comment === null ? [m, []] : write(m, { kind: "edit", id: e.comment, text }, saving);
}

function threadAction(m: Model, id: string, action: ThreadActionKind): Step {
  const hit = findComment(m.files, m.state, id);
  if (hit === null) return [m, []]; // the thread went away since the click
  const { thread, comment } = hit;
  const mine = comment.author === "user";
  // A card and a line are never both picked, or the bar would show the line's buttons under a highlighted card.
  if (action === "focus") return [{ ...m, focus: id, confirm: null, selection: null }, []];
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
  const base = { ...m, focus: id, confirm: null, status: null };
  const there = showEditor(keyPath(cardKey(m, thread.id) ?? ""));
  switch (action) {
    case "toggle": {
      if (thread.status !== "resolved") return [{ ...base, focus: thread.id }, []];
      const open = m.expanded.has(thread.id);
      const expanded = new Set(m.expanded);
      if (open) expanded.delete(thread.id);
      else expanded.add(thread.id);
      const next = { ...base, focus: thread.id, expanded };
      // Opening a resolved thread an agent closed is looking at it: it is not new any more.
      return !open && thread.is_new ? write({ ...next, seenSent: new Set(m.seenSent).add(thread.id) }, { kind: "seen", id: thread.id }) : [next, []];
    }
    case "reply":
      return [{ ...base, reveal: there, editor: { kind: "reply", anchor: null, thread: thread.id, comment: null, initial: "" } }, []];
    case "edit":
      return mine ? [{ ...base, reveal: there, editor: { kind: "edit", anchor: null, thread: thread.id, comment: id, initial: comment.body } }, []] : [base, []];
    case "resolve":
      return thread.status === "open" ? write(base, { kind: "resolve", id: thread.id }) : [base, []];
    case "reopen":
      return thread.status === "resolved" ? write(base, { kind: "reopen", id: thread.id }) : [base, []];
    case "delete":
      if (!mine) return [base, []];
      // Only the question: DeleteConfirmed writes, ConfirmCancelled undoes this. The comment asked about takes the focus, so
      // the tinted card is the one the bar's question is about.
      return [{ ...m, confirm: id, focus: id }, []];
    default: {
      const unhandled: never = action;
      throw new Error(`unhandled thread action ${String(unhandled)}`);
    }
  }
}

// Next or previous comment. A thread that is drawn is focused and scrolled to at once. A file whose threads are not
// drawn is opened (unfolded) and, when it has not arrived, waits in `jump` for the fetch that brings it.
function jumped(m: Model, forward: boolean): Step {
  if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
  if (m.root === null || m.picker !== null) return [m, []];
  const stop = jumpStop(m, forward);
  if (stop === null) return [{ ...m, status: "no comments in this review" }, []];
  const base = { ...m, confirm: null, confirmArchive: false, status: null };
  if ("id" in stop) {
    const key = cardKey(m, stop.id);
    return [{ ...base, jump: null, focus: stop.id, reveal: key === null ? m.reveal : { key } }, []];
  }
  const jump: Jump = { path: stop.path, edge: forward ? "first" : "last" };
  const open = { ...base, selection: null, folds: new Map(m.folds).set(stop.path, false) };
  const view = m.files.get(stop.path);
  if (view !== undefined) return flush(landed(open, jump, view));
  return flush({ ...open, jump, reveal: { key: itemKey(stop.path, "file") } });
}

// The file a jump waited for is there: land on its first or last card.
function landed(m: Model, jump: Jump, view: FileView): Model {
  const ids = cardOrder(view);
  const id = jump.edge === "first" ? ids[0] : ids[ids.length - 1];
  if (id === undefined) return { ...m, jump: null, status: `no comments in ${jump.path}` };
  const key = cardKey({ ...m, files: new Map(m.files).set(jump.path, view) }, id);
  return { ...m, jump: null, focus: id, reveal: key === null ? m.reveal : { key } };
}

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

// Looking is seeing, as the TUI counts a cursor landing on a card. A tick that finds a card still on screen from the tick
// before sends its seen. One a tick, so a screen of them is a short queue and not a burst of writes, and a card that was
// scrolled past between two ticks sends nothing. The write is the one opening the card sends.
function glanced(m: Model): Step {
  const id = m.fresh.find((x) => m.glance.includes(x) && !m.seenSent.has(x));
  const same = m.glance.length === m.fresh.length && m.glance.every((x, i) => x === m.fresh[i]);
  if (m.root === null || m.stopped || id === undefined) return same ? [m, []] : [{ ...m, glance: m.fresh }, []];
  return write({ ...m, glance: m.fresh, seenSent: new Set(m.seenSent).add(id) }, { kind: "seen", id });
}

function deleteConfirmed(m: Model, id: string): Step {
  const hit = findComment(m.files, m.state, id);
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

const summaryJson = (state: State | null, path: string): string => JSON.stringify(state?.files.find((f) => f.path === path) ?? null);

// What a reply must leave out of the model once the comments and files it names are known: a focus, a question or a
// selection that points at something that is gone.
function settle(m: Model): Model {
  const sel = m.selection;
  const file = sel === null ? undefined : m.files.get(sel.path);
  return {
    ...m,
    selection: sel !== null && file === undefined ? null : sel,
    drag: m.drag !== null && !m.files.has(m.drag.path) ? null : m.drag,
    focus: findComment(m.files, m.state, m.focus) === null ? null : m.focus,
    confirm: findComment(m.files, m.state, m.confirm) === null ? null : m.confirm,
  };
}

// The state arrived. A file that is gone from the diff goes from the stream. A file whose summary changed, and the
// files on screen, are fetched again. The sidebar's file stays while it is in the diff, else it is the first.
function loadedState(m: Model, key: string, state: State): Step {
  if (key !== m.key) return flush({ ...m, inFlight: false }); // stale: the reply is dropped
  const paths = new Set(state.files.map((f) => f.path));
  const files = new Map([...m.files].filter(([path]) => paths.has(path)));
  const dirty = new Set([...m.dirty, ...[...files.keys()].filter((p) => m.visible.includes(p) || summaryJson(m.state, p) !== summaryJson(state, p))].filter((p) => files.has(p)));
  // A fold the user made that agrees with the file's mark is the file's own again.
  const folds = new Map([...m.folds].filter(([path, fold]) => paths.has(path) && state.files.find((f) => f.path === path)?.viewed !== fold));
  const shown = settle(
    rekey(
      {
        ...m,
        state,
        files,
        dirty,
        folds,
        selected: m.selected !== null && paths.has(m.selected) ? m.selected : (state.files[0]?.path ?? null),
        banner: null,
        status: m.sticky ? m.status : null,
        inFlight: false,
        retried: false,
        // The question names a number, so it goes when the new state has nothing to archive.
        confirmArchive: m.confirmArchive && state.archivable > 0,
      },
      { spec: state.spec },
    ),
  );
  // A saved draft whose card no fetch will bring (a thread not in the diff, a file not loaded or not refetched) lands now.
  const done = shown.landing !== null && (shown.landing === "" || !shown.dirty.has(shown.landing)) ? draftLanded(shown) : shown;
  if (done.storeWatch === null && state.store_dir !== "") return flush({ ...done, storeWatch: state.store_dir }, [{ _tag: "WatchStore", dir: state.store_dir }]);
  return flush(done);
}

// Files arrived. A file above the top of the screen that is now another height moves what is on screen, so the
// item that was at the top is scrolled to again, to within that item. A jump that waited for one of them lands.
function loadedFiles(m: Model, key: string, replies: FileView[]): Step {
  if (key !== m.key) return flush({ ...m, inFlight: false, batch: [] });
  const files = new Map(m.files);
  const dirty = new Set(m.dirty);
  let selection = m.selection;
  let cursor = m.cursor;
  for (const f of replies) {
    if (selection !== null && selection.path === f.path && !sameRows(m.files.get(f.path) ?? null, f, selection)) selection = null;
    // The cursor is a row index: when that row is another line now, a c on it would comment on the wrong line.
    if (cursor !== null && cursor.path === f.path && !sameRows(m.files.get(f.path) ?? null, f, { path: f.path, side: "new", from: cursor.row, to: cursor.row, anchor: cursor.row })) cursor = null;
    files.set(f.path, f);
    dirty.delete(f.path);
  }
  const order = (m.state?.files ?? []).map((f) => f.path);
  const first = m.visible[0] === undefined ? -1 : order.indexOf(m.visible[0]);
  const above = first >= 0 && replies.some((f) => order.indexOf(f.path) >= 0 && order.indexOf(f.path) < first);
  let next = settle({ ...m, files, dirty, selection, cursor, inFlight: false, batch: [], reveal: above && m.top !== null && m.reveal === null ? { key: m.top } : m.reveal });
  // The saved draft's file arrived with its card: the editor goes in this same update.
  if (next.landing !== null && replies.some((f) => f.path === next.landing)) next = draftLanded(next);
  const waiting = m.jump === null ? undefined : replies.find((f) => f.path === (m.jump as Jump).path);
  if (m.jump !== null && waiting !== undefined) next = landed(next, m.jump, waiting);
  return flush(next);
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
    case "Loaded":
      return loadedState(m, msg.key, msg.state);
    case "FilesLoaded":
      return loadedFiles(m, msg.key, msg.files);
    case "Scrolled": {
      // A scroll the panel asked for lands on the file it asked for, even when the end of the stream stops short of
      // putting its header at the top. Otherwise the topmost file on screen is the selected one.
      const goal = msg.revealed && m.reveal !== null ? keyPath(m.reveal.key) : null;
      const selected = goal !== null && msg.visible.includes(goal) ? goal : (msg.visible[0] ?? m.selected);
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
      return hovered(m, msg.path, msg.row, msg.side, msg.on);
    case "RowPressed":
      return pressed(m, msg.path, msg.row, msg.side);
    case "LayoutToggled":
      return [flipped(m, { ...m, split: !isSplit(m) }), []];
    case "Shifted": {
      // Left always moves (down to 0). Right stops at `max`, and past it, after a vertical scroll to shorter lines, does not jump back.
      const next = m.shift + msg.dx;
      const shift = msg.dx < 0 ? Math.max(0, next) : Math.max(m.shift, Math.min(msg.max, next));
      return shift === m.shift ? [m, []] : [{ ...m, shift }, []];
    }
    case "Released":
      return released(m);
    case "FoldToggled": {
      // The editor is drawn in its file: a fold would hide it.
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      const f = m.state?.files.find((x) => x.path === msg.path);
      if (f === undefined) return [m, []];
      const folded = !isFolded(m, f);
      return flush({ ...m, folds: new Map(m.folds).set(f.path, folded), selection: folded && m.selection?.path === f.path ? null : m.selection });
    }
    case "Failed":
      return failed(m, msg);
    case "SpecChosen": {
      if (specText(m.spec) === specText(msg.spec)) return [m, []];
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      // Another spec is another diff: no file that was fetched is kept.
      const fresh = { ...m, files: new Map(), dirty: new Set<string>(), skipped: new Set<string>(), selection: null, cursor: null, visual: false, drag: null, hover: null, grip: null, offset: 0, jump: null, reveal: null, visible: [], top: null, status: m.sticky ? null : m.status, sticky: false };
      return trigger(rekey(fresh, { spec: msg.spec }));
    }
    case "FileChosen": {
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      if (m.state?.files.every((f) => f.path !== msg.path)) return [m, []];
      // The stream scrolls to the file's header, and the files around it are fetched as the stream shows them.
      return flush({ ...m, selection: null, jump: null, selected: msg.path, reveal: { key: itemKey(msg.path, "file") }, status: m.sticky ? null : m.status, sticky: false });
    }
    case "EditorCancelled":
      // Closing the editor keeps the selection, and Esc on the panel drops it (SelectionCleared).
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
      return plusClicked(m, msg.path, msg.row, msg.side);
    case "FileCommentClicked":
      if (m.editor !== null || m.selected === null) return [m, []];
      return [{ ...m, selection: null, status: null, reveal: showEditor(m.selected), editor: { kind: "comment", anchor: { path: m.selected, old_path: null, side: null, line: null, end_line: null }, thread: null, comment: null, initial: "" } }, []];
    case "EditorSaved":
      return saved(m, msg.text);
    case "ThreadAction":
      return threadAction(m, msg.id, msg.action);
    case "ThreadJumped":
      return jumped(m, msg.forward);
    case "Wrote": {
      // A draft is written: the editor stays, saving, until the reload brings the card (`landing`), and goes in the same
      // update. The other writes leave an editor opened since then alone.
      const draft = msg.kind === "comment" || msg.kind === "reply" || msg.kind === "edit";
      return trigger(draft ? { ...m, saving: true, landing: draftFile(m) } : m);
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
      if (m.editor !== null) return [{ ...m, status: BUSY_EDITOR }, []];
      // Marking a file viewed folds it and puts its header at the top, as the TUI does; the mark is the CLI's.
      return [
        { ...m, folds: new Map(m.folds).set(msg.path, msg.viewed), selection: m.selection?.path === msg.path && msg.viewed ? null : m.selection, reveal: msg.viewed ? { key: itemKey(msg.path, "file") } : m.reveal },
        [{ _tag: "SetViewed", root: m.root, spec: m.spec, path: msg.path, viewed: msg.viewed }],
      ];
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
    case "ViewChosen":
      return viewChosen(m, msg.view);
    case "MessageLoaded":
      return messageLoaded(m, msg.reply);
    case "MessageRefreshed":
      return messageRefreshed(m);
    case "MessageHovered":
      return messageHovered(m, msg.line, msg.on);
    case "MessagePressed":
      return messagePressed(m, msg.line);
    case "MessageReleased":
      return messageReleased(m);
    case "MessagePlusClicked":
      return messagePlusClicked(m, msg.line);
    case "MessageCommentAction":
      return commentAction(m, msg.id, msg.action);
    case "MessageSendClicked":
      return messageSendClicked(m);
    case "MessageSent":
      return messageSent(m, msg.sent);
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
  if (op === "message" || op === "message-send") return messageFailed(m, error); // the message view's own status, the diff's is left alone
  const fetch = op === "load" || op === "files";
  const base = {
    ...m,
    inFlight: fetch ? false : m.inFlight,
    // The files of a fetch that failed are not asked for again until the next load, or the stream would ask for ever.
    skipped: op === "files" ? new Set([...m.skipped, ...m.batch]) : m.skipped,
    batch: op === "files" ? [] : m.batch,
    jump: fetch ? null : m.jump,
    saving: op === "write" ? false : m.saving,
    // The write failed: the editor, still open behind the draft's card, is drawn again with its text.
    draft: op === "write" ? null : m.draft,
    sending: op === "send" ? false : m.sending,
    sticky: false,
  };
  // The draft was written; only the reload that would show it failed. The editor goes rather than wait for ever.
  if (fetch && m.landing !== null) return flush(...reduceError(draftLanded(base), error));
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
