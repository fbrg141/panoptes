import { beforeEach, describe, expect, it, vi } from "vitest";
import type { api } from "./api";
import { createAppStore } from "./store";
import type {
  OpenedSession,
  PromptOutcome,
  SessionEvent,
  SessionSettings,
  SidebarSession,
} from "./types";

const session: SidebarSession = {
  key: "pi:s1",
  harness: "pi",
  id: "s1",
  locator: "/s1.jsonl",
  cwd: "/code",
  title: "One",
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/code",
  pinned: false,
  open: false,
};

const opened: OpenedSession = {
  key: "pi:s1",
  session: { harness: "pi", id: "s1", locator: "/s1.jsonl", cwd: "/code" },
  history: [],
};

const settings: SessionSettings = {
  model: { id: "p/small", name: "Small", provider: "p", contextWindow: 32_000, images: false },
  thinkingLevel: "low",
  thinkingLevels: ["off", "low", "high"],
  context: { tokens: 1200, window: 32_000 },
};

function fakeBackend() {
  let emit: (event: SessionEvent) => void = () => {};
  const backend = {
    appInfo: vi.fn(async () => ({ harnesses: [], errors: [] })),
    listSessions: vi.fn(async () => ({ sessions: [session], errors: [] })),
    openSession: vi.fn(async () => opened),
    newSession: vi.fn(async () => opened),
    history: vi.fn(async () => []),
    sendPrompt: vi.fn(async (): Promise<PromptOutcome> => "started"),
    settings: vi.fn(async () => settings),
    listModels: vi.fn(async () => (settings.model ? [settings.model] : [])),
    setModel: vi.fn(async () => settings),
    setThinkingLevel: vi.fn(async (_key: string, level: string) => ({
      ...settings,
      thinkingLevel: level,
    })),
    listFiles: vi.fn(async () => []),
    readAttachment: vi.fn(),
    abort: vi.fn(async () => {}),
    answer: vi.fn(async () => {}),
    closeSession: vi.fn(async () => {}),
    setPinned: vi.fn(async () => {}),
    onSessionEvent: vi.fn(async (handler: (event: SessionEvent) => void) => {
      emit = handler;
      return () => {};
    }),
  };
  return { backend, emit: (event: SessionEvent) => emit(event) };
}

describe("store", () => {
  let fake: ReturnType<typeof fakeBackend>;
  let store: ReturnType<typeof createAppStore>;

  beforeEach(async () => {
    fake = fakeBackend();
    store = createAppStore(fake.backend as unknown as typeof api);
    await store.getState().init();
    await store.getState().init();
  });

  it("listens once even when initialised twice", () => {
    expect(fake.backend.onSessionEvent).toHaveBeenCalledTimes(1);
    expect(store.getState().sessions).toEqual([session]);
  });

  it("steers when the agent is already busy", async () => {
    await store.getState().open(session);
    await store.getState().send("first");
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "first", [], "start");
    expect(store.getState().conversations["pi:s1"].busy).toBe(true);

    await store.getState().send("second");
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "second", [], "steer");
  });

  it("queues a follow-up and drops it once the agent reads it", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.backend.sendPrompt.mockResolvedValueOnce("queued");
    await store.getState().send("then this", { queue: true });
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "then this", [], "followUp");
    expect(store.getState().conversations["pi:s1"].queued).toEqual([
      { text: "then this", mode: "followUp" },
    ]);

    fake.emit({
      key: "pi:s1",
      event: {
        type: "messageCompleted",
        message: {
          id: "u2",
          role: "user",
          blocks: [{ type: "text", text: "then this" }],
          timestamp: null,
          error: null,
          stopped: false,
        },
      },
    });
    expect(store.getState().conversations["pi:s1"].queued).toEqual([]);
  });

  it("loads settings on open and after each run", async () => {
    await store.getState().open(session);
    await vi.waitFor(() => expect(store.getState().settings["pi:s1"]).toEqual(settings));
    fake.emit({ key: "pi:s1", event: { type: "runSettled", aborted: false } });
    await vi.waitFor(() => expect(fake.backend.settings).toHaveBeenCalledTimes(2));
  });

  it("changes the thinking level and keeps the reported settings", async () => {
    await store.getState().open(session);
    await store.getState().setThinkingLevel("high");
    expect(fake.backend.setThinkingLevel).toHaveBeenCalledWith("pi:s1", "high");
    expect(store.getState().settings["pi:s1"].thinkingLevel).toBe("high");

    await store.getState().loadModels();
    expect(store.getState().models["pi:s1"]).toHaveLength(1);
  });

  it("reloads history when a run settles", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.emit({ key: "pi:s1", event: { type: "runSettled", aborted: false } });
    await vi.waitFor(() => expect(fake.backend.history).toHaveBeenCalledWith("pi:s1"));
    expect(store.getState().conversations["pi:s1"].busy).toBe(false);
  });

  it("resumes a session whose process is gone and sends again", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "exited", code: 1, stderrTail: "" } });
    fake.backend.sendPrompt.mockRejectedValueOnce({ kind: "notOpen", message: "Not open" });
    await store.getState().send("hello");
    expect(fake.backend.openSession).toHaveBeenCalledTimes(2);
    expect(fake.backend.sendPrompt).toHaveBeenCalledTimes(2);
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "hello", [], "start");
    expect(store.getState().error).toBeNull();
  });

  it("shows a rejected prompt without reopening", async () => {
    await store.getState().open(session);
    fake.backend.sendPrompt.mockRejectedValueOnce({ kind: "rejected", message: "No model" });
    await store.getState().send("hello");
    expect(fake.backend.openSession).toHaveBeenCalledTimes(1);
    expect(store.getState().error).toBe("No model");
  });

  it("keeps the status the backend reports, until the session closes", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "statusChanged", status: "waiting" } });
    expect(store.getState().statuses["pi:s1"]).toBe("waiting");
    expect(store.getState().conversations["pi:s1"].messages).toEqual([]);
    await store.getState().close("pi:s1");
    expect(store.getState().statuses["pi:s1"]).toBeUndefined();
  });

  it("removes a question once answered", async () => {
    await store.getState().open(session);
    fake.emit({
      key: "pi:s1",
      event: {
        type: "question",
        question: {
          id: "q1",
          title: "Ok?",
          message: null,
          kind: { type: "confirm" },
          expiresAt: null,
        },
      },
    });
    expect(store.getState().conversations["pi:s1"].questions).toHaveLength(1);
    await store.getState().answer("q1", { type: "confirmed", confirmed: true });
    expect(fake.backend.answer).toHaveBeenCalledWith("pi:s1", "q1", {
      type: "confirmed",
      confirmed: true,
    });
    expect(store.getState().conversations["pi:s1"].questions).toHaveLength(0);
  });

  it("forgets a closed session's conversation", async () => {
    await store.getState().open(session);
    await store.getState().close("pi:s1");
    expect(store.getState().conversations["pi:s1"]).toBeUndefined();
    expect(store.getState().activeKey).toBeNull();
  });

  it("shows backend errors", async () => {
    fake.backend.openSession.mockRejectedValueOnce({
      kind: "unavailable",
      message: "The pi harness is not available",
    });
    await store.getState().open(session);
    expect(store.getState().error).toBe("The pi harness is not available");

    fake.backend.openSession.mockRejectedValueOnce("Pi is not installed");
    await store.getState().open(session);
    expect(store.getState().error).toBe("Pi is not installed");
  });
});
