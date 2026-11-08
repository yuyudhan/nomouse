#!/usr/bin/env bash
# Runs a command inside a snapshot of exactly what is being committed or pushed, so files other
# people (or agents) have left uncommitted in the working tree cannot pass or fail the check.
#
# Usage: scripts/hooks/in-snapshot.sh NAME SOURCE -- COMMAND...
#   NAME     snapshot directory under target/hook-tree/ (one per hook)
#   SOURCE   --index for the staged index (pre-commit), or a commit (pre-push)
#
# The snapshot lives under target/ (git-ignored) and is updated in place with rsync --checksum,
# so unchanged files keep their modification times and cargo rebuilds only what changed. Cargo
# builds into the repository's own target/ directory. Inside the snapshot, git sees the
# snapshot as its work tree and the snapshot's index, so git-based guards list exactly the
# committed files.
set -euo pipefail

if [ "$#" -lt 4 ] || [ "$3" != "--" ]; then
    echo "usage: $0 NAME --index|COMMIT -- COMMAND..." >&2
    exit 2
fi
name="$1"
source="$2"
shift 3

root="$(git rev-parse --show-toplevel)"
git_dir="$(git rev-parse --absolute-git-dir)"
base="${root}/target/hook-tree"
snapshot="${base}/${name}"
stage="${base}/${name}.stage"
index="${base}/${name}.index"
mkdir -p "$base"

# Two commits at once would share the snapshot; serialise them where flock exists (Linux).
if command -v flock >/dev/null 2>&1; then
    exec 9>"${base}/${name}.lock"
    flock 9
fi

if [ "$source" = "--index" ]; then
    # Git exports GIT_INDEX_FILE for `commit -a` and partial commits; it may be relative.
    source_index="${GIT_INDEX_FILE:-${git_dir}/index}"
    case "$source_index" in
        /*) ;;
        *) source_index="${root}/${source_index}" ;;
    esac
    cp "$source_index" "$index"
else
    GIT_INDEX_FILE="$index" git read-tree "$source"
fi

rm -rf "$stage"
mkdir -p "$stage" "$snapshot"
GIT_INDEX_FILE="$index" git checkout-index --all --force --prefix="${stage}/"
rsync --recursive --links --perms --checksum --delete "${stage}/" "${snapshot}/"
rm -rf "$stage"

cd "$snapshot"
export GIT_DIR="$git_dir" GIT_WORK_TREE="$snapshot" GIT_INDEX_FILE="$index"
export CARGO_TARGET_DIR="${root}/target"
echo "hook: checking the ${name} snapshot (${source}) in target/hook-tree/${name}" >&2
exec "$@"
