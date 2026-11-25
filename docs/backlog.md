# Backlog

*Cut on 2026-11-12. The foundation audit of 2026-11-11 left more open fix
issues than one person can steer by, so the list was cut to what loses work,
misleads, or is hit every day.*

## The gate

*Closed 2026-11-21.* All 21 fixes cut on 2026-11-12 are closed, and so are the
rename regression (#263) added since and CI (#3):
[check.yml](../.github/workflows/check.yml) runs `scripts/check.sh` on every
pull request and push to `main`.

## Intake until the bet review

Bug-only intake (see [AGENTS.md](../AGENTS.md#issue-intake)) stays in place
until the bet review on 2026-12-05. One exception: a small fix for pain hit in
daily use may be filed and worked when the observation log records it, as
#264 (idle-lane cleanup and worktree disk use, now shipped in `cleanup.rs`)
was. Everything else, including #266 and the bet work (#201 to #206), stays
parked until the review.

## Later

Real, but not blocking daily use. Their issues are closed so the open list
stays short; each number links to the full description. Reopen one when it
bites, or pick from here after the bet review.

### Lanes and store

- Every rollback error is swallowed (#81)
- `base_branch` goes to git unvalidated (#85)
- Lane creation cleanup can delete a branch Panoptes did not create (#94)
- Schema test does not test the CHECK it claims to (#97)

### Service

- Service tests miss every interleaving and rely on real sleeps (#119)

The webview and IPC findings (#106, #108, #109, #110, #114) went with the
GPUI rewrite, or were settled by design: errors cross as `{ kind, message }`
and `idle` on the wire is the documented contract. The React frontend's
state findings (#148, #154 to #159, #161) died with React; the GPUI app's
logic is covered by its `model/` tests.

### Harness and Pi

- The fake Pi is the only evidence for several protocol shapes and cannot interleave (#125)
- One 60 s timeout for every command, no cancellation (#126)
- `history()` swallows every error from the incremental fetch (#127)
- History cost grows with the session on every reload (#128)
- Unparseable stdout lines vanish silently and line length is unbounded (#130)
- Session listing assumes `"type"` is the first JSON key (#134)
- First scan reads every session file in full (#135)
- `read_only` is promised neutrally but implemented as a tool list (#137)

### UI and design system

- Two modal systems that look and behave differently (#171)
- The terminal block shows a false working directory (#177)
- The header truncates the title before the path (#178)
- The same actions are worded differently across the UI (#184)
- Approval and question cards take no focus and are not announced (#185)
- Small hit targets and hover-only controls (#186)

## Not planned

Thirty polish and cleanup issues (the audit's P4) were closed as not planned
on the same day, along with two made obsolete by one session per lane (#102,
#104). They are listed under the closed issues with the label `P4`.
