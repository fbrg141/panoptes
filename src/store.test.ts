import { beforeEach, describe, expect, it, vi } from "vitest";
import type { api } from "./api";
import { createAppStore } from "./store";
import type { OpenedSession, SessionEvent, SidebarSession } from "./types";

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

function fakeBackend() {
  let emit: (event: SessionEvent) => void = () => {};
  const backend = {
    appInfo: vi.fn(async () => ({ harnesses: [], errors: [] })),
    listSessions: vi.fn(async () => ({ sessions: [session], errors: [] })),
    openSession: vi.fn(async () => opened),
    newSession: vi.fn(async () => opened),
    history: vi.fn(async () => []),
    sendPrompt: vi.fn(async () => "started" as const),
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
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "first", "start");
    expect(store.getState().conversations["pi:s1"].busy).toBe(true);

    await store.getState().send("second");
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "second", "steer");
  });

  it("reloads history when a run settles", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.emit({ key: "pi:s1", event: { type: "runSettled", aborted: false } });
    await vi.waitFor(() => expect(fake.backend.history).toHaveBeenCalledWith("pi:s1"));
    expect(store.getState().conversations["pi:s1"].busy).toBe(false);
  });

  it("resumes an exited session before sending", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "exited", code: 1, stderrTail: "" } });
    await store.getState().send("hello");
    expect(fake.backend.openSession).toHaveBeenCalledTimes(2);
    expect(fake.backend.sendPrompt).toHaveBeenCalledWith("pi:s1", "hello", "start");
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
    fake.backend.openSession.mockRejectedValueOnce("Pi is not installed");
    await store.getState().open(session);
    expect(store.getState().error).toBe("Pi is not installed");
  });
});
