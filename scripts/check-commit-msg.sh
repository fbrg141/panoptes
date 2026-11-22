#!/bin/sh
# Validate one commit message. Called by .githooks/commit-msg and by the
# repository check suite. Uses only POSIX tools so contributors need no setup
# beyond Git.
set -eu

if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
    echo "Usage: sh scripts/check-commit-msg.sh <message-file>" >&2
    exit 2
fi

awk '
    BEGIN {
        errors = ""
        header = ""
        separated = 0
        generated = 0
    }

    # Git strips these before creating the commit, so the linter does too.
    /^[[:space:]]*#/ { next }

    {
        if (header == "") {
            if ($0 ~ /^[[:space:]]*$/) { next }
            header = $0

            # Git-generated history commands must remain usable.
            generated = header ~ /^(Merge |Revert |fixup! |squash! |amend! )/
            if (generated) { next }

            if (length(header) > 72) {
                errors = errors "  - Keep the subject to 72 characters or fewer (aim for 50).\n"
            }
            if (header ~ /[[:space:]]+$/) {
                errors = errors "  - Remove trailing whitespace from the subject.\n"
            }
            if (header !~ /^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9][a-z0-9._\/-]*\))?!?: [^[:space:]].*$/) {
                errors = errors "  - Use <type>(<optional-scope>): <summary>, with a supported lowercase type.\n"
            } else {
                summary = header
                sub(/^[^:]*: /, "", summary)
                if (summary ~ /\.$/) {
                    errors = errors "  - Do not end the subject with a period.\n"
                }
            }
            next
        }

        if (!generated && !separated) {
            if ($0 ~ /^[[:space:]]*$/) {
                separated = 1
            } else {
                errors = errors "  - Leave one blank line between the subject and body.\n"
                separated = 1
            }
        }
    }

    END {
        if (header == "") {
            errors = errors "  - Write a commit subject.\n"
        }
        if (errors != "") {
            print "Invalid commit message:" > "/dev/stderr"
            printf "%s", errors > "/dev/stderr"
            print "See .gitmessage.txt for the convention and examples." > "/dev/stderr"
            exit 1
        }
    }
' "$1"
