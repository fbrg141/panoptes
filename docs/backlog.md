# Backlog

*Cut on 2026-11-12. The foundation audit of 2026-11-11 left more open fix
issues than one person can steer by, so the list was cut to what loses work,
misleads, or is hit every day.*

## The gate

*Refreshed on 2026-11-15.* Twenty of the 21 fixes cut on 2026-11-12 are
closed, and so is the rename regression (#263) added since. One is left:

- Add CI that runs `scripts/check.sh` on PRs and main (#3). The workflow in
  #245 predates the move to GPUI and needs that command; its first run is
  blocked by a GitHub billing lock on the account. Until it runs,
  `scripts/check.sh` locally (and the pre-push hook) is the check every merge
  must pass.

Closed: #77, #83, #84, #87, #98, #101, #105, #123, #124, #129, #131, #136,
#151, #160, #172, #173, #174, #175, #176, #197, #263.

## Next

- Offer to archive merged or idle lanes and show worktree disk use (#264).
  Worktrees pile up in daily use (see the
  [observation log](bets.md#observation-log)), so this counts as a fix.

## Intake until the bet review

Bug-only intake (see [AGENTS.md](../AGENTS.md#issue-intake)) stays in place
until the bet review on 2026-12-05, even once #3 closes. One exception: a
small fix for pain hit in daily use may be filed and worked when the
observation log records it, as #264 is. Everything else, including #266 and
the bet work (#201 to #206), stays parked until the review.

## Later

Real, but not blocking daily use. Their issues are closed so the open list
stays short; each number links to the full description. Reopen one when it
bites, or pick from here after the bet review.

### Lanes and store

- Every rollback error is swallowed (#81)
- `base_branch` goes to git unvalidated (#85)
- No timeout on any git call (#93)
- Lane creation cleanup can delete a branch Panoptes did not create (#94)
- Schema test does not test the CHECK it claims to (#97)

### Service and IPC

- `open` trusts the webview for harness, locator and cwd (#106)
- The `{ kind, message }` contract is uneven (#108)
- `list()` does filesystem I/O on the runtime while holding the `open` lock (#109)
- Type parity is manual and unguarded (#110)
- Idle is sent as a fifth status word (#114)
- Service tests miss every interleaving and rely on real sleeps (#119)

### Harness and Pi

- The fake Pi is the only evidence for several protocol shapes and cannot interleave (#125)
- One 60 s timeout for every command, no cancellation (#126)
- `history()` swallows every error from the incremental fetch (#127)
- History cost grows with the session on every reload (#128)
- Unparseable stdout lines vanish silently and line length is unbounded (#130)
- Session listing assumes `"type"` is the first JSON key (#134)
- First scan reads every session file in full (#135)
- `read_only` is promised neutrally but implemented as a tool list (#137)

### Frontend state

- Streaming cost is O(conversation) per token (#148)
- `init` cannot recover from a failure and leaks its listener (#154)
- One global `error` string (#155)
- `thoughtFor` is keyed by full thinking text (#156)
- `splitMarkdown` changes what the markdown means (#158)
- The mock backend breaks the contract it stands in for (#159)
- Reducer and store tests miss duplicates, ordering and failure cases (#161)

### UI and design system

- Two modal systems that look and behave differently (#171)
- The terminal block shows a false working directory (#177)
- The header truncates the title before the path (#178)
- React effects re-subscribe every render or cache failures (#183; gone with React)
- The same actions are worded differently across the UI (#184)
- Approval and question cards take no focus and are not announced (#185)
- Small hit targets and hover-only controls (#186)

## Not planned

Thirty polish and cleanup issues (the audit's P4) were closed as not planned
on the same day, along with two made obsolete by one session per lane (#102,
#104). They are listed under the closed issues with the label `P4`.
