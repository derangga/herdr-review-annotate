// Every type of the mod. No code. The JSON types mirror design/shapes.md field for field, so a reply is
// decoded once in decode.ts and read as is.

export type Spec = { kind: "worktree" } | { kind: "branch"; base: string };

export type Change =
  | "modified"
  | "added"
  | "deleted"
  | "renamed"
  | "untracked"
  | "binary"
  | "submodule"
  | "too_large"
  | "unparsed";

export type FileSummary = {
  path: string;
  old_path: string | null;
  change: Change;
  add: number;
  del: number;
  viewed: boolean;
  threads: number;
  open: number;
  new: number;
  unsent: number;
};

// What a piece of code is, as far as its colour goes: the kinds of the Rust theme, sent by `file --json`.
export type TokenKind = "comment" | "string" | "number" | "keyword" | "operator" | "function" | "type";

// A coloured part of a row's text: `text.slice(from, to)`, offsets in UTF-16 code units. A row's tokens are in order,
// do not overlap and are not empty; decode.ts drops any that are not.
export type Token = { from: number; to: number; kind: TokenKind };

export type Row = { kind: "hunk" | "ctx" | "add" | "del"; old: number | null; new: number | null; text: string; tokens: Token[] };

export type Placement = { kind: "matched" | "outdated" | "not_in_diff"; row: number | null; start_row: number | null };

export type Anchor = {
  path: string;
  old_path: string | null;
  kind: "line" | "range" | "file";
  side: "old" | "new" | null;
  line: number | null;
  end_line: number | null;
  text: string | null;
  spec: Spec;
};

export type Comment = {
  id: string;
  parent: string | null;
  author: string;
  at: string;
  body: string;
  sent: boolean;
  edited_since_sent: boolean;
};

export type Thread = {
  id: string;
  status: "open" | "resolved";
  resolved_by: string | null;
  is_new: boolean;
  unsent: boolean;
  placement: Placement;
  anchor: Anchor;
  comments: Comment[];
};

export type State = {
  schema: 1;
  root: string;
  store_dir: string;
  spec: Spec;
  base: string | null;
  rev: string;
  notices: string[];
  warnings: string[];
  archivable: number; // what archive would move now: resolved threads whose agent resolve the user has seen
  unread_resolved: number; // resolved threads archive leaves because the agent's resolve is unseen: the same predicate, negated
  files: FileSummary[];
  orphans: Thread[];
};

export type FileView = {
  schema: 1;
  path: string;
  old_path: string | null;
  change: Change;
  add: number;
  del: number;
  viewed: boolean;
  note: string | null;
  rows: Row[];
  threads: Thread[];
};

export type Candidate = { address: string; agent: string; cwd: string; status: string };

// What send --json printed on success. sent is 0 when there was nothing to send.
export type SendReply = { schema: 1; sent: number; queued: boolean; agent: string | null; warnings: string[] };

export type ArchiveReply = { schema: 1; archived: number; unsent: number };

export type Props = { root: string | null; target: string | null; bin: string | null };

// What the host says about the focused terminal and the panes, after decoding.
export type Focus = { tab: string | null; cwd: string | null; terminal: string | null; agent: boolean };
export type PaneEntry = { tab: string | null; cwd: string; terminal: string | null; agent: boolean };

// Errors are tagged objects that an inner function throws and the runner turns into Msg.Failed.
export type ReviewError =
  | { _tag: "BinaryMissing" }
  | { _tag: "TooOld" }
  | { _tag: "ContractMismatch"; path: string; detail: string }
  | { _tag: "Busy" }
  | { _tag: "BackendFailed"; code: number; stderr: string }
  | { _tag: "BadRequest"; stderr: string }
  | { _tag: "Refused"; why: "Blocked" | "NotReady" | "AgentGone" }
  | { _tag: "NoTarget" }
  | { _tag: "AmbiguousTarget"; candidates: Candidate[] }
  | { _tag: "WatchUnavailable" };

// from and to are row indexes, the first and last row that has a line on `side`. `anchor` is the row a drag began on, which
// is from or to.
export type Selection = { path: string; side: "old" | "new"; from: number; to: number; anchor: number };

// Where a new comment points, as the batch the CLI reads names it. A file comment has only the path.
export type NewAnchor = { path: string; old_path: string | null; side: "old" | "new" | null; line: number | null; end_line: number | null };

// The text lives in the textarea state the view owns. `initial` is what the state starts with: the comment's
// own text for an edit, empty otherwise. `thread` is the root id a reply goes to, `comment` the id an edit changes.
export type Editor = { kind: "comment" | "reply" | "edit"; anchor: NewAnchor | null; thread: string | null; comment: string | null; initial: string };

// The panel's own settings, kept in a file under appHome(): which side the sidebar is on, and whether it shows.
export type Panel = { side: "left" | "right"; open: boolean };

// A next or previous comment into a file whose threads are not loaded yet: the file to wait for, and which of its
// threads to land on.
export type Jump = { path: string; edge: "first" | "last" };

// A request to scroll the stream so the item with this key is at the top. A new object is a new request.
export type Reveal = { key: string };

// The row the pointer is over, if it is a row of a file's diff.
export type Hover = { path: string; row: number };

// A left press on a row, until the button is released. `row` is where it began, `head` the row the pointer is on now, and
// `moved` says the pointer left the first row, so the press is a drag and not a click.
export type Drag = { path: string; row: number; head: number; moved: boolean };

// A grip on the scrollbar's thumb: where the pointer was and where the stream was.
export type Grip = { y: number; offset: number };

export type Model = {
  root: string | null;
  target: string | null;
  spec: Spec | null; // null until the first reply names the spec meta.json holds
  key: string; // root and spec as one value, to drop a stale reply
  state: State | null;
  files: ReadonlyMap<string, FileView>; // the diffs loaded so far, by path
  selected: string | null; // the sidebar's file: the topmost one in the stream, or the one a click chose
  top: string | null; // the key of the first item on screen
  visible: string[]; // the paths of the files on screen, topmost first
  reveal: Reveal | null; // a scroll the view has not made yet
  width: number; // the stream's width, for the height a card takes
  height: number; // the stream's height, the part of it on screen
  offset: number; // how far the stream is scrolled, in pixels from its top; the mod scrolls it itself
  total: number; // the stream's height in pixels as the last Scrolled saw it, to clamp the offset and size the thumb
  hover: Hover | null; // the code row under the pointer: the one that draws a [+]
  drag: Drag | null; // a press on a row that has not been released
  grip: Grip | null; // a press on the scrollbar's thumb that has not been released
  fresh: string[]; // the cards on screen of threads an agent resolved and the user has not seen, as the last Scrolled said
  glance: string[]; // `fresh` as the last Glanced saw it: a card in both has been on screen for a whole tick
  seenSent: ReadonlySet<string>; // the threads a seen was already sent for, so looking sends one once
  folds: ReadonlyMap<string, boolean>; // files the user folded (true) or opened (false); the rest follow `viewed`
  dirty: ReadonlySet<string>; // loaded files to fetch again
  skipped: ReadonlySet<string>; // files whose last fetch failed, not tried again until the next load
  batch: readonly string[]; // the files the running fetch is for
  expanded: ReadonlySet<string>;
  selection: Selection | null;
  focus: string | null; // the comment the thread buttons act on
  confirm: string | null; // the comment a click on delete asked about, until Delete or Cancel answers
  jump: Jump | null; // a next or previous comment that waits for its file to load
  editor: Editor | null;
  saving: boolean; // a draft is being written, so a second save does nothing
  picker: Candidate[] | null;
  sending: boolean; // a send is running, so a second click does nothing
  sendThreads: string[]; // the threads of the send the picker interrupted, empty for "all unsent"
  confirmArchive: boolean; // a click on archive asked, until Archive or Cancel answers
  sticky: boolean; // status is a result (sent, archived) that a reload must not clear
  banner: string | null;
  status: string | null;
  inFlight: boolean;
  pending: boolean;
  stopped: boolean; // after TooOld, ContractMismatch or BinaryMissing: only Refresh loads again
  storeWatch: string | null; // the store directory the watch loop runs on
  retried: boolean; // a BadRequest already led to a reload that has not succeeded yet
  panel: Panel; // the sidebar's side and whether it shows, remembered between openings
  filter: string; // the sidebar's query, empty for no filter; it narrows the file list and nothing else
  filtering: boolean; // the query box holds the keyboard, so a key types and is not a command
};

export type Trigger = "watch" | "timer" | "refresh" | "write" | "change" | "editor";

// What happened. The first block is built, the second belongs to the comment, send and archive work.
export type Msg =
  | { _tag: "Opened"; root: string | null; target: string | null }
  | { _tag: "Triggered"; source: Trigger }
  | { _tag: "Loaded"; key: string; state: State }
  | { _tag: "FilesLoaded"; key: string; files: FileView[] }
  | { _tag: "Scrolled"; offset: number; total: number; top: string | null; visible: string[]; fresh: string[]; revealed: boolean }
  | { _tag: "Glanced" }
  | { _tag: "Wheeled"; dy: number }
  | { _tag: "ThumbGrabbed"; y: number }
  | { _tag: "ThumbMoved"; y: number }
  | { _tag: "TrackClicked"; y: number }
  | { _tag: "Resized"; width: number; height: number }
  | { _tag: "Hovered"; path: string; row: number; on: boolean }
  | { _tag: "RowPressed"; path: string; row: number }
  | { _tag: "Released" }
  | { _tag: "FoldToggled"; path: string }
  | { _tag: "Failed"; op: Op; error: ReviewError }
  | { _tag: "SpecChosen"; spec: Spec }
  | { _tag: "FileChosen"; path: string }
  | { _tag: "EditorCancelled" }
  | { _tag: "PlusClicked"; path: string; row: number }
  | { _tag: "SelectionCleared" }
  | { _tag: "FileCommentClicked" }
  | { _tag: "EditorSaved"; text: string }
  | { _tag: "ThreadAction"; id: string; action: ThreadActionKind }
  | { _tag: "ThreadJumped"; forward: boolean }
  | { _tag: "SendClicked"; threads?: string[] }
  | { _tag: "PickerChosen"; target: string }
  | { _tag: "PickerCancelled" }
  | { _tag: "Sent"; sent: number; agent: string | null; queued: boolean; warnings: string[] }
  | { _tag: "ArchiveClicked" }
  | { _tag: "ArchiveConfirmed" }
  | { _tag: "DeleteConfirmed"; id: string }
  | { _tag: "ConfirmCancelled" }
  | { _tag: "Archived"; archived: number; unsent: number }
  | { _tag: "ViewedClicked"; path: string; viewed: boolean }
  | { _tag: "Wrote"; kind: WriteKind }
  | { _tag: "SidebarToggled" }
  | { _tag: "SidebarFlipped" }
  | { _tag: "FilterChanged"; text: string }
  | { _tag: "FilterFocused"; on: boolean }
  | { _tag: "FilterOpened" }
  | { _tag: "FilterEntered" }
  | { _tag: "FilterCleared" }
  | { _tag: "PanelLoaded"; panel: Panel };

// An id is a comment id. reply, resolve and reopen act on the thread that comment is in, edit and delete on
// the comment. delete asks, DeleteConfirmed writes. focus selects a comment, toggle selects a thread and opens or closes a resolved one.
export type ThreadActionKind = "focus" | "toggle" | "reply" | "edit" | "resolve" | "reopen" | "delete";

export type WriteOp =
  | { kind: "comment"; anchor: NewAnchor; text: string }
  | { kind: "reply"; id: string; text: string }
  | { kind: "edit"; id: string; text: string }
  | { kind: "resolve"; id: string }
  | { kind: "reopen"; id: string }
  | { kind: "delete"; id: string }
  | { kind: "seen"; id: string };
export type WriteKind = WriteOp["kind"] | "viewed";

export type Op = "load" | "files" | "watch-root" | "watch-store" | "write" | "seen" | "send" | "archive" | "viewed" | "panel";

// What to do next.
export type Cmd =
  | { _tag: "Load"; key: string; root: string; spec: Spec | null }
  | { _tag: "LoadFiles"; key: string; root: string; spec: Spec | null; paths: string[] }
  | { _tag: "WatchStore"; dir: string }
  | { _tag: "Notify"; title: string; body: string }
  | { _tag: "Write"; root: string; spec: Spec | null; op: WriteOp }
  | { _tag: "Send"; root: string; target: string | null; threads: string[] }
  | { _tag: "Archive"; root: string }
  | { _tag: "SetViewed"; root: string; spec: Spec | null; path: string; viewed: boolean }
  | { _tag: "ReadPanel" }
  | { _tag: "SavePanel"; panel: Panel };
