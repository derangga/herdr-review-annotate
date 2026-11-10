# What the design assumes about herdr-app and hc

Part of the [herdr-review design](../DESIGN.md).

Checked on 2026-11-09 with herdr-app 0.1.14, hc/herdr-core from `~/.local/bin`, Node 22.21.1, esbuild 0.28.2,
TypeScript 7.0.2, effect 4.1.2, macOS arm64. Spike mods lived in `~/.config/herdr-app/mods/spike-*` and
are deleted. Scratch builds were in the session scratchpad, outside the repo.

**One limit on everything below, until the window checks.** The first spikes ran with no herdr-app window. The
window checks of 2026-11-09 are in section 7, and the items marked unknown in sections 3, 5 and 6 are
answered there.

No herdr-app window was running during the first spikes. `herdr-app status`
answered `no Herdr window at .../herdr-app.sock: Connection refused`, and `ps` showed only the plain
`herdr` TUI, `herdr server` and herdr-core. `herdr-app check` works without a window, `hc` works without
a window, but anything that needs a running mod does not. `check` loads the module, runs `init`, renders
once and exits. A task started with `cx.spawn` never finishes inside it: an `exec` in a `spawn` wrote
nothing, two seconds after `check` returned. So what needs a live copy is marked **unknown** below and is
collected in section 7. It was not worked around by launching the window, because that opens a
window on the user's screen.

## 1. Toolchain: esbuild and the test runner

Ran, in a scratch folder with `npm i esbuild effect typescript @types/node`:

```
esbuild src/main.ts --bundle --format=esm --target=es2022 \
  --external:gpui-kit --external:gpui-base --external:herdr --outfile=out/main.js
herdr-app check ~/.config/herdr-app/mods/spike-toolchain      # main.js + {"place":"none"} mod.json
```

Saw: `check passed`, exit 0. `src/main.ts` imported `View, div` from `gpui-kit`, `v_flex` from `gpui-base`,
`theme, props` from `herdr`, and a sibling `./rows.ts` with `import { label, type Row } from "./rows.ts"`.
The bundle is 515 bytes and ends in `export { Spike as default }`. Two controls: a mod importing a name
`herdr` does not export (`nope`), and a `render` that throws, both gave a message, `check failed` and
exit 1. So `check` fails on a bad import and a render error, and the pass above means something.

- **Yes, esbuild bundles and the result loads.** `--format=esm` is needed (the mod is an ES module with a
  default export). Class fields compile to `rows;` (a field declaration). That is harmless: a field
  declared `rows!: Row[]` and one initialised `r = {}` both worked, because `init` runs after the
  constructor.
- `check` writes `gpui-kit.d.ts` (793 KB) and `jsconfig.json` into the mod folder. The staged folder will
  hold these two files. Do not copy them back into the repo (`.gitignore` them). Saving any other file in
  the folder reloads the mod, so `stage-app-mod.sh` writes `main.js` last.
- **Typecheck.** `tsc` 7.0.2 (`npx tsc -p .`, 0.2 s) typechecks `src` and `test` when `files` names that
  `gpui-kit.d.ts`. **Correction (2026-11-09, when the mod was built):** it declares `gpui-kit`, `gpui-base`
  and some Node-like modules (`path`, `url`, `process`, `console`, `fs/promises` ...), but not `herdr`. That
  module is `~/.config/herdr-app/herdr.d.ts`, reached with `paths`. With `@types/node` also loaded the two
  collide (`process`, `URL`, `console` declared twice), so `skipLibCheck` is on. The path to the files is
  machine-specific, so the repo cannot hold it in `tsconfig.json`. Both are copied to `herdr-app-mod/`
  (gitignored) by `npm run setup-types`, as the header of `gpui-kit.d.ts` advises.
- `strictNullChecks: true` works with fields declared `rows!: Row[]` or initialised. Costs found:
  1. `theme()` returns `Record<string, string>` but `.bg()` and `.text_color()` take `` `#${string}` ``.
     Under the shipped config (`strictNullChecks` off) this fails the same way. `host.ts` or `view.ts`
     needs `theme() as Record<string, `#${string}`>` once.
  2. Tests import `node:test`, so `tsc` over `test/` needs `@types/node` and `"types": ["node"]`. That
     adds the Node globals (`setTimeout`) to `src` in the typecheck. Accepted, or keep tests in a second
     tsconfig.
- **CI.** The `.d.ts` exists only where herdr-app has run. A GitHub runner has none, so CI cannot run the
  typecheck as the plan assumed. CI can run the tests and the esbuild bundle. The plan's CI bullet is
  edited to say so. Decided by the user (2026-11-09): the typecheck runs locally before a bead closes,
  and the declarations stay out of the repo.
- **Test runner: `node --test`, no extra dependency.** Node 22.21.1 runs `.ts` with type stripping on by
  default (since 22.18), so `node --test 'test/*.test.ts'` passed, 1 test. `--experimental-strip-types`
  is also accepted on 22.21 and printed no warning, so it can stay in the script for Node 22.6 to 22.17.
  `node --test test/` with a directory argument fails (`Cannot find module .../test`), use the glob.
  `"type": "module"` in `package.json` avoids a reparse warning. Strip-only mode rejects enums and
  parameter properties, and needs `import type` or `type` modifiers and the `.ts` extension in imports.
  `tsx` is not needed. `ci.yml` uses Node 22.

## 2. Effect: no

Ran a bundle with `import { Effect } from "effect"` (210 KB) under `check`:

- Loads, passes `check`. `Effect.runSync(Effect.gen(...))` returned 42 and `Effect.runFork(Effect.sync(...))`
  ran, both inside `init`, read back through a thrown render error.
- `import { Schema } from "effect"` fails at module load: `TextEncoder is not defined`. That is a missing
  Web API. It would need a polyfill, so the plan's rule applies: do not use it. `decode.ts` is a
  hand-written check.
- `Effect.runPromise` with `Effect.sleep` and with `exec` inside `cx.spawn`: **unknown**. `check` renders
  once and exits before any task finishes, so neither result can be seen. In the same run `typeof
  setTimeout` was `"function"` and `typeof setImmediate`, `MessageChannel` were `undefined`. Effect's
  scheduler picks among those, so whether `runPromise` settles here is a live-window question.

Answer: **do not use Effect.** The part that fails is certain (Schema), the part the design needs most
(`runPromise` around `exec`) is unproven, and the design is plain `async` plus `withRecovery`. It adds
210 KB to a mod that reloads on every save. The plan already says the design works in plain TypeScript.

## 3. Open path: one object key, `pane.split` with `mod`

`keys.json` accepts an object action `{"command": "...", "args": {...}}` and `args` may be nested
(AGENTS.md, Keys). `herdr-app commands` lists `pane.split` args `direction` (required), `pane`, `argv`,
`mod`, `props`. So one key opens the mod in a split with no launcher:

```json
"cmd-shift-r": {"command": "pane.split", "args": {"direction": "right", "mod": "herdr-review"}}
```

Add it under `"global"` in `~/.config/herdr-app/keys.json` (shipped `cmd-d` is also global and splits from a
terminal, so global keys act while a terminal has focus). `cmd-shift-r` is unused there. The file was not
edited.

Why this one:

- A launcher mod (`mod.open:name`, then `run("pane.split", ...)` and `closeOverlay`) is two mods and an
  overlay that flashes. It also cannot be a hidden mod: `<mod>.<command>` reaches shown copies only
  (`place: "none"` is not shown), so only `mod.open` can start it.
- `openOverlay({kind: "window"})` is a floating window over the layout, not a panel beside the agent, and
  a key cannot call it.
- A layout clone with the panel in its stack makes the user switch layouts and stop receiving updates to
  the shipped one.
- The object form cannot put the focused folder in `props`. The mod finds `root` and `target` itself from
  `focused()` and `panes()` at `init`, the way `mods/changes/main.js` does with `location()`.
  `props().root` and `props().target` stay as an optional override for tests and for a later key that
  carries them.

Answered in section 7: inside `init`, `focused()` names the pane that was split, with its `cwd`, so the
key works and the launcher is not needed. Also set
`"focusable": true` in `mod.json` so `pane.go` can reach the panel for typing a comment, as `mod.json`
rules in AGENTS.md describe.

## 4. hc facts, from a Claude Code terminal I started and removed

Ran `hc run --bg --name spike-cc --cwd <scratch dir> claude`, accepted the trust dialog with
`hc keys t1d down enter`, and used only that terminal (`t1d`). It is killed and removed. Claude Code ran
in auto mode, so I cycled it to manual mode with `hc keys t1d shift+tab` to get a permission prompt.

**`hc get t1d --json`**: `address, argv, cols, rows, created_ms, cwd, die_with_parent, exit_code,
finished_ms, foreground{argv,cwd,name,pid}, identity, keep, labels, name, notify, parent, pid, reason,
reported, rm, screen, signal, state, status, status_ms, template, title, turn`.

- `state`: `live` or `finished`. `status` is an object or null.
- Before the agent reports (trust dialog): `status: null`, `reported: false`, `title: ""`.
- Idle: `status: {"app":"claude-code","state":"idle"}`, `reported: true`, `title: "Claude Code"`.
- Blocked on a permission prompt: `status: {"app":"claude-code","kind":"permission","msg":"approve Bash: touch
  /tmp/...","state":"blocked"}`.
- Words seen: `idle`, `working`, `done`, `blocked`. The manual lists `error` too (not seen). `turn` went up by
  one per turn (0 to 4). `done` stays `done` in hc until the next turn. The "done becomes idle once
  looked at" rule is herdr-app's, not hc's.
- After `hc kill`: `state: "finished"`, `exit_code: 129`, `reason: "killed"`, `status: null`.

**`hc ls --json`**: `{"hidden_finished": N, "terminals": [...]}`. Each terminal has `address, agent, cwd,
name, state, turn` and, once an agent reports, `status` as a plain word (`"done"`). `agent` is `claude-code`
for Claude Code, `fish` for a shell, `cat` for a plain program. `cwd` is the launch folder. So `get` has the
status as an object and `ls` as a word: two shapes for the Rust decoder.

**`hc send`** (errors are JSON on stderr: `{"error":{"hint","kind","message"}}`, stdout is empty):

| State of the terminal | Command | Exit | Output |
|---|---|---|---|
| idle | `send --if idle t "..."` | 0 | stdout `{"started":true,"status":"idle","turn":0}`, the agent starts working |
| working | `send t "..."` | 0 | `{"started":true,"status":"working","turn":1}`. The text is typed into the agent's prompt box and read as a mid-turn message |
| working | `send --if working t "..."` | 0 | same |
| working | `send --if idle t "..."` | 10 | kind `status_mismatch`, "t1d is working, not idle", hint "nothing was typed" |
| blocked (permission) | `send t "..."`, and with `--if idle`, and with `--if working` | 9 | kind `blocked`, "t1d is blocked (permission)", hint "typing would answer its dialog; read its screen first (hc read t1d), or --force". Nothing typed |
| plain program, no status | `send t "..."` | 0 | `{"started":null,"status":null,"turn":0}` |
| plain program, no status | `send --if idle t "..."` | 10 | `status_mismatch`, "t1g is silent, not idle" |
| killed | `send t "..."` | 11 | kind `finished`, "t1d has finished" |
| removed, or no such | `send t "..."` and `get t` | 3 | kind `not_found`, "no terminal ..." |

`--force` was not run (it would answer the dialog). Mapping to `Refusal`: `Blocked` is exit 9, `AgentGone` is
exit 11 or 3, `NotReady` is `hc get` showing `status: null` or `reported: false`, or exit 10 under `--if idle`.
The existing design sends to a working agent and says "queued", which matches exit 0 with `status: working`.
A send while the agent was still booting (before `reported`) was not tried: the manual says it can be dropped.

**Env in an hc terminal** (`hc run -- sh -c 'env'`): `HC_TERMINAL_ID=t1f`, `HC_TERMINAL_SOCKET`, `HC_SCOPE`. No
`HERDR_*` variables. `PATH` there includes `/etc/profiles/per-user/sociolla/bin`, so `herdr-review` resolves, but it
has no fnm Node (a Claude Code hook in that terminal printed `node: command not found`).

**Does the address survive a core restart? Unknown.** I did not restart core. `hc --skill`: "`hc server
restart` ... programs keep running", and the schema says "Planned restart: screens are saved, programs keep
running, the next core adopts them". Neither says the address stays the same. "Adopts" suggests it, but it is not stated.
Decided by the user (2026-11-09): assume it does not survive. As in ADR 0006, a stored target that no
longer matches falls back to matching by `cwd`.

## 5. Temp files

`writeTextFile(path, text, expected)` is in `herdr.d.ts`: "writes only if it still matches `expected` (null
creates a new file)... existing files are replaced atomically", here only. `appHome()` is `~/.config/herdr-app`
(or `$HERDR_APP_HOME`). Not run: it is a promise, so `check` cannot show its effect (see the limit above).

What was seen without running it: every file the app itself writes under that folder (`view.json`,
`shared.json`, `installed.json`, and the `gpui-kit.d.ts` and `jsconfig.json` that `check` wrote) has mode
644 with the shell's umask 022. That suggests the same for `writeTextFile`, but it is inference. A body file
in that folder would be readable by other users on the machine until the CLI deletes it.

Answered in section 7: mode 644, and it creates missing parents (755). `fs/promises.mkdir` rejects a
`mode` option.

## 6. Binary

**`PATH` that `exec` sees from the Dock: documented, not observed.** The app's own help text (strings in
`/Applications/Herdr.app`) says: "Finder launches load the login shell's PATH, with the usual install folders
as a fallback", naming `~/.local/bin`, Homebrew, mise and Nix. The login shell here is fish (`dscl`). As a
stand-in I ran fish as a login shell with `PATH=/usr/bin:/bin:/usr/sbin:/sbin` and `env -i`, and it found
`herdr-review` at `/etc/profiles/per-user/sociolla/bin/herdr-review`. A bare `env -i sh` has
`PATH=/usr/gnu/bin:/usr/local/bin:/bin:/usr/bin:.` and does not. So whether `exec("herdr-review")` works from
a Dock launch depends on the app doing what its help says. **Unknown until a window is open.**

`herdr-review --version` prints `unknown command '--version'` for the nix binary, so "TooOld" detection is by
exit 2 on `state`, as the plan has it.

**A file in the mod folder (`mods/herdr-review/bin/herdr-review`): documented to stay, not run.** AGENTS.md
says shipped mods are replaced at start and `installed.json` tracks only their files. A folder with another
name is the user's. Saving any file in it reloads the mod, hidden files and the app's own are excluded.
Not tested: that `exec` can run an absolute path there, and that the execute bit survives. **Unknown.** A
cargo-built arm64 binary is ad-hoc signed by the linker, so Gatekeeper should not block a copy, but that is
also untested.

Order stays as the plan has it: `props().bin`, the staged copy, `PATH`. The window checks below confirm the
order matters: `PATH` here finds an older `herdr-review` that answers `state` with exit 2.

## 7. Window checks (2026-11-09, herdr-app window opened from the Dock, 0.1.14)

Run in the user's window, with spike mods `spike-all` and `spike-watch` (place `none`, opened with
`herdr-app do pane.split '{"direction":"right","mod":"..."}'`, the command the key would run, with no `pane`
argument so the focused pane is the one split). The spikes wrote their findings to a file with
`fs/promises.writeFile`, so the output did not depend on the thing under test. They and their test files are
deleted. The focused pane was a fish terminal whose folder is this repo (uncommitted changes).

**1. `focused()` inside `init` of the split-opened mod.** It names the pane that was split, not the new one:
`terminal: "t19"`, `pane: "pc1e..."` (the fish pane), `cwd` the repo, `agent: null` (a plain shell), and
`panes()` listed that one pane with `focused: true`. One second later `focused()` already named the new mod
pane (`terminal: null`, `cwd: null`). So the root must be read synchronously in `init`, which `main.ts` does
(`open(realHost)` before anything async). The real panel opened this way found the repo root and loaded
the right files. The `mod.open` launcher is not needed. `props()` was `null` under this key.

**2. `exec` and `PATH`.** `exec(staged, ["state","--json","--repo",repo])` with the absolute path
`~/.config/herdr-app/mods/herdr-review/bin/herdr-review`: exit 0, schema 1 JSON. The execute bit survived the copy. `PATH` as `exec` sees it after the Dock launch (a login-shell PATH, fish):
`~/.local/bin:~/.nix-profile/bin:/etc/profiles/per-user/sociolla/bin:/run/current-system/sw/bin:`
`/nix/var/nix/profiles/default/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin`.
`exec("herdr-review", ...)` resolves, to `/etc/profiles/per-user/sociolla/bin/herdr-review`, the older nix
build: `state` gives exit 2 `unknown command 'state'`. So the bare name works as a lookup and is the wrong
version here, which is why the staged copy comes before it. `hc` is found at `~/.local/bin/hc`, `git` at the
nix profile, and `node` is not on that `PATH`.

**3. Files.**
- `writeTextFile(path, text, null)` on a new file: resolves, mode `-rw-r--r--` (644, umask 0022).
- A path whose parent folders do not exist (`data/spike-noparent/deep/x.txt`): resolves, and creates the
  missing folders as 755. A replacing write (expected matches) keeps 644.
- `mkdir(path, {recursive: true, mode: 0o700})` from `fs/promises`: rejects with
  ``unknown option `mode` for fs.mkdir(path, options); expected recursive``. So the design's call does not
  work. `mkdir(path, {recursive: true})` works (nested too, existing folder does not throw) and gives 755.
- What does give a 0700 folder: `exec("chmod", ["700", dir])` after the `mkdir` (mode `drwx------`), and
  `exec("mkdir", ["-p", "-m", "700", dir])` (also `drwx------`, exit 0).
- A file written by `writeTextFile` inside a 700 folder is still 644, so the folder carries the privacy.
- The app makes an empty `data/<mod>/` folder (755) for each mod that runs, `data/herdr-review/` included
  (it appeared when the panel first opened). So `mkdir` is not needed for existence, only `chmod 700`.
- `herdr-app check` is stricter than the window: an `init` that wrote with `fs/promises` outside the mod's own
  folders failed under `check` with "outside every granted write root; add its directory to
  capabilities.fs.write in the manifest". The same write worked in the window. `check` does not run `spawn`
  bodies, so a write inside one is not exercised by it.

**4. `Effect.runPromise`.** Not run. Effect is not used (section 2), so the question is moot.

**5. A staged binary and an app restart.** Not verified: the app was not restarted, by instruction.
AGENTS.md says shipped-mod replacement at start touches only the shipped mods' own files (`installed.json`),
and a folder with another name, `mods/herdr-review/`, is left alone. The binary has survived every save of
the mod folder so far, and `stage-app-mod.sh` ran again with the app running.

**6. The panel in the window (the skeleton).** `herdr-app do pane.split '{"direction":"right","mod":"herdr-review"}'`
opened the panel beside the fish pane. `herdr-app shot` showed the header (`Review`, `72 files`, `+10562 -229`),
the spec buttons (Working tree, Branch origin/HEAD, Refresh), the file list with status letters, directory
prefix dimmed, and `+n -n` counts, and the diff of the first file (`@@ -0,0 +1,376 @@`) with line numbers.
Creating `scratch-window-check.txt` (3 lines) with nobody pressing Refresh: the header became `73 files`,
`+10565`. Deleting it brought `72 files` back. `herdr-app status` listed no errors.

Two things about `shot`:
- **The last frame lags.** A shot taken 6 s and 9 s after the split still showed the closed spike pane (two
  byte-identical shots, after a visible change), and the panel appeared only in a shot 12 s later. After the
  file was created the new count was in the shot 16 s later but not at 4 s. After the delete it was in the
  shot 19 s later, not at 17 s. So the window draws a frame only now and then (the cause is not known: this window sat behind the
  terminal that drove the checks), and a shot needs about 20 s to show a change.
  Compare shots 20 s after a change, not 4 s.
- **`watchFolder` is prompt.** A spike that only looped on `watchFolder(repo root)` and logged wake times
  woke about 1 s after the file was created, and again after the delete, with the changed path. So the root
  watch works on a repo this size, and the lag above is the drawing, not the watch.

Shots are in the session scratchpad, not the repo.

## 8. Keys and scrolling for next and previous comment (2026-11-09)

**Keys.** Read, not run, because no key could be pressed (no synthetic input is allowed).
- `~/.config/herdr-app/AGENTS.md`, Keys section: an action is a command, `send:`, `<mod>.<command>` ("delivered to
  that mod's `nextCommand()`") or `terminal`. A `mod:<name>` section holds keys that apply to a mod. Sections
  are `global`, `layout:<name>`, `terminal`, `mod:<name>`.
- `herdr.d.ts`, `nextCommand`: "With `focusable: true` in mod.json, pane.go can move onto this copy and send
  `focus`. Focus your own handle then (`this.handle.focus()`) and handle keys with `on_key_down`." AGENTS.md,
  Mods: "The copy gets `focus` again after a popup over it closes." The pane section says a mod pane "keeps the
  keyboard when clicked".
- `gpui-kit.d.ts`, `on_key_down`: delivered "while this element or something inside it holds the keyboard",
  `track_focus(handle)` is half of the registration, and "without it the handler sits on an element the keyboard
  never reaches". So a key outside a focused element does not reach a mod. The mod's way in is a tracked handle
  that holds the keyboard.
- What the mod does with that: the root element tracks its own handle (`nav`) with an `on_key_down` that maps `n`
  and `shift-n`; the handle is focused on a `focus` command, after a jump, and after the editor or a question
  closes. A click on the pane gives it the keyboard (documented); whether that click lands on `nav` (so that `n`
  works with no other step) is **not verified**: the window was not redrawing and no click can be made from
  here. The keystroke spelling of shift+n (`shift-n`, or `N`) is not verified either; the mod accepts both.
- Mod commands: `herdr-app do herdr-review.next-comment` answered `{"ok": true}`. Whether the copy got it is not
  verified (see the window below). It is the route for a key that needs no click: a binding under `global` or
  `mod:herdr-review` in `keys.json`, which the user adds.

**Keys in the filter box.** The shipped `changes` mod (`mods/changes/main.js`) is the pattern: an `InputState` made in
`init` with `on("change" | "submit" | "focus" | "blur")`, `Input.new(state)` inside a frame that stops the mouse
press (`on_mouse_down("left", ...)` with `stop_propagation`), and an `on_key_down` on the root, tracked with its own
handle, that sees `escape` while the input has the keyboard, so a key typed in an input reaches an ancestor's
handler. The filter box copies it. That Esc and Enter behave so in this mod, and that `/` arrives as `"/"`, are
**not verified**: typing needs a person (no synthetic input is allowed).

**Scrolling a list.** A `list` cannot be scrolled from code: `herdr-app check` on a build that called `track_scroll` on the
`list` printed `WARN ... track_scroll is ignored on a list: its scroll position is GPUI's own`. `v_virtual_list` takes the
handle, so the diff became one (section 10), and later the mod scrolled it itself (section 12). The workaround this paragraph used to describe (a new list id that starts at
the top, and a first row that shows the rest of the file) is gone.

**Window.** The window did not redraw: with the panel open in a throwaway tab, a shot after 25 s, one after
55 s, one after another 30 s and 20 s were all byte-identical (md5 `f077046c...`), and a `tab.rename` of that tab
(which `herdr-app ls` accepted) did not show in any of them. So no shot shows a jump. The first shot, taken
before any command, shows the two new buttons in the bar (`Previous comment`, `Next comment`) ahead of `Send`,
`Archive` and `Mark viewed`.

## 9. File-type icons: glyphs or SVGs (2026-11-09)

Spike: a throwaway mod drew the TUI's glyphs (`\ue68b \ue6b2 \ue672 \uf48a \ue60b \ue795 \ue628 \uf016`, private use
area) in four rows, opened in a throwaway tab, and a shot was read (`scratchpad/g2.png`, the tab was `zz-check`, so
the frame was fresh).
- The default UI font draws every one as a boxed question mark (the missing glyph).
- `div().font_family("CaskaydiaCove Nerd Font")` draws them all (also with the name `CaskaydiaCove NF`). `Menlo`
  does not. `herdr.fonts()` lists the machine's families (205 here, with Nerd Font ones among them).
- So the glyphs need a Nerd Font installed on the user's machine, and the mod would have to find its name in
  `fonts()` and fall back to something else when there is none. The glyph route works only where that holds.

Decision: SVGs in the mod's own folder, which render whatever the fonts. `svg(path)` resolves against the app root,
not the mod file, as `gpui-kit.d.ts` says and the shipped `files` mod does (`svg("mods/files/file.svg")`), so the
panel uses `mods/herdr-review/icons/<name>.svg`. An SVG with `stroke="currentColor"` takes the text colour, so
`.text_color(t.dim)` tints it by the theme. `herdr-app check` passes with them staged and prints no WARN for a
missing file. The stage script copies the folder, and the mod folder name is the mod's name, so a copy of the mod
under another name would look for its icons in the wrong place.

Shot of the panel on this repo (`scratchpad/i2.png`): the icons show for Markdown (`.P.herdr-app-gui.md`,
`AGENTS.md`), `.gitignore` (git branch), YAML (`config.yaml`, `ci.yml`), JSON (`metadata.json`) and the generic
page (`issues.jsonl`), after the status letter and before the name. The shot was the only one that showed after
a change: three later shots of a second throwaway tab (a scratch repo with one file of each type) were
byte-identical to it (md5 `f2258ab1...`), still showing the earlier tab after 8 s, 23 s, 53 s and 113 s, so the window had
stopped redrawing. The rust, toml, lock, ts, js, shell, docker, make and license drawings were therefore not seen
in the window; they are plain strokes in the same style and the stage check loaded them.

## 10. A virtual list for the stream of every file (2026-11-09)

**No longer used.** The stream left `v_virtual_list` for a stream the mod scrolls itself (section 12), because a list row can report only a
click. This section is what the list spike found, kept for why it was left and for the findings that still hold (the import, what throws
inside a renderer).

Spike: a throwaway mod (`spike-vlist`, deleted) drew `v_virtual_list("spike-list", 200, sizes, key, render)` with one height per
item (20, 32, 44, 56, 68 in turn), inside the same `div().w_full().min_w(900).h_full()` in an `overflow_x_scrollbar()` box that the
panel uses, a `VirtualListScrollHandle` given by `track_scroll`, a `Scrollbar.vertical("spike-list").viewport_from_layout()` beside
it, and an `on_scroll_wheel` on the box. Its timer wrote what it saw to a file under `data/`. The window drew only now and then
(a frame after about three minutes, as in section 7), so only the first frames were seen. **Seen** means it ran in the window,
**not yet seen in the window** means it is read from `gpui-kit.d.ts` or only `herdr-app check` ran it, and the user is to confirm it in
their own panel.

- **The import.** Seen: `v_virtual_list` and `VirtualListScrollHandle` come from `gpui-base`, not `gpui-kit` (`list` and
  `uniform_list` are in `gpui-kit`). `check` fails on the wrong module with `Could not find export`.
- **One height per item.** The signature is `v_virtual_list(id, count, item_sizes, get_key, render(range))`, `item_sizes` a number
  or an array exactly `count` long. Seen: a list of 200 items with 200 sizes, then a list of 202 items with 202 sizes, drew.
  **Not yet seen in the window:** that each item takes exactly its stated height, and what a 100,000 number array costs on every
  render (the declarations warn that it crosses the language boundary each time). `check` does not run the renderer, so it shows
  neither.
- **The visible range.** Seen: `render(range)` is called twice for a frame. The first call is `{start: 0, end: 1}` (the one item it
  measures, the first by default, or the one `with_item_to_measure_index` names), the second is the items on screen, for example
  `[183, 202)`. Four calls in two frames were `[0,1] [183,202] [0,1] [183,202]`. The mod keeps the last two ranges and takes the
  wider. The range is the items the list builds, which may include a few outside the viewport.
- **Getting the range out.** Seen: the renderer may store the range in a plain field of the view (a field assignment throws
  nothing) and a `cx.timer.every(150, ...)` callback read it back, one tick after the frame. A timer callback may dispatch and call
  `cx.notify()` as any event handler. So the visible range reaches the model as a message sent from a timer, and the model
  does not notify for a scroll that changed nothing it draws.
- **What throws inside the renderer.** Seen, with these messages:
  - `cx.notify()`: ``TypeError: cx.notify() is not allowed during the `layout` phase; request a re-render from an event handler instead``.
  - `on_click` on an element built in it: ``TypeError: `on_click` cannot be registered from a virtual list's item renderer: the rows are rebuilt every frame ... Use `on_item_click((key, cx) => ...)` or `on_item_secondary_click(...)` ``. Other handlers, new state and
    `focus_handle()` are refused the same way, as the declarations say.
  - Not an error, seen: `handle.scroll_to_item(...)` called inside the renderer, and inside `render()`.
- **`scroll_to_item(index, "top" | "center")`.** Seen: it does not throw, from `render()`, from the renderer and from a timer. The
  one frame that followed a scroll to the last item (index 201 of 202) had the range `[183, 202)`: the end of the list stops the
  scroll short, as the TUI's stream does. **Not yet seen in the window:** that `scroll_to_item(i, "top")` puts item `i` first in the range
  (a frame after `scroll_to_item(60, "top")` in `render()` and `scroll_to_item(50, "top")` in the renderer showed `[33, 53)`,
  which neither explains, so the mod calls it once, from a handler or the timer, after the model changes, and never from
  the renderer); and where the scroll lands when the heights above the target changed in the same update (the
  declarations say the scroll is resolved before the next frame, so it should use the new heights).
- **The scroll position survives a changed size array.** Not yet seen. The mod depends on it when a file's placeholder is replaced by its
  diff: it asks for the item at the top of the screen again (`reveal`) whenever a file above the screen arrives.
- **The scrollbar and the horizontal scroll box.** `Scrollbar.vertical("<list id>").viewport_from_layout()` beside the list and the
  `overflow_x_scrollbar` box with `min_w(width)` around it are the shapes the old `list` had. `check` passes with them and prints no
  WARN (the `track_scroll is ignored on a list` line is gone). Not yet seen in the window: that the bar drives the list and that the
  box scrolls sideways with it.
- **A click.** `on_item_click(key)` reports the key `get_key(index)` gave, as for a `list`. Not yet seen on the stream.

## 11. Clicks, hover and drag on the stream's rows (2026-11-09)

**Superseded by section 12.** This is what a row of a virtual list could and could not do. The user's tests showed that the `[+]` coloured by
the row's hover and the gutter test on a press did not do what the TUI does, and section 12 replaced the list. Nothing below is built
any more.

Read from `gpui-kit.d.ts` and `AGENTS.md`; `herdr-app check` ran the staged mod. No window was driven (no synthetic input is
allowed), so everything marked **not yet seen in the window** is for the user to confirm in their own panel.

- **What a row can report.** `on_item_click(key)` is the key and nothing else. `on_item_secondary_click(key, event)` gives the
  right-button press with `position`, and `local_position` and `bounds` against the row's own box, but it is the right button, and the
  mod has no use for a context menu. `on_click` and every other handler throw inside the item renderer (seen, section 10).
- **A press on a box around the list.** `on_mouse_down(button, handler)` is allowed on an element outside the renderer. Its event
  has `position` (window), `local_position` and `bounds` (against that element, absent before the first paint), `click_count` and
  `modifiers` (`shift`, `control`, `alt`, `platform`). It fires on the press, before the click, and the declarations say the row's own
  handler and an outer one for the same button both run in bubble order. So the mod keeps the press's x and shift in a plain field and
  reads them in `on_item_click`. **Not yet seen in the window:** that a press on a list row reaches an outer `on_mouse_down` at all
  (a list row could stop it), that `local_position` is filled in, and that its x is against the box's moved position.
- **Which box.** The stream sits in `review-stream-x` (`overflow_x_scrollbar`, fixed in the pane) and inside it a `w_full().min_w(width)`
  content box that moves left as the code scrolls sideways. A local x against `review-stream-x` would be the pointer's place in the
  pane, so the gutter would slide away from x 0 on a sideways scroll. The mod puts the handler on the content box
  (`review-stream-content`), whose bounds move with the scroll, so the gutter stays at x 0. **Not yet seen in the window:** that
  GPUI gives the content box's moved bounds to the local position. Test: scroll sideways and click a `+`.
- **Hover.** `.hover(declare)` takes a callback that fills a detached element with styles ("Styles applied while the pointer is over the
  element"), so it is a style and not a handler, and the declarations list handlers, new state and focus handles as what a list
  renderer refuses. No `group_hover` exists, so a child cannot show when its parent is hovered. The mod uses what is there: the row has a
  hover style that changes the row's text colour, and the `+` is an `svg`, which is tinted with the text colour it inherits, so it is a
  filled button that follows the row (the rest of the row names its colours). **Seen in the window:** a `hover` inside the renderer does
  not throw and applies to the element under the pointer (the `+` filled when the pointer was on it). **Not yet seen:** that an `svg`
  inherits the text colour of a hovered parent. If it does not, the `+` shows only on the cursor's row.
- **Drag: not built.** A drag needs to know which row the pointer is over at each move. `on_mouse_down`, `on_mouse_move` ("delivered
  while this element is hovered") and `on_mouse_up` give y against the box, and `on_mouse_move` carries no button state, but nothing
  gives the list's scroll offset in pixels: `VirtualListScrollHandle` has `scroll_to_item`, `scroll_to_bottom` and `release`, and the
  renderer's range names items, not how far the first one is scrolled. Without the offset a y cannot be turned into a row, and the
  error is up to one row (20 px for code), which makes a range off by one at both ends. `on_item_click` cannot fill the gap: a drag
  across rows ends in no click, and a click names only the row it finished on. Wheel events (`on_scroll_wheel`) could be summed, but the
  scrollbar, `scroll_to_item` and a keyboard scroll move the list without one, so the sum would drift. There is also no auto-scroll
  while the pointer is outside, which the TUI has. So a range is a shift-click, which needs only the key and the modifiers.
- **Esc.** The keys reach the mod only along the focus path (section 8), so a click on a row asks `main.ts` to give the keyboard back to
  the panel's handle (`focusNav`), after which Esc is `rows.panelKey`'s `SelectionCleared`. **Not yet seen in the window:** that Esc arrives
  as `"escape"` on that handle after a click on a row.

## 12. A stream the mod scrolls itself (2026-11-09)

Spike: a throwaway mod (`zz-scroll-spike`, deleted) showed about 6,000 fake code rows of 20 px, with a fake card of 2 to 6 wrapped lines every
40 rows, in the shape the panel uses now: a pixel `offset` in the view's state changed by `on_scroll_wheel` on a box inside the
`overflow_x_scrollbar` box, a prefix sum of the heights, and only the items that overlap the screen built each render as plain children of an
`overflow_hidden` box, the first one at `top(-shift)` in an absolutely placed column. It had its own track and thumb on the right, a `[+]`
(`icons/plus.svg` tinted by `theme().accent`) on the row that `on_hover` and `on_mouse_move` named, a drag that `on_mouse_down` began and
`on_hover` extended with `on_mouse_up` on the box ending it, a status line with counters for the events, and a chip that drew the cards at a
fixed or at their natural height. Its source was in the session's scratch folder, not in the repo. `herdr-app check` passed with no WARN.
The user tried it in their own window and reported that it behaves as they expected: the scrolling, the `[+]` following the mouse and the
drag selection. They gave no numbers (no render times, no event counts), so those are not recorded here. **Seen** below means that report;
**not yet seen** means read from `gpui-kit.d.ts` or only `herdr-app check` ran it.

- **Handlers on ordinary elements work.** Seen: `on_hover`, `on_mouse_move`, `on_mouse_down`, `on_mouse_up` and `on_click` on elements built in
  `render()` each frame, with ids derived from the row, did what they should. Only a list item's renderer forbids them (section 10).
  The `[+]` is `icons/plus.svg` drawn by `svg(...)` with `text_color(theme().accent)`, shown only for the hovered row.
- **The mod's own offset scrolls smoothly enough.** Seen (reported as expected, with no numbers): wheel and trackpad scrolling moved the
  stream, with the offset from `ScrollWheelEvent.delta` (pixels; `delta_lines` times 20 px when the device reports lines), flipped in sign
  (content up is offset down), clamped to the content. The panel keeps that sign.
- **Hover and drag.** Seen: the `[+]` follows the mouse, a drag selects rows between the press and the pointer, the selected rows are tinted.
  The panel's row sends both `on_hover` and `on_mouse_move` and both are idempotent.
- **A vertical wheel over the sideways box.** Read from GPUI's behaviour and the declarations, **not separately seen**: an
  `overflow_x_scrollbar` box that has no vertical overflow takes a vertical wheel as a sideways scroll, and stops the event. So the wheel
  handler sits on the inner content (which gets the event first) and stops it unless the gesture is mostly sideways (`|dx| > |dy|`), which
  is left to the box. The spike worked with it; the panel does the same (`rows.wheelDelta`).
- **No button state in a move.** `MouseMoveEvent` carries `position`, `local_position`, `bounds` and `modifiers`, and no button, so a drag
  cannot be told to have ended from a move. The panel ends it on `on_mouse_up` on its root element (anywhere in the panel), and a press
  starts over. A release outside the window is **not seen** and is not handled beyond that.
- **The scrollbar.** The thumb drag (`on_mouse_move` on the stream box while a grip is set, from the press's window `position.y`) and the click
  on the track (`local_position.y`) were in the spike; the user's report does not single them out. **Not yet seen separately.**
- **Hover after a wheel tick with a still pointer.** The row under the pointer changes without a move. Whether GPUI fires `on_hover` then is
  **not seen**; the `[+]` is allowed to wait for the next move.
- **Cards.** The spike's chip drew cards at a fixed estimated height or a natural one; the user gave no preference. The panel draws every
  item at its counted height (a card's box is its estimate, overflow hidden), because the prefix sums, the scrollbar, a scroll to an item and
  the offset all use those numbers, and a natural height that differs from the estimate moves everything under it away from where the offset says it is.
- **Render time.** Not measured by the user. The panel renders only the items on screen (a few dozen elements) and keeps the stream's layout
  while nothing it is built from changes, so a wheel tick or a hover does not rebuild it. The time of a render in the real panel, and whether
  the 'interrupted' banner can still appear, are **not yet seen**.

## 13. Icon buttons (2026-11-10)

Read from `gpui-kit.d.ts` and `herdr-app check`; **not seen in the window**, so each line below is for the user to confirm in their own panel.
- `div().tooltip(text)` takes a string and shows it after half a second. `.accessibility_label(text)` names the element for a screen reader.
  `.active(declare)` styles the pressed element. All three type-check on a `div` and `check` loads the mod.
- The 16 button drawings (`panel-left panel-right swap columns refresh arrow-up arrow-down circle-check comment-plus send archive reply check
  undo pencil trash`) are in `icons/`, 24 px viewBox, stroke 2, round caps, and the stage script copies them with the file-type ones. `check`
  prints no WARN for them.
- Unknown, to see: whether `.active(...)` fires on a `div` with `on_click`; whether a tooltip appears over a disabled (opacity 0.4) `div`; whether
  the icon takes the hover colour set on its parent (the icon also has its own `text_color`, which may win); whether a 16 px icon at stroke 2 is
  too heavy (the fix is `stroke-width="1.75"` in the files). If `.active` does nothing, drop it; do not fake it with `mouse_down`.

## Still unknown

1. That the staged binary survives an app restart (documented, not run).
2. (Settled) The file list scrolls by wheel and draws a scrollbar, checked with a synthetic wheel event over the list: the last file, `tests/state_readonly.rs`, was reached.

3. That a click on the panel gives the root's `nav` handle the keyboard, that `shift+n` arrives as `shift-n` or
   `N`, that a `herdr-review.next-comment` command reaches the copy, and that a list built under a new id starts
   at the top (section 8).

4. The stream (section 12): that the sidebar's file changes as the offset changes, that a click in the sidebar and a jump put the item at the
   top, that the heights the mod gives (a card's is an estimate) leave no gap or overlap, that the offset holds when a file's diff replaces its
   placeholder (the item at the top is scrolled to again), that the thumb drags and the track click jumps, and that no 'could not be
   re-rendered' banner shows on a 30-file review.

5. The rows (section 12): that `on_hover` and `on_mouse_move` keep working in the real panel's rows, that the `[+]` on the hovered line opens the
   editor on the line or the range, that a plain click highlights nothing and clears a range, that a drag released outside the stream but inside the
   panel ends, and that Esc arrives after a press on a row (the panel gives the keyboard to its handle on a press).
