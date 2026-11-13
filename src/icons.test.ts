import { ICON_NAMES } from "@panoptes/design-system";
import { describe, expect, it } from "vitest";

// `Icon` takes any string, so TypeScript cannot catch a name outside the
// design system's set; an unknown name renders blank. This reads every
// literal icon name in the app's source and checks it against the set.
const sources = import.meta.glob(["./**/*.{ts,tsx}", "!./**/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

describe("icon names", () => {
  it("are all in the design system's set", () => {
    const used = new Set<string>();
    for (const text of Object.values(sources)) {
      // `icon: "x"` and `icon="x"` in items and buttons, `<Icon name="x"`.
      for (const match of text.matchAll(
        /\bicon[:=]\s*"([a-z0-9-]+)"|<Icon\s+name="([a-z0-9-]+)"/g,
      )) {
        used.add(match[1] ?? match[2]);
      }
    }
    expect(used.size).toBeGreaterThan(10);
    expect([...used].filter((name) => !ICON_NAMES.includes(name))).toEqual([]);
  });
});
