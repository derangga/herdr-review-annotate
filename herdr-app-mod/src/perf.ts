// Opt-in timing of the panel: how long a render and a dispatch take, how often the stream's layout is rebuilt, and how many items a
// render draws. Off unless the mod's props say `perf: true`; off is a recorder whose calls do nothing. One summary line per window
// goes to `${appHome}/data/herdr-review/perf.log` (design/graphs/gui-perf.md). No state of a view lives at module level; the constants and the shared no-op recorder are stateless.
import type { Host } from "./host.ts";

export interface Recorder {
  on: boolean;
  // The time to hand back to render() or dispatch() when the work is done.
  start(): number;
  render(from: number, drawn: number): void;
  dispatch(from: number): void;
  rebuild(): void;
  // Emits the window now, if it holds anything. For close.
  flush(): void;
}

export const WINDOW_MS = 5000;
export const LOG_LINES = 200;

const NOOP: Recorder = { on: false, start: () => 0, render: () => {}, dispatch: () => {}, rebuild: () => {}, flush: () => {} };
export const noop = (): Recorder => NOOP;

// Nearest rank: the smallest value that at least p of the values are at or below. Empty is 0.
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(p * sorted.length) - 1))] as number;
}

const ms = (n: number): string => n.toFixed(2);
const stat = (name: string, v: number[]): string => `${name} n=${v.length} mean=${ms(v.length === 0 ? 0 : v.reduce((a, b) => a + b, 0) / v.length)}ms p95=${ms(percentile(v, 0.95))}ms max=${ms(Math.max(0, ...v))}ms`;

// The line for one window.
export function summary(seconds: number, render: number[], dispatch: number[], rebuilds: number, drawn: number[]): string {
  const mean = drawn.length === 0 ? 0 : drawn.reduce((a, b) => a + b, 0) / drawn.length;
  return `perf window=${seconds.toFixed(1)}s ${stat("render", render)} | ${stat("dispatch", dispatch)} | layout_rebuilds=${rebuilds} | drawn mean=${mean.toFixed(1)} max=${Math.max(0, ...drawn)}`;
}

// The window rolls over when an event arrives after it ended, so an idle panel writes nothing.
export function makeRecorder(now: () => number, emit: (line: string) => void, windowMs = WINDOW_MS): Recorder {
  let began = now();
  let renders: number[] = [];
  let dispatches: number[] = [];
  let drawn: number[] = [];
  let rebuilds = 0;
  const flush = (): void => {
    if (renders.length + dispatches.length + rebuilds > 0) emit(summary((now() - began) / 1000, renders, dispatches, rebuilds, drawn));
    began = now();
    renders = [];
    dispatches = [];
    drawn = [];
    rebuilds = 0;
  };
  const tick = (): void => {
    if (now() - began >= windowMs) flush();
  };
  return {
    on: true,
    start: now,
    render(from, count) {
      renders.push(now() - from);
      drawn.push(count);
      tick();
    },
    dispatch(from) {
      dispatches.push(now() - from);
      tick();
    },
    rebuild() {
      rebuilds++;
    },
    flush,
  };
}

// The file keeps its last LOG_LINES lines.
export const appendCapped = (old: string | null, line: string, max = LOG_LINES): string => `${[...(old ?? "").split("\n").filter((l) => l !== ""), line].slice(-max).join("\n")}\n`;

// Appends through writeTextFile, which replaces a file only when `expected` is what it holds, as savePanel does. Lines are written one
// after another, and a refused or failed write drops the line: a measurement must not disturb the panel.
export function fileSink(host: Host, stamp: () => string = () => new Date().toISOString()): (line: string) => void {
  const file = `${host.appHome()}/data/herdr-review/perf.log`;
  let tail: Promise<unknown> = Promise.resolve();
  return (line) => {
    tail = tail.then(async () => {
      for (let i = 0; i < 3; i++) {
        try {
          const old = await host.readTextFile(file);
          await host.writeTextFile(file, appendCapped(old, `${stamp()} ${line}`), old);
          return;
        } catch {
          // read or write refused: read again
        }
      }
    });
  };
}

const clock = (): number => (typeof performance !== "undefined" ? performance.now() : Date.now());

// On only for `perf: true` in the mod's props.
export function recorderFor(host: Host, now: () => number = clock): Recorder {
  const props = host.props();
  if (typeof props !== "object" || props === null || (props as Record<string, unknown>).perf !== true) return NOOP;
  return makeRecorder(now, fileSink(host));
}
