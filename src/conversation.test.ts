import { afterEach, describe, expect, it, vi } from "vitest";
import { applyEvent, emptyConversation, removeQuestion } from "./conversation";
import { groupSessions, relativeTime } from "./grouping";
import type { Approval, HarnessEvent, SidebarSession, ToolCall } from "./types";

const fold = (events: HarnessEvent[]) => events.reduce(applyEvent, emptyConversation());

describe("applyEvent", () => {
  it("shows the harness's queue, and clears it once the run settles", () => {
    const queued = [
      { id: "q1", text: "a", mode: "steer" },
      { id: "q2", text: "a", mode: "followUp" },
    ] as const;
    let c = fold([{ type: "runStarted" }, { type: "queueChanged", queued: [...queued] }]);
    expect(c.queued).toEqual(queued);
    // Delivering a prompt does not touch the queue; the harness says when it shrinks.
    c = applyEvent(c, {
      type: "messageCompleted",
      message: {
        id: "u1",
        role: "user",
        blocks: [{ type: "text", text: "a" }],
        timestamp: null,
        error: null,
        stopped: false,
      },
    });
    expect(c.queued).toHaveLength(2);
    c = applyEvent(c, { type: "queueChanged", queued: [queued[1]] });
    expect(c.queued.map((q) => q.id)).toEqual(["q2"]);
    expect(applyEvent(c, { type: "runSettled", aborted: true }).queued).toEqual([]);
  });

  it("streams text and replaces it with the completed message", () => {
    const streamed = fold([
      { type: "runStarted" },
      { type: "messageStarted", messageId: "live-1", role: "assistant" },
      { type: "blockStarted", messageId: "live-1", index: 0, block: { type: "text", text: "" } },
      { type: "blockDelta", messageId: "live-1", index: 0, kind: "text", delta: "Hel" },
      { type: "blockDelta", messageId: "live-1", index: 0, kind: "text", delta: "lo" },
    ]);
    expect(streamed.busy).toBe(true);
    expect(streamed.messages[0].blocks[0]).toEqual({ type: "text", text: "Hello" });

    const done = [
      {
        type: "messageCompleted",
        message: {
          id: "live-1",
          role: "assistant",
          blocks: [{ type: "text", text: "Hello!" }],
          timestamp: 1,
          error: null,
          stopped: false,
        },
      },
      { type: "runSettled", aborted: false },
    ] satisfies HarnessEvent[];
    const settled = done.reduce(applyEvent, streamed);
    expect(settled.messages).toHaveLength(1);
    expect(settled.messages[0].blocks[0]).toEqual({ type: "text", text: "Hello!" });
    expect(settled.busy).toBe(false);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("holds approvals until they are decided or the process ends", () => {
    const approval: Approval = {
      id: "a1",
      toolCallId: "c1",
      title: "Allow?",
      message: null,
      expiresAt: null,
    };
    const c = fold([{ type: "approval", approval }]);
    expect(c.approvals).toEqual([approval]);
    expect(removeQuestion(c, "a1").approvals).toEqual([]);
    expect(applyEvent(c, { type: "exited", code: 1, stderrTail: "" }).approvals).toEqual([]);
  });

  it("updates a tool call in place, keeping its streamed arguments", () => {
    const pending: ToolCall = {
      id: "c1",
      name: "bash",
      kind: "execute",
      subject: null,
      input: null,
      status: "pending",
      output: null,
      startedAt: null,
      endedAt: null,
    };
    let c = fold([
      { type: "messageStarted", messageId: "live-1", role: "assistant" },
      {
        type: "blockStarted",
        messageId: "live-1",
        index: 0,
        block: { type: "toolCall", ...pending },
      },
      { type: "blockDelta", messageId: "live-1", index: 0, kind: "toolArgs", delta: '{"command"' },
    ]);
    const done: ToolCall = {
      ...pending,
      subject: "ls",
      input: { command: "ls" },
      status: "done",
      output: { type: "shell", text: "a", exitCode: 0 },
      startedAt: 1000,
      endedAt: 2000,
    };
    c = applyEvent(c, { type: "toolCallUpdated", call: done });
    expect(c.messages[0].blocks[0]).toEqual({ type: "toolCall", argsText: '{"command"', ...done });
    // An update for a call no message holds changes nothing.
    expect(applyEvent(c, { type: "toolCallUpdated", call: { ...done, id: "other" } })).toBe(c);
  });

  it("times a thinking block until the next block starts", () => {
    vi.useFakeTimers({ now: 0 });
    let c = fold([
      { type: "messageStarted", messageId: "live-1", role: "assistant" },
      {
        type: "blockStarted",
        messageId: "live-1",
        index: 0,
        block: { type: "thinking", text: "" },
      },
      { type: "blockDelta", messageId: "live-1", index: 0, kind: "thinking", delta: "Hmm." },
    ]);
    expect(c.thinking).toEqual({ messageId: "live-1", index: 0, at: 0 });
    vi.setSystemTime(8000);
    c = applyEvent(c, {
      type: "blockStarted",
      messageId: "live-1",
      index: 1,
      block: { type: "text", text: "" },
    });
    expect(c.thinking).toBeNull();
    expect(c.thoughtFor).toEqual({ "Hmm.": 8000 });
  });
});

const session = (over: Partial<SidebarSession>): SidebarSession => ({
  key: "pi:x",
  harness: "pi",
  id: "x",
  locator: "",
  cwd: "/r",
  title: "t",
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/r",
  pinned: false,
  open: false,
  ...over,
});

describe("groupSessions", () => {
  it("puts each session in its first matching group", () => {
    const groups = groupSessions([
      session({ key: "a", open: true, pinned: true, updatedAt: 1 }),
      session({ key: "b", pinned: true, updatedAt: 2 }),
      session({ key: "c", projectRoot: "/code/panoptes", updatedAt: 3 }),
      session({ key: "d", projectRoot: "/code/panoptes", updatedAt: 4 }),
    ]);
    expect(groups.map((g) => [g.label, g.sessions.map((s) => s.key)])).toEqual([
      ["Open", ["a"]],
      ["Pinned", ["b"]],
      ["panoptes", ["d", "c"]],
    ]);
  });
});

describe("relativeTime", () => {
  it("speaks in the design system's voice", () => {
    const now = 10 * 86_400_000;
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now - 12 * 60_000, now)).toBe("12m ago");
    expect(relativeTime(now - 3 * 86_400_000, now)).toBe("3d ago");
  });
});
