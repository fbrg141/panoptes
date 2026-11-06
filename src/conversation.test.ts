import { afterEach, describe, expect, it, vi } from "vitest";
import { applyEvent, emptyConversation, enqueue } from "./conversation";
import { groupSessions, relativeTime } from "./grouping";
import type { HarnessEvent, SidebarSession } from "./types";

const fold = (events: HarnessEvent[]) => events.reduce(applyEvent, emptyConversation());

describe("applyEvent", () => {
  it("clears queued prompts once the run settles", () => {
    let c = enqueue(emptyConversation(), { text: "a", mode: "steer" });
    c = enqueue(c, { text: "b", mode: "followUp" });
    expect(c.queued).toHaveLength(2);
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

  it("tracks tool runs by call id, with how long they took", () => {
    vi.useFakeTimers({ now: 1000 });
    let c = fold([{ type: "toolStarted", callId: "c1", name: "bash", args: { command: "ls" } }]);
    vi.setSystemTime(1500);
    c = applyEvent(c, { type: "toolUpdated", callId: "c1", output: "a" });
    vi.setSystemTime(4000);
    c = applyEvent(c, { type: "toolFinished", callId: "c1", output: "a\nb", isError: false });
    expect(c.tools.c1).toEqual({ status: "done", output: "a\nb", startedAt: 1000, endedAt: 4000 });
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
