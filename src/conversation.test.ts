import { describe, expect, it } from "vitest";
import { applyEvent, emptyConversation } from "./conversation";
import { groupSessions } from "./grouping";
import type { HarnessEvent, SidebarSession } from "./types";

const fold = (events: HarnessEvent[]) => events.reduce(applyEvent, emptyConversation());

describe("applyEvent", () => {
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
        },
      },
      { type: "runSettled", aborted: false },
    ] satisfies HarnessEvent[];
    const settled = done.reduce(applyEvent, streamed);
    expect(settled.messages).toHaveLength(1);
    expect(settled.messages[0].blocks[0]).toEqual({ type: "text", text: "Hello!" });
    expect(settled.busy).toBe(false);
  });

  it("tracks tool runs by call id", () => {
    const c = fold([
      { type: "toolStarted", callId: "c1", name: "bash", args: { command: "ls" } },
      { type: "toolFinished", callId: "c1", output: "a\nb", isError: false },
    ]);
    expect(c.tools.c1).toEqual({ status: "done", output: "a\nb" });
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
      ["Running", ["a"]],
      ["Pinned", ["b"]],
      ["panoptes", ["d", "c"]],
    ]);
  });
});
