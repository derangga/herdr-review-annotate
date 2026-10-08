# The config file: keys, theme and sidebar

Part of the [herdr-review design](../DESIGN.md).

## Keymap file

The user can change any key in the table. The file is `config.toml` in the plugin config directory
(`HERDR_PLUGIN_CONFIG_DIR`, printed by `herdr plugin config-dir review`). It holds `[keys]`, `[theme]` and
`[sidebar]`.

```toml
[keys]
send = "ctrl+s"                 # one key
next_hunk = ["]", "ctrl+n"]     # several keys
switch_spec = ""                # unbound
```

- Key spelling follows Herdr's config: `ctrl+`, `shift+`, `alt+` and named keys such as `enter`, `tab`,
  `pageup`. `shift+s` and `S` mean the same key.
- An action that is not listed keeps its default keys.
- When a configured key is also another action's default, the configured binding wins and the other
  action loses that key. The pane shows one warning line that names each action left with no key.
- Two configured actions on one key: the earlier in the table above keeps it, and an action left with no key is named in the same warning line. An action whose listed keys all fail to parse keeps its defaults. `[]` unbinds like `""`.
- An unknown action name or a key that does not parse is skipped with a warning. A file that is not
  valid TOML is ignored as a whole with a warning. The pane always starts.
- The file is read once when the pane starts. The footer and the `help` overlay are drawn from the
  effective keymap, never from hard-coded text.
- Keys inside the comment editor (cursor movement, save, cancel) are not in `[keys]`. They stay as
  `edit_keys.rs` defines them. The same goes for the keys of the sidebar's query (`enter`, `esc`,
  `backspace`, `ctrl+u`). Only the key that opens it, `filter`, is in the table.
- The agent CLI, `open` and `send` do not read the file.

The same file chooses the pane's colours:

```toml
[theme]
name = "catppuccin-latte"
```

- The names are `catppuccin-mocha` (the default), `catppuccin-macchiato`, `catppuccin-frappe` and
  `catppuccin-latte`. A `config.toml` with no `[theme]`, or a `[theme]` with no `name`, is mocha.
- A name that is not one of the four, a `name` that is not a string, and a `theme` that is not a table are
  each one `Warning::Config`, and the pane starts in mocha.
- The table is read once when the pane starts, with `[keys]`.
- `theme.rs` is the only module that names a colour. Every other module draws with a role of `Theme`: base,
  text, subtle text, border, accent, agent, cursor, selection, visual, added, removed, their two tints, their
  two changed-word backgrounds, filler, header, popup, warning and success. The tint behind an added or a
  removed row is the flavor's green or red mixed 15 parts in a hundred into its base, and the background of a
  changed word is the same mix at 35 parts, so both follow the flavor.
- The pane paints the theme's base behind everything and its text colour on unstyled text, so it does not
  show the terminal's own background. The colours are 24-bit. A terminal without truecolor is not handled.

The same file says whether the sidebar starts open, and whether its file rows show an icon:

```toml
[sidebar]
open = false
icons = false
```

- `open` is `true` or `false`. A `config.toml` with no `[sidebar]`, or a `[sidebar]` with no `open`, starts
  with the sidebar shown.
- `icons` is `true` or `false` and is on when missing. The glyphs are Nerd Font glyphs, and the pane cannot
  tell whether the terminal's font has them, so a user without one sets `icons = false`.
- An `open` that is not a boolean, an `icons` that is not a boolean and a `sidebar` that is not a table are
  each one `Warning::Config`. A bad `open` starts the sidebar shown, a bad `icons` shows icons, and each
  falls back alone: a bad `icons` does not reset a good `open`, and the reverse. A bad `sidebar` table is one
  warning and both defaults.
- The table is read once when the pane starts, with `[keys]` and `[theme]`. `toggle_sidebar` flips the
  state for the session and writes nothing, so the next start takes the file's value again.
