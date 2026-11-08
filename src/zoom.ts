// Window zoom driven by ⌘+/- (and ⌘0 to reset). The key mapping and step
// logic live here so they can be tested without a window; App keeps the
// current scale and asks the shell to apply it.

export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2;
const ZOOM_STEP = 0.1;

export type ZoomCommand = "in" | "out" | "reset";

/** The zoom command a keyboard event asks for, or null if it is none. */
export function zoomCommand(
  e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "key">,
): ZoomCommand | null {
  if (!e.metaKey && !e.ctrlKey) return null;
  switch (e.key) {
    case "+":
    case "=": // ⌘= is what macOS reports for the plus key, shifted or not
    case "NumpadAdd":
      return "in";
    case "-":
    case "_":
    case "NumpadSubtract":
      return "out";
    case "0":
      return "reset";
    default:
      return null;
  }
}

/** The next zoom scale, clamped to [MIN_ZOOM, MAX_ZOOM]. */
export function nextZoom(current: number, command: ZoomCommand): number {
  const target = command === "reset" ? 1 : current + (command === "in" ? ZOOM_STEP : -ZOOM_STEP);
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(target * 10) / 10));
}
