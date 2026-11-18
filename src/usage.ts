import { formatTokens } from "./composer";
import type { Message, StopReason } from "./types";

/** What a run of model calls used, added up. */
export interface UsageTotal {
  /** Model calls that reported usage. */
  calls: number;
  /** Distinct models, in the order they first answered. */
  models: string[];
  /** Prompt tokens, cached or not. */
  inputTokens: number;
  outputTokens: number;
  /** Null when no call was priced. */
  cost: number | null;
  /** Why the last call ended. */
  stopReason: StopReason | null;
  /** When the last call ended, Unix milliseconds. */
  endedAt: number | null;
}

/** Adds up the usage the messages report; null when none does. */
export function sumUsage(messages: Message[]): UsageTotal | null {
  let total: UsageTotal | null = null;
  for (const { usage } of messages) {
    if (!usage) continue;
    total ??= {
      calls: 0,
      models: [],
      inputTokens: 0,
      outputTokens: 0,
      cost: null,
      stopReason: null,
      endedAt: null,
    };
    total.calls += 1;
    if (usage.model && !total.models.includes(usage.model)) total.models.push(usage.model);
    total.inputTokens += usage.inputTokens + usage.cacheReadTokens + usage.cacheWriteTokens;
    total.outputTokens += usage.outputTokens;
    if (usage.cost !== null) total.cost = (total.cost ?? 0) + usage.cost;
    total.stopReason = usage.stopReason;
    if (usage.endedAt !== null) total.endedAt = Math.max(total.endedAt ?? 0, usage.endedAt);
  }
  return total;
}

/** Whether two totals would show the same. */
export function sameUsage(a: UsageTotal | null, b: UsageTotal | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.calls === b.calls &&
    a.models.join("\n") === b.models.join("\n") &&
    a.inputTokens === b.inputTokens &&
    a.outputTokens === b.outputTokens &&
    a.cost === b.cost &&
    a.stopReason === b.stopReason &&
    a.endedAt === b.endedAt
  );
}

/** "$0.42", "<$0.01"; null for an unpriced or free run, which has nothing to show. */
export function formatCost(cost: number | null): string | null {
  if (cost === null || cost <= 0) return null;
  if (cost < 0.01) return "<$0.01";
  return `$${cost.toFixed(2)}`;
}

/** "12k in · 1.3k out". */
export function tokensLabel(total: UsageTotal): string {
  return `${formatTokens(total.inputTokens)} in · ${formatTokens(total.outputTokens)} out`;
}

/** The parts of a turn's footer that say what it used: model, tokens, cost, an unusual stop. */
export function turnUsageParts(total: UsageTotal): string[] {
  return [
    total.models.join(", "),
    tokensLabel(total),
    formatCost(total.cost),
    total.stopReason === "maxTokens" ? "Hit the output limit" : null,
  ].filter((part): part is string => Boolean(part));
}

/** The session total beside the composer: its cost, or its tokens when it has no price. */
export function sessionUsageLabel(total: UsageTotal): string {
  return formatCost(total.cost) ?? `${formatTokens(total.inputTokens + total.outputTokens)} tokens`;
}

/** The session total in full, for its tooltip. */
export function sessionUsageDetail(total: UsageTotal): string {
  const calls = `${total.calls} model ${total.calls === 1 ? "call" : "calls"}`;
  return [
    "This session",
    formatCost(total.cost),
    tokensLabel(total),
    calls,
    total.models.length > 0 ? total.models.join(", ") : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
