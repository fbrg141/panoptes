import { afterEach, describe, expect, it, vi } from "vitest";
import { applyEvent, emptyConversation, removeQuestion } from "./conversation";
import { relativeTime } from "./grouping";
import type { Approval, HarnessEvent, ToolCall } from "./types";

const fold = (events: HarnessEvent[]) => events.reduce(applyEvent, emptyConversation());

describe("applyEvent", () => {
  it("leaves the transcript alone on an event it does not know", () => {
    const before = fold([
      { type: "runStarted" },
      { type: "messageStarted", messageId: "m1", role: "assistant" },
    ]);
    const after = applyEvent(before, { type: "fromANewerCore" } as unknown as HarnessEvent);
    expect(after).toBe(before);
  });

  it("ignores a repeated message start", () => {
    const c = fold([
      { type: "messageStarted", messageId: "m1", role: "assistant" },
      { type: "messageStarted", messageId: "m1", role: "assistant" },
    ]);
    expect(c.messages).toHaveLength(1);
  });

  it("drops tool arguments aimed at a block that is not a tool call", () => {
    const c = fold([
      { type: "messageStarted", messageId: "m1", role: "assistant" },
      { type: "blockStarted", messageId: "m1", index: 0, block: { type: "text", text: "Hi" } },
      { type: "blockDelta", messageId: "m1", index: 0, kind: "toolArgs", delta: '{"path":' },
      { type: "blockDelta", messageId: "m1", index: 3, kind: "toolArgs", delta: '{"path":' },
    ]);
    expect(c.messages[0].blocks).toEqual([{ type: "text", text: "Hi" }]);
  });

  it("fills the gap when a block arrives ahead of the ones before it", () => {
    const c = fold([
      { type: "messageStarted", messageId: "m1", role: "assistant" },
      { type: "blockDelta", messageId: "m1", index: 2, kind: "text", delta: "late" },
    ]);
    const blocks = c.messages[0].blocks;
    expect(blocks).toHaveLength(3);
    expect(blocks.every((b) => b !== undefined)).toBe(true);
    expect(blocks[2]).toEqual({ type: "text", text: "late" });
  });

  it("clears questions and approvals when the run settles, and keeps one card per id", () => {
    const question = {
      id: "q1",
      title: "Ok?",
      message: null,
      kind: { type: "confirm" as const },
      expiresAt: null,
    };
    let c = fold([
      { type: "runStarted" },
      { type: "question", question },
      { type: "question", question },
    ]);
    expect(c.questions).toHaveLength(1);
    c = applyEvent(c, { type: "runSettled", aborted: false });
    expect(c.questions).toEqual([]);
    expect(c.approvals).toEqual([]);
  });

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

describe("relativeTime", () => {
  it("speaks in the design system's voice", () => {
    const now = 10 * 86_400_000;
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now - 12 * 60_000, now)).toBe("12m ago");
    expect(relativeTime(now - 3 * 86_400_000, now)).toBe("3d ago");
  });
});
