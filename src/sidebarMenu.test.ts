import { describe, expect, it } from "vitest";
import { laneMenuItems, projectMenuItems, sessionMenuItems } from "./sidebarMenu";
import type { Lane, Session } from "./types";

const ids = (items: ReturnType<typeof projectMenuItems>) =>
  items.flatMap((item) => ("id" in item ? [item.id] : []));

const session: Pick<Session, "laneId" | "archived" | "open"> = {
  laneId: null,
  archived: false,
  open: false,
};

describe("sidebar row menus", () => {
  it("offers lane creation and removal for a project", () => {
    expect(ids(projectMenuItems())).toEqual(["new-lane", "remove-project"]);
  });

  it("offers the actions appropriate to the lane state", () => {
    expect(ids(laneMenuItems({ state: "active" } satisfies Pick<Lane, "state">))).toEqual([
      "new-session",
      "archive-lane",
    ]);
    expect(ids(laneMenuItems({ state: "archived" }))).toEqual(["restore-lane"]);
  });

  it("lists session actions by name only, without icons", () => {
    const items = sessionMenuItems(
      { ...session, laneId: 1, open: true },
      { adoptable: true, laneActions: true },
    );
    expect(items.every((item) => !("icon" in item))).toBe(true);
  });

  it("offers adoption and stopping outside a lane", () => {
    expect(ids(sessionMenuItems({ ...session, open: true }, { adoptable: true }))).toEqual([
      "adopt",
      "stop",
    ]);
  });

  it("offers stopping and archiving for an active lane session", () => {
    expect(
      ids(sessionMenuItems({ ...session, laneId: 1, open: true }, { laneActions: true })),
    ).toEqual(["stop", "archive"]);
  });

  it("offers only unarchiving for an archived lane session", () => {
    const items = sessionMenuItems(
      { ...session, laneId: 1, archived: true },
      { laneActions: true },
    );
    expect(ids(items)).toEqual(["archive"]);
    expect(items[0]).toMatchObject({ label: "Unarchive" });
  });
});
