// Models built the way the panel gets them: the state arrives, then the files the stream asks for, until nothing is
// left to ask. Only the tests use it.
import { decodeFile, decodeState } from "../src/decode.ts";
import { initModel, update } from "../src/update.ts";
import type { Cmd, FileView, Model, Msg, State } from "../src/types.ts";
import { fixture } from "./fake-host.ts";

export const fixtureState = (): State => decodeState(JSON.parse(fixture("state")));
export const fixtureFile = (): FileView => decodeFile(JSON.parse(fixture("file")));

// A file with nothing in it, standing for the files a test does not look at.
export const blank = (of: FileView, path: string): FileView => ({ ...of, path, old_path: null, rows: [], threads: [], note: null });

export function steps(m: Model, ...msgs: Msg[]): { model: Model; cmds: Cmd[] } {
  const cmds: Cmd[] = [];
  for (const msg of msgs) {
    const [next, out] = update(m, msg);
    m = next;
    cmds.push(...out);
  }
  return { model: m, cmds };
}

// Answers every fetch the model asks for, with `file` for its own path and a blank file for the others, until it asks
// for no more. The commands of all the steps are returned.
export function answer(m: Model, cmds: Cmd[], file: FileView = fixtureFile()): { model: Model; cmds: Cmd[] } {
  const all = [...cmds];
  for (let guard = 0; guard < 50; guard++) {
    const ask = all.slice(all.length - cmds.length).find((c) => c._tag === "LoadFiles");
    if (ask === undefined || ask._tag !== "LoadFiles") {
      break;
    }
    const out = steps(m, { _tag: "FilesLoaded", key: ask.key, files: ask.paths.map((p) => (p === file.path ? file : blank(file, p))) });
    m = out.model;
    cmds = out.cmds;
    all.push(...cmds);
  }
  return { model: m, cmds: all };
}

// The state arrives and the files it names are fetched. `file` is the one with comments (src/a.rs in the fixtures).
// A file that is not what the model holds has a new hash in the state, as the CLI's would be, so it is fetched again.
export function reload(m: Model, state: State, file: FileView = fixtureFile()): { model: Model; cmds: Cmd[] } {
  const held = m.files.get(file.path);
  const changed = held !== undefined && JSON.stringify(held) !== JSON.stringify(file);
  const next = changed ? { ...state, files: state.files.map((f) => (f.path === file.path ? { ...f, hash: `${f.hash}+` } : f)) } : state;
  const out = steps(m, { _tag: "Loaded", key: m.key, state: next });
  return answer(out.model, out.cmds, file);
}

// A panel that has opened, loaded src/a.rs and every other file, and has `shown` at the top of the stream.
export function loadedModel(state: State = fixtureState(), file: FileView = fixtureFile(), target: string | null = "t7", shown: string | null = file.path): Model {
  const open = steps(initModel(), { _tag: "Opened", root: "/repo", target });
  const load = open.cmds[0];
  if (load === undefined || load._tag !== "Load") {
    throw new Error("opening did not load");
  }
  const arrived = steps(open.model, { _tag: "Loaded", key: load.key, state });
  const done = answer(arrived.model, arrived.cmds, file);
  return shown === null ? done.model : steps(done.model, { _tag: "Scrolled", offset: 0, total: 0, top: `${shown}|file`, visible: [shown], fresh: [], revealed: false }).model;
}
