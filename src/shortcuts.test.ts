import { describe, expect, it } from "vitest";
import { shortcutFor } from "./shortcuts";

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
