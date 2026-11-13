import { describe, expect, it } from "vitest";
import { paletteGroups, paletteTarget, recentSessions } from "./palette";
import type { Lane, Project, Session } from "./types";

const project: Project = {
  id: 1,
  repo: "/code/web",
  name: "Web app",
  baseBranch: "main",
  copyFiles: [],
  setup: null,
};

const lane: Lane = {
  id: 4,
  projectId: 1,
  name: "fix-login",
  task: "Fix login",
  branch: "panoptes/fix-login",
  state: "active",
  worktrees: [],
};

const session: Session = {
  key: "pi:s1",
  id: null,
  laneId: null,
  harness: "pi",
  harnessRef: null,
  cwd: "/code/web",
  title: "Release checklist",
  tags: [],
  createdAt: 0,
  updatedAt: 0,
  messageCount: 0,
  parentLocator: null,
  projectRoot: "/code/web",
  checkout: "/code/web",
  projectId: 1,
  pinned: false,
  archived: false,
  open: false,
};

describe("paletteGroups", () => {
  it("lists sessions, active lanes, projects and actions with traceable ids", () => {
    const groups = paletteGroups(
      [session],
      [project],
      [lane, { ...lane, id: 5, state: "archived" }],
    );
    expect(groups.map((group) => group.label)).toEqual([
      "Sessions",
      "Lanes",
      "Projects",
      "Actions",
    ]);
    expect(groups[3].items.map((item) => item.id)).toEqual([
      "action:newSession",
      "action:newProject",
      "action:shortcuts",
      "action:showLogs",
    ]);
    // Keys contain colons, so the id is split on the first one only.
    expect(groups[0].items[0]).toMatchObject({ id: "session:pi:s1", label: "Release checklist" });
    expect(groups[1].items).toHaveLength(1);
    expect(groups[1].items[0]).toMatchObject({
      id: "lane:4",
      hint: "Start a session · Web app · panoptes/fix-login",
    });
    expect(groups[2].items[0]).toMatchObject({ id: "project:1", hint: "New lane · /code/web" });
  });

  it("only warns that picking a lane starts a session when it has none", () => {
    const [, lanes] = paletteGroups([{ ...session, laneId: 4 }], [project], [lane]);
    expect(lanes.items[0].hint).toBe("Web app · panoptes/fix-login");
  });

  it("hints a lane session with its project and lane, an archived one first", () => {
    const owned = { ...session, laneId: 4 };
    const [sessions] = paletteGroups([owned], [project], [lane]);
    expect(sessions.items[0].hint).toBe("Web app · Fix login");

    const [archived] = paletteGroups([{ ...owned, archived: true }], [project], [lane]);
    expect(archived.items[0].hint).toBe("Archived · Web app · Fix login");
  });

  it("marks the session on screen as current", () => {
    const [sessions] = paletteGroups([session], [project], [lane], "pi:s1");
    expect(sessions.items[0].hint).toBe("Current · Web app");
  });
});

describe("recentSessions", () => {
  it("puts recent live sessions first, archived after, and the current one last", () => {
    const at = (key: string, updatedAt: number, archived = false) => ({
      ...session,
      key,
      updatedAt,
      archived,
    });
    const order = recentSessions(
      [at("old", 1), at("current", 9), at("new", 5), at("archived", 8, true)],
      "current",
    ).map((item) => item.key);
    expect(order).toEqual(["new", "old", "archived", "current"]);
  });
});

describe("paletteTarget", () => {
  it("traces ids back to the thing they stand for", () => {
    expect(paletteTarget("session:pi:s1", [session], [project], [lane])).toEqual({
      kind: "session",
      session,
    });
    expect(paletteTarget("lane:4", [session], [project], [lane])).toEqual({ kind: "lane", lane });
    expect(paletteTarget("project:1", [session], [project], [lane])).toEqual({
      kind: "project",
      project,
    });
    expect(paletteTarget("action:newSession", [session], [project], [lane])).toEqual({
      kind: "newSession",
    });
    expect(paletteTarget("action:newProject", [session], [project], [lane])).toEqual({
      kind: "newProject",
    });
    expect(paletteTarget("action:shortcuts", [session], [project], [lane])).toEqual({
      kind: "shortcuts",
    });
    expect(paletteTarget("action:showLogs", [session], [project], [lane])).toEqual({
      kind: "showLogs",
    });
  });

  it("returns null for ids that no longer match anything", () => {
    expect(paletteTarget("session:pi:gone", [session], [project], [lane])).toBeNull();
    expect(paletteTarget("lane:99", [session], [project], [lane])).toBeNull();
    expect(paletteTarget("action:nope", [session], [project], [lane])).toBeNull();
    expect(paletteTarget("nonsense", [session], [project], [lane])).toBeNull();
  });
});
