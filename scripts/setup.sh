#!/bin/sh
# Configure this clone only; never change a contributor's global Git setup.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"

git config --local core.hooksPath .githooks
# Relative paths work from subdirectories and remain valid in every worktree.
git config --local commit.template .gitmessage.txt

echo "Enabled Panoptes Git hooks and the commit-message template for this clone."
