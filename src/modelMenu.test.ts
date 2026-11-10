import { describe, expect, it } from "vitest";
import { filterModels } from "./modelMenu";
import type { ModelInfo } from "./types";

const model = (id: string, name: string, provider: string): ModelInfo => ({
  id,
  name,
  provider,
  contextWindow: null,
  images: true,
});

const models = [
  model("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5", "anthropic"),
  model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5", "anthropic"),
  model("local/qwen3-coder", "Qwen3 Coder", "local"),
];

describe("filterModels", () => {
  it("matches by name, id and provider without reordering", () => {
    expect(filterModels(models, "sonnet").map((m) => m.id)).toEqual([
      "anthropic/claude-sonnet-4-5",
    ]);
    expect(filterModels(models, "qwen3-coder").map((m) => m.id)).toEqual(["local/qwen3-coder"]);
    expect(filterModels(models, "ANTHROPIC").map((m) => m.id)).toEqual([
      "anthropic/claude-sonnet-4-5",
      "anthropic/claude-haiku-4-5",
    ]);
  });

  it("keeps every model for an empty query and marks the current one", () => {
    const rows = filterModels(models, "  ", "local/qwen3-coder");
    expect(rows).toHaveLength(3);
    expect(rows.map((r) => r.current)).toEqual([false, false, true]);
  });
});
