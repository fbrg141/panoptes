// A fake backend for opening the UI in a plain browser (`pnpm dev`, then
// http://localhost:1420). Handy for design work without running agents.
// Never loaded inside the Tauri app.

import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Message, ModelInfo, SessionSettings, SessionUpdate, SidebarSession } from "../types";

const now = Date.now();
const hour = 3_600_000;

const session = (
  over: Partial<SidebarSession> & Pick<SidebarSession, "id" | "title">,
): SidebarSession => ({
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
  session({
    id: "d",
    title: "Parse invoice PDFs",
    cwd: "/Users/you/code/pdf-review",
    projectRoot: "/Users/you/code/pdf-review",
    updatedAt: now - 26 * hour,
  }),
  session({
    id: "e",
    title: "Try a new tokenizer",
    cwd: "/Users/you/scratch",
    projectRoot: "/Users/you/scratch",
    updatedAt: now - 9 * 24 * hour,
  }),
];

type ToolCall = Extract<Message["blocks"][number], { type: "toolCall" }>;

const call = (
  callId: string,
  name: string,
  kind: ToolCall["kind"],
  args: Record<string, unknown>,
  subject: string | null,
  edits: ToolCall["edits"] = [],
): ToolCall => ({
  type: "toolCall",
  callId,
  name,
  args,
  kind,
  subject,
  edits,
});

const msg = (
  id: string,
  role: Message["role"],
  blocks: Message["blocks"],
  at: number,
): Message => ({
  id,
  role,
  blocks,
  timestamp: now - at,
  error: null,
  stopped: false,
});

const result = (callId: string, name: string, output: string, isError = false) =>
  ({ type: "toolResult", callId, name, output, isError }) as const;

const RETRY_OLD = `      {message.error && (
        <div className="flex items-center gap-2 text-error">
          <Icon name="circle-alert" size={14} />
          {message.error}
        </div>
      )}`;
const RETRY_NEW = `      {message.error && (
        <div className="flex items-center gap-2 text-error">
          <Icon name="circle-alert" size={14} />
          <span className="flex-1">{message.error}</span>
          {last && (
            <Button size="sm" icon="rotate-ccw" onClick={retry}>
              Retry
            </Button>
          )}
        </div>
      )}`;

const history: Message[] = [
  msg("m1", "user", [{ type: "text", text: "What files are in src?" }], 20 * 60_000),
  msg(
    "m2",
    "assistant",
    [
      { type: "thinking", text: "I should list the directory." },
      call("t1", "bash", "execute", { command: "ls src" }, "ls src"),
    ],
    20 * 60_000 - 2_000,
  ),
  msg("m3", "tool", [result("t1", "bash", "App.tsx\napi.ts\nstore.ts")], 20 * 60_000 - 3_000),
  msg(
    "m4",
    "assistant",
    [{ type: "text", text: "There are three files: `App.tsx`, `api.ts` and `store.ts`." }],
    20 * 60_000 - 5_000,
  ),
  msg(
    "m5",
    "user",
    [
      {
        type: "text",
        text: "Failed turns should have a Retry button that resends the last prompt.",
      },
    ],
    10 * 60_000,
  ),
  msg(
    "m6",
    "assistant",
    [
      {
        type: "thinking",
        text: "Find where message errors render, then add a **Retry** next to them.",
      },
      call(
        "t2",
        "read",
        "read",
        { path: "src/components/ChatPane.tsx" },
        "src/components/ChatPane.tsx",
      ),
      call("t3", "read", "read", { path: "src/conversation.ts" }, "src/conversation.ts"),
      call("t4", "grep", "search", { pattern: "message.error" }, "message.error"),
    ],
    10 * 60_000 - 4_000,
  ),
  msg(
    "m7",
    "tool",
    [
      result("t2", "read", RETRY_OLD),
      result("t3", "read", "export interface Conversation {\n  messages: Message[];\n}"),
      result("t4", "grep", "src/components/ChatPane.tsx:202:      {message.error && ("),
    ],
    10 * 60_000 - 6_000,
  ),
  msg(
    "m8",
    "assistant",
    [
      call(
        "t5",
        "edit",
        "edit",
        { path: "src/components/ChatPane.tsx" },
        "src/components/ChatPane.tsx",
        [{ path: "src/components/ChatPane.tsx", oldText: RETRY_OLD, newText: RETRY_NEW }],
      ),
      call("t6", "bash", "execute", { command: "pnpm test" }, "pnpm test"),
    ],
    10 * 60_000 - 30_000,
  ),
  msg(
    "m9",
    "tool",
    [
      result("t5", "edit", "Edited src/components/ChatPane.tsx"),
      result(
        "t6",
        "bash",
        " RUN  v3.2.4\n ✓ src/composer.test.ts (18 tests) 41ms\n ✓ src/store.test.ts (12 tests) 33ms\n ✓ src/conversation.test.ts (9 tests) 12ms\n\n Test Files  3 passed (3)\n      Tests  39 passed (39)",
      ),
    ],
    10 * 60_000 - 70_000,
  ),
  msg(
    "m10",
    "assistant",
    [
      {
        type: "text",
        text: "Added a **Retry** action to failed turns. It resends the turn's prompt through the same path as the composer, so queueing still applies.\n\n- `ChatPane.tsx`: Retry button next to the error, on the last failed turn only\n- Tests pass",
      },
    ],
    10 * 60_000 - 78_000,
  ),
];

const models: ModelInfo[] = [
  {
    id: "anthropic/claude-sonnet-4-5",
    name: "Claude Sonnet 4.5",
    provider: "anthropic",
    contextWindow: 200_000,
    images: true,
  },
  {
    id: "anthropic/claude-haiku-4-5",
    name: "Claude Haiku 4.5",
    provider: "anthropic",
    contextWindow: 200_000,
    images: true,
  },
  {
    id: "openai/gpt-5.1-codex",
    name: "GPT-5.1 Codex",
    provider: "openai",
    contextWindow: 400_000,
    images: true,
  },
  {
    id: "local/qwen3-coder",
    name: "Qwen3 Coder",
    provider: "local",
    contextWindow: 32_000,
    images: false,
  },
];

const settings: SessionSettings = {
  model: models[0],
  thinkingLevel: "medium",
  thinkingLevels: ["off", "minimal", "low", "medium", "high"],
  context: { tokens: 42_300, window: 200_000 },
};

const files = [
  "AGENTS.md",
  "package.json",
  "src/App.tsx",
  "src/api.ts",
  "src/composer.ts",
  "src/components/ChatPane.tsx",
  "src/components/Composer.tsx",
  "src/components/Sidebar.tsx",
  "src/conversation.ts",
  "src/store.ts",
  "src-tauri/src/service.rs",
  "crates/panoptes-pi/src/rpc.rs",
  "docs/decisions.md",
];

async function stream(key: string, text: string) {
  // The real service reports status beside the harness's events.
  const send = (event: SessionUpdate) => emit("session-event", { key, event });
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  await send({ type: "runStarted" });
  await send({ type: "statusChanged", status: "running" });
  const user: Message = {
    id: `u${Date.now()}`,
    role: "user",
    blocks: [{ type: "text", text }],
    timestamp: Date.now(),
    error: null,
    stopped: false,
  };
  await send({ type: "messageCompleted", message: user });
  if (text === "ask") {
    await send({
      type: "question",
      question: {
        id: `q${Date.now()}`,
        title: "Allow `rm -rf build`?",
        message: null,
        kind: { type: "select", options: ["Allow", "Block"] },
        expiresAt: Date.now() + 30_000,
      },
    });
  }
  await send({ type: "messageStarted", messageId: "work", role: "assistant" });
  const thought = "Check the tests first, then answer.";
  await send({
    type: "blockStarted",
    messageId: "work",
    index: 0,
    block: { type: "thinking", text: "" },
  });
  for (const word of thought.split(" ")) {
    await wait(120);
    await send({
      type: "blockDelta",
      messageId: "work",
      index: 0,
      kind: "thinking",
      delta: `${word} `,
    });
  }
  const test = call("live-t", "bash", "execute", { command: "pnpm test" }, "pnpm test");
  await send({ type: "blockStarted", messageId: "work", index: 1, block: test });
  const work = msg(`w${Date.now()}`, "assistant", [{ type: "thinking", text: thought }, test], 0);
  await send({ type: "messageCompleted", message: { ...work, id: "work", timestamp: Date.now() } });
  await send({ type: "toolStarted", callId: "live-t", name: "bash", args: test.args });
  let output = " RUN  v3.2.4";
  for (const line of [
    " ✓ src/composer.test.ts (18 tests) 41ms",
    " ✓ src/store.test.ts (12 tests) 33ms",
    " ✓ src/conversation.test.ts (9 tests) 12ms",
  ]) {
    await wait(text === "slow" ? 4000 : 600);
    output += `\n${line}`;
    await send({ type: "toolUpdated", callId: "live-t", output });
  }
  await send({ type: "toolFinished", callId: "live-t", output, isError: false });
  const tested = msg("r-t", "tool", [result("live-t", "bash", output)], 0);
  await send({ type: "messageCompleted", message: { ...tested, timestamp: Date.now() } });
  await send({ type: "messageStarted", messageId: "live", role: "assistant" });
  await send({
    type: "blockStarted",
    messageId: "live",
    index: 0,
    block: { type: "text", text: "" },
  });
  const reply =
    "This is a **mocked** reply, streamed word by word. Send `ask` to see a question, or `slow` for a long-running tool.";
  for (const word of reply.split(" ")) {
    await wait(80);
    await send({
      type: "blockDelta",
      messageId: "live",
      index: 0,
      kind: "text",
      delta: `${word} `,
    });
  }
  history.push(
    user,
    { ...work, timestamp: Date.now() },
    { ...tested, id: `t${Date.now()}`, timestamp: Date.now() },
    msg(`r${Date.now()}`, "assistant", [{ type: "text", text: reply }], 0),
  );
  await send({ type: "runSettled", aborted: false });
  await send({ type: "statusChanged", status: "done" });
}

export function installMockBackend() {
  const open = new Set<string>();
  let streaming = false;
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, string>;
      switch (cmd) {
        case "app_info":
          return {
            harnesses: [{ id: "pi", program: "mock", version: "1.4.0", supported: true }],
            errors: [],
          };
        case "list_sessions":
          return { sessions: sessions.map((s) => ({ ...s, open: open.has(s.key) })), errors: [] };
        case "open_session":
          if (!open.has(a.key)) {
            void emit("session-event", {
              key: a.key,
              event: { type: "statusChanged", status: "idle" },
            });
          }
          open.add(a.key);
          return {
            key: a.key,
            session: { harness: "pi", id: a.key.slice(3), locator: a.locator, cwd: a.cwd },
            history,
          };
        case "session_history":
          return history;
        case "send_prompt":
          // While a reply streams, further prompts wait their turn; the
          // run settling clears them.
          if (streaming) return "queued";
          streaming = true;
          void stream(a.key, a.text).finally(() => {
            streaming = false;
          });
          return "started";
        // Copies, as the real backend sends fresh JSON every time.
        case "session_settings":
          return { ...settings };
        case "list_models":
          return models;
        case "set_model": {
          const model = models.find((m) => m.id === a.modelId) ?? null;
          settings.model = model;
          settings.thinkingLevels =
            model?.provider === "local" ? ["off"] : ["off", "minimal", "low", "medium", "high"];
          settings.context = { tokens: 42_300, window: model?.contextWindow ?? 0 };
          return { ...settings };
        }
        case "set_thinking_level":
          settings.thinkingLevel = a.level;
          return { ...settings };
        case "list_files":
          return files;
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
