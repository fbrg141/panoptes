import { describe, expect, it } from "vitest";
import { emptyLane, workLabel, writersLabel } from "./laneRow";
import type { Lane, LaneNavigation, LaneWriter, Session } from "./types";

const lane: Lane = {
  id: 2,
  projectId: 1,
  name: "parser",
  task: "Build the parser",
  branch: "panoptes/parser",
  state: "active",
  worktrees: [],
};

const writer = (over: Partial<LaneWriter> = {}): LaneWriter => ({
  sessionKey: "panoptes:1",
  title: "Plan the work",
  laneId: 1,
  via: "edit",
  lastAt: 0,
  live: false,
  ...over,
});

const navigation = (over: Partial<LaneNavigation> = {}): LaneNavigation => ({
  laneId: 2,
  sessions: [],
  archivedSessions: [],
  work: { changedFiles: 0, commitsAhead: 0 },
  writers: [],
  ...over,
});

describe("lane row", () => {
  it("tells the three kinds of empty lane apart", () => {
    expect(emptyLane(lane, navigation())).toEqual({ kind: "notStarted" });
    expect(emptyLane(lane, undefined)).toEqual({ kind: "notStarted" });
    expect(emptyLane(lane, navigation({ work: { changedFiles: 2, commitsAhead: 0 } }))).toEqual({
      kind: "hasWork",
    });
    expect(emptyLane(lane, navigation({ work: { changedFiles: 0, commitsAhead: 1 } }))).toEqual({
      kind: "hasWork",
    });
    // Work another session made and then committed away still came from it.
    expect(emptyLane(lane, navigation({ writers: [writer()] }))).toEqual({ kind: "hasWork" });
    const live = writer({ live: true });
    expect(emptyLane(lane, navigation({ writers: [writer({ sessionKey: "x" }), live] }))).toEqual({
      kind: "beingWritten",
      writer: live,
    });
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

  it("names the latest writer and how it was seen", () => {
    const time = () => "4m ago";
    expect(writersLabel([], time)).toBeNull();
    expect(writersLabel([writer()], time)).toBe("Edited by “Plan the work” · 4m ago");
    expect(writersLabel([writer({ via: "shell" })], time)).toBe(
      "Changed by commands in “Plan the work” · 4m ago",
    );
    expect(writersLabel([writer({ live: true }), writer(), writer()], time)).toBe(
      "Being edited by “Plan the work” and 2 others · 4m ago",
    );
    expect(writersLabel([writer({ title: null })], time)).toBe(
      "Edited by a removed session · 4m ago",
    );
  });
});
