import { expect, it } from "vitest";
import { plainField, proseField } from "./textFields";

it("keeps native spellcheck on prose and turns all rewriting off", () => {
  expect(proseField).toEqual({
    spellCheck: true,
    autoCorrect: "off",
    autoCapitalize: "off",
    autoComplete: "off",
  });
  expect(plainField).toEqual({ ...proseField, spellCheck: false });
});

// Every text field opts out of macOS autocorrect through `proseField` or
// `plainField`, so a field added later cannot quietly bring it back.
const sources = import.meta.glob<string>("./components/*.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
});

it("turns autocorrect off in every text field", () => {
  const missing: string[] = [];
  for (const [file, source] of Object.entries(sources)) {
    for (const match of source.matchAll(/<(input|textarea|Input)\b/g)) {
      // Every field here is self-closing, so its props end at the first "/>".
      const tag = source.slice(match.index, source.indexOf("/>", match.index));
      if (!/\{\.\.\.(prose|plain)Field\}/.test(tag)) {
        const line = source.slice(0, match.index).split("\n").length;
        missing.push(`${file}:${line}`);
      }
    }
  }
  expect(missing).toEqual([]);
});
