import { beforeEach, describe, expect, it, vi } from "vitest";
import type { api } from "./api";
import { createAppStore, describeError } from "./store";
import type {
  AdoptionOutcome,
  ArchiveLaneOutcome,
  Lane,
  Listing,
  OpenedSession,
  Project,
  PromptOutcome,
  Session,
  SessionEvent,
  SessionSettings,
} from "./types";

const session: Session = {
  key: "pi:s1",
  id: null,
  laneId: null,
  harness: "pi",
  harnessRef: { harness: "pi", id: "s1", locator: "/s1.jsonl", cwd: "/code" },
  cwd: "/code",
  title: "One",
  tags: [],
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/code",
  checkout: null,
  projectId: null,
  pinned: false,
  archived: false,
  open: false,
};

const project: Project = {
  id: 1,
  repo: "/code",
  name: "code",
  baseBranch: "main",
  copyFiles: [".env"],
  setup: null,
};

const lane: Lane = {
  id: 1,
  projectId: 1,
  name: "fix-login",
  task: "Fix login",
  branch: "panoptes/fix-login",
  state: "active",
  worktrees: [
    {
      projectId: 1,
      path: "/lanes/fix-login/code",
      mainCheckout: false,
      checkpointRef: null,
    },
  ],
};

const opened: OpenedSession = { key: "pi:s1", history: [], status: "idle", replay: [] };

/** A session recorded in `lane`, as the backend lists it. */
const laneSession: Session = {
  ...session,
  key: "panoptes:7",
  id: 7,
  laneId: 1,
  harnessRef: null,
  cwd: "/lanes/fix-login/code",
  title: "New session",
  projectId: 1,
  open: true,
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
    listSessions: vi.fn(
      async (): Promise<Listing> => ({
        sessions: [session],
        projects: [],
        lanes: [],
        navigation: [],
        errors: [],
      }),
    ),
    openSession: vi.fn(async () => opened),
    newSession: vi.fn(async (): Promise<OpenedSession> => opened),
    adoptSession: vi.fn(
      async (): Promise<AdoptionOutcome> => ({ type: "adopted", key: "panoptes:7" }),
    ),
    archiveSession: vi.fn(async () => {}),
    renameSession: vi.fn(async () => {}),
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
    addProject: vi.fn(async (): Promise<Project> => project),
    updateProject: vi.fn(async (): Promise<Project> => project),
    removeProject: vi.fn(async () => {}),
    createLane: vi.fn(async (): Promise<Lane> => lane),
    cancelLaneCreation: vi.fn(async () => {}),
    archiveLane: vi.fn(async (): Promise<ArchiveLaneOutcome> => ({ type: "archived" })),
    restoreLane: vi.fn(async (): Promise<Lane> => lane),
    deleteLane: vi.fn(async () => {}),
    showLogs: vi.fn(async () => {}),
    onSessionEvent: vi.fn(async (handler: (event: SessionEvent) => void) => {
      emit = handler;
      return () => {};
    }),
  };
  return { backend, emit: (event: SessionEvent) => emit(event) };
}

describe("describeError", () => {
  it("shows a service error's message, never [object Object]", () => {
    expect(describeError({ kind: "exited", message: "Pi is not running" })).toBe(
      "Pi is not running",
    );
    expect(describeError(new Error("boom"))).toBe("boom");
    expect(describeError("plain")).toBe("plain");
  });
});

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

  it("catches a reopened session up on its run, status and waiting questions", async () => {
    const question = {
      id: "q1",
      title: "Ok?",
      message: null,
      kind: { type: "confirm" as const },
      expiresAt: null,
    };
    fake.backend.openSession.mockResolvedValueOnce({
      ...opened,
      status: "waiting",
      replay: [{ type: "runStarted" }, { type: "question", question }],
    });
    await store.getState().open(session);
    const conversation = store.getState().conversations["pi:s1"];
    expect(conversation.busy).toBe(true);
    expect(conversation.questions).toEqual([question]);
    expect(store.getState().statuses["pi:s1"]).toBe("waiting");
  });

  it("keeps a status that arrived while the session was opening", async () => {
    fake.backend.openSession.mockImplementationOnce(async () => {
      fake.emit({ key: "pi:s1", event: { type: "statusChanged", status: "done" } });
      return { ...opened, status: "running" };
    });
    await store.getState().open(session);
    expect(store.getState().statuses["pi:s1"]).toBe("done");
  });

  it("steers when the agent is already busy", async () => {
    await store.getState().open(session);
    await store.getState().send("first");
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "first", [], "start");
    expect(store.getState().conversations["pi:s1"].busy).toBe(true);

    await store.getState().send("second");
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "second", [], "steer");
  });

  it("sends a follow-up and shows the queue the harness reports", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.backend.sendPrompt.mockResolvedValueOnce("queued");
    await store.getState().send("then this", { queue: true });
    expect(fake.backend.sendPrompt).toHaveBeenLastCalledWith("pi:s1", "then this", [], "followUp");
    // Nothing is guessed from the reply; the harness's queue is the record.
    expect(store.getState().conversations["pi:s1"].queued).toEqual([]);

    const queued = [{ id: "q1", text: "then this", mode: "followUp" }] as const;
    fake.emit({ key: "pi:s1", event: { type: "queueChanged", queued: [...queued] } });
    expect(store.getState().conversations["pi:s1"].queued).toEqual(queued);
    fake.emit({ key: "pi:s1", event: { type: "queueChanged", queued: [] } });
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

  const streamed = {
    id: "m2",
    role: "assistant" as const,
    blocks: [],
    timestamp: 0,
    error: null,
    stopped: false,
  };

  it("drops a history reply that a newer run has overtaken", async () => {
    await store.getState().open(session);
    let reply: (messages: never[]) => void = () => {};
    fake.backend.history.mockImplementationOnce(
      () => new Promise<never[]>((resolve) => (reply = resolve)),
    );
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.emit({ key: "pi:s1", event: { type: "runSettled", aborted: false } });
    await vi.waitFor(() => expect(fake.backend.history).toHaveBeenCalled());
    // The next run starts streaming before the reload answers.
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.emit({
      key: "pi:s1",
      event: { type: "messageStarted", messageId: "m2", role: "assistant" },
    });
    reply([]);
    await Promise.resolve();
    await Promise.resolve();
    expect(store.getState().conversations["pi:s1"].messages.map((m) => m.id)).toEqual(["m2"]);
  });

  it("keeps the streaming transcript when a running session is opened again", async () => {
    await store.getState().open(session);
    fake.emit({ key: "pi:s1", event: { type: "runStarted" } });
    fake.emit({
      key: "pi:s1",
      event: { type: "messageStarted", messageId: "m2", role: "assistant" },
    });
    await store.getState().open(session);
    expect(store.getState().conversations["pi:s1"].messages).toMatchObject([{ id: streamed.id }]);
  });

  it("brings a question back when answering it fails", async () => {
    await store.getState().open(session);
    const question = {
      id: "q1",
      title: "Ok?",
      message: null,
      kind: { type: "confirm" as const },
      expiresAt: null,
    };
    fake.emit({ key: "pi:s1", event: { type: "question", question } });
    fake.backend.answer.mockRejectedValueOnce({ kind: "timedOut", message: "Pi did not answer" });
    await store.getState().answer("q1", { type: "confirmed", confirmed: true });
    expect(store.getState().conversations["pi:s1"].questions).toEqual([question]);
    expect(store.getState().error).toBe("Pi did not answer");

    // A process that has gone asks nothing any more.
    fake.backend.answer.mockRejectedValueOnce({ kind: "exited", message: "Pi is not running" });
    await store.getState().answer("q1", { type: "confirmed", confirmed: true });
    expect(store.getState().conversations["pi:s1"].questions).toEqual([]);
  });

  it("says whether a prompt was delivered", async () => {
    await store.getState().open(session);
    expect(await store.getState().send("hello")).toBe(true);
    fake.backend.sendPrompt.mockRejectedValueOnce({ kind: "rejected", message: "No model" });
    expect(await store.getState().send("hello")).toBe(false);
  });

  it("lists again as soon as a session is seen editing another lane", async () => {
    await store.getState().open(session);
    const before = fake.backend.listSessions.mock.calls.length;
    fake.emit({ key: "pi:s1", event: { type: "laneWritesChanged", laneId: 2 } });
    await vi.waitFor(() =>
      expect(fake.backend.listSessions.mock.calls.length).toBeGreaterThan(before),
    );
    // It is not a harness event, so the conversation is left as it was.
    expect(store.getState().conversations["pi:s1"].messages).toEqual([]);
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

  it("keeps each session's draft, dropping it once it is empty", () => {
    store.getState().saveDraft("pi:s1", {
      text: "typed halfway",
      attachments: [{ name: "a.txt", path: "/code/a.txt", mimeType: null, data: null }],
      mentioned: [],
    });
    expect(store.getState().drafts["pi:s1"]?.text).toBe("typed halfway");

    store.getState().saveDraft("pi:s2", { text: "", attachments: [], mentioned: [] });
    expect(store.getState().drafts["pi:s2"]).toBeUndefined();

    store.getState().saveDraft("pi:s1", { text: "", attachments: [], mentioned: [] });
    expect(store.getState().drafts).toEqual({});
  });

  it("forgets a draft with its session", async () => {
    await store.getState().open(session);
    store.getState().saveDraft("pi:s1", { text: "typed halfway", attachments: [], mentioned: [] });
    await store.getState().close("pi:s1");
    expect(store.getState().drafts).toEqual({});
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

  it("registers a project and lists it after", async () => {
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [{ ...session, projectId: 1 }],
      projects: [project],
      lanes: [],
      navigation: [],
      errors: [],
    });
    await store.getState().addProject("/code/src");
    expect(fake.backend.addProject).toHaveBeenCalledWith("/code/src");
    expect(store.getState().projects).toEqual([project]);
    expect(store.getState().sessions[0].projectId).toBe(1);

    expect(await store.getState().removeProject(1)).toBe(true);
    expect(fake.backend.removeProject).toHaveBeenCalledWith(1);
    expect(store.getState().projects).toEqual([]);
  });

  it("keeps the removal dialog open when a project cannot be removed", async () => {
    fake.backend.removeProject.mockRejectedValueOnce({
      kind: "other",
      message: "That project still has lanes and cannot be removed",
    });
    expect(await store.getState().removeProject(1)).toBe(false);
    expect(store.getState().error).toBe("That project still has lanes and cannot be removed");
  });

  it("shows why a folder could not be registered", async () => {
    fake.backend.addProject.mockRejectedValueOnce({
      kind: "other",
      message: "/tmp is not in a git repository",
    });
    await store.getState().addProject("/tmp");
    expect(store.getState().error).toBe("/tmp is not in a git repository");
  });

  it("creates a durable lane and lists it without a session", async () => {
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [session],
      projects: [project],
      lanes: [lane],
      navigation: [],
      errors: [],
    });
    const created = await store.getState().createLane(1, "Fix login", false);
    expect(fake.backend.createLane).toHaveBeenCalledWith(1, "Fix login", false);
    expect(created).toEqual(lane);
    expect(store.getState().lanes).toEqual([lane]);
  });

  it("creates a lane and its only session from the project prompt", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:7",
      history: [],
      status: "idle",
      replay: [],
    });
    const created = await store.getState().createFromPrompt(1, "Fix login", true, {
      model: "p/small",
      thinking: "high",
    });
    expect(created).toBe(true);
    expect(fake.backend.createLane).toHaveBeenCalledWith(1, "Fix login", true);
    expect(fake.backend.newSession).toHaveBeenCalledWith(lane.id, "pi");
    expect(fake.backend.setModel).toHaveBeenCalledWith("panoptes:7", "p/small");
    expect(fake.backend.setThinkingLevel).toHaveBeenCalledWith("panoptes:7", "high");
    expect(fake.backend.sendPrompt).toHaveBeenCalledWith("panoptes:7", "Fix login", [], "start");
  });

  it("treats lane creation cancellation as a choice, not an error", async () => {
    fake.backend.createLane.mockRejectedValueOnce({ kind: "cancelled", message: "Cancelled" });
    const created = await store.getState().createLane(1, "Slow setup", false);
    expect(created).toBeNull();
    expect(store.getState().error).toBeNull();
    await store.getState().cancelLaneCreation(1);
    expect(fake.backend.cancelLaneCreation).toHaveBeenCalledWith(1);
  });

  it("waits for a dirty lane's archive choice and refreshes once archived", async () => {
    fake.backend.archiveLane.mockResolvedValueOnce({ type: "dirty", lane });
    const dirty = await store.getState().archiveLane(1, null);
    expect(dirty).toEqual({ type: "dirty", lane });
    expect(fake.backend.archiveLane).toHaveBeenCalledWith(1, null);

    fake.backend.archiveLane.mockResolvedValueOnce({ type: "archived" });
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [session],
      projects: [project],
      lanes: [{ ...lane, state: "archived" }],
      navigation: [{ laneId: 1, sessions: [], archivedSessions: [], work: null, busyWriter: null }],
      errors: [],
    });
    const archived = await store.getState().archiveLane(1, "checkpoint");
    expect(archived).toEqual({ type: "archived" });
    expect(store.getState().lanes[0].state).toBe("archived");
  });

  it("deletes a lane, forgets its sessions and refreshes the projects", async () => {
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [laneSession],
      projects: [project],
      lanes: [lane],
      navigation: [
        { laneId: 1, sessions: [laneSession], archivedSessions: [], work: null, busyWriter: null },
      ],
      errors: [],
    });
    await store.getState().refresh();
    store.setState({ activeKey: laneSession.key });
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [],
      projects: [project],
      lanes: [],
      navigation: [],
      errors: [],
    });

    const deleted = await store.getState().deleteLane(lane.id);

    expect(deleted).toBe(true);
    expect(fake.backend.deleteLane).toHaveBeenCalledWith(lane.id);
    expect(store.getState().activeKey).toBeNull();
    expect(store.getState().sessions).toEqual([]);
    expect(store.getState().lanes).toEqual([]);
  });

  it("keeps the lane visible and shows the error when deletion fails", async () => {
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [],
      projects: [project],
      lanes: [lane],
      navigation: [{ laneId: 1, sessions: [], archivedSessions: [], work: null, busyWriter: null }],
      errors: [],
    });
    await store.getState().refresh();
    fake.backend.deleteLane.mockRejectedValueOnce({
      kind: "rejected",
      message: "Close this lane's session before deleting it",
    });

    const deleted = await store.getState().deleteLane(lane.id);

    expect(deleted).toBe(false);
    expect(store.getState().lanes).toEqual([lane]);
    expect(store.getState().error).toBe("Close this lane's session before deleting it");
  });

  it("shows the error when the logs folder cannot be opened", async () => {
    fake.backend.showLogs.mockRejectedValueOnce({ kind: "io", message: "No Finder" });

    await store.getState().showLogs();

    expect(fake.backend.showLogs).toHaveBeenCalled();
    expect(store.getState().error).toBe("No Finder");
  });

  it("starts a session in a lane and lists it at once, from its record", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:7",
      history: [],
      status: "idle",
      replay: [],
    });
    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [laneSession, session],
      projects: [project],
      lanes: [lane],
      navigation: [
        { laneId: 1, sessions: [laneSession], archivedSessions: [], work: null, busyWriter: null },
      ],
      errors: [],
    });
    await store.getState().create(lane.id);
    expect(fake.backend.newSession).toHaveBeenCalledWith(1, "pi");
    expect(store.getState().activeKey).toBe("panoptes:7");
    expect(store.getState().sessions[0]).toEqual(laneSession);
    expect(store.getState().navigation[0].sessions).toEqual([laneSession]);
  });

  it("starts a session with the chosen model, thinking level and first prompt", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:9",
      history: [],
      status: "idle",
      replay: [],
    });
    await store.getState().create(lane.id, {
      model: "p/small",
      thinking: "high",
      prompt: "Fix the login redirect",
    });
    expect(fake.backend.newSession).toHaveBeenCalledWith(1, "pi");
    expect(fake.backend.setModel).toHaveBeenCalledWith("panoptes:9", "p/small");
    expect(fake.backend.setThinkingLevel).toHaveBeenCalledWith("panoptes:9", "high");
    expect(fake.backend.sendPrompt).toHaveBeenCalledWith(
      "panoptes:9",
      "Fix the login redirect",
      [],
      "start",
    );
    expect(store.getState().activeKey).toBe("panoptes:9");
    expect(store.getState().conversations["panoptes:9"].busy).toBe(true);
  });

  it("starts a bare session when no launch options are given", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:9",
      history: [],
      status: "idle",
      replay: [],
    });
    await store.getState().create(lane.id);
    expect(fake.backend.setModel).not.toHaveBeenCalled();
    expect(fake.backend.setThinkingLevel).not.toHaveBeenCalled();
    expect(fake.backend.sendPrompt).not.toHaveBeenCalled();
  });

  it("starts the session even when a launch choice is refused", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:9",
      history: [],
      status: "idle",
      replay: [],
    });
    fake.backend.setModel.mockRejectedValueOnce({ kind: "rejected", message: "No such model" });
    await store.getState().create(lane.id, { model: "p/missing", prompt: "Go" });
    expect(fake.backend.sendPrompt).toHaveBeenCalledWith("panoptes:9", "Go", [], "start");
    expect(store.getState().error).toBe("No such model");
  });

  it("reloads history when the launch prompt was handled synchronously", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:9",
      history: [],
      status: "idle",
      replay: [],
    });
    fake.backend.sendPrompt.mockResolvedValueOnce("handled");
    await store.getState().create(lane.id, { prompt: "Go" });
    expect(fake.backend.history).toHaveBeenCalledWith("panoptes:9");
  });

  it("adopts a raw session after the backend asks for a lane", async () => {
    fake.backend.adoptSession.mockResolvedValueOnce({ type: "chooseLane" });
    const choice = await store.getState().adopt(session);
    expect(choice).toEqual({ type: "chooseLane" });
    expect(fake.backend.adoptSession).toHaveBeenCalledWith(session, null);

    fake.backend.listSessions.mockResolvedValueOnce({
      sessions: [laneSession],
      projects: [project],
      lanes: [lane],
      navigation: [
        { laneId: 1, sessions: [laneSession], archivedSessions: [], work: null, busyWriter: null },
      ],
      errors: [],
    });
    const adopted = await store.getState().adopt(session, 1);
    expect(adopted).toEqual({ type: "adopted", key: "panoptes:7" });
    expect(fake.backend.adoptSession).toHaveBeenLastCalledWith(session, 1);
    expect(store.getState().navigation[0].sessions).toEqual([laneSession]);
  });

  it("follows an adopted conversation to its Panoptes key", async () => {
    fake.backend.openSession.mockResolvedValueOnce({
      key: "panoptes:7",
      history: [],
      status: "idle",
      replay: [],
    });
    await store.getState().open(session);
    expect(store.getState().activeKey).toBe("panoptes:7");
  });

  it("renames a lane session, clearing the title when it is emptied", async () => {
    await store.getState().rename("panoptes:7", "  Fix redirects ");
    expect(fake.backend.renameSession).toHaveBeenCalledWith("panoptes:7", "Fix redirects");
    await store.getState().rename("panoptes:7", "   ");
    expect(fake.backend.renameSession).toHaveBeenLastCalledWith("panoptes:7", null);
  });

  it("archives a session, closing it here", async () => {
    fake.backend.newSession.mockResolvedValueOnce({
      key: "panoptes:7",
      history: [],
      status: "idle",
      replay: [],
    });
    await store.getState().create(lane.id);
    await store.getState().setArchived("panoptes:7", true);
    expect(fake.backend.archiveSession).toHaveBeenCalledWith("panoptes:7", true);
    expect(store.getState().activeKey).toBeNull();
  });
});
