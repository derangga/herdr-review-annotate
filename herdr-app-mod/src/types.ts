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

// from and to are row indexes, the first and last row that has a line on `side`. One row is a click.
export type Selection = { path: string; side: "old" | "new"; from: number; to: number };

// Where a new comment points, as the batch the CLI reads names it. A file comment has only the path.
export type NewAnchor = { path: string; old_path: string | null; side: "old" | "new" | null; line: number | null; end_line: number | null };

// The text lives in the textarea state the view owns. `initial` is what the state starts with: the comment's
// own text for an edit, empty otherwise. `thread` is the root id a reply goes to, `comment` the id an edit changes.
export type Editor = { kind: "comment" | "reply" | "edit"; anchor: NewAnchor | null; thread: string | null; comment: string | null; initial: string };

// The panel's own settings, kept in a file under appHome(): which side the sidebar is on, and whether it shows.
export type Panel = { side: "left" | "right"; open: boolean };

// A jump into a file that is not loaded yet: the file to wait for, and which of its threads to land on.
export type Jump = { path: string; edge: "first" | "last" };

// The diff list cannot be scrolled from code, so a jump shows its target by starting the list a few lines above
// it. `from` is the key of the first item (null for the whole file) and `id` names the list: a new id is a new
// list, which starts at the top, so every jump and every return to the whole file gets one.
export type DiffWindow = { from: string | null; id: number };

export type Model = {
  root: string | null;
  target: string | null;
  spec: Spec | null; // null until the first reply names the spec meta.json holds
  key: string; // root, spec and selected path as one value, to drop a stale reply
  state: State | null;
  file: FileView | null;
  selected: string | null;
  expanded: ReadonlySet<string>;
  selection: Selection | null;
  focus: string | null; // the comment the thread buttons act on
  confirm: string | null; // the comment a click on delete asked about, until Delete or Cancel answers
  jump: Jump | null; // a next or previous comment that waits for its file to load
  window: DiffWindow; // where the diff list starts, so a jump can show a thread without a scroll position
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
  | { _tag: "Loaded"; key: string; state: State; file: FileView | null }
  | { _tag: "Failed"; op: Op; error: ReviewError }
  | { _tag: "SpecChosen"; spec: Spec }
  | { _tag: "FileChosen"; path: string }
  | { _tag: "EditorCancelled" }
  | { _tag: "RowClicked"; row: number }
  | { _tag: "FileCommentClicked" }
  | { _tag: "CommentClicked" }
  | { _tag: "EditorSaved"; text: string }
  | { _tag: "ThreadAction"; id: string; action: ThreadActionKind }
  | { _tag: "ThreadJumped"; forward: boolean }
  | { _tag: "EarlierClicked" }
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

export type Op = "load" | "watch-root" | "watch-store" | "write" | "seen" | "send" | "archive" | "viewed" | "panel";

// What to do next.
export type Cmd =
  | { _tag: "Load"; key: string; root: string; spec: Spec | null; selected: string | null }
  | { _tag: "WatchStore"; dir: string }
  | { _tag: "Notify"; title: string; body: string }
  | { _tag: "Write"; root: string; spec: Spec | null; op: WriteOp }
  | { _tag: "Send"; root: string; target: string | null; threads: string[] }
  | { _tag: "Archive"; root: string }
  | { _tag: "SetViewed"; root: string; spec: Spec | null; path: string; viewed: boolean }
  | { _tag: "ReadPanel" }
  | { _tag: "SavePanel"; panel: Panel };
