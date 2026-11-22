#!/bin/sh
# Focused regression tests for the commit-msg policy; run by scripts/check.sh.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/panoptes-commit-msg.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
message="$tmp/message"
count=0

valid() {
    name=$1
    shift
    printf '%s\n' "$@" > "$message"
    if ! sh "$root/scripts/check-commit-msg.sh" "$message"; then
        echo "Valid case rejected: $name" >&2
        exit 1
    fi
    count=$((count + 1))
}

invalid() {
    name=$1
    shift
    printf '%s\n' "$@" > "$message"
    if sh "$root/scripts/check-commit-msg.sh" "$message" >/dev/null 2>&1; then
        echo "Invalid case accepted: $name" >&2
        exit 1
    fi
    count=$((count + 1))
}

valid "feature with scope" "feat(app): add appearance settings"
valid "fix without scope" "fix: preserve session identifiers"
valid "breaking change" "feat(core)!: require schema version nine" "" "BREAKING CHANGE: existing stores migrate on open."
valid "body and reference" "docs: explain commit conventions" "" "Document the local hook and rationale." "" "Refs: #123"
valid "comments ignored" "# Write a subject" "test(scripts): cover the commit linter"
valid "merge commit" "Merge branch 'feature' into main"
valid "revert commit" 'Revert "feat(app): add appearance settings"'
valid "fixup commit" "fixup! feat(app): add appearance settings"
valid "squash commit" "squash! feat(app): add appearance settings"

invalid "untyped subject" "Add appearance settings"
invalid "uppercase type" "Feat(app): add appearance settings"
invalid "missing separator space" "feat(app):add appearance settings"
invalid "empty summary" "feat(app): "
invalid "uppercase scope" "feat(App): add appearance settings"
invalid "subject period" "fix(core): preserve existing data."
invalid "overlong subject" "feat(app): add a deliberately overlong subject that cannot fit within the seventy-two character limit"
invalid "missing body separator" "feat(app): add appearance settings" "Explain why the setting is needed."
invalid "trailing whitespace" "feat(app): add appearance settings "
invalid "empty message" ""
invalid "only comments" "# feat(app): add appearance settings"

echo "commit-message linter: $count cases passed"
