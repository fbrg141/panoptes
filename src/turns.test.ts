import { describe, expect, it } from "vitest";
import { emptyConversation } from "./conversation";
import {
  buildTurns,
  formatDuration,
  groupLabel,
  looseApprovals,
  sameTurn,
  summarizeWork,
  toolImages,
  toolSteps,
} from "./turns";
import type { Block, Message, ToolCall, ToolKind } from "./types";

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
  usage: null,
  ...over,
});
const text = (t: string): Block => ({ type: "text", text: t });
const call = (
  id: string,
  kind: ToolKind,
  subject: string,
  over: Partial<ToolCall> = {},
): Block => ({
  type: "toolCall",
  id,
  name: kind,
  kind,
  subject,
  input: {},
  status: "done",
  output: null,
  images: [],
  startedAt: null,
  endedAt: null,
  ...over,
});

const turnsOf = (messages: Message[], busy = false) =>
  buildTurns({ ...emptyConversation(messages), busy });

describe("buildTurns", () => {
  it("starts a turn at each prompt and times tool calls", () => {
    const output = { type: "shell", text: "a\nb", exitCode: 0 } as const;
    const turns = turnsOf([
      message("u1", "user", [text("list files")]),
      message("a1", "assistant", [
        call("c1", "execute", "ls", { output, startedAt: 1000, endedAt: 3500 }),
      ]),
      message("a2", "assistant", [text("Two files.")]),
      message("u2", "user", [text("thanks")]),
    ]);
    expect(turns.map((t) => t.key)).toEqual(["u1", "u2"]);
    const [first] = turns;
    expect(first.work).toMatchObject([
      { type: "tool", call: { status: "done", output }, durationMs: 2500 },
    ]);
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
        call("c3", "read", "b.ts", { status: "failed" }),
        call("c4", "execute", "pnpm test"),
        call("c5", "read", "c.ts"),
      ]),
    ]);
    // The failed read breaks the run, so only the first two group.
    expect(turn.work.map((s) => s.type)).toEqual(["group", "tool", "tool", "tool"]);
    expect(toolSteps(turn.work)).toHaveLength(5);
    const group = turn.work[0];
    expect(group.type === "group" && groupLabel(group.steps)).toBe(
      "Read 1 file, searched 1 pattern",
    );
  });

  it("keeps a read that returned an image out of a group", () => {
    const shot = { mimeType: "image/png", data: "aGVsbG8=" };
    const [turn] = turnsOf([
      message("u1", "user", [text("look")]),
      message("a1", "assistant", [
        call("c1", "read", "a.ts"),
        call("c2", "read", "shot.png", { images: [shot] }),
        call("c3", "read", "b.ts"),
      ]),
    ]);
    expect(turn.work.map((s) => s.type)).toEqual(["tool", "tool", "tool"]);
    expect(toolSteps(turn.work)[1].call.images).toEqual([shot]);
    // A finished turn folds its work; the images stay listed for the fold.
    expect(summarizeWork(turn.work)).not.toBeNull();
    expect(toolImages(turn.work)).toEqual([shot]);
  });

  it("puts an approval on the call it is about, outside any group", () => {
    const approval = {
      id: "a1",
      toolCallId: "c2",
      title: "Allow?",
      message: null,
      expiresAt: null,
    };
    const loose = { ...approval, id: "a2", toolCallId: "gone" };
    const conversation = {
      ...emptyConversation([
        message("u1", "user", [text("look")]),
        message("a1", "assistant", [
          call("c1", "read", "a.ts"),
          call("c2", "read", "b.ts", { status: "running" }),
        ]),
      ]),
      approvals: [approval, loose],
    };
    const [turn] = buildTurns(conversation);
    expect(turn.work.map((s) => s.type)).toEqual(["tool", "tool"]);
    expect(toolSteps(turn.work).map((s) => s.approval)).toEqual([null, approval]);
    expect(looseApprovals(conversation)).toEqual([loose]);
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

  it("adds up a turn's usage and ends it when its last call ended", () => {
    const usage = (endedAt: number, cost: number) => ({
      model: "m",
      provider: "p",
      inputTokens: 10,
      outputTokens: 5,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      cost,
      stopReason: "finished" as const,
      endedAt,
    });
    const [first, second] = turnsOf([
      message("u1", "user", [text("a")], { timestamp: 1_000 }),
      message("a1", "assistant", [text("b")], { timestamp: 1_200, usage: usage(3_000, 0.1) }),
      message("a2", "assistant", [text("c")], { timestamp: 3_100, usage: usage(8_000, 0.2) }),
      message("u2", "user", [text("d")], { timestamp: 9_000 }),
    ]);
    expect(first).toMatchObject({ startedAt: 1_000, endedAt: 8_000 });
    expect(first.usage).toMatchObject({ calls: 2, inputTokens: 20, outputTokens: 10 });
    expect(first.usage?.cost).toBeCloseTo(0.3);
    expect(second.usage).toBeNull();
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

describe("sameTurn", () => {
  const finished = [
    message("u1", "user", [text("First")]),
    message("a1", "assistant", [call("t1", "edit", "a.ts"), text("Done")]),
  ];
  const streaming = (reply: string) =>
    turnsOf(
      [
        ...finished,
        message("u2", "user", [text("Second")]),
        message("a2", "assistant", [text(reply)]),
      ],
      true,
    );

  it("treats a finished turn as unchanged when another turn streams", () => {
    const before = streaming("Hel");
    // The reducer keeps every untouched message by identity.
    const after = streaming("Hello");
    expect(sameTurn(before[0], after[0])).toBe(true);
    expect(sameTurn(before[1], after[1])).toBe(false);
  });

  it("notices a changed tool call and a turn finishing", () => {
    const base = turnsOf(finished);
    const updated = turnsOf([
      finished[0],
      message("a1", "assistant", [call("t1", "edit", "a.ts", { status: "failed" }), text("Done")]),
    ]);
    expect(sameTurn(base[0], updated[0])).toBe(false);
    expect(sameTurn(turnsOf(finished, true)[0], base[0])).toBe(false);
  });
});
