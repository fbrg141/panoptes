import { describe, expect, it } from "vitest";
import { laneMenuItems, projectMenuItems, sessionMenuItems } from "./sidebarMenu";
import type { Lane, Session } from "./types";

const ids = (items: ReturnType<typeof projectMenuItems>) =>
  items.flatMap((item) => ("id" in item ? [item.id] : []));

const session: Pick<Session, "laneId" | "pinned" | "archived" | "open"> = {
  laneId: null,
  pinned: false,
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

  it("offers adoption, pinning and stopping outside a lane", () => {
    expect(ids(sessionMenuItems({ ...session, open: true }, { adoptable: true }))).toEqual([
      "adopt",
      "pin",
      "stop",
    ]);
  });

  it("offers all applicable actions for an active lane session", () => {
    expect(
      ids(
        sessionMenuItems(
          { ...session, laneId: 1, pinned: true, open: true },
          { laneActions: true },
        ),
      ),
    ).toEqual(["pin", "stop", "replace", "archive"]);
    expect(
      sessionMenuItems({ ...session, laneId: 1, pinned: true }, { laneActions: true })[0],
    ).toMatchObject({ label: "Unpin" });
  });

  it("does not offer replacement for an archived lane session", () => {
    expect(
      ids(
        sessionMenuItems(
          { ...session, laneId: 1, archived: true },
          { laneActions: true, laneActive: false },
        ),
      ),
    ).toEqual(["pin", "archive"]);
    expect(
      sessionMenuItems(
        { ...session, laneId: 1, archived: true },
        { laneActions: true, laneActive: false },
      )[1],
    ).toMatchObject({ label: "Unarchive" });
  });
});
