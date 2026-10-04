---
status: accepted
---

# Agents find the binary by absolute path, taken from Herdr

The agent runs `herdr-review` from its own shell, which Herdr did not launch as a plugin command. We do
not add the binary to `PATH` and we do not create a symlink, because both change the user's
environment and both break when the plugin moves. On this machine the plugin root is a Nix store path
that changes with every version.

The agent gets the absolute path from two places. Every sent prompt contains it, shell-quoted. For a
review the agent starts itself, the skill reads `plugin_root` from
`herdr plugin list --plugin review --json` each time it needs the path.

## Considered options

- **A `[[startup]]` hook that writes the path to a file.** Rejected. Startup hooks run only after a
  server start or a live handoff, not when a plugin is installed, linked or enabled, so the file is
  missing until the persistent Herdr server restarts.
- **A generated skill with the path filled in.** Rejected. The path goes stale on every update.
- **`herdr plugin config-dir review`.** It prints a path and exits 0 for any id, installed or not, so
  it cannot tell the agent whether the plugin exists.
