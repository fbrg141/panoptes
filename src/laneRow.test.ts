import { describe, expect, it } from "vitest";
import { emptyLane, workLabel } from "./laneRow";
import type { Lane, LaneNavigation, Session } from "./types";

const lane: Lane = {
  id: 2,
  projectId: 1,
  name: "parser",
  task: "Build the parser",
  branch: "panoptes/parser",
  state: "active",
  worktrees: [],
};

const navigation = (over: Partial<LaneNavigation> = {}): LaneNavigation => ({
  laneId: 2,
  sessions: [],
  archivedSessions: [],
  work: { changedFiles: 0, commitsAhead: 0 },
  busyWriter: null,
  ...over,
});

describe("lane row", () => {
  it("offers a session unless another one is still changing the lane", () => {
    expect(emptyLane(lane, navigation())).toEqual({ kind: "startable" });
    expect(emptyLane(lane, undefined)).toEqual({ kind: "startable" });
    expect(
      emptyLane(
        lane,
        navigation({ busyWriter: { sessionKey: "panoptes:1", title: "Plan the work" } }),
      ),
    ).toEqual({ kind: "busy", writer: "Plan the work" });
  });

  it("leaves lanes with a session, and archived lanes, alone", () => {
    const session = { key: "panoptes:2" } as Session;
    expect(emptyLane(lane, navigation({ sessions: [session] }))).toBeNull();
    expect(emptyLane(lane, navigation({ archivedSessions: [session] }))).toBeNull();
    expect(emptyLane({ ...lane, state: "archived" }, navigation())).toBeNull();
  });

  it("counts what a worktree holds", () => {
    expect(workLabel(null)).toBeNull();
    expect(workLabel({ changedFiles: 0, commitsAhead: 0 })).toBe("No changes yet");
    expect(workLabel({ changedFiles: 0, commitsAhead: null })).toBe("No changes yet");
    expect(workLabel({ changedFiles: 1, commitsAhead: 0 })).toBe("1 file changed");
    expect(workLabel({ changedFiles: 6, commitsAhead: 2 })).toBe(
      "6 files changed · 2 commits ahead",
    );
    expect(workLabel({ changedFiles: 0, commitsAhead: 1 })).toBe("1 commit ahead");
  });
});
