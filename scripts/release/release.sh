#!/usr/bin/env bash
# Cuts a release: sets VERSION in the root Cargo.toml [workspace.package] (every crate inherits
# it), refreshes Cargo.lock, turns CHANGELOG.md's "## [Unreleased]" section into
# "## [VERSION] - DATE", runs `just verify`, and only when that passes commits
# `🔖 Release vVERSION`, tags it and pushes branch and tag. The tag push starts
# .github/workflows/release.yml, which builds every platform and publishes the GitHub release.
# A failure before the commit restores the edited files, so nothing half-done is left behind.
#
# This publishes a release: run it only when the owner has said to.
#
# Usage: scripts/release/release.sh 0.1.0      (or: just release 0.1.0)
#   Needs a clean working tree on `nightly`, not behind origin, and push access to origin.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

version="${1:-}"
version="${version#v}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "usage: $0 MAJOR.MINOR.PATCH (got '${1:-}')" >&2
    exit 1
fi
tag="v${version}"
files=(Cargo.toml Cargo.lock CHANGELOG.md)

if [ -n "$(git status --porcelain)" ]; then
    echo "release: the working tree has uncommitted changes - commit them first" >&2
    exit 1
fi
branch="$(git symbolic-ref --short -q HEAD)" || {
    echo "release: check out a branch first" >&2
    exit 1
}
if [ "$branch" != "nightly" ]; then
    echo "release: releases are cut from nightly (on ${branch})" >&2
    exit 1
fi
git fetch --quiet --tags origin
if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
    echo "release: tag ${tag} already exists" >&2
    exit 1
fi
if ! git merge-base --is-ancestor "origin/${branch}" HEAD; then
    echo "release: ${branch} is behind origin/${branch}; pull first" >&2
    exit 1
fi
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
    echo "release: CHANGELOG.md has no '## [Unreleased]' section" >&2
    exit 1
fi

committed=false
restore() {
    if [ "$committed" = false ]; then
        git restore --source=HEAD --staged --worktree -- "${files[@]}"
        echo "release: failed before the release commit; ${files[*]} restored" >&2
    fi
}
trap restore EXIT

# Only the version line inside [workspace.package]; dependency versions stay untouched.
VERSION="$version" perl -0pi -e \
    's/(\[workspace\.package\][^\[]*?\nversion\s*=\s*)"[^"]*"/$1"$ENV{VERSION}"/' Cargo.toml
if [ "$(scripts/release/version.sh)" != "$version" ]; then
    echo "release: failed to set version ${version} in Cargo.toml" >&2
    exit 1
fi
cargo update --workspace

today="$(date -u +%Y-%m-%d)"
VERSION="$version" TODAY="$today" perl -0pi -e \
    's/^## \[Unreleased\]\n/## [Unreleased]\n\n## [$ENV{VERSION}] - $ENV{TODAY}\n/m' CHANGELOG.md
scripts/release/notes.sh "$version" >/dev/null

just verify

git add -- "${files[@]}"
git commit -m "🔖 Release ${tag}"
git tag -a "$tag" -m "nomouse ${tag}"
committed=true

if ! git push --atomic origin "HEAD:refs/heads/${branch}" "refs/tags/${tag}"; then
    echo "release: push failed; fix it, then: git push --atomic origin HEAD:${branch} ${tag}" >&2
    exit 1
fi
echo "release: pushed ${tag}; the release workflow publishes it:"
echo "  gh run watch \$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
