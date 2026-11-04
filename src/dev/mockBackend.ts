// A fake backend for opening the UI in a plain browser (`pnpm dev`, then
// http://localhost:1420). Handy for design work without running agents.
// Never loaded inside the Tauri app.

import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { HarnessEvent, Message, SidebarSession } from "../types";

const now = Date.now();
const hour = 3_600_000;

const session = (over: Partial<SidebarSession> & Pick<SidebarSession, "id" | "title">): SidebarSession => ({
  key: `pi:${over.id}`,
  harness: "pi",
  locator: `/sessions/${over.id}.jsonl`,
  cwd: "/Users/you/code/panoptes",
  createdAt: now - 48 * hour,
  updatedAt: now - hour,
  messageCount: 12,
  parentLocator: null,
  projectRoot: "/Users/you/code/panoptes",
  pinned: false,
  open: false,
  ...over,
});

const sessions: SidebarSession[] = [
  session({ id: "a", title: "Sketch the harness interface", updatedAt: now - 5 * 60_000 }),
  session({ id: "b", title: "Release checklist", pinned: true, updatedAt: now - 30 * hour }),
  session({ id: "c", title: "Fix streaming scroll jump", updatedAt: now - 3 * hour }),
  session({ id: "d", title: "Parse invoice PDFs", cwd: "/Users/you/code/pdf-review", projectRoot: "/Users/you/code/pdf-review", updatedAt: now - 26 * hour }),
  session({ id: "e", title: "Try a new tokenizer", cwd: "/Users/you/scratch", projectRoot: "/Users/you/scratch", updatedAt: now - 9 * 24 * hour }),
];

const history: Message[] = [
  { id: "m1", role: "user", blocks: [{ type: "text", text: "What files are in src?" }], timestamp: now, error: null },
  {
    id: "m2",
    role: "assistant",
    blocks: [
      { type: "thinking", text: "I should list the directory." },
      { type: "toolCall", callId: "t1", name: "bash", args: { command: "ls src" } },
    ],
    timestamp: now,
    error: null,
  },
  { id: "m3", role: "tool", blocks: [{ type: "toolResult", callId: "t1", name: "bash", output: "App.tsx\napi.ts\nstore.ts", isError: false }], timestamp: now, error: null },
  { id: "m4", role: "assistant", blocks: [{ type: "text", text: "There are three files: App.tsx, api.ts and store.ts." }], timestamp: now, error: null },
];

async function stream(key: string, text: string) {
  const send = (event: HarnessEvent) => emit("session-event", { key, event });
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  await send({ type: "runStarted" });
  const user: Message = { id: `u${Date.now()}`, role: "user", blocks: [{ type: "text", text }], timestamp: Date.now(), error: null };
  await send({ type: "messageCompleted", message: user });
  await send({ type: "messageStarted", messageId: "live", role: "assistant" });
  await send({ type: "blockStarted", messageId: "live", index: 0, block: { type: "text", text: "" } });
  const reply = "This is a mocked reply, streamed word by word.";
  for (const word of reply.split(" ")) {
    await wait(80);
    await send({ type: "blockDelta", messageId: "live", index: 0, kind: "text", delta: `${word} ` });
  }
  history.push(user, { id: `r${Date.now()}`, role: "assistant", blocks: [{ type: "text", text: reply }], timestamp: Date.now(), error: null });
  await send({ type: "runSettled", aborted: false });
}

export function installMockBackend() {
  const open = new Set<string>();
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, string>;
      switch (cmd) {
        case "app_info":
          return { harnesses: [{ id: "pi", program: "mock", version: "1.4.0", supported: true }], errors: [] };
        case "list_sessions":
          return sessions.map((s) => ({ ...s, open: open.has(s.key) }));
        case "open_session":
          open.add(a.key);
          return { key: a.key, session: { harness: "pi", id: a.key.slice(3), locator: a.locator, cwd: a.cwd }, history };
        case "session_history":
          return history;
        case "send_prompt":
          void stream(a.key, a.text);
          return null;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
