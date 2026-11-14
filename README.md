# Panoptes

A minimal macOS desktop app for the [Pi](https://github.com/earendil-works/pi)
coding agent. It observes what the agent does and presents that work; it does
not plan, delegate or coordinate agents. Sessions stay Pi's own files and
lanes are plain git worktrees and branches. The direction is recorded in
[docs/decisions.md](docs/decisions.md#observe-pi-do-not-orchestrate-agents).

## What works

- Two navigation tabs: **Lanes** is a collapsible project → lane → session
  hierarchy; **All sessions** is one flat, reverse-chronological list of every
  Pi conversation, including ones started from the terminal. External sessions
  can be adopted into lanes without copying or rewriting Pi's files.
- Opens or resumes any session. New sessions start in a lane, as Panoptes
  sessions with their own id; their conversations are still Pi's own files,
  so the CLI (`pi --continue`, `/resume`) sees the same history.
- **+** on a project asks for a prompt and creates a lane with it: its own
  Git worktree and branch, with project file copying and setup. The main
  checkout is a marked opt-in. A lane holds exactly one session, so it has
  one writer; a new task is a new lane.
- Clean lanes can be archived, and dirty work can be committed, stashed or
  saved as a restorable private checkpoint. A lane can also be deleted for
  good, after a confirmation.
- A lane that another lane's session is editing says so, and its own session
  cannot start until that session is Done.
- Streams replies (rendered as markdown), thinking and tool calls as a work
  log. While the agent is working, Enter steers it, Option+Enter queues a
  follow-up and Esc stops it.
- Permission prompts and questions from Pi extensions (confirm, select,
  input, editor) appear above the composer and are answered in place.
- Each open session runs in its own `pi --mode rpc` process; opening the same
  session twice never starts a second one, and stopping a session stops what
  it started.

Decided but not built yet: per-turn checkpoints (#204) and nested Pi
subagents (#227).

## Requirements

- macOS 13 or later
- [Pi](https://github.com/earendil-works/pi) 1.x installed (`pi --version`).
  Set `PANOPTES_PI` to point at a specific `pi` binary.
- For development: Rust (via [rustup](https://rustup.rs)), Node 22+, pnpm,
  and Xcode Command Line Tools.

## Install on your Mac

```bash
pnpm app:install
```

This builds a release, signs it ad hoc (no Apple developer account needed for
a build you made yourself), replaces `/Applications/Panoptes.app` and opens
it. Run it again after pulling to update; it quits a running Panoptes first.

The installed app keeps its own data, apart from `pnpm tauri dev`:

| | Installed app | `pnpm tauri dev` |
|---|---|---|
| Store | `~/Library/Application Support/dev.panoptes.app/panoptes.db` | `…/dev.panoptes.app/dev/panoptes.db` |
| Lane worktrees | `~/Panoptes/lanes` | `~/Panoptes/dev/lanes` |
| Event log | `…/dev.panoptes.app/logs/YYYY-MM-DD.jsonl` | `…/dev.panoptes.app/dev/logs/` |

The event log is Panoptes' own record for evaluating it in daily use: one
JSON line per session start, close or crash, status change, question asked
or answered, lane change, and error shown in the window. It never holds
prompts, replies or file contents, stays on your Mac, and keeps 30 days.
To open the folder, choose Help → Show Logs in Finder, or press ⌘P and pick
Show logs in Finder.

Apps opened from Finder don't get your shell's `PATH`, so at startup the
installed app reads it from your login shell. That is how it finds `pi`,
`node` and whatever a project's setup runs.

## Development

```bash
pnpm install
pnpm tauri dev
```

Opening http://localhost:1420 in a normal browser while the dev server runs
shows the UI with fixture data and no agent (`src/dev/mockBackend.ts`). This
is useful for design work.

```bash
pnpm check                                   # everything: lint, types, formatting, clippy, all tests
pnpm smoke                                   # one real round trip with your Pi; fails if the contract broke
```

CI runs `pnpm check` on macOS for every pull request and every push to
`main`, and a pre-push hook (installed by `pnpm install`) runs it locally. That suite only talks to a scripted stand-in for Pi, so run
`pnpm smoke` after updating Pi or changing the adapter: it needs an installed,
logged-in Pi and makes one small model call. Agents: see [AGENTS.md](AGENTS.md); decisions are in
[docs/decisions.md](docs/decisions.md).

## Layout

| Path | What it is |
|---|---|
| `crates/panoptes-harness` | The harness-neutral interface: sessions, messages, events, and the `Harness` / `AgentSession` traits |
| `crates/panoptes-pi` | The Pi adapter: finds `pi`, lists sessions from Pi's session folder, drives `pi --mode rpc` |
| `src-tauri` | The app: `service.rs` (open sessions, event forwarding), `store.rs` (Panoptes' own data in SQLite), exposed as Tauri commands in `lib.rs` |
| `src` | React UI: `conversation.ts` folds events into state, `store.ts` talks to the app, `components/` renders |

The UI only ever sees the neutral types, so a new harness means a new adapter
crate and nothing else.

## License

MIT
