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

**Scrolling a list.** `gpui-kit.d.ts` has `VirtualListScrollHandle` with `scroll_to_item(index, "top" | "center")`,
and `track_scroll(handle)` is documented for a virtual list. The diff is a `list`. `herdr-app check` on a build
that called `track_scroll` on the `list` printed `WARN gpui_shell::materialize::components::list: track_scroll is
ignored on a list: its scroll position is GPUI's own, filed under the id it was built with, which is where a
Scrollbar of that name finds it`. No other call in the declarations scrolls a list or a scroll area from code
(searched `scroll_to`, `scroll_by`, `ScrollHandle`, `reveal`). `v_virtual_list` and `uniform_list` do take the
handle but need each item's height, which a card of wrapped text does not have. So the panel starts the list a
few lines above the target instead, as a new list (new id), which begins at the top (gui.md). That a list built
under a new id begins at its top is what the warning's text says, and is not verified in the window.

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

## Still unknown

1. That the staged binary survives an app restart (documented, not run).
2. (Settled) The file list scrolls by wheel and draws a scrollbar, checked with a synthetic wheel event over the list: the last file, `tests/state_readonly.rs`, was reached.

3. That a click on the panel gives the root's `nav` handle the keyboard, that `shift+n` arrives as `shift-n` or
   `N`, that a `herdr-review.next-comment` command reaches the copy, and that a list built under a new id starts
   at the top (section 8).

The user starts herdr-app for these checks (decided 2026-11-09). No scratch instance.
