// Window shortcuts, in one place so the key handler and the cheatsheet never
// drift apart. Panoptes is a macOS app, so only ⌘ counts: Ctrl+P, Ctrl+N and
// Ctrl+T are text-editing keys there and must reach the composer.

/** The shortcuts this file knows how to detect. */
export type ShortcutId = "search" | "newSession" | "newProject" | "shortcuts";

/** The shortcut a keyboard event asks for, or null if it is none. */
export function shortcutFor(
  e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "key">,
): ShortcutId | null {
  if (!e.metaKey || e.ctrlKey || e.altKey) return null;
  // ⌘⇧P and the like are other shortcuts, not these.
  const letter = (id: ShortcutId) => (e.shiftKey ? null : id);
  switch (e.key.toLowerCase()) {
    case "p":
      return letter("search");
    case "t":
      return letter("newSession");
    case "n":
      return letter("newProject");
    // ⌘/ arrives as "/" unshifted and "?" with shift held.
    case "/":
    case "?":
      return "shortcuts";
    default:
      return null;
  }
}

export interface ShortcutEntry {
  label: string;
  keys: string[];
}

export interface ShortcutGroup {
  label: string;
  entries: ShortcutEntry[];
}

/** Every shortcut the app responds to, as the cheatsheet shows it. */
export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    label: "Panoptes",
    entries: [
      { label: "Search projects, lanes and sessions", keys: ["⌘", "P"] },
      { label: "New lane", keys: ["⌘", "T"] },
      { label: "Add a project", keys: ["⌘", "N"] },
      { label: "Keyboard shortcuts", keys: ["⌘", "/"] },
    ],
  },
  {
    label: "Composer",
    entries: [
      { label: "Send the message", keys: ["↵"] },
      { label: "New line", keys: ["⇧", "↵"] },
      { label: "Queue for after the run", keys: ["⌥", "↵"] },
      { label: "Stop the agent", keys: ["Esc"] },
      { label: "Mention a file", keys: ["@"] },
      { label: "Cycle the thinking level", keys: ["⇧", "Tab"] },
    ],
  },
  {
    label: "Reading",
    entries: [{ label: "Jump to the latest output", keys: ["⌘", "↓"] }],
  },
  {
    label: "Zoom",
    entries: [
      { label: "Zoom in or out", keys: ["⌘", "+/−"] },
      { label: "Reset the zoom", keys: ["⌘", "0"] },
    ],
  },
];

/**
 * The cheatsheet narrowed to `query`: an entry stays when its label or its
 * group's name contains every word typed, and groups left empty drop out.
 */
export function filterShortcuts(groups: ShortcutGroup[], query: string): ShortcutGroup[] {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return groups;
  return groups
    .map((group) => ({
      ...group,
      entries: group.entries.filter((entry) => {
        const text = `${group.label} ${entry.label}`.toLowerCase();
        return words.every((word) => text.includes(word));
      }),
    }))
    .filter((group) => group.entries.length > 0);
}
