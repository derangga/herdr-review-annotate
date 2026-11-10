#!/usr/bin/env bash
# Builds the herdr-app mod and the release binary, and copies both to the mod's own folder:
# ${HERDR_APP_HOME:-~/.config/herdr-app}/mods/herdr-review (the binary under bin/, the file-type and button drawings under icons/). Nothing else under
# the app folder is touched. Saving any file in the mod folder reloads the mod, so main.js goes last.
set -euo pipefail

cd "$(dirname "$0")/.."
dest="${HERDR_APP_HOME:-$HOME/.config/herdr-app}/mods/herdr-review"

(
  cd herdr-app-mod
  [ -d node_modules ] || npm ci
  npm run build
)
cargo build --release

mkdir -p "$dest/bin"
# A copy next to the old one and a rename, so a running panel never execs a half-written file.
cp target/release/herdr-review "$dest/bin/herdr-review.new"
mv "$dest/bin/herdr-review.new" "$dest/bin/herdr-review"
mkdir -p "$dest/icons"
cp herdr-app-mod/icons/*.svg "$dest/icons/"
cp herdr-app-mod/mod.json "$dest/mod.json"
cp herdr-app-mod/out/main.js "$dest/main.js"

echo "staged in $dest"
echo "check it:  herdr-app check $dest"
echo "to open the panel, add this under \"global\" in ${HERDR_APP_HOME:-~/.config/herdr-app}/keys.json (this script never edits it):"
echo '    "cmd-shift-r": {"command": "pane.split", "args": {"direction": "right", "mod": "herdr-review"}}'
echo "the panel runs bin/herdr-review from this folder, then herdr-review on PATH. If the app cannot exec the"
echo "staged copy and PATH has no herdr-review, the panel shows a banner that says so (props.bin overrides)."
