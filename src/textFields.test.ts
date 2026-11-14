import { expect, it } from "vitest";

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
