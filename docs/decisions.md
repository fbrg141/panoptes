# Decisions

The choices that shape Panoptes, and why. Each one is cheap to keep and
expensive to reverse, so change one only on purpose, and record it here.

## Tauri v2, not Electron or a pure Rust UI

*Decided 2026-11-04.*

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

*Decided 2026-11-07.*

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
  (`packages/design-system/tokens/tokens.json`). CSS variables follow from
  that file and are never the only place a value exists.
- **TypeScript keeps no model of its own of a session.** It holds what the
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
on top (pins now; projects, groups, workspaces and tags next) in its own
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

## The design system lives in the repo as a package

*Decided 2026-11-05.*

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
applies from the next message. A global default waits for Settings (#8).
Shift+Tab cycles the thinking level, as in Pi's terminal UI. While the agent
runs, Enter steers it and Option+Enter queues a follow-up for when it
finishes; both show as chips until the agent reads them.

Images go to the model inline. Any other attached file is appended to the
prompt as an `@path` mention, so the agent reads it with its own tools and no
harness needs a file-attachment command. @-mentions are plain text drawn as
chips by a backdrop behind the text field, which keeps the field a native
`textarea`. Slash commands are left for later: they need Pi to list its
commands over RPC.

## Open gaps

- No automated test drives the real app window; the UI is tested through its
  store and reducer, and the backend through the service and a fake Pi.
- Pi is not bundled yet. The app uses the installed `pi` and looks for a
  bundled one first, so shipping one later means adding files, not code.
