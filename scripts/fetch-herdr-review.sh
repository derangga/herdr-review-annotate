#!/usr/bin/env bash
# Put the release binary into bin/. Herdr runs this with cwd = plugin root.
# The version is the one in Cargo.toml. HERDR_REVIEW_BASE_URL replaces the release URL (tests).
set -euo pipefail

cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -n "$version" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
mkdir -p bin
if [ -x bin/herdr-review ] && [ "$(cat bin/herdr-review.version 2>/dev/null || true)" = "$version" ]; then
  echo "herdr-review $version already installed"
  exit 0
fi

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64)              target=aarch64-apple-darwin ;;
  Darwin/x86_64)             target=x86_64-apple-darwin ;;
  Linux/x86_64)              target=x86_64-unknown-linux-gnu ;;
  Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "no herdr-review build for $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac

asset="herdr-review-$target"
base="${HERDR_REVIEW_BASE_URL:-https://github.com/derangga/herdr-review-annotate/releases/download/v$version}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $base/$asset"
curl -fsSL --retry 3 -o "$tmp/$asset" "$base/$asset"
curl -fsSL --retry 3 -o "$tmp/$asset.sha256" "$base/$asset.sha256"
expected="$(awk '{print $1}' "$tmp/$asset.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$asset" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$tmp/$asset" | awk '{print $1}')"
fi
[ -n "$expected" ] && [ "$actual" = "$expected" ] || {
  echo "sha256 mismatch for $asset: expected '$expected', got $actual" >&2
  exit 1
}

chmod +x "$tmp/$asset"
mv "$tmp/$asset" bin/herdr-review
echo "$version" > bin/herdr-review.version
echo "installed herdr-review $version ($target)"
