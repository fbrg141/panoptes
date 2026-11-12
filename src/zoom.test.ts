import { describe, expect, it } from "vitest";
import { MAX_ZOOM, MIN_ZOOM, nextZoom, zoomCommand } from "./zoom";

const key = (key: string, over: Partial<KeyboardEvent> = {}) =>
  ({ key, metaKey: true, ctrlKey: false, altKey: false, ...over }) as KeyboardEvent;

describe("zoomCommand", () => {
  it("recognises the zoom keys", () => {
    expect(zoomCommand(key("="))).toBe("in");
    expect(zoomCommand(key("+"))).toBe("in");
    expect(zoomCommand(key("-"))).toBe("out");
    expect(zoomCommand(key("_"))).toBe("out");
    expect(zoomCommand(key("0"))).toBe("reset");
  });

  it("needs ⌘ alone, and leaves other shortcuts alone", () => {
    expect(zoomCommand(key("=", { metaKey: false }))).toBeNull();
    expect(zoomCommand(key("=", { metaKey: false, ctrlKey: true }))).toBeNull();
    expect(zoomCommand(key("0", { altKey: true }))).toBeNull();
    expect(zoomCommand(key("a"))).toBeNull();
    expect(zoomCommand(key("ArrowDown"))).toBeNull();
  });
});

describe("nextZoom", () => {
  it("steps by a tenth in each direction", () => {
    expect(nextZoom(1, "in")).toBe(1.1);
    expect(nextZoom(1.1, "out")).toBe(1);
  });

  it("resets to 1", () => {
    expect(nextZoom(1.6, "reset")).toBe(1);
  });

  it("clamps at both ends", () => {
    expect(nextZoom(MAX_ZOOM, "in")).toBe(MAX_ZOOM);
    expect(nextZoom(MIN_ZOOM, "out")).toBe(MIN_ZOOM);
  });
});
