# Panoptes

A desktop environment for coding agents on macOS. It starts with
[Pi](https://github.com/earendil-works/pi) and is built so other agent
harnesses can be added later.

Early days: this is the walking skeleton. You get a session sidebar and a
plain streaming chat. The styling is deliberately bare and will be replaced
by a design system.

## What works

- Lists your existing Pi sessions, grouped as **Running**, **Pinned**, then by
  project (git repository root, or folder).
- Opens or resumes any session, and starts new ones in a folder you pick.
  Sessions are Pi's own files, so the CLI (`pi --continue`, `/resume`) sees
  the same history.
- Streams replies (rendered as markdown), thinking and tool calls. While the
  agent is working, Enter steers it and Esc stops it.
- Questions from Pi extensions (confirm, select, input, editor) appear above
  the composer and are answered in place.
- Each open session runs in its own `pi --mode rpc` process; opening the same
  session twice never starts a second one.

## Requirements

- macOS 13 or later
- [Pi](https://github.com/earendil-works/pi) 1.x installed (`pi --version`).
  Set `PANOPTES_PI` to point at a specific `pi` binary.
- For development: Rust (via [rustup](https://rustup.rs)), Node 22+, pnpm,
  and Xcode Command Line Tools.

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
cargo run -p panoptes-pi --example smoke     # one real round trip with your Pi
```

There is no CI. A pre-push hook (installed by `pnpm install`) runs
`pnpm check`. Agents: see [AGENTS.md](AGENTS.md); decisions are in
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
