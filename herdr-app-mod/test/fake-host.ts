// A Host that answers from a table and records every call. Only the tests use it.
import { readFileSync } from "node:fs";
import type { ExecResult, Host, Notice } from "../src/host.ts";

export const fixture = (name: string): string => readFileSync(new URL(`./fixtures/${name}.json`, import.meta.url), "utf8");

export type Call = { method: string; args: unknown[] };
export type Reply = ExecResult | "reject";

export type Setup = {
  // The answer to an exec, by the binary and the argv. The default answers state and file from the fixtures.
  exec?: (command: string, args: string[]) => Reply;
  props?: unknown;
  // What readTextFile gives for any path: the text, null for a missing file, or "reject".
  file?: string | null | "reject";
  focused?: unknown;
  panes?: unknown;
  // A watch wakes when the test calls wake(), or rejects when `watchRejects` is set.
  watchRejects?: boolean;
};

export const ok = (stdout: string): ExecResult => ({ code: 0, stdout, stderr: "" });
export const exit = (code: number, stderr: string): ExecResult => ({ code, stdout: "", stderr });

export function defaultExec(_command: string, args: string[]): Reply {
  if (args[0] === "state") return ok(fixture("state"));
  if (args[0] === "file") return ok(fixture("file"));
  // The fixture file under each path asked for.
  if (args[0] === "files") return ok(JSON.stringify({ schema: 1, files: args.flatMap((a, i) => (args[i - 1] === "--path" ? [{ ...JSON.parse(fixture("file")), path: a }] : [])) }));
  return exit(2, "unexpected call");
}

export function fakeHost(setup: Setup = {}): { host: Host; calls: Call[]; wake: () => void } {
  const calls: Call[] = [];
  const wakes: (() => void)[] = [];
  const record = (method: string, args: unknown[]): void => {
    calls.push({ method, args });
  };
  const host: Host = {
    async exec(command, args, options) {
      record("exec", [command, args, options]);
      const reply = (setup.exec ?? defaultExec)(command, args);
      if (reply === "reject") throw new Error("spawn failed");
      return reply;
    },
    async readTextFile(path) {
      record("readTextFile", [path]);
      if (setup.file === "reject") throw new Error("unreadable");
      return setup.file ?? null;
    },
    async writeTextFile(path, text, expected) {
      record("writeTextFile", [path, text, expected]);
      return null;
    },
    watchFolder(path) {
      record("watchFolder", [path]);
      if (setup.watchRejects) return Promise.reject(new Error("no watch"));
      return new Promise((resolve) => wakes.push(() => resolve([path])));
    },
    notify(notice: Notice) {
      record("notify", [notice]);
      return "n1";
    },
    run(name, args) {
      record("run", [name, args]);
      return null;
    },
    focused() {
      record("focused", []);
      return setup.focused ?? null;
    },
    panes() {
      record("panes", []);
      return setup.panes ?? [];
    },
    terminals() {
      record("terminals", []);
      return [];
    },
    props() {
      record("props", []);
      return setup.props ?? {};
    },
    appHome() {
      record("appHome", []);
      return "/home/app";
    },
    nextCommand() {
      record("nextCommand", []);
      return new Promise(() => {});
    },
    theme() {
      record("theme", []);
      return {};
    },
  };
  return { host, calls, wake: () => wakes.splice(0).forEach((w) => w()) };
}
