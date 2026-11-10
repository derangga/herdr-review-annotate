---
status: accepted
---

# The GUI is a herdr-app mod over the CLI, written in TypeScript, kept in this repo

The user wants the review as a native panel in herdr-app, drawn like its Changes panel: click a line, drag
over a range, write a comment, send. herdr-app mods are TypeScript modules that draw GPUI elements and reach
the machine through a small API (`exec`, `watchFolder`, `writeTextFile`). So the panel is a TypeScript mod,
and it does no review work of its own. For every read and write it runs the `herdr-review` binary and
draws what comes back. The diff, the anchors, placement, the event log, the fold, the prompt and the choice
of target agent stay in Rust, once, and the TUI and the panel show one review because both go through that
code. The mod lives in `herdr-app-mod/` in this repository.

## Considered options

- **Extend the TUI.** A ratatui pane in a terminal cannot be a herdr-app panel. It cannot draw the native
  list and scrollbar, take a mouse drag with line numbers, or sit in the app's layout beside the agent. The
  TUI stays for plain Herdr, where the plugin still runs. Rejected for the panel, kept for what it is.
- **A separate repository for the mod.** The JSON the mod reads and the commands it calls change with the
  Rust code. In two repositories a change to a field is two commits that must land together, and the binary
  that the mod stages next to itself would be fetched from a release that may not have the field yet. In one
  repository one commit changes the contract and its decoder, and `stage-app-mod.sh` stages the binary it
  built. The cost is a second language in the tree, set out below.
- **Write the review logic again in TypeScript.** The mod could parse `git diff` and fold `review.jsonl`
  itself. Rejected: that is the diff parser, the anchor matching and the fold (ADR 0007, ADR 0008), written a
  second time, and the two would drift. Two front ends over one binary cannot disagree.
- **Effect for the mod's errors and requirements.** The design method maps onto Effect, and an Effect mod
  would have `Effect.gen` and layers where the plain version has `async` functions and a `Host` parameter.
  Tried and rejected: `import { Schema } from "effect"` fails to load in the mod runtime with `TextEncoder is
  not defined`, which means a polyfill, and the rest of the library adds 210 KB to a mod that reloads on
  every save. `Effect.runPromise` around `exec` could not be tested without a window (herdr-app-facts,
  section 2). The design (`design/gui.md`) is a straight-line `async` handler, a `withRecovery` wrapper and a
  `Host` parameter, which are the same three ideas without the library. Effect can come back if the runtime
  grows the missing global.

## Consequences

- The repo has two toolchains. Rust is checked by `cargo clippy` and `cargo test`. The mod is checked by
  `node --test`, an esbuild bundle in CI, and a typecheck that runs only on a machine where herdr-app has
  written `gpui-kit.d.ts`. CI cannot hold that file, and the user decided to keep it out of the repo.
  Dev dependencies are `esbuild`, `typescript` and `@types/node`, with no runtime dependency.
- The Rust binary gains a second surface that people do not read: `state` and `file` (JSON with an integer
  `schema`), `--as user`, body files, `comment edit`, `comment delete`, `comment seen`, `archive`, `view`,
  and `send --json`. A decoder in TypeScript copies the JSON shapes. Golden files shared by a Rust test and
  a TypeScript test hold the two together. A change a decoder would notice raises `schema`, and a reader of
  another number stops with a banner and not with a wrong screen.
- `state` and `file` write nothing, not even a `seen` event or `meta.json`, because the panel watches the
  repo root and the store directory, and a read that wrote would wake it again without end.
- A load is two `git diff` runs, one for `state` and one for `file`. A review is small, so it is accepted. A
  cache keyed by the compared revision is the answer if it is not.
- The panel runs outside any terminal, and herdr-app terminals are hc terminals. `send` therefore gets an hc
  backend next to the herdr one, behind the same three operations (get, list, prompt). The hc address is a
  different kind of id from a pane and a terminal id, so `Target` has a variant for each, stored under
  separate keys in `meta.json`. A v0.5.0 binary reads the file it always did and ignores the new key.
- The address is assumed not to survive a restart of herdr-core, since the manuals do not say it does. A
  stored address that fails the check falls back to the working directory, as ADR 0006 does for the Herdr
  terminal id. If it turns out to survive, nothing changes but the fallback is used less.
- ADR 0005 has the agent find the binary through `$HERDR_PLUGIN_ROOT`, which herdr-app does not set. Under hc
  the prompt carries the path of the binary that sent it (`current_exe`), so the agent replies with a binary
  that has the same commands, and not the older one on `PATH`. The nix setup pins v0.5.0 on `PATH`, so
  `stage-app-mod.sh` puts the binary it built in the mod folder, and the mod looks there before `PATH`.
- `--as user` and the user commands are labels. The CLI cannot tell a person from an agent, as nothing can
  that writes `review.jsonl` by hand. The agent's prompt names none of them, and the rights rule of ADR 0004
  holds in the fold, not in the caller. This is the existing trust model and not a new one.
- A comment body goes to the CLI in a file under `appHome()/data/`, never on argv, and the CLI deletes the
  file once it has opened it. The mode `writeTextFile` gives that file is not known yet. If it is 644, other
  users of the machine can read a body during the milliseconds before the CLI deletes it. Review comments are
  not secrets, but the user may want to know.
- The Herdr plugin is unchanged. `herdr-plugin.toml` does not name `herdr-app-mod/`, and the plugin install
  copies its source files and nothing else.
- Five things about herdr-app need a running window and were not checked (herdr-app-facts, last section).
  `design/gui.md` takes the safe default for each and says what changes if it is wrong.
