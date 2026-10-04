#!/usr/bin/env bash
# Build and stage the binary for `herdr plugin link`, which skips manifest build hooks.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --manifest-path "$root/Cargo.toml" --release
mkdir -p "$root/bin"
cp "$root/target/release/herdr-review" "$root/bin/herdr-review.tmp"
mv "$root/bin/herdr-review.tmp" "$root/bin/herdr-review"
echo "staged $root/bin/herdr-review"
