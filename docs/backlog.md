# Backlog

*Cut on 2026-11-12. The foundation audit of 2026-11-11 left more open fix
issues than one person can steer by, so the list was cut to what loses work,
misleads, or is hit every day.*

## The gate

These 21 issues are the open fix list. Bug-only intake (see
[AGENTS.md](../AGENTS.md#issue-intake)) lasts until they are closed.

- Add CI that runs `pnpm check` on PRs and main (#3)
- Archive is not atomic with the store (#77)
- Restoring a checkpoint rewrites the user's branch (#83)
- A moved or deleted repository is a dead end (#84)
- Check-then-write store operations use deferred transactions or none (#87)
- No migration test starts from a populated v2 or v3 database (#98)
- "One process per open session" is only enforced open-versus-open (#101)
- Status and pending cards cannot be recovered (#105)
- Tool output truncation destroys the exit code and keeps the wrong end (#123)
- Message-id stability rests on a non-unique `(role, timestamp)` key (#124)
- Tool calls with no end event stay `Running` forever (#129)
- `approvals.js` is never executed by any test (#131)
- The version check is computed but not enforced (#136)
- Component error path prints `[object Object]` (#151)
- Shortcuts fire on Ctrl as well as ⌘ (#160)
- Popup menus lose their edge and shadow (#172)
- Removing a project has no confirmation (#173)
- Picking a lane in "Jump to…" starts a new session (#174)
- The "All sessions" tab has a blank icon (#175)
- Branch names are shown uppercased (#176)
- Docs describe behaviour the code does not have (#197)

## Later

Real, but not blocking daily use. Their issues are closed so the open list
stays short; each number links to the full description. Reopen one when it
bites, or pick from here once the gate is closed.

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
- React effects re-subscribe every render or cache failures (#183)
- The same actions are worded differently across the UI (#184)
- Approval and question cards take no focus and are not announced (#185)
- Small hit targets and hover-only controls (#186)

## Not planned

Thirty polish and cleanup issues (the audit's P4) were closed as not planned
on the same day, along with two made obsolete by one session per lane (#102,
#104). They are listed under the closed issues with the label `P4`.
