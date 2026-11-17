#!/usr/bin/env bash
# Bumps the version (the patch number, or to the version given), then commits, tags and pushes it from main.
# The tag push starts the "Build and release" workflow, which publishes the release.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "release from main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "commit or stash your changes first" >&2; exit 1; }
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml)
IFS=. read -r major minor patch <<< "$current"
version=${1:-$major.$minor.$((patch + 1))}
git rev-parse -q --verify "refs/tags/v$version" >/dev/null && { echo "v$version is already tagged" >&2; exit 1; }
sed -i.bak "s/^version = \"$current\"$/version = \"$version\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --quiet
git commit --quiet --all --message "Version $version"
git tag --annotate "v$version" --message "plaingram v$version"
git push --quiet origin main "v$version"
echo "Pushed v$version; GitHub Actions now builds and publishes it."
