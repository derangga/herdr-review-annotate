#!/usr/bin/env bash
# Builds the herdr-app mod (build-dist.sh) and the release binary, and copies both to the mod's own folder:
# ${HERDR_APP_HOME:-~/.config/herdr-app}/mods/herdr-review (the binary under bin/, the file-type and button drawings under icons/), and the
# herdr-review layout to layouts/. Nothing else under
# the app folder is touched. Saving any file in the mod folder reloads the mod, so main.js goes last.
set -euo pipefail

cd "$(dirname "$0")/.."
dest="${HERDR_APP_HOME:-$HOME/.config/herdr-app}/mods/herdr-review"

# The mod folder comes from herdr-app-mod/dist, the one an agent installs, so a staged copy and an installed one match.
bash scripts/build-dist.sh
src=herdr-app-mod/dist/mods/herdr-review
cargo build --release

mkdir -p "$dest/bin"
# A copy next to the old one and a rename, so a running panel never execs a half-written file.
cp target/release/herdr-review "$dest/bin/herdr-review.new"
mv "$dest/bin/herdr-review.new" "$dest/bin/herdr-review"
mkdir -p "$dest/icons"
cp "$src"/icons/*.svg "$src"/icons/LICENSE-* "$dest/icons/"
cp "$src/mod.json" "$src/VERSION" "$src/fetch-binary.sh" "$dest/"
cp "$src/main.js" "$dest/main.js"
# The layout with the Review button in the tab bar: Herdr's main with one more block. Ours, so it is replaced.
cp herdr-app-mod/layouts/herdr-review.json "$dest/../../layouts/herdr-review.json"

echo "staged in $dest"
echo "check it:  herdr-app check $dest"
echo "the Review button in the tab bar:  herdr-app do layout.switch '{\"name\":\"herdr-review\"}'"
echo "to open the panel, add this under \"global\" in ${HERDR_APP_HOME:-~/.config/herdr-app}/keys.json (this script never edits it):"
echo '    "cmd-shift-r": {"command": "pane.split", "args": {"direction": "right", "mod": "herdr-review"}}'
echo "the panel runs bin/herdr-review from this folder, then herdr-review on PATH. If the app cannot exec the"
echo "staged copy and PATH has no herdr-review, the panel shows a banner that says so (props.bin overrides)."
