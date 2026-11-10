# Call graphs: the GUI opens

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). Each `E:` line is
one test, and each test swaps only `Host`.

## Open the panel (`init`)

```
-> pane.split with mod "herdr-review"                  herdr-app, not the mod
-> init(props, cx)                                     once per copy
  -> decodeProps(props())                              R: Host.props      E: a bad field -> escape, field dropped
  -> choose root                                       R: Host.focused, Host.panes
       props().root, else focused().cwd, else the cwd of a pane in the same tab
                                                       E: none found -> escape: banner "open the panel from a pane
                                                          that has a folder", no load, no watch
  -> choose target                                     (pure)
       props().target, else focused().terminal when focused().agent is set, else null
                                                       E: none -> escape: target null, send resolves by folder
  -> read the panel settings                           Cmd.ReadPanel   R: Host.readTextFile
       panel.json under appHome()/data/herdr-review -> decodePanel -> Msg.PanelLoaded { side, open }
                                                       E: file missing, unreadable or not JSON -> escape: left, shown
                                                       E: a field not an allowed value -> escape: that field's default
  -> start the timer, the root watch                   R: cx.timer, Host.watchFolder   only when a root was found
  -> the key loop (Host.nextCommand) starts with the first key the mod handles: the editor work
  -> first load (gui-load.md)                          Msg.Triggered
```

`herdr-app check` loads the module, runs `init`, renders once and exits. A task started with `cx.spawn`
never finishes inside it, so `init` must render a banner or an empty panel without waiting for a load.

Cardinality: once per copy. Opening again is a new copy with its own state.

## Choose the binary (the first load)

```
-> candidates: props().bin, the staged copy, herdr-review on PATH      (pure)
-> exec candidate state --json --repo <root>           R: Host.exec
     E: BinaryMissing (exec rejected) -> next candidate
     E: no candidate left -> escape: banner "herdr-review not found"
     E: TooOld -> escape: banner "update herdr-review", the search stops
  -> the first that starts is kept on the instance
```

## Read the theme (once the binary is known)

```
-> the first Msg.Loaded of the copy               update: Cmd.ReadTheme, once per copy   (pure)
-> run Cmd.ReadTheme: exec theme --json           R: Host.exec (the kept binary), decodeTheme
     -> Msg.ThemeLoaded { colors, warnings }      colors null under "app"
                                                  E: warnings not empty -> the status line shows them joined, once: sticky, so a reload keeps it, and a
                                                     result already in the status line is not replaced
                                                  E: any failure (TooOld, ContractMismatch, BackendFailed, a
                                                     timeout) -> escape: Msg.ThemeLoaded { colors: null }, one
                                                     status "could not read the plugin theme: ...", no banner,
                                                     no retry
-> view: t = { ...Host.theme(), ...colors }       on every render, so a herdr-app theme switch still reaches
                                                  the names colors leaves out
```

Until `ThemeLoaded` arrives the panel draws in herdr-app's theme, so with `gui = "plugin"` the first frames
are herdr-app's colours. The theme is read once: an edit to `config.toml` shows in a new copy of the panel, as
it shows in a new pane.

## Show, hide or move the sidebar

```
-> SidebarToggled | SidebarFlipped                     user action, a header button
  -> update: panel replaced, the view changes at once  (pure)  Cmd.SavePanel { panel }
-> run Cmd.SavePanel
  -> Host.readTextFile(panel.json)                     the text now there, or null
  -> Host.writeTextFile(panel.json, text, expected)    expected is what was read
                                                       E: refused or unreadable -> read again, three tries
                                                       E: still failing -> Msg.Failed op panel -> status line
                                                          "could not remember the sidebar setting", view kept
```

## Filter the file list

```
-> key "/" on the panel root                           panelKey: not in the editor, a question or the picker
  -> FilterOpened -> update: sidebar hidden? panel.open = true, Cmd.SavePanel   (pure)
  -> main: filter.focus()                              again after 300 ms
-> the input's focus | blur event -> FilterFocused { on }   filtering, so n and N type
-> the input's change event -> FilterChanged { text }       filter; rows.sidebarRows(files, filter) and
                                                            rows.filterInfo draw the list, the count, "no match"
-> the input's submit event -> FilterEntered
     E: empty query                                    -> filtering off, keyboard to the root
     E: no file matches                                -> nothing
     E: a match, the file shown matches                -> filtering off, keyboard to the root
     E: a match, the file shown does not               -> FileChosen { first match }, as a click on it
-> key "escape" in the box, or on the root with a filter on -> FilterCleared
                                                            filter "", filtering off, input emptied, keyboard to the root
```

## Close


```
-> close(cx)                                           bounded, returns at once
  -> set closed; release every textarea state          scope
  -> the watch loops end after their next wake
```

## Tests

Each `E:` line is one test. Also: the happy path (root from `focused()`, target set), and `init` against a
`FakeHost` whose `focused()` returns null renders the banner and throws nothing. For the theme: a second
`Loaded` sends no second `ReadTheme`, and the merge keeps a `Host.theme()` name that `colors` lacks.
