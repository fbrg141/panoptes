import { describe, expect, it } from "vitest";
import { splitMarkdown } from "./markdown";

describe("splitMarkdown", () => {
  it("splits at blank lines", () => {
    expect(splitMarkdown("# Title\n\nOne.\n\n\nTwo.")).toEqual(["# Title", "One.", "Two."]);
  });

  it("keeps code fences whole", () => {
    const fence = "```ts\nconst a = 1;\n\nconst b = 2;\n```";
    expect(splitMarkdown(`Before.\n\n${fence}\n\nAfter.`)).toEqual(["Before.", fence, "After."]);
  });

  it("keeps a list item's indented paragraph with it", () => {
    expect(splitMarkdown("- one\n\n  more\n- two")).toEqual(["- one\n\n  more\n- two"]);
  });
});
