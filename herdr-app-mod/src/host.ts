// The only door to the outside. One interface, and the real object behind it. Tests pass a FakeHost.
import { appHome, exec, focused, nextCommand, notify, panes, props, readTextFile, run, terminals, theme, watchFolder, writeTextFile } from "herdr";

export type ExecResult = { code: number; stdout: string; stderr: string };
export type Notice = { title: string; body?: string; level?: "info" | "success" | "warning" | "error" };

export interface Host {
  exec(command: string, args: string[], options?: { cwd?: string; timeout?: number }): Promise<ExecResult>;
  readTextFile(path: string): Promise<string | null>;
  writeTextFile(path: string, text: string, expected: string | null): Promise<null>;
  watchFolder(path: string): Promise<string[]>;
  notify(notice: Notice): string;
  run(name: string, args?: Record<string, unknown>): null;
  // Untrusted: decoded in decode.ts before use.
  focused(): unknown;
  panes(): unknown;
  terminals(): unknown;
  props(): unknown;
  appHome(): string;
  nextCommand(): Promise<string>;
  theme(): Record<string, `#${string}`>;
}

export const realHost: Host = {
  exec: (command, args, options) => exec(command, args, options),
  readTextFile: (path) => readTextFile(path),
  writeTextFile: (path, text, expected) => writeTextFile(path, text, expected),
  watchFolder: (path) => watchFolder(path),
  notify: (notice) => notify(notice),
  run: (name, args) => run(name, args),
  focused: () => focused(),
  panes: () => panes(),
  terminals: () => terminals(),
  props: () => props(),
  appHome: () => appHome(),
  nextCommand: () => nextCommand(),
  // The setters want `#${string}` and theme() says string, so this is the one cast.
  theme: () => theme() as Record<string, `#${string}`>,
};
