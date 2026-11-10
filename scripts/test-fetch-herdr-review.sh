#!/usr/bin/env bash
# The fetch script installs a binary whose checksum matches and refuses one that does not, both as the plugin's
# build step and as fetch-binary.sh in the herdr-app mod folder.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
t="$(mktemp -d)"
trap 'rm -rf "$t"' EXIT
case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) target=aarch64-apple-darwin ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
  *) target=aarch64-unknown-linux-gnu ;;
esac
asset="herdr-review-$target"

mkdir -p "$t/plugin/scripts" "$t/release"
cp "$repo/scripts/fetch-herdr-review.sh" "$t/plugin/scripts/"
cp "$repo/Cargo.toml" "$t/plugin/"
printf '#!/bin/sh\necho ok\n' > "$t/release/$asset"
if command -v sha256sum >/dev/null 2>&1; then sum() { sha256sum "$1"; }; else sum() { shasum -a 256 "$1"; }; fi
export HERDR_REVIEW_BASE_URL="file://$t/release"

echo "0000000000000000000000000000000000000000000000000000000000000000  $asset" > "$t/release/$asset.sha256"
if bash "$t/plugin/scripts/fetch-herdr-review.sh" 2>"$t/err"; then
  echo "a wrong checksum was accepted" >&2; exit 1
fi
grep -q "sha256 mismatch" "$t/err"
test ! -e "$t/plugin/bin/herdr-review"

(cd "$t/release" && sum "$asset" > "$asset.sha256")
bash "$t/plugin/scripts/fetch-herdr-review.sh"
test "$("$t/plugin/bin/herdr-review")" = ok

out="$(bash "$t/plugin/scripts/fetch-herdr-review.sh")"
case "$out" in *"already installed"*) ;; *) echo "second run downloaded again: $out" >&2; exit 1 ;; esac

# The copy in the mod folder reads VERSION and installs beside itself, as an agent runs it after copying dist/.
mod="$t/app/mods/herdr-review"
mkdir -p "$mod"
cp "$repo"/herdr-app-mod/dist/mods/herdr-review/{fetch-binary.sh,VERSION} "$mod/"
bash "$mod/fetch-binary.sh"
test "$("$mod/bin/herdr-review")" = ok
test "$(cat "$mod/bin/herdr-review.version")" = "$(tr -d '[:space:]' < "$mod/VERSION")"
