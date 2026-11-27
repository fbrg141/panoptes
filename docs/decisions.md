# Decisions

The durable choices that shape Panoptes, and why. Change one deliberately and
record the change here. Current behaviour belongs in
[domain-model.md](domain-model.md); the [product bet review](bets.md) holds the
proposed direction and evidence. Implementation history, milestones, issue
status and delivery commitment belong in GitHub, not in a second Markdown
backlog.

## One Rust app with a GPUI UI

*Decided 2026-11-20; superseded the earlier Tauri and React direction.*

Panoptes is one Rust app: `panoptes-core` owns backend behaviour without a UI
dependency, and `panoptes-app` renders it with GPUI. This gives the app one
language and type system, with no webview or IPC layer. The rewrite was a
deliberate product and maintenance choice, not a response to a measured
performance problem.

- Logic stays outside views, in the core or tested app models.
- GPUI comes through an exactly pinned `gpui-kit`; upgrades follow the
  [dependency policy](dependencies.md).
- The design tool's export is vendored into the app. Tokens and fonts live in
  `crates/panoptes-app/assets`; icons and logos live in `src/design_assets.rs`.
  Views use those values rather than defining a second design system.

## Drive `pi --mode rpc`, not Pi's Node SDK

*Decided 2026-11-04.*

The SDK requires Node, which would force a sidecar that effectively rebuilds
Pi's RPC mode. RPC is intended for custom user interfaces, is maintained
upstream, and uses the user's installed Pi, settings, logins and extensions.
Each open session has its own `pi --mode rpc --session <file>` process, giving
crash isolation and independent parallel sessions.

Capabilities that only the SDK exposes would need a small sidecar or an
upstream RPC addition; neither justifies coupling the core to Node.

## Keep a neutral harness boundary

*Decided 2026-11-04.*

Panoptes may support other harnesses later. The app and core therefore use
`panoptes-harness` types for sessions, messages, blocks, questions and events;
each harness has an adapter crate. The interface is loosely shaped like Zed's
Agent Client Protocol, but follows Panoptes' needs rather than copying a
protocol.

Harness conversations stay in the harness's own storage. Panoptes stores only
the organisation and metadata it adds, referencing conversations by harness
identity. Pi-specific protocol and file details remain inside `panoptes-pi`.

## Store Panoptes metadata in SQLite

*Decided 2026-11-06.*

Projects, lanes, session records and preferences cannot be rebuilt from harness
conversations, so their integrity matters. SQLite provides transactions,
relationships and integrity checks without another service.

Schema changes are append-only migrations using SQLite's `user_version`. Each
migration runs in an `IMMEDIATE` transaction, checks foreign keys before
commit, and is preceded by a backup. A damaged, locked, newer or unmigratable
store stops the app with an error instead of silently starting empty. Debug
builds use separate stores and lane folders so unshipped migrations never touch
installed-app data.

## Observe Pi; do not orchestrate agents

*Decided 2026-11-11 (#225).*

Panoptes is a minimal desktop app for observing and presenting an agent's work.
It does not plan tasks, delegate work or coordinate a team of agents. Harness
conversations remain in harness files, and lanes remain plain git worktrees and
branches.

This keeps the core small, preserves compatibility with Pi's CLI, and avoids
multiplying process, lock and cleanup failure modes. Value comes from making
work understandable rather than adding autonomy. Multi-agent coordination,
Panoptes-owned subsessions, deferred unattended approvals and cross-lane
coordination were rejected under this direction.

Showing a harness-owned subagent read-only is visibility, not orchestration:
the harness retains its lifecycle and execution control. A later product track
may broaden session scope to projects or groups, but the user initiates that
work and the harness still executes it. The proposed sequence is recorded in
[bets.md](bets.md); it is not a commitment to implementation before #226.

Until the bet review on 2026-12-05, GitHub issue intake is bug-only. Existing
feature issues and milestones are planning artefacts, not approved work. New
feature ideas go to the [observation log](bets.md#observation-log), except for
a small fix for pain recorded there during daily use.

## Make the lane the unit of work

*Decided 2026-11-07; revised for retained session history on 2026-11-19.*

A project wraps a local git repository. A lane owns a task, branch, worktree
and session history. The lane survives any individual conversation and is the
durable unit the user organises and reviews.

- **One live session, one writer.** A lane retains earlier sessions but runs at
  most one harness process. `/new` and `/resume` stop the current process before
  opening another; an earlier conversation can also be viewed read-only.
  Parallel work happens in separate lanes, without cross-lane coordination.
- **One worktree in the UI, room in the model.** Storage permits multiple
  worktrees per lane, but creation currently makes one. This does not commit
  Panoptes to multi-repo lanes.
- **A fresh worktree by default.** A lane normally uses a new worktree and a
  `panoptes/<lane>` branch. Using the project's main checkout is an explicit,
  clearly marked alternative.
- **Archiving protects work.** Clean worktrees can be removed. Dirty work must
  be committed, stashed or saved as a restorable checkpoint. Branches and
  harness conversations are not deleted by archiving.
- **The word is “lane”.** “Workspace” already has conflicting meanings in VS
  Code, Zed and Cargo.

Nested groups, multi-repo creation and Panoptes-owned subsessions are not part
of the **current** model. The third proposed track in [bets.md](bets.md) would
deliberately revisit groups and scoped sessions after #226, with explicit
per-repository boundaries. Any Pi subagent presentation remains observational
and read-only.

## Review lanes from git trees

*Lane review decided with #203; base readiness added with #210.*

The tree, not the agent's account or edit-tool records, is the source of truth
for review. Tree snapshots include shell edits and untracked files and work for
active and archived lanes. The review pane is read-only and always names its
comparison.

Each worktree resolves its own merge-base and tree range. Hunk requests carry
lane, project, path and range identities so future multiple-worktree rendering
cannot mix equal paths from different repositories. App models own selection,
expansion, caching and request generations; views only render.

Base freshness and merge previews are also read-only. A preview may report
clean, named conflicts or unavailable; failure never looks like a clean merge.
It does not fetch, rebase, move refs, or touch HEAD, the real index, stash or
config.

## Keep per-turn checkpoints on plain git

*Decided 2026-11-07; implemented with #204.*

A writable lane run records before and after tree snapshots as commits under
`refs/panoptes/lanes/<lane>/turns/`. These durable boundaries support later
turn review, attribution and recovery without changing the user's repository
state.

- Turn identity belongs to the lane; trees and refs belong to each worktree.
  There is no claim of atomic snapshots across repositories.
- Immediate prompts have exact before boundaries. Queued prompts refresh theirs
  when the run actually starts and are marked observed. Steering does not create
  a turn. Missing snapshots are explicitly unavailable, never empty diffs.
- Snapshots leave HEAD, the branch, real index, stash and config untouched.
  Ignored files stay excluded, large files are retained, and unsafe nested
  repositories fail loudly.
- Store intent precedes git refs. Startup reconciles interrupted turns, adopts
  valid generated refs, recovers readable worktrees, and removes only generated
  orphans.
- Snapshot failures are recorded and do not block the user's prompt.

Jujutsu was rejected because its workspace, operation and bookmark models would
add another dependency and interfere with ordinary git tooling while duplicating
what a few git plumbing operations already provide.

## Review the product direction after real use

*Decided 2026-11-07; proposed track recorded 2026-11-27, pending #226.*

A dependable worktree-per-task spine is necessary but not a differentiator.
Panoptes will confirm, reject or extend one coherent product direction from
observed daily use rather than build several partial bets.

The proposed order is **depth first, completeness second, breadth third**:
reviewable lanes, read-only visibility into harness-owned subagents, then
project groups and user-initiated sessions at group, project or lane scope. The
evidence and boundaries are in [bets.md](bets.md). GitHub milestones group
proposed outcomes, while Project Status controls commitment: feature issues
stay `Parked` until #226, even when a milestone names the proposed track. The
third track would supersede the earlier rejection of groups and multi-repo
work only after a deliberate decision.

## Keep composer behaviour close to Pi

Model and thinking level belong to the session, as in Pi, and apply from the
next message. Enter steers a running agent; Option+Enter queues a follow-up.
Images are sent inline, while other files become `@path` mentions for the agent
to read with its own tools. Slash commands expose capabilities that Pi can run
through a normal RPC prompt; terminal-only commands are not offered.

These choices avoid maintaining a second model, attachment or command system
that can drift from the harness.
