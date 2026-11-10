#!/usr/bin/env bash
# The CI steps, in CI's order. ci.yml runs `ci.sh test` and `ci.sh build` as separate jobs, and release.yml
# runs `ci.sh test`, so a green run here is a green run there when the toolchain matches the pin in ci.yml.
# With no stage it runs both. Without rustfmt or clippy on PATH the test stage re-runs under nix.
set -euo pipefail

stage=${1:-all}
case $stage in
  test | build | all) ;;
  *)
    echo "usage: ci.sh [test|build]" >&2
    exit 2
    ;;
esac

cd "$(dirname "$0")/.."
if [ "$stage" != build ] && ! { cargo fmt --version && cargo clippy --version; } >/dev/null 2>&1; then
  [ -z "${CI_SH_IN_NIX:-}" ] || { echo "rustfmt or clippy is missing" >&2; exit 1; }
  CI_SH_IN_NIX=1 exec nix shell nixpkgs#rustfmt nixpkgs#clippy --command bash "$0" "$stage"
fi

cargo --version
node --version

if [ "$stage" != build ]; then
  cargo fmt --all --check
  cargo clippy --all-targets --locked -- -D warnings
  # The build without the `syntax` feature has its own cfg branches, so it is linted and tested too.
  cargo clippy --all-targets --no-default-features --locked -- -D warnings
  cargo test --all-targets --locked
  cargo test --all-targets --no-default-features --locked
  bash scripts/test-fetch-herdr-review.sh

  # The herdr-app mod. The typecheck is local only, it needs gpui-kit.d.ts, which only a machine where
  # herdr-app has run has (design/gui.md).
  (
    cd herdr-app-mod
    npm ci
    npm run lint
    npm test
  )
fi

if [ "$stage" != test ]; then
  cargo build --no-default-features --locked
  (
    cd herdr-app-mod
    # With no stage the test steps above already installed the mod's packages.
    [ "$stage" = all ] || npm ci
    npm run build
  )
  # The folder an agent installs is committed, so it has to match what the source builds.
  dist="$(mktemp -d)"
  trap 'rm -rf "$dist"' EXIT
  bash scripts/build-dist.sh "$dist"
  diff -r "$dist/mods/herdr-review" herdr-app-mod/dist/mods/herdr-review || {
    echo "herdr-app-mod/dist is stale: run scripts/build-dist.sh and commit it" >&2
    exit 1
  }
fi
