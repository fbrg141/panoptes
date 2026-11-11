/** A box in viewport pixels, as `getBoundingClientRect` gives it. */
export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** The gap between a trigger and its menu, and between a menu and the window edge. */
export const MENU_GAP = 4;

/**
 * Where a row menu goes: below its trigger with their right edges lined up,
 * flipped above when it would run past the bottom and there is more room
 * above, and kept inside the window. Returns the menu's top-left corner.
 */
export function placeMenu(
  trigger: Box,
  menu: { width: number; height: number },
  viewport: { width: number; height: number },
): { left: number; top: number } {
  const below = trigger.bottom + MENU_GAP;
  const above = trigger.top - MENU_GAP - menu.height;
  const fitsBelow = below + menu.height <= viewport.height - MENU_GAP;
  const roomAbove = trigger.top - MENU_GAP;
  const roomBelow = viewport.height - trigger.bottom - MENU_GAP;
  const top = fitsBelow || roomBelow >= roomAbove ? below : above;
  return {
    left: clamp(trigger.right - menu.width, MENU_GAP, viewport.width - MENU_GAP - menu.width),
    top: clamp(top, MENU_GAP, viewport.height - MENU_GAP - menu.height),
  };
}

function clamp(value: number, min: number, max: number) {
  return Math.max(min, Math.min(value, Math.max(min, max)));
}
