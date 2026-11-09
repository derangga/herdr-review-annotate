// Runs a Cmd and returns the Msg it makes. Every run is under withRecovery: a thrown tagged object
// becomes Msg.Failed, anything else is a bug and goes on up.
import { decodeFocus, decodePanel, decodePanes, decodeProps } from "./decode.ts";
import type { ReviewCli } from "./cli.ts";
import type { Host } from "./host.ts";
import { batchOf } from "./rows.ts";
import type { Cmd, Msg, Op, Panel, ReviewError, Spec, WriteOp } from "./types.ts";

export type Env = { host: Host; cli: ReviewCli; dispatch: (msg: Msg) => void; closed: () => boolean };

function isTagged(e: unknown): e is ReviewError {
  return typeof e === "object" && e !== null && typeof (e as { _tag?: unknown })._tag === "string";
}

export async function withRecovery(op: Op, handler: () => Promise<Msg | null>): Promise<Msg | null> {
  try {
    return await handler();
  } catch (e) {
    if (isTagged(e)) return { _tag: "Failed", op, error: e };
    throw e;
  }
}

export function run(env: Env, cmd: Cmd): Promise<Msg | null> {
  switch (cmd._tag) {
    case "Load":
      return withRecovery("load", async () => ({ _tag: "Loaded", key: cmd.key, state: await env.cli.state(cmd.root, cmd.spec) }));
    case "LoadFiles":
      // One file after the other: a file is a process, and a fetch that holds many would keep one that scrolled into
      // view waiting. A file that fails fails the fetch, and the reducer does not ask for those files again.
      return withRecovery("files", async () => {
        const files = [];
        for (const path of cmd.paths) files.push(await env.cli.file(cmd.root, cmd.spec, path));
        return { _tag: "FilesLoaded", key: cmd.key, files };
      });
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
    case "ReadPanel":
      return withRecovery("panel", async () => {
        // A missing or unreadable file is the defaults, so a read never fails.
        const text = await env.host.readTextFile(panelFile(env.host)).catch(() => null);
        return { _tag: "PanelLoaded", panel: decodePanel(text) };
      });
    case "SavePanel":
      return withRecovery("panel", async () => {
        await savePanel(env.host, cmd.panel);
        return null;
      });
    default: {
      const unhandled: never = cmd;
      throw new Error(`unhandled Cmd ${JSON.stringify(unhandled)}`);
    }
  }
}

const panelFile = (host: Host): string => `${host.appHome()}/data/herdr-review/panel.json`;

// writeTextFile replaces a file only when `expected` is what it holds now, so the text is read first. Two
// clicks in a row can make the second read stale, so a refused write reads again, up to three times.
async function savePanel(host: Host, panel: Panel): Promise<void> {
  const file = panelFile(host);
  for (let i = 0; i < 3; i++) {
    try {
      await host.writeTextFile(file, `${JSON.stringify(panel)}\n`, await host.readTextFile(file));
      return;
    } catch {
      // read or write refused: read again
    }
  }
  throw { _tag: "BackendFailed", code: -1, stderr: "could not write the panel settings" } satisfies ReviewError;
}

// Runs a command in the review folder's setup, where a failure to start is a backend failure.
async function shell(host: Host, command: string, args: string[]): Promise<void> {
  const fail = (stderr: string): never => {
    throw { _tag: "BackendFailed", code: -1, stderr } satisfies ReviewError;
  };
  let result;
  try {
    result = await host.exec(command, args, { timeout: 30_000 });
  } catch {
    return fail(`could not run ${command}`);
  }
  if (result.code !== 0) fail(result.stderr.trim() || `${command} failed (${result.code})`);
}

// The text goes to a file in a folder only this user can open, and the file's path is all the CLI is told.
// `mkdir` rejects a mode, so the folder is made and then closed with chmod.
async function bodyFile(env: Env, text: string): Promise<string> {
  const dir = `${env.host.appHome()}/data/herdr-review`;
  await shell(env.host, "mkdir", ["-p", dir]);
  await shell(env.host, "chmod", ["700", dir]);
  const file = `${dir}/${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}.txt`;
  try {
    await env.host.writeTextFile(file, text, null);
  } catch {
    throw { _tag: "BackendFailed", code: -1, stderr: "could not write the draft file" } satisfies ReviewError;
  }
  return file;
}

// The CLI deletes the file once it has opened it. When it never got that far (no binary, a binary too old),
// the file still holds the text, and a mod cannot delete, so it is overwritten with nothing.
async function withBodyFile(env: Env, text: string, send: (file: string) => Promise<void>): Promise<void> {
  const file = await bodyFile(env, text);
  try {
    await send(file);
  } catch (e) {
    if (typeof e === "object" && e !== null && ["BinaryMissing", "TooOld"].includes((e as { _tag?: string })._tag ?? "")) {
      await env.host.writeTextFile(file, "", text).catch(() => null);
    }
    throw e;
  }
}

async function write(env: Env, root: string, spec: Spec | null, op: WriteOp): Promise<void> {
  const cli = env.cli;
  switch (op.kind) {
    case "comment":
      return withBodyFile(env, batchOf(op.anchor, op.text), (file) => cli.apply(root, spec, file));
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
      const unhandled: never = op;
      throw new Error(`unhandled write ${JSON.stringify(unhandled)}`);
    }
  }
}

// One watch loop. watchFolder cannot be cancelled, so a wake after close is the last, and it dispatches
// nothing. A rejection is WatchUnavailable.
export async function watch(env: Env, dir: string, op: "watch-root" | "watch-store"): Promise<null> {
  while (!env.closed()) {
    try {
      await env.host.watchFolder(dir);
    } catch {
      if (!env.closed()) env.dispatch({ _tag: "Failed", op, error: { _tag: "WatchUnavailable" } });
      return null;
    }
    if (env.closed()) break;
    env.dispatch({ _tag: "Triggered", source: "watch" });
  }
  return null;
}

// Waits for the commands herdr-app sends a mod. "focus" (pane.go moved onto this copy, or a popup over it closed)
// asks for the keyboard, so n and N reach the panel. "next-comment" and "previous-comment" are what
// `herdr-app do herdr-review.next-comment` and a keys.json binding to that name deliver, from anywhere. Any
// other command is not ours.
export const commandMsg = (command: string): Msg | null =>
  command === "next-comment" ? { _tag: "ThreadJumped", forward: true } : command === "previous-comment" ? { _tag: "ThreadJumped", forward: false } : null;

export async function listen(env: Env, onFocus: () => void): Promise<null> {
  while (!env.closed()) {
    try {
      const command = await env.host.nextCommand();
      if (env.closed()) break;
      if (command === "focus") onFocus();
      const msg = commandMsg(command);
      if (msg !== null) env.dispatch(msg);
    } catch {
      return null;
    }
  }
  return null;
}

// The reads of init: root and target from props, else the focused terminal, else a pane of the same tab.
export function open(host: Host): Msg {
  const props = decodeProps(host.props());
  if (props.root !== null) return { _tag: "Opened", root: props.root, target: props.target };
  const focus = decodeFocus(host.focused());
  if (focus !== null && focus.cwd !== null) return { _tag: "Opened", root: focus.cwd, target: props.target ?? (focus.agent ? focus.terminal : null) };
  const tab = focus === null ? null : focus.tab;
  const panes = decodePanes(host.panes()).filter((p) => p.tab === tab);
  const pane = panes.find((p) => p.agent) ?? panes[0];
  if (pane === undefined) return { _tag: "Opened", root: null, target: null };
  return { _tag: "Opened", root: pane.cwd, target: props.target ?? (pane.agent ? pane.terminal : null) };
}
