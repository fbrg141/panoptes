import { describe, expect, it } from "vitest";
import {
  DEFAULT_CHOICE,
  modelGroups,
  profileGroups,
  shortModelId,
  thinkingGroups,
} from "./newSession";
import type { ModelInfo } from "./types";

const model = (id: string, name: string, provider: string): ModelInfo => ({
  id,
  name,
  provider,
  contextWindow: null,
  images: true,
});

describe("model pickers", () => {
  it("groups models by provider after the harness default", () => {
    const groups = modelGroups(
      [
        model("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5", "anthropic"),
        model("local/qwen", "Qwen", "local"),
        model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5", "anthropic"),
      ],
      "local/qwen",
    );
    expect(groups.map((group) => group.label)).toEqual([undefined, "anthropic", "local"]);
    expect(groups[0].items[0]).toMatchObject({ id: DEFAULT_CHOICE, current: false });
    expect(groups[1].items.map((item) => item.detail)).toEqual([
      "claude-sonnet-4-5",
      "claude-haiku-4-5",
    ]);
    expect(groups[2].items[0].current).toBe(true);
  });

  it("puts favorites first, hides models only from pickers and keeps current ticks", () => {
    const groups = modelGroups(
      [
        model("anthropic/claude-sonnet", "Claude Sonnet", "anthropic"),
        model("openai/gpt", "GPT", "openai"),
        model("anthropic/claude-haiku", "Claude Haiku", "anthropic"),
        model("local/qwen", "Qwen", "local"),
      ],
      "anthropic/claude-haiku",
      {
        favoriteModels: ["openai/gpt", "anthropic/claude-sonnet", "missing/model"],
        hiddenModels: ["local/qwen"],
      },
    );
    expect(groups.map((group) => group.label)).toEqual([undefined, "Favorites", "anthropic"]);
    expect(groups[1].items.map((item) => item.id)).toEqual([
      "openai/gpt",
      "anthropic/claude-sonnet",
    ]);
    expect(groups[2].items.map((item) => item.id)).toEqual(["anthropic/claude-haiku"]);
    expect(groups[2].items[0].current).toBe(true);
    expect(groups.flatMap((group) => group.items)).not.toContainEqual(
      expect.objectContaining({ id: "local/qwen" }),
    );
  });

  it("offers no profile before the saved profiles", () => {
    const [group] = profileGroups(
      [
        {
          id: 4,
          name: "Careful",
          modelId: "anthropic/claude-sonnet",
          thinkingLevel: "high",
          instructions: null,
        },
      ],
      4,
    );
    expect(group.items.map((item) => item.id)).toEqual([DEFAULT_CHOICE, "4"]);
    expect(group.items[1]).toMatchObject({
      label: "Careful",
      detail: "anthropic/claude-sonnet",
      current: true,
    });
  });

  it("shortens a model id only when it carries its provider", () => {
    expect(shortModelId({ id: "openai/gpt-5", provider: "openai" })).toBe("gpt-5");
    expect(shortModelId({ id: "gpt-5", provider: "openai" })).toBe("gpt-5");
  });

  it("offers the model's default before its thinking levels", () => {
    const [group] = thinkingGroups(["low", "high"], "high");
    expect(group.items.map((item) => item.id)).toEqual([DEFAULT_CHOICE, "low", "high"]);
    expect(group.items[2].current).toBe(true);
  });
});
