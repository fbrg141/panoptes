# Decisions

The choices that shape Panoptes, and why. Each one is cheap to keep and
expensive to reverse, so change one only on purpose, and record it here.

## Move the UI to GPUI, and drop Tauri and React

*Decided 2026-11-20. Supersedes the two decisions below on the UI.*

Panoptes becomes one Rust app with a GPUI UI. This is a deliberate choice
without a measured performance problem: one language, one type system, no
webview and no IPC layer, accepting a full UI rewrite. The plan and its risks
are on [#222](https://github.com/fbrg141/panoptes/issues/222).

- The backend lives in `crates/panoptes-core` and has no UI dependency. The
  Tauri app and the GPUI app (`crates/panoptes-app`) both ran on it until the
  GPUI app reached parity; then Tauri, React and the JS toolchain were
  removed.
- GPUI comes through `gpui-kit` (GPUI, `gpui-base` and `gpui-component`),
  pinned to an exact version, since it is pre-1.0 and breaks often.
- The standing rules below still apply: decisions live in Rust outside the
  views, and design tokens stay data that Rust reads.
- The design system's export is vendored into the app: `tokens.json` and the
  fonts under `crates/panoptes-app/assets`, icons and logos in
  `src/design_assets.rs`. The React components went with React; their GPUI
  versions are in `ds.rs`.

## Tauri v2, not Electron or a pure Rust UI

*Decided 2026-11-04. Superseded by the move to GPUI above.*

- **Tauri v2** gives a small native app (about 10 MB, against 150 MB or more
  for Electron with its bundled Chromium) and low memory use. That matters
  when many sessions and panes stay open all day. The Rust core is a good fit
  for the system work on the roadmap: many agent processes, terminals, file
  watching, git and search.
- **Electron** would have let the app import Pi's TypeScript SDK directly. We
  don't want that coupling (see the next decision). The UI is plain React,
  so a move to Electron later would be modest work.
- **A pure Rust UI** (GPUI, Iced) is the fastest option, but every chat
  widget (markdown, code blocks, streaming, selection) would have to be
  hand-built.

The UI is React rather than Svelte. It has the larger ecosystem for chat,
markdown and diff views, and agents write it more reliably.

## Stay hybrid for now, and keep a GPUI rewrite possible

*Decided 2026-11-07. Superseded by the move to GPUI above; its standing rules
still apply.*

Panoptes stays on Tauri, React and a Rust core. A later rewrite of the UI
onto GPUI (Zed's Rust UI framework) is kept genuinely possible, but is not
planned.

**Why not now.**

- **GPUI now** would give smoother scrolling, faster startup, less memory,
  no IPC and one type system. Against that, its docs are thin, its API still
  moves, its Windows support is weaker, and it would throw away the shipped
  work-log chat and the design system.
- **Dioxus** is still a webview, and it loses the React ecosystem.
- **egui, Floem or Slint** would mean rebuilding markdown, diffs, code
  highlighting and text selection by hand.
- **Our own UI on winit, wgpu, cosmic-text, taffy and accesskit** is writing
  a framework, not an app.

**Standing rules.** These keep the UI replaceable and apply to every change:

- **Decisions live in Rust; the UI only renders.** Status, tool calls,
  approvals, ordering and ids are decided in the core, as #23 to #27 already
  do. If a component needs logic, the logic belongs in the core or the
  adapter.
- **The typed event stream is the only frontend boundary.** The UI learns
  everything from `HarnessEvent`s and the service's typed commands, and from
  nothing else.
- **Design tokens are data, not CSS.** They live in one place Rust can read
  (`crates/panoptes-app/assets/tokens.json`), and the theme is built from
  that file, never from values written into a view.
- **The UI keeps no model of its own of a session.** It holds what the
  core sent and the view state on top (scroll, focus, open panels), and
  nothing it would have to keep in step with the core.

**Accepted risk.** The rewrite may never happen, because the hybrid stays
good enough. That is fine: the layering pays for itself in testability and
in a thinner UI, whether or not GPUI ever arrives.

## Drive `pi --mode rpc`, not Pi's Node SDK

*Decided 2026-11-04.*

The SDK only runs in Node, so a Rust core could reach it only through a Node
sidecar, and that sidecar would amount to rebuilding Pi's RPC mode. RPC mode
is documented by Pi for exactly this use ("IDEs, and custom user
interfaces"). It is maintained upstream, uses the user's installed Pi with
their settings, logins and extensions, and behaves the same as the CLI.

Each open session runs in its own `pi --mode rpc --session <file>` process.
That isolates crashes and makes parallel sessions free.

Things only the SDK can do (registering tools at runtime, changing the agent
loop) would arrive later as a small Node sidecar or an upstream RPC command.
Neither needs a redesign.

## A neutral harness interface, loosely shaped like ACP

*Decided 2026-11-04.*

Panoptes is meant to drive other harnesses later, or a home-grown one. So
the app and UI only see `panoptes-harness` types: sessions, messages,
blocks, questions and `HarnessEvent`. Each harness is an adapter crate. The
shape follows Zed's Agent Client Protocol (ACP) loosely, so an ACP adapter
could cover several harnesses at once.

Sessions stay in each harness's own storage. Panoptes keeps only what it adds
on top (pins and projects now; lanes, groups and tags next) in its own
store, keyed by `<harness>:<session id>`.

## Panoptes' data in SQLite, with versioned migrations

*Decided 2026-11-06.*

Once Panoptes owns renames, tags, groups and workspaces, its data can't be
rebuilt from the harnesses, so losing it matters. `index.json` was read as
empty when it was damaged and then overwritten on the next pin. The store is
now SQLite (`panoptes.db` in the app data folder, through `rusqlite` with
SQLite bundled), because the domain model has several many-to-many links and
SQLite gives transactions and integrity checks for free.

- The schema version is SQLite's `user_version`. Each migration runs in its
  own `IMMEDIATE` transaction and reads the version again under the lock, so
  a failed one leaves the previous version intact and two copies of Panoptes
  opening the store at once never run the same step twice.
- Foreign keys are off while migrating, so a step can rebuild a table the
  way SQLite requires, and `foreign_key_check` must pass before it commits.
- Before migrating, the database is copied to `panoptes.db.v<old>.bak`. The
  copy is written under a temporary name first, so a failed copy never
  replaces an earlier backup.
- A damaged database, one from a newer Panoptes, one another Panoptes is
  holding, or a failed migration stops the app with an error dialog instead
  of starting empty.
- The first migration imports the pins from `index.json` once and, after it
  commits, renames it to `index.json.migrated`. An empty database next to
  that file is refused rather than started fresh.
- Debug builds keep their store in a `dev` folder inside the app data
  folder, so a migration that has not shipped never touches the installed
  app's data.

## Observe Pi; do not orchestrate agents

*Decided 2026-11-11 (#225).*

Panoptes is a minimal desktop app for Pi. It owns no agent sessions and no
parallel version-control system: sessions stay in Pi's files, and lanes are
plain git worktrees and branches. Panoptes' own store holds only the app
metadata it adds. The app observes what the agent does and presents that work
rather than planning, delegating or coordinating a team of agents.

This keeps the core small and trustworthy, preserves compatibility with Pi's
CLI, and avoids multiplying process, lock and cleanup failure modes. Value
comes from making the agent's work easier to understand, not from adding
autonomy.

**Triage.** Applying this direction closed #6, #8, #21, #44, #45, #58, #207,
#208, #209, #210, #212, #213, #214, #215, #216, #217, #218, #219, #220, #221,
#222, #223 and #224. Until the open fix issues are closed, feature ideas go to
the [observation log](bets.md#observation-log), not to new GitHub issues.

**Backlog cut, 2026-11-12.** The audit's fix list was too long to steer by, so
it was cut to the issues that lose work, mislead, or are hit every day. That
short list is [the gate](backlog.md#the-gate): bug-only intake lasts until it
is closed, not until every P1 to P4 issue is. The rest is closed and kept as
one "Later" list in [backlog.md](backlog.md).

**Gate refresh, 2026-11-15; gate closed 2026-11-21.** Every gate fix,
CI (#3) included, is closed: `check.yml` runs `scripts/check.sh` on every
pull request and push to `main`. Bug-only
intake now lasts until the bet review instead of ending with the gate, so the
review decides from the observation log rather than from a fresh feature
backlog. The one exception is a small fix for pain hit in daily use that the
log records, such as clearing finished lanes (#264, since shipped).

**Event log, 2026-11-13.** An exception to bug-only intake, made by the
product owner: the 2026-12-05 bet review needs a record of how Panoptes
behaved in daily use, not only memory and the observation log. The service
writes one JSON line per event to `<app data>/logs/` (see `eventlog.rs`).
It records what happened, never what was said, and never leaves the
machine. It is the smallest part of #43; the rest of #43 stays parked.

## Lanes are the unit of work

*Decided 2026-11-07.* The full model is in
[domain-model.md](domain-model.md); this records the choices behind it.

A project wraps a git repository (#19). A project has **lanes**. A lane owns
a worktree, a branch, a task and its session history (#228). The lane is the
durable unit: it keeps its worktree, branch, task and earlier conversations
when the current session closes.

1. **One live session, one writer.** A lane holds many session records over
   time, but the core permits only one open process. Starting a fresh session
   or resuming an earlier one first stops the current process. Parallel work
   still means separate lanes, and Panoptes does not coordinate them.
2. **One worktree now, room for more.** The schema lets a lane hold several
   worktrees, so multi-repo lanes (#21) need no migration, but the UI only
   ever creates one. Room without complexity.
3. **Archiving never destroys tracked or configured work.** A dirty tree is
   never removed as it is: Panoptes offers commit, stash (into the
   repository's own stash, named after the lane; restoring the lane does not
   reapply it, so it stays in `git stash list`), or keeping it as a checkpoint (see the checkpoint decision below). Archive
   leaves the branch alone. Current `copyFiles` values are preserved separately
   under `refs/panoptes/lanes/<lane>/copy-files` and restored without staging
   them. Other ignored files are not captured and may be removed with the
   worktree.
4. **A fresh worktree by default, the main checkout on purpose.** A lane
   normally gets its own worktree at `~/Panoptes/lanes/<lane>/<repo>`, on a
   `panoptes/<lane>` branch; the central folder is fixed (Settings covers
   appearance and notifications, not this yet). For big repos it can point at
   the main checkout instead, as an explicit opt-in that is clearly marked.
5. **Pi subagents stay nested.** Pi's own subagents will appear under the
   lane's session, read-only (#227). The proposed Panoptes subsessions that
   shared the parent's worktree and returned a result were dropped (#58).
6. **The word is "lane".** "Workspace" was the working name, but it already
   means something different in VS Code, Zed and Cargo, and the clash would
   confuse both users and agents reading the code.

**Sessions stay the harness's.** A Panoptes session is its own record that
references a harness session; it is never the Pi session itself, and Pi
sessions stay an implementation detail of the adapter. Navigation splits in
two: projects, lanes and their sessions form a collapsible **Lanes** tree, and
every harness session found on disk appears in a flat **All sessions** list
that you can adopt from.

All of this follows the standing rule that decisions live in Rust and the UI
only renders: the one-live-session rule, lane state and what appears where are
worked out in the core.

**Implementation status.** #228 made the one-writer rule real: project `+`
creates a lane from a prompt and starts its first session. The write lock,
queue and non-holder read-only sessions remain gone. The later `/new` and
`/resume` flows retain conversation history without weakening that rule.

**Deferred on purpose.** What happens when the base branch moves underneath
a lane, and remote or cloud execution. The harness adapter boundary keeps the
second open without changing the model.

## Build the lane spine fast; choose one bet after real use

*Decided 2026-11-07.* The candidates and the review are in
[bets.md](bets.md).

A worktree per task is table stakes: Conductor, Crystal, Terragon and others
already do it. Building it well earns Panoptes nothing on its own, so the
spine is built fast and solid rather than polished. What sets Panoptes apart
has to come from one chosen bet, picked after two weeks of real use of the
spine, not before. Until then no bet gets more than the work the spine needs
anyway.

## Review the lane from tree snapshots

*Lane scope implemented with #203; later scopes remain in #201.*

The review pane is a read-only, workspace-level surface beside the focused
conversation. Its file list and lazily loaded hunks come from two git trees,
not from edit-tool records, so shell edits and untracked files cannot disappear
from review. Active lanes snapshot their worktree through the existing private
index; archived lanes use their checkpoint or clean branch. The visible scope
always names its comparison.

A lane review contains ordered worktree sections. Every section resolves its
own merge-base and tree range, and every hunk request names `(lane, project,
path, range)`, so equal paths in a future multi-repo lane cannot collide. The
first UI renders one section without introducing groups or multi-repo creation.
Pure state in `model/review.rs` owns stable selection, expansion, hunk caching
and response generations; GPUI views only render it.

Each worktree also reports its local base's new commits and a merge preview
(#210). An advanced compatible base reports clean; conflicts name literal
NUL-delimited paths; an unsupported or failed preview says unavailable rather
than looking clean. The preview uses an ephemeral `commit-tree` object and
`merge-tree --write-tree` against the reviewed snapshot, never a worktree merge.
It does not fetch, rebase, update a ref, or touch HEAD, the real index, stash or
config. Snapshot invocations use unique private indexes so overlapping review
requests cannot corrupt one another. Each preview whose base has advanced writes
a loose commit and tree, so only the review on screen is refreshed by a settled
run; a hidden one is read again when it is opened. The objects are unreachable
and reclaimed by the next `git gc`, which is the price of never moving a ref.

Zed informs the ambient change summary, continuous file sections and keyboard
navigation, but its agent action log is not Panoptes' truth and its editable
buffers, staging and immediate reject actions are not copied. Turn checkpoints
are built on this pane; attribution, review marks, comments and checkpointed
reverts remain later work in that order.

## Per-turn checkpoints on plain git, not Jujutsu

*Decided 2026-11-07; implemented with #204.*

A turn begins at `Service::prompt`; a harness-only `RunStarted` with no
prompt does not invent one or block event forwarding. A writable lane session
keeps a before and after snapshot of every turn. Each snapshot is a complete
tree written through a private copy of the real index, then made reachable as
a commit under a generated `refs/panoptes/lanes/<lane>/turns/<n>/{before,after}`
ref. The repository stays plain git; Panoptes does not use Jujutsu (jj).

- **Boundary quality is explicit.** An immediate prompt snapshots before the
  harness receives it, so the boundary is exact. A queued prompt refreshes on
  its real `RunStarted` and is recorded as observed. A steering prompt modifies
  the current run and invents no turn. Unavailable boundaries carry an error
  and are never shown as an empty diff.
- **Turn identity is lane-level; trees are worktree-level.** A turn spans
  session history, while `turn_worktrees` holds one tree/ref pair per project.
  Snapshots are parented to the previous after ref, or HEAD for the first turn,
  and are never claimed to be atomic across repositories.
- **Durable state lives in Panoptes' store.** A migration appends `turns` and
  `turn_worktrees`. Intent and deterministic ref names are written before any
  Git ref. Startup adopts interrupted refs, snapshots an existing worktree, uses
  an archive checkpoint when an archived worktree is gone, marks unrecoverable
  rows, settles them as stopped, and removes only generated orphan refs. One
  turn that cannot be restored is settled as failed with its reason rather than
  blocking the rest at every launch.
- **Only a turn that ran is settled by a run.** A settled run closes the turn it
  ran, never a follow-up still waiting in the harness's queue, and a prompt
  answered while another turn is active waits its turn instead of replacing it.
  A turn that waited, like one the harness queued, takes its boundary at its own
  `RunStarted` and is recorded as observed: the snapshot taken at prompt time
  belongs to the run still going.
  Closing or replacing a session settles the turns it held from the worktree as
  it is then, and drops the ones that never started.
- **Nothing the user or agent sees changes.** The real index, `HEAD`, branch,
  stash and config are untouched. The refs are not branches, so a normal push
  never sends them. Refs survive archive and are removed by the existing lane
  deletion cleanup.
- **Snapshots are complete.** Ignored files stay excluded, but large files are
  retained: skipping them would make later review or undo unsafe. A new nested
  repository fails the snapshot loudly rather than becoming a bare gitlink.
  Configured `copyFiles` remain archive concerns, not turn checkpoints.
- **Failures do not block a prompt.** A snapshot or store failure records an
  unavailable boundary and the run proceeds. Settled, stopped, failed and
  exited runs share an idempotent after path, so a late event from a replaced
  process cannot overwrite a newer turn. Read-only previews and raw sessions
  take no checkpoints.

**Why not jj.** jj's operation log gives undo-like history, but in a
colocated repo git sees a detached `HEAD`, the agents (trained on git) run git
commands that jj only picks up on its next command, extra jj workspaces are not
git worktrees so git tooling inside them fails, bookmarks must be moved by
hand before `panoptes/<lane>` can be pushed, git hooks don't run, and the jj
binary (or the unstable `jj-lib` crate) becomes a dependency. All of that
buys what a few git plumbing calls already give. The two don't compose: with
jj the checkpoints would be redundant.

**What it changes in #20.** Archiving a dirty lane offers commit, stash, or
**keep as checkpoint**: the tree is saved to a final checkpoint ref and the
worktree is removed, and restoring creates a fresh worktree and puts the tree
back as uncommitted changes, leaving the branch where it was. That replaces
moving the tree aside to a holding area. #20 reserves the
`refs/panoptes/lanes/` namespace, keyed by lane name rather than store id,
because debug and release builds keep separate stores that can reuse an id in
the same repository. Per-turn checkpoints use the one-live-session boundary
from #228, not #22's removed write lock.

## One live session per lane, with retained history

*Decided 2026-11-11 (#228), implemented the same day; revised 2026-11-19.* A
lane starts with one session and can hold earlier sessions over time. Clicking
`+` on a project asks for a prompt and creates a lane with it, with the
existing clearly marked option to use the main checkout when a worktree is
unnecessary.

- One live session means one writer by construction. The write lock, read-only
  non-holder sessions and lock queue are removed rather than repaired.
- `/new` stops the current process and creates a fresh Panoptes session record
  and Pi conversation in the same lane. `/resume` offers only that lane's own
  earlier records, newest first, and reopens the selected conversation's file.
- Clicking an earlier session opens a read-only preview without stopping the
  live one. Sending a message from the preview stops the live session, resumes
  the previewed conversation with normal tools and delivers that message.
- Neither flow uses Pi's in-process `new_session` or `switch_session` RPC. A
  record keeps its harness identity, timeline, title and usage permanently.
- Changing sessions is refused while the current agent streams, waits for a
  question or approval, or has queued steering or follow-up messages.
- Pi's own subagents appear nested under their parent and are read-only (#227).
- The store schema does not forbid multiple records; the service enforces one
  live session and at most one read-only preview per lane. A preview never
  holds its lane: starting, archiving or deleting closes or ignores it.

This supersedes #22's write-lock decision and the earlier subsession proposal.
It is intentionally simpler than worker fan-out: Panoptes observes one agent's
work in each lane.

**Another lane's session can still write.** Nothing stops a lane's agent from
editing another lane's worktree with its tools. While an open session that
has edited a lane (a finished edit tool call naming a file in its worktree)
is Running or Waiting, the core refuses that lane's own session, so it never
gets two writers. The sidebar says so instead of offering Start session, and
every active lane shows its uncommitted files and commits ahead, so a lane
without a session no longer looks empty when it holds work. This is kept in
memory for as long as the writing session runs; recording who wrote what, and
showing it, is left to the bet review (2026-11-12).

## The design system lives in the repo as a package

*Decided 2026-11-05. Superseded by the move to GPUI: the package is gone and
its export is vendored into `crates/panoptes-app`.*

`packages/design-system` (`@panoptes/design-system`, a pnpm workspace
package) holds the tokens, fonts, logos, app icons and components, exported
from the design tool. Keeping it in the same repository means one commit can
change a token and the screens that use it, and it can still be published to
npm later. The app maps its tokens onto Tailwind colour names in
`src/index.css`, so utilities and components share one source of values.

The bundled app icon is `pupil-on-bone`, the design system's default; the
other seven are in the package for a future icon picker.

## Known gap: RPC cannot list sessions

Pi's RPC mode has no command that lists sessions; only the SDK's
`SessionManager.list()` does. `panoptes-pi` therefore reads Pi's documented
session folder (`~/.pi/agent/sessions/--<cwd>--/<ts>_<id>.jsonl`, or the
configured `sessionDir`). That ties the adapter to Pi's on-disk format, which
is less stable than the protocol. The scanner sits behind
`Harness::list_sessions`, so it can be replaced once Pi gains a
`list_sessions` command, which would be worth proposing upstream.

## Composer: settings per session, files by path

Model and thinking level belong to the session, as they do in Pi: changing
them in one session leaves the others and new sessions alone, and a change
applies from the next message. A global default is still missing; Settings
today covers appearance and notifications. Shift+Tab cycles the thinking
level, as in Pi's terminal UI. While the agent runs, Enter steers it and
Option+Enter queues a follow-up for when it finishes; both show as chips
until the agent reads them.

Images go to the model inline. Any other attached file is appended to the
prompt as an `@path` mention, so the agent reads it with its own tools and no
harness needs a file-attachment command. @-mentions are plain text in the
field, drawn as chips by a backdrop behind it. Typing `/` as the first token
offers the commands the harness can run through a normal prompt (Pi's
extension commands, prompt templates and skills, listed over RPC). Built-in
terminal UI commands are not offered, since they do not run over RPC.

## Open gaps

- No automated test drives the real app window; the UI is tested through its
  store and `model/`, and the backend through the service and a fake Pi.
- The real Pi is only exercised by `scripts/smoke.sh`, by hand. It is not
  part of `scripts/check.sh` because it needs a logged-in Pi and makes a
  model call.
- Pi is not bundled yet. The app uses the installed `pi` and looks for a
  bundled one first, so shipping one later means adding files, not code.
