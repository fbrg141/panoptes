import { describe, expect, it } from "vitest";
import { filterShortcuts, SHORTCUT_GROUPS, shortcutFor } from "./shortcuts";

const key = (k: string, mods: { meta?: boolean; ctrl?: boolean } = {}) =>
  shortcutFor({ key: k, metaKey: mods.meta ?? false, ctrlKey: mods.ctrl ?? false });

describe("shortcutFor", () => {
  it("maps the window shortcuts with either command key", () => {
    expect(key("p", { meta: true })).toBe("search");
    expect(key("P", { ctrl: true })).toBe("search");
    expect(key("t", { meta: true })).toBe("newSession");
    expect(key("T", { ctrl: true })).toBe("newSession");
    expect(key("n", { meta: true })).toBe("newProject");
    expect(key("N", { ctrl: true })).toBe("newProject");
  });

  it("takes the cheatsheet with or without shift", () => {
    expect(key("/", { meta: true })).toBe("shortcuts");
    expect(key("?", { ctrl: true })).toBe("shortcuts");
  });

  it("ignores everything else", () => {
    expect(key("p")).toBeNull();
    expect(key("x", { meta: true })).toBeNull();
    expect(key("p", { meta: true, ctrl: true })).toBe("search");
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
