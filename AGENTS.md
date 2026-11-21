# Agent guide

Panoptes is a macOS desktop app for coding agents. It starts with Pi and is
built so other harnesses can be added. It is one Rust app with a GPUI UI.
Read [docs/decisions.md](docs/decisions.md) for why it is built this way.

## Layout

| Path | Owns |
|---|---|
| `crates/panoptes-harness` | The neutral interface: `Harness` and `AgentSession` traits, `HarnessEvent`, `Message`, `Block`, `Question` |
| `crates/panoptes-pi` | The Pi adapter: finds `pi`, lists sessions from Pi's session folder, drives `pi --mode rpc` |
| `crates/panoptes-core` | The backend with no UI dependency |
| `crates/panoptes-app` | The GPUI app: `backend.rs` runs the service on tokio, `store.rs` holds the app state, `model/` the pure logic (tested), `ds.rs` the design system's components, and `views/` render |
| `crates/panoptes-core/src/service/` | The `Service` facade: open sessions (one process each), event forwarding, and the only user of the store |
| `crates/panoptes-core/src/store.rs` | Panoptes' own data in SQLite: schema version, migrations, backup, pins, projects, lanes, session records, profiles and model-picker preferences |
| `crates/panoptes-core/src/lanes.rs` | Lane worktree creation and rollback, safe archiving, checkpoints and restoration |
| `crates/panoptes-core/src/changes.rs` | Read-only tree diffs: lane file lists, totals and lazily loaded hunks |
| `crates/panoptes-core/src/cleanup.rs` | On-demand worktree disk usage and safe idle-lane suggestions |
| `crates/panoptes-core/src/git.rs` | Bounded Git process execution, including descendants and large output |
| `crates/panoptes-core/src/projects.rs` | Which git repository a folder belongs to (worktrees included), a new project's defaults, and the rules its settings follow |
| `crates/panoptes-core/src/eventlog.rs` | The local event log for evaluating the app: one JSON line per event, never prompt or file contents |
| `crates/panoptes-core/src/files.rs` | Workspace file listing for @-mentions, and reading dropped files |
| `crates/panoptes-app/src/model/` | Pure logic: `conversation.rs` folds harness events into state, `composer.rs`, `turns.rs`, `diff.rs`, palettes, panes and the sidebar |
| `crates/panoptes-app/src/views/` | Rendering only |
| `crates/panoptes-app/assets` | The design system's tokens (`tokens.json`) and fonts; `src/design_assets.rs` holds its icons and logos |

## Issue intake

Until the bet review on 2026-12-05, file GitHub issues for bugs only. Do not
file feature or enhancement issues; put feature ideas in the
[observation log](docs/bets.md#observation-log) instead, where parked features
wait for the review. The one exception is a small fix for pain hit in daily
use that the observation log records ([backlog](docs/backlog.md#intake-until-the-bet-review)). See [the direction decision](docs/decisions.md#observe-pi-do-not-orchestrate-agents).

## Rules

- **Harness details stay inside their adapter.** Nothing Pi-specific (its
  event names, entry format, CLI flags, file layout) may appear in
  `panoptes-harness`, `panoptes-core` or `panoptes-app`. If the UI needs
  something new,
  add a neutral type to `panoptes-harness` and map it in the adapter. A
  feature only one harness has travels as `HarnessEvent::Raw` until it earns
  a neutral type.
- **The event model is the contract.** The UI rebuilds streaming state from
  `HarnessEvent`s. `MessageCompleted` replaces whatever was streamed under
  the same id. `ToolCallUpdated` replaces the tool call with the same id: a
  `ToolCall` holds its status and output, there are no result messages, and
  the UI keeps no tool state of its own. An `Approval` names the tool call it
  is about and is answered with allow or reject. `QueueChanged` lists the
  prompts still waiting, by id. `RunSettled` means the run is finished, and
  the history is then reloaded as the authoritative message list; a message
  keeps the id it streamed under, so nothing is swapped. A session's status (Running,
  Waiting, Failed, Done) is worked out once, in the service, and sent as
  `statusChanged`; the UI never derives it. Opening a session that is
  already open also returns its status and the events to replay (the run's
  start, waiting questions and approvals), so a reloaded window catches up. Errors reach the UI as
  `{ kind, message }`, and adapters mark theirs with `HarnessError` so the
  kind survives.
- **Sessions belong to the harness.** Pi keeps its sessions in its own files
  (`~/.pi/agent/sessions` by default), shared with the CLI. Panoptes never
  copies or rewrites them. What Panoptes adds (projects, lanes, its own
  session records, and pins for conversations no lane has adopted) lives in
  its own store, `panoptes.db` in the app data folder. A session in a lane is
  keyed `panoptes:<id>`, an id given before its harness starts; any other
  conversation `<harness>:<session id>`. A session record points at its
  harness conversation and never copies it.
- **Schema changes are migrations.** Append a step to `MIGRATIONS` in
  `store.rs`; never edit one that has shipped. A store that can't be read or
  migrated stops the app with an error, never a silent fresh start.
  Debug builds use their own store (`<app data>/dev`) and lanes folder
  (`~/Panoptes/dev/lanes`), so a step can be reworked freely until it ships.
- **One process per open session.** Open, create, close and adopt each hold
  the session key's lifecycle lock in `Service`, so a process never starts
  while another for the same session is starting or shutting down. Never
  start a second process on a session that is already open.
- **Style only through the design system.** Use its components in `ds.rs`
  first (`Button`, `kbd`, `status_pill`, `loop_mark`, `icon`, the overlay
  parts, ...), then the palette's tokens (`p.surface`, `p.ink_muted`,
  `p.line`) and the type scale (`t_body`, `t_caption`, ...). No raw colours
  and no emoji. Sizes go through `crate::zoom::px`, so the window zooms.
  Status words are exactly Running, Waiting, Failed and Done; a session that
  has not run yet is `idle` on the wire and shows no word. `assets/tokens.json`
  and `src/design_assets.rs` come from the design tool's export; replace them
  from a new export rather than editing values by hand.
- Logic goes where it can be tested without a window: the adapter, the
  service, the app's store or `model/`. Keep views thin.
- Add or update a test with every behaviour change. The Pi adapter is tested
  against `crates/panoptes-pi/tests/fixtures/fake_pi.py`; extend the fake
  rather than needing a real Pi.

## Before you finish

Run `sh scripts/check.sh`. It runs Rust formatting, clippy (warnings are
errors) and every test, and the pre-push hook runs it too. `cargo fmt --all`
fixes formatting.

`sh scripts/smoke.sh` does one real round trip with the installed Pi and
fails if the event or history contract broke; run it after changing the
adapter, since the check only talks to the fake Pi.
