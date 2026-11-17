import { describe, expect, it } from "vitest";
import type { Message, Usage } from "./types";
import {
  formatCost,
  sameUsage,
  sessionUsageDetail,
  sessionUsageLabel,
  sumUsage,
  turnUsageParts,
} from "./usage";

const usage = (over: Partial<Usage> = {}): Usage => ({
  model: "claude-sonnet-4-5",
  provider: "anthropic",
  inputTokens: 1_000,
  outputTokens: 200,
  cacheReadTokens: 9_000,
  cacheWriteTokens: 0,
  cost: 0.01,
  stopReason: "toolUse",
  endedAt: 5_000,
  ...over,
});

/** The total of messages known to report usage. */
const totalOf = (messages: Message[]) => {
  const total = sumUsage(messages);
  if (!total) throw new Error("no usage");
  return total;
};

const message = (id: string, u: Usage | null): Message => ({
  id,
  role: u ? "assistant" : "user",
  blocks: [],
  timestamp: null,
  error: null,
  stopped: false,
  usage: u,
});

describe("sumUsage", () => {
  it("adds up every call and keeps the last stop and end", () => {
    const total = sumUsage([
      message("u", null),
      message("a1", usage()),
      message("a2", usage({ model: "gpt-5", cost: 0.02, stopReason: "finished", endedAt: 9_000 })),
    ]);
    expect(total).toEqual({
      calls: 2,
      models: ["claude-sonnet-4-5", "gpt-5"],
      inputTokens: 20_000,
      outputTokens: 400,
      cost: 0.03,
      stopReason: "finished",
      endedAt: 9_000,
    });
  });

  it("is null without usage, and unpriced calls leave the cost unknown", () => {
    expect(sumUsage([message("u", null)])).toBeNull();
    expect(sumUsage([message("a", usage({ cost: null }))])?.cost).toBeNull();
  });

  it("compares totals by value", () => {
    const messages = [message("a", usage())];
    expect(sameUsage(sumUsage(messages), sumUsage(messages))).toBe(true);
    expect(sameUsage(sumUsage(messages), sumUsage([message("a", usage({ cost: 1 }))]))).toBe(false);
    expect(sameUsage(null, null)).toBe(true);
  });
});

describe("labels", () => {
  it("formats cost, hiding free and unpriced runs", () => {
    expect(formatCost(1.234)).toBe("$1.23");
    expect(formatCost(0.004)).toBe("<$0.01");
    expect(formatCost(0)).toBeNull();
    expect(formatCost(null)).toBeNull();
  });

  it("says what a turn used and flags a cut-off answer", () => {
    const total = totalOf([message("a", usage({ stopReason: "maxTokens", cost: 0.42 }))]);
    expect(turnUsageParts(total)).toEqual([
      "claude-sonnet-4-5",
      "10k in · 200 out",
      "$0.42",
      "Hit the output limit",
    ]);
    const free = totalOf([message("a", usage({ model: null, cost: 0 }))]);
    expect(turnUsageParts(free)).toEqual(["10k in · 200 out"]);
  });

  it("shows a session's cost, or its tokens when it has no price", () => {
    const priced = totalOf([message("a", usage()), message("b", usage())]);
    expect(sessionUsageLabel(priced)).toBe("$0.02");
    expect(sessionUsageDetail(priced)).toBe(
      "This session · $0.02 · 20k in · 400 out · 2 model calls · claude-sonnet-4-5",
    );
    const free = totalOf([message("a", usage({ cost: 0 }))]);
    expect(sessionUsageLabel(free)).toBe("10k tokens");
  });
});
