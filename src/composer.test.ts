import { describe, expect, it } from "vitest";
import {
  composePrompt,
  contextLabel,
  formatTokens,
  insertMention,
  matchFiles,
  mentionAt,
  nextLevel,
  restoreUnsent,
  segments,
} from "./composer";

describe("mentions", () => {
  it("finds the @query before the caret", () => {
    expect(mentionAt("look at @src/ap", 15)).toEqual({ start: 8, query: "src/ap" });
    expect(mentionAt("@", 1)).toEqual({ start: 0, query: "" });
  });

  it("ignores e-mail addresses and finished mentions", () => {
    expect(mentionAt("mail me@example.com", 19)).toBeNull();
    expect(mentionAt("@src/a.ts and", 13)).toBeNull();
  });

  it("replaces the query with the path and moves the caret after it", () => {
    const text = "see @ap please";
    const mention = mentionAt(text, 7);
    expect(mention).not.toBeNull();
    if (!mention) return;
    expect(insertMention(text, 7, mention, "src/api.ts")).toEqual({
      text: "see @src/api.ts please",
      caret: 16,
    });
    expect(insertMention("@ap", 3, { start: 0, query: "ap" }, "src/api.ts")).toEqual({
      text: "@src/api.ts ",
      caret: 12,
    });
  });

  it("splits out only known mentions", () => {
    expect(segments("fix @a.ts not @b.ts", new Set(["a.ts"]))).toEqual([
      { text: "fix ", mention: false },
      { text: "@a.ts", mention: true },
      { text: " not @b.ts", mention: false },
    ]);
    expect(segments("@a.ts", new Set(["a.ts"]))).toEqual([{ text: "@a.ts", mention: true }]);
  });
});

describe("matchFiles", () => {
  const files = [
    "src/components/ChatPane.tsx",
    "src/store.ts",
    "src/store.test.ts",
    "crates/panoptes-pi/src/map.rs",
    "docs/decisions.md",
  ];

  it("prefers matches in the file name", () => {
    expect(matchFiles("store", files)).toEqual(["src/store.ts", "src/store.test.ts"]);
    expect(matchFiles("chat", files)[0]).toBe("src/components/ChatPane.tsx");
    expect(matchFiles("cp", files)[0]).toBe("src/components/ChatPane.tsx");
  });

  it("matches across folders and drops non-matches", () => {
    expect(matchFiles("pimap", files)).toEqual(["crates/panoptes-pi/src/map.rs"]);
    expect(matchFiles("zzz", files)).toEqual([]);
  });

  it("lists the shortest paths for an empty query, up to the limit", () => {
    expect(matchFiles("", files, 2)).toEqual(["src/store.ts", "docs/decisions.md"]);
  });
});

describe("labels", () => {
  it("shortens token counts", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1200)).toBe("1.2k");
    expect(formatTokens(42_400)).toBe("42k");
    expect(formatTokens(200_000)).toBe("200k");
    expect(formatTokens(1_000_000)).toBe("1M");
  });

  it("shows usage out of the window", () => {
    expect(contextLabel({ tokens: 42_000, window: 200_000 })).toBe("42k / 200k");
    expect(contextLabel({ tokens: null, window: 200_000 })).toBe("200k context");
  });

  it("cycles thinking levels", () => {
    const levels = ["off", "low", "high"];
    expect(nextLevel(levels, "low")).toBe("high");
    expect(nextLevel(levels, "high")).toBe("off");
    expect(nextLevel(levels, null)).toBe("off");
    expect(nextLevel([], "off")).toBeNull();
  });
});

describe("composePrompt", () => {
  it("sends images inline and other files as mentions", () => {
    const prompt = composePrompt(" fix this ", [
      { name: "a.png", path: "/a.png", mimeType: "image/png", data: "AQID" },
      { name: "log.txt", path: "/tmp/log.txt", mimeType: null, data: null },
    ]);
    expect(prompt).toEqual({
      text: "fix this\n\n@/tmp/log.txt",
      images: [{ mimeType: "image/png", data: "AQID" }],
    });
  });

  it("allows an attachment without text", () => {
    expect(composePrompt("", [{ name: "x", path: "/x", mimeType: null, data: null }]).text).toBe(
      "@/x",
    );
  });
});

describe("restoreUnsent", () => {
  const unsent = { text: "fix the login", attachments: [], mentioned: ["a.ts"] };

  it("gives the unsent prompt back to an empty composer", () => {
    expect(restoreUnsent(unsent, undefined)).toEqual(unsent);
    expect(restoreUnsent(unsent, { text: "  ", attachments: [], mentioned: [] })).toEqual(unsent);
  });

  it("keeps what was typed since, after the unsent prompt", () => {
    expect(
      restoreUnsent(unsent, { text: "and the tests", attachments: [], mentioned: ["b.ts"] }),
    ).toEqual({
      text: "fix the login\nand the tests",
      attachments: [],
      mentioned: ["a.ts", "b.ts"],
    });
  });
});
