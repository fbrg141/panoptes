import { describe, expect, it } from "vitest";
import { filterShortcuts, SHORTCUT_GROUPS, shortcutFor } from "./shortcuts";

const key = (
  k: string,
  mods: { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean } = {},
) =>
  shortcutFor({
    key: k,
    metaKey: mods.meta ?? false,
    ctrlKey: mods.ctrl ?? false,
    altKey: mods.alt ?? false,
    shiftKey: mods.shift ?? false,
  });

describe("shortcutFor", () => {
  it("maps the window shortcuts on ⌘", () => {
    expect(key("p", { meta: true })).toBe("search");
    expect(key("t", { meta: true })).toBe("newSession");
    expect(key("n", { meta: true })).toBe("newProject");
  });

  it("leaves Ctrl to text editing, as macOS does", () => {
    expect(key("p", { ctrl: true })).toBeNull();
    expect(key("t", { ctrl: true })).toBeNull();
    expect(key("n", { ctrl: true })).toBeNull();
    expect(key("p", { meta: true, ctrl: true })).toBeNull();
  });

  it("does not take ⌘ with shift or option on a letter", () => {
    expect(key("P", { meta: true, shift: true })).toBeNull();
    expect(key("t", { meta: true, alt: true })).toBeNull();
  });

  it("takes the cheatsheet with or without shift", () => {
    expect(key("/", { meta: true })).toBe("shortcuts");
    expect(key("?", { meta: true, shift: true })).toBe("shortcuts");
  });

  it("ignores everything else", () => {
    expect(key("p")).toBeNull();
    expect(key("x", { meta: true })).toBeNull();
  });
});

describe("filterShortcuts", () => {
  it("keeps everything for an empty query", () => {
    expect(filterShortcuts(SHORTCUT_GROUPS, "  ")).toBe(SHORTCUT_GROUPS);
  });

  it("matches every word against the entry and its group, dropping empty groups", () => {
    const result = filterShortcuts(SHORTCUT_GROUPS, "zoom reset");
    expect(result.map((group) => group.label)).toEqual(["Zoom"]);
    expect(result[0].entries.map((entry) => entry.label)).toEqual(["Reset the zoom"]);

    const byGroup = filterShortcuts(SHORTCUT_GROUPS, "composer");
    expect(byGroup).toHaveLength(1);
    expect(byGroup[0].entries.length).toBeGreaterThan(1);

    expect(filterShortcuts(SHORTCUT_GROUPS, "nothing like this")).toEqual([]);
  });
});
