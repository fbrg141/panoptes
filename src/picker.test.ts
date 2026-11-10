import { describe, expect, it } from "vitest";
import { filterPicker, initialPick, type PickerGroup, pickable } from "./picker";

const groups: PickerGroup[] = [
  {
    label: "anthropic",
    items: [
      {
        id: "anthropic/claude-sonnet-4-5",
        label: "Claude Sonnet 4.5",
        detail: "claude-sonnet-4-5",
        keywords: ["anthropic"],
        current: true,
      },
      { id: "anthropic/claude-haiku-4-5", label: "Claude Haiku 4.5", keywords: ["anthropic"] },
    ],
  },
  {
    label: "local",
    items: [
      { id: "local/off", label: "Offline", disabled: true },
      { id: "local/qwen3-coder", label: "Qwen3 Coder", detail: "qwen3-coder", keywords: ["local"] },
    ],
  },
];

describe("filterPicker", () => {
  it("keeps everything for an empty query", () => {
    expect(filterPicker(groups, " ")).toBe(groups);
  });

  it("matches label, detail and keywords, every word, keeping order", () => {
    expect(pickable(filterPicker(groups, "anthropic")).map((item) => item.id)).toEqual([
      "anthropic/claude-sonnet-4-5",
      "anthropic/claude-haiku-4-5",
    ]);
    expect(pickable(filterPicker(groups, "sonnet-4")).map((item) => item.id)).toEqual([
      "anthropic/claude-sonnet-4-5",
    ]);
    expect(pickable(filterPicker(groups, "claude haiku")).map((item) => item.id)).toEqual([
      "anthropic/claude-haiku-4-5",
    ]);
  });

  it("drops groups that end up empty", () => {
    expect(filterPicker(groups, "qwen").map((group) => group.label)).toEqual(["local"]);
    expect(filterPicker(groups, "nothing")).toEqual([]);
  });
});

describe("pickable and initialPick", () => {
  it("skips disabled items and starts on the current one", () => {
    expect(pickable(groups).map((item) => item.id)).not.toContain("local/off");
    expect(initialPick(groups)).toBe(0);
    expect(
      initialPick([
        {
          items: [
            { id: "a", label: "A" },
            { id: "b", label: "B", current: true },
          ],
        },
      ]),
    ).toBe(1);
    expect(initialPick([{ items: [{ id: "a", label: "A" }] }])).toBe(0);
  });
});
