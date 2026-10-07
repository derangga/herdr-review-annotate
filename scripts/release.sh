#!/usr/bin/env bash
# Cuts a release from a clean master. Asks for the new version, writes it to Cargo.toml, Cargo.lock
# and herdr-plugin.toml, commits that, tags the commit, then asks before pushing. release.yml builds
# on any pushed v* tag.
set -euo pipefail

cd "$(dirname "$0")/.."

[ "$(git branch --show-current)" = master ] || { echo "not on master" >&2; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "working tree has uncommitted changes" >&2; exit 1; }

version_of() { sed -n 's/^version = "\(.*\)"/\1/p' "$1" | head -1; }

echo "Latest tag:        $(git describe --tags --abbrev=0 2>/dev/null || echo none)"
echo "Cargo.toml:        $(version_of Cargo.toml)"
echo "herdr-plugin.toml: $(version_of herdr-plugin.toml)"
read -r -p "New version (for example 0.3.0): " version
version=${version#v}

[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || { echo "not a semver version: $version" >&2; exit 1; }
tag=v$version
! git rev-parse -q --verify "refs/tags/$tag" >/dev/null || { echo "tag $tag already exists" >&2; exit 1; }

# release.yml requires the tag, Cargo.toml and herdr-plugin.toml to agree.
if [ "$(version_of Cargo.toml)" != "$version" ] || [ "$(version_of herdr-plugin.toml)" != "$version" ]; then
  for file in Cargo.toml herdr-plugin.toml; do
    # Only the first version line, the package's, changes.
    awk -v v="$version" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
      "$file" > "$file.new" && mv "$file.new" "$file"
  done
  cargo update --offline --package herdr-review
  git add Cargo.toml Cargo.lock herdr-plugin.toml
  git commit -q -m "Release $tag"
  echo "Set $version in Cargo.toml, Cargo.lock and herdr-plugin.toml"
  undo="git tag -d $tag && git reset --hard HEAD~1"
else
  undo="git tag -d $tag"
fi

git tag -a "$tag" -m "$tag"
echo "Created tag $tag at $(git rev-parse --short HEAD)"

read -r -p "Push master and $tag to origin? [y/N] " answer
if [[ $answer =~ ^[Yy]$ ]]; then
  git push origin master "$tag"
else
  echo "Not pushed. Push with: git push origin master $tag"
  echo "Undo with:      $undo"
fi
