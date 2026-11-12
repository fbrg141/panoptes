import type { MenuItem } from "@panoptes/design-system";
import type { Lane, Session } from "./types";

export const projectMenuItems = (): MenuItem[] => [
  { id: "new-lane", label: "New lane", icon: "git-branch" },
  { separator: true },
  { id: "remove-project", label: "Remove project", icon: "trash-2", danger: true },
];

export const laneMenuItems = (lane: Pick<Lane, "state">): MenuItem[] =>
  lane.state === "active"
    ? [{ id: "archive-lane", label: "Archive lane", icon: "x" }]
    : [{ id: "restore-lane", label: "Restore lane", icon: "rotate-ccw" }];

/** The items in a session row's right-click menu: adopt, stop, archive. */
export const sessionMenuItems = (
  session: Pick<Session, "laneId" | "archived" | "open">,
  { adoptable = false, laneActions = false } = {},
): MenuItem[] => {
  const items: MenuItem[] = [];
  if (adoptable) items.push({ id: "adopt", label: "Adopt into lane" });
  if (session.open) items.push({ id: "stop", label: "Stop" });
  if (laneActions && session.laneId !== null) {
    items.push({ id: "archive", label: session.archived ? "Unarchive" : "Archive" });
  }
  return items;
};
