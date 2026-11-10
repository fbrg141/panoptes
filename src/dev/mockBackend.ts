// A fake backend for opening the UI in a plain browser (`pnpm dev`, then
// http://localhost:1420). Handy for design work without running agents.
// Never loaded inside the Tauri app.

import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  Answer,
  Lane,
  Message,
  ModelInfo,
  Project,
  QueuedPrompt,
  Session,
  SessionSettings,
  SessionUpdate,
  ToolCall,
  ToolOutput,
} from "../types";

const now = Date.now();
const hour = 3_600_000;

const session = (over: Partial<Session> & { name: string; title: string }): Session => ({
  key: `pi:${over.name}`,
  id: null,
  laneId: null,
  harness: "pi",
  harnessRef: {
    harness: "pi",
    id: over.name,
    locator: `/sessions/${over.name}.jsonl`,
    cwd: over.cwd ?? "/Users/you/code/panoptes",
  },
  cwd: "/Users/you/code/panoptes",
  tags: [],
  createdAt: now - 48 * hour,
  updatedAt: now - hour,
  messageCount: 12,
  parentLocator: null,
  projectRoot: "/Users/you/code/panoptes",
  checkout: "/Users/you/code/panoptes",
  projectId: 1,
  pinned: false,
  archived: false,
  open: false,
  ...over,
});

let sessions: Session[] = [
  session({
    name: "a",
    key: "panoptes:1",
    id: 1,
    laneId: 1,
    cwd: "/Users/you/Panoptes/lanes/reviewable-work/panoptes",
    title: "Sketch the harness interface",
    updatedAt: now - 5 * 60_000,
  }),
  session({ name: "b", title: "Release checklist", pinned: true, updatedAt: now - 30 * hour }),
  session({ name: "c", title: "Fix streaming scroll jump", updatedAt: now - 3 * hour }),
  session({
    name: "d",
    title: "Parse invoice PDFs",
    cwd: "/Users/you/code/pdf-review",
    projectRoot: "/Users/you/code/pdf-review",
    checkout: "/Users/you/code/pdf-review",
    projectId: null,
    updatedAt: now - 26 * hour,
  }),
  session({
    name: "e",
    title: "Try a new tokenizer",
    cwd: "/Users/you/scratch",
    projectRoot: "/Users/you/scratch",
    checkout: null,
    projectId: null,
    updatedAt: now - 9 * 24 * hour,
  }),
];

let projects: Project[] = [
  {
    id: 1,
    repo: "/Users/you/code/panoptes",
    name: "panoptes",
    baseBranch: "main",
    copyFiles: [".env"],
    setup: "pnpm install",
  },
  {
    id: 2,
    repo: "/Users/you/code/design-system",
    name: "design-system",
    baseBranch: "main",
    copyFiles: [".env"],
    setup: null,
  },
];
let nextProjectId = 3;
let lanes: Lane[] = [
  {
    id: 1,
    projectId: 1,
    name: "reviewable-work",
    task: "Make lane work reviewable",
    branch: "panoptes/reviewable-work",
    state: "active",
    worktrees: [
      {
        projectId: 1,
        path: "/Users/you/Panoptes/lanes/reviewable-work/panoptes",
        mainCheckout: false,
        checkpointRef: null,
      },
    ],
    writeLock: { holder: null, queue: [] },
  },
];
let nextLaneId = 2;
let nextSessionId = 2;

/** Records a Panoptes session in a lane, as the backend does before it starts. */
const recordSession = (laneId: number): Session => {
  const lane = lanes.find((item) => item.id === laneId);
  if (lane?.state !== "active") {
    throw { kind: "other", message: "That lane is archived; restore it to start a session" };
  }
  const id = nextSessionId++;
  const cwd = lane.worktrees[0].path;
  const recorded: Session = {
    ...session({ name: `new-${id}`, title: "New session", cwd }),
    key: `panoptes:${id}`,
    id,
    laneId,
    harnessRef: null,
    cwd,
    projectRoot: cwd,
    checkout: projects.find((p) => p.id === lane.projectId)?.repo ?? null,
    projectId: lane.projectId,
    createdAt: Date.now(),
    updatedAt: Date.now(),
    messageCount: 0,
  };
  sessions.push(recorded);
  return recorded;
};

type ToolCallBlock = Extract<Message["blocks"][number], { type: "toolCall" }>;

const call = (
  id: string,
  name: string,
  kind: ToolCallBlock["kind"],
  input: Record<string, unknown>,
  subject: string | null,
  output: ToolOutput | null = null,
  tookMs = 1_000,
): ToolCallBlock => ({
  type: "toolCall",
  id,
  name,
  kind,
  subject,
  input,
  status: "done",
  output,
  startedAt: now - tookMs,
  endedAt: now,
});

const text = (text: string): ToolOutput => ({ type: "text", text });
const shell = (text: string, exitCode: number | null = 0): ToolOutput => ({
  type: "shell",
  text,
  exitCode,
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

/** A small diagram, as a pasted screenshot would be. */
const SKETCH = btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><rect width="160" height="90" fill="#efe7da"/><rect x="12" y="14" width="40" height="62" rx="4" fill="#d8ccb8"/><rect x="62" y="14" width="86" height="12" rx="3" fill="#c9bba4"/><rect x="62" y="34" width="70" height="8" rx="3" fill="#d8ccb8"/><rect x="62" y="50" width="78" height="8" rx="3" fill="#d8ccb8"/></svg>',
);

const history: Message[] = [
  msg(
    "m1",
    "user",
    [
      { type: "text", text: "What files are in src?" },
      { type: "image", mimeType: "image/svg+xml", data: SKETCH },
    ],
    20 * 60_000,
  ),
  msg(
    "m2",
    "assistant",
    [
      { type: "thinking", text: "I should list the directory." },
      call(
        "t1",
        "bash",
        "execute",
        { command: "ls src" },
        "ls src",
        shell("App.tsx\napi.ts\nstore.ts"),
      ),
    ],
    20 * 60_000 - 2_000,
  ),
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
        text(RETRY_OLD),
      ),
      call(
        "t3",
        "read",
        "read",
        { path: "src/conversation.ts" },
        "src/conversation.ts",
        text("export interface Conversation {\n  messages: Message[];\n}"),
      ),
      call(
        "t4",
        "grep",
        "search",
        { pattern: "message.error" },
        "message.error",
        text("src/components/ChatPane.tsx:202:      {message.error && ("),
      ),
    ],
    10 * 60_000 - 4_000,
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
        {
          type: "edits",
          edits: [{ path: "src/components/ChatPane.tsx", oldText: RETRY_OLD, newText: RETRY_NEW }],
          text: "Edited src/components/ChatPane.tsx",
        },
      ),
      call(
        "t6",
        "bash",
        "execute",
        { command: "pnpm test" },
        "pnpm test",
        shell(
          " RUN  v3.2.4\n ✓ src/composer.test.ts (18 tests) 41ms\n ✓ src/store.test.ts (12 tests) 33ms\n ✓ src/conversation.test.ts (9 tests) 12ms\n\n Test Files  3 passed (3)\n      Tests  39 passed (39)",
        ),
        8_000,
      ),
    ],
    10 * 60_000 - 30_000,
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

/** Prompts sent while a reply streams; the run settling drops them. */
const queue: QueuedPrompt[] = [];

/** Resolves the approval the mock is waiting on, if any. */
let decide: ((allow: boolean) => void) | null = null;

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
  const pending: ToolCall = {
    id: "live-t",
    name: "bash",
    kind: "execute",
    subject: "pnpm test",
    input: { command: "pnpm test" },
    status: "pending",
    output: null,
    startedAt: null,
    endedAt: null,
  };
  const test: ToolCallBlock = { type: "toolCall", ...pending };
  await send({ type: "blockStarted", messageId: "work", index: 1, block: test });
  const work = msg(`w${Date.now()}`, "assistant", [{ type: "thinking", text: thought }, test], 0);
  await send({ type: "messageCompleted", message: { ...work, id: "work", timestamp: Date.now() } });
  const running: ToolCall = { ...pending, status: "running", startedAt: Date.now() };
  await send({ type: "toolCallUpdated", call: running });
  if (text === "approve") {
    const decision = new Promise<boolean>((resolve) => {
      decide = resolve;
    });
    await send({
      type: "approval",
      approval: {
        id: `a${Date.now()}`,
        toolCallId: pending.id,
        title: "Run pnpm test?",
        message: "The permission extension asks before running commands.",
        expiresAt: Date.now() + 60_000,
      },
    });
    await send({ type: "statusChanged", status: "waiting" });
    const allowed = await decision;
    await send({ type: "statusChanged", status: "running" });
    if (!allowed) {
      const blocked: ToolCall = {
        ...running,
        status: "failed",
        output: shell("Blocked by user", null),
        endedAt: Date.now(),
      };
      await send({ type: "toolCallUpdated", call: blocked });
      history.push(user, { ...work, blocks: [work.blocks[0], { type: "toolCall", ...blocked }] });
      await send({ type: "runSettled", aborted: false });
      await send({ type: "statusChanged", status: "done" });
      return;
    }
  }
  let output = " RUN  v3.2.4";
  for (const line of [
    " ✓ src/composer.test.ts (18 tests) 41ms",
    " ✓ src/store.test.ts (12 tests) 33ms",
    " ✓ src/conversation.test.ts (9 tests) 12ms",
  ]) {
    await wait(text === "slow" ? 4000 : 600);
    output += `\n${line}`;
    await send({ type: "toolCallUpdated", call: { ...running, output: shell(output, null) } });
  }
  const tested: ToolCall = {
    ...running,
    status: "done",
    output: shell(output),
    endedAt: Date.now(),
  };
  await send({ type: "toolCallUpdated", call: tested });
  await send({ type: "messageStarted", messageId: "live", role: "assistant" });
  await send({
    type: "blockStarted",
    messageId: "live",
    index: 0,
    block: { type: "text", text: "" },
  });
  const reply =
    "This is a **mocked** reply, streamed word by word. Send `ask` to see a question, `approve` for a tool call that needs approval, or `slow` for a long-running tool.";
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
    {
      ...work,
      blocks: [work.blocks[0], { type: "toolCall", ...tested }],
      timestamp: Date.now(),
    },
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
        case "set_zoom":
          return null;
        case "list_sessions": {
          const all = sessions
            .map((s) => {
              const project = projects.find((p) => p.repo === s.checkout);
              const projectId = s.laneId === null ? (project?.id ?? null) : s.projectId;
              return { ...s, projectId, open: open.has(s.key) };
            })
            .sort((a, b) => b.updatedAt - a.updatedAt);
          return {
            sessions: all,
            projects: [...projects].sort((x, y) => x.name.localeCompare(y.name)),
            lanes,
            navigation: lanes.map((lane) => {
              const owned = all
                .filter((s) => s.laneId === lane.id)
                .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt);
              return {
                laneId: lane.id,
                sessions: owned.filter((s) => !s.archived),
                archivedSessions: owned.filter((s) => s.archived),
              };
            }),
            errors: [],
          };
        }
        case "add_project": {
          if (projects.some((p) => p.repo === a.folder)) {
            throw { kind: "other", message: `${a.folder} is already registered` };
          }
          const name = a.folder.split("/").filter(Boolean).pop() ?? a.folder;
          const project = {
            id: nextProjectId++,
            repo: a.folder,
            name,
            baseBranch: "main",
            copyFiles: [".env"],
            setup: null,
          };
          projects.push(project);
          return project;
        }
        case "remove_project": {
          const id = (args as { id: number }).id;
          projects = projects.filter((p) => p.id !== id);
          return null;
        }
        case "create_lane": {
          const request = args as unknown as {
            projectId: number;
            task: string;
            mainCheckout: boolean;
          };
          const name = request.task.toLowerCase().replace(/[^a-z0-9]+/g, "-");
          const project = projects.find((p) => p.id === request.projectId);
          const lane: Lane = {
            id: nextLaneId++,
            projectId: request.projectId,
            name,
            task: request.task,
            branch: request.mainCheckout ? project?.baseBranch || "main" : `panoptes/${name}`,
            state: "active",
            worktrees: [
              {
                projectId: request.projectId,
                path: request.mainCheckout
                  ? project?.repo || ""
                  : `/Users/you/Panoptes/lanes/${name}/${project?.name || "repo"}`,
                mainCheckout: request.mainCheckout,
                checkpointRef: null,
              },
            ],
            writeLock: { holder: null, queue: [] },
          };
          lanes.push(lane);
          return lane;
        }
        case "archive_lane": {
          const request = args as unknown as { id: number; strategy: string | null };
          const lane = lanes.find((item) => item.id === request.id);
          if (!lane) throw { kind: "other", message: "That lane no longer exists" };
          if (!request.strategy && !lane.worktrees[0].mainCheckout) {
            return { type: "dirty", lane };
          }
          lanes = lanes.map((item) =>
            item.id === request.id ? { ...item, state: "archived" as const } : item,
          );
          return { type: "archived" };
        }
        case "restore_lane": {
          const id = (args as { id: number }).id;
          const lane = lanes.find((item) => item.id === id);
          lanes = lanes.map((item) =>
            item.id === id ? { ...item, state: "active" as const } : item,
          );
          return lane;
        }
        case "open_session":
          if (!open.has(a.key)) {
            void emit("session-event", {
              key: a.key,
              event: { type: "statusChanged", status: "idle" },
            });
          }
          open.add(a.key);
          return { key: a.key, history };
        case "new_session": {
          const recorded = recordSession((args as { laneId: number }).laneId);
          open.add(recorded.key);
          return { key: recorded.key, history: [] };
        }
        case "adopt_session": {
          const request = args as unknown as { key: string; laneId: number | null };
          const existing = sessions.find((s) => s.key === request.key);
          if (!existing || existing.laneId !== null) {
            return { type: "adopted", key: existing?.key ?? request.key };
          }
          if (open.has(existing.key)) {
            throw { kind: "rejected", message: "Stop this session before adopting it into a lane" };
          }
          let laneId = request.laneId;
          if (laneId === null) {
            const matches = lanes.filter(
              (lane) =>
                lane.state === "active" &&
                lane.worktrees.some((tree) => existing.cwd.startsWith(tree.path)),
            );
            if (matches.length !== 1) return { type: "chooseLane" };
            laneId = matches[0].id;
          }
          const id = nextSessionId++;
          sessions = sessions.map((s) =>
            s.key === request.key
              ? { ...s, key: `panoptes:${id}`, id, laneId, archived: false }
              : s,
          );
          return { type: "adopted", key: `panoptes:${id}` };
        }
        case "replace_session": {
          const old = sessions.find((s) => s.key === a.key);
          if (old?.laneId == null) throw { kind: "rejected", message: "Not in a lane" };
          sessions = sessions.map((s) => (s.key === a.key ? { ...s, archived: true } : s));
          open.delete(a.key);
          const recorded = recordSession(old.laneId);
          open.add(recorded.key);
          return { key: recorded.key, history: [] };
        }
        case "archive_session": {
          const archived = (args as { archived: boolean }).archived;
          sessions = sessions.map((s) => (s.key === a.key ? { ...s, archived } : s));
          if (archived) open.delete(a.key);
          return null;
        }
        case "rename_session": {
          const title = (args as { title: string | null }).title;
          sessions = sessions.map((s) => (s.key === a.key ? { ...s, title: title ?? s.title } : s));
          return null;
        }
        case "session_history":
          return history;
        case "send_prompt":
          // While a reply streams, further prompts wait their turn; the
          // run settling clears them.
          if (streaming) {
            queue.push({
              id: `q${Date.now()}`,
              text: a.text,
              mode: a.mode === "steer" ? "steer" : "followUp",
            });
            void emit("session-event", {
              key: a.key,
              event: { type: "queueChanged", queued: [...queue] },
            });
            return "queued";
          }
          streaming = true;
          void stream(a.key, a.text).finally(() => {
            streaming = false;
            queue.length = 0;
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
        case "answer_question": {
          const answer = (args as { answer: Answer }).answer;
          if (answer.type === "decision" || answer.type === "cancelled") {
            decide?.(answer.type === "decision" && answer.allow);
            decide = null;
          }
          return null;
        }
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  if (new URLSearchParams(window.location.search).has("showcase")) void showcase(open);
}

/**
 * `?showcase`: every session status at once, plus a lock holder and a long
 * title, so the sidebar's session rows can be reviewed side by side.
 */
async function showcase(open: Set<string>) {
  sessions.push(
    session({
      name: "f",
      key: "panoptes:90",
      id: 90,
      laneId: 1,
      cwd: "/Users/you/Panoptes/lanes/reviewable-work/panoptes",
      title: "Wire the approval card into the composer and keep focus on the textarea",
      updatedAt: now - 2 * 60_000,
    }),
    session({
      name: "g",
      key: "panoptes:91",
      id: 91,
      laneId: 1,
      cwd: "/Users/you/Panoptes/lanes/reviewable-work/panoptes",
      title: "Run the migration tests",
      updatedAt: now - 40 * 60_000,
    }),
  );
  const statuses: [string, string][] = [
    ["panoptes:1", "running"],
    ["panoptes:90", "waiting"],
    ["panoptes:91", "failed"],
    ["pi:b", "done"],
    ["pi:c", "running"],
    ["pi:d", "waiting"],
  ];
  // Give the store a moment to subscribe before the events arrive.
  await new Promise((resolve) => setTimeout(resolve, 600));
  for (const [key, status] of statuses) {
    open.add(key);
    await emit("session-event", { key, event: { type: "statusChanged", status } });
  }
  await emit("session-event", {
    key: "panoptes:1",
    event: { type: "lockChanged", holder: true, queued: false },
  });
}
