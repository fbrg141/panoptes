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
| `src-tauri/src/service.rs` | Open sessions (one process each), event forwarding, Panoptes' index |
| `src-tauri/src/lib.rs` | Thin Tauri commands over the service, and nothing else |
| `src-tauri/src/files.rs` | Workspace file listing for @-mentions, and reading dropped files |
| `src/conversation.ts` | Pure reducer: harness events to conversation state |
| `src/composer.ts` | Pure composer logic: mention parsing, fuzzy file matching, labels |
| `src/turns.ts` | Pure transcript logic: messages to turns, tool rows and folded-work summaries (with `diff.ts` and `markdown.ts`) |
| `src/store.ts` | Zustand store: talks to the backend through `src/api.ts` |
| `src/components/` | Rendering only |
| `src/dev/mockBackend.ts` | Fixture backend used when the UI runs in a plain browser |
| `packages/design-system` | Tokens, fonts, logos, app icons and React components; its `GUIDE.md` is the brand and usage guide |

## Rules

- **Harness details stay inside their adapter.** Nothing Pi-specific (its
  event names, entry format, CLI flags, file layout) may appear in
  `panoptes-harness`, `src-tauri` or `src/`. If the UI needs something new,
  add a neutral type to `panoptes-harness` and map it in the adapter. A
  feature only one harness has travels as `HarnessEvent::Raw` until it earns
  a neutral type.
- **The event model is the contract.** The UI rebuilds streaming state from
  `HarnessEvent`s. `MessageCompleted` replaces whatever was streamed under
  the same id, `RunSettled` means the run is finished, and the history is then
  reloaded to swap live ids for stable ones. A session's status (Running,
  Waiting, Failed, Done) is worked out once, in the service, and sent as
  `statusChanged`; the UI never derives it. Errors reach the UI as
  `{ kind, message }`, and adapters mark theirs with `HarnessError` so the
  kind survives. Keep `src/types.ts` in step with the Rust types.
- **Sessions belong to the harness.** Pi keeps its sessions in its own files
  (`~/.pi/agent/sessions` by default), shared with the CLI. Panoptes never
  copies or rewrites them. What Panoptes adds (pins for now) lives in its own
  `index.json` in the app data folder, keyed by `<harness>:<session id>`.
- **One process per open session.** `Service::open` serialises opens of the
  same key. Never start a second process on a session that is already open.
- **Style only through the design system.** Use its components first
  (`Button`, `Input`, `AgentRow`, `StatusPill`, `LoopMark`, `Icon`, ...),
  then Tailwind utilities mapped to its tokens (`bg-surface`,
  `text-ink-muted`, `border-line`, `text-caption`). No raw colours, no
  Tailwind palette colours (`neutral-500` and the like), no emoji. Status
  words are exactly Running, Waiting, Failed and Done. Files under
  `packages/design-system` are exported by the design tool; change the design
  system there and re-export rather than editing them here.
- Logic goes where it can be tested without a window: the adapter, the
  service, or the reducer. Keep Tauri commands and React components thin.
- Add or update a test with every behaviour change. The Pi adapter is tested
  against `crates/panoptes-pi/tests/fixtures/fake_pi.py`; extend the fake
  rather than needing a real Pi.

## Before you finish

Run `pnpm check`. It runs lint, type checks, Rust formatting, clippy (warnings
are errors) and every test, and the pre-push hook runs it too. `pnpm format`
fixes formatting of the UI code; `cargo fmt --all` fixes the Rust.

To see the UI without an agent, run `pnpm dev` and open http://localhost:1420
in a browser. `cargo run -p panoptes-pi --example smoke` does one real round
trip with the installed Pi.
