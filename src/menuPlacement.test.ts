import { describe, expect, it } from "vitest";
import { MENU_GAP, placeMenu } from "./menuPlacement";

const viewport = { width: 800, height: 600 };
const menu = { width: 220, height: 80 };
const trigger = (top: number, right = 300) => ({
  left: right - 24,
  top,
  right,
  bottom: top + 24,
});

describe("row menu placement", () => {
  it("opens below the trigger, right edges lined up", () => {
    expect(placeMenu(trigger(100), menu, viewport)).toEqual({
      left: 300 - 220,
      top: 124 + MENU_GAP,
    });
  });

  it("flips above the trigger near the bottom of the window", () => {
    expect(placeMenu(trigger(550), menu, viewport)).toEqual({
      left: 80,
      top: 550 - MENU_GAP - 80,
    });
  });

  it("stays below when neither side fits and below has more room", () => {
    const tall = { width: 220, height: 500 };
    expect(placeMenu(trigger(200), tall, viewport).top).toBe(600 - MENU_GAP - 500);
  });

  it("keeps a trigger near the left edge from pushing the menu off the window", () => {
    expect(placeMenu(trigger(100, 60), menu, viewport).left).toBe(MENU_GAP);
  });
});
