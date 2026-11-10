// ReviewCli: builds argv, runs the binary through Host.exec, maps exit codes to the error list in
// design/gui.md and decodes stdout. Inner functions throw tagged objects.
import { decodeArchive, decodeFile, decodeMessage, decodeMessageSent, decodeSend, decodeState, parseJson } from "./decode.ts";
import type { ExecResult, Host } from "./host.ts";
import type { ArchiveReply, FileView, MessageReply, MessageSentReply, ReviewError, SendReply, Spec, State } from "./types.ts";

const READ_TIMEOUT = 60_000;
const WRITE_TIMEOUT = 30_000;

export type ReviewCli = {
  state(root: string, spec: Spec | null): Promise<State>;
  file(root: string, spec: Spec | null, path: string): Promise<FileView>;
  // The user's writes. A body is always a file the CLI reads and deletes, never an argument.
  apply(root: string, spec: Spec | null, batchFile: string): Promise<void>;
  reply(root: string, id: string, bodyFile: string): Promise<void>;
  edit(root: string, id: string, bodyFile: string): Promise<void>;
  resolve(root: string, id: string): Promise<void>;
  reopen(root: string, id: string): Promise<void>;
  remove(root: string, id: string): Promise<void>;
  seen(root: string, id: string): Promise<void>;
  // Send the unsent threads, or only `threads`. With a target it goes there, without one the CLI finds the agent by folder under hc.
  send(root: string, target: string | null, threads: string[]): Promise<SendReply>;
  archive(root: string): Promise<ArchiveReply>;
  // The agent's newest message, and comments on its lines sent back as the agent's next prompt (`message send --file`). The
  // target is the hc address the panel knows. The file is the panel's draft: the CLI reads and deletes it.
  message(target: string): Promise<MessageReply>;
  messageSend(target: string, file: string): Promise<MessageSentReply>;
  view(root: string, spec: Spec | null, path: string, set: boolean): Promise<void>;
};

function fail(error: ReviewError): never {
  throw error;
}

export function specArgs(spec: Spec | null): string[] {
  if (spec === null) return [];
  return spec.kind === "worktree" ? ["--spec", "worktree"] : ["--spec", "branch", "--base", spec.base];
}

// The first of props().bin, the staged copy and PATH that exec can start. It is kept after the first
// answer, so every later call uses the same binary.
export function makeCli(host: Host, bin: string | null): ReviewCli {
  let chosen: string | null = null;

  // The reply of the first binary that starts, whatever its exit code.
  async function spawn(args: string[], timeout: number): Promise<ExecResult> {
    const candidates = chosen !== null ? [chosen] : [bin, `${host.appHome()}/mods/herdr-review/bin/herdr-review`, "herdr-review"].filter((c): c is string => c !== null);
    for (const candidate of candidates) {
      let result;
      try {
        result = await host.exec(candidate, args, { timeout });
      } catch {
        continue; // exec rejected: this binary cannot start, try the next
      }
      chosen = candidate;
      return result;
    }
    return fail({ _tag: "BinaryMissing" });
  }

  // The stdout of a reply with exit 0, else the error its exit code stands for.
  function check(result: ExecResult): string {
    if (result.code === 0) return result.stdout;
    const stderr = result.stderr.trim();
    if (result.code === 2 && /^unknown (command|option|comment command)/.test(stderr)) return fail({ _tag: "TooOld" });
    if (result.code === 1 && stderr.includes("review is busy")) return fail({ _tag: "Busy" });
    if (result.code === 2) return fail({ _tag: "BadRequest", stderr });
    return fail({ _tag: "BackendFailed", code: result.code, stderr: stderr });
  }

  const exec = async (args: string[], timeout: number): Promise<string> => check(await spawn(args, timeout));

  const write = async (args: string[]): Promise<void> => {
    await exec(args, WRITE_TIMEOUT);
  };

  return {
    async state(root, spec) {
      return decodeState(parseJson(await exec(["state", "--json", "--repo", root, ...specArgs(spec)], READ_TIMEOUT)));
    },
    async file(root, spec, path) {
      return decodeFile(parseJson(await exec(["file", "--json", "--repo", root, ...specArgs(spec), "--path", path], READ_TIMEOUT)));
    },
    apply: (root, spec, file) => write(["comment", "apply", "--as", "user", "--file", file, "--repo", root, ...specArgs(spec)]),
    reply: (root, id, file) => write(["comment", "reply", "--as", "user", id, "--body-file", file, "--repo", root]),
    edit: (root, id, file) => write(["comment", "edit", id, "--body-file", file, "--repo", root]),
    resolve: (root, id) => write(["comment", "resolve", "--as", "user", id, "--repo", root]),
    reopen: (root, id) => write(["comment", "reopen", "--as", "user", id, "--repo", root]),
    remove: (root, id) => write(["comment", "delete", id, "--repo", root]),
    seen: (root, id) => write(["comment", "seen", id, "--repo", root]),
    async send(root, target, threads) {
      const argv = ["send", "--repo", root, "--json", ...(target === null ? ["--backend", "hc"] : ["--target", target]), ...threads.flatMap((id) => ["--thread", id])];
      const result = await spawn(argv, WRITE_TIMEOUT);
      // A refused send is JSON on stdout with exit 1. Exit 1 with anything else is a plain failure.
      return decodeSend(parseJson(result.code === 1 && result.stdout.trim().startsWith("{") ? result.stdout : check(result)));
    },
    async archive(root) {
      return decodeArchive(parseJson(await exec(["archive", "--repo", root, "--json"], WRITE_TIMEOUT)));
    },
    async message(target) {
      return decodeMessage(parseJson(await exec(["message", "--json", "--target", target], READ_TIMEOUT)));
    },
    async messageSend(target, file) {
      return decodeMessageSent(parseJson(await exec(["message", "send", "--target", target, "--file", file], WRITE_TIMEOUT)));
    },
    view: (root, spec, path, set) => write(["view", "--repo", root, "--path", path, set ? "--set" : "--unset", ...specArgs(spec)]),
  };
}
