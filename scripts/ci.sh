#!/usr/bin/env bash
# The CI steps, in CI's order. ci.yml runs this script, so a green run here is a green run there when
# the toolchain matches the pin in ci.yml. Without rustfmt or clippy on PATH it re-runs under nix.
set -euo pipefail

cd "$(dirname "$0")/.."
if ! { cargo fmt --version && cargo clippy --version; } >/dev/null 2>&1; then
  [ -z "${CI_SH_IN_NIX:-}" ] || { echo "rustfmt or clippy is missing" >&2; exit 1; }
  CI_SH_IN_NIX=1 exec nix shell nixpkgs#rustfmt nixpkgs#clippy --command bash "$0"
fi

cargo --version
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --no-default-features
bash scripts/test-fetch-herdr-review.sh

# The herdr-app mod: tests and the bundle. The typecheck is local only, it needs gpui-kit.d.ts, which only
# a machine where herdr-app has run has (design/gui.md).
(
  cd herdr-app-mod
  node --version
  npm ci
  npm run lint
  npm test
  npm run build
)
