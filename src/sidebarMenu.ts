import type { MenuItem } from "@panoptes/design-system";
import type { Lane, Session } from "./types";

export const projectMenuItems = (): MenuItem[] => [
  { id: "new-lane", label: "New lane", icon: "git-branch" },
  { separator: true },
  { id: "remove-project", label: "Remove project", icon: "trash-2", danger: true },
];

export const laneMenuItems = (lane: Pick<Lane, "state">): MenuItem[] =>
  lane.state === "active"
    ? [
        { id: "new-session", label: "New session", icon: "plus" },
        { separator: true },
        { id: "archive-lane", label: "Archive lane", icon: "x" },
      ]
    : [{ id: "restore-lane", label: "Restore lane", icon: "rotate-ccw" }];

export const sessionMenuItems = (
  session: Pick<Session, "laneId" | "pinned" | "archived" | "open">,
  { adoptable = false, laneActions = false, laneActive = true } = {},
): MenuItem[] => {
  const inLane = laneActions && session.laneId !== null;
  const items: MenuItem[] = [];
  if (adoptable) items.push({ id: "adopt", label: "Adopt into lane", icon: "git-branch" });
  items.push({ id: "pin", label: session.pinned ? "Unpin" : "Pin" });
  if (session.open) items.push({ id: "stop", label: "Stop", icon: "square" });
  if (inLane && laneActive && !session.archived) {
    items.push({ id: "replace", label: "Replace", icon: "rotate-ccw" });
  }
  if (inLane) {
    items.push({
      id: "archive",
      label: session.archived ? "Unarchive" : "Archive",
      icon: session.archived ? "rotate-ccw" : "x",
    });
  }
  return items;
};
