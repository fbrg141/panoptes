import { describe, expect, it } from "vitest";
import { emptyConversation } from "./conversation";
import { buildTurns, formatDuration, groupLabel, summarizeWork, toolSteps } from "./turns";
import type { Block, Message, ToolKind } from "./types";

const message = (
  id: string,
  role: Message["role"],
  blocks: Block[],
  over: Partial<Message> = {},
) => ({
  id,
  role,
  blocks,
  timestamp: null,
  error: null,
  stopped: false,
  ...over,
});
const text = (t: string): Block => ({ type: "text", text: t });
const call = (callId: string, kind: ToolKind, subject: string): Block => ({
  type: "toolCall",
  callId,
  name: kind,
  args: {},
  kind,
  subject,
  edits: [],
});
const result = (callId: string, output: string, isError = false): Block => ({
  type: "toolResult",
  callId,
  name: "x",
  output,
  isError,
});

const turnsOf = (messages: Message[], busy = false) =>
  buildTurns({ ...emptyConversation(messages), busy });

describe("buildTurns", () => {
  it("starts a turn at each prompt and joins tool results to their calls", () => {
    const turns = turnsOf([
      message("u1", "user", [text("list files")]),
      message("a1", "assistant", [call("c1", "execute", "ls")]),
      message("t1", "tool", [result("c1", "a\nb")]),
      message("a2", "assistant", [text("Two files.")]),
      message("u2", "user", [text("thanks")]),
    ]);
    expect(turns.map((t) => t.key)).toEqual(["u1", "u2"]);
    const [first] = turns;
    expect(first.work).toMatchObject([{ type: "tool", status: "done", output: "a\nb" }]);
    expect(first.answer).toMatchObject([{ type: "text", text: "Two files." }]);
  });

  it("keeps output before the first prompt in a turn of its own", () => {
    const turns = turnsOf([message("a0", "assistant", [text("resumed")])]);
    expect(turns).toMatchObject([{ key: "a0", prompt: null }]);
  });

  it("groups runs of successful reads and searches", () => {
    const [turn] = turnsOf([
      message("u1", "user", [text("look")]),
      message("a1", "assistant", [
        call("c1", "read", "a.ts"),
        call("c2", "search", "TODO"),
        call("c3", "read", "b.ts"),
        call("c4", "execute", "pnpm test"),
        call("c5", "read", "c.ts"),
      ]),
      message("t1", "tool", [result("c1", ""), result("c2", ""), result("c3", "", true)]),
    ]);
    // The failed read breaks the run, so only the first two group.
    expect(turn.work.map((s) => s.type)).toEqual(["group", "tool", "tool", "tool"]);
    expect(toolSteps(turn.work)).toHaveLength(5);
    const group = turn.work[0];
    expect(group.type === "group" && groupLabel(group.steps)).toBe(
      "Read 1 file, searched 1 pattern",
    );
  });

  it("marks only the last turn live while the agent works", () => {
    const turns = turnsOf(
      [
        message("u1", "user", [text("a")], { timestamp: 1 }),
        message("a1", "assistant", [text("b")], { timestamp: 5 }),
        message("u2", "user", [text("c")], { timestamp: 9 }),
      ],
      true,
    );
    expect(turns.map((t) => t.live)).toEqual([false, true]);
    expect(turns[0]).toMatchObject({ startedAt: 1, endedAt: 5 });
    expect(turns[1].endedAt).toBeNull();
  });

  it("keeps errors in the answer and carries the stopped flag", () => {
    const [turn] = turnsOf([
      message("u1", "user", [text("go")]),
      message("a1", "assistant", [call("c1", "execute", "make")], {
        error: "Rate limited",
        stopped: true,
      }),
    ]);
    expect(turn.answer).toMatchObject([{ type: "error", text: "Rate limited" }]);
    expect(turn.stopped).toBe(true);
  });
});

describe("summarizeWork", () => {
  it("says what the folded work did", () => {
    const [turn] = turnsOf([
      message("u1", "user", [text("fix")]),
      message("a1", "assistant", [
        call("c1", "read", "src/a.ts"),
        call("c2", "read", "src/b.ts"),
        call("c3", "edit", "src/a.ts"),
        call("c4", "edit", "src/a.ts"),
        call("c5", "execute", "pnpm test"),
        call("c6", "execute", "pnpm lint"),
      ]),
      message("a2", "assistant", [text("Done.")]),
    ]);
    expect(summarizeWork(turn.work)).toEqual({
      title: "Ran 6 tools",
      detail: "read 2 files · edited a.ts · ran pnpm test and 1 more",
    });
  });

  it("has nothing to fold for fewer than two calls", () => {
    const [turn] = turnsOf([
      message("u1", "user", [text("ls")]),
      message("a1", "assistant", [call("c1", "execute", "ls")]),
    ]);
    expect(summarizeWork(turn.work)).toBeNull();
  });
});

describe("formatDuration", () => {
  it("reads like a clock for long runs", () => {
    expect([400, 8_200, 72_000].map(formatDuration)).toEqual(["0.4s", "8s", "1m 12s"]);
  });
});
