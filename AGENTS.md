# Agent guide

Panoptes is a macOS desktop app for coding agents. It starts with Pi and is
built so other harnesses can be added. It is a Tauri v2 app: a Rust core and
a React UI. Read [docs/decisions.md](docs/decisions.md) for why it is built
this way.

## Layout

| Path | Owns |
|---|---|
| `crates/panoptes-harness` | The neutral interface: `Harness` and `AgentSession` traits, `HarnessEvent`, `Message`, `Block`, `Question` |
| `crates/panoptes-pi` | The Pi adapter: finds `pi`, lists sessions from Pi's session folder, drives `pi --mode rpc` |
| `src-tauri/src/service.rs` | Open sessions (one process each), event forwarding, and the only user of the store |
| `src-tauri/src/store.rs` | Panoptes' own data in SQLite: schema version, migrations, backup, pins, projects, lanes and session records |
| `src-tauri/src/lanes.rs` | Lane worktree creation and rollback, safe archiving, checkpoints and restoration |
| `src-tauri/src/projects.rs` | Which git repository a folder belongs to (worktrees included), a new project's defaults, and the rules its settings follow |
| `src-tauri/src/lib.rs` | Thin Tauri commands over the service, and nothing else |
| `src-tauri/src/eventlog.rs` | The local event log for evaluating the app: one JSON line per event, never prompt or file contents |
| `src-tauri/src/files.rs` | Workspace file listing for @-mentions, and reading dropped files |
| `src/conversation.ts` | Pure reducer: harness events to conversation state |
| `src/composer.ts` | Pure composer logic: mention parsing, fuzzy file matching, labels |
| `src/turns.ts` | Pure transcript logic: messages to turns, tool rows and folded-work summaries (with `diff.ts` and `markdown.ts`) |
| `src/store.ts` | Zustand store: talks to the backend through `src/api.ts` |
| `src/components/` | Rendering only |
| `src/dev/mockBackend.ts` | Fixture backend used when the UI runs in a plain browser |
| `packages/design-system` | Tokens, fonts, logos, app icons and React components; its `GUIDE.md` is the brand and usage guide |

## Issue intake

Until every issue on [the gate list](docs/backlog.md#the-gate) is closed, file
GitHub issues for bugs only. Do not file feature or enhancement issues. Put feature ideas in the
[observation log](docs/bets.md#observation-log) instead. Parked features wait
for the bet review on 2026-12-05. See [the direction decision](docs/decisions.md#observe-pi-do-not-orchestrate-agents).

## Rules

- **Harness details stay inside their adapter.** Nothing Pi-specific (its
  event names, entry format, CLI flags, file layout) may appear in
  `panoptes-harness`, `src-tauri` or `src/`. If the UI needs something new,
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
  kind survives. Keep `src/types.ts` in step with the Rust types.
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
- **Style only through the design system.** Use its components first
  (`Button`, `Input`, `AgentRow`, `StatusPill`, `LoopMark`, `Icon`, ...),
  then Tailwind utilities mapped to its tokens (`bg-surface`,
  `text-ink-muted`, `border-line`, `text-caption`). No raw colours, no
  Tailwind palette colours (`neutral-500` and the like), no emoji. Status
  words are exactly Running, Waiting, Failed and Done; a session that has
  not run yet is `idle` on the wire and shows no word. Files under
  `packages/design-system` are exported by the design tool; change the design
  system there and re-export rather than editing them here.
- Logic goes where it can be tested without a window: the adapter, the
  service, or the reducer. Keep Tauri commands and React components thin.
- Add or update a test with every behaviour change. The Pi adapter is tested
  against `crates/panoptes-pi/tests/fixtures/fake_pi.py`; extend the fake
  rather than needing a real Pi.

## Before you finish

Run `pnpm check`. It runs lint, type checks, Rust formatting, clippy (warnings
are errors) and every test; the pre-push hook and CI run it too. `pnpm format`
fixes formatting of the UI code; `cargo fmt --all` fixes the Rust.

To see the UI without an agent, run `pnpm dev` and open http://localhost:1420
in a browser. `pnpm smoke` does one real round trip with the installed Pi and
fails if the event or history contract broke; run it after changing the
adapter, since `pnpm check` only talks to the fake Pi.
