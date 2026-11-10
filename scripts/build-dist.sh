#!/usr/bin/env bash
# Builds the herdr-app mod into <out>/mods/herdr-review (default herdr-app-mod/dist), the folder an agent copies
# into ~/.config/herdr-app/mods. It is generated: main.js, mod.json, icons/, fetch-binary.sh and VERSION.
# keys.json, RECIPE.md and preview.png beside it are written by hand and left alone.
# release.sh runs it into the "Release" commit. ci.sh runs it into a temp folder and fails if that differs from the
# committed one.
set -euo pipefail

cd "$(dirname "$0")/.."
out="${1:-herdr-app-mod/dist}"
mod="$out/mods/herdr-review"

(
  cd herdr-app-mod
  [ -d node_modules ] || npm ci
  npm run build
)

rm -rf "$mod"
mkdir -p "$mod/icons"
cp herdr-app-mod/out/main.js herdr-app-mod/mod.json "$mod/"
cp herdr-app-mod/icons/*.svg "$mod/icons/"
cp scripts/fetch-herdr-review.sh "$mod/fetch-binary.sh"
sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1 > "$mod/VERSION"
[ -s "$mod/VERSION" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
echo "built $mod ($(cat "$mod/VERSION"))"
